//! On-flash node layouts (little-endian).

use crate::crc::crc32;

pub const MAGIC: u16 = 0x1985;
pub const NODETYPE_DIRENT: u16 = 0xe001;
pub const NODETYPE_INODE: u16 = 0xe002;
pub const NODETYPE_CLEANMARKER: u16 = 0x2003;
pub const NODETYPE_PADDING: u16 = 0x2004;
pub const NODETYPE_SUMMARY: u16 = 0x2006;
pub const NODETYPE_XATTR: u16 = 0xe008;
pub const NODETYPE_XREF: u16 = 0xe009;

pub const HDR_LEN: usize = 12;
pub const DIRENT_LEN: usize = 40;
pub const INODE_LEN: usize = 68;

pub const DT_FIFO: u8 = 1;
pub const DT_CHR: u8 = 2;
pub const DT_DIR: u8 = 4;
pub const DT_BLK: u8 = 6;
pub const DT_REG: u8 = 8;
pub const DT_LNK: u8 = 10;
pub const DT_SOCK: u8 = 12;

pub const S_IFMT: u32 = 0o170000;
pub const S_IFSOCK: u32 = 0o140000;
pub const S_IFLNK: u32 = 0o120000;
pub const S_IFREG: u32 = 0o100000;
pub const S_IFBLK: u32 = 0o060000;
pub const S_IFDIR: u32 = 0o040000;
pub const S_IFCHR: u32 = 0o020000;
pub const S_IFIFO: u32 = 0o010000;

pub(crate) fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

pub(crate) fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

pub(crate) fn pad4(n: usize) -> usize {
    (n + 3) & !3
}

fn header(out: &mut Vec<u8>, nodetype: u16, totlen: u32) {
    out.extend_from_slice(&MAGIC.to_le_bytes());
    out.extend_from_slice(&nodetype.to_le_bytes());
    out.extend_from_slice(&totlen.to_le_bytes());
    let c = crc32(&out[out.len() - 8..]);
    out.extend_from_slice(&c.to_le_bytes());
}

/// Append a directory-entry node (not padded).
pub fn write_dirent(
    out: &mut Vec<u8>,
    pino: u32,
    version: u32,
    ino: u32,
    mctime: u32,
    dtype: u8,
    name: &[u8],
) {
    let start = out.len();
    header(out, NODETYPE_DIRENT, (DIRENT_LEN + name.len()) as u32);
    out.extend_from_slice(&pino.to_le_bytes());
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&ino.to_le_bytes());
    out.extend_from_slice(&mctime.to_le_bytes());
    out.push(name.len() as u8);
    out.push(dtype);
    out.extend_from_slice(&[0, 0]);
    let c = crc32(&out[start..start + DIRENT_LEN - 8]);
    out.extend_from_slice(&c.to_le_bytes());
    out.extend_from_slice(&crc32(name).to_le_bytes());
    out.extend_from_slice(name);
}

/// Fields of an inode (data) node.
#[derive(Clone, Copy, Debug)]
pub struct InodeFields {
    pub ino: u32,
    pub version: u32,
    pub mode: u32,
    pub uid: u16,
    pub gid: u16,
    pub isize: u32,
    pub mtime: u32,
    pub offset: u32,
    pub dsize: u32,
    pub compr: u8,
}

/// Append an inode node with its (already compressed) payload (not padded).
pub fn write_inode(out: &mut Vec<u8>, f: &InodeFields, payload: &[u8]) {
    let start = out.len();
    header(out, NODETYPE_INODE, (INODE_LEN + payload.len()) as u32);
    out.extend_from_slice(&f.ino.to_le_bytes());
    out.extend_from_slice(&f.version.to_le_bytes());
    out.extend_from_slice(&f.mode.to_le_bytes());
    out.extend_from_slice(&f.uid.to_le_bytes());
    out.extend_from_slice(&f.gid.to_le_bytes());
    out.extend_from_slice(&f.isize.to_le_bytes());
    for _ in 0..3 {
        out.extend_from_slice(&f.mtime.to_le_bytes()); // atime, mtime, ctime
    }
    out.extend_from_slice(&f.offset.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&f.dsize.to_le_bytes());
    out.push(f.compr);
    out.push(0); // usercompr
    out.extend_from_slice(&[0, 0]); // flags
    out.extend_from_slice(&crc32(payload).to_le_bytes());
    let c = crc32(&out[start..start + INODE_LEN - 8]);
    out.extend_from_slice(&c.to_le_bytes());
    out.extend_from_slice(payload);
}

/// Check a node header at the start of `b`; returns (nodetype, totlen).
pub fn parse_header(b: &[u8]) -> Option<(u16, usize)> {
    if b.len() < HDR_LEN || u16le(b, 0) != MAGIC {
        return None;
    }
    if crc32(&b[..8]) != u32le(b, 8) {
        return None;
    }
    Some((u16le(b, 2), u32le(b, 4) as usize))
}
