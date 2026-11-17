//! `GET /metrics`: a Prometheus scrape endpoint.
//!
//! The endpoint is served on both listeners **without authentication**, so
//! that a stock Prometheus can scrape the phone with no credentials.  It is
//! on by default and can be switched off (`metrics_enabled` in the Open
//! Desk Phone settings, or `dphone_metrics_enabled` from provisioning; it then
//! answers 404), independently of whether web configuration is enabled.  For
//! that reason it carries nothing that identifies a person or a peer: no
//! names, numbers, SIP identities, host names, addresses, key or token
//! material, or free-form text.  Labels are the firmware version, the model,
//! and small fixed vocabularies (subsystem, port, account index, state
//! names, flash partition names).
//!
//! The body is OpenMetrics text, written by the `prometheus-client` crate.
//! A scraper that asks for `application/openmetrics-text` gets that media
//! type; anything else (a browser, `curl`) gets the same body as
//! `text/plain; version=0.0.4`, which the classic parser also accepts.
//!
//! Cost is bounded for an open endpoint: every value is read when a scrape
//! arrives (a state snapshot, a handful of `/proc` and `/sys` files, one
//! `statvfs`), the set of series is fixed (a few hundred lines, under 20 KiB at
//! most), and the rendered body is reused for [`MIN_INTERVAL`], so however
//! many requests arrive, the phone renders at most once a second.  The
//! only state kept between scrapes is the call counters, fed by
//! `CallEnded` events on the web server's own task.
//!
//! # Metrics
//!
//! | Name | Type | Labels |
//! | --- | --- | --- |
//! | `dphone_build_info` | gauge (1) | `version`, `model` |
//! | `dphone_system_uptime_seconds` | gauge | |
//! | `dphone_system_phase` | gauge (0/1) | `phase` |
//! | `dphone_time_synced` | gauge (0/1) | |
//! | `dphone_healthy` | gauge (0/1) | |
//! | `dphone_subsystem_health` | gauge (0/1) | `subsystem`, `status` |
//! | `dphone_subsystem_heartbeat_age_seconds` | gauge | `subsystem` |
//! | `dphone_network_phase` | gauge (0/1) | `phase` |
//! | `dphone_network_link_up` | gauge (0/1) | `port` |
//! | `dphone_network_link_speed_bytes` | gauge (bytes per second) | `port` |
//! | `dphone_network_link_full_duplex` | gauge (0/1) | `port` |
//! | `dphone_network_address_present` | gauge (0/1) | `family` |
//! | `dphone_network_dhcp_lease_remaining_seconds` | gauge | |
//! | `dphone_ntp_last_sync_age_seconds` | gauge | |
//! | `dphone_accounts` | gauge | |
//! | `dphone_account_registration` | gauge (0/1) | `account`, `state` |
//! | `dphone_account_registration_expires_seconds` | gauge | `account` |
//! | `dphone_calls_active` | gauge | `state` |
//! | `dphone_calls_total` | counter | `direction`, `cause` |
//! | `dphone_rtp_received_packets_total` | counter | |
//! | `dphone_rtp_sent_packets_total` | counter | |
//! | `dphone_rtp_lost_packets_total` | counter | |
//! | `dphone_rtp_jitter_seconds` | gauge (worst live call) | |
//! | `dphone_events_lost_total` | counter | |
//! | `dphone_dsp_status` | gauge (0/1) | `status` |
//! | `dphone_provisioning_phase` | gauge (0/1) | `phase` |
//! | `dphone_provisioning_attached` | gauge (0/1) | |
//! | `dphone_provisioning_last_success_timestamp_seconds` | gauge | |
//! | `dphone_config_generation` | gauge | |
//! | `dphone_config_restart_needed` | gauge (0/1) | |
//! | `dphone_config_error` | gauge (0/1) | |
//! | `dphone_firmware_running_bank` | gauge (0/1) | `bank` |
//! | `dphone_firmware_update_phase` | gauge (0/1) | `phase` |
//! | `dphone_firmware_trial` | gauge (0/1) | `state` |
//! | `dphone_apps_installed`, `dphone_apps_running` | gauge | |
//! | `dphone_ssh_listening` | gauge (0/1) | |
//! | `dphone_web_sessions`, `dphone_web_shells` | gauge | |
//! | `dphone_log_lines_total` | counter | |
//! | `dphone_memory_total_bytes`, `_available_bytes`, `_free_bytes` | gauge | |
//! | `dphone_load_average` | gauge | `window` |
//! | `dphone_nvdata_size_bytes`, `dphone_nvdata_available_bytes` | gauge | |
//! | `dphone_mtd_ecc_corrected_total`, `dphone_mtd_ecc_failures_total` | counter | `partition` |
//! | `dphone_mtd_bad_blocks` | gauge | `partition` |
//! | `process_resident_memory_bytes`, `process_virtual_memory_bytes` | gauge | |
//! | `process_cpu_seconds_total` | counter | |
//! | `process_threads`, `process_open_fds` | gauge | |
//! | `process_start_time_seconds` | gauge | |
//!
//! The `process_*` names are the ones every Prometheus client library uses,
//! so stock dashboards work.  Call and RTP counters start at zero when the
//! main role starts.  RTP counters grow when a call ends, by that call's
//! totals.

use crate::rpc::Body;
use crate::App;
use bytes::Bytes;
use dphone_state::{self as st, Subscription, Subsystem, TopicSet};
use http::{header, HeaderValue, Request, Response};
use prometheus_client::collector::Collector;
use prometheus_client::encoding::{DescriptorEncoder, EncodeMetric};
use prometheus_client::metrics::counter::ConstCounter;
use prometheus_client::metrics::gauge::ConstGauge;
use prometheus_client::metrics::MetricType;
use prometheus_client::registry::Registry;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A rendered body is reused for this long, which caps the work that
/// requests from the network can cause.
pub(crate) const MIN_INTERVAL: Duration = Duration::from_secs(1);

const OPENMETRICS: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";
const TEXT: &str = "text/plain; version=0.0.4; charset=utf-8";

const DIRECTIONS: [&str; 2] = ["incoming", "outgoing"];
const CAUSES: [&str; 10] = [
    "local_hangup",
    "remote_hangup",
    "rejected",
    "canceled",
    "answered_elsewhere",
    "busy",
    "no_answer",
    "transferred",
    "failed",
    "error",
];

fn cause_index(c: &st::EndCause) -> usize {
    match c {
        st::EndCause::LocalHangup => 0,
        st::EndCause::RemoteHangup => 1,
        st::EndCause::Rejected => 2,
        st::EndCause::Canceled { answered_elsewhere: false } => 3,
        st::EndCause::Canceled { answered_elsewhere: true } => 4,
        st::EndCause::Busy => 5,
        st::EndCause::NoAnswer => 6,
        st::EndCause::Transferred => 7,
        st::EndCause::Failed { .. } => 8,
        st::EndCause::Error(_) => 9,
    }
}

/// What the endpoint keeps between scrapes.
pub(crate) struct Metrics {
    /// Delivers `CallEnded` events; taken from the bind onward, so no call
    /// goes uncounted.
    events: Subscription,
    calls: [[AtomicU64; CAUSES.len()]; DIRECTIONS.len()],
    rtp_rx: AtomicU64,
    rtp_tx: AtomicU64,
    rtp_lost: AtomicU64,
    events_lost: AtomicU64,
    cache: Mutex<Option<(Instant, Bytes)>>,
    nvdata: PathBuf,
}

impl Metrics {
    pub fn new(store: &st::Store, nvdata: PathBuf) -> Self {
        Metrics {
            events: store.subscribe_events(TopicSet::NONE),
            calls: Default::default(),
            rtp_rx: AtomicU64::new(0),
            rtp_tx: AtomicU64::new(0),
            rtp_lost: AtomicU64::new(0),
            events_lost: AtomicU64::new(0),
            cache: Mutex::new(None),
            nvdata,
        }
    }

    fn absorb(&self, wake: st::Wake) {
        self.events_lost.fetch_add(wake.lost_events, Ordering::Relaxed);
        for event in wake.events {
            if let st::Event::CallEnded { call } = event {
                let dir = match call.direction {
                    st::Direction::Incoming => 0,
                    st::Direction::Outgoing => 1,
                };
                let cause = match &call.state {
                    st::CallState::Ended(c) => cause_index(c),
                    _ => 9,
                };
                self.calls[dir][cause].fetch_add(1, Ordering::Relaxed);
                self.rtp_rx.fetch_add(call.stats.rx_packets.into(), Ordering::Relaxed);
                self.rtp_tx.fetch_add(call.stats.tx_packets.into(), Ordering::Relaxed);
                self.rtp_lost.fetch_add(call.stats.rx_lost.into(), Ordering::Relaxed);
            }
        }
    }

    /// Counts calls as they end.  Runs for as long as the server does, on
    /// the server's own task.
    pub async fn run(&self) {
        loop {
            let wake = self.events.changed().await;
            self.absorb(wake);
        }
    }
}

/// Answers `GET /metrics`.
pub(crate) fn respond<B>(app: &App, req: &Request<B>) -> Response<Body> {
    let body = {
        let mut cache = app.metrics.cache.lock().unwrap_or_else(|p| p.into_inner());
        match &*cache {
            Some((at, body)) if at.elapsed() < MIN_INTERVAL => body.clone(),
            _ => {
                let body = Bytes::from(render(app));
                *cache = Some((Instant::now(), body.clone()));
                body
            }
        }
    };
    let openmetrics = req
        .headers()
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("application/openmetrics-text"));
    let mut resp = Response::new(Body::Empty);
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(if openmetrics { OPENMETRICS } else { TEXT }));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(body.len()));
    if req.method() != http::Method::HEAD {
        *resp.body_mut() = Body::from(body);
    }
    resp
}

/// Reads everything and writes the exposition text.
pub(crate) fn render(app: &App) -> String {
    // Anything still queued counts in this scrape.
    if let Some(wake) = app.metrics.events.try_take() {
        app.metrics.absorb(wake);
    }
    let m = &app.metrics;
    let scrape = Scrape {
        state: app.store.snapshot(),
        heartbeats: Subsystem::ALL.map(|s| (s, app.store.last_heartbeat(s))),
        now_mono_ms: st::clock::mono_ms(),
        calls: std::array::from_fn(|d| std::array::from_fn(|c| m.calls[d][c].load(Ordering::Relaxed))),
        rtp_rx: m.rtp_rx.load(Ordering::Relaxed),
        rtp_tx: m.rtp_tx.load(Ordering::Relaxed),
        rtp_lost: m.rtp_lost.load(Ordering::Relaxed),
        events_lost: m.events_lost.load(Ordering::Relaxed),
        log_lines: app.store.log().last_seq(),
        ssh_listening: app.ssh.as_ref().and_then(|s| s.listening()),
        web_sessions: app.sessions.count() as u64,
        web_shells: app.shells.count() as u64,
        os: Os::read(&m.nvdata),
    };
    let mut registry = Registry::default();
    registry.register_collector(Box::new(scrape));
    let mut out = String::with_capacity(16 << 10);
    if let Err(e) = prometheus_client::encoding::text::encode(&mut out, &registry) {
        log::warn!("metrics: {e}");
    }
    out
}

// ------------------------------------------------------ operating system

/// Figures from `/proc`, `/sys`, and `statvfs`.  All of these are answered
/// from kernel memory; none waits on flash or the network.
#[derive(Default)]
struct Os {
    uptime: Option<f64>,
    load: Option<[f64; 3]>,
    mem_total: Option<u64>,
    mem_available: Option<u64>,
    mem_free: Option<u64>,
    rss: Option<u64>,
    vsize: Option<u64>,
    cpu: Option<f64>,
    threads: Option<u64>,
    start_time: Option<f64>,
    fds: Option<u64>,
    nvdata: Option<(u64, u64)>,
    mtd: Vec<Mtd>,
}

struct Mtd {
    name: String,
    corrected: Option<u64>,
    failures: Option<u64>,
    bad_blocks: Option<u64>,
}

fn read(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn number<T: std::str::FromStr>(path: &Path) -> Option<T> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Parses the kernel's `123.45` figures without the general float parser,
/// which is large and otherwise unused in the firmware.
fn hundredths(text: &str) -> Option<f64> {
    let (whole, frac) = text.split_once('.').unwrap_or((text, "0"));
    let whole: u64 = whole.parse().ok()?;
    let mut digits = frac.bytes().take(2).map(|b| b.is_ascii_digit().then(|| u64::from(b - b'0')));
    let tenths = digits.next().unwrap_or(Some(0))?;
    let hundredths = digits.next().unwrap_or(Some(0))?;
    Some((whole * 100 + tenths * 10 + hundredths) as f64 / 100.0)
}

fn meminfo(text: &str, key: &str) -> Option<u64> {
    let rest = text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))?;
    rest.split_whitespace().next()?.parse::<u64>().ok().map(|kib| kib * 1024)
}

/// A flash partition name as a label value: the names come from the boot
/// loader's `mtdparts=`, so they are kept to a plain, short vocabulary.
fn partition_label(name: &str) -> String {
    name.trim().chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')).take(24).collect()
}

impl Os {
    fn read(nvdata: &Path) -> Os {
        let mut os = Os::default();
        // SAFETY: `sysconf` only reads its argument.
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        let hz = if hz > 0 { hz as f64 } else { 100.0 };
        // SAFETY: as above.
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        let page = if page > 0 { page as u64 } else { 4096 };

        os.uptime = read("/proc/uptime").and_then(|t| hundredths(t.split_whitespace().next()?));
        os.load = read("/proc/loadavg").and_then(|t| {
            let mut it = t.split_whitespace().map(hundredths);
            Some([it.next()??, it.next()??, it.next()??])
        });
        if let Some(text) = read("/proc/meminfo") {
            os.mem_total = meminfo(&text, "MemTotal");
            os.mem_available = meminfo(&text, "MemAvailable");
            os.mem_free = meminfo(&text, "MemFree");
        }
        // The command name (field 2) may hold spaces; the rest follows the
        // last ')'.  Fields below are numbered as in proc(5).
        if let Some(stat) = read("/proc/self/stat")
            && let Some((_, rest)) = stat.rsplit_once(')')
        {
            let f: Vec<&str> = rest.split_whitespace().collect();
            let field = |n: usize| f.get(n - 3).and_then(|v| v.parse::<u64>().ok());
            if let (Some(utime), Some(stime)) = (field(14), field(15)) {
                os.cpu = Some((utime + stime) as f64 / hz);
            }
            os.threads = field(20);
            os.vsize = field(23);
            os.rss = field(24).map(|pages| pages * page);
            if let (Some(start), Some(uptime)) = (field(22), os.uptime) {
                let age = uptime - start as f64 / hz;
                let now = st::clock::unix_ms() as f64 / 1000.0;
                os.start_time = Some(now - age);
            }
        }
        os.fds = std::fs::read_dir("/proc/self/fd").ok().map(|d| d.count().saturating_sub(1) as u64);

        if let Ok(path) = std::ffi::CString::new(nvdata.as_os_str().as_encoded_bytes()) {
            // SAFETY: `path` is a NUL-terminated string and `vfs` is a
            // properly sized, writable `statvfs` structure.
            let mut vfs: libc::statvfs = unsafe { std::mem::zeroed() };
            if unsafe { libc::statvfs(path.as_ptr(), &mut vfs) } == 0 {
                let unit = vfs.f_frsize as u64;
                os.nvdata = Some((vfs.f_blocks as u64 * unit, vfs.f_bavail as u64 * unit));
            }
        }

        // The phones have nine partitions; the cap bounds a strange host.
        for n in 0..16 {
            let dir = PathBuf::from(format!("/sys/class/mtd/mtd{n}"));
            let Some(name) = read(&format!("/sys/class/mtd/mtd{n}/name")) else { break };
            os.mtd.push(Mtd {
                name: partition_label(&name),
                corrected: number(&dir.join("corrected_bits")),
                failures: number(&dir.join("ecc_failures")),
                bad_blocks: number(&dir.join("bad_blocks")),
            });
        }
        os
    }
}

// --------------------------------------------------------------- encoding

/// Everything one scrape reports, gathered before any text is written.
struct Scrape {
    state: st::State,
    heartbeats: [(Subsystem, Option<u64>); 11],
    now_mono_ms: u64,
    calls: [[u64; CAUSES.len()]; DIRECTIONS.len()],
    rtp_rx: u64,
    rtp_tx: u64,
    rtp_lost: u64,
    events_lost: u64,
    log_lines: u64,
    ssh_listening: Option<bool>,
    web_sessions: u64,
    web_shells: u64,
    os: Os,
}

// `Collector` asks for `Debug`.  Deriving it would compile formatting code
// for the whole state tree into the firmware for no use.
impl fmt::Debug for Scrape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Scrape")
    }
}

/// Lower-case label values for the subsystems.
fn subsystem_label(s: Subsystem) -> &'static str {
    match s {
        Subsystem::Core => "core",
        Subsystem::Config => "config",
        Subsystem::Net => "net",
        Subsystem::Sip => "sip",
        Subsystem::Media => "media",
        Subsystem::Dpma => "dpma",
        Subsystem::Ui => "ui",
        Subsystem::Apps => "apps",
        Subsystem::Web => "web",
        Subsystem::Ssh => "ssh",
        Subsystem::Update => "update",
    }
}

type Labels<'a> = &'a [(&'static str, &'a str)];

/// Writes one metric family.  A family without labels has one sample,
/// which uses up the encoder.
struct Family<'a> {
    enc: Option<prometheus_client::encoding::MetricEncoder<'a>>,
}

impl Family<'_> {
    fn put(&mut self, labels: Labels<'_>, metric: &impl EncodeMetric) -> fmt::Result {
        if labels.is_empty() {
            metric.encode(self.enc.take().ok_or(fmt::Error)?)
        } else {
            metric.encode(self.enc.as_mut().ok_or(fmt::Error)?.encode_family(&labels)?)
        }
    }
    fn int(&mut self, labels: Labels<'_>, value: u64) -> fmt::Result {
        self.put(labels, &ConstGauge::new(i64::try_from(value).unwrap_or(i64::MAX)))
    }
    fn float(&mut self, labels: Labels<'_>, value: f64) -> fmt::Result {
        self.put(labels, &ConstGauge::new(value))
    }
    fn flag(&mut self, labels: Labels<'_>, value: bool) -> fmt::Result {
        self.int(labels, value.into())
    }
    fn count(&mut self, labels: Labels<'_>, value: u64) -> fmt::Result {
        self.put(labels, &ConstCounter::new(value))
    }
    fn count_float(&mut self, value: f64) -> fmt::Result {
        self.put(&[], &ConstCounter::new(value))
    }
    /// One 0/1 series per member of a fixed vocabulary.
    fn one_of(&mut self, fixed: Labels<'_>, key: &'static str, all: &[&'static str], current: &str) -> fmt::Result {
        for name in all {
            let mut labels: Vec<(&'static str, &str)> = fixed.to_vec();
            labels.push((key, name));
            self.flag(&labels, *name == current)?;
        }
        Ok(())
    }
}

fn gauge<'a>(e: &'a mut DescriptorEncoder<'_>, name: &'static str, help: &'static str) -> Result<Family<'a>, fmt::Error> {
    Ok(Family { enc: Some(e.encode_descriptor(name, help, None, MetricType::Gauge)?) })
}

/// `name` is given without `_total`; the encoder appends it.
fn counter<'a>(e: &'a mut DescriptorEncoder<'_>, name: &'static str, help: &'static str) -> Result<Family<'a>, fmt::Error> {
    Ok(Family { enc: Some(e.encode_descriptor(name, help, None, MetricType::Counter)?) })
}

fn secs(ms: u64) -> f64 {
    ms as f64 / 1000.0
}

impl Collector for Scrape {
    fn encode(&self, mut e: DescriptorEncoder) -> fmt::Result {
        let s = &self.state;
        let e = &mut e;
        let now = self.now_mono_ms;

        // ------------------------------------------------------- system
        let version = s.system.version.dotted();
        gauge(e, "dphone_build_info", "Firmware version and phone model")?
            .int(&[("version", &version), ("model", s.system.model.name())], 1)?;
        if let Some(up) = self.os.uptime {
            gauge(e, "dphone_system_uptime_seconds", "Time since the kernel started")?.float(&[], up)?;
        }
        gauge(e, "dphone_system_phase", "Where the firmware is in its life")?.one_of(
            &[],
            "phase",
            &["starting", "network_wait", "provisioning", "running", "recovery", "shutting_down"],
            match s.system.phase {
                st::Phase::Starting => "starting",
                st::Phase::NetworkWait => "network_wait",
                st::Phase::Provisioning => "provisioning",
                st::Phase::Running => "running",
                st::Phase::Recovery => "recovery",
                st::Phase::ShuttingDown => "shutting_down",
            },
        )?;
        gauge(e, "dphone_time_synced", "The clock has been set from a time server")?.flag(&[], s.system.time_synced)?;

        // ------------------------------------------------------- health
        gauge(e, "dphone_healthy", "The firmware has declared itself healthy")?.flag(&[], s.health.healthy)?;
        {
            let mut f = gauge(e, "dphone_subsystem_health", "Declared status of each subsystem")?;
            for sub in Subsystem::ALL {
                let current = match s.health.status(sub) {
                    st::HealthStatus::Stopped => "stopped",
                    st::HealthStatus::Starting => "starting",
                    st::HealthStatus::Ok => "ok",
                    st::HealthStatus::Degraded(_) => "degraded",
                    st::HealthStatus::Failed(_) => "failed",
                };
                f.one_of(
                    &[("subsystem", subsystem_label(sub))],
                    "status",
                    &["stopped", "starting", "ok", "degraded", "failed"],
                    current,
                )?;
            }
        }
        {
            let mut f = gauge(e, "dphone_subsystem_heartbeat_age_seconds", "Time since each subsystem last reported in")?;
            for (sub, beat) in &self.heartbeats {
                if let Some(at) = beat {
                    f.float(&[("subsystem", subsystem_label(*sub))], secs(now.saturating_sub(*at)))?;
                }
            }
        }

        // ------------------------------------------------------ network
        let net = &s.network;
        gauge(e, "dphone_network_phase", "State of the network connection")?.one_of(
            &[],
            "phase",
            &["down", "configuring", "up", "failed"],
            match net.phase {
                st::NetPhase::Down => "down",
                st::NetPhase::Configuring => "configuring",
                st::NetPhase::Up => "up",
                st::NetPhase::Failed => "failed",
            },
        )?;
        let ports = [("lan", &net.lan), ("pc", &net.pc)];
        {
            let mut f = gauge(e, "dphone_network_link_up", "Ethernet link state of each jack")?;
            for (port, link) in ports {
                f.flag(&[("port", port)], link.up)?;
            }
        }
        {
            let mut f = gauge(e, "dphone_network_link_speed_bytes", "Negotiated link speed in bytes per second")?;
            for (port, link) in ports {
                f.int(&[("port", port)], u64::from(link.speed) * 125_000)?;
            }
        }
        {
            let mut f = gauge(e, "dphone_network_link_full_duplex", "The link is full duplex")?;
            for (port, link) in ports {
                f.flag(&[("port", port)], link.up && link.full_duplex)?;
            }
        }
        {
            let mut f = gauge(e, "dphone_network_address_present", "The phone has an address of this family")?;
            f.flag(&[("family", "ipv4")], net.ipv4.address.is_some())?;
            f.flag(&[("family", "ipv6")], !net.ipv6.is_empty())?;
        }
        if net.ipv4.source == st::AddrSource::Dhcp && net.ipv4.address.is_some() {
            let end = net.ipv4.lease_obtained_mono_ms + u64::from(net.ipv4.lease_secs) * 1000;
            gauge(e, "dphone_network_dhcp_lease_remaining_seconds", "Time left on the DHCP lease")?
                .float(&[], secs(end.saturating_sub(now)))?;
        }
        if let Some(at) = net.ntp_synced_mono_ms {
            gauge(e, "dphone_ntp_last_sync_age_seconds", "Time since the last answer from a time server")?
                .float(&[], secs(now.saturating_sub(at)))?;
        }

        // ----------------------------------------------------- accounts
        let accounts = &s.accounts.accounts;
        gauge(e, "dphone_accounts", "Configured SIP accounts")?.int(&[], accounts.len() as u64)?;
        {
            let mut f = gauge(e, "dphone_account_registration", "Registration state of each SIP account, by account index")?;
            for a in accounts {
                let index = a.id.0.to_string();
                f.one_of(
                    &[("account", &index)],
                    "state",
                    &["disabled", "unregistered", "registering", "registered", "failed"],
                    match a.registration {
                        st::Registration::Disabled => "disabled",
                        st::Registration::Unregistered => "unregistered",
                        st::Registration::Registering => "registering",
                        st::Registration::Registered { .. } => "registered",
                        st::Registration::Failed { .. } => "failed",
                    },
                )?;
            }
        }
        {
            let mut f =
                gauge(e, "dphone_account_registration_expires_seconds", "Time until each account's registration expires")?;
            for a in accounts {
                if let st::Registration::Registered { expires_mono_ms, .. } = a.registration {
                    f.float(&[("account", &a.id.0.to_string())], secs(expires_mono_ms.saturating_sub(now)))?;
                }
            }
        }

        // -------------------------------------------------------- calls
        {
            let mut by_state = [0u64; 4];
            let mut jitter = 0u16;
            for c in s.calls.live() {
                let i = match c.state {
                    st::CallState::Dialing | st::CallState::Proceeding | st::CallState::Ringback { .. } => 0,
                    st::CallState::Ringing => 1,
                    st::CallState::Held { .. } => 3,
                    _ => 2,
                };
                by_state[i] += 1;
                jitter = jitter.max(c.stats.jitter_ms);
            }
            let mut f = gauge(e, "dphone_calls_active", "Calls in progress")?;
            for (name, n) in ["setup", "ringing", "active", "held"].into_iter().zip(by_state) {
                f.int(&[("state", name)], n)?;
            }
            let mut f = counter(e, "dphone_calls", "Calls ended since the firmware started")?;
            for (d, direction) in DIRECTIONS.into_iter().enumerate() {
                for (c, cause) in CAUSES.into_iter().enumerate() {
                    f.count(&[("direction", direction), ("cause", cause)], self.calls[d][c])?;
                }
            }
            counter(e, "dphone_rtp_received_packets", "RTP packets received in ended calls")?.count(&[], self.rtp_rx)?;
            counter(e, "dphone_rtp_sent_packets", "RTP packets sent in ended calls")?.count(&[], self.rtp_tx)?;
            counter(e, "dphone_rtp_lost_packets", "RTP packets lost in ended calls")?.count(&[], self.rtp_lost)?;
            gauge(e, "dphone_rtp_jitter_seconds", "Worst receive jitter among calls in progress")?
                .float(&[], f64::from(jitter) / 1000.0)?;
            counter(e, "dphone_events_lost", "Events the call counters missed")?.count(&[], self.events_lost)?;
        }
        gauge(e, "dphone_dsp_status", "State of the voice DSP")?.one_of(
            &[],
            "status",
            &["not_started", "starting", "running", "failed"],
            match s.audio.dsp {
                st::DspStatus::NotStarted => "not_started",
                st::DspStatus::Starting => "starting",
                st::DspStatus::Running => "running",
                st::DspStatus::Failed(_) => "failed",
            },
        )?;

        // ------------------------------------------------- provisioning
        gauge(e, "dphone_provisioning_phase", "State of provisioning")?.one_of(
            &[],
            "phase",
            &[
                "idle",
                "discovering",
                "awaiting_server",
                "handshaking",
                "awaiting_user",
                "fetching_config",
                "applying",
                "provisioned",
                "standalone",
                "failed",
            ],
            match s.provisioning.phase {
                st::ProvisionPhase::Idle => "idle",
                st::ProvisionPhase::Discovering => "discovering",
                st::ProvisionPhase::AwaitingServer => "awaiting_server",
                st::ProvisionPhase::Handshaking => "handshaking",
                st::ProvisionPhase::AwaitingUser => "awaiting_user",
                st::ProvisionPhase::FetchingConfig => "fetching_config",
                st::ProvisionPhase::Applying => "applying",
                st::ProvisionPhase::Provisioned => "provisioned",
                st::ProvisionPhase::Standalone => "standalone",
                st::ProvisionPhase::Failed { .. } => "failed",
            },
        )?;
        gauge(e, "dphone_provisioning_attached", "The provisioning session is up")?.flag(&[], s.provisioning.attached)?;
        if let Some(at) = s.provisioning.last_success_unix {
            gauge(e, "dphone_provisioning_last_success_timestamp_seconds", "When provisioning last succeeded")?
                .int(&[], at)?;
        }

        // ------------------------------------------- configuration, flash
        gauge(e, "dphone_config_generation", "Configuration merges since the firmware started")?
            .int(&[], s.config.generation)?;
        gauge(e, "dphone_config_restart_needed", "A setting changed that needs a restart")?
            .flag(&[], s.config.restart_needed)?;
        gauge(e, "dphone_config_error", "The last configuration merge failed")?.flag(&[], !s.config.error.is_empty())?;
        {
            let mut f = gauge(e, "dphone_firmware_running_bank", "The flash bank the phone booted from")?;
            f.flag(&[("bank", "a")], s.update.running_bank == Some(st::Bank::A))?;
            f.flag(&[("bank", "b")], s.update.running_bank == Some(st::Bank::B))?;
        }
        gauge(e, "dphone_firmware_update_phase", "State of a firmware update")?.one_of(
            &[],
            "phase",
            &[
                "idle",
                "receiving",
                "checking",
                "erasing",
                "writing_kernel",
                "writing_rootfs",
                "verifying",
                "committing",
                "complete",
                "failed",
            ],
            match s.update.phase {
                st::UpdatePhase::Idle => "idle",
                st::UpdatePhase::Receiving { .. } => "receiving",
                st::UpdatePhase::Checking => "checking",
                st::UpdatePhase::Erasing { .. } => "erasing",
                st::UpdatePhase::WritingKernel { .. } => "writing_kernel",
                st::UpdatePhase::WritingRootfs { .. } => "writing_rootfs",
                st::UpdatePhase::Verifying { .. } => "verifying",
                st::UpdatePhase::Committing => "committing",
                st::UpdatePhase::Complete => "complete",
                st::UpdatePhase::Failed(_) => "failed",
            },
        )?;
        gauge(e, "dphone_firmware_trial", "Trial-boot state after an update")?.one_of(
            &[],
            "state",
            &["none", "pending", "rolled_back"],
            match s.update.trial {
                st::TrialBoot::None => "none",
                st::TrialBoot::Pending { .. } => "pending",
                st::TrialBoot::RolledBack { .. } => "rolled_back",
            },
        )?;

        // ------------------------------------------------ apps, access
        gauge(e, "dphone_apps_installed", "Installed phone apps")?.int(&[], s.apps.installed.len() as u64)?;
        gauge(e, "dphone_apps_running", "Running phone apps")?
            .int(&[], s.apps.installed.iter().filter(|a| a.running).count() as u64)?;
        if let Some(listening) = self.ssh_listening {
            gauge(e, "dphone_ssh_listening", "The SSH server is accepting connections")?.flag(&[], listening)?;
        }
        gauge(e, "dphone_web_sessions", "Logged-in web sessions")?.int(&[], self.web_sessions)?;
        gauge(e, "dphone_web_shells", "Open web terminals")?.int(&[], self.web_shells)?;
        counter(e, "dphone_log_lines", "Lines written to the log since the firmware started")?
            .count(&[], self.log_lines)?;

        // ------------------------------------------------------- memory
        let os = &self.os;
        if let Some(v) = os.mem_total {
            gauge(e, "dphone_memory_total_bytes", "Usable RAM")?.int(&[], v)?;
        }
        if let Some(v) = os.mem_available {
            gauge(e, "dphone_memory_available_bytes", "RAM available without swapping")?.int(&[], v)?;
        }
        if let Some(v) = os.mem_free {
            gauge(e, "dphone_memory_free_bytes", "Unused RAM")?.int(&[], v)?;
        }
        if let Some(load) = os.load {
            let mut f = gauge(e, "dphone_load_average", "System load average")?;
            for (window, v) in ["1m", "5m", "15m"].into_iter().zip(load) {
                f.float(&[("window", window)], v)?;
            }
        }
        if let Some((size, available)) = os.nvdata {
            gauge(e, "dphone_nvdata_size_bytes", "Size of the settings file system")?.int(&[], size)?;
            gauge(e, "dphone_nvdata_available_bytes", "Free space in the settings file system")?.int(&[], available)?;
        }
        if os.mtd.iter().any(|m| m.corrected.is_some()) {
            let mut f = counter(e, "dphone_mtd_ecc_corrected", "Flash bit errors corrected, by partition")?;
            for m in &os.mtd {
                if let Some(v) = m.corrected {
                    f.count(&[("partition", &m.name)], v)?;
                }
            }
        }
        if os.mtd.iter().any(|m| m.failures.is_some()) {
            let mut f = counter(e, "dphone_mtd_ecc_failures", "Flash reads with uncorrectable errors, by partition")?;
            for m in &os.mtd {
                if let Some(v) = m.failures {
                    f.count(&[("partition", &m.name)], v)?;
                }
            }
        }
        if os.mtd.iter().any(|m| m.bad_blocks.is_some()) {
            let mut f = gauge(e, "dphone_mtd_bad_blocks", "Bad flash blocks, by partition")?;
            for m in &os.mtd {
                if let Some(v) = m.bad_blocks {
                    f.int(&[("partition", &m.name)], v)?;
                }
            }
        }

        // ------------------------------------------------------ process
        if let Some(v) = os.rss {
            gauge(e, "process_resident_memory_bytes", "Resident memory of the firmware process")?.int(&[], v)?;
        }
        if let Some(v) = os.vsize {
            gauge(e, "process_virtual_memory_bytes", "Virtual memory of the firmware process")?.int(&[], v)?;
        }
        if let Some(v) = os.cpu {
            counter(e, "process_cpu_seconds", "CPU time used by the firmware process")?.count_float(v)?;
        }
        if let Some(v) = os.threads {
            gauge(e, "process_threads", "Threads in the firmware process")?.int(&[], v)?;
        }
        if let Some(v) = os.fds {
            gauge(e, "process_open_fds", "Open file descriptors")?.int(&[], v)?;
        }
        if let Some(v) = os.start_time {
            gauge(e, "process_start_time_seconds", "When the firmware process started, in Unix time")?.float(&[], v)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meminfo_lines() {
        let text = "MemTotal:         115200 kB\nMemFree:           20480 kB\nMemAvailable:      61440 kB\n";
        assert_eq!(meminfo(text, "MemTotal"), Some(115_200 * 1024));
        assert_eq!(meminfo(text, "MemAvailable"), Some(61_440 * 1024));
        assert_eq!(meminfo(text, "Mem"), None);
        assert_eq!(meminfo(text, "SwapTotal"), None);
    }

    #[test]
    fn kernel_decimals() {
        assert_eq!(hundredths("566001.07"), Some(566001.07));
        assert_eq!(hundredths("0.98"), Some(0.98));
        assert_eq!(hundredths("12"), Some(12.0));
        assert_eq!(hundredths("3.5"), Some(3.5));
        assert_eq!(hundredths("1.2345"), Some(1.23));
        assert_eq!(hundredths("x.1"), None);
        assert_eq!(hundredths("1.x"), None);
    }

    #[test]
    fn partition_names_are_plain() {
        assert_eq!(partition_label("nvdata\n"), "nvdata");
        assert_eq!(partition_label("a\"b{c}\\d e"), "abcde");
        assert_eq!(partition_label(&"x".repeat(100)).len(), 24);
    }

    #[test]
    fn every_end_cause_has_a_label() {
        let causes = [
            st::EndCause::LocalHangup,
            st::EndCause::RemoteHangup,
            st::EndCause::Rejected,
            st::EndCause::Canceled { answered_elsewhere: false },
            st::EndCause::Canceled { answered_elsewhere: true },
            st::EndCause::Busy,
            st::EndCause::NoAnswer,
            st::EndCause::Transferred,
            st::EndCause::Failed { code: 486, reason: String::new() },
            st::EndCause::Error(String::new()),
        ];
        let mut seen: Vec<usize> = causes.iter().map(cause_index).collect();
        seen.sort_unstable();
        assert_eq!(seen, (0..CAUSES.len()).collect::<Vec<_>>());
    }

    #[test]
    fn os_figures_on_this_machine() {
        let os = Os::read(Path::new("/"));
        assert!(os.uptime.unwrap() > 0.0);
        assert!(os.rss.unwrap() > 0);
        assert!(os.threads.unwrap() >= 1);
        assert!(os.fds.unwrap() >= 3);
        assert!(os.mem_total.unwrap() > 0);
        assert!(os.nvdata.unwrap().0 > 0);
        assert!(os.cpu.is_some() && os.start_time.is_some());
    }
}
