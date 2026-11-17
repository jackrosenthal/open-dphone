//! The ringtones.  The built-in ones are recordings in
//! `/usr/share/dphone/ringtones` (`assets/ringtones` in the source, where
//! their origin and license are).  The user's own are
//! `/nvdata/ringtones/<id>.sln16`, the files stock's web upload leaves
//! there (signed 16-bit little-endian, mono, 16 kHz, no header; stock's
//! own ringtones in `/app/data/ringtones` have the same form).
//! `active_ringtone` names a built-in ringtone by its name and a user
//! ringtone by its ID.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use dphone_config::Nvdata;
use dphone_media::ring::{RingSource, decode_wav};
use log::warn;

/// Where the image keeps the built-in ringtones.
pub const DIR: &str = "/usr/share/dphone/ringtones";
/// Names another directory, for running off the phone.
pub const DIR_ENV: &str = "DPHONE_RINGTONES";

/// The largest ringtone file loaded: about a minute of sound.  A guard for
/// memory, not a stock limit.
pub const MAX_BYTES: u64 = 2 << 20;

/// Silence between two passes of a user ringtone.  Stock's value (its
/// `loop_delay`) is not known; its files carry no pause of their own.
pub const PAUSE_MS: u32 = 2000;

pub struct Ringtone {
    pub name: &'static str,
    file: &'static str,
    /// Silence between two passes.
    pause_ms: u32,
}

/// Every built-in ringtone: the default first (what the phone rings with
/// when nothing else is chosen, and for a name that is no ringtone), then
/// the others in alphabetical order.  A ringtone without a pause runs into
/// its next pass, as its upstream file does when it loops.
pub const BUILTIN: [Ringtone; 149] = [
    Ringtone { name: "Solarium", file: "solarium.wav", pause_ms: 804 },
    Ringtone { name: "Acheron", file: "acheron.wav", pause_ms: 634 },
    Ringtone { name: "Andromeda", file: "andromeda.wav", pause_ms: 1350 },
    Ringtone { name: "Aquila", file: "aquila.wav", pause_ms: 1750 },
    Ringtone { name: "Argo Navis", file: "argo-navis.wav", pause_ms: 1328 },
    Ringtone { name: "Atria", file: "atria.wav", pause_ms: 0 },
    Ringtone { name: "Backroad", file: "backroad.wav", pause_ms: 0 },
    Ringtone { name: "Beat Plucker", file: "beat-plucker.wav", pause_ms: 0 },
    Ringtone { name: "Bell Phone", file: "bell-phone.wav", pause_ms: 1454 },
    Ringtone { name: "Bentley Dubs", file: "bentley-dubs.wav", pause_ms: 44 },
    Ringtone { name: "Big Easy", file: "big-easy.wav", pause_ms: 0 },
    Ringtone { name: "Bird Loop", file: "bird-loop.wav", pause_ms: 0 },
    Ringtone { name: "Bollywood", file: "bollywood.wav", pause_ms: 0 },
    Ringtone { name: "Boötes", file: "bootes.wav", pause_ms: 0 },
    Ringtone { name: "Bus' a Move", file: "bus-a-move.wav", pause_ms: 0 },
    Ringtone { name: "Cairo", file: "cairo.wav", pause_ms: 0 },
    Ringtone { name: "Callisto", file: "callisto.wav", pause_ms: 0 },
    Ringtone { name: "Callisto Classic", file: "callisto-classic.wav", pause_ms: 0 },
    Ringtone { name: "Calypso Steel", file: "calypso-steel.wav", pause_ms: 0 },
    Ringtone { name: "Canis Major", file: "canis-major.wav", pause_ms: 0 },
    Ringtone { name: "Caribbean Ice", file: "caribbean-ice.wav", pause_ms: 0 },
    Ringtone { name: "Carina", file: "carina.wav", pause_ms: 1501 },
    Ringtone { name: "Cassiopeia", file: "cassiopeia.wav", pause_ms: 760 },
    Ringtone { name: "Centaurus", file: "centaurus.wav", pause_ms: 290 },
    Ringtone { name: "Champagne Edition", file: "champagne-edition.wav", pause_ms: 0 },
    Ringtone { name: "Chimey Phone", file: "chimey-phone.wav", pause_ms: 1514 },
    Ringtone { name: "Classic 1", file: "classic-1.wav", pause_ms: 662 },
    Ringtone { name: "Classic 3", file: "classic-3.wav", pause_ms: 1437 },
    Ringtone { name: "Classic 4", file: "classic-4.wav", pause_ms: 1374 },
    Ringtone { name: "Classic 5", file: "classic-5.wav", pause_ms: 2061 },
    Ringtone { name: "Club Cubano", file: "club-cubano.wav", pause_ms: 0 },
    Ringtone { name: "Crayon Rock", file: "crayon-rock.wav", pause_ms: 0 },
    Ringtone { name: "Crazy Dream", file: "crazy-dream.wav", pause_ms: 0 },
    Ringtone { name: "Curve Ball Blend", file: "curve-ball-blend.wav", pause_ms: 0 },
    Ringtone { name: "Cygnus", file: "cygnus.wav", pause_ms: 740 },
    Ringtone { name: "Dancin' Fool", file: "dancin-fool.wav", pause_ms: 0 },
    Ringtone { name: "Digital 1", file: "digital-1.wav", pause_ms: 2381 },
    Ringtone { name: "Digital 3", file: "digital-3.wav", pause_ms: 2461 },
    Ringtone { name: "Digital 4", file: "digital-4.wav", pause_ms: 2481 },
    Ringtone { name: "Digital 5", file: "digital-5.wav", pause_ms: 37 },
    Ringtone { name: "Digital Phone", file: "digital-phone.wav", pause_ms: 2499 },
    Ringtone { name: "Dione", file: "dione.wav", pause_ms: 620 },
    Ringtone { name: "Dione Classic", file: "dione-classic.wav", pause_ms: 680 },
    Ringtone { name: "Don' Mess Wiv It", file: "don-mess-wiv-it.wav", pause_ms: 0 },
    Ringtone { name: "Draco", file: "draco.wav", pause_ms: 2150 },
    Ringtone { name: "Dream Theme", file: "dream-theme.wav", pause_ms: 0 },
    Ringtone { name: "Eastern Sky", file: "eastern-sky.wav", pause_ms: 0 },
    Ringtone { name: "Enter the Nexus", file: "enter-the-nexus.wav", pause_ms: 0 },
    Ringtone { name: "Eridani", file: "eridani.wav", pause_ms: 1000 },
    Ringtone { name: "Ether Shake", file: "ether-shake.wav", pause_ms: 0 },
    Ringtone { name: "Flutey Phone", file: "flutey-phone.wav", pause_ms: 1973 },
    Ringtone { name: "Free Flight", file: "free-flight.wav", pause_ms: 0 },
    Ringtone { name: "Friendly Ghost", file: "friendly-ghost.wav", pause_ms: 0 },
    Ringtone { name: "Funk Y'all", file: "funk-yall.wav", pause_ms: 0 },
    Ringtone { name: "Game Over Guitar", file: "game-over-guitar.wav", pause_ms: 0 },
    Ringtone { name: "Ganymede", file: "ganymede.wav", pause_ms: 61 },
    Ringtone { name: "Ganymede Classic", file: "ganymede-classic.wav", pause_ms: 259 },
    Ringtone { name: "Gimme Mo' Town", file: "gimme-mo-town.wav", pause_ms: 0 },
    Ringtone { name: "Girtab", file: "girtab.wav", pause_ms: 930 },
    Ringtone { name: "Glacial Groove", file: "glacial-groove.wav", pause_ms: 0 },
    Ringtone { name: "Growl", file: "growl.wav", pause_ms: 0 },
    Ringtone { name: "Halfway Home", file: "halfway-home.wav", pause_ms: 0 },
    Ringtone { name: "Hydra", file: "hydra.wav", pause_ms: 2050 },
    Ringtone { name: "Insert Coin", file: "insert-coin.wav", pause_ms: 250 },
    Ringtone { name: "Iridium", file: "iridium.wav", pause_ms: 500 },
    Ringtone { name: "Kuma", file: "kuma.wav", pause_ms: 0 },
    Ringtone { name: "Loopy Lounge", file: "loopy-lounge.wav", pause_ms: 0 },
    Ringtone { name: "Love Flute", file: "love-flute.wav", pause_ms: 0 },
    Ringtone { name: "Luna", file: "luna.wav", pause_ms: 1556 },
    Ringtone { name: "Luna Classic", file: "luna-classic.wav", pause_ms: 0 },
    Ringtone { name: "Lyra", file: "lyra.wav", pause_ms: 1210 },
    Ringtone { name: "Lyra Classic", file: "lyra-classic.wav", pause_ms: 526 },
    Ringtone { name: "Machina", file: "machina.wav", pause_ms: 1005 },
    Ringtone { name: "Medieval Jaunt", file: "medieval-jaunt.wav", pause_ms: 0 },
    Ringtone { name: "Nairobi", file: "nairobi.wav", pause_ms: 0 },
    Ringtone { name: "Nasqueron", file: "nasqueron.wav", pause_ms: 1100 },
    Ringtone { name: "Nassau", file: "nassau.wav", pause_ms: 0 },
    Ringtone { name: "New Player", file: "new-player.wav", pause_ms: 350 },
    Ringtone { name: "No Limits", file: "no-limits.wav", pause_ms: 0 },
    Ringtone { name: "Noisey One", file: "noisey-one.wav", pause_ms: 0 },
    Ringtone { name: "Oberon", file: "oberon.wav", pause_ms: 70 },
    Ringtone { name: "Oberon Classic", file: "oberon-classic.wav", pause_ms: 107 },
    Ringtone { name: "Organ Dub", file: "organ-dub.wav", pause_ms: 0 },
    Ringtone { name: "Orion", file: "orion.wav", pause_ms: 0 },
    Ringtone { name: "Paradise Island", file: "paradise-island.wav", pause_ms: 0 },
    Ringtone { name: "Pegasus", file: "pegasus.wav", pause_ms: 0 },
    Ringtone { name: "Perseus", file: "perseus.wav", pause_ms: 0 },
    Ringtone { name: "Phobos", file: "phobos.wav", pause_ms: 1845 },
    Ringtone { name: "Phobos Classic", file: "phobos-classic.wav", pause_ms: 0 },
    Ringtone { name: "Playa", file: "playa.wav", pause_ms: 0 },
    Ringtone { name: "Pyxis", file: "pyxis.wav", pause_ms: 0 },
    Ringtone { name: "Rasalas", file: "rasalas.wav", pause_ms: 235 },
    Ringtone { name: "Revelation", file: "revelation.wav", pause_ms: 0 },
    Ringtone { name: "Rigel", file: "rigel.wav", pause_ms: 304 },
    Ringtone { name: "Ringtone 18", file: "ringtone-18.wav", pause_ms: 557 },
    Ringtone { name: "Ringtone 19", file: "ringtone-19.wav", pause_ms: 1130 },
    Ringtone { name: "Ringtone 20", file: "ringtone-20.wav", pause_ms: 1170 },
    Ringtone { name: "Ringtone 21", file: "ringtone-21.wav", pause_ms: 940 },
    Ringtone { name: "Ringtone 23", file: "ringtone-23.wav", pause_ms: 1750 },
    Ringtone { name: "Ringtone 24", file: "ringtone-24.wav", pause_ms: 1375 },
    Ringtone { name: "Ringtone 25", file: "ringtone-25.wav", pause_ms: 889 },
    Ringtone { name: "Ringtone 26", file: "ringtone-26.wav", pause_ms: 879 },
    Ringtone { name: "Ringtone 28", file: "ringtone-28.wav", pause_ms: 693 },
    Ringtone { name: "Ringtone 29", file: "ringtone-29.wav", pause_ms: 1780 },
    Ringtone { name: "Ringtone 30", file: "ringtone-30.wav", pause_ms: 1157 },
    Ringtone { name: "Ringtone 31", file: "ringtone-31.wav", pause_ms: 773 },
    Ringtone { name: "Ringtone 32", file: "ringtone-32.wav", pause_ms: 664 },
    Ringtone { name: "Ringtone 33", file: "ringtone-33.wav", pause_ms: 645 },
    Ringtone { name: "Ringtone 34", file: "ringtone-34.wav", pause_ms: 834 },
    Ringtone { name: "Ringtone 35", file: "ringtone-35.wav", pause_ms: 992 },
    Ringtone { name: "Ringtone 36", file: "ringtone-36.wav", pause_ms: 0 },
    Ringtone { name: "Ringtone 37", file: "ringtone-37.wav", pause_ms: 697 },
    Ringtone { name: "Ringtone 38", file: "ringtone-38.wav", pause_ms: 940 },
    Ringtone { name: "Road Trip", file: "road-trip.wav", pause_ms: 0 },
    Ringtone { name: "Romancing The Tone", file: "romancing-the-tone.wav", pause_ms: 0 },
    Ringtone { name: "Safari", file: "safari.wav", pause_ms: 0 },
    Ringtone { name: "Savannah", file: "savannah.wav", pause_ms: 0 },
    Ringtone { name: "Scarabaeus", file: "scarabaeus.wav", pause_ms: 531 },
    Ringtone { name: "Sceptrum", file: "sceptrum.wav", pause_ms: 300 },
    Ringtone { name: "Sceptrum Classic", file: "sceptrum-classic.wav", pause_ms: 708 },
    Ringtone { name: "Sedna", file: "sedna.wav", pause_ms: 117 },
    Ringtone { name: "Seville", file: "seville.wav", pause_ms: 0 },
    Ringtone { name: "She's All That", file: "shes-all-that.wav", pause_ms: 0 },
    Ringtone { name: "Silky Way", file: "silky-way.wav", pause_ms: 0 },
    Ringtone { name: "Sitar Versus Sitar", file: "sitar-versus-sitar.wav", pause_ms: 0 },
    Ringtone { name: "Solarium Classic", file: "solarium-classic.wav", pause_ms: 1075 },
    Ringtone { name: "Springy Jalopy", file: "springy-jalopy.wav", pause_ms: 0 },
    Ringtone { name: "Steppin' Out", file: "steppin-out.wav", pause_ms: 0 },
    Ringtone { name: "Synth 1", file: "synth-1.wav", pause_ms: 1292 },
    Ringtone { name: "Synth 3", file: "synth-3.wav", pause_ms: 2155 },
    Ringtone { name: "Synth 5", file: "synth-5.wav", pause_ms: 51 },
    Ringtone { name: "Terminated", file: "terminated.wav", pause_ms: 0 },
    Ringtone { name: "Terrible Twos", file: "terrible-twos.wav", pause_ms: 0 },
    Ringtone { name: "Testudo", file: "testudo.wav", pause_ms: 416 },
    Ringtone { name: "Third Eye", file: "third-eye.wav", pause_ms: 0 },
    Ringtone { name: "Thriller Three", file: "thriller-three.wav", pause_ms: 0 },
    Ringtone { name: "Thunderfoot", file: "thunderfoot.wav", pause_ms: 0 },
    Ringtone { name: "Titania", file: "titania.wav", pause_ms: 0 },
    Ringtone { name: "Triton", file: "triton.wav", pause_ms: 533 },
    Ringtone { name: "Triton Classic", file: "triton-classic.wav", pause_ms: 0 },
    Ringtone { name: "Twirl Away", file: "twirl-away.wav", pause_ms: 0 },
    Ringtone { name: "Umbriel", file: "umbriel.wav", pause_ms: 1117 },
    Ringtone { name: "Umbriel Classic", file: "umbriel-classic.wav", pause_ms: 0 },
    Ringtone { name: "Ursa Minor", file: "ursa-minor.wav", pause_ms: 50 },
    Ringtone { name: "Ursa Minor Classic", file: "ursa-minor-classic.wav", pause_ms: 0 },
    Ringtone { name: "Very Alarmed", file: "very-alarmed.wav", pause_ms: 0 },
    Ringtone { name: "Vespa", file: "vespa.wav", pause_ms: 2117 },
    Ringtone { name: "World", file: "world.wav", pause_ms: 0 },
    Ringtone { name: "Zeta", file: "zeta.wav", pause_ms: 1190 },
];

/// The default ringtone's name.
pub const DEFAULT: &str = BUILTIN[0].name;

/// The default ringtone's recording, carried in the program: what rings
/// when a ringtone's file cannot be played, so that the phone never rings
/// silent.
const DEFAULT_WAV: &[u8] = include_bytes!("../../../../../assets/ringtones/solarium.wav");

static NVDATA: OnceLock<PathBuf> = OnceLock::new();

/// The ringtone file last decoded, so that every ring does not read the
/// flash.
type Cached = (PathBuf, SystemTime, Arc<[i16]>);
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

/// Every ringtone the phone can ring with: the built-in ones, the default
/// first, and then the user ringtones (a file named like a built-in
/// ringtone is not offered twice).
pub fn all(nvdata: &Path) -> Vec<String> {
    let mut all: Vec<String> = BUILTIN.iter().map(|r| r.name.to_string()).collect();
    for name in names(nvdata) {
        if builtin(&name).is_none() {
            all.push(name);
        }
    }
    all
}

/// The built-in ringtone of this name, without regard to letter case.
pub fn builtin(name: &str) -> Option<&'static Ringtone> {
    BUILTIN.iter().find(|r| r.name.eq_ignore_ascii_case(name))
}

/// Samples of a headerless signed 16-bit little-endian file.
pub fn decode_sln16(bytes: &[u8]) -> Arc<[i16]> {
    bytes.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect()
}

/// The samples of a ringtone file: the cached ones while the file is
/// unchanged, else what `read` reads and decodes.
fn load(path: &Path, read: impl FnOnce() -> Result<Arc<[i16]>, String>) -> Result<Arc<[i16]>, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_BYTES {
        return Err(format!("{} bytes is more than the {MAX_BYTES} this phone loads", meta.len()));
    }
    let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((file, when, samples)) = cache.as_ref()
        && file == path
        && *when == modified
    {
        return Ok(samples.clone());
    }
    let samples = read()?;
    if samples.is_empty() {
        return Err("no sound".into());
    }
    *cache = Some((path.to_path_buf(), modified, samples.clone()));
    Ok(samples)
}

/// The samples of the user ringtone of this ID, if there is one that can
/// be played.
fn user(nvdata: &Path, id: &str) -> Option<Arc<[i16]>> {
    let nv = Nvdata::new(nvdata);
    let path = nv.path(&format!("ringtones/{id}{}", dphone_config::nvdata::RINGTONE_SUFFIX));
    // Most names are no user ringtone (they name a stock ringtone, say).
    if !path.exists() {
        return None;
    }
    // `user_ringtone` refuses an ID that is no plain file name.
    let read = || match nv.user_ringtone(id) {
        Ok(Some(bytes)) => Ok(decode_sln16(&bytes)),
        Ok(None) => Err("gone".to_string()),
        Err(e) => Err(e.to_string()),
    };
    load(&path, read).map_err(|e| warn!("ringtone {id}: {e}")).ok()
}

/// The default ringtone from the program's copy.
pub fn fallback() -> RingSource {
    static SAMPLES: OnceLock<Arc<[i16]>> = OnceLock::new();
    let samples = SAMPLES.get_or_init(|| decode_wav(DEFAULT_WAV).expect("the default ringtone is a WAV file").into());
    RingSource::Pcm { samples: samples.clone(), pause_ms: BUILTIN[0].pause_ms }
}

/// What to ring with for a ringtone name, with the built-in ringtones in
/// `dir`: the built-in ringtone of that name (whatever is in nvdata), else
/// the user ringtone of that ID, else the default.  A built-in ringtone
/// whose file cannot be played rings the program's copy of the default.
pub fn source_in(nvdata: Option<&Path>, dir: &Path, name: &str) -> RingSource {
    let tone = match builtin(name) {
        Some(tone) => tone,
        None => {
            if let Some(samples) = nvdata.and_then(|nv| user(nv, name)) {
                return RingSource::Pcm { samples, pause_ms: PAUSE_MS };
            }
            &BUILTIN[0]
        }
    };
    let path = dir.join(tone.file);
    let read = || {
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        decode_wav(&bytes).map(Arc::from).map_err(|e| e.to_string())
    };
    match load(&path, read) {
        Ok(samples) => RingSource::Pcm { samples, pause_ms: tone.pause_ms },
        Err(e) => {
            warn!("ringtone {}: {}: {e}", tone.name, path.display());
            fallback()
        }
    }
}

fn dir() -> PathBuf {
    std::env::var_os(DIR_ENV).map_or_else(|| PathBuf::from(DIR), PathBuf::from)
}

/// [`source_in`] for the phone's nvdata and built-in ringtones.
pub fn source(name: &str) -> RingSource {
    source_in(NVDATA.get().map(PathBuf::as_path), &dir(), name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The built-in ringtones as the source has them.
    fn assets() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ringtones")
    }

    fn pcm(source: RingSource) -> (Arc<[i16]>, u32) {
        match source {
            RingSource::Pcm { samples, pause_ms } => (samples, pause_ms),
            other => panic!("{other:?}"),
        }
    }

    /// The samples of a built-in ringtone's file.
    fn recording(tone: &Ringtone) -> Arc<[i16]> {
        decode_wav(&std::fs::read(assets().join(tone.file)).unwrap()).unwrap().into()
    }

    #[test]
    fn every_ringtone_has_its_recording() {
        let mut total = 0;
        let notice = std::fs::read_to_string(assets().join("NOTICE")).unwrap();
        for tone in &BUILTIN {
            let bytes = std::fs::read(assets().join(tone.file)).unwrap();
            total += bytes.len();
            // 16 kHz mono mu-law: a second is 16,000 bytes, and nothing
            // is resampled on the phone.
            assert_eq!(&bytes[20..28], [7, 0, 1, 0, 0x80, 0x3e, 0, 0], "{}", tone.name);
            assert_eq!(&bytes[50..54], b"data", "{}", tone.name);
            let len = u32::from_le_bytes(bytes[54..58].try_into().unwrap()) as usize;
            let (samples, pause_ms) = pcm(source_in(None, &assets(), tone.name));
            assert_eq!(samples.len(), len, "{}", tone.name);
            assert_eq!(pause_ms, tone.pause_ms);
            // It can be heard, and it does not clip.
            let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
            assert!((8000..32000).contains(&peak), "{}: {peak}", tone.name);
            if tone.pause_ms > 0 {
                // A pass ends as quietly as the pause.
                assert!(samples[len - 16..].iter().all(|s| s.abs() < 400), "{}", tone.name);
            } else if !["Love Flute", "Revelation"].contains(&tone.name) {
                // A pass runs into the next without a step that the sound
                // does not have anyway.  (Love Flute and Revelation step
                // where they loop as their upstream files do: decoded at
                // 44.1 kHz, by 12,490 and 13,437, against a largest step
                // inside the sound of 4,186 and 5,376.)
                let largest = samples.windows(2).map(|w| (w[1] as i32 - w[0] as i32).abs()).max().unwrap();
                let seam = (samples[0] as i32 - samples[len - 1] as i32).abs();
                assert!(seam <= largest, "{}: {seam} > {largest}", tone.name);
            }
            // The notice names each file it covers.
            assert!(notice.lines().any(|l| l.split_whitespace().next() == Some(tone.file)), "{}", tone.file);
        }
        // What the ringtones cost in the image, before its compression.
        assert!(total < 16_000_000, "{total}");
        // The image installs every .wav of the directory: each is one of
        // these.
        for entry in std::fs::read_dir(assets()).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            if name.ends_with(".wav") {
                assert!(BUILTIN.iter().any(|t| t.file == name), "{name}");
            }
        }
        // The license goes where the ringtones go.
        for file in ["NOTICE", "LICENSE-Apache-2.0.txt"] {
            assert!(assets().join(file).exists(), "{file}");
        }
    }

    #[test]
    fn a_name_that_is_no_ringtone_is_the_default() {
        let names: Vec<&str> = BUILTIN.iter().map(|t| t.name).collect();
        // After the default, A to Z, and every name and file once.
        assert!(names[1..].windows(2).all(|w| w[0].to_lowercase() < w[1].to_lowercase()), "{names:?}");
        for (i, t) in BUILTIN.iter().enumerate() {
            assert!(BUILTIN[..i].iter().all(|u| u.file != t.file), "{}", t.file);
        }
        assert_eq!(builtin("bell phone").map(|t| t.name), Some("Bell Phone"));
        let default = recording(&BUILTIN[0]);
        // The six synthesized ringtones of earlier firmware, the stock
        // firmware's ringtones, and any other name ring the default.
        for name in ["Marimba", "Classic", "Chime", "Pulse", "Rise", "Mellow", "Digium", "Chimes", "GuitarStrum", "", "Nope"] {
            let (samples, pause_ms) = pcm(source_in(None, &assets(), name));
            assert_eq!((&samples[..], pause_ms), (&default[..], BUILTIN[0].pause_ms), "{name}");
        }
        // The program's copy of the default is the default's file, and it
        // rings when a ringtone's file cannot be played.
        assert_eq!(DEFAULT_WAV, std::fs::read(assets().join(BUILTIN[0].file)).unwrap());
        let empty = tempfile::tempdir().unwrap();
        let (samples, pause_ms) = pcm(source_in(None, empty.path(), "Pegasus"));
        assert_eq!((&samples[..], pause_ms), (&default[..], BUILTIN[0].pause_ms));
    }

    #[test]
    fn user_ringtones_are_found_by_id() {
        let dir = tempfile::tempdir().unwrap();
        let nv = dir.path();
        std::fs::create_dir_all(nv.join("ringtones")).unwrap();
        // 1, -2, 300 as little-endian samples, and a stray odd byte.
        std::fs::write(nv.join("ringtones/Mine.sln16"), [1, 0, 0xfe, 0xff, 0x2c, 0x01, 7]).unwrap();
        std::fs::write(nv.join("ringtones/Empty.sln16"), []).unwrap();
        std::fs::write(nv.join("ringtones/readme.txt"), b"x").unwrap();
        // One named like a built-in ringtone.
        std::fs::write(nv.join("ringtones/zeta.sln16"), [1, 0]).unwrap();
        assert_eq!(names(nv), ["Empty", "Mine", "zeta"]);
        let all = all(nv);
        assert_eq!(all[0], DEFAULT);
        assert_eq!(all[BUILTIN.len()..], ["Empty", "Mine"]);

        let (samples, pause_ms) = pcm(source_in(Some(nv), &assets(), "Mine"));
        assert_eq!(&samples[..], [1, -2, 300]);
        assert_eq!(pause_ms, PAUSE_MS);
        // A built-in name is the built-in ringtone, whatever is on disk.
        let zeta = builtin("Zeta").unwrap();
        assert_eq!(pcm(source_in(Some(nv), &assets(), "ZETA")).0[..], recording(zeta)[..]);
        // Every built-in name is its own ringtone.
        for tone in &BUILTIN {
            assert_eq!(pcm(source_in(Some(nv), &assets(), tone.name)).0[..], recording(tone)[..], "{}", tone.name);
        }
        // Unknown, empty, or unusable names ring the default.
        let default = recording(&BUILTIN[0]);
        for name in ["Nope", "Empty", "../etc/enet", ""] {
            assert_eq!(pcm(source_in(Some(nv), &assets(), name)).0[..], default[..], "{name}");
        }
        assert_eq!(pcm(source_in(None, &assets(), "Mine")).0[..], default[..]);

        // A replaced file is read again.
        std::fs::write(nv.join("ringtones/Mine.sln16"), [5, 0, 6, 0]).unwrap();
        let later = SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options().write(true).open(nv.join("ringtones/Mine.sln16")).unwrap().set_modified(later).unwrap();
        assert_eq!(pcm(source_in(Some(nv), &assets(), "Mine")).0[..], [5, 6]);
    }
}
