//! Mount test in a VM: the kernel's JFFS2 driver mounts an image written to
//! a simulated NAND with the phones' geometry (2048-byte pages, 64 bytes
//! OOB, 128 KiB erase blocks), and what it sees is compared with our
//! reader's view of the same image.  Needs no root: QEMU boots the host's
//! distribution kernel with an initramfs made here.
//!
//! ```text
//! cargo build --release -p jffs2-image --example vm_init --target x86_64-unknown-linux-musl
//! cargo run --release -p jffs2-image --example vm_mount_test -- \
//!     --init target/x86_64-unknown-linux-musl/release/examples/vm_init \
//!     [--image FILE.jffs2 | --dir ROOTFS_DIR] \
//!     [--kernel /boot/vmlinuz-linux] [--modules /usr/lib/modules/$(uname -r)]
//! ```
//!
//! Without `--image` or `--dir` a synthetic tree with every node kind is
//! used.  Exit status 0 means the kernel's listing matched and the kernel
//! log had no JFFS2 complaints.

use jffs2_image::{Kind, Options, Tree, read_image, write_image};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf29ce484222325u64, |h, &b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}

fn expected(tree: &Tree) -> Vec<String> {
    // The guest lists depth-first in name order; produce the same order.
    fn rec(
        children: &BTreeMap<&str, Vec<(&str, &str)>>,
        tree: &Tree,
        dir: &str,
        out: &mut Vec<String>,
    ) {
        let Some(list) = children.get(dir) else {
            return;
        };
        for (_, path) in list {
            let e = &tree.entries[*path];
            let own = format!("{}:{}", e.uid, e.gid);
            let perm = e.mode;
            out.push(match &e.kind {
                Kind::Dir => format!("d {perm:o} {own} {path}"),
                Kind::File(d) => format!("f {perm:o} {own} {} {:016x} {path}", d.len(), fnv(d)),
                Kind::Symlink(t) => format!("l {perm:o} {own} {:016x} {path}", fnv(t)),
                Kind::CharDev { major, minor } => {
                    format!("c {perm:o} {own} {major}:{minor} {path}")
                }
                Kind::BlockDev { major, minor } => {
                    format!("b {perm:o} {own} {major}:{minor} {path}")
                }
                Kind::Fifo => format!("p {perm:o} {own} {path}"),
                Kind::Socket => format!("s {perm:o} {own} {path}"),
            });
            if e.kind == Kind::Dir {
                rec(children, tree, path, out);
            }
        }
    }
    let mut children: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
    for path in tree.entries.keys() {
        let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
        children.entry(parent).or_default().push((name, path));
    }
    for l in children.values_mut() {
        l.sort();
    }
    let mut out = Vec::new();
    rec(&children, tree, "", &mut out);
    out
}

fn synthetic() -> Tree {
    let t0 = 1_700_000_000;
    let mut t = Tree::new();
    let mut x = 12345u32;
    let mut noise = |n: usize| -> Vec<u8> {
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                (x >> 24) as u8
            })
            .collect()
    };
    t.file(
        "etc/passwd",
        b"root:x:0:0:root:/root:/bin/sh\n".to_vec(),
        0o644,
        t0,
    )
    .unwrap();
    t.file("etc/empty", Vec::new(), 0o600, t0).unwrap();
    t.symlink("sbin/init", "../bin/dphone", t0).unwrap();
    t.char_dev("dev/console", 5, 1, 0o600, t0).unwrap();
    t.char_dev("dev/big", 300, 70000, 0o600, t0).unwrap();
    t.insert(
        "dev/mtdblock3",
        jffs2_image::Entry {
            kind: Kind::BlockDev {
                major: 31,
                minor: 3,
            },
            mode: 0o660,
            uid: 0,
            gid: 6,
            mtime: t0,
        },
    )
    .unwrap();
    t.insert(
        "run/fifo",
        jffs2_image::Entry {
            kind: Kind::Fifo,
            mode: 0o644,
            uid: 1000,
            gid: 1000,
            mtime: t0,
        },
    )
    .unwrap();
    t.insert(
        "run/sock",
        jffs2_image::Entry {
            kind: Kind::Socket,
            mode: 0o755,
            uid: 0,
            gid: 0,
            mtime: t0,
        },
    )
    .unwrap();
    t.dir("setuid-dir", 0o2775, t0).unwrap();
    t.file(
        "bin/text",
        b"The quick brown fox jumps over the lazy dog.\n".repeat(20000),
        0o4755,
        t0,
    )
    .unwrap();
    t.file("bin/noise", noise(3_000_001), 0o755, t0).unwrap();
    t.file("bin/zeros", vec![0u8; 1_234_567], 0o644, t0)
        .unwrap();
    let mut mixed = noise(2_000_000);
    for (i, b) in mixed.iter_mut().enumerate() {
        if (i / 5000) % 2 == 0 {
            *b = b'a' + (i % 7) as u8;
        }
    }
    t.file("usr/share/mixed", mixed, 0o644, t0).unwrap();
    for i in 0..500 {
        let data = noise(i * 53);
        t.file(&format!("usr/lib/many/file-{i:03}"), data, 0o644, t0)
            .unwrap();
    }
    t
}

fn cpio_entry(out: &mut Vec<u8>, ino: u32, name: &str, mode: u32, rdev: (u32, u32), data: &[u8]) {
    write!(
        out,
        "070701{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}",
        ino,
        mode,
        0,
        0,
        1,
        0,
        data.len(),
        0,
        0,
        rdev.0,
        rdev.1,
        name.len() + 1,
        0
    )
    .unwrap();
    out.extend_from_slice(name.as_bytes());
    out.push(0);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
    out.extend_from_slice(data);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

/// Modules to load, dependencies first, from `modules.dep`.
fn module_order(moddir: &Path, wanted: &[&str]) -> Vec<String> {
    let dep = std::fs::read_to_string(moddir.join("modules.dep")).expect("modules.dep");
    let mut deps: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for line in dep.lines() {
        if let Some((m, d)) = line.split_once(':') {
            deps.insert(m, d.split_whitespace().collect());
        }
    }
    fn visit<'a>(m: &'a str, deps: &BTreeMap<&'a str, Vec<&'a str>>, out: &mut Vec<String>) {
        if out.iter().any(|o| o == m) {
            return;
        }
        // modules.dep lists the nearest dependency first; load in reverse.
        for d in deps
            .get(m)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
            .iter()
            .rev()
        {
            visit(d, deps, out);
        }
        out.push(m.to_owned());
    }
    let mut out = Vec::new();
    for w in wanted {
        let key = deps.keys().find(|k| {
            let base = k.rsplit('/').next().unwrap();
            base.split('.').next() == Some(w)
        });
        visit(
            key.unwrap_or_else(|| panic!("module {w} not found in modules.dep")),
            &deps,
            &mut out,
        );
    }
    out
}

fn read_module(path: &Path) -> Vec<u8> {
    let raw = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    if path.extension().is_some_and(|e| e == "zst") {
        let mut out = Vec::new();
        ruzstd::decoding::StreamingDecoder::new(&raw[..])
            .expect("zstd")
            .read_to_end(&mut out)
            .expect("zstd");
        out
    } else if path.extension().is_some_and(|e| e == "ko") {
        raw
    } else {
        panic!("{}: unsupported module compression", path.display());
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut init, mut image_path, mut dir) = (None, None, None);
    let mut kernel = PathBuf::from("/boot/vmlinuz-linux");
    let mut modules = None;
    let mut keep_log = None;
    while let Some(a) = args.next() {
        let mut val = || PathBuf::from(args.next().expect("option needs a value"));
        match a.as_str() {
            "--init" => init = Some(val()),
            "--image" => image_path = Some(val()),
            "--dir" => dir = Some(val()),
            "--kernel" => kernel = val(),
            "--modules" => modules = Some(val()),
            "--log" => keep_log = Some(val()),
            _ => panic!("unknown argument {a}"),
        }
    }
    let init = init.expect(
        "--init PATH (the vm_init example built for x86_64-unknown-linux-musl) is required",
    );
    let modules = modules.unwrap_or_else(|| {
        let rel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap();
        PathBuf::from(format!("/usr/lib/modules/{}", rel.trim()))
    });

    let image = match (&image_path, &dir) {
        (Some(p), _) => std::fs::read(p).unwrap(),
        (None, Some(d)) => write_image(
            &Tree::from_dir(d, false, Some(1_700_000_000)).unwrap(),
            &Options::default(),
        )
        .unwrap(),
        (None, None) => write_image(&synthetic(), &Options::default()).unwrap(),
    };
    let (tree, stats) = read_image(&image, 0x20000).unwrap();
    let want = expected(&tree);
    eprintln!(
        "image: {} bytes, {} entries, {stats:?}",
        image.len(),
        want.len()
    );

    // Initramfs.
    let mut cpio = Vec::new();
    let mut ino = 1;
    let mut add = |cpio: &mut Vec<u8>, name: &str, mode: u32, rdev: (u32, u32), data: &[u8]| {
        ino += 1;
        cpio_entry(cpio, ino, name, mode, rdev, data);
    };
    add(&mut cpio, "dev", 0o040755, (0, 0), &[]);
    add(&mut cpio, "dev/console", 0o020600, (5, 1), &[]);
    add(
        &mut cpio,
        "init",
        0o100755,
        (0, 0),
        &std::fs::read(&init).unwrap(),
    );
    let mut list = String::new();
    for (i, m) in module_order(&modules, &["nandsim", "jffs2"])
        .iter()
        .enumerate()
    {
        let name = format!("m{i}.ko");
        add(
            &mut cpio,
            &name,
            0o100644,
            (0, 0),
            &read_module(&modules.join(m)),
        );
        // 0x20 0xa1 0x00 0x15: 128 MiB, 2048-byte pages, 64 bytes OOB, 128 KiB blocks.
        let params = if m.contains("nandsim") {
            " first_id_byte=0x20 second_id_byte=0xa1 third_id_byte=0x00 fourth_id_byte=0x15"
        } else {
            ""
        };
        list.push_str(&format!("/{name}{params}\n"));
    }
    add(&mut cpio, "modules.list", 0o100644, (0, 0), list.as_bytes());
    add(&mut cpio, "image.jffs2", 0o100644, (0, 0), &image);
    cpio_entry(&mut cpio, 0, "TRAILER!!!", 0, (0, 0), &[]);
    let tmp = std::env::temp_dir().join(format!("jffs2-vm-{}.cpio", std::process::id()));
    std::fs::write(&tmp, &cpio).unwrap();

    let mut cmd = Command::new("qemu-system-x86_64");
    if Path::new("/dev/kvm").exists() {
        cmd.args(["-enable-kvm", "-cpu", "host"]);
    }
    let output = cmd
        .args(["-m", "2048", "-nographic", "-no-reboot", "-kernel"])
        .arg(&kernel)
        .arg("-initrd")
        .arg(&tmp)
        .args(["-append", "console=ttyS0 loglevel=7 panic=-1"])
        .stdin(Stdio::null())
        .output()
        .expect("run qemu-system-x86_64");
    let _ = std::fs::remove_file(&tmp);
    let log = String::from_utf8_lossy(&output.stdout).replace('\r', "");
    if let Some(p) = keep_log {
        std::fs::write(p, log.as_bytes()).unwrap();
    }

    let mut failures = Vec::new();
    let mut got = Vec::new();
    let mut in_list = false;
    let mut ended = false;
    for line in log.lines() {
        if line.starts_with("VMTEST-BEGIN") {
            in_list = true;
        } else if line.starts_with("VMTEST-END") {
            in_list = false;
            ended = true;
        } else if line.starts_with("VMTEST-FAIL") {
            failures.push(line.to_owned());
        } else if line.starts_with("VMTEST-GEOMETRY") {
            eprintln!("{line}");
            if line != "VMTEST-GEOMETRY erase=131072 page=2048 oob=64 size=134217728" {
                failures.push(format!("unexpected simulated NAND geometry: {line}"));
            }
        } else if line.starts_with('[') {
            // Kernel log line.  A clean read-only mount of a good image prints
            // only the driver banner and the xattr summary.
            let lower = line.to_lowercase();
            if lower.contains("jffs2") {
                eprintln!("kernel: {line}");
                if !lower.contains("version 2.2")
                    && !lower.contains("complete building xattr subsystem")
                {
                    failures.push(format!("kernel JFFS2 message: {line}"));
                }
            }
        } else if in_list {
            got.push(line.to_owned());
        }
    }
    if !ended {
        failures.push("guest did not finish its listing".into());
        let tail: Vec<_> = log.lines().rev().take(25).collect();
        for l in tail.iter().rev() {
            eprintln!("| {l}");
        }
    } else if got != want {
        failures.push(format!(
            "listing differs: kernel saw {} entries, expected {}",
            got.len(),
            want.len()
        ));
        for (g, w) in got.iter().zip(&want).filter(|(g, w)| g != w).take(10) {
            eprintln!("kernel:   {g}\nexpected: {w}");
        }
    }
    if failures.is_empty() {
        println!(
            "PASS: kernel JFFS2 mounted the image on simulated NAND; all {} entries match",
            want.len()
        );
    } else {
        for f in &failures {
            println!("FAIL: {f}");
        }
        std::process::exit(1);
    }
}
