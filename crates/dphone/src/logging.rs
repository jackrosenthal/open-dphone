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
            send_remote(record.level(), time_ms, record.target(), &message);
            sink(record.level(), record.target(), &message);
        } else {
            send_remote(record.level(), time_ms, record.target(), &record.args().to_string());
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

// ---------------------------------------------------------- remote syslog

/// Where log records are also sent, as syslog over UDP.
struct Remote {
    socket: std::net::UdpSocket,
    /// Records less severe than this are not sent.
    level: Level,
}

static REMOTE: Mutex<Option<Remote>> = Mutex::new(None);

/// The level a stock `log_level` value lets through: `error`, `warning`,
/// `information`, or `debug`; anything else is `error`, as in stock.
pub fn level_from_setting(value: &str) -> Level {
    match value.trim().to_ascii_lowercase().as_str() {
        "warning" => Level::Warn,
        "information" => Level::Info,
        "debug" => Level::Debug,
        _ => Level::Error,
    }
}

/// Send records at `level` and above to a syslog server, or stop
/// (`None`).  Sending never blocks and never fails loudly: a datagram that
/// cannot be sent is dropped.
pub fn set_remote(server: Option<std::net::SocketAddr>, level: Level) -> std::io::Result<()> {
    let remote = match server {
        Some(server) => {
            let local: std::net::SocketAddr = if server.is_ipv4() {
                (std::net::Ipv4Addr::UNSPECIFIED, 0).into()
            } else {
                (std::net::Ipv6Addr::UNSPECIFIED, 0).into()
            };
            let socket = std::net::UdpSocket::bind(local)?;
            socket.connect(server)?;
            socket.set_nonblocking(true)?;
            Some(Remote { socket, level })
        }
        None => None,
    };
    if let Ok(mut slot) = REMOTE.lock() {
        *slot = remote;
    }
    Ok(())
}

/// One record as a syslog datagram in the BSD format (RFC 3164) that the
/// stock firmware's `syslogd -R` forwarded: priority, time, tag, message.
/// The facility is "user"; the time is UTC.
pub fn syslog_line(level: Level, time_secs: i64, pid: u32, target: &str, message: &str) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let severity = match level {
        Level::Error => 3,
        Level::Warn => 4,
        Level::Info => 6,
        Level::Debug | Level::Trace => 7,
    };
    let (_, month, day) = crate::clock::civil_from_days(time_secs.div_euclid(86400));
    let rem = time_secs.rem_euclid(86400);
    // One datagram, one line: line breaks in the message become spaces.
    let message = message.replace(['\n', '\r'], " ");
    let mut line = format!(
        "<{}>{} {:2} {:02}:{:02}:{:02} dphone[{pid}]: {target}: {message}",
        8 + severity,
        MONTHS[(month - 1) as usize],
        day,
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
    );
    // RFC 3164 limits a packet to 1024 bytes.
    if line.len() > 1024 {
        let mut end = 1024;
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        line.truncate(end);
    }
    line
}

fn send_remote(level: Level, time_ms: u64, target: &str, message: &str) {
    // Never wait for the lock: a record logged while the destination is
    // being changed is simply not sent.
    let Ok(slot) = REMOTE.try_lock() else { return };
    let Some(remote) = slot.as_ref() else { return };
    if level > remote.level {
        return;
    }
    let line = syslog_line(level, (time_ms / 1000) as i64, std::process::id(), target, message);
    let _ = remote.socket.send(line.as_bytes());
}

#[cfg(test)]
mod syslog_tests {
    use super::*;

    #[test]
    fn line_format() {
        // 2026-11-05 03:04:05 UTC.
        let t = crate::clock::days_from_civil(2026, 11, 5) * 86400 + 3 * 3600 + 4 * 60 + 5;
        assert_eq!(
            syslog_line(Level::Info, t, 42, "dphone::net", "link up\non lan"),
            "<14>Nov  5 03:04:05 dphone[42]: dphone::net: link up on lan"
        );
        assert!(syslog_line(Level::Error, t, 1, "x", "boom").starts_with("<11>Nov  5 "));
        assert!(syslog_line(Level::Warn, t, 1, "x", "hm").starts_with("<12>"));
        assert!(syslog_line(Level::Debug, t, 1, "x", "dbg").starts_with("<15>"));
        // Cut at 1024 bytes without splitting a character.
        assert!((1023..=1024).contains(&syslog_line(Level::Info, t, 1, "x", &"é".repeat(2000)).len()));
    }

    #[test]
    fn levels() {
        assert_eq!(level_from_setting("error"), Level::Error);
        assert_eq!(level_from_setting("Warning"), Level::Warn);
        assert_eq!(level_from_setting("information"), Level::Info);
        assert_eq!(level_from_setting("debug"), Level::Debug);
        assert_eq!(level_from_setting("nonsense"), Level::Error);
    }

    #[test]
    fn records_reach_the_server_and_respect_the_level() {
        let server = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        server.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
        set_remote(Some(server.local_addr().unwrap()), Level::Warn).unwrap();
        send_remote(Level::Info, 0, "t", "not sent");
        send_remote(Level::Warn, 0, "t", "sent");
        let mut buf = [0u8; 256];
        let n = server.recv(&mut buf).unwrap();
        assert_eq!(std::str::from_utf8(&buf[..n]).unwrap(), format!("<12>Jan  1 00:00:00 dphone[{}]: t: sent", std::process::id()));
        set_remote(None, Level::Warn).unwrap();
        send_remote(Level::Error, 0, "t", "after stop");
        server.set_read_timeout(Some(std::time::Duration::from_millis(100))).unwrap();
        assert!(server.recv(&mut buf).is_err());
    }
}
