//! Guest side of the VM mount test (see `vm_mount_test.rs`): runs as
//! `/init` in an initramfs on an x86_64 kernel.  Loads the modules listed in
//! `/modules.list`, writes `/image.jffs2` to the simulated NAND the way the
//! stock updater does, mounts it read-only, and prints one line per entry.
//!
//! Build: `cargo build --release -p jffs2-image --example vm_init
//! --target x86_64-unknown-linux-musl`.  Uses only `std` and three raw
//! system calls, so it has no dependencies.

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod guest {
    use std::ffi::CString;
    use std::io::Read;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{FileExt, FileTypeExt, MetadataExt};
    use std::path::Path;

    unsafe fn syscall5(n: usize, a: usize, b: usize, c: usize, d: usize, e: usize) -> isize {
        let ret: isize;
        unsafe {
            std::arch::asm!("syscall", inlateout("rax") n as isize => ret, in("rdi") a, in("rsi") b,
                in("rdx") c, in("r10") d, in("r8") e, lateout("rcx") _, lateout("r11") _);
        }
        ret
    }

    fn mount(src: &str, target: &str, fstype: &str, flags: usize) -> isize {
        let (s, t, f) = (
            CString::new(src).unwrap(),
            CString::new(target).unwrap(),
            CString::new(fstype).unwrap(),
        );
        unsafe {
            syscall5(
                165,
                s.as_ptr() as usize,
                t.as_ptr() as usize,
                f.as_ptr() as usize,
                flags,
                0,
            )
        }
    }

    fn load_module(path: &str, params: &str) -> isize {
        let f = std::fs::File::open(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let p = CString::new(params).unwrap();
        unsafe { syscall5(313, f.as_raw_fd() as usize, p.as_ptr() as usize, 0, 0, 0) }
    }

    fn power_off() -> ! {
        unsafe {
            syscall5(162, 0, 0, 0, 0, 0); // sync
            syscall5(169, 0xfee1dead, 672274793, 0x4321fedc, 0, 0);
        }
        loop {
            std::hint::spin_loop();
        }
    }

    /// FNV-1a, 64-bit: the content hash both sides print.
    fn fnv(data: &[u8]) -> u64 {
        data.iter().fold(0xcbf29ce484222325u64, |h, &b| {
            (h ^ b as u64).wrapping_mul(0x100000001b3)
        })
    }

    fn list(dir: &Path, prefix: &str, out: &mut Vec<String>) {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap())
            .collect();
        names.sort_by_key(|e| e.file_name());
        for e in names {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let md = std::fs::symlink_metadata(e.path()).unwrap();
            let ft = md.file_type();
            let perm = md.mode() & 0o7777;
            let own = format!("{}:{}", md.uid(), md.gid());
            let line = if ft.is_dir() {
                format!("d {perm:o} {own} {path}")
            } else if ft.is_file() {
                let mut data = Vec::new();
                std::fs::File::open(e.path())
                    .unwrap()
                    .read_to_end(&mut data)
                    .unwrap();
                format!("f {perm:o} {own} {} {:016x} {path}", data.len(), fnv(&data))
            } else if ft.is_symlink() {
                let t = std::fs::read_link(e.path()).unwrap();
                format!(
                    "l {perm:o} {own} {:016x} {path}",
                    fnv(t.as_os_str().as_bytes())
                )
            } else if ft.is_char_device() || ft.is_block_device() {
                let r = md.rdev();
                let major = ((r >> 32) & 0xffff_f000) | ((r >> 8) & 0xfff);
                let minor = ((r >> 12) & 0xffff_ff00) | (r & 0xff);
                format!(
                    "{} {perm:o} {own} {major}:{minor} {path}",
                    if ft.is_char_device() { 'c' } else { 'b' }
                )
            } else if ft.is_fifo() {
                format!("p {perm:o} {own} {path}")
            } else {
                format!("s {perm:o} {own} {path}")
            };
            out.push(line);
            if ft.is_dir() {
                list(&e.path(), &path, out);
            }
        }
    }

    pub fn main() {
        std::panic::set_hook(Box::new(|info| {
            println!("VMTEST-FAIL panic: {info}");
            power_off();
        }));
        for d in ["/dev", "/proc", "/sys", "/mnt"] {
            let _ = std::fs::create_dir(d);
        }
        mount("devtmpfs", "/dev", "devtmpfs", 0);
        mount("proc", "/proc", "proc", 0);
        mount("sysfs", "/sys", "sysfs", 0);
        for line in std::fs::read_to_string("/modules.list").unwrap().lines() {
            let (path, params) = line.split_once(' ').unwrap_or((line, ""));
            let r = load_module(path, params);
            if r != 0 {
                println!("VMTEST-FAIL finit_module {path}: errno {}", -r);
                power_off();
            }
        }
        // Geometry from sysfs, then write like the stock updater: per erase
        // block, without trailing 0xff, rounded up to a whole page.
        let num = |name: &str| -> usize {
            std::fs::read_to_string(format!("/sys/class/mtd/mtd0/{name}"))
                .unwrap()
                .trim()
                .parse()
                .unwrap()
        };
        let (erase, page, oob, size) = (
            num("erasesize"),
            num("writesize"),
            num("oobsize"),
            num("size"),
        );
        println!("VMTEST-GEOMETRY erase={erase} page={page} oob={oob} size={size}");
        let image = std::fs::read("/image.jffs2").unwrap();
        let dev = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/mtd0")
            .unwrap();
        for (i, block) in image.chunks(erase).enumerate() {
            let used = block.iter().rposition(|&b| b != 0xff).map_or(0, |p| p + 1);
            let len = used.div_ceil(page) * page;
            let mut buf = block[..used].to_vec();
            buf.resize(len, 0xff);
            dev.write_all_at(&buf, (i * erase) as u64).unwrap();
        }
        drop(dev);
        let r = mount("mtd0", "/mnt", "jffs2", 1 /* MS_RDONLY */);
        if r != 0 {
            println!("VMTEST-FAIL mount: errno {}", -r);
            power_off();
        }
        let mut lines = Vec::new();
        list(Path::new("/mnt"), "", &mut lines);
        println!("VMTEST-BEGIN");
        for l in &lines {
            println!("{l}");
        }
        println!("VMTEST-END {}", lines.len());
        power_off();
    }
}

fn main() {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    guest::main();
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    eprintln!("vm_init only runs as /init in the x86_64 test VM");
}
