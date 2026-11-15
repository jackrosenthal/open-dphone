//! What the early-boot stage decides, kept apart from the system calls
//! that carry it out so that it is tested on the build machine.

use crate::flash::Replication;
use crate::trial::Trial;

/// The value of the last `key=value` word (`key` includes the `=`) on a
/// kernel command line; the last one wins, as in the kernel.
pub fn param<'a>(cmdline: &'a str, key: &str) -> Option<&'a str> {
    cmdline.split_ascii_whitespace().filter_map(|t| t.strip_prefix(key)).next_back()
}

/// Whether the command line asks for a read-only root (`ro`, not undone by
/// a later `rw`).
pub fn read_only(cmdline: &str) -> bool {
    cmdline.split_ascii_whitespace().rfind(|t| *t == "ro" || *t == "rw") == Some("ro")
}

/// The name of MTD partition `index`, from `/proc/mtd` text
/// (`mtd6: 02d00000 00020000 "root0"`).
pub fn mtd_name(proc_mtd: &str, index: u32) -> Option<&str> {
    proc_mtd.lines().find_map(|line| {
        let (dev, rest) = line.split_once(':')?;
        if dev.trim().strip_prefix("mtd")?.parse::<u32>().ok()? != index {
            return None;
        }
        let (q1, q2) = (rest.find('"')?, rest.rfind('"')?);
        rest.get(q1 + 1..q2)
    })
}

/// The index of the MTD partition called `name`.
pub fn mtd_index(proc_mtd: &str, name: &str) -> Option<u32> {
    proc_mtd.lines().find_map(|line| {
        let (dev, rest) = line.split_once(':')?;
        let index = dev.trim().strip_prefix("mtd")?.parse::<u32>().ok()?;
        let (q1, q2) = (rest.find('"')?, rest.rfind('"')?);
        (rest.get(q1 + 1..q2)? == name).then_some(index)
    })
}

/// The bank (0 or 1) the kernel was booted from: the partition that
/// `root=/dev/mtdblockN` names, when it is `root0` or `root1`.  `None` for
/// a system started from RAM (`dphone ramboot`: `root=mtd:ramroot`) or a
/// test root elsewhere: such a system is not the image a trial is about.
pub fn running_bank(cmdline: &str, proc_mtd: &str) -> Option<u8> {
    let index = param(cmdline, "root=")?.strip_prefix("/dev/mtdblock")?.parse().ok()?;
    match mtd_name(proc_mtd, index)? {
        "root0" => Some(0),
        "root1" => Some(1),
        _ => None,
    }
}

/// The `ctrl` layout of the phone: by `digium_board=` on boot2's command
/// line.  The models of the new family get a copy in every page; so does
/// a board that is not named, because boot2 and stock of both families
/// read only the first page of each block.
pub fn replication(cmdline: &str) -> Replication {
    match param(cmdline, "digium_board=") {
        Some("D40" | "D45" | "D50" | "D70") => Replication::BlockStart,
        _ => Replication::EveryPage,
    }
}

/// What to do about the trial record before the root is mounted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing is on trial in this boot; the record, if any, stays as it is.
    Boot,
    /// This boot counts against the trial: store the record, then boot.
    /// When the root cannot be mounted or has no init, go back at once.
    Count(Trial),
    /// The trial has used up its boots: select the previous bank, remove
    /// the record, and restart.
    GoBack(Trial),
}

/// Decides from the bank that was booted and the record in nvdata.
pub fn step(running: Option<u8>, trial: Option<Trial>) -> Step {
    let (Some(running), Some(mut trial)) = (running, trial) else { return Step::Boot };
    // A record about the other bank, or one left behind after the phone
    // already went back, is not about this boot.
    if running == trial.previous_bank || trial.trial_bank.is_some_and(|t| t != running) {
        return Step::Boot;
    }
    trial.boots = trial.boots.saturating_add(1);
    if trial.boots > trial.max_boots { Step::GoBack(trial) } else { Step::Count(trial) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MTD: &str = "dev:    size   erasesize  name\n\
                       mtd0: 00020000 00020000 \"boot1\"\n\
                       mtd2: 00080000 00020000 \"ctrl\"\n\
                       mtd3: 00ee0000 00020000 \"nvdata\"\n\
                       mtd6: 02d00000 00020000 \"root0\"\n\
                       mtd7: 02d00000 00020000 \"root1\"\n\
                       mtd8: 01000000 00020000 \"userdata\"\n";
    const STOCK: &str = "console=ttyAMA0,115200n8 vmalloc=256m  digium_board=D60 digium_boot_rev=3 \
                         mtdparts=bcm_umi-nand:0x00020000(boot1) root=/dev/mtdblock7 rootfstype=jffs2 ro bootmemheap";

    #[test]
    fn command_line_and_partitions() {
        assert_eq!(param(STOCK, "root="), Some("/dev/mtdblock7"));
        assert_eq!(param(STOCK, "rootfstype="), Some("jffs2"));
        assert_eq!(param("root=a root=b", "root="), Some("b"));
        assert_eq!(param(STOCK, "init="), None);
        assert!(read_only(STOCK));
        assert!(!read_only("root=/dev/mtdblock7 ro rw"));
        assert!(!read_only("root=/dev/mtdblock7"));
        assert_eq!(mtd_index(MTD, "ctrl"), Some(2));
        assert_eq!(mtd_index(MTD, "root"), None);
        assert_eq!(mtd_name(MTD, 3), Some("nvdata"));
        assert_eq!(mtd_name(MTD, 1), None);
        assert_eq!(running_bank(STOCK, MTD), Some(1));
        assert_eq!(running_bank("root=/dev/mtdblock6 ro", MTD), Some(0));
        // A root in userdata, the RAM root of 'dphone ramboot', and no root.
        assert_eq!(running_bank("root=/dev/mtdblock8 ro", MTD), None);
        assert_eq!(running_bank("root=mtd:ramroot rootfstype=jffs2 ro phram.phram=ramroot,0x6ec0000,0x1140000,0x20000", MTD), None);
        assert_eq!(running_bank("", MTD), None);
        assert_eq!(replication(STOCK), Replication::EveryPage);
        assert_eq!(replication("digium_board=D40 root=/dev/mtdblock6"), Replication::BlockStart);
        assert_eq!(replication("root=/dev/mtdblock6"), Replication::EveryPage);
    }

    #[test]
    fn trial_steps() {
        let t = Trial { previous_bank: 1, trial_bank: Some(0), boots: 0, max_boots: 3 };
        // No record, or a system started from RAM: nothing is counted.
        assert_eq!(step(Some(0), None), Step::Boot);
        assert_eq!(step(None, Some(t)), Step::Boot);
        // The bank on trial: three boots are counted, the fourth goes back.
        let mut now = t;
        for boots in 1..=3 {
            let Step::Count(next) = step(Some(0), Some(now)) else { panic!("boot {boots} is not counted") };
            assert_eq!(next.boots, boots);
            now = next;
        }
        assert_eq!(step(Some(0), Some(now)), Step::GoBack(Trial { boots: 4, ..t }));
        // Before the first boot of the new bank, and after going back, the
        // phone runs the previous bank: the record is not about this boot.
        assert_eq!(step(Some(1), Some(t)), Step::Boot);
        // A record without the bank on trial (an older writer) counts in
        // the bank that is not the previous one.
        let old = Trial { trial_bank: None, ..t };
        assert!(matches!(step(Some(0), Some(old)), Step::Count(_)));
        assert_eq!(step(Some(1), Some(old)), Step::Boot);
    }
}
