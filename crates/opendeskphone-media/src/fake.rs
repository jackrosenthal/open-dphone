// SPDX-License-Identifier: GPL-2.0-or-later
//! Stand-ins for the audio front end, for tests and the host simulator.

use std::io;
use std::sync::{Arc, Mutex};

use crate::device::{AudioDevice, AudioPath, GainControl};

/// Fills a capture buffer.
pub type MicSource = Box<dyn FnMut(&mut [i16]) + Send>;

/// What a [`FakeAudio`] did and what it captures; shared with the test.
pub struct FakeAudioState {
    pub open: Option<AudioPath>,
    pub opens: u32,
    /// Everything written for playback while `record` is set.
    pub played: Vec<i16>,
    pub record: bool,
    pub gains: Vec<(AudioPath, GainControl, i32)>,
    /// Fills capture buffers; silence when `None`.
    pub mic: Option<MicSource>,
    /// Sleep this long in every read, to pace like a real device.
    pub read_delay: Option<std::time::Duration>,
    pub fail_reads: bool,
}

/// An audio device that captures from a generator and records playback.
/// Reads do not block (unless a delay is set), so a test can run the engine
/// faster than real time.
pub struct FakeAudio {
    rate: u32,
    state: Arc<Mutex<FakeAudioState>>,
}

impl FakeAudio {
    pub fn new(rate: u32) -> (Self, Arc<Mutex<FakeAudioState>>) {
        let state = Arc::new(Mutex::new(FakeAudioState {
            open: None,
            opens: 0,
            played: Vec::new(),
            record: true,
            gains: Vec::new(),
            mic: None,
            read_delay: None,
            fail_reads: false,
        }));
        (Self { rate, state: state.clone() }, state)
    }
}

impl AudioDevice for FakeAudio {
    fn rate(&self) -> u32 {
        self.rate
    }

    fn open(&mut self, path: AudioPath) -> io::Result<()> {
        let mut s = self.state.lock().unwrap();
        s.open = Some(path);
        s.opens += 1;
        Ok(())
    }

    fn close(&mut self) {
        self.state.lock().unwrap().open = None;
    }

    fn read(&mut self, buf: &mut [i16]) -> io::Result<()> {
        let delay = {
            let mut s = self.state.lock().unwrap();
            if s.open.is_none() || s.fail_reads {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "fake audio device not readable"));
            }
            match &mut s.mic {
                Some(f) => f(buf),
                None => buf.fill(0),
            }
            s.read_delay
        };
        if let Some(d) = delay {
            std::thread::sleep(d);
        }
        Ok(())
    }

    fn write(&mut self, buf: &[i16]) -> io::Result<()> {
        let mut s = self.state.lock().unwrap();
        if s.open.is_none() {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "fake audio device not open"));
        }
        if s.record {
            s.played.extend_from_slice(buf);
        }
        Ok(())
    }

    fn set_gain(&mut self, path: AudioPath, control: GainControl, value: i32) -> io::Result<()> {
        self.state.lock().unwrap().gains.push((path, control, value));
        Ok(())
    }
}
