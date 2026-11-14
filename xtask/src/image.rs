//! `cargo xtask dev-image`: assemble a development image for one model from
//! host-built parts, without Nix.  The release path is `nix build .#eff-<model>`;
//! this is the quick loop for bring-up, and it produces the same layout.
//!
//! A development image differs from a release image in three ways: SSH is on
//! (`/etc/dphone/dev`), one SSH public key is compiled into `dphone`, and
//! (with `--no-stock-writes`) stock-owned files in nvdata are never written.

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{Result, repo_root};

const TARGET: &str = "arm-unknown-linux-musleabi";

/// Applets linked to the coreutils multi-call binary; keep in step with
/// `nix/userland.nix`.
const APPLETS: &[&str] = &[
    "base64", "basename", "cat", "chmod", "chown", "cksum", "cp", "cut", "date", "dd", "df", "dirname", "du", "echo",
    "env", "expr", "false", "head", "hostname", "id", "ln", "ls", "md5sum", "mkdir", "mknod", "mktemp", "mv", "od",
    "printenv", "printf", "pwd", "readlink", "realpath", "rm", "rmdir", "seq", "sha256sum", "sleep", "sort", "stat",
    "stty", "sync", "tail", "tee", "test", "touch", "tr", "true", "truncate", "uname", "uniq", "wc", "yes",
];

/// System commands `dphone` provides under its own name.
const DPHONE_LINKS: &[&str] =
    &["mount", "umount", "insmod", "rmmod", "lsmod", "modprobe", "reboot", "poweroff", "halt", "dmesg"];

const USAGE: &str = "dev-image <model> --dsp-firmware <file> [--kernel-out <dir>] \
[--brush <file>] [--coreutils <file>] [--out <dir>] [--no-stock-writes] [--no-web] [--version 3_0_0]";

struct Options {
    model: String,
    dsp: PathBuf,
    kernel_out: PathBuf,
    brush: PathBuf,
    coreutils: PathBuf,
    out: PathBuf,
    no_stock_writes: bool,
    /// Do not rebuild the frontend (use web/dist as it is, or the
    /// placeholder page when it is absent).
    no_web: bool,
    version: String,
}

fn parse(args: &[String]) -> Result<Options> {
    let root = repo_root();
    let mut model = None;
    let mut dsp = None;
    let mut kernel_out = root.join("target/kernel-build/out");
    let mut brush = root.join("target/nix-spike/out-brush/bin/brush");
    let mut coreutils = root.join("target/nix-spike/out-uu/bin/coreutils");
    let mut out = None;
    let mut no_stock_writes = false;
    let mut no_web = false;
    let mut version = "3_0_0".to_string();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = || it.next().cloned().ok_or_else(|| format!("{a} needs a value\nusage: {USAGE}"));
        match a.as_str() {
            "--dsp-firmware" => dsp = Some(PathBuf::from(value()?)),
            "--kernel-out" => kernel_out = PathBuf::from(value()?),
            "--brush" => brush = PathBuf::from(value()?),
            "--coreutils" => coreutils = PathBuf::from(value()?),
            "--out" => out = Some(PathBuf::from(value()?)),
            "--version" => version = value()?,
            "--no-stock-writes" => no_stock_writes = true,
            "--no-web" => no_web = true,
            m if !m.starts_with('-') && model.is_none() => model = Some(m.to_ascii_lowercase()),
            other => return Err(format!("unknown argument '{other}'\nusage: {USAGE}").into()),
        }
    }
    let model = model.ok_or_else(|| format!("usage: {USAGE}"))?;
    if !["d40", "d45", "d50", "d60", "d62", "d65", "d70"].contains(&model.as_str()) {
        return Err(format!("unknown model '{model}'").into());
    }
    let dsp = dsp.ok_or("--dsp-firmware is required: the family's dspapp1107_lcc.bin from a stock rootfs")?;
    let out = out.unwrap_or_else(|| root.join("target/images").join(format!("{model}-dev")));
    Ok(Options { model, dsp, kernel_out, brush, coreutils, out, no_stock_writes, no_web, version })
}

fn run(cmd: &mut Command) -> Result<()> {
    let status = cmd.status().map_err(|e| format!("cannot run {:?}: {e}", cmd.get_program()))?;
    if !status.success() {
        return Err(format!("{:?} failed: {status}", cmd.get_program()).into());
    }
    Ok(())
}

fn install(from: &Path, to: &Path, mode: u32) -> Result<()> {
    if let Some(dir) = to.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::copy(from, to).map_err(|e| format!("copy {} to {}: {e}", from.display(), to.display()))?;
    fs::set_permissions(to, fs::Permissions::from_mode(mode))?;
    Ok(())
}

fn write(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, text)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o644))?;
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_tree(&entry.path(), &dest)?;
        } else if kind.is_symlink() {
            // `build` and `source` point back into the kernel build tree.
            continue;
        } else {
            install(&entry.path(), &dest, 0o644)?;
        }
    }
    Ok(())
}

pub fn dev_image(args: &[String]) -> Result<()> {
    let o = parse(args)?;
    let root = repo_root();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let upper = o.model.to_ascii_uppercase();
    let family = if ["d60", "d62", "d65"].contains(&o.model.as_str()) { "new" } else { "old" };

    // 0. The web frontend bundle, which `dphone-web`'s build script embeds
    //    from web/dist.
    if !o.no_web {
        let web = root.join("web");
        run(Command::new("pnpm").current_dir(&web).args(["install", "--frozen-lockfile"]))?;
        run(Command::new("pnpm").current_dir(&web).args(["run", "build"]))?;
    }

    // 1. dphone for the phone.
    run(Command::new(&cargo).current_dir(&root).args(["build", "--release", "-p", "dphone", "--target", TARGET]))?;
    let target_dir = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| root.join("target"));
    let dphone = target_dir.join(TARGET).join("release/dphone");

    // 2. The image tool, for this machine.
    run(Command::new(&cargo).current_dir(&root).args(["build", "--release", "-p", "dphone-image"]))?;
    let tool = target_dir.join("release/dphone-image");

    // 2b. The WebAssembly apps: build the modules, then turn them into
    //     components precompiled for the phone's interpreter.
    run(Command::new(&cargo).current_dir(root.join("apps")).args(["build", "--release"]))?;
    run(Command::new(&cargo).current_dir(&root).args([
        "build",
        "--release",
        "-p",
        "dphone-apps",
        "--features",
        "build-tool",
        "--bin",
        "dphone-apps-build",
    ]))?;
    let apps_tool = target_dir.join("release/dphone-apps-build");

    // 3. The root file system tree (same layout as nix/image.nix).
    let tree = o.out.join("rootfs");
    if tree.exists() {
        fs::remove_dir_all(&tree)?;
    }
    for dir in ["bin", "sbin", "etc/dphone", "dev", "proc", "sys", "tmp", "run", "var", "nvdata", "root", "usr/share/dphone/apps"] {
        fs::create_dir_all(tree.join(dir))?;
    }
    install(&dphone, &tree.join("sbin/init"), 0o755)?;
    symlink("../sbin/init", tree.join("bin/dphone"))?;
    for name in DPHONE_LINKS {
        symlink("../sbin/init", tree.join("bin").join(name))?;
    }
    install(&o.brush, &tree.join("bin/brush"), 0o755)?;
    symlink("brush", tree.join("bin/sh"))?;
    install(&o.coreutils, &tree.join("bin/coreutils"), 0o755)?;
    for applet in APPLETS {
        symlink("coreutils", tree.join("bin").join(applet))?;
    }
    symlink("test", tree.join("bin/["))?;
    run(Command::new(&apps_tool)
        .arg("stage")
        .arg("--apps")
        .arg(root.join("apps"))
        .arg("--wasm-dir")
        .arg(target_dir.join("wasm32-unknown-unknown/release"))
        .arg("--out")
        .arg(tree.join("usr/share/dphone/apps"))
        .args(["--target", "phone", "--no-wasm"]))?;
    copy_tree(&o.kernel_out.join("modules/lib/modules"), &tree.join("lib/modules"))?;
    install(&o.dsp, &tree.join(format!("lib/firmware/{family}-family/dspapp1107_lcc.bin")), 0o644)?;
    write(&tree.join("etc/passwd"), "root:x:0:0:root:/nvdata/root:/bin/brush\n")?;
    write(&tree.join("etc/group"), "root:x:0:\ntty:x:5:\n")?;
    write(&tree.join("etc/shells"), "/bin/brush\n/bin/sh\n")?;
    write(&tree.join("etc/dphone-release"), &format!("Open DPhone {} (development image)\n", o.version))?;
    write(&tree.join("etc/dphone/dev"), "development image: SSH on, /tmp/dphone override allowed\n")?;
    if o.no_stock_writes {
        write(&tree.join("etc/dphone/no-stock-writes"), "stock-owned nvdata files are never written by this image\n")?;
    }

    // 4. Kernel image for the slot, the JFFS2 image, and the EFF.
    let kernel = o.out.join("kernel.bin");
    run(Command::new(&tool)
        .arg("kernel-image")
        .arg("--zimage")
        .arg(o.kernel_out.join("images/zImage"))
        .arg("--dtb")
        .arg(o.kernel_out.join(format!("images/{}.dtb", o.model)))
        .args(["--model", &upper, "--out"])
        .arg(&kernel))?;
    let nodes = ["--node", "dev/console:c:5:1:600", "--node", "dev/null:c:1:3:666"];
    let jffs2 = o.out.join("rootfs.jffs2");
    run(Command::new(&tool).args(["jffs2", "build"]).arg(&tree).arg(&jffs2).args(["--mtime", "0"]).args(nodes))?;
    let eff = o.out.join(format!("{}_{upper}_firmware.eff", o.version));
    run(Command::new(&tool)
        .args(["image", "build", "--model", &upper, "--version", &o.version, "--mtime", "0", "--kernel"])
        .arg(&kernel)
        .arg("--rootfs-image")
        .arg(&jffs2)
        .arg("--out")
        .arg(&eff))?;

    for path in [&kernel, &jffs2, &eff] {
        println!("{:>10} bytes  {}", fs::metadata(path)?.len(), path.display());
    }
    Ok(())
}
