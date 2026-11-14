//! A model of what the stock boot2 does with the flash contents, used to
//! check images before they are committed, to report the next boot in
//! `inspect`, and as the judge in the power-cut tests.
//!
//! From the boot2 disassembly (both families; `kernel-re/boot/NOTES.md`):
//! read `ctrl` (see [`crate::ctrl::scan`]), choose a side, read the first
//! 0x30 bytes of that side's kernel partition, require the zImage magic at
//! 0x24, take the load span from words 0x28 (start) and 0x2c (end), and
//! copy `end - start` bytes from the partition's good blocks to `start`.
//! The sizes and CRCs in the `ctrl` headers are printed and otherwise
//! unused.  boot2 hangs (it does not fall back) when the span is empty or
//! the partition runs out.

use dphone_hal::{Mtd, MtdPartition};

use crate::bank::{self, Bank};
use crate::ctrl::{self, CTRL_PART, CtrlScan};
use crate::error::Result;
use crate::flash;

/// Little-endian word at 0x24 of an ARM zImage.
pub const ZIMAGE_MAGIC: u32 = 0x016f_2818;
/// Bytes of the kernel boot2 inspects before loading.
pub const KERNEL_HEADER_LEN: usize = 0x30;

/// Lowest load address accepted.  boot2 itself runs at 0x4000 and puts the
/// tag list at the start of RAM (0x00200004 or 0x00400004); stock kernels
/// load at 0x0d000000 and ours at 0x02000000.
pub const MIN_LOAD_ADDR: u32 = 0x0100_0000;
/// Highest end address accepted: boot2's own data sits just under 128 MiB
/// (0x07ff4644 and up), and the smallest phones have 128 MiB of RAM.
pub const MAX_LOAD_END: u32 = 0x0780_0000;
/// Addresses are taken modulo this (128 MiB) before the two bounds apply.
pub const RAM_ALIAS: u32 = 0x0800_0000;

/// The load span in a kernel image's header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KernelSpan {
    pub start: u32,
    pub end: u32,
}

impl KernelSpan {
    pub fn len(&self) -> u32 {
        self.end - self.start
    }
    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }
}

/// Why boot2 would not get a kernel running from an image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KernelProblem {
    /// Not a zImage.  (boot2 then tries its `boot_img` path, which no
    /// firmware uses; treated as unbootable here.)
    NotZimage,
    /// `start >= end`: boot2 prints "kstart >= kend" and hangs.
    EmptySpan,
    /// The load span would overwrite boot2, the tag list, or run past RAM.
    /// A zImage straight out of a mainline build has start 0 and is in
    /// this class: boot2 would copy it over itself.
    UnsafeSpan(KernelSpan),
    /// The span is longer than what is stored (or than the partition's
    /// good blocks): boot2 would load unwritten flash or hang.
    TooLong { span: u32, available: u64 },
}

impl std::fmt::Display for KernelProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KernelProblem::NotZimage => f.write_str("kernel is not a zImage (no magic at 0x24)"),
            KernelProblem::EmptySpan => f.write_str("kernel load span is empty (boot2 would hang)"),
            KernelProblem::UnsafeSpan(s) => write!(
                f,
                "kernel load span {:#x}..{:#x} does not land in {MIN_LOAD_ADDR:#x}..{MAX_LOAD_END:#x} of RAM \
                 (image not header-patched for boot2?)",
                s.start, s.end
            ),
            KernelProblem::TooLong { span, available } => {
                write!(f, "kernel load span is {span} bytes but only {available} are stored")
            }
        }
    }
}

/// Checks a kernel image's first bytes the way boot2 will, plus the safety
/// bounds boot2 does not check.  `available` is how many bytes of kernel
/// are (or will be) stored.
pub fn check_kernel_header(head: &[u8], available: u64) -> std::result::Result<KernelSpan, KernelProblem> {
    if head.len() < KERNEL_HEADER_LEN {
        return Err(KernelProblem::NotZimage);
    }
    let word = |o: usize| u32::from_le_bytes(head[o..o + 4].try_into().unwrap());
    if word(0x24) != ZIMAGE_MAGIC {
        return Err(KernelProblem::NotZimage);
    }
    let span = KernelSpan { start: word(0x28), end: word(0x2c) };
    if span.start >= span.end {
        return Err(KernelProblem::EmptySpan);
    }
    // Stock kernels load at 0x0d000000, above the 128 MiB of RAM that the
    // smaller phones have: the address wraps within RAM.  Judge the span
    // where it lands in the first 128 MiB.
    let low = span.start & (RAM_ALIAS - 1);
    if low < MIN_LOAD_ADDR || low as u64 + span.len() as u64 > MAX_LOAD_END as u64 {
        return Err(KernelProblem::UnsafeSpan(span));
    }
    if span.len() as u64 > available {
        return Err(KernelProblem::TooLong { span: span.len(), available });
    }
    Ok(span)
}

/// The outcome of a simulated boot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Boot {
    /// What the `ctrl` walk found.
    pub ctrl: CtrlScan,
    /// The side boot2 picks.
    pub side: Bank,
    /// The kernel it would load, or why it would not.
    pub kernel: std::result::Result<KernelSpan, KernelProblem>,
}

/// Checks the kernel stored in a partition as boot2 would load it.
pub fn check_stored_kernel(part: &mut dyn MtdPartition) -> Result<std::result::Result<KernelSpan, KernelProblem>> {
    let (available, _) = flash::capacity(part)?;
    let mut head = [0u8; KERNEL_HEADER_LEN];
    if !flash::read_skip_bad(part, &mut head)? {
        return Ok(Err(KernelProblem::NotZimage));
    }
    Ok(check_kernel_header(&head, available))
}

/// Which side boot2 would boot from the current flash contents, and
/// whether that side's kernel would load.
pub fn simulate(mtd: &mut dyn Mtd) -> Result<Boot> {
    let scan = ctrl::scan(mtd.open(CTRL_PART)?.as_mut())?;
    let side = bank::from_index(ctrl::boot_side(&scan)).unwrap_or(Bank::A);
    let kernel = check_stored_kernel(mtd.open(bank::kernel_part(side))?.as_mut())?;
    Ok(Boot { ctrl: scan, side, kernel })
}

/// The bytes boot2 would copy to RAM for a side (for tests and for
/// `dphone update status --verify`).
pub fn load_kernel(mtd: &mut dyn Mtd, side: Bank) -> Result<std::result::Result<Vec<u8>, KernelProblem>> {
    let mut part = mtd.open(bank::kernel_part(side))?;
    let span = match check_stored_kernel(part.as_mut())? {
        Ok(s) => s,
        Err(p) => return Ok(Err(p)),
    };
    let mut image = vec![0u8; span.len() as usize];
    if !flash::read_skip_bad(part.as_mut(), &mut image)? {
        return Ok(Err(KernelProblem::TooLong { span: span.len(), available: 0 }));
    }
    Ok(Ok(image))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(start: u32, end: u32) -> Vec<u8> {
        let mut h = vec![0u8; 0x30];
        h[0x24..0x28].copy_from_slice(&ZIMAGE_MAGIC.to_le_bytes());
        h[0x28..0x2c].copy_from_slice(&start.to_le_bytes());
        h[0x2c..0x30].copy_from_slice(&end.to_le_bytes());
        h
    }

    #[test]
    fn header_checks() {
        // Stock D60 k0 (from the NAND dump) and D40 k0.
        assert!(check_kernel_header(&header(0x0d00_0000, 0x0d1d_f000), 0x1df000).is_ok());
        assert!(check_kernel_header(&header(0x0d00_0000, 0x0d15_f1e0), 0x15f1e0).is_ok());
        // Ours.
        assert!(check_kernel_header(&header(0x0200_0000, 0x0200_0000 + 1_760_488), 1_760_488).is_ok());
        // An unpatched mainline zImage: start 0.
        assert!(matches!(
            check_kernel_header(&header(0, 1_760_488), 1_760_488),
            Err(KernelProblem::UnsafeSpan(_))
        ));
        assert_eq!(check_kernel_header(&header(0x0200_0000, 0x0200_0000), 100), Err(KernelProblem::EmptySpan));
        assert_eq!(check_kernel_header(&header(0x0200_0010, 0x0200_0000), 100), Err(KernelProblem::EmptySpan));
        assert!(matches!(
            check_kernel_header(&header(0x0200_0000, 0x0200_1000), 0xfff),
            Err(KernelProblem::TooLong { .. })
        ));
        assert!(matches!(
            check_kernel_header(&header(0x0700_0000, 0x0790_0000), 0x90_0000),
            Err(KernelProblem::UnsafeSpan(_))
        ));
        assert_eq!(check_kernel_header(&[0u8; 0x30], 100), Err(KernelProblem::NotZimage));
        assert_eq!(check_kernel_header(&[0u8; 8], 100), Err(KernelProblem::NotZimage));
    }
}
