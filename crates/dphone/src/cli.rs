//! Role dispatch for the multi-call binary.
//!
//! | Invocation | Role |
//! | --- | --- |
//! | PID 1 (as `/sbin/init`) | init role |
//! | `dphone main [options]` | main role |
//! | `dphone mount ...`, or a symlink named `mount`, etc. | system command |
//! | `dphone <anything else>` | diagnostic command sent to the main role |

use std::ffi::OsString;
use std::path::Path;

use crate::init;

pub const LOCAL_HELP: &str = "\
Commands run locally:
  main       run the main role (see 'dphone main --help')
  update     firmware: status, install an EFF, switch bank (see 'dphone update')
  mount, umount, insmod, rmmod, lsmod, modprobe, reboot, poweroff, halt, dmesg
";

fn usage() -> String {
    format!(
        "{} {}\nusage: dphone <command> [args...]\n{}Other commands go to the running main role; 'dphone help' lists them.\n",
        crate::PRODUCT,
        crate::VERSION,
        LOCAL_HELP
    )
}

/// Hooks other crates supply to the init role.  The updater's rollback
/// function is added here when that crate is wired in.
fn init_hooks() -> init::Hooks {
    init::Hooks { rollback: Some(crate::update_cli::rollback_hook) }
}

pub fn run(args: Vec<OsString>) -> i32 {
    let args: Vec<String> = args.into_iter().map(|a| a.to_string_lossy().into_owned()).collect();
    let argv0 = args.first().map(String::as_str).unwrap_or("dphone");
    let base = Path::new(argv0).file_name().and_then(|n| n.to_str()).unwrap_or("dphone");
    let rest = args.get(1..).unwrap_or(&[]);

    // PID 1 is always the init role, whatever the name or arguments.
    if crate::sys::getpid() == 1 {
        init::run(init_hooks());
    }
    // Symlinks: /sbin/mount -> dphone, and so on.
    if crate::cmds::is_command(base) {
        return crate::cmds::run(base, rest);
    }
    let Some(command) = rest.first().map(String::as_str) else {
        eprint!("{}", usage());
        return 2;
    };
    let command_args = &rest[1..];
    match command {
        "main" => {
            if command_args.iter().any(|a| a == "--help" || a == "-h") {
                print!("{}", crate::mainrole::USAGE);
                return 0;
            }
            crate::mainrole::run(command_args)
        }
        "init" => run_init_test(command_args),
        "update" => crate::update_cli::run(command_args),
        "--help" | "-h" => {
            print!("{}", usage());
            0
        }
        "--version" | "-V" => {
            println!("{} {}", crate::PRODUCT, crate::VERSION);
            0
        }
        c if crate::cmds::is_command(c) => crate::cmds::run(c, command_args),
        _ => crate::diag::client::run(&crate::paths::client_diag_socket(), rest),
    }
}

/// `dphone init --supervise-only ...`: the supervisor without PID 1's
/// duties, for tests on a workstation.  The real init role only ever runs
/// as PID 1.
fn run_init_test(args: &[String]) -> i32 {
    const USAGE: &str = "usage: dphone init --supervise-only --nvdata DIR --run-dir DIR \
[--grace SECS] [--beat-timeout SECS] [--unhealthy-reboot SECS] -- PROGRAM [ARGS...]\n\
(the init role proper runs only as PID 1)\n";
    let mut nvdata = None;
    let mut run_dir = None;
    let mut grace = init::START_GRACE_SECS;
    let mut beat = init::BEAT_TIMEOUT_SECS;
    let mut unhealthy = init::UNHEALTHY_REBOOT_SECS;
    let mut supervise_only = false;
    let mut command = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let num = |it: &mut std::slice::Iter<String>| it.next().and_then(|v| v.parse::<u64>().ok());
        match a.as_str() {
            "--supervise-only" => supervise_only = true,
            "--nvdata" => nvdata = it.next().cloned(),
            "--run-dir" => run_dir = it.next().cloned(),
            "--grace" => grace = num(&mut it).unwrap_or(grace),
            "--beat-timeout" => beat = num(&mut it).unwrap_or(beat),
            "--unhealthy-reboot" => unhealthy = num(&mut it).unwrap_or(unhealthy),
            "--" => {
                command = it.cloned().collect();
                break;
            }
            _ => {
                eprint!("{USAGE}");
                return 2;
            }
        }
    }
    let (true, Some(nvdata), Some(run_dir), false) = (supervise_only, nvdata, run_dir, command.is_empty()) else {
        eprint!("{USAGE}");
        return 2;
    };
    init::run_supervisor_test(init::TestOptions {
        paths: crate::paths::Paths::new(nvdata, run_dir),
        main_command: command,
        start_grace: grace,
        beat_timeout: beat,
        unhealthy_reboot: unhealthy,
    })
}
