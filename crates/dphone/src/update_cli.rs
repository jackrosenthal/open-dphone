//! `dphone update ...`: firmware update commands that run in the calling
//! process, not through the diag socket, so they work over SSH or the serial
//! console even when the main role is down.
//!
//! This is also the way back to stock firmware: `dphone update install
//! FILE.eff` accepts a stock EFF.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use clap::Subcommand;
use dphone_hal::Mtd;
use dphone_hal::phone::PhoneMtd;
use dphone_update::{Bank, Options, Updater, inspect, select_bank, trial};

use crate::init::trial::TrialRecord;

#[derive(Debug, Subcommand)]
pub enum UpdateCommand {
    /// Show both banks, the ctrl record, and what boot2 will boot next
    Status,
    /// Install an image into the bank that is not running, and select it
    ///
    /// The image (Open DPhone or stock) is written, verified, and selected
    /// for the next boot; reboot afterwards with 'dphone reboot'.
    Install {
        /// The image (an EFF file)
        file: PathBuf,
        /// The bank to write [default: the one that is not running]
        #[arg(long, value_parser = parse_bank)]
        bank: Option<Bank>,
        /// Go ahead even if the bank that stays is not bootable (for
        /// recovery work from a system started from RAM)
        #[arg(long)]
        allow_unbootable_other: bool,
    },
    /// Select a bank for the next boot without writing an image
    ///
    /// Refused unless that bank holds a bootable image.  Removes a trial
    /// record.
    SwitchBank {
        #[arg(value_parser = parse_bank)]
        bank: Bank,
    },
    /// Record a trial boot of the bank 'ctrl' selects for the next boot
    ///
    /// For an image installed and selected by other means (e.g., by the
    /// stock updater, or with switch-bank, which must come first because it
    /// removes a trial record): Open DPhone returns to the other bank by
    /// itself if the image does not come up healthy within a few boots.
    ArmTrial {
        /// The bank to return to [default: the running one]
        ///
        /// The running bank is the one in root= on the kernel command line;
        /// when that is the selected bank itself or neither bank (a kernel
        /// started from RAM), name the bank to return to.
        #[arg(long, value_name = "BANK", value_parser = parse_bank)]
        return_to: Option<Bank>,
    },
    /// Erase the unused 'userdata' partition and write a root file system image there
    ///
    /// For booting a test kernel from RAM with root=/dev/mtdblockN without
    /// touching either firmware bank.
    WriteUserdata {
        /// The image (JFFS2); '-' reads it from standard input
        image: PathBuf,
    },
}

fn cmdline() -> String {
    std::fs::read_to_string("/proc/cmdline").unwrap_or_default()
}

fn model() -> Option<dphone_state::Model> {
    let detected = crate::model::detect()?;
    detected.model.name().parse().ok()
}

fn open_mtd() -> Result<PhoneMtd, String> {
    PhoneMtd::open().map_err(|e| format!("flash not available: {e}"))
}

fn parse_bank(s: &str) -> Result<Bank, String> {
    match s {
        "a" | "A" => Ok(Bank::A),
        "b" | "B" => Ok(Bank::B),
        _ => Err("not a bank (a or b)".into()),
    }
}

fn bank_name(b: Bank) -> &'static str {
    match b {
        Bank::A => "A (k0/root0)",
        Bank::B => "B (k1/root1)",
    }
}

fn open_dir() -> std::path::PathBuf {
    crate::paths::Paths::default().open_dir()
}

/// The init role's rollback hook: select the previous bank in `ctrl`.
pub fn rollback_hook(record: &TrialRecord) -> std::io::Result<()> {
    let model = model().ok_or_else(|| std::io::Error::other("model unknown"))?;
    let previous = if record.previous_bank == 0 { Bank::A } else { Bank::B };
    let mut mtd = PhoneMtd::open().map_err(|e| std::io::Error::other(e.to_string()))?;
    trial::rollback(&mut mtd, model, &cmdline(), previous).map_err(Into::into)
}

pub fn run(command: UpdateCommand) -> i32 {
    let result = match command {
        UpdateCommand::Status => status(),
        UpdateCommand::Install { file, bank, allow_unbootable_other } => {
            install(&file, Options { target: bank, allow_unbootable_other, ..Options::default() })
        }
        UpdateCommand::SwitchBank { bank } => switch_bank(bank),
        UpdateCommand::ArmTrial { return_to } => arm_trial(return_to),
        UpdateCommand::WriteUserdata { image } => write_userdata(&image),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            crate::errln!("dphone update: {e}");
            1
        }
    }
}

fn status() -> Result<(), String> {
    let mut mtd = open_mtd()?;
    let dir = open_dir();
    let st = inspect(&mut mtd, &cmdline(), dir.exists().then_some(dir.as_path())).map_err(|e| e.to_string())?;
    let name = |b: Option<Bank>| b.map(bank_name).unwrap_or("neither bank");
    crate::outln!("running from:   {}", name(st.running));
    crate::outln!("ctrl active:    {}", name(st.active));
    crate::outln!("boot2 boots:    {}", bank_name(st.next_boot));
    crate::outln!(
        "ctrl record:    {}, {} valid copies ({} good blocks, {} bad){}",
        if st.ctrl_valid { "valid" } else { "INVALID" },
        st.ctrl_copies.0,
        st.ctrl_copies.1,
        st.ctrl_copies.2,
        if st.ctrl_consistent { "" } else { ", copies DISAGREE" },
    );
    for b in &st.banks {
        crate::outln!("bank {}:", bank_name(b.bank));
        crate::outln!("  header:       {}", if b.header.is_some() { "valid" } else { "none or bad CRC" });
        crate::outln!("  image:        {} ({})", b.image_name, b.build_date);
        crate::outln!("  version:      {}", b.version.as_deref().unwrap_or("?"));
        match &b.kernel {
            Ok(span) => crate::outln!("  kernel:       loadable ({span:?})"),
            Err(p) => crate::outln!("  kernel:       NOT loadable: {p}"),
        }
        crate::outln!("  bad blocks:   kernel {}, rootfs {}", b.kernel_bad_blocks, b.rootfs_bad_blocks);
        crate::outln!("  bootable:     {}", if b.bootable() { "yes" } else { "no" });
    }
    match &st.trial {
        Some(t) => crate::outln!("trial:          pending ({t:?})"),
        None => crate::outln!("trial:          none"),
    }
    Ok(())
}

fn switch_bank(bank: Bank) -> Result<(), String> {
    let model = model().ok_or("model unknown (no digium_board= and no devicetree match)")?;
    let mut mtd = open_mtd()?;
    select_bank(&mut mtd, model, bank).map_err(|e| e.to_string())?;
    // A manual choice ends any trial: the init role must not undo it.
    let _ = trial::clear(&open_dir());
    crate::outln!("bank {} selected for the next boot; 'dphone reboot' to boot it", bank_name(bank));
    Ok(())
}

fn arm_trial(return_to: Option<Bank>) -> Result<(), String> {
    let mut mtd = open_mtd()?;
    let record = trial::arm(&mut mtd, &cmdline(), &open_dir(), return_to).map_err(|e| e.to_string())?;
    let on_trial = record.trial_bank.unwrap_or(record.previous_bank.other());
    crate::outln!(
        "trial recorded: boot2 boots bank {} next; it must come up healthy within {} boots, or the phone returns to bank {}",
        bank_name(on_trial),
        record.max_boots,
        bank_name(record.previous_bank)
    );
    Ok(())
}

fn install(file: &Path, options: Options) -> Result<(), String> {
    let model = model().ok_or("model unknown (no digium_board= and no devicetree match)")?;
    let reader = BufReader::with_capacity(64 * 1024, File::open(file).map_err(|e| format!("{}: {e}", file.display()))?);
    let mut mtd = open_mtd()?;
    let parts = mtd.partitions().map_err(|e| e.to_string())?;
    if parts.is_empty() {
        return Err("no MTD partitions".into());
    }
    let mut last = String::new();
    let installed = Updater::new(&mut mtd, model, &cmdline())
        .options(options)
        .progress(|phase| {
            let text = format!("{phase:?}");
            if text != last {
                crate::out!("\r{text:<40}");
                crate::out::flush();
                last = text;
            }
        })
        .install_eff(reader)
        .map_err(|e| format!("\n{e}"))?;
    crate::outln!();
    crate::outln!(
        "installed version {} into bank {}{}; bank {} is kept as it was",
        installed.version,
        bank_name(installed.bank),
        if installed.is_stock { " (stock firmware)" } else { "" },
        bank_name(installed.previous),
    );
    let dir = open_dir();
    if dir.exists() {
        match trial::begin(&dir, &installed) {
            Ok(true) => crate::outln!("trial boot recorded: the phone returns to the kept bank if the new image does not come up healthy"),
            Ok(false) => {}
            Err(e) => crate::errln!("warning: could not record the trial boot: {e}"),
        }
    }
    crate::outln!("'dphone reboot' to boot it");
    Ok(())
}

/// Write a root file system image to `userdata`, which stock leaves unused.
/// Neither bank and nothing boot2 reads is touched.
fn write_userdata(file: &Path) -> Result<(), String> {
    const PART: &str = "userdata";
    // Streamed one erase block at a time ("-" is standard input): a stock
    // phone has too little free memory to hold the image twice.
    let mut input: Box<dyn Read> = if file == Path::new("-") {
        Box::new(std::io::stdin().lock())
    } else {
        Box::new(File::open(file).map_err(|e| format!("{}: {e}", file.display()))?)
    };
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    let mut mtd = open_mtd()?;
    let parts = mtd.partitions().map_err(|e| e.to_string())?;
    let info = parts.iter().find(|p| p.name == PART).ok_or("no partition named userdata")?.clone();
    if mounts.lines().any(|l| l.split_ascii_whitespace().next() == Some(&format!("/dev/mtdblock{}", info.index))) {
        return Err("userdata is mounted; refusing to overwrite it".into());
    }
    let mut part = mtd.open(PART).map_err(|e| e.to_string())?;
    crate::outln!("erasing {PART} ({} bytes)", info.size);
    let bad = dphone_hal::erase_all(part.as_mut(), |_, _| {}).map_err(|e| e.to_string())?;
    crate::outln!("writing ({bad} bad blocks skipped)");
    let bs = info.erase_size as usize;
    let mut block = vec![0u8; bs];
    let mut back = vec![0u8; bs];
    let mut off = 0u64;
    let mut total = 0usize;
    loop {
        let mut n = 0;
        while n < bs {
            match input.read(&mut block[n..]) {
                Ok(0) => break,
                Ok(k) => n += k,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(format!("{}: {e}", file.display())),
            }
        }
        if n == 0 {
            break;
        }
        let start = off;
        off = dphone_hal::write_skip_bad(part.as_mut(), start, &block[..n], |_, _| {}).map_err(|e| e.to_string())?;
        dphone_hal::read_skip_bad(part.as_mut(), start, &mut back[..n]).map_err(|e| e.to_string())?;
        if back[..n] != block[..n] {
            return Err(format!("read-back differs from the image in the block at {start:#x}"));
        }
        total += n;
        if n < bs {
            break;
        }
    }
    crate::outln!("wrote and verified {total} bytes; boot with root=/dev/mtdblock{} rootfstype=jffs2 ro", info.index);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Debug, Parser)]
    struct Cli {
        #[command(subcommand)]
        command: UpdateCommand,
    }

    fn parse(words: &[&str]) -> Result<UpdateCommand, clap::Error> {
        Cli::try_parse_from(std::iter::once(&"update").chain(words)).map(|c| c.command)
    }

    #[test]
    fn commands() {
        assert!(matches!(parse(&["status"]), Ok(UpdateCommand::Status)));
        let Ok(UpdateCommand::Install { file, bank, allow_unbootable_other }) = parse(&["install", "/tmp/fw.eff"]) else {
            panic!("install does not parse")
        };
        assert_eq!((file.as_path(), bank, allow_unbootable_other), (Path::new("/tmp/fw.eff"), None, false));
        let Ok(UpdateCommand::Install { bank, allow_unbootable_other, .. }) =
            parse(&["install", "--bank", "b", "fw.eff", "--allow-unbootable-other"])
        else {
            panic!("install with options does not parse")
        };
        assert_eq!((bank, allow_unbootable_other), (Some(Bank::B), true));
        assert!(matches!(parse(&["switch-bank", "A"]), Ok(UpdateCommand::SwitchBank { bank: Bank::A })));
        assert!(matches!(parse(&["arm-trial"]), Ok(UpdateCommand::ArmTrial { return_to: None })));
        assert!(matches!(parse(&["arm-trial", "--return-to", "a"]), Ok(UpdateCommand::ArmTrial { return_to: Some(Bank::A) })));
        assert!(matches!(parse(&["write-userdata", "-"]), Ok(UpdateCommand::WriteUserdata { .. })));
    }

    #[test]
    fn mistakes() {
        for bad in [
            &[][..],
            // An image is installed by the command that says so.
            &["fw.eff"],
            &["install"],
            &["install", "a.eff", "b.eff"],
            &["install", "a.eff", "--bank"],
            &["install", "a.eff", "--bank", "c"],
            &["switch-bank"],
            &["switch-bank", "2"],
            &["arm-trial", "a"],
            &["arm-trial", "--return-to"],
            &["write-userdata"],
            &["status", "now"],
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }
}
