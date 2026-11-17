//! The alarm clock's tones: recordings in `/usr/share/dphone/alarm-tones`
//! (`assets/alarm-tones` in the source, where their origin and license
//! are), separate from the ringtones.  An alarm rings with one of them,
//! around and around with the tone's pause between two passes.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use dphone_media::ring::{BuiltinRing, RingSource, decode_wav};
use log::warn;

/// Where the image keeps the tones.
pub const DIR: &str = "/usr/share/dphone/alarm-tones";
/// Names another directory, for running off the phone.
pub const DIR_ENV: &str = "DPHONE_ALARM_TONES";

pub struct AlarmTone {
    pub name: &'static str,
    file: &'static str,
    /// Silence between two passes.
    pause_ms: u32,
}

/// Every alarm tone; the first is what an alarm without one rings with,
/// and an alarm whose tone is not among them.
pub const ALL: [AlarmTone; 6] = [
    AlarmTone { name: "Beep", file: "beep.wav", pause_ms: 600 },
    AlarmTone { name: "Buzzer", file: "buzzer.wav", pause_ms: 100 },
    AlarmTone { name: "Classic", file: "classic.wav", pause_ms: 1070 },
    AlarmTone { name: "Osmium", file: "osmium.wav", pause_ms: 850 },
    AlarmTone { name: "Rooster", file: "rooster.wav", pause_ms: 2000 },
    AlarmTone { name: "Argon", file: "argon.wav", pause_ms: 0 },
];

/// The tone last decoded, so that every pass of a preview does not read
/// the flash.
static CACHE: Mutex<Option<(PathBuf, Arc<[i16]>)>> = Mutex::new(None);

pub fn names() -> Vec<String> {
    ALL.iter().map(|t| t.name.to_string()).collect()
}

/// The tone of this name, without regard to letter case; the first tone
/// for any other name.
pub fn named(name: &str) -> &'static AlarmTone {
    ALL.iter().find(|t| t.name.eq_ignore_ascii_case(name)).unwrap_or(&ALL[0])
}

fn dir() -> PathBuf {
    std::env::var_os(DIR_ENV).map_or_else(|| PathBuf::from(DIR), PathBuf::from)
}

/// What to ring with for an alarm tone's name, with the tones in `dir`.  A
/// tone whose file cannot be played rings the default ringtone: an alarm
/// is never silent.
pub fn source_in(dir: &Path, name: &str) -> RingSource {
    let tone = named(name);
    let path = dir.join(tone.file);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((file, samples)) = cache.as_ref()
        && *file == path
    {
        return RingSource::Pcm { samples: samples.clone(), pause_ms: tone.pause_ms };
    }
    let decoded = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| decode_wav(&b).map_err(|e| e.to_string()));
    match decoded {
        Ok(samples) if !samples.is_empty() => {
            let samples: Arc<[i16]> = samples.into();
            *cache = Some((path, samples.clone()));
            RingSource::Pcm { samples, pause_ms: tone.pause_ms }
        }
        Ok(_) => {
            warn!("alarm tone {}: {} is empty", tone.name, path.display());
            RingSource::Builtin(BuiltinRing::DEFAULT)
        }
        Err(e) => {
            warn!("alarm tone {}: {}: {e}", tone.name, path.display());
            RingSource::Builtin(BuiltinRing::DEFAULT)
        }
    }
}

/// [`source_in`] for the phone's tones.
pub fn source(name: &str) -> RingSource {
    source_in(&dir(), name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tones as the source has them.
    fn assets() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/alarm-tones")
    }

    #[test]
    fn every_tone_has_its_recording() {
        let mut total = 0;
        for tone in &ALL {
            let bytes = std::fs::read(assets().join(tone.file)).unwrap();
            total += bytes.len();
            // 16 kHz mono mu-law: a second is 16,000 bytes, and nothing
            // is resampled on the phone.
            assert_eq!(&bytes[20..28], [7, 0, 1, 0, 0x80, 0x3e, 0, 0], "{}", tone.name);
            match source_in(&assets(), tone.name) {
                RingSource::Pcm { samples, pause_ms } => {
                    assert_eq!(samples.len(), bytes.len() - 58, "{}", tone.name);
                    assert_eq!(pause_ms, tone.pause_ms);
                    // It can be heard, and a pass ends as quietly as the
                    // pause, or the next pass, begins.
                    assert!(samples.iter().any(|s| s.abs() > 8000), "{}", tone.name);
                    assert!(samples[samples.len() - 16..].iter().all(|s| s.abs() < 400), "{}", tone.name);
                }
                other => panic!("{}: {other:?}", tone.name),
            }
        }
        // What the tones cost in the image, before its compression.
        assert!(total < 160_000, "{total}");
        // The license goes where the tones go.
        for file in ["NOTICE", "LICENSE-Apache-2.0.txt"] {
            assert!(assets().join(file).exists(), "{file}");
        }
    }

    #[test]
    fn a_name_that_is_no_tone_is_the_first() {
        assert_eq!(names(), ["Beep", "Buzzer", "Classic", "Osmium", "Rooster", "Argon"]);
        assert_eq!(named("rooster").name, "Rooster");
        for name in ["Marimba", "Chime", "", "../etc/passwd"] {
            assert_eq!(named(name).name, "Beep", "{name}");
        }
        // Without its file a tone still rings: with the default ringtone.
        let empty = tempfile::tempdir().unwrap();
        assert!(matches!(source_in(empty.path(), "Osmium"), RingSource::Builtin(BuiltinRing::DEFAULT)));
    }
}
