//! The location `local` of the app interface `storage`: an app's keys on
//! the phone, a file for each key in the app's directory under
//! `/nvdata/appdata`.  The file's modification time is the entry's
//! `modified`.  A key the app wrote has a file that begins with [`MAGIC`]
//! and the entry's version (8 bytes, little-endian), then the value.  A
//! file without them (one an administrator or a test put there, e.g., a
//! game's seed) is all value, and its version is its modification time.
//!
//! A new version is greater than every version a key of the directory
//! has had, so that a key set again after a removal never has a version
//! it had before: the directory's file `.seq` (not a key, as no key
//! starts with `.`) keeps the version of each key that was removed.

use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use crate::services::{StoreEntry, StoreReply};

/// The largest value.
pub(crate) const VALUE_MAX: usize = 1 << 20;
/// The longest key, in bytes: as long as a key on the PBX may be.
pub(crate) const KEY_MAX: usize = 64;
/// The most entries of one `list` answer, and the most bytes of values
/// past its first entry (as on the PBX).
const LIST_MAX: usize = 64;
const LIST_BYTES_MAX: usize = 16 * 1024;
/// What a file the store wrote begins with: no text does.
const MAGIC: [u8; 4] = *b"\0DPK";
/// The bytes of such a file before the value: [`MAGIC`] and the version.
const HEADER: usize = 12;
const SEQ: &str = ".seq";

/// Why `key` cannot be a key, if it cannot.
pub(crate) fn key_error(key: &str) -> Option<String> {
    if key.len() > KEY_MAX {
        return Some(format!("a key is at most {KEY_MAX} bytes"));
    }
    if key.is_empty() || key.starts_with('.') || key.contains(['/', '\\', '\0']) {
        return Some("a key cannot be empty, start with `.`, or contain `/`, `\\`, or NUL".into());
    }
    None
}

/// Whether a write conditional on `expect` may replace `current`.
fn holds(expect: Option<u64>, current: Option<&StoreEntry>) -> bool {
    match expect {
        None => true,
        Some(0) => current.is_none(),
        Some(v) => current.is_some_and(|e| e.version == v),
    }
}

fn modified(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

/// The version and the value in a key's file.
fn split<'a>(bytes: &'a [u8], meta: &std::fs::Metadata) -> (u64, &'a [u8]) {
    match bytes.split_first_chunk::<HEADER>() {
        Some((head, value)) if head[..4] == MAGIC => {
            (u64::from_le_bytes(head[4..].try_into().unwrap()).max(1), value)
        }
        _ => (modified(meta).max(1), bytes),
    }
}

/// Writes `parts` to `path` at once: a power loss leaves the file as it
/// was or as written.
fn write_atomic(dir: &Path, path: &Path, parts: &[&[u8]]) -> std::io::Result<()> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(".{name}.tmp"));
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        for part in parts {
            f.write_all(part)?;
        }
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// One app's keys on the phone.
pub(crate) struct LocalStore {
    dir: PathBuf,
    /// The highest version a key of the directory has had, once read.
    seq: Option<u64>,
}

impl LocalStore {
    pub fn new(dir: PathBuf) -> LocalStore {
        LocalStore { dir, seq: None }
    }

    /// The entry of a valid key; `Err` with the reason when the file
    /// cannot be read.  A file that is not a key's (a directory, or a file
    /// too large) is no entry.
    fn read(&self, key: &str) -> Result<Option<StoreEntry>, String> {
        let path = self.dir.join(key);
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        if meta.file_type().is_symlink() {
            return Err("the key's file is a symbolic link, which the phone does not follow".into());
        }
        if !meta.is_file() || meta.len() > (HEADER + VALUE_MAX) as u64 {
            return Ok(None);
        }
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        let (version, value) = split(&bytes, &meta);
        Ok(Some(StoreEntry {
            key: key.to_string(),
            value: value.to_vec(),
            version,
            modified: modified(&meta),
            author_id: String::new(),
            author_name: String::new(),
        }))
    }

    /// The keys that have files, sorted.
    fn keys(&self) -> std::io::Result<Vec<String>> {
        let mut keys = Vec::new();
        let dir = match std::fs::read_dir(&self.dir) {
            Ok(dir) => dir,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(keys),
            Err(e) => return Err(e),
        };
        for e in dir {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            if e.file_type().is_ok_and(|t| t.is_file()) && key_error(&name).is_none() {
                keys.push(name);
            }
        }
        keys.sort();
        Ok(keys)
    }

    /// The highest version a key of the directory has had.
    fn seq(&mut self) -> u64 {
        if let Some(seq) = self.seq {
            return seq;
        }
        let mut seq = std::fs::read_to_string(self.dir.join(SEQ))
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0);
        for key in self.keys().unwrap_or_default() {
            let mut head = [0; HEADER];
            let read = std::fs::File::open(self.dir.join(&key)).and_then(|mut f| {
                let meta = f.metadata()?;
                let n = f.read(&mut head)?;
                Ok(split(&head[..n], &meta).0)
            });
            seq = seq.max(read.unwrap_or(0));
        }
        self.seq = Some(seq);
        seq
    }

    pub fn get(&mut self, key: &str) -> StoreReply {
        if let Some(why) = key_error(key) {
            return StoreReply::Refused(why);
        }
        match self.read(key) {
            Ok(entry) => StoreReply::Entry(entry),
            Err(why) => StoreReply::Unreachable(why),
        }
    }

    pub fn list(&mut self, prefix: &str, after: &str) -> StoreReply {
        let keys = match self.keys() {
            Ok(keys) => keys,
            Err(e) => return StoreReply::Unreachable(e.to_string()),
        };
        let mut out: Vec<StoreEntry> = Vec::new();
        let mut bytes = 0;
        for key in keys.iter().filter(|k| k.starts_with(prefix) && k.as_str() > after) {
            let entry = match self.read(key) {
                Ok(Some(entry)) => entry,
                Ok(None) => continue,
                Err(why) => return StoreReply::Unreachable(why),
            };
            if out.len() >= LIST_MAX || (!out.is_empty() && bytes + entry.value.len() > LIST_BYTES_MAX) {
                return StoreReply::Entries(out, true);
            }
            bytes += entry.value.len();
            out.push(entry);
        }
        StoreReply::Entries(out, false)
    }

    pub fn set(&mut self, key: &str, value: &[u8], expect: Option<u64>) -> StoreReply {
        if let Some(why) = key_error(key) {
            return StoreReply::Refused(why);
        }
        if value.len() > VALUE_MAX {
            return StoreReply::Refused("a value is at most 1 MiB".into());
        }
        let old = match self.read(key) {
            Ok(old) => old,
            Err(why) => return StoreReply::Unreachable(why),
        };
        if !holds(expect, old.as_ref()) {
            return StoreReply::Conflict(old);
        }
        let full = || StoreReply::Refused("the phone's flash is full".into());
        if let Err(e) = std::fs::create_dir_all(&self.dir) {
            return match e.kind() {
                ErrorKind::StorageFull => full(),
                _ => StoreReply::Unreachable(e.to_string()),
            };
        }
        // Replacing a value takes only what the new one needs more.
        let had = old.as_ref().map_or(0, |e| HEADER + e.value.len());
        if (HEADER + value.len()).saturating_sub(had) as u64 > crate::host::usable_space(&self.dir) {
            return full();
        }
        let version = self.seq().max(old.map_or(0, |e| e.version)) + 1;
        let path = self.dir.join(key);
        if let Err(e) = write_atomic(&self.dir, &path, &[&MAGIC, &version.to_le_bytes(), value]) {
            return match e.kind() {
                ErrorKind::StorageFull => full(),
                _ => StoreReply::Unreachable(e.to_string()),
            };
        }
        self.seq = Some(version);
        StoreReply::Stored(StoreEntry {
            key: key.to_string(),
            value: value.to_vec(),
            version,
            modified: std::fs::metadata(&path).map_or(0, |m| modified(&m)),
            author_id: String::new(),
            author_name: String::new(),
        })
    }

    pub fn remove(&mut self, key: &str, expect: Option<u64>) -> StoreReply {
        if let Some(why) = key_error(key) {
            return StoreReply::Refused(why);
        }
        let old = match self.read(key) {
            Ok(old) => old,
            Err(why) => return StoreReply::Unreachable(why),
        };
        if !holds(expect, old.as_ref()) {
            return StoreReply::Conflict(old);
        }
        let Some(old) = old else {
            return StoreReply::Removed;
        };
        // The removed version stays known, for the next write.
        let seq = self.seq().max(old.version);
        let kept = write_atomic(&self.dir, &self.dir.join(SEQ), &[seq.to_string().as_bytes()]);
        if let Err(e) = kept.and_then(|()| std::fs::remove_file(self.dir.join(key))) {
            return StoreReply::Unreachable(e.to_string());
        }
        self.seq = Some(seq);
        StoreReply::Removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Removes the directory when dropped.
    struct Dir(PathBuf);

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn store() -> (Dir, LocalStore) {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("localstore-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = LocalStore::new(dir.join("app"));
        (Dir(dir), store)
    }

    fn stored(reply: StoreReply) -> StoreEntry {
        match reply {
            StoreReply::Stored(e) => e,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_key_is_at_most_64_bytes() {
        assert!(key_error(&"k".repeat(64)).is_none());
        assert!(key_error(&"k".repeat(65)).is_some());
        // Bytes, not characters.
        assert!(key_error(&"\u{e9}".repeat(32)).is_none());
        assert!(key_error(&"\u{e9}".repeat(33)).is_some());
        for k in ["", ".", "..", ".hidden", "a/b", "a\\b", "a\0b"] {
            assert!(key_error(k).is_some(), "{k:?}");
        }
    }

    #[test]
    fn versions_change_with_every_write_and_never_repeat() {
        let (_dir, mut s) = store();
        let a = stored(s.set("a", b"1", None)).version;
        let b = stored(s.set("a", b"2", None)).version;
        assert!(a > 0 && b > a);
        assert_eq!(s.remove("a", None), StoreReply::Removed);
        // A new instance of the app: the removed version is remembered.
        let mut s = LocalStore::new(s.dir.clone());
        let c = stored(s.set("a", b"3", Some(0))).version;
        assert!(c > b);
        let StoreReply::Entry(Some(e)) = s.get("a") else { panic!() };
        assert_eq!((e.value.as_slice(), e.version), (&b"3"[..], c));
        assert!(e.modified > 0 && e.author_id.is_empty());
    }

    #[test]
    fn conditional_writes() {
        let (_dir, mut s) = store();
        let v = stored(s.set("k", b"1", Some(0))).version;
        let StoreReply::Conflict(Some(e)) = s.set("k", b"2", Some(0)) else { panic!() };
        assert_eq!(e.version, v);
        assert!(matches!(s.set("k", b"2", Some(v + 1)), StoreReply::Conflict(Some(_))));
        let w = stored(s.set("k", b"2", Some(v))).version;
        assert!(matches!(s.remove("k", Some(v)), StoreReply::Conflict(Some(e)) if e.version == w));
        assert_eq!(s.remove("k", Some(w)), StoreReply::Removed);
        assert_eq!(s.remove("k", None), StoreReply::Removed);
        assert_eq!(s.set("k", b"3", Some(w)), StoreReply::Conflict(None));
    }

    #[test]
    fn lists_are_in_key_order_and_in_pieces() {
        let (_dir, mut s) = store();
        for i in 0..70 {
            stored(s.set(&format!("n.{i:03}"), b"v", None));
        }
        stored(s.set("other", b"v", None));
        let StoreReply::Entries(first, true) = s.list("n.", "") else { panic!() };
        assert_eq!(first.len(), 64);
        assert_eq!(first[0].key, "n.000");
        let StoreReply::Entries(rest, false) = s.list("n.", &first[63].key) else { panic!() };
        assert_eq!(rest.len(), 6);
        assert_eq!(rest[5].key, "n.069");
        // About 16 KiB of values, and always one entry.
        let big = vec![0; 10 * 1024];
        for k in ["big.a", "big.b", "big.c"] {
            stored(s.set(k, &big, None));
        }
        let StoreReply::Entries(piece, true) = s.list("big.", "") else { panic!() };
        assert_eq!(piece.len(), 1);
        let huge = vec![0; VALUE_MAX];
        stored(s.set("huge", &huge, None));
        let StoreReply::Entries(one, false) = s.list("huge", "") else { panic!() };
        assert_eq!(one[0].value.len(), VALUE_MAX);
    }

    #[test]
    fn a_file_put_there_is_all_value() {
        let (_dir, mut s) = store();
        std::fs::create_dir_all(&s.dir).unwrap();
        std::fs::write(s.dir.join("seed"), "42").unwrap();
        let StoreReply::Entry(Some(e)) = s.get("seed") else { panic!() };
        assert_eq!(e.value, b"42");
        assert!(e.version > 1);
        let w = stored(s.set("seed", b"43", Some(e.version))).version;
        assert!(w > e.version);
        assert!(std::fs::read(s.dir.join("seed")).unwrap().starts_with(&MAGIC));
    }

    #[test]
    fn what_cannot_be_stored_is_refused() {
        let (_dir, mut s) = store();
        assert!(matches!(s.set("a/b", b"x", None), StoreReply::Refused(_)));
        assert!(matches!(s.get(".seq"), StoreReply::Refused(_)));
        assert!(matches!(s.set("k", &vec![0; VALUE_MAX + 1], None), StoreReply::Refused(_)));
        assert_eq!(s.get("k"), StoreReply::Entry(None));
        assert_eq!(s.list("", ""), StoreReply::Entries(Vec::new(), false));
    }
}
