//! RAM boot: start the kernel and the root file system of an EFF with
//! `kexec`, writing nothing to flash.
//!
//! The image is the file the updater installs.  Its kernel (a zImage with
//! the model's devicetree appended, as it goes into `k0`/`k1`) is entered
//! the way boot2 enters it: at the load address in its header, with a tag
//! list (ATAGs) that carries the command line and the memory size, which
//! the kernel's decompressor folds into the appended devicetree.  Its root
//! file system, the JFFS2 image as it goes into `root0`/`root1`, is placed
//! at the top of RAM; the tag list ends the kernel's memory below it, and
//! `phram.phram=` on the command line turns that range into an MTD device
//! that the kernel mounts as the root (`root=mtd:ramroot`).
//!
//! Nothing here touches the hardware or makes a system call: this module
//! reads the image and works out what to hand to `kexec_load(2)`, so that
//! all of it runs in tests on a workstation.  The caller (`dphone ramboot`)
//! supplies the facts about the running phone and makes the calls.
//!
//! Why the tag list and not a devicetree of our own: it is what boot2
//! passes, so the kernel under test is entered exactly as it will be from
//! flash, and the devicetree in the image is used as it is.  The kernel
//! takes the tag list from `entry - 0x8000 + 0x1000` when no kexec segment
//! starts with the devicetree magic (`arch/arm/kernel/machine_kexec.c`).

use std::io::Read;

use dphone_state::Model;
use eff::ota::OtaReader;
use eff::{EffHeader, EffReader};

use crate::boot2::{self, KernelSpan, RAM_ALIAS};
use crate::error::{Result, UpdateError};
use crate::install::is_stock_version;

pub const PAGE: u32 = 4096;
/// Erase block size of the RAM-backed root device: the size the image was
/// laid out for (no JFFS2 node crosses a 128 KiB boundary).
pub const ERASE_BLOCK: u32 = 0x2_0000;
/// JFFS2 refuses a device with fewer erase blocks.
pub const MIN_ROOT_BLOCKS: u32 = 5;
/// Name of the RAM-backed root device (`root=mtd:ramroot`).
pub const ROOT_MTD: &str = "ramroot";

/// `KEXEC_ARM_ZIMAGE_OFFSET` and `KEXEC_ARM_ATAGS_OFFSET` of the running
/// kernel (`arch/arm/include/asm/kexec.h`, Linux 7.3): the tag list is
/// taken from `entry - ZIMAGE_OFFSET + ATAGS_OFFSET`.
pub const KEXEC_ZIMAGE_OFFSET: u32 = 0x8000;
pub const KEXEC_ATAGS_OFFSET: u32 = 0x1000;

/// `COMMAND_LINE_SIZE` on ARM.  The decompressor joins the devicetree's
/// `bootargs` and the tag list's line in a buffer of this size and drops
/// the tag list's line, silently, when the two do not fit.
pub const COMMAND_LINE_SIZE: usize = 1024;

/// Memory the started system must be left with.  The installed system
/// uses 12 MiB (D40) to 14 MiB (D60) with an empty `/tmp`, which is a
/// tmpfs of up to 40 MiB; this covers both with room to spare.
pub const MIN_SYSTEM_RAM: u32 = 64 << 20;

/// Free memory to leave to the running system while the image is loaded.
pub const LOAD_HEADROOM: u64 = 4 << 20;

/// Seconds after which the started system gives up by itself unless told
/// to stay (`dphone.max_uptime=`), when the caller does not say.
pub const DEFAULT_MAX_UPTIME: u64 = 900;

const ATAG_CORE: u32 = 0x5441_0001;
const ATAG_MEM: u32 = 0x5441_0002;
const ATAG_CMDLINE: u32 = 0x5441_0009;
const FDT_MAGIC: u32 = 0xd00d_feed;

fn bad<T>(msg: impl Into<String>) -> Result<T> {
    Err(UpdateError::Image(msg.into()))
}

fn refused<T>(msg: impl Into<String>) -> Result<T> {
    Err(UpdateError::Refused(msg.into()))
}

fn round_up(n: u64, to: u64) -> u64 {
    n.div_ceil(to) * to
}

/// Bytes the root file system occupies in RAM: whole erase blocks.
pub fn root_len(rootfs_size: u32) -> u64 {
    round_up(rootfs_size as u64, ERASE_BLOCK as u64).max((MIN_ROOT_BLOCKS * ERASE_BLOCK) as u64)
}

/// An EFF whose headers have been read and accepted; the kernel and the
/// root file system are still in the stream.
pub struct Opened<R: Read> {
    header: EffHeader,
    ota: OtaReader<EffReader<R>>,
}

/// Reads the EFF and OTA headers and refuses what cannot be RAM-booted:
/// another model's image, stock firmware, a damaged header.
pub fn open<R: Read>(reader: R, phone: Model) -> Result<Opened<R>> {
    let eff = EffReader::new(reader)?;
    let header = eff.header().clone();
    let phone = phone.to_string();
    if !header.model.trim().eq_ignore_ascii_case(&phone) {
        return Err(UpdateError::WrongModel { image: header.model, phone });
    }
    if is_stock_version(&header.version) {
        return refused(format!(
            "version {} is stock firmware; only an Open Desk Phone image can be started from RAM \
             (its kernel must mount a root file system held in RAM)",
            header.version.trim()
        ));
    }
    let ota = OtaReader::new(eff)?;
    let app = ota.header().app;
    if app.kernel_size == 0 || app.rootfs_size == 0 {
        return bad("empty kernel or root file system");
    }
    Ok(Opened { header, ota })
}

impl<R: Read> Opened<R> {
    pub fn version(&self) -> &str {
        self.header.version.trim()
    }

    pub fn kernel_size(&self) -> u32 {
        self.ota.header().app.kernel_size
    }

    pub fn rootfs_size(&self) -> u32 {
        self.ota.header().app.rootfs_size
    }

    /// Bytes `kexec_load` will be handed (page-rounded, as the kernel
    /// counts them).
    pub fn segment_bytes(&self) -> u64 {
        round_up(self.kernel_size() as u64, PAGE as u64) + PAGE as u64 + root_len(self.rootfs_size())
    }

    /// Free memory needed to load the image: once in this process and once
    /// in the pages the kernel copies it to, both at the time of the call.
    pub fn memory_needed(&self) -> u64 {
        2 * self.segment_bytes() + LOAD_HEADROOM
    }

    /// Refuses when the phone cannot hold the image for loading.
    /// `available` and `total` are `MemAvailable` and `MemTotal` in bytes.
    pub fn check_memory(&self, available: u64, total: u64) -> Result<()> {
        let need = self.memory_needed();
        if available < need {
            return refused(format!(
                "not enough free memory to load the image: {} MiB needed (the kernel and the root file \
                 system, {} MiB, twice while loading), {} MiB available; free some (e.g., files in /tmp; \
                 the image can be piped in: 'dphone ramboot -')",
                need.div_ceil(1 << 20),
                self.segment_bytes().div_ceil(1 << 20),
                available >> 20
            ));
        }
        // kexec_load itself refuses more than half of memory.
        if self.segment_bytes() > total / 2 {
            return refused(format!(
                "the image ({} MiB) is more than half of this phone's memory ({} MiB); the kernel does not load that much",
                self.segment_bytes().div_ceil(1 << 20),
                total >> 20
            ));
        }
        Ok(())
    }

    /// Reads the kernel and the root file system into memory, checking
    /// sizes, both CRCs, the end of the stream, and the kernel's header and
    /// devicetree.
    pub fn load(mut self) -> Result<Image> {
        let app = self.ota.header().app;
        let version = self.version().to_string();

        let mut kernel = vec![0u8; app.kernel_size as usize];
        let mut c = self.ota.next_component().expect("kernel component");
        c.read_exact(&mut kernel)?;
        c.finish()?;
        let span =
            boot2::check_kernel_header(&kernel, app.kernel_size as u64).map_err(|p| UpdateError::Image(p.to_string()))?;
        // What boot2 would copy, and so what runs: nothing past the span.
        kernel.truncate(span.len() as usize);
        let Some(bootargs) = appended_dtb_bootargs(&kernel) else {
            return bad("the kernel carries no devicetree at its end (not an Open Desk Phone kernel image)");
        };

        // 0xff to the end of the last erase block, as erased flash reads.
        let mut rootfs = vec![0xffu8; root_len(app.rootfs_size) as usize];
        let mut c = self.ota.next_component().expect("rootfs component");
        c.read_exact(&mut rootfs[..app.rootfs_size as usize])?;
        c.finish()?;
        self.ota.finish()?;
        if rootfs[..2] != [0x85, 0x19] {
            return bad("the root file system is not a little-endian JFFS2 image");
        }
        Ok(Image { version, kernel, span, dtb_bootargs: bootargs, rootfs })
    }
}

/// A verified image in memory.
pub struct Image {
    pub version: String,
    /// The kernel image: zImage, devicetree, padding.
    pub kernel: Vec<u8>,
    /// Where its header says it loads.
    pub span: KernelSpan,
    /// `/chosen/bootargs` of the kernel's devicetree (empty when absent).
    pub dtb_bootargs: String,
    /// The root file system, 0xff-padded to whole erase blocks.
    pub rootfs: Vec<u8>,
}

/// `/chosen/bootargs` of the devicetree appended to a kernel image: `None`
/// when no devicetree ends the image, an empty string when the devicetree
/// has no such property.
///
/// A reader for this one property rather than a devicetree crate: the
/// workspace has none, and nothing else here needs one.
pub fn appended_dtb_bootargs(kernel: &[u8]) -> Option<String> {
    let be = |b: &[u8], o: usize| b.get(o..o + 4).map(|w| u32::from_be_bytes(w.try_into().unwrap()));
    // The devicetree starts on a word boundary and runs to the end of the
    // image, up to three bytes of padding aside.  Take the last such one.
    let mut found = None;
    let mut off = kernel.len() & !3;
    while off >= 4 {
        off -= 4;
        if be(kernel, off) == Some(FDT_MAGIC) {
            let total = be(kernel, off + 4)? as usize;
            if total >= 40 && off + total <= kernel.len() && kernel.len() - (off + total) < 4 {
                found = Some(&kernel[off..off + total]);
                break;
            }
        }
    }
    let dtb = found?;
    let structs = be(dtb, 8)? as usize;
    let strings = be(dtb, 12)? as usize;
    let cstr = |from: usize| {
        let s = dtb.get(from..)?;
        let end = s.iter().position(|b| *b == 0)?;
        std::str::from_utf8(&s[..end]).ok()
    };
    let mut p = structs;
    let mut depth = 0u32;
    let mut in_chosen = false;
    loop {
        let token = be(dtb, p)?;
        p += 4;
        match token {
            // FDT_BEGIN_NODE: name, NUL, padding
            1 => {
                let name = cstr(p)?;
                p += (name.len() + 1 + 3) & !3;
                depth += 1;
                if depth == 2 {
                    in_chosen = name == "chosen";
                }
            }
            // FDT_END_NODE
            2 => {
                if depth == 2 {
                    in_chosen = false;
                }
                depth = depth.checked_sub(1)?;
            }
            // FDT_PROP: length, name offset, value, padding
            3 => {
                let len = be(dtb, p)? as usize;
                let name = cstr(strings + be(dtb, p + 4)? as usize)?;
                let value = dtb.get(p + 8..p + 8 + len)?;
                p += 8 + ((len + 3) & !3);
                if in_chosen && depth == 2 && name == "bootargs" {
                    let text = value.split(|b| *b == 0).next().unwrap_or(&[]);
                    return Some(String::from_utf8_lossy(text).into_owned());
                }
            }
            // FDT_NOP
            4 => {}
            // FDT_END
            9 => return Some(String::new()),
            _ => return None,
        }
    }
}

/// The phone's main memory: the one `System RAM` range of `/proc/iomem`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ram {
    pub start: u32,
    /// One past the last byte.
    pub end: u32,
}

/// Finds main memory in the text of `/proc/iomem` (as root sees it).
pub fn system_ram(iomem: &str) -> Result<Ram> {
    let mut ranges = iomem.lines().filter(|l| !l.starts_with(' ')).filter_map(|l| {
        let (range, name) = l.split_once(" : ")?;
        if name.trim() != "System RAM" {
            return None;
        }
        let (a, b) = range.trim().split_once('-')?;
        Some((u64::from_str_radix(a, 16).ok()?, u64::from_str_radix(b, 16).ok()?))
    });
    let (Some((start, last)), None) = (ranges.next(), ranges.next()) else {
        return Err(UpdateError::Layout("expected exactly one System RAM range in /proc/iomem".into()));
    };
    if start == 0 && last == 0 {
        return Err(UpdateError::Layout("/proc/iomem hides addresses (not root?)".into()));
    }
    if last <= start || last >= u32::MAX as u64 {
        return Err(UpdateError::Layout(format!("unusable System RAM range {start:#x}-{last:#x}")));
    }
    Ok(Ram { start: start as u32, end: last as u32 + 1 })
}

/// A value of `/proc/meminfo` in bytes.
pub fn meminfo(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|l| {
        let rest = l.strip_prefix(key)?.strip_prefix(':')?;
        let kb: u64 = rest.trim().strip_suffix("kB")?.trim().parse().ok()?;
        Some(kb * 1024)
    })
}

/// What the caller chooses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// `dphone.max_uptime=`: the started system resets the phone after
    /// this many seconds unless `/run/dphone/stay` exists.  `None`: no
    /// limit.
    pub max_uptime: Option<u64>,
    /// Extra kernel arguments, added last.
    pub append: String,
}

impl Default for Options {
    fn default() -> Self {
        Options { max_uptime: Some(DEFAULT_MAX_UPTIME), append: String::new() }
    }
}

/// One `kexec_load` segment: `data` goes to physical `mem`, and the rest
/// of `memsz` is cleared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment<'a> {
    pub name: &'static str,
    pub data: &'a [u8],
    pub mem: u32,
    pub memsz: u32,
}

/// Everything `kexec_load` needs, and what the started kernel is told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Physical entry point: the start of the kernel image.
    pub entry: u32,
    pub kernel_at: u32,
    pub atags_at: u32,
    pub root_at: u32,
    pub root_len: u32,
    /// Main memory left to the started kernel: `mem_start..root_at`.
    pub mem_start: u32,
    /// The tag list: one page.
    pub atags: Vec<u8>,
    /// The line in the tag list (the kernel puts its devicetree's
    /// `bootargs` in front).
    pub cmdline: String,
}

impl Plan {
    pub fn mem_size(&self) -> u32 {
        self.root_at - self.mem_start
    }

    /// The segments, in address order.
    pub fn segments<'a>(&'a self, image: &'a Image) -> [Segment<'a>; 3] {
        [
            Segment { name: "tag list", data: &self.atags, mem: self.atags_at, memsz: PAGE },
            Segment {
                name: "kernel",
                data: &image.kernel,
                mem: self.kernel_at,
                memsz: round_up(image.kernel.len() as u64, PAGE as u64) as u32,
            },
            Segment { name: "root file system", data: &image.rootfs, mem: self.root_at, memsz: self.root_len },
        ]
    }
}

/// The tag list boot2 builds, with our memory size and command line:
/// CORE, MEM, CMDLINE, NONE.
fn build_atags(mem_start: u32, mem_size: u32, cmdline: &str) -> Vec<u8> {
    let text_words = (cmdline.len() + 1).div_ceil(4);
    let mut words: Vec<u32> = vec![
        5, ATAG_CORE, 0, PAGE, 0, // flags, page size, root device
        4, ATAG_MEM, mem_size, mem_start,
        2 + text_words as u32, ATAG_CMDLINE,
    ];
    let mut text = cmdline.as_bytes().to_vec();
    text.resize(text_words * 4, 0);
    words.extend(text.chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())));
    words.extend([0, 0]); // ATAG_NONE
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// The command line for the tag list: the running system's own line (what
/// boot2 passed: `mtdparts=`, the board, and so on) without what names the
/// root or belongs to an earlier RAM boot, without what the image's
/// devicetree already says, and with the RAM root and the options added.
pub fn build_cmdline(running: &str, dtb_bootargs: &str, root_at: u32, root_len: u32, options: &Options) -> String {
    const DROP_KEYS: &[&str] =
        &["root", "rootfstype", "rootflags", "rootwait", "rootdelay", "ro", "rw", "phram.phram", "bcmring.coldsim", "dphone.max_uptime"];
    let from_dtb: Vec<&str> = dtb_bootargs.split_ascii_whitespace().collect();
    let mut out: Vec<String> = running
        .split_ascii_whitespace()
        .filter(|t| !DROP_KEYS.contains(&t.split('=').next().unwrap_or(t)))
        .filter(|t| !from_dtb.contains(t))
        .map(str::to_string)
        .collect();
    out.push(format!("root=mtd:{ROOT_MTD}"));
    out.push("rootfstype=jffs2".into());
    out.push("ro".into());
    out.push(format!("phram.phram={ROOT_MTD},{root_at:#x},{root_len:#x},{ERASE_BLOCK:#x}"));
    // A kernel entered from a running system inherits its hardware state;
    // this makes the kernel put the peripherals back as a reset leaves
    // them before any driver looks (mach-bcmring).  Every RAM boot of the
    // bring-up that came up used it.
    out.push("bcmring.coldsim".into());
    if let Some(secs) = options.max_uptime {
        out.push(format!("dphone.max_uptime={secs}"));
    }
    out.extend(options.append.split_ascii_whitespace().map(str::to_string));
    out.join(" ")
}

/// Lays the image out in memory.  `ram` is the running kernel's main
/// memory and `running_cmdline` its command line.
pub fn plan(image: &Image, ram: Ram, running_cmdline: &str, options: &Options) -> Result<Plan> {
    if options.append.bytes().any(|b| b == b'"' || !(b == b' ' || b.is_ascii_graphic())) {
        return refused("extra kernel arguments must be printable ASCII without quotes");
    }
    // The root file system: whole erase blocks, ending where memory ends.
    let root_len = image.rootfs.len() as u64;
    if !root_len.is_multiple_of(ERASE_BLOCK as u64) || root_len < (MIN_ROOT_BLOCKS * ERASE_BLOCK) as u64 {
        return bad("root file system is not padded to whole erase blocks");
    }
    let top = ram.end as u64 & !(PAGE as u64 - 1);
    let left = (top - ram.start as u64).saturating_sub(root_len);
    if left < MIN_SYSTEM_RAM as u64 {
        return refused(format!(
            "not enough memory: the root file system takes {} MiB of the {} MiB this system has, which leaves \
             {} MiB; the started system needs {} MiB{}",
            root_len.div_ceil(1 << 20),
            (top - ram.start as u64) >> 20,
            left >> 20,
            MIN_SYSTEM_RAM >> 20,
            if running_cmdline.contains("phram.phram=") {
                " (this system was itself started from RAM and has less; reboot into the installed firmware first)"
            } else {
                ""
            }
        ));
    }
    let root_at = (top - root_len) as u32;

    // The kernel, where boot2 would put it, and the tag list where the
    // running kernel will point the new one.
    let kernel_at = image.span.start & (RAM_ALIAS - 1);
    let kernel_end = kernel_at as u64 + round_up(image.kernel.len() as u64, PAGE as u64);
    if !kernel_at.is_multiple_of(PAGE) {
        return bad(format!("kernel load address {kernel_at:#x} is not page-aligned"));
    }
    if (kernel_at as u64) < ram.start as u64 + KEXEC_ZIMAGE_OFFSET as u64 || kernel_end > root_at as u64 {
        return refused(format!(
            "the kernel's load range {kernel_at:#x}..{kernel_end:#x} does not fit this system's memory \
             ({:#x}..{root_at:#x} once the root file system is placed)",
            ram.start
        ));
    }
    let atags_at = kernel_at - KEXEC_ZIMAGE_OFFSET + KEXEC_ATAGS_OFFSET;

    let cmdline = build_cmdline(running_cmdline, &image.dtb_bootargs, root_at, root_len as u32, options);
    // "bootargs cmdline\0" must fit, with one byte the kernel's check keeps.
    if image.dtb_bootargs.len() + 1 + cmdline.len() + 2 >= COMMAND_LINE_SIZE {
        return refused(format!(
            "the kernel command line would be {} characters; the kernel takes fewer than {}",
            image.dtb_bootargs.len() + 1 + cmdline.len(),
            COMMAND_LINE_SIZE - 2
        ));
    }
    let mem_size = root_at - ram.start;
    let mut atags = build_atags(ram.start, mem_size, &cmdline);
    debug_assert!(atags.len() <= PAGE as usize);
    atags.resize(PAGE as usize, 0);
    Ok(Plan { entry: kernel_at, kernel_at, atags_at, root_at, root_len: root_len as u32, mem_start: ram.start, atags, cmdline })
}

#[cfg(test)]
mod tests {
    use super::*;

    const D60_CMDLINE: &str = "console=ttyAMA0,115200n8 earlycon=pl011,0x80012000 panic=10 console=ttyAMA0,115200n8 vmalloc=256m  \
        digium_board=D60 digium_boot_rev=3 mtdparts=bcm_umi-nand:0x00020000(boot1),0x00080000(boot2),0x00080000(ctrl),\
        0x00ee0000(nvdata),0x00200000(k0),0x00200000(k1),0x02d00000(root0),0x02d00000(root1),0x01000000(userdata) \
        root=/dev/mtdblock6 rootfstype=jffs2 ro bootmemheap";
    const DTB_ARGS: &str = "console=ttyAMA0,115200n8 earlycon=pl011,0x80012000 panic=10";
    const D60_IOMEM: &str = "00400000-07ffffff : System RAM\n  00408000-008a9fff : Kernel code\n  008d0000-009041fb : Kernel data\n\
        30200000-30200fff : 30200000.dma-controller dma-controller@30200000\n80017800-80017fff : watchdog@80017800\n";

    /// A flattened devicetree with `/chosen/bootargs` (when given) and
    /// some other nodes and properties around it.
    fn dtb(bootargs: Option<&str>) -> Vec<u8> {
        let mut strings = Vec::new();
        let mut name_off = |n: &str| {
            let o = strings.len() as u32;
            strings.extend_from_slice(n.as_bytes());
            strings.push(0);
            o
        };
        let mut s: Vec<u8> = Vec::new();
        fn pad(s: &mut Vec<u8>) {
            while !s.len().is_multiple_of(4) {
                s.push(0);
            }
        }
        let begin = |s: &mut Vec<u8>, name: &str| {
            s.extend(1u32.to_be_bytes());
            s.extend(name.as_bytes());
            s.push(0);
            pad(s);
        };
        let mut prop = |s: &mut Vec<u8>, name: &str, value: &[u8]| {
            s.extend(3u32.to_be_bytes());
            s.extend((value.len() as u32).to_be_bytes());
            s.extend(name_off(name).to_be_bytes());
            s.extend(value);
            pad(s);
        };
        let end = |s: &mut Vec<u8>| s.extend(2u32.to_be_bytes());
        begin(&mut s, "");
        prop(&mut s, "compatible", b"digium,d60\0brcm,bcm11189\0");
        begin(&mut s, "aliases");
        prop(&mut s, "bootargs", b"not this one\0");
        end(&mut s);
        begin(&mut s, "chosen");
        prop(&mut s, "stdout-path", b"serial0:115200n8\0");
        begin(&mut s, "deeper");
        prop(&mut s, "bootargs", b"nor this one\0");
        end(&mut s);
        if let Some(b) = bootargs {
            let mut v = b.as_bytes().to_vec();
            v.push(0);
            s.extend(4u32.to_be_bytes()); // a NOP on the way
            prop(&mut s, "bootargs", &v);
        }
        end(&mut s);
        end(&mut s);
        s.extend(9u32.to_be_bytes());
        let off_struct = 40u32;
        let off_strings = off_struct + s.len() as u32;
        let total = off_strings + strings.len() as u32;
        let mut out = Vec::new();
        for w in [FDT_MAGIC, total, off_struct, off_strings, 40, 17, 16, 0, strings.len() as u32, s.len() as u32] {
            out.extend(w.to_be_bytes());
        }
        out.extend(s);
        out.extend(strings);
        out
    }

    /// A kernel image as the build makes it: a zImage body with boot2's
    /// header words, the devicetree, padding to a word.
    fn kernel(body: usize, bootargs: Option<&str>) -> Vec<u8> {
        // The zImage ends on a word boundary; the devicetree need not.
        let mut k: Vec<u8> = (0..body.div_ceil(4) * 4).map(|i| (i * 7 + 3) as u8).collect();
        // A stray magic in the body must not be taken for the devicetree.
        k[0x100..0x104].copy_from_slice(&FDT_MAGIC.to_be_bytes());
        k.extend(dtb(bootargs));
        while !k.len().is_multiple_of(4) {
            k.push(0);
        }
        let len = k.len() as u32;
        k[0x24..0x28].copy_from_slice(&boot2::ZIMAGE_MAGIC.to_le_bytes());
        k[0x28..0x2c].copy_from_slice(&0x0200_0000u32.to_le_bytes());
        k[0x2c..0x30].copy_from_slice(&(0x0200_0000 + len).to_le_bytes());
        k
    }

    fn rootfs(len: usize) -> Vec<u8> {
        let mut r: Vec<u8> = (0..len).map(|i| (i * 13 + 1) as u8).collect();
        r[..4].copy_from_slice(&[0x85, 0x19, 0x01, 0xe0]);
        r
    }

    fn eff_of(model: &str, version: &str, kernel: &[u8], rootfs: &[u8]) -> Vec<u8> {
        let name = format!("{version}_{model}_firmware.bin");
        let ota = eff::ota::pack(kernel, rootfs, "Sat Nov 14 12:00:00 2026", &name).unwrap();
        eff::envelope::pack(&EffHeader::new(model, version).unwrap(), &ota, [7; 8]).unwrap()
    }

    fn d60_image() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let (k, r) = (kernel(300_000, Some(DTB_ARGS)), rootfs(6 * ERASE_BLOCK as usize + 1234));
        let eff = eff_of("D60", "3_0_0", &k, &r);
        (eff, k, r)
    }

    fn loaded() -> Image {
        open(&d60_image().0[..], Model::D60).unwrap().load().unwrap()
    }

    fn message<T>(r: Result<T>) -> String {
        match r {
            Ok(_) => panic!("expected an error"),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn reads_bootargs_from_the_appended_devicetree() {
        assert_eq!(appended_dtb_bootargs(&kernel(5000, Some(DTB_ARGS))).as_deref(), Some(DTB_ARGS));
        assert_eq!(appended_dtb_bootargs(&kernel(5001, Some("a=1"))).as_deref(), Some("a=1"));
        // A devicetree without the property: present, empty.
        assert_eq!(appended_dtb_bootargs(&kernel(5000, None)).as_deref(), Some(""));
        // No devicetree at the end: a bare zImage, or one cut short.
        let bare: Vec<u8> = vec![0x55; 5000];
        assert_eq!(appended_dtb_bootargs(&bare), None);
        let mut cut = kernel(5000, Some(DTB_ARGS));
        cut.truncate(cut.len() - 8);
        assert_eq!(appended_dtb_bootargs(&cut), None);
        // Damaged structure: no panic, no answer.
        let mut broken = kernel(5000, Some(DTB_ARGS));
        let at = broken.len() - dtb(Some(DTB_ARGS)).len() + 40;
        broken[at..at + 4].copy_from_slice(&77u32.to_be_bytes());
        assert_eq!(appended_dtb_bootargs(&broken), None);
    }

    #[test]
    fn loads_and_verifies_an_image() {
        let (eff, k, r) = d60_image();
        let opened = open(&eff[..], Model::D60).unwrap();
        assert_eq!(opened.version(), "3_0_0");
        assert_eq!((opened.kernel_size() as usize, opened.rootfs_size() as usize), (k.len(), r.len()));
        let image = opened.load().unwrap();
        assert_eq!(image.kernel, k);
        assert_eq!(image.span, KernelSpan { start: 0x0200_0000, end: 0x0200_0000 + k.len() as u32 });
        assert_eq!(image.dtb_bootargs, DTB_ARGS);
        // The root file system fills whole erase blocks, 0xff after its end.
        assert_eq!(image.rootfs.len(), 7 * ERASE_BLOCK as usize);
        assert_eq!(image.rootfs[..r.len()], r[..]);
        assert!(image.rootfs[r.len()..].iter().all(|b| *b == 0xff));
    }

    #[test]
    fn a_small_root_file_system_still_gets_the_blocks_jffs2_wants() {
        assert_eq!(root_len(1), (MIN_ROOT_BLOCKS * ERASE_BLOCK) as u64);
        assert_eq!(root_len(18_067_404), 138 * ERASE_BLOCK as u64);
        assert_eq!(root_len(138 * ERASE_BLOCK), 138 * ERASE_BLOCK as u64);
    }

    #[test]
    fn refuses_another_models_image() {
        let (k, r) = (kernel(300_000, Some(DTB_ARGS)), rootfs(ERASE_BLOCK as usize));
        let eff = eff_of("D40", "3_0_0", &k, &r);
        let e = open(&eff[..], Model::D60).err().unwrap();
        assert!(matches!(e, UpdateError::WrongModel { .. }), "{e}");
        assert_eq!(e.to_string(), "firmware image is for the D40, this phone is a D60");
    }

    #[test]
    fn refuses_stock_firmware() {
        let (k, r) = (kernel(300_000, Some(DTB_ARGS)), rootfs(ERASE_BLOCK as usize));
        let eff = eff_of("D60", "2_9_28", &k, &r);
        let m = message(open(&eff[..], Model::D60));
        assert!(m.contains("stock firmware"), "{m}");
    }

    #[test]
    fn refuses_corrupt_images() {
        let (eff, _, _) = d60_image();
        // Not an EFF at all.
        assert!(open(&b"not an image"[..], Model::D60).is_err());
        assert!(open(&[0u8; 4096][..], Model::D60).is_err());
        // Cut short in the kernel, and in the root file system.
        for keep in [2000, eff.len() - 5000] {
            let r = open(&eff[..keep], Model::D60).and_then(Opened::load);
            assert!(matches!(r, Err(UpdateError::Image(_))), "cut at {keep}");
        }
        // One byte changed in the kernel body, and in the root file system:
        // the CRCs catch both.  (The cipher is CBC, so the change garbles
        // the rest of its 16-byte block and the next; both stay inside
        // the component.)
        for at in [700 + 100_000, eff.len() - 100_000] {
            let mut bad = eff.clone();
            bad[at] ^= 0x40;
            let r = open(&bad[..], Model::D60).and_then(Opened::load);
            let m = message(r);
            assert!(m.contains("crc"), "change at {at}: {m}");
        }
        // Trailing data after the image.
        let mut long = eff.clone();
        long.extend([0u8; 64]);
        assert!(open(&long[..], Model::D60).and_then(Opened::load).is_err());
    }

    #[test]
    fn refuses_kernels_boot2_could_not_enter_or_without_a_devicetree() {
        let r = rootfs(ERASE_BLOCK as usize);
        // No zImage magic.
        let mut k = kernel(300_000, Some(DTB_ARGS));
        k[0x24] ^= 1;
        let m = message(open(&eff_of("D60", "3_0_0", &k, &r)[..], Model::D60).and_then(Opened::load));
        assert!(m.contains("not a zImage"), "{m}");
        // Header not patched with a load address (a zImage straight from
        // a kernel build).
        let mut k = kernel(300_000, Some(DTB_ARGS));
        let len = k.len() as u32;
        k[0x28..0x2c].copy_from_slice(&0u32.to_le_bytes());
        k[0x2c..0x30].copy_from_slice(&len.to_le_bytes());
        let m = message(open(&eff_of("D60", "3_0_0", &k, &r)[..], Model::D60).and_then(Opened::load));
        assert!(m.contains("load span"), "{m}");
        // No devicetree appended.
        let mut k: Vec<u8> = vec![0x11; 300_000];
        k[0x24..0x28].copy_from_slice(&boot2::ZIMAGE_MAGIC.to_le_bytes());
        k[0x28..0x2c].copy_from_slice(&0x0200_0000u32.to_le_bytes());
        k[0x2c..0x30].copy_from_slice(&(0x0200_0000u32 + 300_000).to_le_bytes());
        let m = message(open(&eff_of("D60", "3_0_0", &k, &r)[..], Model::D60).and_then(Opened::load));
        assert!(m.contains("no devicetree"), "{m}");
        // A root file system that is not JFFS2.
        let k = kernel(300_000, Some(DTB_ARGS));
        let m = message(open(&eff_of("D60", "3_0_0", &k, &vec![0x33; 70_000])[..], Model::D60).and_then(Opened::load));
        assert!(m.contains("JFFS2"), "{m}");
    }

    #[test]
    fn refuses_when_memory_is_short_for_loading() {
        let (eff, k, _) = d60_image();
        let opened = open(&eff[..], Model::D60).unwrap();
        let seg = (k.len() as u64).div_ceil(4096) * 4096 + 4096 + 7 * ERASE_BLOCK as u64;
        assert_eq!(opened.segment_bytes(), seg);
        assert_eq!(opened.memory_needed(), 2 * seg + LOAD_HEADROOM);
        assert!(opened.check_memory(2 * seg + LOAD_HEADROOM, 120 << 20).is_ok());
        let m = message(opened.check_memory(2 * seg + LOAD_HEADROOM - 1, 120 << 20));
        assert!(m.contains("not enough free memory"), "{m}");
        // The kernel's own bound: half of memory.
        let m = message(opened.check_memory(1 << 30, 2 * seg - 2));
        assert!(m.contains("more than half"), "{m}");
    }

    #[test]
    fn finds_main_memory() {
        assert_eq!(system_ram(D60_IOMEM).unwrap(), Ram { start: 0x0040_0000, end: 0x0800_0000 });
        let d40 = "00200000-07ffffff : System RAM\n  00208000-006a9fff : Kernel code\n";
        assert_eq!(system_ram(d40).unwrap(), Ram { start: 0x0020_0000, end: 0x0800_0000 });
        // A system that was itself started from RAM has less.
        let nested = "00400000-06ebffff : System RAM\n";
        assert_eq!(system_ram(nested).unwrap(), Ram { start: 0x0040_0000, end: 0x06ec_0000 });
        // Not root: the kernel prints zeros.
        assert!(system_ram("00000000-00000000 : System RAM\n").is_err());
        assert!(system_ram("30200000-30200fff : dma\n").is_err());
        assert!(system_ram("00400000-03ffffff : System RAM\n04000000-07ffffff : System RAM\n").is_err());
    }

    #[test]
    fn reads_meminfo() {
        let text = "MemTotal:         120808 kB\nMemFree:           58564 kB\nMemAvailable:      74964 kB\n";
        assert_eq!(meminfo(text, "MemAvailable"), Some(74964 * 1024));
        assert_eq!(meminfo(text, "MemTotal"), Some(120808 * 1024));
        assert_eq!(meminfo(text, "Mem"), None);
        assert_eq!(meminfo(text, "SwapFree"), None);
    }

    #[test]
    fn command_line_for_a_d60() {
        let line = build_cmdline(D60_CMDLINE, DTB_ARGS, 0x06ec_0000, 0x0114_0000, &Options::default());
        assert_eq!(
            line,
            "vmalloc=256m digium_board=D60 digium_boot_rev=3 mtdparts=bcm_umi-nand:0x00020000(boot1),0x00080000(boot2),\
             0x00080000(ctrl),0x00ee0000(nvdata),0x00200000(k0),0x00200000(k1),0x02d00000(root0),0x02d00000(root1),\
             0x01000000(userdata) bootmemheap root=mtd:ramroot rootfstype=jffs2 ro \
             phram.phram=ramroot,0x6ec0000,0x1140000,0x20000 bcmring.coldsim dphone.max_uptime=900"
        );
        // The started system is from neither bank by the updater's rule.
        assert_eq!(crate::bank::root_mtd_index(&format!("{DTB_ARGS} {line}")), None);
    }

    #[test]
    fn command_line_options_and_a_second_ram_boot() {
        let options = Options { max_uptime: None, append: " amba_pl011.console_output=1   panic=30 ".into() };
        // The line of a system that was itself started from RAM: its root,
        // its RAM device, and its limit must not be passed on.
        let first = format!("{DTB_ARGS} {}", build_cmdline(D60_CMDLINE, DTB_ARGS, 0x06ec_0000, 0x0114_0000, &Options::default()));
        let line = build_cmdline(&first, DTB_ARGS, 0x05d8_0000, 0x0114_0000, &options);
        assert_eq!(line.matches("phram.phram=").count(), 1);
        assert_eq!(line.matches("root=").count(), 1);
        assert_eq!(line.matches("bcmring.coldsim").count(), 1);
        assert!(line.contains("phram.phram=ramroot,0x5d80000,0x1140000,0x20000"));
        assert!(!line.contains("dphone.max_uptime"));
        assert!(!line.contains("earlycon"));
        assert!(line.ends_with(" bcmring.coldsim amba_pl011.console_output=1 panic=30"), "{line}");
        assert!(line.contains(" mtdparts=bcm_umi-nand:") && line.contains(" bootmemheap "));
        // A devicetree with other arguments: the running line's are kept.
        let line = build_cmdline(D60_CMDLINE, "panic=5", 0x06ec_0000, 0x0114_0000, &Options::default());
        assert!(line.starts_with("console=ttyAMA0,115200n8 earlycon=pl011,0x80012000 panic=10 console=ttyAMA0,115200n8 vmalloc=256m "));
    }

    #[test]
    fn plan_for_a_d60() {
        let image = loaded();
        let ram = system_ram(D60_IOMEM).unwrap();
        let plan = plan(&image, ram, D60_CMDLINE, &Options::default()).unwrap();
        // Seven erase blocks of root file system end where memory ends.
        assert_eq!((plan.root_at, plan.root_len), (0x0800_0000 - 0xe_0000, 0xe_0000));
        assert_eq!((plan.mem_start, plan.mem_size()), (0x0040_0000, 0x0800_0000 - 0xe_0000 - 0x0040_0000));
        // The kernel where boot2 loads it; the tag list where the running
        // kernel points register 2.
        assert_eq!((plan.entry, plan.kernel_at, plan.atags_at), (0x0200_0000, 0x0200_0000, 0x01ff_9000));
        assert!(plan.cmdline.contains("phram.phram=ramroot,0x7f20000,0xe0000,0x20000"));

        let segs = plan.segments(&image);
        assert_eq!(segs.map(|s| s.name), ["tag list", "kernel", "root file system"]);
        // Page-aligned, in order, not overlapping, inside memory, and no
        // segment begins with the devicetree magic (the kernel would hand
        // that one over in place of the tag list).
        let mut last_end = ram.start as u64;
        for s in &segs {
            assert_eq!((s.mem % PAGE, s.memsz % PAGE), (0, 0), "{}", s.name);
            assert!(s.data.len() <= s.memsz as usize, "{}", s.name);
            assert!(s.mem as u64 >= last_end, "{}", s.name);
            last_end = s.mem as u64 + s.memsz as u64;
            assert_ne!(&s.data[..4], &FDT_MAGIC.to_be_bytes(), "{}", s.name);
        }
        assert_eq!(last_end, ram.end as u64);
        assert_eq!(segs[1].memsz as usize, image.kernel.len().div_ceil(4096) * 4096);
        assert_eq!(segs[2].data.len(), segs[2].memsz as usize);
    }

    #[test]
    fn tag_list_is_what_the_decompressor_reads() {
        let image = loaded();
        let plan = plan(&image, system_ram(D60_IOMEM).unwrap(), D60_CMDLINE, &Options::default()).unwrap();
        assert_eq!(plan.atags.len(), PAGE as usize);
        let w = |i: usize| u32::from_le_bytes(plan.atags[4 * i..4 * i + 4].try_into().unwrap());
        // ATAG_CORE, five words (arch/arm/boot/compressed/atags_to_fdt.c
        // accepts 5 or 2).
        assert_eq!((w(0), w(1), w(3)), (5, 0x5441_0001, 4096));
        // ATAG_MEM: size, then start.
        assert_eq!((w(5), w(6), w(7), w(8)), (4, 0x5441_0002, plan.mem_size(), 0x0040_0000));
        // ATAG_CMDLINE: NUL-terminated, padded to a word.
        assert_eq!(w(10), 0x5441_0009);
        let text = &plan.atags[44..];
        let end = text.iter().position(|b| *b == 0).unwrap();
        assert_eq!(std::str::from_utf8(&text[..end]).unwrap(), plan.cmdline);
        assert_eq!(w(9) as usize, 2 + (plan.cmdline.len() + 1).div_ceil(4));
        // ATAG_NONE follows, and the rest of the page is zero.
        let none = 9 + w(9) as usize;
        assert_eq!((w(none), w(none + 1)), (0, 0));
        assert!(plan.atags[4 * none..].iter().all(|b| *b == 0));
    }

    #[test]
    fn refuses_when_the_started_system_would_have_too_little_memory() {
        let image = loaded();
        // 64 MiB of memory: less would be left.
        let small = Ram { start: 0x0040_0000, end: 0x0440_0000 };
        let m = message(plan(&image, small, D60_CMDLINE, &Options::default()));
        assert!(m.contains("not enough memory") && m.contains("needs 64 MiB"), "{m}");
        // Exactly enough.
        let enough = Ram { start: 0x0040_0000, end: 0x0440_0000 + 0xe_0000 };
        assert!(plan(&image, enough, D60_CMDLINE, &Options::default()).is_ok());
        // From a system that was itself started from RAM, the message says so.
        let nested = format!("{D60_CMDLINE} phram.phram=ramroot,0x6ec0000,0x1140000,0x20000");
        let m = message(plan(&image, small, &nested, &Options::default()));
        assert!(m.contains("itself started from RAM"), "{m}");
    }

    #[test]
    fn refuses_a_kernel_that_does_not_fit_memory() {
        let mut image = loaded();
        // RAM that begins above the load address.
        let high = Ram { start: 0x0200_0000, end: 0x0800_0000 };
        let m = message(plan(&image, high, D60_CMDLINE, &Options::default()));
        assert!(m.contains("load range"), "{m}");
        // A load address that would put the kernel into the root file system.
        image.span = KernelSpan { start: 0x07f6_0000, end: 0x07f6_0000 + image.kernel.len() as u32 };
        let m = message(plan(&image, system_ram(D60_IOMEM).unwrap(), D60_CMDLINE, &Options::default()));
        assert!(m.contains("load range"), "{m}");
    }

    #[test]
    fn refuses_a_command_line_the_kernel_would_drop() {
        let image = loaded();
        let ram = system_ram(D60_IOMEM).unwrap();
        let base = plan(&image, ram, D60_CMDLINE, &Options::default()).unwrap().cmdline.len();
        let room = COMMAND_LINE_SIZE - 2 - DTB_ARGS.len() - 1 - base - 1;
        let fits = Options { append: "x".repeat(room - 1), ..Options::default() };
        assert!(plan(&image, ram, D60_CMDLINE, &fits).is_ok());
        let too_long = Options { append: "x".repeat(room), ..Options::default() };
        let m = message(plan(&image, ram, D60_CMDLINE, &too_long));
        assert!(m.contains("command line would be"), "{m}");
        let quoted = Options { append: "a=\"b c\"".into(), ..Options::default() };
        assert!(plan(&image, ram, D60_CMDLINE, &quoted).is_err());
    }

    #[test]
    fn stock_load_addresses_wrap_as_boot2_wraps_them() {
        // Not reachable through `open` (stock is refused), but the layout
        // follows boot2 for any header it accepts.
        let mut image = loaded();
        image.span = KernelSpan { start: 0x0d00_0000, end: 0x0d00_0000 + image.kernel.len() as u32 };
        let plan = plan(&image, system_ram(D60_IOMEM).unwrap(), D60_CMDLINE, &Options::default()).unwrap();
        assert_eq!((plan.kernel_at, plan.atags_at), (0x0500_0000, 0x04ff_9000));
    }
}
