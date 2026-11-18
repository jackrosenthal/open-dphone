//! Time helpers.  The phones have no RTC, so state records both a monotonic
//! stamp (always valid) and, where useful, wall time (valid once restored
//! or set by SNTP).

use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

static EPOCH: OnceLock<Instant> = OnceLock::new();

/// Milliseconds since the first call in this process (monotonic).
pub fn mono_ms() -> u64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Wall-clock milliseconds since the Unix epoch (0 if the clock is before it).
pub fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Wall-clock seconds since the Unix epoch.
pub fn unix_secs() -> u64 {
    unix_ms() / 1000
}
