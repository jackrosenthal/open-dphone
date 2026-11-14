//! Configuration backup and restore.
//!
//! A backup is a gzip-compressed tar archive of the files that hold local
//! configuration.  Restore accepts only those same paths, so an archive
//! cannot write anywhere else.

use crate::assets::{gunzip, gzip};
use dphone_config::Nvdata;
use std::io;

/// Makes and restores backups.  The default is [`NvdataBackup`].
pub trait Backup: Send + Sync {
    /// The archive bytes.
    fn export(&self) -> io::Result<Vec<u8>>;
    /// Restores an archive; returns the number of files written.
    fn import(&self, archive: &[u8]) -> io::Result<u32>;
}

/// Files a backup holds, relative to the nvdata root.  Provisioning caches
/// (`fetched-config.xml`, `configurator.xml`, `md5s/`), identity keys, and
/// stock-owned files are left out: a restore must not move an identity to
/// another phone or fight the provisioning server.
const FILES: &[&str] = &["ui-config.xml", "open/config.xml", "blf_items.xml", "netselect.xml"];
/// Directories whose plain files are included.
const DIRS: &[&str] = &["contact_lists"];
const MAX_FILE: usize = 4 << 20;

fn allowed(name: &str) -> bool {
    if FILES.contains(&name) {
        return true;
    }
    DIRS.iter().any(|d| {
        name.strip_prefix(d)
            .and_then(|r| r.strip_prefix('/'))
            .is_some_and(|file| !file.is_empty() && !file.contains('/') && !file.starts_with('.') && !file.ends_with(".tmp"))
    })
}

/// Backs up and restores the local configuration under an nvdata root.
pub struct NvdataBackup {
    nv: Nvdata,
}

impl NvdataBackup {
    pub fn new(nv: Nvdata) -> Self {
        NvdataBackup { nv }
    }
}

impl Backup for NvdataBackup {
    fn export(&self) -> io::Result<Vec<u8>> {
        let mut tar = Vec::new();
        let mut add = |name: &str| -> io::Result<()> {
            if let Some(data) = self.nv.read(name)? {
                tar_entry(&mut tar, name, &data);
            }
            Ok(())
        };
        for f in FILES {
            add(f)?;
        }
        for d in DIRS {
            for file in self.nv.list(d).unwrap_or_default() {
                let name = format!("{d}/{file}");
                if allowed(&name) {
                    add(&name)?;
                }
            }
        }
        tar.extend_from_slice(&[0u8; 1024]);
        Ok(gzip(&tar))
    }

    fn import(&self, archive: &[u8]) -> io::Result<u32> {
        let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_owned());
        let tar = gunzip(archive).ok_or_else(|| bad("not a gzip archive"))?;
        let entries = tar_entries(&tar).ok_or_else(|| bad("damaged tar archive"))?;
        if entries.is_empty() {
            return Err(bad("the archive holds no files"));
        }
        // Check everything before writing anything.
        for (name, data) in &entries {
            if !allowed(name) {
                return Err(bad(&format!("unexpected file in backup: {name}")));
            }
            if data.len() > MAX_FILE {
                return Err(bad(&format!("file too large: {name}")));
            }
            if name.ends_with(".xml") {
                let text = std::str::from_utf8(data).map_err(|_| bad(&format!("{name} is not text")))?;
                dphone_config::Element::parse(text).map_err(|e| bad(&format!("{name}: {e}")))?;
            }
        }
        for (name, data) in &entries {
            self.nv.write(name, data)?;
        }
        Ok(entries.len() as u32)
    }
}

fn octal(field: &mut [u8], value: u64) {
    let s = format!("{:0width$o}", value, width = field.len() - 1);
    field[..s.len()].copy_from_slice(s.as_bytes());
}

fn tar_entry(out: &mut Vec<u8>, name: &str, data: &[u8]) {
    let mut h = [0u8; 512];
    let n = name.as_bytes();
    h[..n.len().min(100)].copy_from_slice(&n[..n.len().min(100)]);
    octal(&mut h[100..108], 0o600);
    octal(&mut h[108..116], 0);
    octal(&mut h[116..124], 0);
    octal(&mut h[124..136], data.len() as u64);
    octal(&mut h[136..148], dphone_state::clock::unix_secs());
    h[148..156].fill(b' ');
    h[156] = b'0';
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    let sum: u32 = h.iter().map(|b| *b as u32).sum();
    let s = format!("{sum:06o}\0 ");
    h[148..156].copy_from_slice(s.as_bytes());
    out.extend_from_slice(&h);
    out.extend_from_slice(data);
    out.resize(out.len().div_ceil(512) * 512, 0);
}

fn tar_entries(tar: &[u8]) -> Option<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + 512 <= tar.len() {
        let h = &tar[pos..pos + 512];
        if h.iter().all(|b| *b == 0) {
            break;
        }
        let field = |r: std::ops::Range<usize>| {
            let f = &h[r];
            let end = f.iter().position(|b| *b == 0).unwrap_or(f.len());
            std::str::from_utf8(&f[..end]).ok().map(|s| s.trim().to_owned())
        };
        let name = field(0..100)?;
        let size = usize::from_str_radix(&field(124..136)?, 8).ok()?;
        let kind = h[156];
        pos += 512;
        let data = tar.get(pos..pos.checked_add(size)?)?;
        if kind == b'0' || kind == 0 {
            out.push((name.trim_start_matches("./").to_owned(), data.to_vec()));
        }
        pos += size.div_ceil(512) * 512;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> (std::path::PathBuf, Nvdata) {
        let dir = std::env::temp_dir().join(format!("dphone-web-backup-{}", crate::auth::random_token()));
        std::fs::create_dir_all(&dir).unwrap();
        let nv = Nvdata::new(&dir);
        (dir, nv)
    }

    #[test]
    fn round_trip() {
        let (dir, nv) = temp();
        nv.write("ui-config.xml", b"<config><setting id=\"a\" value=\"1\"/></config>").unwrap();
        nv.write("contact_lists/local.xml", b"<contacts/>").unwrap();
        nv.write("fetched-config.xml", b"<config/>").unwrap();
        let archive = NvdataBackup::new(nv.clone()).export().unwrap();

        let (dir2, nv2) = temp();
        assert_eq!(NvdataBackup::new(nv2.clone()).import(&archive).unwrap(), 2);
        assert_eq!(nv2.read("ui-config.xml").unwrap(), nv.read("ui-config.xml").unwrap());
        assert_eq!(nv2.read("contact_lists/local.xml").unwrap().unwrap(), b"<contacts/>");
        assert!(nv2.read("fetched-config.xml").unwrap().is_none());
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(dir2);
    }

    #[test]
    fn refuses_other_paths() {
        let (dir, nv) = temp();
        for name in ["etc/enet.cfg", "../x", "contact_lists/../ui-config.xml", "contact_lists/.hidden"] {
            let mut tar = Vec::new();
            tar_entry(&mut tar, name, b"x");
            tar.extend_from_slice(&[0u8; 1024]);
            assert!(NvdataBackup::new(nv.clone()).import(&gzip(&tar)).is_err(), "{name}");
        }
        let mut tar = Vec::new();
        tar_entry(&mut tar, "ui-config.xml", b"<not closed");
        tar.extend_from_slice(&[0u8; 1024]);
        assert!(NvdataBackup::new(nv.clone()).import(&gzip(&tar)).is_err());
        assert!(NvdataBackup::new(nv).import(b"junk").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
