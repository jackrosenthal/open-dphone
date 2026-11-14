//! `cargo xtask deploy <phone>`: push a fresh `dphone` to a running phone
//! without flashing.
//!
//! The init role runs `/tmp/dphone` as the main role whenever that file
//! exists, so a deploy is: build for the phone, copy the binary to
//! `/tmp/dphone`, and ask the main role to restart.  `/tmp` is a tmpfs, so a
//! reboot returns the phone to the binary in its flash image.
//!
//! This uses the workstation's `scp` and `ssh` against the phone's built-in
//! SSH server; the phone must have your key in
//! `/nvdata/root/.ssh/authorized_keys`.

use std::path::PathBuf;
use std::process::Command;

use crate::{Result, repo_root};

const TARGET: &str = "arm-unknown-linux-musleabi";
const REMOTE: &str = "/tmp/dphone";
const USAGE: &str = "deploy <phone> [--no-build]   (<phone> is a host name or address; the login is root)";

fn run(cmd: &mut Command) -> Result<()> {
    let status = cmd.status().map_err(|e| format!("cannot run {:?}: {e}", cmd.get_program()))?;
    if !status.success() {
        return Err(format!("{:?} failed: {status}", cmd.get_program()).into());
    }
    Ok(())
}

pub fn deploy(args: &[String]) -> Result<()> {
    let mut phone = None;
    let mut build = true;
    for a in args {
        match a.as_str() {
            "--no-build" => build = false,
            h if !h.starts_with('-') && phone.is_none() => phone = Some(h.to_string()),
            other => return Err(format!("unknown argument '{other}'\nusage: {USAGE}").into()),
        }
    }
    let phone = phone.ok_or_else(|| format!("usage: {USAGE}"))?;
    let login = if phone.contains('@') { phone } else { format!("root@{phone}") };

    let root = repo_root();
    if build {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        run(Command::new(cargo).current_dir(&root).args(["build", "--release", "-p", "dphone", "--target", TARGET]))?;
    }
    let target_dir = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| root.join("target"));
    let binary = target_dir.join(TARGET).join("release/dphone");
    if !binary.is_file() {
        return Err(format!("{} does not exist; run without --no-build", binary.display()).into());
    }

    // Copy under another name and rename, so the init role never starts a
    // half-written file.
    let staged = format!("{REMOTE}.new");
    run(Command::new("scp").arg("-q").arg(&binary).arg(format!("{login}:{staged}")))?;
    run(Command::new("ssh").arg(&login).arg(format!("chmod 755 {staged} && mv {staged} {REMOTE} && dphone restart")))?;
    println!("{} is now the main role on {login}; reboot the phone to go back to its image", binary.display());
    Ok(())
}
