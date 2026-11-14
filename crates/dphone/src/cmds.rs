//! System commands the image has no other provider for.  uutils coreutils
//! covers file tools; these are the Linux-specific ones a person at an SSH
//! prompt expects.  Each is reachable as `dphone <name>` and, through a
//! symlink named after it, directly.

use std::path::Path;

use crate::init::modules;
use crate::sys::{self, Shutdown};

/// Names handled here.
pub const NAMES: &[&str] =
    &["mount", "umount", "insmod", "rmmod", "lsmod", "modprobe", "reboot", "poweroff", "halt", "dmesg"];

pub fn is_command(name: &str) -> bool {
    NAMES.contains(&name)
}

/// Run a system command.  `args` excludes the command name.
pub fn run(name: &str, args: &[String]) -> i32 {
    let result = match name {
        "mount" => mount(args),
        "umount" => umount(args),
        "insmod" => insmod(args),
        "rmmod" => rmmod(args),
        "lsmod" => lsmod(),
        "modprobe" => modprobe(args),
        "reboot" => shutdown(Shutdown::Reboot, args),
        "poweroff" => shutdown(Shutdown::PowerOff, args),
        "halt" => shutdown(Shutdown::Halt, args),
        "dmesg" => dmesg(args),
        _ => Err(format!("unknown command '{name}'")),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{name}: {e}");
            1
        }
    }
}

fn print(text: &str) {
    use std::io::Write;
    // A closed pipe (e.g., `dmesg | head`) is not an error worth a panic.
    let _ = std::io::stdout().write_all(text.as_bytes());
}

fn mount(args: &[String]) -> Result<(), String> {
    let mut fstype = String::new();
    let mut opts = String::new();
    let mut positional = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-t" => fstype = it.next().ok_or("-t needs a type")?.clone(),
            "-o" => {
                if !opts.is_empty() {
                    opts.push(',');
                }
                opts.push_str(it.next().ok_or("-o needs options")?);
            }
            "-r" => opts.push_str(",ro"),
            "-w" => opts.push_str(",rw"),
            "-h" | "--help" => {
                print("usage: mount [-t TYPE] [-o OPTIONS] SOURCE TARGET\n       mount            (list mounts)\n");
                return Ok(());
            }
            _ => positional.push(a.clone()),
        }
    }
    if positional.is_empty() {
        let mounts = std::fs::read_to_string("/proc/mounts").map_err(|e| format!("/proc/mounts: {e}"))?;
        for line in mounts.lines() {
            let f: Vec<&str> = line.split_ascii_whitespace().collect();
            if f.len() >= 4 {
                print(&format!("{} on {} type {} ({})\n", f[0], f[1], f[2], f[3]));
            }
        }
        return Ok(());
    }
    let (flags, data) = sys::parse_mount_options(&opts);
    let (source, target) = match positional.as_slice() {
        [source, target] => (source.as_str(), target.as_str()),
        // `mount -o remount,rw /nvdata`
        [target] if flags & libc::MS_REMOUNT != 0 => ("none", target.as_str()),
        _ => return Err("usage: mount [-t TYPE] [-o OPTIONS] SOURCE TARGET".into()),
    };
    if fstype.is_empty() && flags & (libc::MS_REMOUNT | libc::MS_BIND) == 0 {
        return Err("a filesystem type is needed (-t TYPE)".into());
    }
    sys::mount(source, target, &fstype, flags, &data).map_err(|e| format!("{source} on {target}: {e}"))
}

fn umount(args: &[String]) -> Result<(), String> {
    let mut flags = 0;
    let mut targets = Vec::new();
    for a in args {
        match a.as_str() {
            "-f" => flags |= libc::MNT_FORCE,
            "-l" => flags |= libc::MNT_DETACH,
            _ => targets.push(a),
        }
    }
    if targets.is_empty() {
        return Err("usage: umount [-f] [-l] TARGET...".into());
    }
    for t in targets {
        sys::umount(t, flags).map_err(|e| format!("{t}: {e}"))?;
    }
    Ok(())
}

fn insmod(args: &[String]) -> Result<(), String> {
    let (path, params) = args.split_first().ok_or("usage: insmod FILE [PARAM=VALUE...]")?;
    modules::insmod(Path::new(path), &params.join(" ")).map_err(|e| format!("{path}: {e}"))
}

fn modprobe(args: &[String]) -> Result<(), String> {
    let (name, params) = args.split_first().ok_or("usage: modprobe NAME [PARAM=VALUE...]")?;
    let index = modules::Index::scan(Path::new(modules::MODULES_ROOT));
    modules::modprobe(&index, name, &params.join(" ")).map(|_| ()).map_err(|e| format!("{name}: {e}"))
}

fn rmmod(args: &[String]) -> Result<(), String> {
    let mut flags = libc::O_NONBLOCK;
    let mut names = Vec::new();
    for a in args {
        match a.as_str() {
            "-f" => flags |= libc::O_TRUNC,
            _ => names.push(a),
        }
    }
    if names.is_empty() {
        return Err("usage: rmmod [-f] NAME...".into());
    }
    for n in names {
        let name = modules::normalize(n);
        sys::delete_module(&name, flags).map_err(|e| format!("{n}: {e}"))?;
    }
    Ok(())
}

fn lsmod() -> Result<(), String> {
    let text = std::fs::read_to_string("/proc/modules").map_err(|e| format!("/proc/modules: {e}"))?;
    print(&format!("{:<28} {:>8}  Used by\n", "Module", "Size"));
    for line in text.lines() {
        let f: Vec<&str> = line.split_ascii_whitespace().collect();
        if f.len() >= 4 {
            let users = f[3].trim_end_matches(',');
            let users = if users == "-" { "" } else { users };
            print(&format!("{:<28} {:>8}  {} {}\n", f[0], f[1], f[2], users));
        }
    }
    Ok(())
}

fn shutdown(kind: Shutdown, args: &[String]) -> Result<(), String> {
    let force = args.iter().any(|a| a == "-f");
    if force {
        sys::sync();
        return Err(format!("reboot system call: {}", sys::reboot(kind)));
    }
    let sig = match kind {
        Shutdown::Reboot => libc::SIGTERM,
        Shutdown::Halt => libc::SIGUSR1,
        Shutdown::PowerOff => libc::SIGUSR2,
    };
    // The init role (PID 1) does the orderly part.
    sys::kill(1, sig).map_err(|e| format!("cannot signal PID 1: {e}"))
}

fn dmesg(args: &[String]) -> Result<(), String> {
    let clear = args.iter().any(|a| a == "-c");
    let raw = sys::read_kernel_log(clear).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&raw);
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        // Strip the "<N>" priority prefix.
        let line = match line.strip_prefix('<').and_then(|l| l.split_once('>')) {
            Some((prio, rest)) if prio.bytes().all(|b| b.is_ascii_digit()) => rest,
            _ => line,
        };
        out.push_str(line);
        out.push('\n');
    }
    print(&out);
    Ok(())
}
