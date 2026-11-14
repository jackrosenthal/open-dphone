//! Logging.  Everything logs through the `log` facade.
//!
//! The main role keeps recent entries in an in-memory ring that `dphone log`
//! reads, and copies them to stderr.  The init role writes to the kernel log
//! instead, so its messages survive in `dmesg` and on the serial console.
//!
//! The ring here is the stand-in until the state crate's log ring is wired;
//! the interface (`recent`, `subscribe`) is what the diag command uses.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Mutex, OnceLock};

use log::{Level, LevelFilter, Log, Metadata, Record};

#[derive(Clone, Debug)]
pub struct Entry {
    pub seq: u64,
    /// Milliseconds since the Unix epoch.
    pub time_ms: u64,
    pub level: Level,
    pub target: String,
    pub message: String,
}

impl Entry {
    pub fn format(&self) -> String {
        let secs = (self.time_ms / 1000) as i64;
        let rem = secs.rem_euclid(86400);
        format!(
            "{:02}:{:02}:{:02}.{:03} {:<5} {}: {}",
            rem / 3600,
            rem % 3600 / 60,
            rem % 60,
            self.time_ms % 1000,
            self.level,
            self.target,
            self.message
        )
    }
}

const RING_ENTRIES: usize = 2000;
const RING_BYTES: usize = 256 * 1024;

struct Ring {
    entries: VecDeque<Entry>,
    bytes: usize,
    next_seq: u64,
}

pub struct RingLogger {
    ring: Mutex<Ring>,
    seq_tx: tokio::sync::watch::Sender<u64>,
    stderr: bool,
}

static LOGGER: OnceLock<RingLogger> = OnceLock::new();

/// A second destination for every accepted record (the state store's log
/// ring, which the web UI streams).
pub type Sink = Box<dyn Fn(Level, &str, &str) + Send + Sync>;
static SINK: OnceLock<Sink> = OnceLock::new();

/// Forward every record to `sink` as well.  Only the first call takes effect.
pub fn set_sink(sink: Sink) {
    let _ = SINK.set(sink);
}

/// Crates whose debug output is not ours to read by default.
fn quiet_target(target: &str) -> bool {
    target.starts_with("russh") || target.starts_with("mio") || target.starts_with("tokio")
}

impl Log for RingLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        if quiet_target(metadata.target()) {
            metadata.level() <= Level::Warn
        } else {
            metadata.level() <= log::max_level()
        }
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let time_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let message = record.args().to_string();
        let for_sink = SINK.get().map(|sink| (sink, message.clone()));
        let mut entry =
            Entry { seq: 0, time_ms, level: record.level(), target: record.target().to_string(), message };
        let seq = {
            let Ok(mut ring) = self.ring.lock() else { return };
            entry.seq = ring.next_seq;
            ring.next_seq += 1;
            ring.bytes += entry.message.len() + entry.target.len();
            if self.stderr {
                let _ = writeln!(std::io::stderr(), "{}", entry.format());
            }
            ring.entries.push_back(entry);
            while ring.entries.len() > RING_ENTRIES || ring.bytes > RING_BYTES {
                match ring.entries.pop_front() {
                    Some(old) => ring.bytes -= old.message.len() + old.target.len(),
                    None => break,
                }
            }
            ring.next_seq
        };
        let _ = self.seq_tx.send(seq);
        if let Some((sink, message)) = for_sink {
            sink(record.level(), record.target(), &message);
        }
    }

    fn flush(&self) {}
}

/// Install the ring logger (main role and tools).  Safe to call more than once.
pub fn init_ring(level: LevelFilter, stderr: bool) {
    let logger = LOGGER.get_or_init(|| RingLogger {
        ring: Mutex::new(Ring { entries: VecDeque::new(), bytes: 0, next_seq: 1 }),
        seq_tx: tokio::sync::watch::channel(1).0,
        stderr,
    });
    if log::set_logger(logger).is_ok() {
        log::set_max_level(level);
    }
}

/// Entries with `seq >= from_seq`, oldest first, at most the last `limit`.
pub fn recent(from_seq: u64, limit: usize) -> Vec<Entry> {
    let Some(logger) = LOGGER.get() else { return Vec::new() };
    let Ok(ring) = logger.ring.lock() else { return Vec::new() };
    let matching: Vec<&Entry> = ring.entries.iter().filter(|e| e.seq >= from_seq).collect();
    let skip = matching.len().saturating_sub(limit);
    matching.into_iter().skip(skip).cloned().collect()
}

/// A watch on the next sequence number; changes whenever an entry is added.
pub fn subscribe() -> Option<tokio::sync::watch::Receiver<u64>> {
    LOGGER.get().map(|l| l.seq_tx.subscribe())
}

/// Logger for the init role: one line per record to `/dev/kmsg`, falling back
/// to stderr (the console) when the kernel log is not available.
pub struct KmsgLogger {
    kmsg: Mutex<Option<std::fs::File>>,
    tag: &'static str,
}

static KMSG: OnceLock<KmsgLogger> = OnceLock::new();

impl Log for KmsgLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let prio = match record.level() {
            Level::Error => 3,
            Level::Warn => 4,
            Level::Info => 6,
            Level::Debug | Level::Trace => 7,
        };
        let line = format!("<{prio}>{}: {}\n", self.tag, record.args());
        let Ok(mut kmsg) = self.kmsg.lock() else { return };
        if kmsg.is_none() {
            *kmsg = std::fs::OpenOptions::new().write(true).open("/dev/kmsg").ok();
        }
        let written = match kmsg.as_mut() {
            Some(f) => f.write_all(line.as_bytes()).is_ok(),
            None => false,
        };
        if !written {
            let _ = writeln!(std::io::stderr(), "{}: {}", self.tag, record.args());
        }
    }

    fn flush(&self) {}
}

/// Install the kernel-log logger (init role).
pub fn init_kmsg(tag: &'static str, level: LevelFilter) {
    let logger = KMSG.get_or_init(|| KmsgLogger { kmsg: Mutex::new(None), tag });
    if log::set_logger(logger).is_ok() {
        log::set_max_level(level);
    }
}
