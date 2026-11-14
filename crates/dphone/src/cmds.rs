//! System commands the image has no other provider for.  uutils coreutils
//! and uutils sed cover file and text tools; these are the Linux-specific
//! ones a person at an SSH prompt expects.  Each is reachable as `dphone <name>` and, through a
//! symlink named after it, directly.

use std::path::Path;

use crate::init::modules;
use crate::sys::{self, Shutdown};

/// Names handled here.
pub const NAMES: &[&str] =
    &["mount", "umount", "insmod", "rmmod", "lsmod", "modprobe", "reboot", "poweroff", "halt", "dmesg", "ps"];

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
        "ps" => ps(args),
        _ => Err(format!("unknown command '{name}'")),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            crate::errln!("{name}: {e}");
            1
        }
    }
}

fn print(text: &str) {
    crate::out!("{text}");
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

/// One line of `ps`, from `/proc/PID/stat`.
#[derive(Debug, PartialEq)]
struct Process {
    comm: String,
    state: char,
    ppid: u32,
    /// User plus system CPU time, in clock ticks.
    ticks: u64,
    /// Resident set, in pages.
    rss: u64,
}

/// Parse `/proc/PID/stat`: "PID (COMM) STATE PPID ...".  COMM may hold
/// spaces and parentheses, so the fields are counted from its last ')'.
fn parse_stat(stat: &str) -> Option<Process> {
    let (head, tail) = stat.rsplit_once(')')?;
    let comm = head.split_once('(')?.1.to_string();
    let f: Vec<&str> = tail.split_ascii_whitespace().collect();
    Some(Process {
        comm,
        state: f.first()?.chars().next()?,
        ppid: f.get(1)?.parse().ok()?,
        ticks: f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?,
        rss: f.get(21)?.parse().ok()?,
    })
}

/// Every process: PID, parent, state, resident memory, CPU time, and command
/// line.  The usual ways of asking for all processes (`ps aux`, `ps -ef`)
/// are accepted; there is no selection, since everything is always shown.
fn ps(args: &[String]) -> Result<(), String> {
    const USAGE: &str = "usage: ps    (lists every process; 'aux', '-ef', '-A', and the like are accepted)";
    for a in args {
        if a == "-h" || a == "--help" {
            print(&format!("{USAGE}\n"));
            return Ok(());
        }
        if !a.trim_start_matches('-').chars().all(|c| "aAefluwx".contains(c)) {
            return Err(format!("unknown option '{a}'\n{USAGE}"));
        }
    }
    // SAFETY: sysconf with valid names has no preconditions.
    let (tick, page_kb) = unsafe { (libc::sysconf(libc::_SC_CLK_TCK).max(1) as u64, libc::sysconf(libc::_SC_PAGESIZE).max(1024) as u64 / 1024) };
    let mut pids: Vec<u32> = std::fs::read_dir("/proc")
        .map_err(|e| format!("/proc: {e}"))?
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
        .collect();
    pids.sort_unstable();
    print(&format!("{:>5} {:>5} S {:>7} {:>8} COMMAND\n", "PID", "PPID", "RSS", "TIME"));
    for pid in pids {
        // A process may end between the directory listing and the read.
        let Some(p) = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok().as_deref().and_then(parse_stat) else {
            continue;
        };
        let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        let words: Vec<_> = cmdline.split(|b| *b == 0).filter(|w| !w.is_empty()).map(String::from_utf8_lossy).collect();
        // Kernel threads have no command line.
        let command = if words.is_empty() { format!("[{}]", p.comm) } else { words.join(" ") };
        let secs = p.ticks / tick;
        let time = format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60);
        print(&format!("{pid:>5} {:>5} {} {:>6}K {time:>8} {command}\n", p.ppid, p.state, p.rss * page_kb));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_line() {
        let line = "812 (tmux: server (1)) S 1 812 812 0 -1 4194560 3160 24710 0 3 141 59 20 14 20 0 1 0 5306 \
                    12587008 1190 18446744073709551615 1 1 0 0 0 0 0 528386 134433281 0 0 0 17 9 0 0 0 0 0";
        assert_eq!(
            parse_stat(line),
            Some(Process { comm: "tmux: server (1)".into(), state: 'S', ppid: 1, ticks: 200, rss: 1190 })
        );
        assert_eq!(parse_stat("12 (x) R"), None);
    }
}
