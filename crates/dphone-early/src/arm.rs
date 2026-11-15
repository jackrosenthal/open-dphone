//! The program proper: ARM Linux system calls and the boot sequence.

use core::arch::asm;
use core::ffi::c_char;
use core::panic::PanicInfo;
use core::ptr::{addr_of_mut, null};

use dphone_ctrl::boot::{self, Step};
use dphone_ctrl::trial::{TEXT_MAX, Trial};
use dphone_ctrl::{Flash, Geometry, record};

// The C library (musl, static) is linked for its start-up code and for the
// memory routines the compiler calls; everything else is a raw system call.
#[link(name = "c")]
unsafe extern "C" {}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    say(&["bug: panic; restarting"]);
    restart()
}

// ------------------------------------------------------------ system calls

const SYS_EXIT: usize = 1;
const SYS_READ: usize = 3;
const SYS_WRITE: usize = 4;
const SYS_OPEN: usize = 5;
const SYS_CLOSE: usize = 6;
const SYS_UNLINK: usize = 10;
const SYS_EXECVE: usize = 11;
const SYS_CHDIR: usize = 12;
const SYS_LSEEK: usize = 19;
const SYS_MOUNT: usize = 21;
const SYS_ACCESS: usize = 33;
const SYS_SYNC: usize = 36;
const SYS_RENAME: usize = 38;
const SYS_MKDIR: usize = 39;
const SYS_UMOUNT2: usize = 52;
const SYS_IOCTL: usize = 54;
const SYS_CHROOT: usize = 61;
const SYS_REBOOT: usize = 88;
const SYS_FSYNC: usize = 118;
const SYS_NANOSLEEP: usize = 162;

const O_WRONLY: usize = 1;
const O_RDWR: usize = 2;
const O_CREAT: usize = 0o100;
const O_TRUNC: usize = 0o1000;
const O_CLOEXEC: usize = 0o2000000;

const MS_RDONLY: usize = 1;
const MS_NOSUID: usize = 2;
const MS_NODEV: usize = 4;
const MS_NOATIME: usize = 1024;
const MS_MOVE: usize = 8192;
const MNT_DETACH: usize = 2;

/// One system call; the result is the kernel's (negative errno on error).
fn sys(n: usize, a: [usize; 5]) -> isize {
    let ret: isize;
    // SAFETY: the Linux ARM EABI convention: number in r7, arguments in
    // r0..r4, result in r0; the kernel preserves everything else.  Every
    // pointer passed by the callers below is valid for what the call does.
    unsafe {
        asm!(
            "svc 0",
            in("r7") n,
            inlateout("r0") a[0] => ret,
            in("r1") a[1],
            in("r2") a[2],
            in("r3") a[3],
            in("r4") a[4],
            options(nostack),
        );
    }
    ret
}

/// A path or other text as the kernel wants it: NUL-terminated.
struct CStr<const N: usize>([u8; N]);

impl<const N: usize> CStr<N> {
    /// `parts` joined; `None` when they do not fit or hold a NUL.
    fn new(parts: &[&str]) -> Option<Self> {
        let mut buf = [0u8; N];
        let mut n = 0;
        for p in parts {
            if n + p.len() >= N || p.bytes().any(|b| b == 0) {
                return None;
            }
            buf[n..n + p.len()].copy_from_slice(p.as_bytes());
            n += p.len();
        }
        Some(CStr(buf))
    }
    fn ptr(&self) -> usize {
        self.0.as_ptr() as usize
    }
}

fn c(s: &str) -> CStr<64> {
    // Every caller passes a short constant.
    CStr::new(&[s]).unwrap_or(CStr([0; 64]))
}

fn open(path: &str, flags: usize) -> isize {
    sys(SYS_OPEN, [c(path).ptr(), flags | O_CLOEXEC, 0o644, 0, 0])
}

fn close(fd: isize) {
    sys(SYS_CLOSE, [fd as usize, 0, 0, 0, 0]);
}

fn write_all(fd: isize, mut data: &[u8]) -> bool {
    while !data.is_empty() {
        let n = sys(SYS_WRITE, [fd as usize, data.as_ptr() as usize, data.len(), 0, 0]);
        if n <= 0 {
            return false;
        }
        data = &data[n as usize..];
    }
    true
}

/// Reads up to `buf.len()` bytes; `None` when the descriptor fails.
fn read_some(fd: isize, buf: &mut [u8]) -> Option<usize> {
    let mut n = 0;
    while n < buf.len() {
        let k = sys(SYS_READ, [fd as usize, buf[n..].as_mut_ptr() as usize, buf.len() - n, 0, 0]);
        if k < 0 {
            return None;
        }
        if k == 0 {
            break;
        }
        n += k as usize;
    }
    Some(n)
}

/// The start of a file as text; empty when it cannot be read.
fn read_text<'a>(path: &str, buf: &'a mut [u8]) -> &'a str {
    let fd = open(path, 0);
    if fd < 0 {
        return "";
    }
    let n = read_some(fd, buf).unwrap_or(0);
    close(fd);
    core::str::from_utf8(&buf[..n]).unwrap_or("")
}

fn mount(source: &CStr<64>, target: &str, fstype: Option<&CStr<64>>, flags: usize) -> isize {
    sys(SYS_MOUNT, [source.ptr(), c(target).ptr(), fstype.map_or(0, |f| f.ptr()), flags, 0])
}

fn sleep(seconds: usize) {
    let ts = [seconds, 0usize];
    sys(SYS_NANOSLEEP, [ts.as_ptr() as usize, 0, 0, 0, 0]);
}

/// Writes everything out and restarts the phone.
fn restart() -> ! {
    sys(SYS_SYNC, [0; 5]);
    sys(SYS_UMOUNT2, [c(NVDATA).ptr(), 0, 0, 0, 0]);
    // LINUX_REBOOT_MAGIC1, MAGIC2, CMD_RESTART.
    sys(SYS_REBOOT, [0xfee1_dead, 672_274_793, 0x0123_4567, 0, 0]);
    // Not reached as PID 1 of the phone.
    sys(SYS_EXIT, [1, 0, 0, 0, 0]);
    loop {
        sleep(3600);
    }
}

// --------------------------------------------------------------- messages

static mut KMSG: isize = -1;

/// One line in the kernel log (`/dev/kmsg`), or on standard error before
/// that is open.
fn say(parts: &[&str]) {
    let mut line = [0u8; 200];
    let mut n = 0;
    for p in ["dphone-early: "].iter().chain(parts) {
        let take = p.len().min(line.len() - 1 - n);
        line[n..n + take].copy_from_slice(&p.as_bytes()[..take]);
        n += take;
    }
    line[n] = b'\n';
    // SAFETY: one thread; the value is a plain descriptor number.
    let fd = unsafe { KMSG };
    write_all(if fd >= 0 { fd } else { 2 }, &line[..n + 1]);
}

/// A small number as text, for messages.
fn digits(n: u32, buf: &mut [u8; 10]) -> &str {
    let mut i = buf.len();
    let mut v = n;
    loop {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    core::str::from_utf8(&buf[i..]).unwrap_or("?")
}

// ------------------------------------------------------------------- ctrl

#[repr(C)]
#[derive(Default)]
struct MtdInfoUser {
    kind: u8,
    flags: u32,
    size: u32,
    erasesize: u32,
    writesize: u32,
    oobsize: u32,
    padding: u64,
}

// _IOR('M', 1, struct mtd_info_user), _IOW('M', 2, struct erase_info_user),
// _IOW('M', 11, loff_t).
const MEMGETINFO: usize = 0x8020_4d01;
const MEMERASE: usize = 0x4008_4d02;
const MEMGETBADBLOCK: usize = 0x4008_4d0b;
const _: () = assert!(size_of::<MtdInfoUser>() == 32);

/// The `ctrl` partition through its MTD character device.
struct CtrlDev {
    fd: isize,
    geometry: Geometry,
}

impl CtrlDev {
    fn open(proc_mtd: &str) -> Option<CtrlDev> {
        let index = boot::mtd_index(proc_mtd, "ctrl")?;
        let path: CStr<64> = CStr::new(&["/dev/mtd", digits(index, &mut [0; 10])])?;
        let fd = sys(SYS_OPEN, [path.ptr(), O_RDWR | O_CLOEXEC, 0, 0, 0]);
        if fd < 0 {
            return None;
        }
        let mut info = MtdInfoUser::default();
        if sys(SYS_IOCTL, [fd as usize, MEMGETINFO, &mut info as *mut _ as usize, 0, 0]) < 0 {
            close(fd);
            return None;
        }
        Some(CtrlDev {
            fd,
            geometry: Geometry { size: info.size as u64, erase_size: info.erasesize, write_size: info.writesize.max(1) },
        })
    }

    fn seek(&mut self, offset: u64) -> Result<(), ()> {
        if sys(SYS_LSEEK, [self.fd as usize, offset as usize, 0, 0, 0]) < 0 { Err(()) } else { Ok(()) }
    }
}

impl Flash for CtrlDev {
    type Error = ();

    fn geometry(&self) -> Geometry {
        self.geometry
    }

    fn is_bad(&mut self, offset: u64) -> Result<bool, ()> {
        let mut off = offset as i64;
        match sys(SYS_IOCTL, [self.fd as usize, MEMGETBADBLOCK, &mut off as *mut _ as usize, 0, 0]) {
            r if r < 0 => Err(()),
            r => Ok(r > 0),
        }
    }

    fn read(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), ()> {
        self.seek(offset)?;
        if read_some(self.fd, buf) == Some(buf.len()) { Ok(()) } else { Err(()) }
    }

    fn erase(&mut self, offset: u64) -> Result<(), ()> {
        let mut e = [offset as u32, self.geometry.erase_size];
        if sys(SYS_IOCTL, [self.fd as usize, MEMERASE, e.as_mut_ptr() as usize, 0, 0]) < 0 { Err(()) } else { Ok(()) }
    }

    /// Whole pages only, which is all the `ctrl` writer asks for.
    fn write(&mut self, offset: u64, data: &[u8]) -> Result<(), ()> {
        if !data.len().is_multiple_of(self.geometry.write_size as usize) {
            return Err(());
        }
        self.seek(offset)?;
        if write_all(self.fd, data) { Ok(()) } else { Err(()) }
    }
}

/// Room for two erase blocks of up to 256 KiB (the phones' are 128 KiB).
static mut SCRATCH: [u8; 2 * 256 * 1024] = [0; 2 * 256 * 1024];

// ------------------------------------------------------------ the sequence

const NVDATA: &str = "/nvdata";
const RECORD: &str = "/nvdata/open/trial";
const RECORD_TMP: &str = "/nvdata/open/trial.tmp";

/// A trial in progress: the record as counted, and `ctrl`, opened before
/// the root is switched so that the way back does not need `/dev`.
struct OnTrial {
    trial: Trial,
    ctrl: CtrlDev,
    repl: dphone_ctrl::Replication,
}

/// Stores the record: a new file, synced, renamed over the old one.
fn store(trial: &Trial) -> bool {
    let mut text = [0u8; TEXT_MAX];
    let n = trial.format(&mut text);
    let fd = open(RECORD_TMP, O_WRONLY | O_CREAT | O_TRUNC);
    if fd < 0 {
        return false;
    }
    let ok = write_all(fd, &text[..n]) && sys(SYS_FSYNC, [fd as usize, 0, 0, 0, 0]) == 0;
    close(fd);
    ok && sys(SYS_RENAME, [c(RECORD_TMP).ptr(), c(RECORD).ptr(), 0, 0, 0]) == 0
}

/// Selects the previous bank, removes the record, and restarts.  Returns
/// only when the bank could not be selected: the image on trial is then
/// all there is to boot.
fn go_back(t: &mut OnTrial, why: &str) {
    let previous = t.trial.previous_bank as usize;
    say(&[why, ": going back to bank ", if previous == 0 { "A" } else { "B" }]);
    let Ok(scan) = dphone_ctrl::scan(&mut t.ctrl) else {
        say(&["ctrl cannot be read"]);
        return;
    };
    let Some(current) = scan.record else {
        say(&["ctrl holds no valid record"]);
        return;
    };
    if !record::side_valid(&current, previous) {
        say(&["the previous bank has no valid header in ctrl"]);
        return;
    }
    let wanted = record::with_active(&current, previous as u32);
    // SAFETY: one thread, and this is the only use of the buffer.
    let scratch = unsafe { &mut *addr_of_mut!(SCRATCH) };
    if dphone_ctrl::write(&mut t.ctrl, &wanted, t.repl, scratch).is_err() {
        say(&["ctrl could not be rewritten"]);
        // Whatever boot2 reads now, the record must keep counting.
        return;
    }
    sys(SYS_UNLINK, [c(RECORD).ptr(), 0, 0, 0, 0]);
    say(&["previous bank selected; restarting"]);
    restart()
}

/// Nothing left to try in this boot.
fn give_up(trial: &mut Option<OnTrial>, why: &str) -> ! {
    if let Some(t) = trial {
        go_back(t, why);
    }
    say(&[why, "; restarting in 10 s"]);
    sleep(10);
    restart()
}

/// Entered from the C library's start-up code with the kernel's arguments
/// and environment.
#[unsafe(no_mangle)]
unsafe extern "C" fn main(argc: i32, argv: *const *const c_char, envp: *const *const c_char) -> i32 {
    let argc = argc.max(0) as usize;

    mount(&c("proc"), "/proc", Some(&c("proc")), 0);
    mount(&c("devtmpfs"), "/dev", Some(&c("devtmpfs")), 0);
    let kmsg = open("/dev/kmsg", O_WRONLY);
    // SAFETY: one thread.
    unsafe { KMSG = kmsg };

    let (mut b1, mut b2) = ([0u8; 4096], [0u8; 4096]);
    let cmdline = read_text("/proc/cmdline", &mut b1);
    let proc_mtd = read_text("/proc/mtd", &mut b2);

    // Steps 1 to 3: the trial.
    let mut on_trial: Option<OnTrial> = None;
    let mut nvdata = false;
    let running = boot::running_bank(cmdline, proc_mtd);
    if running.is_some() {
        nvdata = mount(&c("mtd:nvdata"), NVDATA, Some(&c("jffs2")), MS_NOATIME | MS_NOSUID | MS_NODEV) == 0;
        if !nvdata {
            say(&["nvdata does not mount; the trial record cannot be read"]);
        }
        let mut text = [0u8; 256];
        let recorded = Trial::parse(read_text(RECORD, &mut text).as_bytes());
        let (trial, used_up) = match boot::step(running, recorded) {
            Step::Boot => (None, false),
            Step::Count(t) => (Some(t), false),
            Step::GoBack(t) => (Some(t), true),
        };
        if let Some(trial) = trial {
            say(&[
                "trial boot ",
                digits(trial.boots, &mut [0; 10]),
                " of ",
                digits(trial.max_boots, &mut [0; 10]),
            ]);
            if !store(&trial) {
                say(&["the trial record could not be written"]);
            }
            match CtrlDev::open(proc_mtd) {
                Some(ctrl) => {
                    let mut t = OnTrial { trial, ctrl, repl: boot::replication(cmdline) };
                    if used_up {
                        go_back(&mut t, "trial boots used up");
                    }
                    on_trial = Some(t);
                }
                None => say(&["ctrl cannot be opened; there is no way back from this boot"]),
            }
        }
    }

    // Step 4: the root named on the command line, as the kernel mounts it.
    let Some(root) = boot::param(cmdline, "root=").and_then(|r| CStr::<64>::new(&[r])) else {
        give_up(&mut on_trial, "no root= on the command line")
    };
    let fstype = CStr::<64>::new(&[boot::param(cmdline, "rootfstype=").unwrap_or("jffs2")]).unwrap_or(c("jffs2"));
    let flags = if boot::read_only(cmdline) { MS_RDONLY } else { 0 };
    if mount(&root, "/root", Some(&fstype), flags) != 0 {
        give_up(&mut on_trial, "the root file system does not mount");
    }
    let init = boot::param(cmdline, "init=").unwrap_or("/sbin/init");
    let Some(init_in_root) = CStr::<64>::new(&["/root", init]) else {
        give_up(&mut on_trial, "init= is too long")
    };
    // X_OK
    if sys(SYS_ACCESS, [init_in_root.ptr(), 1, 0, 0, 0]) != 0 {
        give_up(&mut on_trial, "the root file system has no init to run");
    }

    // nvdata goes along, mounted; the init role finds it at /nvdata.
    if nvdata && sys(SYS_MOUNT, [c(NVDATA).ptr(), c("/root/nvdata").ptr(), 0, MS_MOVE, 0]) != 0 {
        sys(SYS_UMOUNT2, [c(NVDATA).ptr(), 0, 0, 0, 0]);
    }
    sys(SYS_UMOUNT2, [c("/proc").ptr(), MNT_DETACH, 0, 0, 0]);
    sys(SYS_UMOUNT2, [c("/dev").ptr(), MNT_DETACH, 0, 0, 0]);
    // The root becomes "/": the rootfs under it cannot be unmounted, and
    // what it holds is this program and four empty directories.
    if sys(SYS_CHDIR, [c("/root").ptr(), 0, 0, 0, 0]) != 0
        || sys(SYS_MOUNT, [c(".").ptr(), c("/").ptr(), 0, MS_MOVE, 0]) != 0
        || sys(SYS_CHROOT, [c(".").ptr(), 0, 0, 0, 0]) != 0
        || sys(SYS_CHDIR, [c("/").ptr(), 0, 0, 0, 0]) != 0
    {
        give_up(&mut on_trial, "cannot switch to the root file system");
    }

    // The kernel's arguments and environment, plus the mark that this boot
    // has been counted.
    let init_path: CStr<64> = CStr::new(&[init]).unwrap_or(c("/sbin/init"));
    let mut args = [null::<c_char>(); 34];
    args[0] = init_path.0.as_ptr() as *const c_char;
    let mut n = 1;
    while n < argc && n < args.len() - 1 {
        // SAFETY: `n < argc`.
        args[n] = unsafe { *argv.add(n) };
        n += 1;
    }
    let mut env = [null::<c_char>(); 34];
    env[0] = c"DPHONE_EARLY=1".as_ptr();
    let mut n = 1;
    // SAFETY: the list ends with a null pointer.
    while n < env.len() - 1 && !unsafe { *envp.add(n - 1) }.is_null() {
        // SAFETY: as above.
        env[n] = unsafe { *envp.add(n - 1) };
        n += 1;
    }
    sys(SYS_EXECVE, [init_path.ptr(), args.as_ptr() as usize, env.as_ptr() as usize, 0, 0]);
    // Still here: the init could not be executed.  /dev is gone with the
    // old root; `ctrl` and the kernel log were opened before.
    sys(SYS_MKDIR, [c("/nvdata").ptr(), 0, 0, 0, 0]);
    give_up(&mut on_trial, "the root file system's init cannot be executed")
}
