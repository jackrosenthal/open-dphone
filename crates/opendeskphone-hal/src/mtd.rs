//! NAND flash through MTD, for the updater.

use crate::{HalError, Result};

/// Geometry of one partition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MtdInfo {
    /// Partition name (`k0`, `root1`, `ctrl`, `nvdata`, ...), from
    /// `/proc/mtd`; offsets come from boot2's `mtdparts=`, never from us.
    pub name: String,
    /// `mtdN`.
    pub index: u32,
    pub size: u64,
    /// Erase block size (128 KiB on these phones).
    pub erase_size: u32,
    /// Minimum write unit (the NAND page).
    pub write_size: u32,
    pub oob_size: u32,
}

/// One open partition.  Offsets are relative to the partition start.
pub trait MtdPartition: Send {
    fn info(&self) -> &MtdInfo;
    /// Whether the erase block containing `offset` is marked bad.
    fn is_bad(&mut self, offset: u64) -> Result<bool>;
    /// Erases the erase block at `offset` (must be block-aligned).
    fn erase(&mut self, offset: u64) -> Result<()>;
    /// Reads raw data (ECC-corrected by the kernel).
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> Result<()>;
    /// Writes to erased flash.  `offset` must be page-aligned; a final
    /// partial page is padded with 0xFF.
    fn write(&mut self, offset: u64, data: &[u8]) -> Result<()>;
    /// Marks the block containing `offset` bad.
    fn mark_bad(&mut self, offset: u64) -> Result<()>;
}

/// The flash partitions.
pub trait Mtd: Send {
    fn partitions(&self) -> Result<Vec<MtdInfo>>;
    /// Opens a partition by name.
    fn open(&mut self, name: &str) -> Result<Box<dyn MtdPartition>>;
}

fn check_block_aligned(info: &MtdInfo, offset: u64) -> Result<()> {
    if !offset.is_multiple_of(info.erase_size as u64) {
        return Err(HalError::Invalid(format!("offset {offset:#x} is not erase-block aligned")));
    }
    Ok(())
}

/// Erases every good block of the partition; returns the number of bad
/// blocks skipped.
pub fn erase_all(part: &mut dyn MtdPartition, mut progress: impl FnMut(u64, u64)) -> Result<u32> {
    let info = part.info().clone();
    let bs = info.erase_size as u64;
    let mut bad = 0;
    let mut off = 0;
    while off < info.size {
        if part.is_bad(off)? {
            bad += 1;
        } else {
            part.erase(off)?;
        }
        off += bs;
        progress(off, info.size);
    }
    Ok(bad)
}

/// Writes `data` starting at block-aligned `start`, skipping bad blocks
/// the way boot2's reader does (data continues in the next good block).
/// The target blocks must already be erased.  Returns the offset just past
/// the last block used.
pub fn write_skip_bad(
    part: &mut dyn MtdPartition,
    start: u64,
    data: &[u8],
    mut progress: impl FnMut(usize, usize),
) -> Result<u64> {
    let info = part.info().clone();
    check_block_aligned(&info, start)?;
    let bs = info.erase_size as usize;
    let mut off = start;
    let mut done = 0;
    while done < data.len() {
        if off >= info.size {
            return Err(HalError::Invalid(format!(
                "data does not fit in {}: {} bytes left at end of partition",
                info.name,
                data.len() - done
            )));
        }
        if part.is_bad(off)? {
            off += bs as u64;
            continue;
        }
        let n = bs.min(data.len() - done);
        part.write(off, &data[done..done + n])?;
        done += n;
        off += bs as u64;
        progress(done, data.len());
    }
    Ok(off)
}

/// Reads `buf.len()` bytes starting at block-aligned `start`, skipping bad
/// blocks.  Returns the offset just past the last block used.
pub fn read_skip_bad(part: &mut dyn MtdPartition, start: u64, buf: &mut [u8]) -> Result<u64> {
    let info = part.info().clone();
    check_block_aligned(&info, start)?;
    let bs = info.erase_size as usize;
    let mut off = start;
    let mut done = 0;
    while done < buf.len() {
        if off >= info.size {
            return Err(HalError::Invalid(format!("read past the end of {}", info.name)));
        }
        if part.is_bad(off)? {
            off += bs as u64;
            continue;
        }
        let n = bs.min(buf.len() - done);
        part.read(off, &mut buf[done..done + n])?;
        done += n;
        off += bs as u64;
    }
    Ok(off)
}
