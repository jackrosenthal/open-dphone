//! Kernel port maintenance.
//!
//! The port is kept in the repository as:
//!
//! - `kernel/patches/NNNN-<group>.patch`: changes to files that exist in
//!   mainline v7.3, one patch per subsystem group, applied with `patch -p1`;
//! - `kernel/files/`: files the port adds, at their kernel-tree paths;
//! - `kernel/configs/`: the defconfigs (they belong in `arch/arm/configs/`).
//!
//! `kernel-sync` regenerates all three from a git checkout of v7.3 whose
//! working tree holds the port.  `kernel-prepare` is the inverse, and
//! `kernel-build` runs the kernel's own build and collects the results.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::{Result, repo_root};

const CONFIG_DIR: &str = "arch/arm/configs/";
const DTS_DIR: &str = "arch/arm/boot/dts/broadcom";
const DEFCONFIG: &str = "bcmring_tiny_defconfig";

/// The kernel slot is 2 MiB; every bad block in it costs 128 KiB.  The build
/// fails unless the image leaves room for this many bad blocks.
const SLOT: u64 = 0x20_0000;
const BLOCK: u64 = 0x2_0000;
const BAD_BLOCK_MARGIN: u64 = 1;

/// Model, DTB base name.
pub const MODELS: &[(&str, &str)] = &[
    ("d40", "bcm11170-digium-d40"),
    ("d45", "bcm11109-digium-d45"),
    ("d50", "bcm11170-digium-d50"),
    ("d60", "bcm11189-digium-d60"),
    ("d62", "bcm11188-digium-d62"),
    ("d65", "bcm11188-digium-d65"),
    ("d70", "bcm11109-digium-d70"),
];

/// Path prefix to patch group, first match wins.  Paths that match nothing
/// are grouped by their first two components.
const GROUPS: &[(&str, &str)] = &[
    ("Documentation/", "docs"),
    ("arch/arm/", "arch-arm"),
    ("drivers/char/hw_random/", "hwrng"),
    ("drivers/clk/", "clk"),
    ("drivers/dma/", "dma"),
    ("include/linux/dma/", "dma"),
    ("drivers/gpio/", "gpio"),
    ("drivers/gpu/", "drm"),
    ("drivers/input/", "input"),
    ("drivers/irqchip/", "irqchip"),
    ("drivers/leds/", "leds"),
    ("drivers/mtd/", "mtd"),
    ("drivers/net/", "net"),
    ("include/linux/brcmphy.h", "net"),
    ("drivers/remoteproc/", "remoteproc"),
    ("drivers/video/", "backlight"),
    ("drivers/watchdog/", "watchdog"),
    ("sound/", "sound"),
];

fn group_of(path: &str) -> String {
    for (prefix, group) in GROUPS {
        if path.starts_with(prefix) {
            return (*group).to_string();
        }
    }
    path.split('/').take(2).collect::<Vec<_>>().join("-")
}

fn git(tree: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(tree)
        .args(["-c", "core.quotepath=off"])
        .args(args)
        .stderr(Stdio::inherit())
        .output()?;
    if !out.status.success() {
        return Err(format!("git {} failed: {}", args.join(" "), out.status).into());
    }
    Ok(out.stdout)
}

fn lines(bytes: &[u8]) -> Result<Vec<String>> {
    Ok(std::str::from_utf8(bytes)?
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

fn kernel_dir() -> PathBuf {
    repo_root().join("kernel")
}

fn one_arg<'a>(args: &'a [String], what: &str) -> Result<&'a Path> {
    match args {
        [a] => Ok(Path::new(a)),
        _ => Err(format!("expected exactly one argument: {what}").into()),
    }
}

fn remove_dir_if_exists(p: &Path) -> Result<()> {
    match fs::remove_dir_all(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn copy_file(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(from, to).map_err(|e| format!("copy {} -> {}: {e}", from.display(), to.display()))?;
    Ok(())
}

/// All regular files under `dir`, as paths relative to it, sorted.
fn walk(dir: &Path) -> Result<Vec<String>> {
    fn rec(base: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                rec(base, &path, out)?;
            } else {
                let rel = path.strip_prefix(base)?.to_str().ok_or("non-UTF-8 path")?;
                out.push(rel.to_string());
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    if dir.exists() {
        rec(dir, dir, &mut out)?;
    }
    out.sort();
    Ok(out)
}

pub fn sync(args: &[String]) -> Result<()> {
    let tree = one_arg(args, "the kernel working tree")?;
    let kdir = kernel_dir();

    let modified = lines(&git(tree, &["diff", "--name-only", "--no-renames", "HEAD"])?)?;
    let added = lines(&git(tree, &["ls-files", "--others", "--exclude-standard"])?)?;

    // Patches, one per group.
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for path in &modified {
        if !tree.join(path).exists() {
            return Err(format!("{path} is deleted in the tree; deletions are not supported").into());
        }
        groups.entry(group_of(path)).or_default().push(path.clone());
    }
    let patches = kdir.join("patches");
    remove_dir_if_exists(&patches)?;
    fs::create_dir_all(&patches)?;
    for (n, (group, files)) in groups.iter().enumerate() {
        let mut cmd = vec![
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-renames",
            "--full-index",
            "HEAD",
            "--",
        ];
        cmd.extend(files.iter().map(String::as_str));
        let diff = git(tree, &cmd)?;
        let name = format!("{:04}-{group}.patch", n + 1);
        fs::write(patches.join(&name), diff)?;
        println!("patches/{name}: {} file(s)", files.len());
    }

    // New files; the defconfigs go to kernel/configs.
    let files_dir = kdir.join("files");
    let configs_dir = kdir.join("configs");
    remove_dir_if_exists(&files_dir)?;
    remove_dir_if_exists(&configs_dir)?;
    let (mut nfiles, mut nconfigs) = (0, 0);
    for path in &added {
        let dest = match path.strip_prefix(CONFIG_DIR) {
            Some(name) => {
                nconfigs += 1;
                configs_dir.join(name)
            }
            None => {
                nfiles += 1;
                files_dir.join(path)
            }
        };
        copy_file(&tree.join(path), &dest)?;
    }
    println!("files/: {nfiles} file(s); configs/: {nconfigs} file(s)");
    Ok(())
}

pub fn prepare(args: &[String]) -> Result<()> {
    let tree = one_arg(args, "a pristine v7.3 tree")?;
    let kdir = kernel_dir();

    let mut patches: Vec<PathBuf> = fs::read_dir(kdir.join("patches"))?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<_>>()?;
    patches.retain(|p| p.extension().is_some_and(|e| e == "patch"));
    patches.sort();
    for patch in &patches {
        let status = Command::new("patch")
            .args(["-p1", "--no-backup-if-mismatch", "-s", "-d"])
            .arg(tree)
            .arg("-i")
            .arg(patch)
            .status()?;
        if !status.success() {
            return Err(format!("{} did not apply", patch.display()).into());
        }
    }
    let files_dir = kdir.join("files");
    let files = walk(&files_dir)?;
    for rel in &files {
        copy_file(&files_dir.join(rel), &tree.join(rel))?;
    }
    let configs_dir = kdir.join("configs");
    let configs = walk(&configs_dir)?;
    for rel in &configs {
        copy_file(&configs_dir.join(rel), &tree.join(CONFIG_DIR).join(rel))?;
    }
    println!(
        "applied {} patch(es), {} file(s), {} config(s)",
        patches.len(),
        files.len(),
        configs.len()
    );
    Ok(())
}

fn make(tree: &Path, obj: &Path, targets: &[&str]) -> Result<()> {
    let jobs = std::thread::available_parallelism().map_or(4, |n| n.get());
    let cross = std::env::var("CROSS_COMPILE").unwrap_or_else(|_| "arm-none-eabi-".to_string());
    let status = Command::new("make")
        .arg("-C")
        .arg(tree)
        .arg(format!("O={}", obj.display()))
        .arg("ARCH=arm")
        .arg(format!("CROSS_COMPILE={cross}"))
        // Reproducible banner; the Nix build sets the same.
        .arg("KBUILD_BUILD_USER=dphone")
        .arg("KBUILD_BUILD_HOST=open-dphone")
        // Fixed release string: without this the kernel appends "-dirty" or
        // "+" depending on the state of a git checkout, and the module
        // directory name would differ between a checkout and a tarball.
        .arg("LOCALVERSION=")
        .arg(format!("-j{jobs}"))
        .args(targets)
        .status()?;
    if !status.success() {
        return Err(format!("make {} failed", targets.join(" ")).into());
    }
    Ok(())
}

pub fn build(args: &[String]) -> Result<()> {
    let [tree, out] = args else {
        return Err("expected <tree> <out>".into());
    };
    let tree = fs::canonicalize(tree)?;
    fs::create_dir_all(out)?;
    let out = fs::canonicalize(out)?;
    let obj = out.join("obj");
    fs::create_dir_all(&obj)?;

    make(&tree, &obj, &[DEFCONFIG])?;
    make(&tree, &obj, &["zImage", "dtbs", "modules"])?;

    // Modules: /lib/modules/<release>/... as depmod-free flat install.
    let modroot = out.join("modules");
    remove_dir_if_exists(&modroot)?;
    let install = format!("INSTALL_MOD_PATH={}", modroot.display());
    make(
        &tree,
        &obj,
        &["modules_install", &install, "INSTALL_MOD_STRIP=1", "DEPMOD=true"],
    )?;

    // Per model: zImage followed by the DTB, zero-padded to a multiple of 4.
    let zimage = fs::read(obj.join("arch/arm/boot/zImage"))?;
    let images = out.join("images");
    fs::create_dir_all(&images)?;
    let limit = SLOT - BAD_BLOCK_MARGIN * BLOCK;
    let mut too_big = Vec::new();
    for (model, dtb_name) in MODELS {
        let dtb_path = obj.join(DTS_DIR).join(format!("{dtb_name}.dtb"));
        let dtb = fs::read(&dtb_path).map_err(|e| format!("{}: {e}", dtb_path.display()))?;
        let mut image = zimage.clone();
        image.extend_from_slice(&dtb);
        while image.len() % 4 != 0 {
            image.push(0);
        }
        let len = image.len() as u64;
        fs::write(images.join(format!("{model}-zImage-dtb.raw")), &image)?;
        fs::write(images.join(format!("{model}.dtb")), &dtb)?;
        println!(
            "{model}: {len} bytes, {} spare in the slot, {} with {BAD_BLOCK_MARGIN} bad block(s)",
            SLOT as i64 - len as i64,
            limit as i64 - len as i64
        );
        if len > limit {
            too_big.push(*model);
        }
    }
    fs::copy(obj.join("arch/arm/boot/zImage"), images.join("zImage"))?;
    fs::copy(obj.join(".config"), out.join("config"))?;
    if !too_big.is_empty() {
        return Err(format!(
            "image too large for the 2 MiB slot less {BAD_BLOCK_MARGIN} bad block(s): {}",
            too_big.join(", ")
        )
        .into());
    }
    Ok(())
}
