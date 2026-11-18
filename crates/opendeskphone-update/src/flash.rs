//! Writing one image (kernel or root file system) into its partition.
//!
//! The layout is what boot2's partition reader and a JFFS2 mount expect,
//! and what stock `ota_ProgramImage` produces: the whole partition erased,
//! image data in consecutive **good** erase blocks from the start (a bad
//! block is skipped and the data continues in the next good one), nothing
//! written to the OOB area, and blocks past the image left erased.  JFFS2
//! on NAND keeps its clean markers in OOB; stock writes none, and a
//! read-only mount (which is how boot2's fixed command line mounts the
//! root) accepts erased blocks without them.
//!
//! Like stock, trailing `0xff` bytes of each block are not programmed.
//! Unlike stock, every block is read back and compared right after it is
//! written, and the caller follows up with [`readback_crc`] over the whole
//! image.

use std::io::Read;

use opendeskphone_hal::MtdPartition;
use eff::crc::Crc;

use crate::error::{Result, UpdateError};

/// What to do when a block fails to erase, program, or read back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OnBlockFailure {
    /// Stop; the update fails with the target bank left invalid.
    Fail,
    /// Mark the block bad and carry on with the next good block (what
    /// stock does on a failed write).
    MarkBad,
}

/// Bytes available for image data: good blocks times the block size.
pub(crate) fn capacity(part: &mut dyn MtdPartition) -> Result<(u64, u32)> {
    let info = part.info().clone();
    let bs = info.erase_size as u64;
    let (mut good, mut bad) = (0u64, 0u32);
    let mut off = 0;
    while off + bs <= info.size {
        if part.is_bad(off)? {
            bad += 1;
        } else {
            good += 1;
        }
        off += bs;
    }
    Ok((good * bs, bad))
}

/// Erases every good block.  A JFFS2 mount scans the whole partition, so a
/// block that kept old data would corrupt the new file system: an erase
/// failure is never ignored.
pub(crate) fn erase_all(
    part: &mut dyn MtdPartition,
    on_failure: OnBlockFailure,
    mut progress: impl FnMut(u64, u64),
) -> Result<()> {
    let info = part.info().clone();
    let bs = info.erase_size as u64;
    let mut off = 0;
    while off + bs <= info.size {
        if !part.is_bad(off)?
            && let Err(e) = part.erase(off)
        {
            match on_failure {
                OnBlockFailure::Fail => return Err(e.into()),
                OnBlockFailure::MarkBad => {
                    log::warn!("{}: erase failed at {off:#x} ({e}); marking the block bad", info.name);
                    part.mark_bad(off)?;
                }
            }
        }
        off += bs;
        progress(off, info.size);
    }
    Ok(())
}

fn read_full(r: &mut dyn Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// Programs `len` bytes into the (erased) partition: first `head`, then
/// the rest from `reader`.  Returns the number of bad blocks skipped.
pub(crate) fn program(
    part: &mut dyn MtdPartition,
    head: &[u8],
    reader: &mut dyn Read,
    len: u64,
    on_failure: OnBlockFailure,
    mut progress: impl FnMut(u64, u64),
) -> Result<u32> {
    let info = part.info().clone();
    let (bs, page) = (info.erase_size as usize, info.write_size as usize);
    if bs == 0 || page == 0 || bs % page != 0 {
        return Err(UpdateError::Layout(format!("{}: unusable geometry", info.name)));
    }
    let mut buf = vec![0xffu8; bs];
    let mut check = vec![0u8; bs];
    let mut head = head;
    let mut done = 0u64;
    let mut off = 0u64;
    let mut skipped = 0u32;
    while done < len {
        // Next block's worth of image data.
        let n = (len - done).min(bs as u64) as usize;
        let from_head = n.min(head.len());
        buf[..from_head].copy_from_slice(&head[..from_head]);
        head = &head[from_head..];
        if read_full(reader, &mut buf[from_head..n])? != n - from_head {
            return Err(UpdateError::Image(format!("image ends early while writing {}", info.name)));
        }
        // Stock leaves trailing 0xff unprogrammed and rounds up to a page.
        let used = buf[..n].iter().rposition(|b| *b != 0xff).map_or(0, |p| p + 1);
        let used = used.div_ceil(page) * page;
        buf[n..].fill(0xff);
        // Place it in the next good block that takes it.
        loop {
            if off + bs as u64 > info.size {
                return Err(UpdateError::Layout(format!(
                    "{}: out of good blocks with {} bytes left to write",
                    info.name,
                    len - done
                )));
            }
            if part.is_bad(off)? {
                skipped += 1;
                off += bs as u64;
                continue;
            }
            let attempt = (|| -> Result<()> {
                if used > 0 {
                    part.write(off, &buf[..used])?;
                }
                // The whole block must read back as data followed by 0xff.
                part.read(off, &mut check)?;
                if check != buf {
                    return Err(UpdateError::Verify(format!(
                        "{}: block at {off:#x} reads back wrong",
                        info.name
                    )));
                }
                Ok(())
            })();
            match (attempt, on_failure) {
                (Ok(()), _) => break,
                (Err(e), OnBlockFailure::Fail) => return Err(e),
                (Err(e), OnBlockFailure::MarkBad) => {
                    log::warn!("{e}; marking the block bad");
                    // Leave no stale data for a JFFS2 scan of a block the
                    // kernel might still read if the mark does not stick.
                    let _ = part.erase(off);
                    part.mark_bad(off)?;
                    skipped += 1;
                    off += bs as u64;
                }
            }
        }
        off += bs as u64;
        done += n as u64;
        progress(done, len);
    }
    Ok(skipped)
}

/// Reads `len` image bytes back with bad-block skipping, as boot2 will,
/// and returns their CRC (the OTA's CRC form).  With `expect_erased`, also
/// requires everything after the image, to the end of the partition, to
/// read as erased.
pub(crate) fn readback_crc(
    part: &mut dyn MtdPartition,
    len: u64,
    expect_erased: bool,
    mut progress: impl FnMut(u64, u64),
) -> Result<u32> {
    let info = part.info().clone();
    let bs = info.erase_size as usize;
    let mut buf = vec![0u8; bs];
    let mut crc = Crc::new();
    let mut done = 0u64;
    let mut off = 0u64;
    while off + bs as u64 <= info.size {
        if !part.is_bad(off)? {
            if done < len || expect_erased {
                part.read(off, &mut buf)?;
            }
            let n = (len - done).min(bs as u64) as usize;
            crc.update(&buf[..n]);
            done += n as u64;
            if expect_erased && buf[n..].iter().any(|b| *b != 0xff) {
                return Err(UpdateError::Verify(format!(
                    "{}: block at {off:#x} is not erased past the image",
                    info.name
                )));
            }
            if done == len && !expect_erased {
                break;
            }
        }
        off += bs as u64;
        progress(done, len);
    }
    if done < len {
        return Err(UpdateError::Verify(format!("{}: the image's length does not fit its good blocks", info.name)));
    }
    Ok(crc.value())
}

pub(crate) fn percent(done: u64, total: u64) -> u8 {
    (done.min(total) * 100).checked_div(total).unwrap_or(100) as u8
}

/// Reads a bank back the way boot2 and the kernel will (good blocks in
/// order, through the MTD driver's error correction) and compares it with
/// the image its header describes: each component's length and CRC,
/// nothing but erased flash after it, and a kernel header boot2 accepts.
/// The updater does this before it selects a bank; `dphone update status`
/// does it for both banks.  A mismatch is [`UpdateError::Verify`].
pub(crate) fn verify_image(
    kpart: &mut dyn MtdPartition,
    rpart: &mut dyn MtdPartition,
    app: &eff::ota::AppHeader,
    mut progress: impl FnMut(u8),
) -> Result<()> {
    let total = app.kernel_size as u64 + app.rootfs_size as u64;
    let kcrc = readback_crc(kpart, app.kernel_size as u64, true, |d, _| progress(percent(d, total)))?;
    if kcrc != app.kernel_crc {
        return Err(UpdateError::Verify(format!(
            "kernel reads back with CRC {kcrc:#010x}, not the image's {:#010x}",
            app.kernel_crc
        )));
    }
    let rcrc =
        readback_crc(rpart, app.rootfs_size as u64, true, |d, _| progress(percent(app.kernel_size as u64 + d, total)))?;
    if rcrc != app.rootfs_crc {
        return Err(UpdateError::Verify(format!(
            "root file system reads back with CRC {rcrc:#010x}, not the image's {:#010x}",
            app.rootfs_crc
        )));
    }
    if let Err(p) = crate::boot2::check_stored_kernel(kpart)? {
        return Err(UpdateError::Verify(format!("stored kernel: {p}")));
    }
    Ok(())
}

/// Reads `buf.len()` bytes from the start of the partition's good blocks,
/// under boot2's rule that a whole block must remain in the partition for
/// each step.
pub(crate) fn read_skip_bad(part: &mut dyn MtdPartition, buf: &mut [u8]) -> Result<bool> {
    let info = part.info().clone();
    let bs = info.erase_size as usize;
    let mut done = 0;
    let mut off = 0u64;
    while done < buf.len() {
        if off + bs as u64 > info.size {
            return Ok(false);
        }
        if !part.is_bad(off)? {
            let n = bs.min(buf.len() - done);
            part.read(off, &mut buf[done..done + n])?;
            done += n;
        }
        off += bs as u64;
    }
    Ok(true)
}
