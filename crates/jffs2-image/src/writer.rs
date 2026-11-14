//! Image writer.

use crate::compr::{self, COMPR_NONE, COMPR_RTIME, COMPR_ZLIB};
use crate::node::*;
use crate::tree::{Entry, Kind, Tree};
use crate::{Result, bad};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    /// Store everything uncompressed.
    None,
    /// zlib where it shrinks the data, otherwise rtime, otherwise stored
    /// (the order `mkfs.jffs2` uses, and what the stock images contain).
    ZlibThenRtime,
    /// rtime only (for kernels built without `JFFS2_ZLIB`).
    Rtime,
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Erase-block size of the target flash; nodes never cross a multiple
    /// of it.  All seven phone models: `0x20000`.
    pub erase_block: usize,
    /// Largest uncompressed data per node; must not exceed the kernel's
    /// page size (4096).
    pub page_size: usize,
    pub compression: Compression,
    /// Fill the last erase block with `0xff`.  The stock images do not.
    pub pad_last_block: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            erase_block: 0x20000,
            page_size: 4096,
            compression: Compression::ZlibThenRtime,
            pad_last_block: false,
        }
    }
}

struct Out {
    buf: Vec<u8>,
    erase_block: usize,
}

impl Out {
    fn room(&self) -> usize {
        self.erase_block - self.buf.len() % self.erase_block
    }

    fn pad_block(&mut self) {
        if !self.buf.len().is_multiple_of(self.erase_block) {
            let n = self.room();
            self.buf.resize(self.buf.len() + n, 0xff);
        }
    }

    /// Make sure a node of `len` bytes fits in the current block.
    fn reserve(&mut self, len: usize) {
        if pad4(len) > self.room() {
            self.pad_block();
        }
    }

    fn align(&mut self) {
        let n = pad4(self.buf.len());
        self.buf.resize(n, 0xff);
    }
}

fn compress(mode: Compression, data: &[u8]) -> (u8, Vec<u8>) {
    match mode {
        Compression::None => {}
        Compression::ZlibThenRtime => {
            if let Some(z) = compr::zlib_compress(data) {
                return (COMPR_ZLIB, z);
            }
            if let Some(r) = compr::rtime_compress(data) {
                return (COMPR_RTIME, r);
            }
        }
        Compression::Rtime => {
            if let Some(r) = compr::rtime_compress(data) {
                return (COMPR_RTIME, r);
            }
        }
    }
    (COMPR_NONE, data.to_vec())
}

fn rdev_bytes(major: u32, minor: u32) -> Vec<u8> {
    if major < 256 && minor < 256 {
        (((major << 8) | minor) as u16).to_le_bytes().to_vec()
    } else {
        ((minor & 0xff) | (major << 8) | ((minor & !0xff) << 12))
            .to_le_bytes()
            .to_vec()
    }
}

/// Smallest remaining room in which a new data node is started rather than
/// moving to the next block.
const MIN_DATA_ROOM: usize = INODE_LEN + 128;

fn write_file(out: &mut Out, opts: &Options, base: InodeFields, data: &[u8]) {
    let mut f = base;
    f.isize = data.len() as u32;
    if data.is_empty() {
        out.reserve(INODE_LEN);
        write_inode(&mut out.buf, &f, &[]);
        out.align();
        return;
    }
    let mut offset = 0;
    while offset < data.len() {
        // Chunks end at page boundaries of the file.
        let page_end = ((offset / opts.page_size + 1) * opts.page_size).min(data.len());
        let chunk = &data[offset..page_end];
        if out.room() < MIN_DATA_ROOM {
            out.pad_block();
        }
        let room = out.room();
        let (mut compr, mut payload) = compress(opts.compression, chunk);
        let mut dsize = chunk.len();
        if pad4(INODE_LEN + payload.len()) > room {
            // Does not fit: store the longest prefix that does, as
            // mkfs.jffs2 does, and continue the page in the next block.
            let budget = (room - INODE_LEN) & !3;
            let (mut lo, mut hi) = (budget.min(chunk.len()), chunk.len());
            // `lo` bytes always fit uncompressed; look for a longer prefix
            // whose compressed form fits.
            let mut best = (COMPR_NONE, chunk[..lo].to_vec(), lo);
            if opts.compression != Compression::None {
                while lo + 1 < hi {
                    let mid = (lo + hi) / 2;
                    let (c, p) = compress(opts.compression, &chunk[..mid]);
                    if p.len() <= budget {
                        lo = mid;
                        best = (c, p, mid);
                    } else {
                        hi = mid;
                    }
                }
            }
            (compr, payload, dsize) = best;
        }
        f.version += 1;
        f.offset = offset as u32;
        f.dsize = dsize as u32;
        f.compr = compr;
        write_inode(&mut out.buf, &f, &payload);
        out.align();
        offset += dsize;
    }
}

/// Build a JFFS2 image of `tree`.
pub fn write_image(tree: &Tree, opts: &Options) -> Result<Vec<u8>> {
    if opts.erase_block < 0x1000 || !opts.erase_block.is_multiple_of(4) {
        return bad("erase block size must be a multiple of 4 and at least 4096");
    }
    if opts.page_size == 0 || opts.page_size > 4096 {
        return bad("page size must be between 1 and 4096");
    }
    // Children by parent path, in name order; every parent must be a directory.
    let mut children: BTreeMap<&str, Vec<(&str, &Entry)>> = BTreeMap::new();
    for (path, entry) in &tree.entries {
        let (parent, name) = match path.rfind('/') {
            Some(i) => (&path[..i], &path[i + 1..]),
            None => ("", path.as_str()),
        };
        if name.is_empty() || name.len() > 254 {
            return bad(format!("bad name in path {path:?}"));
        }
        if !parent.is_empty() {
            match tree.entries.get(parent) {
                Some(e) if e.kind == Kind::Dir => {}
                _ => return bad(format!("parent of {path:?} is missing or not a directory")),
            }
        }
        children.entry(parent).or_default().push((name, entry));
    }

    let mut out = Out {
        buf: Vec::new(),
        erase_block: opts.erase_block,
    };
    let mut next_ino = 2u32;
    // (directory path, its inode number), breadth-first like mkfs.jffs2.
    let mut queue: std::collections::VecDeque<(String, u32)> = [(String::new(), 1)].into();
    while let Some((dir, dir_ino)) = queue.pop_front() {
        let Some(list) = children.get(dir.as_str()) else {
            continue;
        };
        for (version, (name, entry)) in list.iter().enumerate() {
            let ino = next_ino;
            next_ino += 1;
            out.reserve(DIRENT_LEN + name.len());
            write_dirent(
                &mut out.buf,
                dir_ino,
                version as u32,
                ino,
                entry.mtime,
                entry.dtype(),
                name.as_bytes(),
            );
            out.align();
            let base = InodeFields {
                ino,
                version: 1,
                mode: entry.file_type_bits() | (entry.mode & 0o7777),
                uid: entry.uid,
                gid: entry.gid,
                isize: 0,
                mtime: entry.mtime,
                offset: 0,
                dsize: 0,
                compr: COMPR_NONE,
            };
            let small = |out: &mut Out, payload: &[u8], sized: bool| {
                let mut f = base;
                f.dsize = payload.len() as u32;
                if sized {
                    f.isize = payload.len() as u32;
                }
                out.reserve(INODE_LEN + payload.len());
                write_inode(&mut out.buf, &f, payload);
                out.align();
            };
            match &entry.kind {
                Kind::Dir => {
                    small(&mut out, &[], false);
                    let path = if dir.is_empty() {
                        (*name).to_owned()
                    } else {
                        format!("{dir}/{name}")
                    };
                    queue.push_back((path, ino));
                }
                Kind::File(data) => {
                    let mut f = base;
                    f.version = 0; // write_file pre-increments
                    if data.is_empty() {
                        f.version = 1;
                    }
                    write_file(&mut out, opts, f, data);
                }
                Kind::Symlink(target) => {
                    if target.is_empty() || target.len() > 4095 {
                        return bad(format!("{dir}/{name}: bad symlink target length"));
                    }
                    small(&mut out, target, true);
                }
                Kind::CharDev { major, minor } | Kind::BlockDev { major, minor } => {
                    small(&mut out, &rdev_bytes(*major, *minor), false);
                }
                Kind::Fifo | Kind::Socket => small(&mut out, &[], false),
            }
        }
    }
    if opts.pad_last_block {
        out.pad_block();
    }
    Ok(out.buf)
}
