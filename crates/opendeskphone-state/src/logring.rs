//! The in-memory log ring, readable from the web UI and SSH, with a
//! `log`-crate backend.

use crate::clock;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Mutex;

/// Severity, matching the `log` crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl From<log::Level> for Level {
    fn from(l: log::Level) -> Self {
        match l {
            log::Level::Error => Level::Error,
            log::Level::Warn => Level::Warn,
            log::Level::Info => Level::Info,
            log::Level::Debug => Level::Debug,
            log::Level::Trace => Level::Trace,
        }
    }
}

impl Level {
    pub const fn as_str(self) -> &'static str {
        match self {
            Level::Error => "ERROR",
            Level::Warn => "WARN",
            Level::Info => "INFO",
            Level::Debug => "DEBUG",
            Level::Trace => "TRACE",
        }
    }
    /// Syslog severity number.
    pub const fn syslog_severity(self) -> u8 {
        match self {
            Level::Error => 3,
            Level::Warn => 4,
            Level::Info => 6,
            Level::Debug | Level::Trace => 7,
        }
    }
}

/// One log line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    /// Increases by one per entry; never reused.
    pub seq: u64,
    pub mono_ms: u64,
    pub unix_ms: u64,
    pub level: Level,
    /// Module path of the caller (`opendeskphone_sip::register`).
    pub target: String,
    pub message: String,
}

impl LogEntry {
    /// `[  12.345] INFO  target: message`
    pub fn format(&self) -> String {
        format!(
            "[{:5}.{:03}] {:5} {}: {}",
            self.mono_ms / 1000,
            self.mono_ms % 1000,
            self.level.as_str(),
            self.target,
            self.message
        )
    }
}

/// A bounded ring of log entries.
pub struct LogRing {
    inner: Mutex<Ring>,
}

struct Ring {
    entries: VecDeque<LogEntry>,
    next_seq: u64,
    capacity: usize,
    bytes: usize,
    max_bytes: usize,
}

impl LogRing {
    /// Keeps at most `capacity` entries and about `max_bytes` of text.
    pub fn new(capacity: usize, max_bytes: usize) -> Self {
        LogRing {
            inner: Mutex::new(Ring {
                entries: VecDeque::new(),
                next_seq: 1,
                capacity: capacity.max(1),
                bytes: 0,
                max_bytes: max_bytes.max(1024),
            }),
        }
    }

    /// Appends a line; returns its sequence number.
    pub fn push(&self, level: Level, target: &str, message: String) -> u64 {
        let mut g = self.inner.lock().unwrap();
        let seq = g.next_seq;
        g.next_seq += 1;
        g.bytes += message.len() + target.len();
        g.entries.push_back(LogEntry {
            seq,
            mono_ms: clock::mono_ms(),
            unix_ms: clock::unix_ms(),
            level,
            target: target.to_string(),
            message,
        });
        while g.entries.len() > g.capacity || (g.bytes > g.max_bytes && g.entries.len() > 1) {
            if let Some(old) = g.entries.pop_front() {
                g.bytes -= old.message.len() + old.target.len();
            }
        }
        seq
    }

    /// Entries with `seq > after`, oldest first (pass 0 for everything
    /// held).  A follower keeps the last `seq` it saw and calls again when
    /// `Topic::Log` fires.
    pub fn since(&self, after: u64) -> Vec<LogEntry> {
        let g = self.inner.lock().unwrap();
        let start = g.entries.partition_point(|e| e.seq <= after);
        g.entries.iter().skip(start).cloned().collect()
    }

    /// The newest `n` entries, oldest first.
    pub fn tail(&self, n: usize) -> Vec<LogEntry> {
        let g = self.inner.lock().unwrap();
        let skip = g.entries.len().saturating_sub(n);
        g.entries.iter().skip(skip).cloned().collect()
    }

    /// Sequence number of the newest entry (0 when empty).
    pub fn last_seq(&self) -> u64 {
        self.inner.lock().unwrap().next_seq - 1
    }
}
