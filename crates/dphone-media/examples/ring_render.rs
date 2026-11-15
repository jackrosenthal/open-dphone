// SPDX-License-Identifier: GPL-2.0-or-later
//! Renders the built-in ringtones to WAV files for listening on a
//! workstation, through the player the phone rings with:
//!
//! ```text
//! cargo run -p dphone-media --example ring_render -- DIR [CYCLES]
//! ```
//!
//! writes `DIR/<name>.wav` (16 kHz mono, `CYCLES` cycles, 3 by default) and
//! `DIR/<name>-speaker.wav`, the same through a second-order 450 Hz
//! high-pass filter: roughly what a desk phone's small loudspeaker leaves.
//!
//! `ring_render --ticks NAME N` only produces `N` 10 ms ticks of one ring
//! and writes nothing; it is for measuring the cost of the synthesis.

use clap::Parser;

use std::f64::consts::PI;
use std::path::{Path, PathBuf};

use dphone_media::ring::{BuiltinRing, RING_RATE, RingPlayer, RingSource};

fn wav(path: &Path, pcm: &[i16]) -> std::io::Result<()> {
    let mut w = Vec::with_capacity(44 + pcm.len() * 2);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + pcm.len() as u32 * 2).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&RING_RATE.to_le_bytes());
    w.extend_from_slice(&(RING_RATE * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(pcm.len() as u32 * 2).to_le_bytes());
    w.extend(pcm.iter().flat_map(|s| s.to_le_bytes()));
    std::fs::write(path, w)
}

/// A second-order Butterworth high-pass filter at 450 Hz.
fn speaker(pcm: &[i16]) -> Vec<i16> {
    let w0 = 2.0 * PI * 450.0 / f64::from(RING_RATE);
    let alpha = w0.sin() / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
    let a0 = 1.0 + alpha;
    let b = [(1.0 + w0.cos()) / 2.0 / a0, -(1.0 + w0.cos()) / a0, (1.0 + w0.cos()) / 2.0 / a0];
    let a = [-2.0 * w0.cos() / a0, (1.0 - alpha) / a0];
    let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
    pcm.iter()
        .map(|&s| {
            let x = f64::from(s);
            let y = b[0] * x + b[1] * x1 + b[2] * x2 - a[0] * y1 - a[1] * y2;
            (x2, x1, y2, y1) = (x1, x, y1, y);
            y.round().clamp(-32768.0, 32767.0) as i16
        })
        .collect()
}

fn ticks(ring: BuiltinRing, n: u32) -> Vec<i16> {
    let mut player = RingPlayer::new(RingSource::Builtin(ring));
    let mut pcm = Vec::with_capacity(n as usize * 160);
    for _ in 0..n {
        let mut buf = [0i16; 160];
        player.fill(&mut buf);
        pcm.extend_from_slice(&buf);
    }
    pcm
}

/// Render the built-in ringtones to WAV files, or time their synthesis
#[derive(Parser)]
struct Args {
    /// Where to write the WAV files
    #[arg(required_unless_present = "ticks")]
    dir: Option<PathBuf>,
    /// How many cycles of each ring to render
    #[arg(default_value_t = 3)]
    cycles: u32,
    /// Write nothing: only produce N 10 ms ticks of the ring NAME
    #[arg(long, num_args = 2, value_names = ["NAME", "N"], conflicts_with = "dir")]
    ticks: Option<Vec<String>>,
}

fn main() -> std::io::Result<()> {
    let args = Args::parse();
    if let Some(t) = &args.ticks {
        let ring = BuiltinRing::from_name(&t[0]).expect("the name of a built-in ring");
        let n = t[1].parse().expect("a number of ticks");
        let pcm = ticks(ring, n);
        std::hint::black_box(&pcm);
        return Ok(());
    }
    let dir = args.dir.as_deref().expect("clap requires one");
    let cycles = args.cycles;
    std::fs::create_dir_all(dir)?;
    for ring in BuiltinRing::ALL {
        let pcm = ticks(ring, ring.cycle_ms() / 10 * cycles);
        wav(&dir.join(format!("{}.wav", ring.name())), &pcm)?;
        wav(&dir.join(format!("{}-speaker.wav", ring.name())), &speaker(&pcm))?;
        println!("{}: {} cycles of {} ms", ring.name(), cycles, ring.cycle_ms());
    }
    Ok(())
}
