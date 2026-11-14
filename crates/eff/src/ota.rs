//! Type-0 (application) OTA container.
//!
//! ```text
//! 0x00 magic 0x7d9f8001   0x04 type (0)   0x08 type-header CRC
//! 0x0c kernel CRC  0x10 kernel size  0x14 rootfs CRC  0x18 rootfs size
//! 0x1c build date (64 bytes)   0x5c file name (64 bytes)
//! 0x9c application-header CRC
//! 0xa0 kernel, then rootfs
//! ```
//!
//! Stored integers are big-endian.  The header CRCs are computed over the
//! integers in little-endian form (the vendor code checksums after
//! converting to host order); component CRCs cover the raw bytes.

use crate::crc::{Crc, crc};
use crate::{Result, bad};
use std::io::{Read, Write};

pub const OTA_MAGIC: u32 = 0x7d9f_8001;
/// Length of the type header plus the application header.
pub const OTA_HEADER_LEN: usize = 160;
/// Length of the application header alone (also stored per bank in `ctrl`).
pub const APP_HEADER_LEN: usize = 148;
/// Update type of a kernel-plus-rootfs image.
pub const TYPE_APP: u32 = 0;

fn be(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn text_field(value: &str, what: &str) -> Result<[u8; 64]> {
    let bytes = value.as_bytes();
    if bytes.len() > 63 || !value.is_ascii() || bytes.contains(&0) {
        return bad(format!("{what} must be at most 63 non-NUL ASCII bytes"));
    }
    let mut out = [0u8; 64];
    out[..bytes.len()].copy_from_slice(bytes);
    Ok(out)
}

fn text_of(field: &[u8; 64]) -> String {
    let end = field.iter().position(|&b| b == 0).unwrap_or(64);
    String::from_utf8_lossy(&field[..end]).into_owned()
}

/// The 148-byte application header: component CRCs and sizes, build date,
/// and file name.  The same structure is stored once per bank in `ctrl`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppHeader {
    pub kernel_crc: u32,
    pub kernel_size: u32,
    pub rootfs_crc: u32,
    pub rootfs_size: u32,
    pub date: [u8; 64],
    pub name: [u8; 64],
}

impl AppHeader {
    /// Header for the given components.  `date` and `name` are free text
    /// (stock: `Tue Oct 27 13:05:08 2026` and `2_9_28_D60_firmware.bin`).
    pub fn new(kernel: &[u8], rootfs: &[u8], date: &str, name: &str) -> Result<Self> {
        if kernel.is_empty() || rootfs.is_empty() {
            return bad("kernel and rootfs must be nonempty");
        }
        if kernel.len() > u32::MAX as usize || rootfs.len() > u32::MAX as usize {
            return bad("components must fit 32-bit size fields");
        }
        Ok(AppHeader {
            kernel_crc: crc(kernel),
            kernel_size: kernel.len() as u32,
            rootfs_crc: crc(rootfs),
            rootfs_size: rootfs.len() as u32,
            date: text_field(date, "date")?,
            name: text_field(name, "name")?,
        })
    }

    pub fn date(&self) -> String {
        text_of(&self.date)
    }

    pub fn name(&self) -> String {
        text_of(&self.name)
    }

    pub fn set_date(&mut self, date: &str) -> Result<()> {
        self.date = text_field(date, "date")?;
        Ok(())
    }

    pub fn set_name(&mut self, name: &str) -> Result<()> {
        self.name = text_field(name, "name")?;
        Ok(())
    }

    /// The header CRC, over the little-endian form of the four integers
    /// followed by the 128 text bytes.
    pub fn crc(&self) -> u32 {
        let mut c = Crc::new();
        for v in [
            self.kernel_crc,
            self.kernel_size,
            self.rootfs_crc,
            self.rootfs_size,
        ] {
            c.update(&v.to_le_bytes());
        }
        c.update(&self.date);
        c.update(&self.name);
        c.value()
    }

    /// The on-flash and in-file form (big-endian integers, CRC last).
    pub fn to_bytes(&self) -> [u8; APP_HEADER_LEN] {
        let mut out = [0u8; APP_HEADER_LEN];
        out[0..4].copy_from_slice(&self.kernel_crc.to_be_bytes());
        out[4..8].copy_from_slice(&self.kernel_size.to_be_bytes());
        out[8..12].copy_from_slice(&self.rootfs_crc.to_be_bytes());
        out[12..16].copy_from_slice(&self.rootfs_size.to_be_bytes());
        out[16..80].copy_from_slice(&self.date);
        out[80..144].copy_from_slice(&self.name);
        out[144..148].copy_from_slice(&self.crc().to_be_bytes());
        out
    }

    /// Parse without checking the CRC; returns the header and whether its
    /// stored CRC matches.  (`ctrl` deliberately holds invalid headers.)
    pub fn from_bytes_unchecked(b: &[u8; APP_HEADER_LEN]) -> (Self, bool) {
        let mut date = [0u8; 64];
        let mut name = [0u8; 64];
        date.copy_from_slice(&b[16..80]);
        name.copy_from_slice(&b[80..144]);
        let h = AppHeader {
            kernel_crc: be(&b[0..4]),
            kernel_size: be(&b[4..8]),
            rootfs_crc: be(&b[8..12]),
            rootfs_size: be(&b[12..16]),
            date,
            name,
        };
        let valid = be(&b[144..148]) == h.crc();
        (h, valid)
    }

    pub fn from_bytes(b: &[u8; APP_HEADER_LEN]) -> Result<Self> {
        match Self::from_bytes_unchecked(b) {
            (h, true) => Ok(h),
            _ => bad("invalid OTA application-header CRC"),
        }
    }
}

/// CRC of the type header (magic and type, little-endian form).
pub fn type_crc(magic: u32, kind: u32) -> u32 {
    let mut c = Crc::new();
    c.update(&magic.to_le_bytes());
    c.update(&kind.to_le_bytes());
    c.value()
}

/// The complete 160-byte header of an application OTA.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OtaHeader {
    pub app: AppHeader,
}

impl OtaHeader {
    /// Parse and check magic, type, and both header CRCs.
    pub fn parse(b: &[u8; OTA_HEADER_LEN]) -> Result<Self> {
        let (magic, kind, sum) = (be(&b[0..4]), be(&b[4..8]), be(&b[8..12]));
        if magic != OTA_MAGIC {
            return bad("invalid OTA magic");
        }
        if kind != TYPE_APP {
            return bad(format!(
                "unsupported OTA type {kind}; only application updates (type 0) are supported"
            ));
        }
        if sum != type_crc(magic, kind) {
            return bad("invalid OTA type-header CRC");
        }
        let app = AppHeader::from_bytes(b[12..160].try_into().unwrap())?;
        if app.kernel_size == 0 || app.rootfs_size == 0 {
            return bad("OTA component is empty");
        }
        Ok(OtaHeader { app })
    }

    pub fn to_bytes(&self) -> [u8; OTA_HEADER_LEN] {
        let mut out = [0u8; OTA_HEADER_LEN];
        out[0..4].copy_from_slice(&OTA_MAGIC.to_be_bytes());
        out[4..8].copy_from_slice(&TYPE_APP.to_be_bytes());
        out[8..12].copy_from_slice(&type_crc(OTA_MAGIC, TYPE_APP).to_be_bytes());
        out[12..160].copy_from_slice(&self.app.to_bytes());
        out
    }

    /// Total file length this header describes.
    pub fn total_len(&self) -> u64 {
        OTA_HEADER_LEN as u64 + self.app.kernel_size as u64 + self.app.rootfs_size as u64
    }
}

/// A fully parsed OTA held in memory (host-side use).
#[derive(Debug)]
pub struct Ota<'a> {
    pub header: OtaHeader,
    pub kernel: &'a [u8],
    pub rootfs: &'a [u8],
}

impl<'a> Ota<'a> {
    /// Parse a whole OTA: header CRCs, exact length, and component CRCs.
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        if data.len() < OTA_HEADER_LEN {
            return bad("truncated OTA header");
        }
        let header = OtaHeader::parse(data[..OTA_HEADER_LEN].try_into().unwrap())?;
        if data.len() as u64 != header.total_len() {
            return bad("component sizes do not match file length");
        }
        let ks = header.app.kernel_size as usize;
        let kernel = &data[OTA_HEADER_LEN..OTA_HEADER_LEN + ks];
        let rootfs = &data[OTA_HEADER_LEN + ks..];
        if crc(kernel) != header.app.kernel_crc {
            return bad("invalid kernel CRC");
        }
        if crc(rootfs) != header.app.rootfs_crc {
            return bad("invalid rootfs CRC");
        }
        Ok(Ota {
            header,
            kernel,
            rootfs,
        })
    }
}

/// Build an OTA in memory.
pub fn pack(kernel: &[u8], rootfs: &[u8], date: &str, name: &str) -> Result<Vec<u8>> {
    let app = AppHeader::new(kernel, rootfs, date, name)?;
    pack_with_header(app, kernel, rootfs)
}

/// Build an OTA from an existing header whose text fields are kept verbatim;
/// sizes and CRCs are recomputed.
pub fn pack_with_header(mut app: AppHeader, kernel: &[u8], rootfs: &[u8]) -> Result<Vec<u8>> {
    let fresh = AppHeader::new(kernel, rootfs, "", "")?;
    app.kernel_crc = fresh.kernel_crc;
    app.kernel_size = fresh.kernel_size;
    app.rootfs_crc = fresh.rootfs_crc;
    app.rootfs_size = fresh.rootfs_size;
    let mut out = Vec::with_capacity(OTA_HEADER_LEN + kernel.len() + rootfs.len());
    out.extend_from_slice(&OtaHeader { app }.to_bytes());
    out.extend_from_slice(kernel);
    out.extend_from_slice(rootfs);
    Ok(out)
}

/// Write an OTA to a stream.
pub fn write<W: Write>(
    mut w: W,
    kernel: &[u8],
    rootfs: &[u8],
    date: &str,
    name: &str,
) -> Result<()> {
    let app = AppHeader::new(kernel, rootfs, date, name)?;
    w.write_all(&OtaHeader { app }.to_bytes())?;
    w.write_all(kernel)?;
    w.write_all(rootfs)?;
    Ok(())
}

/// Which component a [`ComponentReader`] yields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Component {
    Kernel,
    Rootfs,
}

/// Streaming OTA reader for the on-device updater: parse the header, then
/// read the kernel and the rootfs in order, each checked against its CRC and
/// size when it ends.
pub struct OtaReader<R: Read> {
    inner: R,
    header: OtaHeader,
    next: Option<Component>,
}

impl<R: Read> OtaReader<R> {
    /// Read and validate the 160-byte header.
    pub fn new(mut inner: R) -> Result<Self> {
        let mut b = [0u8; OTA_HEADER_LEN];
        inner.read_exact(&mut b).map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                crate::Error::Format("truncated OTA header".into())
            } else {
                e.into()
            }
        })?;
        Ok(OtaReader {
            inner,
            header: OtaHeader::parse(&b)?,
            next: Some(Component::Kernel),
        })
    }

    pub fn header(&self) -> &OtaHeader {
        &self.header
    }

    /// The next component, or `None` after the rootfs.  The previous
    /// component must have been read to its end (`ComponentReader::finish`).
    pub fn next_component(&mut self) -> Option<ComponentReader<'_, R>> {
        let which = self.next?;
        let (size, want) = match which {
            Component::Kernel => (self.header.app.kernel_size, self.header.app.kernel_crc),
            Component::Rootfs => (self.header.app.rootfs_size, self.header.app.rootfs_crc),
        };
        self.next = match which {
            Component::Kernel => Some(Component::Rootfs),
            Component::Rootfs => None,
        };
        Some(ComponentReader {
            inner: &mut self.inner,
            which,
            left: size as u64,
            want,
            crc: Crc::new(),
        })
    }

    /// After both components: check that the stream ends here.
    pub fn finish(mut self) -> Result<R> {
        if self.next.is_some() {
            return bad("OTA components were not all read");
        }
        let mut one = [0u8; 1];
        loop {
            match self.inner.read(&mut one) {
                Ok(0) => return Ok(self.inner),
                Ok(_) => return bad("trailing data after the OTA rootfs"),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }
    }
}

/// One component of a streamed OTA.  Read to the end, then call
/// [`ComponentReader::finish`] to check the CRC.
pub struct ComponentReader<'a, R: Read> {
    inner: &'a mut R,
    which: Component,
    left: u64,
    want: u32,
    crc: Crc,
}

impl<R: Read> ComponentReader<'_, R> {
    pub fn which(&self) -> Component {
        self.which
    }

    /// Bytes not yet read.
    pub fn remaining(&self) -> u64 {
        self.left
    }

    /// Drain anything unread and check the component CRC.
    pub fn finish(mut self) -> Result<()> {
        let mut buf = [0u8; 4096];
        while self.left > 0 {
            if self.read(&mut buf)? == 0 {
                break;
            }
        }
        if self.left > 0 {
            return bad(format!("truncated OTA {:?}", self.which).to_lowercase());
        }
        if self.crc.value() != self.want {
            return bad(format!("invalid {:?} CRC", self.which).to_lowercase());
        }
        Ok(())
    }
}

impl<R: Read> Read for ComponentReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.left == 0 || buf.is_empty() {
            return Ok(0);
        }
        let n = buf.len().min(self.left.min(usize::MAX as u64) as usize);
        let got = self.inner.read(&mut buf[..n])?;
        if got == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "truncated OTA component",
            ));
        }
        self.crc.update(&buf[..got]);
        self.left -= got as u64;
        Ok(got)
    }
}
