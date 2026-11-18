//! Banks and how the running one is found.

use opendeskphone_hal::MtdInfo;
pub use opendeskphone_state::Bank;

/// Side number as `ctrl` and boot2 count it.
pub const fn index(bank: Bank) -> usize {
    match bank {
        Bank::A => 0,
        Bank::B => 1,
    }
}

pub const fn from_index(i: u32) -> Option<Bank> {
    match i {
        0 => Some(Bank::A),
        1 => Some(Bank::B),
        _ => None,
    }
}

/// Partition names of a bank's kernel and root file system.
pub const fn kernel_part(bank: Bank) -> &'static str {
    match bank {
        Bank::A => "k0",
        Bank::B => "k1",
    }
}

pub const fn rootfs_part(bank: Bank) -> &'static str {
    match bank {
        Bank::A => "root0",
        Bank::B => "root1",
    }
}

/// The MTD index in `root=/dev/mtdblockN` on a kernel command line.
pub fn root_mtd_index(cmdline: &str) -> Option<u32> {
    cmdline
        .split_ascii_whitespace()
        .filter_map(|t| t.strip_prefix("root=/dev/mtdblock"))
        .next_back()
        .and_then(|n| n.parse().ok())
}

/// The bank the running system was booted from: the partition that
/// `root=` names, looked up by index in the MTD table and matched by name.
/// `None` when the root is neither `root0` nor `root1` (e.g., a RAM boot
/// with the root file system in `userdata`).
pub fn running_bank(cmdline: &str, parts: &[MtdInfo]) -> Option<Bank> {
    let idx = root_mtd_index(cmdline)?;
    let part = parts.iter().find(|p| p.index == idx)?;
    [Bank::A, Bank::B].into_iter().find(|b| part.name == rootfs_part(*b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts() -> Vec<MtdInfo> {
        ["boot1", "boot2", "ctrl", "nvdata", "k0", "k1", "root0", "root1", "userdata"]
            .iter()
            .enumerate()
            .map(|(i, n)| MtdInfo {
                name: n.to_string(),
                index: i as u32,
                size: 0x20000,
                erase_size: 0x20000,
                write_size: 2048,
                oob_size: 64,
            })
            .collect()
    }

    #[test]
    fn running_bank_from_cmdline() {
        let p = parts();
        let stock = "console=ttyAMA0,115200n8 vmalloc=256m  digium_board=D60 digium_boot_rev=3 \
                     mtdparts=bcm_umi-nand:0x00020000(boot1) root=/dev/mtdblock7 rootfstype=jffs2 ro bootmemheap";
        assert_eq!(running_bank(stock, &p), Some(Bank::B));
        assert_eq!(running_bank("root=/dev/mtdblock6 ro", &p), Some(Bank::A));
        assert_eq!(running_bank("root=/dev/mtdblock8 ro", &p), None);
        assert_eq!(running_bank("root=/dev/nfs", &p), None);
        assert_eq!(running_bank("", &p), None);
        // The last root= wins, as in the kernel.
        assert_eq!(running_bank("root=/dev/mtdblock6 root=/dev/mtdblock7", &p), Some(Bank::B));
    }
}
