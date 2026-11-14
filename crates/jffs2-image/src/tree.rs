//! In-memory description of a filesystem tree.

use crate::node::*;
use crate::{Result, bad};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Dir,
    File(Vec<u8>),
    Symlink(Vec<u8>),
    CharDev { major: u32, minor: u32 },
    BlockDev { major: u32, minor: u32 },
    Fifo,
    Socket,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub kind: Kind,
    /// Permission bits only (`0o7777`).
    pub mode: u32,
    pub uid: u16,
    pub gid: u16,
    pub mtime: u32,
}

impl Entry {
    pub fn file_type_bits(&self) -> u32 {
        match self.kind {
            Kind::Dir => S_IFDIR,
            Kind::File(_) => S_IFREG,
            Kind::Symlink(_) => S_IFLNK,
            Kind::CharDev { .. } => S_IFCHR,
            Kind::BlockDev { .. } => S_IFBLK,
            Kind::Fifo => S_IFIFO,
            Kind::Socket => S_IFSOCK,
        }
    }

    pub(crate) fn dtype(&self) -> u8 {
        match self.kind {
            Kind::Dir => DT_DIR,
            Kind::File(_) => DT_REG,
            Kind::Symlink(_) => DT_LNK,
            Kind::CharDev { .. } => DT_CHR,
            Kind::BlockDev { .. } => DT_BLK,
            Kind::Fifo => DT_FIFO,
            Kind::Socket => DT_SOCK,
        }
    }
}

/// A filesystem tree keyed by absolute path without the leading slash
/// (`"etc/passwd"`).  The root directory is implicit.  Paths are bytes
/// joined with `/`; ordering is bytewise, so images are reproducible.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tree {
    pub entries: BTreeMap<String, Entry>,
}

fn normalize(path: &str) -> Result<String> {
    let mut parts = Vec::new();
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => return bad(format!("path {path:?} contains '..'")),
            p if p.len() > 254 => return bad(format!("name too long in {path:?}")),
            p => parts.push(p),
        }
    }
    if parts.is_empty() {
        return bad("empty path");
    }
    Ok(parts.join("/"))
}

impl Tree {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert an entry; missing parent directories are created (mode 0755,
    /// root-owned, same mtime).
    pub fn insert(&mut self, path: &str, entry: Entry) -> Result<()> {
        let path = normalize(path)?;
        let mut at = 0;
        while let Some(i) = path[at..].find('/') {
            let parent = &path[..at + i];
            match self.entries.get(parent) {
                Some(e) if e.kind == Kind::Dir => {}
                Some(_) => return bad(format!("{parent:?} is not a directory")),
                None => {
                    self.entries.insert(
                        parent.to_owned(),
                        Entry {
                            kind: Kind::Dir,
                            mode: 0o755,
                            uid: 0,
                            gid: 0,
                            mtime: entry.mtime,
                        },
                    );
                }
            }
            at += i + 1;
        }
        self.entries.insert(path, entry);
        Ok(())
    }

    pub fn dir(&mut self, path: &str, mode: u32, mtime: u32) -> Result<()> {
        self.insert(
            path,
            Entry {
                kind: Kind::Dir,
                mode,
                uid: 0,
                gid: 0,
                mtime,
            },
        )
    }

    pub fn file(
        &mut self,
        path: &str,
        data: impl Into<Vec<u8>>,
        mode: u32,
        mtime: u32,
    ) -> Result<()> {
        self.insert(
            path,
            Entry {
                kind: Kind::File(data.into()),
                mode,
                uid: 0,
                gid: 0,
                mtime,
            },
        )
    }

    pub fn symlink(&mut self, path: &str, target: &str, mtime: u32) -> Result<()> {
        self.insert(
            path,
            Entry {
                kind: Kind::Symlink(target.as_bytes().to_vec()),
                mode: 0o777,
                uid: 0,
                gid: 0,
                mtime,
            },
        )
    }

    pub fn char_dev(
        &mut self,
        path: &str,
        major: u32,
        minor: u32,
        mode: u32,
        mtime: u32,
    ) -> Result<()> {
        self.insert(
            path,
            Entry {
                kind: Kind::CharDev { major, minor },
                mode,
                uid: 0,
                gid: 0,
                mtime,
            },
        )
    }

    pub fn get(&self, path: &str) -> Option<&Entry> {
        self.entries.get(path.trim_matches('/'))
    }

    /// Read a directory from the host.  Ownership is recorded as root
    /// unless `keep_owner`; every mtime becomes `mtime` when given.  Device
    /// nodes, FIFOs, and sockets found on the host are taken as they are.
    pub fn from_dir(root: &Path, keep_owner: bool, mtime: Option<u32>) -> Result<Self> {
        let mut tree = Tree::new();
        walk(root, "", keep_owner, mtime, &mut tree)?;
        Ok(tree)
    }

    /// Write the tree under `root` on the host.  Device nodes, FIFOs, and
    /// sockets are skipped and returned, since creating them needs root;
    /// ownership is not applied.
    pub fn extract(&self, root: &Path) -> Result<Vec<String>> {
        use std::os::unix::fs::PermissionsExt;
        let mut skipped = Vec::new();
        std::fs::create_dir_all(root)?;
        let mut dir_modes = Vec::new();
        for (path, e) in &self.entries {
            let dest = root.join(path);
            match &e.kind {
                Kind::Dir => {
                    std::fs::create_dir_all(&dest)?;
                    dir_modes.push((dest, e.mode));
                }
                Kind::File(data) => {
                    std::fs::write(&dest, data)?;
                    std::fs::set_permissions(
                        &dest,
                        std::fs::Permissions::from_mode(e.mode & 0o7777),
                    )?;
                }
                Kind::Symlink(target) => {
                    use std::os::unix::ffi::OsStrExt;
                    std::os::unix::fs::symlink(std::ffi::OsStr::from_bytes(target), &dest)?;
                }
                _ => skipped.push(path.clone()),
            }
        }
        // Directory modes last, deepest first, so read-only directories do not block writes.
        for (dest, mode) in dir_modes.into_iter().rev() {
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(mode & 0o7777))?;
        }
        Ok(skipped)
    }
}

fn walk(
    dir: &Path,
    prefix: &str,
    keep_owner: bool,
    mtime: Option<u32>,
    tree: &mut Tree,
) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let mut names: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    names.sort_by_key(|e| e.file_name());
    for ent in names {
        let name = ent.file_name();
        let Some(name) = name.to_str() else {
            return bad(format!("non-UTF-8 file name under {}", dir.display()));
        };
        let path = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        };
        let full = ent.path();
        let md = std::fs::symlink_metadata(&full)?;
        let ft = md.file_type();
        let kind = if ft.is_dir() {
            Kind::Dir
        } else if ft.is_file() {
            Kind::File(std::fs::read(&full)?)
        } else if ft.is_symlink() {
            Kind::Symlink(std::fs::read_link(&full)?.as_os_str().as_bytes().to_vec())
        } else if ft.is_char_device() || ft.is_block_device() {
            let rdev = md.rdev();
            let major = (((rdev >> 32) & 0xffff_f000) | ((rdev >> 8) & 0xfff)) as u32;
            let minor = (((rdev >> 12) & 0xffff_ff00) | (rdev & 0xff)) as u32;
            if ft.is_char_device() {
                Kind::CharDev { major, minor }
            } else {
                Kind::BlockDev { major, minor }
            }
        } else if ft.is_fifo() {
            Kind::Fifo
        } else {
            Kind::Socket
        };
        let is_dir = kind == Kind::Dir;
        let (uid, gid) = if keep_owner {
            (md.uid() as u16, md.gid() as u16)
        } else {
            (0, 0)
        };
        tree.entries.insert(
            path.clone(),
            Entry {
                kind,
                mode: md.mode() & 0o7777,
                uid,
                gid,
                mtime: mtime.unwrap_or(md.mtime() as u32),
            },
        );
        if is_dir {
            walk(&full, &path, keep_owner, mtime, tree)?;
        }
    }
    Ok(())
}
