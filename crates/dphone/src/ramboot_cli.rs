//! `dphone ramboot IMAGE.eff`: start an image's kernel and root file system
//! from RAM, without installing it.
//!
//! For trying a kernel that might not boot: nothing is written to the
//! kernel or root partitions or to `ctrl`, so a power cycle always brings
//! the installed firmware back, and so does the hardware watchdog when the
//! new kernel hangs (it keeps running across the jump).
//!
//! The work is in three places:
//!
//! - `dphone_update::ramboot` reads and verifies the image and lays it out
//!   (see there for the design);
//! - this command gathers the facts about the running phone, hands the
//!   layout to the kernel with `kexec_load(2)`, and asks PID 1 to go;
//! - the init role shuts the system down in order (main role, every other
//!   process, nvdata unmounted) and makes the `reboot(2)` call that jumps.

use std::fs::File;
use std::io::{BufReader, Read};

use dphone_update::ramboot::{self, Options, Plan};

use crate::sys;

const USAGE: &str = "\
usage: dphone ramboot IMAGE.eff [--max-uptime SECONDS] [--append 'KERNEL ARGUMENTS'] [--dry-run]
  Start the kernel, devicetree, and root file system of an Open DPhone
  image from RAM.  Nothing is written to the kernel or root partitions or
  to 'ctrl': a reboot, a power cycle, or a kernel that hangs (the watchdog
  resets the phone) brings the installed firmware back.  /nvdata is the
  phone's own, mounted as always, so the started system has the same
  settings, address, and SSH keys.  A pending trial boot is neither counted
  nor confirmed by a system started this way.
  IMAGE.eff          the file 'dphone update' installs; '-' reads it from
                     standard input (saves the copy in /tmp)
  --max-uptime SECONDS
                     the started system resets the phone after this long
                     unless /run/dphone/stay exists (default 900; 0 for no
                     limit)
  --append ARGS      extra kernel arguments, e.g., amba_pl011.console_output=1
  --dry-run          verify the image and print what would be loaded and
                     passed to the kernel, then stop
";

struct Args {
    file: String,
    options: Options,
    dry_run: bool,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut file = None;
    let mut options = Options::default();
    let mut dry_run = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--max-uptime" => {
                let secs: u64 = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--max-uptime needs a number of seconds (0 for no limit)")?;
                options.max_uptime = (secs != 0).then_some(secs);
            }
            "--append" => options.append = it.next().ok_or("--append needs the kernel arguments")?.clone(),
            "--dry-run" => dry_run = true,
            "-" => file = Some(a.clone()),
            other if other.starts_with('-') => return Err(format!("unknown option '{other}'")),
            other if file.is_none() => file = Some(other.to_string()),
            other => return Err(format!("unexpected argument '{other}'")),
        }
    }
    Ok(Args { file: file.ok_or("no image named")?, options, dry_run })
}

pub fn run(args: &[String]) -> i32 {
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h" || a == "help") {
        crate::out!("{USAGE}");
        return if args.is_empty() { 2 } else { 0 };
    }
    let args = match parse(args) {
        Ok(a) => a,
        Err(e) => {
            crate::errln!("dphone ramboot: {e}");
            crate::err!("{USAGE}");
            return 2;
        }
    };
    match start(&args) {
        Ok(()) => 0,
        Err(e) => {
            crate::errln!("dphone ramboot: {e}");
            1
        }
    }
}

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
}

/// Why the jump cannot be made from this system, if it cannot.
fn not_ready() -> Option<String> {
    if !std::path::Path::new(sys::KEXEC_LOADED).exists() {
        return Some(
            "the running kernel has no kexec; install an image whose kernel has it once, with 'dphone update'".into(),
        );
    }
    // PID 1 must be our init role with the watchdog armed: it does the
    // orderly shutdown, and the watchdog is what ends a kernel that hangs.
    let init_has_watchdog = std::fs::read_dir("/proc/1/fd").is_ok_and(|dir| {
        dir.flatten().any(|e| std::fs::read_link(e.path()).is_ok_and(|t| t.starts_with("/dev/watchdog")))
    });
    if !init_has_watchdog {
        return Some(
            "the init role does not hold the watchdog, so a kernel that hangs would stay hung until a power cycle".into(),
        );
    }
    None
}

fn start(args: &Args) -> Result<(), String> {
    let model: dphone_state::Model = crate::model::detect()
        .and_then(|d| d.model.name().parse().ok())
        .ok_or("model unknown (no digium_board= and no devicetree match)")?;
    let ready = not_ready();
    if let (Some(why), false) = (&ready, args.dry_run) {
        return Err(why.clone());
    }

    let input: Box<dyn Read> = if args.file == "-" {
        Box::new(std::io::stdin().lock())
    } else {
        Box::new(File::open(&args.file).map_err(|e| format!("{}: {e}", args.file))?)
    };
    let opened = ramboot::open(BufReader::with_capacity(64 * 1024, input), model).map_err(|e| e.to_string())?;
    let meminfo = read("/proc/meminfo")?;
    let mem = |key: &str| ramboot::meminfo(&meminfo, key).ok_or(format!("/proc/meminfo: no {key}"));
    opened.check_memory(mem("MemAvailable")?, mem("MemTotal")?).map_err(|e| e.to_string())?;
    crate::outln!(
        "image: {model} {}, kernel {} bytes, root file system {} bytes; verifying",
        opened.version(),
        opened.kernel_size(),
        opened.rootfs_size()
    );
    let image = opened.load().map_err(|e| e.to_string())?;
    let ram = ramboot::system_ram(&read("/proc/iomem")?).map_err(|e| e.to_string())?;
    let plan = ramboot::plan(&image, ram, &read("/proc/cmdline")?, &args.options).map_err(|e| e.to_string())?;
    describe(&plan, &image);

    if args.dry_run {
        match ready {
            Some(why) => crate::outln!("dry run: nothing loaded.  Not startable from this system: {why}"),
            None => crate::outln!("dry run: nothing loaded"),
        }
        return Ok(());
    }

    let segments: Vec<sys::KexecSegment> = plan
        .segments(&image)
        .iter()
        .map(|s| sys::KexecSegment {
            buf: s.data.as_ptr() as *const libc::c_void,
            bufsz: s.data.len(),
            mem: s.mem as usize,
            memsz: s.memsz as usize,
        })
        .collect();
    // SAFETY: every segment points into `plan` or `image`, both alive
    // until after the call; the kernel copies the data during it.
    unsafe { sys::kexec_load(plan.entry as usize, &segments) }.map_err(|e| {
        let hint = match e.raw_os_error() {
            Some(libc::EPERM) => " (not root, or kexec is disabled)",
            Some(libc::ENOMEM) => " (not enough free memory)",
            Some(libc::EADDRNOTAVAIL) | Some(libc::EINVAL) => " (the kernel does not accept this layout)",
            Some(libc::EBUSY) => " (another image is being loaded)",
            _ => "",
        };
        format!("kexec_load: {e}{hint}")
    })?;
    drop(segments);
    drop(image);
    if read(sys::KEXEC_LOADED)?.trim() != "1" {
        return Err("the kernel did not keep the image".into());
    }

    crate::outln!(
        "loaded.  Starting it now: this session ends, and the phone answers again in about a minute.{}",
        match args.options.max_uptime {
            Some(secs) => format!(
                "  It returns to the installed firmware after {secs} s unless /run/dphone/stay exists there, \
                 and at any 'dphone reboot' or power cycle."
            ),
            None => "  It returns to the installed firmware at 'dphone reboot' or a power cycle.".into(),
        }
    );
    crate::out::flush();
    if let Err(e) = sys::kill(1, sys::kexec_signal()) {
        // Do not leave an image behind that nobody asked to start.
        sys::kexec_unload();
        return Err(format!("cannot signal PID 1: {e}"));
    }
    Ok(())
}

fn describe(plan: &Plan, image: &ramboot::Image) {
    for s in plan.segments(image) {
        crate::outln!("  {:<17} {:#010x}..{:#010x} ({} bytes)", s.name, s.mem, s.mem as u64 + s.memsz as u64, s.data.len());
    }
    crate::outln!(
        "  memory for the started kernel: {:#010x}..{:#010x} ({} MiB)",
        plan.mem_start,
        plan.root_at,
        plan.mem_size() >> 20
    );
    crate::outln!("  entry {:#010x}, tag list at {:#010x}", plan.entry, plan.atags_at);
    crate::outln!("  command line: {} {}", image.dtb_bootargs, plan.cmdline);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Result<Args, String> {
        parse(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn arguments() {
        let a = args(&["/tmp/fw.eff"]).unwrap();
        assert_eq!(a.file, "/tmp/fw.eff");
        assert_eq!(a.options, Options::default());
        assert_eq!(a.options.max_uptime, Some(900));
        assert!(!a.dry_run);

        let a = args(&["--max-uptime", "120", "fw.eff", "--append", "panic=30 loglevel=7", "--dry-run"]).unwrap();
        assert_eq!(a.file, "fw.eff");
        assert_eq!(a.options, Options { max_uptime: Some(120), append: "panic=30 loglevel=7".into() });
        assert!(a.dry_run);

        // No limit, image on standard input.
        let a = args(&["-", "--max-uptime", "0"]).unwrap();
        assert_eq!((a.file.as_str(), a.options.max_uptime), ("-", None));
    }

    #[test]
    fn bad_arguments() {
        for bad in [
            &[][..],
            &["--dry-run"],
            &["a.eff", "b.eff"],
            &["a.eff", "--max-uptime"],
            &["a.eff", "--max-uptime", "soon"],
            &["a.eff", "--max-uptime", "-5"],
            &["a.eff", "--append"],
            &["a.eff", "--now"],
        ] {
            assert!(args(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn help_and_usage_exit_codes() {
        let run_ = |a: &[&str]| run(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(run_(&[]), 2);
        assert_eq!(run_(&["--help"]), 0);
        assert_eq!(run_(&["a.eff", "--bogus"]), 2);
        // A file that is not there: an error, not a usage problem.  (On a
        // workstation the model is unknown first; either way it is 1.)
        assert_eq!(run_(&["/nonexistent/image.eff", "--dry-run"]), 1);
    }
}
