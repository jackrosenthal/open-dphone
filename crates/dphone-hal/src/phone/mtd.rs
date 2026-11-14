//! NAND partitions through the MTD character devices.

use super::sys::*;
use crate::*;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::fs::FileExt;
use std::path::PathBuf;

#[repr(C)]
#[derive(Default)]
pub(crate) struct MtdInfoUser {
    type_: u8,
    flags: u32,
    size: u32,
    erasesize: u32,
    writesize: u32,
    oobsize: u32,
    padding: u64,
}

#[repr(C)]
#[derive(Default)]
pub(crate) struct EraseInfoUser {
    start: u32,
    length: u32,
}

pub(crate) const MEMGETINFO: u32 = ior::<MtdInfoUser>(b'M', 1);
pub(crate) const MEMERASE: u32 = iow::<EraseInfoUser>(b'M', 2);
pub(crate) const MEMGETBADBLOCK: u32 = iow::<i64>(b'M', 11);
pub(crate) const MEMSETBADBLOCK: u32 = iow::<i64>(b'M', 12);

/// Parses `/proc/mtd`: `mtd4: 00200000 00020000 "k0"`.
pub(crate) fn parse_proc_mtd(text: &str) -> Vec<MtdInfo> {
    text.lines()
        .filter_map(|line| {
            let (dev, rest) = line.split_once(':')?;
            let index = dev.trim().strip_prefix("mtd")?.parse().ok()?;
            let mut it = rest.split_ascii_whitespace();
            let size = u64::from_str_radix(it.next()?, 16).ok()?;
            let erase_size = u32::from_str_radix(it.next()?, 16).ok()?;
            let q1 = rest.find('"')?;
            let q2 = rest.rfind('"')?;
            let name = rest.get(q1 + 1..q2)?.to_string();
            Some(MtdInfo { name, index, size, erase_size, write_size: 0, oob_size: 0 })
        })
        .collect()
}

/// The flash partitions of the phone.
pub struct PhoneMtd;

impl PhoneMtd {
    pub fn open() -> Result<PhoneMtd> {
        if !std::path::Path::new("/proc/mtd").exists() {
            return Err(HalError::Unavailable("/proc/mtd".into()));
        }
        Ok(PhoneMtd)
    }
}

impl Mtd for PhoneMtd {
    fn partitions(&self) -> Result<Vec<MtdInfo>> {
        let text = std::fs::read_to_string("/proc/mtd").map_err(|e| HalError::io("/proc/mtd", e))?;
        Ok(parse_proc_mtd(&text))
    }

    fn open(&mut self, name: &str) -> Result<Box<dyn MtdPartition>> {
        let mut info = self
            .partitions()?
            .into_iter()
            .find(|p| p.name == name)
            .ok_or_else(|| HalError::Unavailable(format!("no MTD partition named {name}")))?;
        let path = PathBuf::from(format!("/dev/mtd{}", info.index));
        let file = open_rw(&path, 0).map_err(|e| HalError::io(path.display().to_string(), e))?;
        let mut u = MtdInfoUser::default();
        ioctl(file.as_raw_fd(), MEMGETINFO, &mut u).map_err(|e| HalError::io("MEMGETINFO", e))?;
        info.size = u.size as u64;
        info.erase_size = u.erasesize;
        info.write_size = u.writesize.max(1);
        info.oob_size = u.oobsize;
        Ok(Box::new(PhonePartition { file, info }))
    }
}

struct PhonePartition {
    file: File,
    info: MtdInfo,
}

impl PhonePartition {
    fn check(&self, offset: u64, len: usize) -> Result<()> {
        if offset.checked_add(len as u64).is_none_or(|end| end > self.info.size) {
            return Err(HalError::Invalid(format!(
                "{offset:#x}+{len:#x} is outside {} ({:#x} bytes)",
                self.info.name, self.info.size
            )));
        }
        Ok(())
    }
}

impl MtdPartition for PhonePartition {
    fn info(&self) -> &MtdInfo {
        &self.info
    }

    fn is_bad(&mut self, offset: u64) -> Result<bool> {
        self.check(offset, 0)?;
        let mut off = (offset - offset % self.info.erase_size as u64) as i64;
        let r = ioctl(self.file.as_raw_fd(), MEMGETBADBLOCK, &mut off)
            .map_err(|e| HalError::io("MEMGETBADBLOCK", e))?;
        Ok(r > 0)
    }

    fn erase(&mut self, offset: u64) -> Result<()> {
        let bs = self.info.erase_size as u64;
        if !offset.is_multiple_of(bs) {
            return Err(HalError::Invalid(format!("erase offset {offset:#x} not block aligned")));
        }
        self.check(offset, bs as usize)?;
        let mut e = EraseInfoUser { start: offset as u32, length: bs as u32 };
        ioctl(self.file.as_raw_fd(), MEMERASE, &mut e)
            .map_err(|e| HalError::io(format!("erase {} at {offset:#x}", self.info.name), e))?;
        Ok(())
    }

    fn read(&mut self, offset: u64, buf: &mut [u8]) -> Result<()> {
        self.check(offset, buf.len())?;
        self.file
            .read_exact_at(buf, offset)
            .map_err(|e| HalError::io(format!("read {} at {offset:#x}", self.info.name), e))
    }

    fn write(&mut self, offset: u64, data: &[u8]) -> Result<()> {
        let page = self.info.write_size as usize;
        if !offset.is_multiple_of(page as u64) {
            return Err(HalError::Invalid(format!("write offset {offset:#x} not page aligned")));
        }
        self.check(offset, data.len())?;
        let whole = data.len() - data.len() % page;
        let ctx = |e| HalError::io(format!("write {} at {offset:#x}", self.info.name), e);
        self.file.write_all_at(&data[..whole], offset).map_err(ctx)?;
        if whole < data.len() {
            let mut last = vec![0xffu8; page];
            last[..data.len() - whole].copy_from_slice(&data[whole..]);
            self.file.write_all_at(&last, offset + whole as u64).map_err(ctx)?;
        }
        Ok(())
    }

    fn mark_bad(&mut self, offset: u64) -> Result<()> {
        self.check(offset, 0)?;
        let mut off = (offset - offset % self.info.erase_size as u64) as i64;
        ioctl(self.file.as_raw_fd(), MEMSETBADBLOCK, &mut off)
            .map_err(|e| HalError::io("MEMSETBADBLOCK", e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_mtd() {
        let text = "dev:    size   erasesize  name\n\
                    mtd0: 00020000 00020000 \"boot1\"\n\
                    mtd4: 00200000 00020000 \"k0\"\n\
                    mtd7: 02d00000 00020000 \"root1\"\n";
        let p = parse_proc_mtd(text);
        assert_eq!(p.len(), 3);
        assert_eq!((p[1].name.as_str(), p[1].index, p[1].size, p[1].erase_size), ("k0", 4, 0x20_0000, 0x2_0000));
        assert_eq!(p[2].name, "root1");
    }
}
