//! `dphone update ...`: firmware update commands that run in the calling
//! process, not through the diag socket, so they work over SSH or the serial
//! console even when the main role is down.
//!
//! This is also the way back to stock firmware: `dphone update FILE.eff`
//! accepts a stock EFF.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use dphone_hal::Mtd;
use dphone_hal::phone::PhoneMtd;
use dphone_update::{Bank, Options, Updater, inspect, select_bank, trial};

use crate::init::trial::TrialRecord;

const USAGE: &str = "\
usage: dphone update status
       dphone update FILE.eff [--bank a|b] [--allow-unbootable-other]
       dphone update switch-bank a|b
       dphone update write-userdata IMAGE.jffs2
       dphone update arm-trial [--return-to a|b]
  status       show both banks, the ctrl record, and what boot2 will boot next
  FILE.eff     install an image (Open DPhone or stock) into the bank that is
               not running, verify it, and select it for the next boot;
               reboot afterwards with 'dphone reboot'
  switch-bank  select a bank for the next boot without writing an image
               (refused unless that bank holds a bootable image)
  arm-trial    record a trial boot of the bank 'ctrl' selects for the next
               boot, for an image installed and selected by other means
               (e.g., by the stock updater, or with switch-bank, which
               must come first because it removes a trial record): Open
               DPhone returns to the other bank by itself if the image
               does not come up healthy within a few boots.  The bank to
               return to is the running one (root= on the kernel command
               line); when that is the selected bank itself or neither
               bank (a kernel started from RAM), name it with --return-to
  write-userdata
               erase the unused 'userdata' partition and write a root file
               system image there, for booting a test kernel from RAM with
               root=/dev/mtdblockN without touching either firmware bank
";

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

fn parse_bank(s: &str) -> Option<Bank> {
    match s.to_ascii_lowercase().as_str() {
        "a" | "0" => Some(Bank::A),
        "b" | "1" => Some(Bank::B),
        _ => None,
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

pub fn run(args: &[String]) -> i32 {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["status"] => status(),
        ["switch-bank", bank] => match parse_bank(bank) {
            Some(b) => switch_bank(b),
            None => Err(format!("unknown bank '{bank}' (a or b)")),
        },
        ["write-userdata", file] => write_userdata(file),
        ["arm-trial"] => arm_trial(None),
        ["arm-trial", "--return-to", bank] => match parse_bank(bank) {
            Some(b) => arm_trial(Some(b)),
            None => Err(format!("unknown bank '{bank}' (a or b)")),
        },
        [file, rest @ ..] if !file.starts_with('-') && *file != "help" => install(file, rest),
        _ => {
            crate::err!("{USAGE}");
            return 2;
        }
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

fn install(file: &str, rest: &[&str]) -> Result<(), String> {
    let mut options = Options::default();
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match *a {
            "--bank" => {
                let b = it.next().and_then(|b| parse_bank(b)).ok_or("--bank needs a or b")?;
                options.target = Some(b);
            }
            "--allow-unbootable-other" => options.allow_unbootable_other = true,
            other => return Err(format!("unknown option '{other}'\n{USAGE}")),
        }
    }
    let model = model().ok_or("model unknown (no digium_board= and no devicetree match)")?;
    let reader = BufReader::with_capacity(64 * 1024, File::open(Path::new(file)).map_err(|e| format!("{file}: {e}"))?);
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
fn write_userdata(file: &str) -> Result<(), String> {
    const PART: &str = "userdata";
    // Streamed one erase block at a time ("-" is standard input): a stock
    // phone has too little free memory to hold the image twice.
    let mut input: Box<dyn Read> = if file == "-" {
        Box::new(std::io::stdin().lock())
    } else {
        Box::new(File::open(Path::new(file)).map_err(|e| format!("{file}: {e}"))?)
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
                Err(e) => return Err(format!("{file}: {e}")),
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
