//! User ringtones: `/nvdata/ringtones/<id>.sln16`, the files stock's web
//! upload leaves there (signed 16-bit little-endian, mono, 16 kHz, no
//! header; stock's own ringtones in `/app/data/ringtones` have the same
//! form).  `active_ringtone` names one by its ID, exactly as it names a
//! built-in ring.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use dphone_config::Nvdata;
use dphone_media::ring::{BuiltinRing, RingSource};
use log::warn;

/// The largest ringtone file loaded: about a minute of sound.  A guard for
/// memory, not a stock limit.
pub const MAX_BYTES: u64 = 2 << 20;

/// Silence between two passes of a user ringtone.  Stock's value (its
/// `loop_delay`) is not known; its files carry no pause of their own.
pub const PAUSE_MS: u32 = 2000;

static NVDATA: OnceLock<PathBuf> = OnceLock::new();

/// The ringtone last decoded, so that every ring does not read the flash.
type Cached = (String, SystemTime, Arc<[i16]>);
static CACHE: Mutex<Option<Cached>> = Mutex::new(None);

/// Tell this module where nvdata is.  Called once at start.
pub fn init(nvdata: &Path) {
    let _ = NVDATA.set(nvdata.to_path_buf());
}

/// IDs of the user ringtones on the phone, sorted.
pub fn names(nvdata: &Path) -> Vec<String> {
    let mut names = Nvdata::new(nvdata).user_ringtones().unwrap_or_default();
    names.sort();
    names
}

/// Every ringtone the phone can ring with: the built-in rings, the default
/// first, and then the user ringtones.
pub fn all(nvdata: &Path) -> Vec<String> {
    let mut all: Vec<String> = BuiltinRing::ALL.iter().map(|r| r.name().to_string()).collect();
    for name in names(nvdata) {
        if !all.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
            all.push(name);
        }
    }
    all
}

/// Samples of a headerless signed 16-bit little-endian file.
pub fn decode_sln16(bytes: &[u8]) -> Arc<[i16]> {
    bytes.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect()
}

fn load(nvdata: &Path, id: &str) -> Option<Arc<[i16]>> {
    let nv = Nvdata::new(nvdata);
    let path = nv.path(&format!("ringtones/{id}{}", dphone_config::nvdata::RINGTONE_SUFFIX));
    let meta = std::fs::metadata(&path).ok()?;
    if meta.len() > MAX_BYTES {
        warn!("ringtone {id}: {} bytes is more than the {MAX_BYTES} this phone loads", meta.len());
        return None;
    }
    let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let mut cache = CACHE.lock().ok()?;
    if let Some((name, when, samples)) = cache.as_ref()
        && name == id
        && *when == modified
    {
        return Some(samples.clone());
    }
    let samples = match nv.user_ringtone(id) {
        Ok(Some(bytes)) => decode_sln16(&bytes),
        Ok(None) => return None,
        Err(e) => {
            warn!("ringtone {id}: {e}");
            return None;
        }
    };
    if samples.is_empty() {
        return None;
    }
    *cache = Some((id.to_owned(), modified, samples.clone()));
    Some(samples)
}

/// What to ring with for a ringtone name: the built-in ring of that name,
/// else the user ringtone of that ID, else the default built-in ring.
pub fn source_in(nvdata: Option<&Path>, name: &str) -> RingSource {
    if let Some(ring) = BuiltinRing::from_name(name) {
        return RingSource::Builtin(ring);
    }
    match nvdata.and_then(|nv| load(nv, name)) {
        Some(samples) => RingSource::Pcm { samples, pause_ms: PAUSE_MS },
        None => RingSource::Builtin(BuiltinRing::DEFAULT),
    }
}

/// [`source_in`] for the phone's nvdata.
pub fn source(name: &str) -> RingSource {
    source_in(NVDATA.get().map(PathBuf::as_path), name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_ringtones_are_found_by_id() {
        let dir = tempfile::tempdir().unwrap();
        let nv = dir.path();
        std::fs::create_dir_all(nv.join("ringtones")).unwrap();
        // 1, -2, 300 as little-endian samples, and a stray odd byte.
        std::fs::write(nv.join("ringtones/Mine.sln16"), [1, 0, 0xfe, 0xff, 0x2c, 0x01, 7]).unwrap();
        std::fs::write(nv.join("ringtones/Empty.sln16"), []).unwrap();
        std::fs::write(nv.join("ringtones/readme.txt"), b"x").unwrap();
        assert_eq!(names(nv), ["Empty", "Mine"]);

        match source_in(Some(nv), "Mine") {
            RingSource::Pcm { samples, pause_ms } => {
                assert_eq!(&samples[..], [1, -2, 300]);
                assert_eq!(pause_ms, PAUSE_MS);
            }
            other => panic!("{other:?}"),
        }
        // A built-in name is the built-in ring, whatever is on disk.
        std::fs::write(nv.join("ringtones/Chime.sln16"), [1, 0]).unwrap();
        assert!(matches!(source_in(Some(nv), "chime"), RingSource::Builtin(BuiltinRing::Chime)));
        // Every built-in name is its own ring.
        for ring in BuiltinRing::ALL {
            assert!(matches!(source_in(Some(nv), ring.name()), RingSource::Builtin(r) if r == ring), "{ring:?}");
        }
        // Unknown, empty, or unusable names, and names of rings that are
        // no more, ring the default.
        for name in ["Nope", "Soft", "Empty", "../etc/enet", ""] {
            assert!(matches!(source_in(Some(nv), name), RingSource::Builtin(BuiltinRing::DEFAULT)), "{name}");
        }
        assert!(matches!(source_in(None, "Mine"), RingSource::Builtin(BuiltinRing::DEFAULT)));

        // A replaced file is read again.
        std::fs::write(nv.join("ringtones/Mine.sln16"), [5, 0, 6, 0]).unwrap();
        let later = SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options().write(true).open(nv.join("ringtones/Mine.sln16")).unwrap().set_modified(later).unwrap();
        match source_in(Some(nv), "Mine") {
            RingSource::Pcm { samples, .. } => assert_eq!(&samples[..], [5, 6]),
            other => panic!("{other:?}"),
        }
    }
}
