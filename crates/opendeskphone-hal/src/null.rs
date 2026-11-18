//! Null devices: stand-ins for hardware that could not be opened.  Every
//! call fails with [`HalError::Unavailable`] (queries return empty sets).

use crate::*;
use std::os::fd::RawFd;
use std::time::Duration;

/// A missing device; the string says which.
pub struct Null(pub &'static str);

impl Null {
    fn err<T>(&self) -> Result<T> {
        Err(HalError::Unavailable(self.0.to_string()))
    }
}

impl Display for Null {
    fn screens(&self) -> Vec<Screen> {
        Vec::new()
    }
    fn info(&self, _: Screen) -> Option<FrameInfo> {
        None
    }
    fn frame(&mut self, _: Screen) -> Result<&mut [u8]> {
        self.err()
    }
    fn flush(&mut self, _: Screen, _: &[Rect]) -> Result<()> {
        self.err()
    }
    fn capture(&self, _: Screen) -> Result<Frame> {
        self.err()
    }
    fn set_power(&mut self, _: Screen, _: bool) -> Result<()> {
        self.err()
    }
}

impl Backlight for Null {
    fn max_brightness(&self, _: Screen) -> u8 {
        0
    }
    fn brightness(&self, _: Screen) -> Result<u8> {
        self.err()
    }
    fn set_brightness(&mut self, _: Screen, _: u8) -> Result<()> {
        self.err()
    }
    fn max_contrast(&self, _: Screen) -> u8 {
        0
    }
    fn contrast(&self, _: Screen) -> Result<u8> {
        self.err()
    }
    fn set_contrast(&mut self, _: Screen, _: u8) -> Result<()> {
        self.err()
    }
}

impl Input for Null {
    fn next_event(&mut self, timeout: Option<Duration>) -> Result<Option<InputEvent>> {
        // Behave like a keypad nobody touches, so input loops do not spin.
        std::thread::sleep(timeout.unwrap_or(Duration::from_secs(3600)));
        Ok(None)
    }
    fn off_hook(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn poll_fds(&self) -> Vec<RawFd> {
        Vec::new()
    }
}

impl Lamps for Null {
    fn lamps(&self) -> Vec<Lamp> {
        Vec::new()
    }
    fn set(&mut self, _: Lamp, _: LampColor) -> Result<()> {
        Ok(())
    }
}

impl Audio for Null {
    fn rate(&self) -> u32 {
        0
    }
    fn open(&mut self, _: PcmDevice, _: PcmConfig) -> Result<Box<dyn Pcm>> {
        self.err()
    }
    fn mixer(&mut self) -> Result<Box<dyn Mixer>> {
        self.err()
    }
}

impl Mtd for Null {
    fn partitions(&self) -> Result<Vec<MtdInfo>> {
        Ok(Vec::new())
    }
    fn open(&mut self, _: &str) -> Result<Box<dyn MtdPartition>> {
        self.err()
    }
}

impl Watchdog for Null {
    fn start(&mut self, _: u32) -> Result<u32> {
        self.err()
    }
    fn feed(&mut self) -> Result<()> {
        self.err()
    }
    fn stop(&mut self) -> Result<()> {
        self.err()
    }
    fn time_left(&mut self) -> Result<u32> {
        self.err()
    }
}

impl Rng for Null {
    fn fill(&mut self, _: &mut [u8]) -> Result<()> {
        self.err()
    }
}

impl Ehs for Null {
    fn open(&mut self, _: SerialConfig) -> Result<Box<dyn EhsPort>> {
        self.err()
    }
}
