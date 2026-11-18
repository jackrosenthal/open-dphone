//! Watchdog and random numbers.

use crate::Result;

/// The hardware watchdog.  boot2 hands off with it disabled; PID 1 arms it.
pub trait Watchdog: Send {
    /// Arms the watchdog with a timeout in seconds; returns the timeout the
    /// hardware actually uses.
    fn start(&mut self, timeout_secs: u32) -> Result<u32>;
    /// Feeds it.
    fn feed(&mut self) -> Result<()>;
    /// Disarms it (magic close), e.g., before a long flash erase in a
    /// context that cannot feed.  May be refused by the kernel (`nowayout`).
    fn stop(&mut self) -> Result<()>;
    /// Seconds until expiry if the driver reports it.
    fn time_left(&mut self) -> Result<u32>;
}

/// Hardware random numbers.
pub trait Rng: Send {
    /// Fills `buf` with random bytes.
    fn fill(&mut self, buf: &mut [u8]) -> Result<()>;
}
