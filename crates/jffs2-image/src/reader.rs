//! Image scanner: rebuilds a [`Tree`] from the newest nodes.

use crate::compr;
use crate::crc::crc32;
use crate::node::*;
use crate::tree::{Entry, Kind, Tree};
use crate::{Result, bad};
use std::collections::{BTreeMap, HashMap};

/// What the scan saw; used to compare an image's structure with another's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanStats {
    pub image_len: usize,
    pub dirents: usize,
    pub inode_nodes: usize,
    pub cleanmarkers: usize,
    pub padding_nodes: usize,
    pub summary_nodes: usize,
    pub xattr_nodes: usize,
    pub other_nodes: usize,
    /// Nodes whose header CRC was valid but whose body CRC was not.
    pub bad_crc_nodes: usize,
    /// Inode nodes by compression type (index = type number).
    pub compr: [usize; 8],
    /// Valid nodes that extend past the end of their erase block.
    pub nodes_crossing_blocks: usize,
    /// Largest uncompressed size of a single data node.
    pub max_dsize: u32,
    /// Whether the root directory (inode 1) has an inode node.
    pub root_has_inode_node: bool,
    /// Bytes that are neither part of a valid node nor `0xff`.
    pub junk_bytes: usize,
    /// `0xff` bytes between and after nodes (alignment and block-tail fill).
    pub ff_bytes: usize,
    pub highest_ino: u32,
}

struct Frag {
    version: u32,
    mode: u32,
    uid: u16,
    gid: u16,
    isize: u32,
    mtime: u32,
    offset: u32,
    dsize: u32,
    compr: u8,
    data: (usize, usize),
}

struct Dirent {
    version: u32,
    ino: u32,
}

/// Scan `image` and rebuild its tree.  `erase_block` is only used for the
/// block-crossing statistic.
pub fn read_image(image: &[u8], erase_block: usize) -> Result<(Tree, ScanStats)> {
    let mut stats = ScanStats {
        image_len: image.len(),
        ..Default::default()
    };
    let mut dirents: HashMap<(u32, Vec<u8>), Dirent> = HashMap::new();
    let mut inodes: HashMap<u32, Vec<Frag>> = HashMap::new();

    let mut pos = 0;
    while pos + 4 <= image.len() {
        let Some((nodetype, totlen)) = parse_header(&image[pos..]) else {
            let word = &image[pos..pos + 4];
            if word == [0xff; 4] {
                stats.ff_bytes += 4;
            } else {
                stats.junk_bytes += 4;
            }
            pos += 4;
            continue;
        };
        if totlen < HDR_LEN || pos + totlen > image.len() {
            stats.junk_bytes += 4;
            pos += 4;
            continue;
        }
        let b = &image[pos..pos + totlen];
        if erase_block != 0 && pos / erase_block != (pos + totlen - 1) / erase_block {
            stats.nodes_crossing_blocks += 1;
        }
        match nodetype {
            NODETYPE_DIRENT if totlen >= DIRENT_LEN => {
                let nsize = b[28] as usize;
                if totlen < DIRENT_LEN + nsize
                    || crc32(&b[..DIRENT_LEN - 8]) != u32le(b, 32)
                    || crc32(&b[DIRENT_LEN..DIRENT_LEN + nsize]) != u32le(b, 36)
                {
                    stats.bad_crc_nodes += 1;
                } else {
                    stats.dirents += 1;
                    let (pino, version, ino) = (u32le(b, 12), u32le(b, 16), u32le(b, 20));
                    let name = b[DIRENT_LEN..DIRENT_LEN + nsize].to_vec();
                    let slot = dirents
                        .entry((pino, name))
                        .or_insert(Dirent { version, ino });
                    if version >= slot.version {
                        *slot = Dirent { version, ino };
                    }
                }
            }
            NODETYPE_INODE if totlen >= INODE_LEN => {
                let csize = u32le(b, 48) as usize;
                if totlen < INODE_LEN + csize
                    || crc32(&b[..INODE_LEN - 8]) != u32le(b, 64)
                    || crc32(&b[INODE_LEN..INODE_LEN + csize]) != u32le(b, 60)
                {
                    stats.bad_crc_nodes += 1;
                } else {
                    stats.inode_nodes += 1;
                    let ino = u32le(b, 12);
                    let f = Frag {
                        version: u32le(b, 16),
                        mode: u32le(b, 20),
                        uid: u16le(b, 24),
                        gid: u16le(b, 26),
                        isize: u32le(b, 28),
                        mtime: u32le(b, 36),
                        offset: u32le(b, 44),
                        dsize: u32le(b, 52),
                        compr: b[56],
                        data: (pos + INODE_LEN, csize),
                    };
                    stats.compr[(f.compr as usize).min(7)] += 1;
                    stats.max_dsize = stats.max_dsize.max(f.dsize);
                    stats.highest_ino = stats.highest_ino.max(ino);
                    if ino == 1 {
                        stats.root_has_inode_node = true;
                    }
                    inodes.entry(ino).or_default().push(f);
                }
            }
            NODETYPE_CLEANMARKER => stats.cleanmarkers += 1,
            NODETYPE_PADDING => stats.padding_nodes += 1,
            NODETYPE_SUMMARY => stats.summary_nodes += 1,
            NODETYPE_XATTR | NODETYPE_XREF => stats.xattr_nodes += 1,
            _ => stats.other_nodes += 1,
        }
        pos += pad4(totlen);
    }

    // Children by parent inode, deleted entries (ino 0) dropped.
    let mut children: BTreeMap<u32, Vec<(Vec<u8>, u32)>> = BTreeMap::new();
    for ((pino, name), d) in dirents {
        if d.ino != 0 {
            children.entry(pino).or_default().push((name, d.ino));
        }
    }
    for list in children.values_mut() {
        list.sort();
    }

    let mut tree = Tree::new();
    let mut stack: Vec<(u32, String)> = vec![(1, String::new())];
    let mut visited = std::collections::HashSet::new();
    while let Some((dir_ino, prefix)) = stack.pop() {
        if !visited.insert(dir_ino) {
            return bad("directory loop in image");
        }
        let Some(list) = children.get(&dir_ino) else {
            continue;
        };
        for (name, ino) in list {
            let Ok(name) = std::str::from_utf8(name) else {
                return bad("non-UTF-8 file name in image");
            };
            let path = if prefix.is_empty() {
                name.to_owned()
            } else {
                format!("{prefix}/{name}")
            };
            let Some(frags) = inodes.get_mut(ino) else {
                return bad(format!(
                    "{path}: directory entry points at inode {ino}, which has no nodes"
                ));
            };
            let entry = build_entry(image, frags, &path)?;
            if entry.kind == Kind::Dir {
                stack.push((*ino, path.clone()));
            }
            tree.entries.insert(path, entry);
        }
    }
    Ok((tree, stats))
}

fn build_entry(image: &[u8], frags: &mut [Frag], path: &str) -> Result<Entry> {
    frags.sort_by_key(|f| f.version);
    let last = frags.last().expect("inode has at least one node");
    let (mode, uid, gid, mtime, isize) = (
        last.mode,
        last.uid,
        last.gid,
        last.mtime,
        last.isize as usize,
    );
    let mut data: Vec<u8> = Vec::new();
    for f in frags.iter() {
        // A node's isize is the file size as of that version: shrink on truncation.
        if (f.isize as usize) < data.len() {
            data.truncate(f.isize as usize);
        }
        if f.dsize == 0 {
            continue;
        }
        let payload = &image[f.data.0..f.data.0 + f.data.1];
        let plain = compr::decompress(f.compr, payload, f.dsize as usize)
            .map_err(|e| crate::Error::Format(format!("{path}: {e}")))?;
        let end = f.offset as usize + plain.len();
        if data.len() < end {
            data.resize(end, 0);
        }
        data[f.offset as usize..end].copy_from_slice(&plain);
    }
    let kind = match mode & S_IFMT {
        S_IFDIR => Kind::Dir,
        S_IFREG => {
            data.resize(isize, 0);
            Kind::File(data)
        }
        S_IFLNK => Kind::Symlink(data),
        S_IFCHR | S_IFBLK => {
            let (major, minor) = match data.len() {
                2 => {
                    let v = u16le(&data, 0) as u32;
                    (v >> 8, v & 0xff)
                }
                4 => {
                    let v = u32le(&data, 0);
                    ((v & 0xfff00) >> 8, (v & 0xff) | ((v >> 12) & 0xfff00))
                }
                n => return bad(format!("{path}: device node with {n} data bytes")),
            };
            if mode & S_IFMT == S_IFCHR {
                Kind::CharDev { major, minor }
            } else {
                Kind::BlockDev { major, minor }
            }
        }
        S_IFIFO => Kind::Fifo,
        S_IFSOCK => Kind::Socket,
        other => return bad(format!("{path}: unknown file type {other:o}")),
    };
    Ok(Entry {
        kind,
        mode: mode & 0o7777,
        uid,
        gid,
        mtime,
    })
}
