//! System commands the image has no other provider for.  uutils coreutils
//! and uutils sed cover file and text tools; these are the Linux-specific
//! ones a person at an SSH prompt expects.  Each is reachable as `dphone <name>` and, through a
//! symlink named after it, directly.

use std::path::PathBuf;

use clap::{ArgAction, Args, Subcommand};

use crate::init::modules;
use crate::sys::{self, Shutdown};

/// The system commands.  The image has a symbolic link to the binary under
/// each of these names (`nix/image.nix`), and the same names are commands
/// of `dphone`.
#[derive(Debug, Subcommand)]
pub enum SystemCommand {
    /// Mount a file system, or list what is mounted
    Mount(Mount),
    /// Unmount file systems
    Umount(Umount),
    /// Load a kernel module from a file
    Insmod(Insmod),
    /// Unload kernel modules
    Rmmod(Rmmod),
    /// List the loaded kernel modules
    Lsmod,
    /// Load a kernel module by name, and the modules it depends on
    Modprobe(Modprobe),
    /// Restart the phone
    Reboot(ShutdownArgs),
    /// Shut the phone down and turn it off
    Poweroff(ShutdownArgs),
    /// Shut the phone down and stop
    Halt(ShutdownArgs),
    /// Print the kernel's log
    Dmesg(Dmesg),
    /// List every process
    Ps(Ps),
}

#[derive(Debug, Args)]
pub struct Mount {
    /// The type of the file system
    #[arg(short = 't', value_name = "TYPE")]
    fstype: Option<String>,
    /// Mount options, separated by commas
    #[arg(short = 'o', value_name = "OPTIONS", value_delimiter = ',')]
    options: Vec<String>,
    /// Read-only (the option ro)
    #[arg(short = 'r')]
    read_only: bool,
    /// Read and write (the option rw)
    #[arg(short = 'w', conflicts_with = "read_only")]
    read_write: bool,
    /// What to mount; without it and TARGET, what is mounted is listed
    ///
    /// With the option remount, the one name given is the mount point.
    source: Option<String>,
    /// Where to mount it
    target: Option<String>,
}

#[derive(Debug, Args)]
pub struct Umount {
    /// Unmount even if the file system is busy
    #[arg(short)]
    force: bool,
    /// Detach now, and clean up when the file system is no longer busy
    #[arg(short)]
    lazy: bool,
    #[arg(required = true, value_name = "TARGET")]
    targets: Vec<String>,
}

#[derive(Debug, Args)]
pub struct Insmod {
    file: PathBuf,
    /// Parameters of the module, as NAME=VALUE
    #[arg(value_name = "PARAMETER")]
    parameters: Vec<String>,
}

#[derive(Debug, Args)]
pub struct Modprobe {
    name: String,
    /// Parameters of the module, as NAME=VALUE
    #[arg(value_name = "PARAMETER")]
    parameters: Vec<String>,
}

#[derive(Debug, Args)]
pub struct Rmmod {
    /// Unload even if the module is in use
    #[arg(short)]
    force: bool,
    #[arg(required = true, value_name = "NAME")]
    names: Vec<String>,
}

/// The reason goes to the stock `reboot_reason` file from here, before the
/// init role is signaled: these commands run in the calling process (they
/// must work with the main role down), and the signal to PID 1 carries no
/// text.
#[derive(Debug, Default, Args)]
pub struct ShutdownArgs {
    /// At once, by the system call: the init role is not asked to stop
    /// the main role and unmount nvdata first
    #[arg(short, long)]
    force: bool,
    /// Why, for the record the next boot shows
    #[arg(value_name = "REASON")]
    reason: Vec<String>,
}

#[derive(Debug, Args)]
pub struct Dmesg {
    /// Clear the log after printing it
    #[arg(short)]
    clear: bool,
}

/// Every process is always listed, so the usual ways of asking for all of
/// them ('ps aux', 'ps -ef', 'ps -A') are accepted and change nothing.
#[derive(Debug, Args)]
pub struct Ps {
    #[arg(short = 'A', short_aliases = ['a', 'e', 'f', 'l', 'u', 'w', 'x'], action = ArgAction::Count, hide = true)]
    all: u8,
    /// Letters of the same kind without a hyphen (e.g., aux)
    #[arg(value_name = "LETTERS", value_parser = ps_letters)]
    letters: Option<String>,
}

fn ps_letters(word: &str) -> Result<String, String> {
    if word.chars().all(|c| "aAefluwx".contains(c)) {
        Ok(word.to_string())
    } else {
        Err("only the letters a, A, e, f, l, u, w, and x are known, and all mean every process".into())
    }
}

/// Run a system command.
pub fn run(command: SystemCommand) -> i32 {
    let (name, result) = match command {
        SystemCommand::Mount(a) => ("mount", mount(a)),
        SystemCommand::Umount(a) => ("umount", umount(a)),
        SystemCommand::Insmod(a) => ("insmod", insmod(a)),
        SystemCommand::Rmmod(a) => ("rmmod", rmmod(a)),
        SystemCommand::Lsmod => ("lsmod", lsmod()),
        SystemCommand::Modprobe(a) => ("modprobe", modprobe(a)),
        SystemCommand::Reboot(a) => ("reboot", shutdown(Shutdown::Reboot, a)),
        SystemCommand::Poweroff(a) => ("poweroff", shutdown(Shutdown::PowerOff, a)),
        SystemCommand::Halt(a) => ("halt", shutdown(Shutdown::Halt, a)),
        SystemCommand::Dmesg(a) => ("dmesg", dmesg(a)),
        SystemCommand::Ps(_) => ("ps", ps()),
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

fn mount(args: Mount) -> Result<(), String> {
    let (Some(first), second) = (args.source, args.target) else {
        let mounts = std::fs::read_to_string("/proc/mounts").map_err(|e| format!("/proc/mounts: {e}"))?;
        for line in mounts.lines() {
            let f: Vec<&str> = line.split_ascii_whitespace().collect();
            if f.len() >= 4 {
                print(&format!("{} on {} type {} ({})\n", f[0], f[1], f[2], f[3]));
            }
        }
        return Ok(());
    };
    let mut options = args.options;
    if args.read_only {
        options.push("ro".into());
    }
    if args.read_write {
        options.push("rw".into());
    }
    let (flags, data) = sys::parse_mount_options(&options.join(","));
    let (source, target) = match second {
        Some(target) => (first, target),
        // `mount -o remount,rw /nvdata`
        None if flags & libc::MS_REMOUNT != 0 => ("none".to_string(), first),
        None => return Err("where to mount it is needed (SOURCE TARGET), except with the option remount".into()),
    };
    let fstype = args.fstype.unwrap_or_default();
    if fstype.is_empty() && flags & (libc::MS_REMOUNT | libc::MS_BIND) == 0 {
        return Err("a filesystem type is needed (-t TYPE)".into());
    }
    sys::mount(&source, &target, &fstype, flags, &data).map_err(|e| format!("{source} on {target}: {e}"))
}

fn umount(args: Umount) -> Result<(), String> {
    let flags = if args.force { libc::MNT_FORCE } else { 0 } | if args.lazy { libc::MNT_DETACH } else { 0 };
    for t in &args.targets {
        sys::umount(t, flags).map_err(|e| format!("{t}: {e}"))?;
    }
    Ok(())
}

fn insmod(args: Insmod) -> Result<(), String> {
    modules::insmod(&args.file, &args.parameters.join(" ")).map_err(|e| format!("{}: {e}", args.file.display()))
}

fn modprobe(args: Modprobe) -> Result<(), String> {
    let index = modules::Index::scan(std::path::Path::new(modules::MODULES_ROOT));
    modules::modprobe(&index, &args.name, &args.parameters.join(" ")).map(|_| ()).map_err(|e| format!("{}: {e}", args.name))
}

fn rmmod(args: Rmmod) -> Result<(), String> {
    let flags = libc::O_NONBLOCK | if args.force { libc::O_TRUNC } else { 0 };
    for n in &args.names {
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

/// `reboot`, `halt`, and `poweroff`.
fn shutdown(kind: Shutdown, args: ShutdownArgs) -> Result<(), String> {
    let paths = crate::paths::Paths::default();
    let may_write = crate::paths::stock_writes_allowed() && !paths.recovery_flag().exists();
    shutdown_with(kind, &args, &paths, may_write, |kind, force| {
        if force {
            sys::sync();
            return Err(format!("reboot system call: {}", sys::reboot(kind)));
        }
        let sig = match kind {
            Shutdown::Reboot => libc::SIGTERM,
            Shutdown::Halt => libc::SIGUSR1,
            Shutdown::PowerOff => libc::SIGUSR2,
            Shutdown::Kexec => sys::kexec_signal(),
        };
        // The init role (PID 1) does the orderly part: it stops the main
        // role, syncs, and unmounts nvdata before the system call.
        sys::kill(1, sig).map_err(|e| format!("cannot signal PID 1: {e}"))
    })
}

/// The part of [`shutdown`] that does not depend on being on a phone:
/// record the reason under `paths` (when `may_write`),
/// then hand over to `go(kind, force)`.  The record is complete on flash
/// (written to a temporary file, synced, renamed, directory synced) before
/// `go` runs.
fn shutdown_with(
    kind: Shutdown,
    args: &ShutdownArgs,
    paths: &crate::paths::Paths,
    may_write: bool,
    go: impl FnOnce(Shutdown, bool) -> Result<(), String>,
) -> Result<(), String> {
    let reason =
        if args.reason.is_empty() { "dphone: requested".to_string() } else { format!("dphone: {}", args.reason.join(" ")) };
    if may_write
        && let Err(e) = crate::paths::write_atomic(&paths.reboot_reason(), format!("{reason}\n").as_bytes(), 0o644)
    {
        crate::errln!("warning: {}: {e}", paths.reboot_reason().display());
    }
    go(kind, args.force)
}

fn dmesg(args: Dmesg) -> Result<(), String> {
    let raw = sys::read_kernel_log(args.clear).map_err(|e| e.to_string())?;
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
/// line.
fn ps() -> Result<(), String> {
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

    #[derive(Debug, clap::Parser)]
    #[command(multicall = true)]
    struct Cli {
        #[command(subcommand)]
        command: SystemCommand,
    }

    fn parse(words: &[&str]) -> Result<SystemCommand, clap::Error> {
        <Cli as clap::Parser>::try_parse_from(words).map(|c| c.command)
    }

    fn bad(words: &[&str]) {
        assert!(parse(words).is_err(), "{words:?}");
    }

    /// The usual ways of asking `ps` for every process.
    #[test]
    fn ps_arguments() {
        for good in [&["ps"][..], &["ps", "aux"], &["ps", "-ef"], &["ps", "-A"], &["ps", "-e", "-f"], &["ps", "auxww"], &["ps", "-eww"]] {
            assert!(matches!(parse(good), Ok(SystemCommand::Ps(_))), "{good:?}");
        }
        bad(&["ps", "-p", "1"]);
        bad(&["ps", "axo"]);
        bad(&["ps", "aux", "more"]);
    }

    fn args(words: &[&str]) -> ShutdownArgs {
        let mut all = vec!["reboot"];
        all.extend(words);
        match parse(&all) {
            Ok(SystemCommand::Reboot(a)) => a,
            other => panic!("{words:?}: {other:?}"),
        }
    }

    /// `dphone reboot REASON` (and `halt`, `poweroff`): the reason is in
    /// `reboot_reason`, whole, by the time the init role is signaled or the
    /// system call is made.
    #[test]
    fn the_reason_is_recorded_before_the_phone_goes_down() {
        let dir = tempfile::tempdir().unwrap();
        let nv = dir.path().join("nv");
        std::fs::create_dir_all(&nv).unwrap();
        let paths = crate::paths::Paths::new(&nv, dir.path().join("run"));
        let file = paths.reboot_reason();
        // What stock left behind.
        std::fs::write(&file, "phonemenu: restart\n").unwrap();

        let run = |kind, words: &[&str], may_write| {
            let mut seen = None;
            shutdown_with(kind, &args(words), &paths, may_write, |kind, force| {
                seen = Some((kind, force, std::fs::read_to_string(&file).unwrap()));
                Ok(())
            })
            .unwrap();
            seen.expect("the hand-over ran")
        };

        assert_eq!(
            run(Shutdown::Reboot, &["trying", "bank", "b"], true),
            (Shutdown::Reboot, false, "dphone: trying bank b\n".to_string())
        );
        assert_eq!(run(Shutdown::Reboot, &[], true), (Shutdown::Reboot, false, "dphone: requested\n".to_string()));
        // Options are not part of the reason, wherever they stand.
        assert_eq!(
            run(Shutdown::Halt, &["going", "-f", "dark"], true),
            (Shutdown::Halt, true, "dphone: going dark\n".to_string())
        );
        assert_eq!(
            run(Shutdown::PowerOff, &["done"], true),
            (Shutdown::PowerOff, false, "dphone: done\n".to_string())
        );

        // A failure to hand over is reported.
        let e = shutdown_with(Shutdown::Reboot, &args(&[]), &paths, true, |_, _| Err("cannot signal PID 1: no".into()));
        assert_eq!(e, Err("cannot signal PID 1: no".to_string()));
    }
}
