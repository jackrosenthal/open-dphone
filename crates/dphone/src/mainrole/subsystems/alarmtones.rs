//! The alarm clock's tones: recordings in `/usr/share/dphone/alarm-tones`
//! (`assets/alarm-tones` in the source, where their origin and license
//! are), separate from the ringtones.  An alarm rings with one of them,
//! around and around with the tone's pause between two passes.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use dphone_media::ring::{RingSource, decode_wav};
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

/// Every alarm tone: the default first (what an alarm without a tone rings
/// with, and an alarm whose tone is not among these), then the others in
/// alphabetical order.  A tone without a pause runs into its next pass, as
/// its upstream file does when it loops.
pub const ALL: [AlarmTone; 39] = [
    AlarmTone { name: "Beep-Beep-Beep", file: "beep-beep-beep.wav", pause_ms: 625 },
    AlarmTone { name: "Argon", file: "argon.wav", pause_ms: 422 },
    AlarmTone { name: "Argon JP", file: "argon-jp.wav", pause_ms: 770 },
    AlarmTone { name: "Awaken", file: "awaken.wav", pause_ms: 1032 },
    AlarmTone { name: "Barium", file: "barium.wav", pause_ms: 0 },
    AlarmTone { name: "BeeBeep", file: "beebeep.wav", pause_ms: 38 },
    AlarmTone { name: "Bounce", file: "bounce.wav", pause_ms: 78 },
    AlarmTone { name: "Buzzer", file: "buzzer.wav", pause_ms: 103 },
    AlarmTone { name: "Carbon", file: "carbon.wav", pause_ms: 344 },
    AlarmTone { name: "Carbon JP", file: "carbon-jp.wav", pause_ms: 1180 },
    AlarmTone { name: "Cesium", file: "cesium.wav", pause_ms: 1090 },
    AlarmTone { name: "Copernicium", file: "copernicium.wav", pause_ms: 1030 },
    AlarmTone { name: "Curium", file: "curium.wav", pause_ms: 740 },
    AlarmTone { name: "Drip", file: "drip.wav", pause_ms: 1709 },
    AlarmTone { name: "Fermium", file: "fermium.wav", pause_ms: 0 },
    AlarmTone { name: "Gallop", file: "gallop.wav", pause_ms: 120 },
    AlarmTone { name: "Hassium", file: "hassium.wav", pause_ms: 854 },
    AlarmTone { name: "Helium", file: "helium.wav", pause_ms: 0 },
    AlarmTone { name: "Helium JP", file: "helium-jp.wav", pause_ms: 790 },
    AlarmTone { name: "Krypton", file: "krypton.wav", pause_ms: 0 },
    AlarmTone { name: "Neon", file: "neon.wav", pause_ms: 0 },
    AlarmTone { name: "Neptunium", file: "neptunium.wav", pause_ms: 0 },
    AlarmTone { name: "Nobelium", file: "nobelium.wav", pause_ms: 780 },
    AlarmTone { name: "Nudge", file: "nudge.wav", pause_ms: 1284 },
    AlarmTone { name: "Orbit", file: "orbit.wav", pause_ms: 659 },
    AlarmTone { name: "Osmium", file: "osmium.wav", pause_ms: 1077 },
    AlarmTone { name: "Osmium Classic", file: "osmium-classic.wav", pause_ms: 736 },
    AlarmTone { name: "Oxygen", file: "oxygen.wav", pause_ms: 374 },
    AlarmTone { name: "Piezo", file: "piezo.wav", pause_ms: 671 },
    AlarmTone { name: "Platinum", file: "platinum.wav", pause_ms: 52 },
    AlarmTone { name: "Plutonium", file: "plutonium.wav", pause_ms: 1553 },
    AlarmTone { name: "Promethium", file: "promethium.wav", pause_ms: 64 },
    AlarmTone { name: "Rise", file: "rise.wav", pause_ms: 201 },
    AlarmTone { name: "Rooster 1", file: "rooster-1.wav", pause_ms: 2000 },
    AlarmTone { name: "Rooster 2", file: "rooster-2.wav", pause_ms: 2000 },
    AlarmTone { name: "Scandium", file: "scandium.wav", pause_ms: 1468 },
    AlarmTone { name: "Sway", file: "sway.wav", pause_ms: 0 },
    AlarmTone { name: "Timer", file: "timer.wav", pause_ms: 1317 },
    AlarmTone { name: "Wag", file: "wag.wav", pause_ms: 1771 },
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
            super::ringtones::source(super::ringtones::DEFAULT)
        }
        Err(e) => {
            warn!("alarm tone {}: {}: {e}", tone.name, path.display());
            super::ringtones::source(super::ringtones::DEFAULT)
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
            assert_eq!(&bytes[50..54], b"data", "{}", tone.name);
            let len = u32::from_le_bytes(bytes[54..58].try_into().unwrap()) as usize;
            match source_in(&assets(), tone.name) {
                RingSource::Pcm { samples, pause_ms } => {
                    assert_eq!(samples.len(), len, "{}", tone.name);
                    assert_eq!(pause_ms, tone.pause_ms);
                    // It can be heard, and it does not clip.
                    let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
                    assert!((8000..32000).contains(&peak), "{}: {peak}", tone.name);
                    if tone.pause_ms > 0 {
                        // A pass ends as quietly as the pause.
                        assert!(samples[len - 16..].iter().all(|s| s.abs() < 400), "{}", tone.name);
                    } else {
                        // A pass runs into the next without a step that
                        // the sound does not have anyway.
                        let largest = samples.windows(2).map(|w| (w[1] as i32 - w[0] as i32).abs()).max().unwrap();
                        let seam = (samples[0] as i32 - samples[len - 1] as i32).abs();
                        assert!(seam <= largest, "{}: {seam} > {largest}", tone.name);
                    }
                }
                other => panic!("{}: {other:?}", tone.name),
            }
        }
        // What the tones cost in the image, before its compression.
        assert!(total < 4_700_000, "{total}");
        // The license goes where the tones go.
        for file in ["NOTICE", "LICENSE-Apache-2.0.txt"] {
            assert!(assets().join(file).exists(), "{file}");
        }
    }

    #[test]
    fn a_name_that_is_no_tone_is_the_first() {
        let names = names();
        assert_eq!(names.len(), 39);
        assert_eq!(names[..4], ["Beep-Beep-Beep", "Argon", "Argon JP", "Awaken"]);
        // After the default, A to Z, and every name once.
        assert!(names[1..].windows(2).all(|w| w[0].to_lowercase() < w[1].to_lowercase()), "{names:?}");
        for (i, t) in ALL.iter().enumerate() {
            assert!(ALL[..i].iter().all(|u| u.file != t.file), "{}", t.file);
        }
        assert_eq!(named("rooster 2").name, "Rooster 2");
        for name in ["Marimba", "Chime", "", "../etc/passwd"] {
            assert_eq!(named(name).name, "Beep-Beep-Beep", "{name}");
        }
        // Without its file a tone still rings: with the default ringtone.
        let empty = tempfile::tempdir().unwrap();
        let default = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ringtones/flutey-phone.wav");
        let default = decode_wav(&std::fs::read(default).unwrap()).unwrap();
        match source_in(empty.path(), "Osmium") {
            RingSource::Pcm { samples, .. } => assert_eq!(samples[..], default[..]),
            other => panic!("{other:?}"),
        }
    }
}
