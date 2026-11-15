//! dphone-sip and dphone-dpma: accounts, calls, and provisioning.
//!
//! The SIP service needs the phone's address, so it starts when the network
//! first has one, and is reconfigured when the address or the merged
//! configuration changes.  DPMA rides on SIP MESSAGEs and starts right
//! after it.
//!
//! This module also keeps what must survive a restart and has no stock
//! file: the call log and the DND/forward settings, under `/nvdata/open/`.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dphone_config::{Config, Nvdata};
use dphone_media::PayloadCodec;
use dphone_media::engine::SrtpConfig;
use dphone_media::srtp::{MasterKey, Suite};
use dphone_sip::{
    AccountConfig, CodecPref, HostConfig, MediaControl, MediaEvent, RingRequest, SipConfig, SipHandle,
    StreamParams,
};
use dphone_state::{
    AccountId, AudioPath, CallId, CallLog, Codec, Features, HealthStatus, Store, Subsystem, Tone, Topic,
};
use log::{error, info, warn};
use tokio::sync::Notify;

use super::SharedConfig;
use super::audio::{AudioService, CallMediaEvent, Stream};
use super::net::NetHandle;
use super::system::ConfigHub;
use crate::mainrole::{Context, state_model};

/// What later subsystems (apps, web) need from telephony.
#[derive(Clone, Default)]
pub struct Telephony {
    /// Set once DPMA runs: that is after the phone has an address and SIP
    /// is up, long after [`start`] returns.  Use [`Telephony::when_dpma`]
    /// for anything the client must have before its first pass.
    pub dpma: Arc<Mutex<Option<dphone_dpma::DpmaHandle>>>,
    waiting: Arc<Mutex<Vec<DpmaHook>>>,
}

type DpmaHook = Box<dyn FnOnce(&dphone_dpma::DpmaHandle) + Send>;

impl Telephony {
    /// Runs `f` with the DPMA client: at once when it exists, else as soon
    /// as it is created, before it is told about the network (so before
    /// its first provisioning pass).
    pub fn when_dpma(&self, f: impl FnOnce(&dphone_dpma::DpmaHandle) + Send + 'static) {
        let dpma = self.dpma.lock().unwrap();
        match dpma.as_ref() {
            Some(handle) => {
                let handle = handle.clone();
                drop(dpma);
                f(&handle);
            }
            // Under the same lock as `set_dpma` takes, so no hook is left
            // behind.
            None => self.waiting.lock().unwrap().push(Box::new(f)),
        }
    }

    /// The DPMA client exists: publish it and run what waited for it.
    fn set_dpma(&self, handle: dphone_dpma::DpmaHandle) {
        let hooks = {
            let mut dpma = self.dpma.lock().unwrap();
            *dpma = Some(handle.clone());
            std::mem::take(&mut *self.waiting.lock().unwrap())
        };
        for hook in hooks {
            hook(&handle);
        }
    }
}

// ------------------------------------------------------------------------
// Media: call control's view of the audio engine.

struct SipMedia {
    audio: AudioService,
    store: Store,
    ringtone: Arc<Mutex<String>>,
}

fn payload_codec(c: Codec) -> Option<PayloadCodec> {
    Some(match c {
        Codec::Pcmu => PayloadCodec::Pcmu,
        Codec::Pcma => PayloadCodec::Pcma,
        Codec::G722 => PayloadCodec::G722,
        Codec::G726_32 => PayloadCodec::G726_32,
        _ => return None,
    })
}

fn state_codec(c: PayloadCodec) -> Codec {
    match c {
        PayloadCodec::Pcmu => Codec::Pcmu,
        PayloadCodec::Pcma => Codec::Pcma,
        PayloadCodec::G722 => Codec::G722,
        PayloadCodec::G726_32 => Codec::G726_32,
    }
}

fn master_key(k: &[u8; 30]) -> MasterKey {
    let mut key = [0u8; 16];
    let mut salt = [0u8; 14];
    key.copy_from_slice(&k[..16]);
    salt.copy_from_slice(&k[16..]);
    MasterKey { key, salt }
}

impl MediaControl for SipMedia {
    fn codecs(&self) -> Vec<Codec> {
        self.audio.codecs().into_iter().map(state_codec).collect()
    }

    fn open(&self, call: CallId) -> Result<u16, String> {
        self.audio.open(call)
    }

    fn start(&self, call: CallId, p: &StreamParams) -> Result<(), String> {
        let codec = payload_codec(p.codec).ok_or_else(|| format!("codec {:?} cannot run on this phone", p.codec))?;
        let srtp = p.srtp.as_ref().map(|s| SrtpConfig {
            suite: match s.suite {
                dphone_sip::sdp::SrtpSuite::AesCm128HmacSha1_80 => Suite::AesCm128HmacSha1_80,
                dphone_sip::sdp::SrtpSuite::AesCm128HmacSha1_32 => Suite::AesCm128HmacSha1_32,
            },
            tx: master_key(&s.tx_key),
            rx: master_key(&s.rx_key),
        });
        self.audio.start(
            call,
            &Stream {
                codec,
                tx_pt: p.tx_pt,
                remote: p.remote,
                ptime_ms: p.ptime_ms,
                dtmf_pt: p.dtmf_tx_pt,
                srtp,
                send: p.send,
                recv: p.recv,
            },
        )
    }

    fn close(&self, call: CallId) {
        self.audio.close(call);
    }

    fn send_dtmf(&self, call: CallId, digit: char, duration_ms: u32) {
        self.audio.send_dtmf(call, digit, duration_ms);
    }

    fn connect(&self, calls: &[CallId]) {
        self.audio.connect(calls);
    }

    fn tone(&self, tone: Option<Tone>) {
        self.audio.tone(tone);
    }

    fn ring(&self, ring: Option<RingRequest>) {
        match ring {
            Some(r) if r.call_waiting => self.audio.tone(Some(Tone::CallWaiting)),
            Some(_) => {
                let name = self.ringtone.lock().unwrap().clone();
                self.audio.ring(Some(&name));
            }
            None => {
                self.audio.ring(None);
                if self.store.read::<dphone_state::Audio, _>(|a| a.tone) == Some(Tone::CallWaiting) {
                    self.audio.tone(None);
                }
            }
        }
    }

    fn request_path(&self, path: AudioPath) {
        self.audio.request_path(path);
    }
}

// ------------------------------------------------------------------------
// Name resolution through the network manager's resolver.

struct NetResolver(NetHandle);

#[async_trait::async_trait]
impl dphone_sip::Resolver for NetResolver {
    async fn resolve(&self, host: &str, port: u16, _transport: dphone_state::Transport) -> Result<SocketAddr, String> {
        if let Ok(ip) = host.trim_matches(|c| c == '[' || c == ']').parse::<IpAddr>() {
            return Ok(SocketAddr::new(ip, port));
        }
        if let Some(resolver) = self.0.resolver() {
            match resolver.lookup_ip(host, false, false).await {
                Ok(addrs) => {
                    if let Some(ip) = addrs.first() {
                        return Ok(SocketAddr::new(*ip, port));
                    }
                }
                Err(e) => return Err(format!("cannot resolve {host}: {e:?}")),
            }
        }
        // Simulator, or the manager is not up: the host system's resolver.
        tokio::net::lookup_host((host, port))
            .await
            .map_err(|e| format!("cannot resolve {host}: {e}"))?
            .next()
            .ok_or_else(|| format!("{host} has no address"))
    }
}

// ------------------------------------------------------------------------
// Configuration mapping.

fn transport(t: dphone_config::view::Transport) -> dphone_state::Transport {
    match t {
        dphone_config::view::Transport::Udp => dphone_state::Transport::Udp,
        dphone_config::view::Transport::Tcp => dphone_state::Transport::Tcp,
        dphone_config::view::Transport::Tls => dphone_state::Transport::Tls,
    }
}

fn host(h: &dphone_config::view::Host) -> HostConfig {
    let mut out = HostConfig::new(h.server.clone());
    out.port = h.port;
    out.transport = transport(h.transport);
    out.expires_secs = h.reregister;
    out.retry_secs = h.retry;
    if let Some(n) = h.num_retries {
        out.num_retries = n;
    }
    out.srtp = h.media_encryption.as_deref().is_some_and(|m| !m.is_empty() && !m.eq_ignore_ascii_case("none"));
    out.auth_user = h.authname.clone();
    out.password = h.password.clone();
    out
}

fn codec_by_name(id: &str) -> Option<Codec> {
    Some(match id {
        "PCMU" => Codec::Pcmu,
        "PCMA" => Codec::Pcma,
        "G722" => Codec::G722,
        "G726-32" => Codec::G726_32,
        "G729" => Codec::G729,
        "iLBC" => Codec::Ilbc,
        "opus" => Codec::Opus,
        "L16" => Codec::L16,
        "L16-256" => Codec::L16Wide,
        _ => return None,
    })
}

/// A lamp behavior as stock's configuration writes it: `color` is `green`,
/// `red`, or `amber`; `state` is `off`, `on`, `slow`, or `fast`.
fn indicator_lamp(i: &dphone_config::view::Indicator) -> Option<dphone_state::LampState> {
    use dphone_state::{Blink, LampColor, LampState};
    let color = match i.color.as_str() {
        "green" => LampColor::Green,
        "red" => LampColor::Red,
        "amber" => LampColor::Amber,
        _ => return None,
    };
    Some(match i.state.as_str() {
        "off" => LampState::OFF,
        "on" => LampState::solid(color),
        "slow" => LampState::blinking(color, Blink::Slow),
        "fast" => LampState::blinking(color, Blink::Fast),
        _ => return None,
    })
}

fn sip_config(ctx: &Context, cfg: &Config, nv: &Nvdata, local_ip: IpAddr) -> SipConfig {
    let mut out = SipConfig::new(state_model(ctx.model), local_ip);
    let sip = cfg.sip();
    out.local_port = sip.udp_port;
    out.accounts = cfg
        .accounts()
        .iter()
        .filter(|a| a.enabled && a.primary.is_some())
        .enumerate()
        .map(|(n, a)| {
            let primary = a.primary.as_ref().expect("filtered");
            AccountConfig {
                id: AccountId(n as u8),
                config_id: a.account_id.clone(),
                label: if a.line_label.is_empty() { a.username.clone() } else { a.line_label.clone() },
                display_name: if a.caller_id.is_empty() { a.display.clone() } else { a.caller_id.clone() },
                user: a.username.clone(),
                auth_user: a.authname.clone(),
                password: a.password.clone(),
                primary: host(primary),
                alternate: a.alternate.as_ref().map(host),
                outbound_proxy: match (&a.outbound_proxy, a.outbound_port) {
                    (Some(p), Some(port)) if !p.is_empty() => format!("{p}:{port}"),
                    (Some(p), None) => p.clone(),
                    _ => String::new(),
                },
                voicemail_number: a.voicemail.clone().unwrap_or_default(),
                register: a.register,
                mwi_subscribe: a.needs_mwi_subscription.or(primary.need_mwi_subscription).unwrap_or(true),
                mwi_uri: a.mwi_subscription_uri.clone().or(primary.mwi_subscription_uri.clone()).unwrap_or_default(),
                dpma: a.kind.eq_ignore_ascii_case("dpma") || cfg.setting_bool("in_switchvox_environment", false),
            }
        })
        .collect();
    let codecs: Vec<CodecPref> = cfg
        .enabled_codecs()
        .iter()
        .filter_map(|c| Some(CodecPref { codec: codec_by_name(&c.id)?, ptime: c.packetization.clamp(10, 60) as u16 }))
        .collect();
    if !codecs.is_empty() {
        out.codecs = codecs;
    }
    // BLF and rapid-dial items; stock's line keys page on the D65 only
    // (`page_linekeys`), and its page-return time is off below 10 s and
    // at most 60 s.
    out.page_line_keys = cfg.setting_bool("page_linekeys", out.model == dphone_state::Model::D65) && out.model.blf_pages() > 1;
    out.blf = super::blfitems::items(cfg, nv, out.model, out.accounts.len(), out.page_line_keys, AccountId(0));
    // The message lamp's behaviors (stock `<activity_indicator>`); a
    // setting that is absent or unreadable keeps stock's base value.
    for (id, slot) in [
        ("new_vm", &mut out.activity.new_vm),
        ("ringing", &mut out.activity.ringing),
        ("active", &mut out.activity.active),
    ] {
        if let Some(lamp) = cfg.activity_indicator(id).and_then(|i| indicator_lamp(&i)) {
            *slot = lamp;
        }
    }
    // Incoming calls the configuration singles out by the server event
    // they carry (stock `<events><event id=".." action=".."/>`); without
    // any, stock's base entry (the voicemail playback call) stays.
    let events: Vec<(String, dphone_sip::EventAction)> = cfg
        .root()
        .children_named("events")
        .flat_map(|e| e.children_named("event"))
        .filter_map(|e| {
            let action = match e.attr("action")? {
                "auto_answer" | "auto_answer_now" => dphone_sip::EventAction::AutoAnswer,
                "app" => dphone_sip::EventAction::App,
                "reject" => dphone_sip::EventAction::Reject,
                _ => return None,
            };
            Some((e.attr("id").filter(|id| !id.is_empty())?.to_string(), action))
        })
        .collect();
    if !events.is_empty() {
        out.events = events;
    }
    out.blf_page_return_secs = match cfg.setting("blf_page_return_timeout").and_then(|v| v.trim().parse::<u32>().ok()) {
        Some(secs) if secs >= 10 => secs.min(60),
        _ => 0,
    };
    if let Some(ka) = sip.udp_keepalive_interval {
        out.udp_keepalive_secs = ka;
    }
    out.call_waiting = cfg.setting_bool("call_waiting_tone", true);
    out.park_blind_transfer = cfg.setting_bool("parking_lot_enable_blind_transfer", false);
    out.check_sync = sip.enable_check_sync;
    // Stock listens for SIP over TCP and TLS as well; the TLS side presents
    // the phone's own certificate (the one the web server generated).
    out.tcp_port = sip.tcp_enabled.then_some(sip.tcp_port);
    out.tls_port = sip.tls_allowed.then_some(sip.tls_port);
    if let (Ok(Some(cert)), Ok(Some(key))) = (nv.open_get("web-tls-cert.der"), nv.open_get("web-tls-key.der")) {
        out.tls.server_cert_der = cert;
        out.tls.server_key_der = key;
    }
    // Servers are checked against the trust bundle in `/nvdata/certs`
    // unless `allow_insecure_ssl` is set, as on stock.
    out.tls.verify = !sip.allow_insecure_ssl;
    out.tls.refuse_wildcard_certs = !sip.tls_allow_wildcard_certs;
    out.tls.roots_pem = super::trust::roots(nv, cfg);
    if let Some(park) = cfg.setting("parking_lot_extension").filter(|p| !p.is_empty()) {
        out.park_extension = park.to_string();
    }
    out
}

/// The phone's address: from the network state, or (simulator) the address
/// the host would use to reach the outside.
fn local_ip(ctx: &Context) -> Option<IpAddr> {
    if ctx.sim {
        let probe = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
        return Some(match probe.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).and_then(|()| probe.local_addr()) {
            Ok(a) if !a.ip().is_unspecified() => a.ip(),
            _ => IpAddr::V4(Ipv4Addr::LOCALHOST),
        });
    }
    ctx.store.read::<dphone_state::Network, _>(|n| n.ipv4.address).map(IpAddr::V4)
}

// ------------------------------------------------------------------------
// DPMA storage and configuration sink.

struct OpenStorage(Nvdata);

impl dphone_dpma::Storage for OpenStorage {
    fn read(&self, name: &str) -> Option<Vec<u8>> {
        self.0.open_get(name).ok().flatten()
    }
    fn write(&self, name: &str, data: &[u8]) -> Result<(), String> {
        self.0.open_put(name, data).map_err(|e| e.to_string())
    }
    fn remove(&self, name: &str) {
        let _ = self.0.open_remove(name);
    }
}

struct Sink {
    config: SharedConfig,
    hub: ConfigHub,
    store: Store,
    stock_writes: bool,
}

impl Sink {
    fn nv(&self) -> Nvdata {
        self.config.lock().unwrap().nvdata().clone()
    }
    fn guard(&self) -> Result<(), String> {
        if self.stock_writes {
            Ok(())
        } else {
            Err("this image leaves stock-owned nvdata files untouched; provisioning results are not saved".into())
        }
    }
}

impl dphone_dpma::ConfigSink for Sink {
    fn configurator(&self) -> Option<Vec<u8>> {
        self.nv().read("configurator.xml").ok().flatten()
    }

    fn write_configurator(&self, xml: &[u8]) -> Result<(), String> {
        self.guard()?;
        self.nv().write("configurator.xml", xml).map_err(|e| e.to_string())?;
        let cfg = self.config.lock().unwrap().reload().map_err(|e| e.to_string())?;
        self.hub.changed(&self.store, &cfg);
        Ok(())
    }

    fn cached_md5(&self) -> Option<String> {
        self.nv().cached_md5("config").ok().flatten()
    }

    fn fetched_config(&self) -> Option<Vec<u8>> {
        self.nv().read("fetched-config.xml").ok().flatten()
    }

    fn apply(&self, fetched_config: &[u8], md5: &str) -> Result<(), String> {
        self.guard()?;
        let xml = std::str::from_utf8(fetched_config).map_err(|e| format!("fetched configuration is not UTF-8: {e}"))?;
        let cfg = self.config.lock().unwrap().apply_fetched(xml).map_err(|e| e.to_string())?;
        self.nv().write("md5s/config", md5.as_bytes()).map_err(|e| e.to_string())?;
        self.hub.changed(&self.store, &cfg);
        Ok(())
    }

    fn ignore_provisioned_firmware(&self, fetched_config: &[u8]) -> Option<bool> {
        let store = self.config.lock().unwrap();
        super::provisioning::ignores_firmware(fetched_config, store.model(), store.dphone_settings())
    }

    fn wanted_resources(&self) -> dphone_dpma::resources::Wanted {
        super::provisioning::wanted_resources(&self.config.lock().unwrap().config())
    }

    fn files(&self) -> Option<Arc<dyn dphone_dpma::resources::Files>> {
        // Contact lists, ringtones, and their digests are stock's files too.
        self.stock_writes.then(|| Arc::new(super::provisioning::NvFiles(self.nv())) as Arc<dyn dphone_dpma::resources::Files>)
    }

    fn resources_changed(&self) {
        // Contact lists and BLF items are read together with the
        // configuration: everyone who follows it looks again.
        let cfg = self.config.lock().unwrap().config();
        self.hub.changed(&self.store, &cfg);
    }

    fn forget(&self) -> Result<(), String> {
        self.guard()?;
        let nv = self.nv();
        for file in ["fetched-config.xml", "configurator.xml", "md5s/config"] {
            nv.remove(file).map_err(|e| e.to_string())?;
        }
        let cfg = self.config.lock().unwrap().reload().map_err(|e| e.to_string())?;
        self.hub.changed(&self.store, &cfg);
        Ok(())
    }
}

// ------------------------------------------------------------------------
// Persistence of the call log and the feature settings.

const CALL_LOG_FILE: &str = "calllog.json";
const FEATURES_FILE: &str = "features.json";

fn restore(store: &Store, nv: &Nvdata) {
    if let Ok(Some(bytes)) = nv.open_get(CALL_LOG_FILE) {
        match serde_json::from_slice::<CallLog>(&bytes) {
            Ok(log) => store.set(log),
            Err(e) => warn!("telephony: saved call log unreadable: {e}"),
        }
    }
    if let Ok(Some(bytes)) = nv.open_get(FEATURES_FILE) {
        match serde_json::from_slice::<Features>(&bytes) {
            Ok(f) => store.set(f),
            Err(e) => warn!("telephony: saved feature settings unreadable: {e}"),
        }
    }
}

/// Save on change, but not more often than every few seconds: flash writes
/// are slow and wear the part.
async fn persist(store: Store, nv: Nvdata) {
    let sub = store.subscribe([Topic::CallLog, Topic::Features]);
    let mut saved_log = store.get::<CallLog>();
    let mut saved_features = store.get::<Features>();
    loop {
        sub.changed().await;
        tokio::time::sleep(Duration::from_secs(3)).await;
        let _ = sub.try_take();
        let log = store.get::<CallLog>();
        if *log != *saved_log {
            match serde_json::to_vec(&*log) {
                Ok(bytes) => {
                    if let Err(e) = nv.open_put(CALL_LOG_FILE, &bytes) {
                        warn!("telephony: saving the call log: {e}");
                    }
                }
                Err(e) => warn!("telephony: encoding the call log: {e}"),
            }
            saved_log = log;
        }
        let features = store.get::<Features>();
        if *features != *saved_features {
            if let Ok(bytes) = serde_json::to_vec(&*features)
                && let Err(e) = nv.open_put(FEATURES_FILE, &bytes)
            {
                warn!("telephony: saving feature settings: {e}");
            }
            saved_features = features;
        }
    }
}

// ------------------------------------------------------------------------

pub fn start(ctx: &Context, config: Option<SharedConfig>, hub: &ConfigHub, net: NetHandle, audio: AudioService) -> Telephony {
    let telephony = Telephony::default();
    let Some(config) = config else {
        error!("telephony: no configuration; SIP and DPMA are not started");
        ctx.store.set_health(Subsystem::Sip, HealthStatus::Failed("no configuration".into()));
        return telephony;
    };
    ctx.store.set_health(Subsystem::Sip, HealthStatus::Starting);
    let nv = config.lock().unwrap().nvdata().clone();
    if ctx.recovery.is_none() {
        restore(&ctx.store, &nv);
        tokio::spawn(persist(ctx.store.clone(), nv.clone()));
    }
    let changed = Arc::new(Notify::new());
    {
        let changed = changed.clone();
        hub.on_change(move |_| changed.notify_one());
    }
    tokio::spawn(run(ctx.clone(), config, hub.clone(), net, audio, changed, telephony.clone()));
    telephony
}

async fn run(
    ctx: Context,
    config: SharedConfig,
    hub: ConfigHub,
    net: NetHandle,
    audio: AudioService,
    changed: Arc<Notify>,
    telephony: Telephony,
) {
    let nv = config.lock().unwrap().nvdata().clone();
    let sub = ctx.store.subscribe(Topic::Network);

    // Wait for an address.
    let ip = loop {
        if let Some(ip) = local_ip(&ctx) {
            break ip;
        }
        sub.changed().await;
    };
    audio.set_local_ip(ip);

    let ringtone = Arc::new(Mutex::new(String::new()));
    let set_ringtone = |cfg: &Config| *ringtone.lock().unwrap() = cfg.ringtones().active;
    let mut current = {
        let cfg = config.lock().unwrap().config();
        set_ringtone(&cfg);
        sip_config(&ctx, &cfg, &nv, ip)
    };
    let media: Arc<dyn MediaControl> =
        Arc::new(SipMedia { audio: audio.clone(), store: ctx.store.clone(), ringtone: ringtone.clone() });
    let resolver: Arc<dyn dphone_sip::Resolver> = Arc::new(NetResolver(net.clone()));
    // SIP sockets carry the network's SIP marks (DSCP and priority), read
    // when each socket is made and again when the configuration changes.
    {
        let net = net.clone();
        dphone_sip::set_socket_marker(Arc::new(move |fd| {
            if let Err(e) = dphone_net::qos::mark_socket(&fd, dphone_net::TrafficClass::Sip, &net.qos()) {
                warn!("sip: cannot mark a socket: {e}");
            }
        }));
    }

    let sip: SipHandle = loop {
        match dphone_sip::start(ctx.store.clone(), current.clone(), media.clone(), Some(resolver.clone())).await {
            Ok(h) => break h,
            Err(e) => {
                error!("sip: {e}; retrying");
                ctx.store.set_health(Subsystem::Sip, HealthStatus::Failed(e));
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        }
    };
    info!("sip: running on {ip}:{} with {} account(s)", sip.local_port(), current.accounts.len());
    ctx.store.set_health(Subsystem::Sip, HealthStatus::Ok);

    // Engine events back to call control.
    {
        let sip = sip.clone();
        audio.set_listener(Box::new(move |e| match e {
            CallMediaEvent::Dtmf { call, digit } => sip.media_event(MediaEvent::Dtmf { call, digit }),
            CallMediaEvent::RtpTimeout { call } => sip.media_event(MediaEvent::RtpTimeout { call }),
            CallMediaEvent::Stats { call, stats } => sip.media_event(MediaEvent::Stats { call, stats }),
            CallMediaEvent::Failed { call, error } => {
                warn!("sip: media for call {call:?} failed: {error}");
                sip.media_event(MediaEvent::RtpTimeout { call });
            }
        }));
    }

    // DPMA.
    let mac = ctx.store.read::<dphone_state::System, _>(|s| s.mac).map(|m| m.iter().map(|b| format!("{b:02x}")).collect::<String>());
    match mac {
        Some(mac) => {
            ctx.store.set_health(Subsystem::Dpma, HealthStatus::Starting);
            let (transport, incoming) = dphone_dpma::sip::transport(&sip, &mac);
            let mut cfg = dphone_dpma::DpmaConfig::new(mac, state_model(ctx.model).user_agent(&dphone_state::FirmwareVersion::CURRENT));
            cfg.ignore_provisioned_firmware = config.lock().unwrap().config().ignore_provisioned_firmware().value;
            let sink = Arc::new(Sink {
                config: config.clone(),
                hub: hub.clone(),
                store: ctx.store.clone(),
                stock_writes: super::stock_writes_allowed(&ctx),
            });
            let handle = dphone_dpma::start(ctx.store.clone(), cfg, transport, incoming, Arc::new(OpenStorage(nv.clone())), sink);
            // "Ignore provisioned firmware" follows the configuration in
            // effect, whether the phone or the server changed it.
            {
                let handle = handle.clone();
                hub.on_change(move |cfg| handle.set_ignore_provisioned_firmware(cfg.ignore_provisioned_firmware().value));
            }
            // Contact lists and the like that the configuration names by
            // HTTP URL are fetched with the phone's resolver; HTTPS servers
            // are checked against the trust bundle.
            handle.set_http_source(super::provisioning::http_source(&net, config.clone()));
            // Whoever waited for the client (the firmware updater) gets it
            // now, before the first report below starts provisioning.
            telephony.set_dpma(handle.clone());
            // Network state, DHCP option 66, and mDNS announcements.
            super::provisioning::start_discovery(&ctx, &net, &handle);
            ctx.store.set_health(Subsystem::Dpma, HealthStatus::Ok);
        }
        None => {
            warn!("dpma: no MAC address; provisioning is not started");
            ctx.store.set_health(Subsystem::Dpma, HealthStatus::Failed("no MAC address".into()));
        }
    }

    // Follow the address and the configuration.
    loop {
        tokio::select! {
            _ = sub.changed() => {}
            _ = changed.notified() => {}
        }
        let Some(ip) = local_ip(&ctx) else { continue };
        audio.set_local_ip(ip);
        let cfg = config.lock().unwrap().config();
        set_ringtone(&cfg);
        let next = sip_config(&ctx, &cfg, &nv, ip);
        if next != current {
            info!("sip: configuration changed ({} account(s), address {ip})", next.accounts.len());
            sip.reconfigure(next.clone());
            current = next;
        }
    }
}
