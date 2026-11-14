//! dphone-vpm and dphone-media: the DSP and the real-time audio engine,
//! plus the audio policy that sits between them and the rest of the phone.
//!
//! This module owns the `Audio` topic of the state store and handles the
//! `Audio` command domain (path, mute, volumes, tones, sounds).  Call
//! control reaches the engine through [`AudioService`]'s methods (the SIP
//! wiring adapts them to its `MediaControl` trait).
//!
//! Path policy: an explicit request (speaker or headset key, `SetAudioPath`)
//! wins; otherwise a call or tone uses the headset in headset mode, the
//! handset when it is off the hook, else the speaker.  The ringer always
//! uses the speaker when nothing else has a path open.  With nothing to
//! play, the path is closed.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dphone_hal::{Audio as HalAudioCard, PcmConfig, PcmDevice};
use dphone_media::engine::{Command as Ec, Engine, EngineOptions, Event as Ee, PayloadCodec, SrtpConfig, StreamConfig};
use dphone_media::ring::{BuiltinRing, RingSource};
use dphone_media::tone::ToneSpec;
use dphone_media::{AudioDevice, AudioPath as MPath, DtmfMode, GainControl, VpmDsp, VpmDspOptions};
use dphone_state::{
    Audio, AudioPath, CallId, Command, CommandError, Domain, DspStatus, HealthStatus, Reply, Store, Subsystem, Tone,
    VOLUME_MAX, VolumeTarget,
};
use log::{error, info, warn};

use super::contained;
use super::hal::SharedHal;
use crate::diag::Output;
use crate::mainrole::{Context, state_model};

/// Stream slots in the engine (three-way conference).
const SLOTS: usize = 3;
/// First RTP port tried; pairs go up from here.
const RTP_PORT_BASE: u16 = 10_000;

/// A negotiated stream, in this module's own terms.
#[derive(Clone, Debug)]
pub struct Stream {
    pub codec: PayloadCodec,
    pub tx_pt: u8,
    pub remote: SocketAddr,
    pub ptime_ms: u16,
    pub dtmf_pt: Option<u8>,
    pub srtp: Option<SrtpConfig>,
    pub send: bool,
    pub recv: bool,
}

/// What the engine reports about a call's stream.
#[derive(Clone, Debug)]
pub enum CallMediaEvent {
    Dtmf { call: CallId, digit: char },
    RtpTimeout { call: CallId },
    Failed { call: CallId, error: String },
}

type Listener = Box<dyn Fn(CallMediaEvent) + Send + Sync>;

struct Sockets {
    rtp: UdpSocket,
    rtcp: Option<UdpSocket>,
    port: u16,
}

struct Inner {
    engine: Option<Engine>,
    why_no_engine: String,
    family: dphone_vpm::Family,
    /// Which call uses each engine slot.
    slots: [Option<CallId>; SLOTS],
    /// Sockets reserved by `open` and not yet handed to the engine.
    reserved: HashMap<CallId, Sockets>,
    ports: HashMap<CallId, u16>,
    started: HashMap<CallId, bool>,
    connected: Vec<CallId>,
    /// Path the user or call control asked for explicitly.
    requested: Option<AudioPath>,
    tone: Option<Tone>,
    ringing: bool,
    sound: bool,
    /// The engine's current path.
    path: AudioPath,
    next_port: u16,
    bind_ip: IpAddr,
}

/// Handle to the audio subsystem.  Cheap to clone; every method returns
/// quickly (it posts to the audio thread).
#[derive(Clone)]
pub struct AudioService {
    inner: Arc<Mutex<Inner>>,
    store: Store,
    listener: Arc<Mutex<Option<Listener>>>,
}

fn to_media(path: AudioPath) -> Option<MPath> {
    match path {
        AudioPath::None => None,
        AudioPath::Handset => Some(MPath::Handset),
        AudioPath::Headset => Some(MPath::Headset),
        AudioPath::Speaker => Some(MPath::Speaker),
    }
}

/// Receive gain for a volume step: the tuned default at step 5, 3 dB a step.
fn rx_gain_q8(default_q8: i16, level: u8) -> i16 {
    let delta_db = (level.min(VOLUME_MAX) as i32 - 5) * 3;
    (default_q8 as i32 + delta_db * 256).clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

/// Ringer gain in Q12 for a volume step: unity at step 5, 3 dB a step, off
/// at 0.  (Table of 10^(dB/20) * 4096, no floating point.)
fn ring_gain_q12(level: u8) -> i32 {
    const TABLE: [i32; 11] = [0, 1029, 1453, 2053, 2900, 4096, 5786, 8173, 11544, 16306, 23033];
    TABLE[level.min(VOLUME_MAX) as usize]
}

fn tone_spec(tone: Tone) -> Option<ToneSpec> {
    Some(match tone {
        Tone::Dial => ToneSpec::DIAL,
        Tone::Ringback => ToneSpec::RINGBACK,
        Tone::Busy => ToneSpec::BUSY,
        Tone::Reorder => ToneSpec::REORDER,
        Tone::CallWaiting => ToneSpec::CALL_WAITING,
        Tone::Confirm => ToneSpec::BEEP,
        Tone::Dtmf(key) => ToneSpec::dtmf(key, -20)?,
    })
}

impl AudioService {
    /// Codecs that can run now.
    pub fn codecs(&self) -> Vec<PayloadCodec> {
        let inner = self.inner.lock().unwrap();
        if inner.engine.is_none() {
            return Vec::new();
        }
        PayloadCodec::ALL.into_iter().filter(|c| c.is_available(inner.family)).collect()
    }

    /// Whether the audio engine is running at all.
    pub fn available(&self) -> Result<(), String> {
        let inner = self.inner.lock().unwrap();
        if inner.engine.is_some() { Ok(()) } else { Err(inner.why_no_engine.clone()) }
    }

    /// The address RTP sockets bind to follows the phone's address.
    pub fn set_local_ip(&self, ip: IpAddr) {
        self.inner.lock().unwrap().bind_ip = ip;
    }

    pub fn set_listener(&self, listener: Listener) {
        *self.listener.lock().unwrap() = Some(listener);
    }

    /// Reserve an RTP/RTCP socket pair for a call; returns the RTP port.
    pub fn open(&self, call: CallId) -> Result<u16, String> {
        let mut inner = self.inner.lock().unwrap();
        if inner.engine.is_none() {
            return Err(format!("audio is not available: {}", inner.why_no_engine));
        }
        if let Some(port) = inner.ports.get(&call) {
            return Ok(*port);
        }
        let start = inner.next_port;
        let pair = dphone_media::net::bind_rtp_pair(IpAddr::V4(Ipv4Addr::UNSPECIFIED), (RTP_PORT_BASE, RTP_PORT_BASE + 2000), start)
            .map_err(|e| format!("no RTP port: {e}"))?;
        let port = pair.port();
        inner.next_port = if port + 2 >= RTP_PORT_BASE + 2000 { RTP_PORT_BASE } else { port + 2 };
        inner.reserved.insert(call, Sockets { rtp: pair.rtp, rtcp: Some(pair.rtcp), port });
        inner.ports.insert(call, port);
        Ok(port)
    }

    /// Start or reconfigure a call's stream.
    pub fn start(&self, call: CallId, s: &Stream) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        let slot = match inner.slots.iter().position(|c| *c == Some(call)) {
            Some(i) => i,
            None => {
                let free = inner.slots.iter().position(Option::is_none).ok_or("all audio stream slots are in use")?;
                inner.slots[free] = Some(call);
                free
            }
        };
        if let Some(socks) = inner.reserved.remove(&call) {
            let mut cfg = StreamConfig::new(s.codec, s.tx_pt, socks.rtp, s.remote);
            cfg.rtcp = socks.rtcp;
            cfg.remote_rtcp = Some(SocketAddr::new(s.remote.ip(), s.remote.port().wrapping_add(1)));
            cfg.ptime_ms = s.ptime_ms.clamp(10, 60) as u8;
            cfg.dtmf_payload_type = s.dtmf_pt;
            cfg.srtp = s.srtp.clone();
            cfg.send = s.send;
            cfg.recv = s.recv;
            let _ = socks.port;
            inner.send(Ec::StartStream { id: slot, config: Box::new(cfg) })?;
            inner.started.insert(call, true);
        } else if inner.started.contains_key(&call) {
            // Already running: a re-INVITE changed direction or the remote.
            // (A codec or key change needs a fresh socket pair: close, open,
            // start.)
            inner.send(Ec::SetRemote {
                id: slot,
                rtp: s.remote,
                rtcp: Some(SocketAddr::new(s.remote.ip(), s.remote.port().wrapping_add(1))),
            })?;
            inner.send(Ec::SetDirection { id: slot, send: s.send, recv: s.recv })?;
        } else {
            inner.slots[slot] = None;
            return Err("stream started without a reserved socket".into());
        }
        drop(inner);
        self.refresh();
        Ok(())
    }

    /// Stop a call's stream and release its sockets.
    pub fn close(&self, call: CallId) {
        {
            let mut inner = self.inner.lock().unwrap();
            inner.reserved.remove(&call);
            inner.ports.remove(&call);
            inner.started.remove(&call);
            inner.connected.retain(|c| *c != call);
            if let Some(slot) = inner.slots.iter().position(|c| *c == Some(call)) {
                inner.slots[slot] = None;
                let _ = inner.send(Ec::StopStream { id: slot });
            }
        }
        self.refresh();
    }

    pub fn send_dtmf(&self, call: CallId, digit: char, duration_ms: u32) {
        let inner = self.inner.lock().unwrap();
        if let Some(slot) = inner.slots.iter().position(|c| *c == Some(call)) {
            let _ = inner.send(Ec::SendDtmf { id: slot, key: digit, duration_ms, mode: DtmfMode::Auto, feedback: true });
        }
    }

    /// Connect these calls' streams to the audio path (none, one, or a
    /// local conference).
    pub fn connect(&self, calls: &[CallId]) {
        self.inner.lock().unwrap().connected = calls.to_vec();
        self.refresh();
    }

    pub fn tone(&self, tone: Option<Tone>) {
        {
            let mut inner = self.inner.lock().unwrap();
            inner.tone = tone;
            let _ = inner.send(Ec::PlayTone(tone.and_then(tone_spec)));
        }
        self.refresh();
    }

    /// Start or stop the ringer.  `name` picks a built-in ring.
    pub fn ring(&self, name: Option<&str>) {
        {
            let mut inner = self.inner.lock().unwrap();
            inner.ringing = name.is_some();
            let source = name.map(|n| {
                let ring = BuiltinRing::ALL.into_iter().find(|r| r.name().eq_ignore_ascii_case(n)).unwrap_or(BuiltinRing::Classic);
                RingSource::Builtin(ring)
            });
            let level = self.store.read::<Audio, _>(|a| a.ringer_volume);
            let _ = inner.send(Ec::PlayRing(source, ring_gain_q12(level)));
        }
        self.refresh();
    }

    pub fn request_path(&self, path: AudioPath) {
        self.inner.lock().unwrap().requested = (path != AudioPath::None).then_some(path);
        self.refresh();
    }

    /// Take the volume settings from the merged configuration.
    pub fn apply_config(&self, cfg: &dphone_config::Config) {
        let level = |id: &str, now: u8| cfg.setting_num::<u8>(id).map(|v| v.min(VOLUME_MAX)).unwrap_or(now);
        self.store.update::<Audio, _>(|a| {
            a.handset_volume = level("handset_volume", a.handset_volume);
            a.headset_volume = level("headset_volume", a.headset_volume);
            a.speaker_volume = level("speaker_volume", a.speaker_volume);
            a.ringer_volume = level("ringer_volume", a.ringer_volume);
        });
    }

    /// Recompute the path and the conference mask and publish the state.
    fn refresh(&self) {
        let (off_hook, headset_mode, volumes) =
            self.store.read::<Audio, _>(|a| (a.off_hook, a.headset_mode, (a.handset_volume, a.headset_volume, a.speaker_volume)));
        let mut inner = self.inner.lock().unwrap();
        let wants_voice = !inner.connected.is_empty() || inner.tone.is_some() || inner.sound;
        if !wants_voice && !inner.ringing {
            // Nothing sounding: an explicit request does not outlive it.
            if inner.requested.is_some() && inner.slots.iter().all(Option::is_none) {
                inner.requested = None;
            }
        }
        let policy = if headset_mode {
            AudioPath::Headset
        } else if off_hook {
            AudioPath::Handset
        } else {
            AudioPath::Speaker
        };
        let path = match (inner.requested, wants_voice, inner.ringing) {
            (Some(p), _, _) => p,
            (None, true, _) => policy,
            (None, false, true) => AudioPath::Speaker,
            (None, false, false) => AudioPath::None,
        };
        if path != inner.path {
            if let Some(mp) = to_media(path) {
                let (default, level) = match path {
                    AudioPath::Handset => (inner.default_gain(MPath::Handset), volumes.0),
                    AudioPath::Headset => (inner.default_gain(MPath::Headset), volumes.1),
                    _ => (inner.default_gain(MPath::Speaker), volumes.2),
                };
                let _ = inner.send(Ec::SetRxGain { path: mp, gain_q8: rx_gain_q8(default, level) });
            }
            let _ = inner.send(Ec::SetPath(to_media(path)));
            inner.path = path;
        }
        let mut mask = 0u8;
        for (slot, call) in inner.slots.iter().enumerate() {
            if call.is_some_and(|c| inner.connected.contains(&c)) {
                mask |= 1 << slot;
            }
        }
        let _ = inner.send(Ec::SetConference { mask });
        let (tone, ringing) = (inner.tone, inner.ringing);
        drop(inner);
        self.store.update::<Audio, _>(|a| {
            a.path = path;
            a.tone = tone;
            a.ringing = ringing;
        });
    }

    fn set_volume(&self, target: VolumeTarget, level: u8) {
        let level = level.min(VOLUME_MAX);
        let (path, ringing) = {
            let inner = self.inner.lock().unwrap();
            (inner.path, inner.ringing)
        };
        let target = match target {
            VolumeTarget::Current => match path {
                AudioPath::Handset => VolumeTarget::Handset,
                AudioPath::Headset => VolumeTarget::Headset,
                AudioPath::Speaker if !ringing => VolumeTarget::Speaker,
                _ => VolumeTarget::Ringer,
            },
            t => t,
        };
        self.store.update::<Audio, _>(|a| match target {
            VolumeTarget::Handset => a.handset_volume = level,
            VolumeTarget::Headset => a.headset_volume = level,
            VolumeTarget::Speaker => a.speaker_volume = level,
            VolumeTarget::Ringer | VolumeTarget::Current => a.ringer_volume = level,
        });
        let inner = self.inner.lock().unwrap();
        let apply = |mp: MPath| {
            let _ = inner.send(Ec::SetRxGain { path: mp, gain_q8: rx_gain_q8(inner.default_gain(mp), level) });
        };
        match target {
            VolumeTarget::Handset => apply(MPath::Handset),
            VolumeTarget::Headset => apply(MPath::Headset),
            VolumeTarget::Speaker => apply(MPath::Speaker),
            VolumeTarget::Ringer | VolumeTarget::Current => {
                // The ring player takes its gain when it starts; a change
                // while ringing applies from the next ring.
            }
        }
    }

    fn current_level(&self, target: VolumeTarget) -> u8 {
        let path = self.inner.lock().unwrap().path;
        self.store.read::<Audio, _>(|a| match target {
            VolumeTarget::Handset => a.handset_volume,
            VolumeTarget::Headset => a.headset_volume,
            VolumeTarget::Speaker => a.speaker_volume,
            VolumeTarget::Ringer => a.ringer_volume,
            VolumeTarget::Current => match path {
                AudioPath::Handset => a.handset_volume,
                AudioPath::Headset => a.headset_volume,
                AudioPath::Speaker if !a.ringing => a.speaker_volume,
                _ => a.ringer_volume,
            },
        })
    }

    /// One command of the `Audio` domain.
    fn command(&self, cmd: Command) -> Result<Reply, CommandError> {
        match cmd {
            Command::SetAudioPath { path } => self.request_path(path),
            Command::SetMute { on } => {
                let _ = self.inner.lock().unwrap().send(Ec::MuteMic(on));
                self.store.update::<Audio, _>(|a| a.muted = on);
            }
            Command::SetVolume { target, level } => self.set_volume(target, level),
            Command::AdjustVolume { target, delta } => {
                let now = self.current_level(target) as i16;
                self.set_volume(target, (now + delta as i16).clamp(0, VOLUME_MAX as i16) as u8);
            }
            Command::SetHeadsetMode { on } => {
                self.store.update::<Audio, _>(|a| a.headset_mode = on);
                self.refresh();
            }
            Command::PlayTone { tone, duration_ms } => {
                self.tone(Some(tone));
                if duration_ms > 0 {
                    let me = self.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(duration_ms as u64)).await;
                        if me.inner.lock().unwrap().tone == Some(tone) {
                            me.tone(None);
                        }
                    });
                }
            }
            Command::StopTone => self.tone(None),
            Command::PlaySound { name } => {
                // Ringtone preview: the named built-in ring, once around.
                self.inner.lock().unwrap().sound = true;
                self.ring(Some(&name));
                let me = self.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(4)).await;
                    let was_preview = std::mem::replace(&mut me.inner.lock().unwrap().sound, false);
                    if was_preview {
                        me.ring(None);
                    }
                });
            }
            Command::StopSound => {
                let was_preview = std::mem::replace(&mut self.inner.lock().unwrap().sound, false);
                if was_preview {
                    self.ring(None);
                }
            }
            other => return Err(CommandError::Invalid(format!("not an audio command: {other:?}"))),
        }
        Ok(Reply::Done)
    }
}

impl Inner {
    fn send(&self, cmd: Ec) -> Result<(), String> {
        match &self.engine {
            Some(e) => e.send(cmd).map_err(|e| e.to_string()),
            None => Err(format!("audio is not available: {}", self.why_no_engine)),
        }
    }

    fn default_gain(&self, path: MPath) -> i16 {
        let g = dphone_vpm::startup::tuning(self.family).gains;
        match path {
            MPath::Handset => g.handset,
            MPath::Headset => g.headset,
            MPath::Speaker => g.speaker,
        }
    }
}

// ------------------------------------------------------------------------
// The sound card behind the engine's device trait.

struct CardDevice {
    card: Box<dyn HalAudioCard>,
    model: dphone_state::Model,
    pcm: Option<Box<dyn dphone_hal::Pcm>>,
    open: Option<MPath>,
}

fn pcm_device(path: MPath) -> PcmDevice {
    match path {
        MPath::Handset => PcmDevice::Handset,
        MPath::Headset => PcmDevice::Headset,
        MPath::Speaker => PcmDevice::Speaker,
    }
}

fn io_err(e: dphone_hal::HalError) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

impl AudioDevice for CardDevice {
    fn rate(&self) -> u32 {
        self.card.rate()
    }

    fn open(&mut self, path: MPath) -> std::io::Result<()> {
        self.close();
        let device = pcm_device(path);
        // The mainline driver does not apply the board's gain table.
        match self.card.mixer() {
            Ok(mut mixer) => {
                if let Err(e) = dphone_hal::apply_board_gains(mixer.as_mut(), self.model, device, 0) {
                    warn!("audio: board gains for {device:?}: {e}");
                }
            }
            Err(e) => warn!("audio: mixer: {e}"),
        }
        let pcm = self.card.open(device, PcmConfig::voice(self.model)).map_err(io_err)?;
        self.pcm = Some(pcm);
        self.open = Some(path);
        Ok(())
    }

    fn close(&mut self) {
        self.pcm = None;
        self.open = None;
    }

    fn read(&mut self, buf: &mut [i16]) -> std::io::Result<()> {
        let pcm = self.pcm.as_mut().ok_or_else(|| std::io::Error::other("no PCM open"))?;
        match pcm.read(buf) {
            Ok(()) => Ok(()),
            Err(dphone_hal::HalError::Xrun) => {
                pcm.recover().map_err(io_err)?;
                buf.fill(0);
                Ok(())
            }
            Err(e) => Err(io_err(e)),
        }
    }

    fn write(&mut self, buf: &[i16]) -> std::io::Result<()> {
        let pcm = self.pcm.as_mut().ok_or_else(|| std::io::Error::other("no PCM open"))?;
        match pcm.write(buf) {
            Ok(()) => Ok(()),
            Err(dphone_hal::HalError::Xrun) => pcm.recover().map_err(io_err),
            Err(e) => Err(io_err(e)),
        }
    }

    fn set_gain(&mut self, _path: MPath, control: GainControl, value: i32) -> std::io::Result<()> {
        // Front-end gains come from the board table at open; the user's
        // volume is the DSP's receive gain.  Explicit overrides are for
        // diagnostics only.
        warn!("audio: device gain {control:?}={value} ignored (board table in force)");
        Ok(())
    }
}

// ------------------------------------------------------------------------
// Start-up.

type Built = (Box<dyn AudioDevice>, Box<dyn dphone_media::Dsp>);

/// Start the DSP and build the engine's two devices.  Runs on its own
/// thread: starting the remoteproc loads firmware and can take seconds.
fn build_phone(card: Box<dyn HalAudioCard>, model: dphone_state::Model) -> Result<Built, String> {
    let rproc = dphone_vpm::remoteproc::Remoteproc::find().map_err(|e| format!("DSP remoteproc: {e}"))?;
    rproc.start().map_err(|e| format!("DSP start: {e}"))?;
    let device = dphone_vpm::Device::open_default().map_err(|e| format!("/dev/bcmring-vpm: {e}"))?;
    let vpm = dphone_vpm::Vpm::new(device).map_err(|e| format!("DSP info: {e}"))?;
    let dsp = VpmDsp::new(vpm, VpmDspOptions::default());
    Ok((Box::new(CardDevice { card, model, pcm: None, open: None }), Box::new(dsp)))
}

fn build_sim(family: dphone_vpm::Family) -> Result<Built, String> {
    let (fake, state) = dphone_media::fake::FakeAudio::new(family.device_rate());
    {
        let mut s = state.lock().unwrap();
        s.record = false;
        s.read_delay = Some(Duration::from_millis(10));
    }
    let vpm = dphone_vpm::Vpm::new(dphone_vpm::MockTransport::new(family)).map_err(|e| e.to_string())?;
    Ok((Box::new(fake), Box::new(VpmDsp::new(vpm, VpmDspOptions::default()))))
}

pub async fn start(ctx: &Context, hal: Option<SharedHal>) -> AudioService {
    ctx.store.set_health(Subsystem::Media, HealthStatus::Starting);
    let model = state_model(ctx.model);
    let family = match ctx.model.family() {
        crate::model::Family::Old => dphone_vpm::Family::Old,
        crate::model::Family::New => dphone_vpm::Family::New,
    };
    let service = AudioService {
        inner: Arc::new(Mutex::new(Inner {
            engine: None,
            why_no_engine: "not started".into(),
            family,
            slots: [None; SLOTS],
            reserved: HashMap::new(),
            ports: HashMap::new(),
            started: HashMap::new(),
            connected: Vec::new(),
            requested: None,
            tone: None,
            ringing: false,
            sound: false,
            path: AudioPath::None,
            next_port: RTP_PORT_BASE,
            bind_ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        })),
        store: ctx.store.clone(),
        listener: Arc::new(Mutex::new(None)),
    };
    ctx.store.update::<Audio, _>(|a| a.dsp = DspStatus::Starting);

    let built = if ctx.sim {
        build_sim(family)
    } else {
        // Take the sound card out of the hardware bundle; the engine owns it.
        let card = hal.and_then(|h| {
            let mut h = h.lock().ok()?;
            if h.missing.iter().any(|(name, _)| *name == "audio") {
                return None;
            }
            Some(std::mem::replace(&mut h.audio, Box::new(dphone_hal::Null("audio"))))
        });
        match card {
            Some(card) => contained("dsp-start", Duration::from_secs(30), move || build_phone(card, model))
                .await
                .and_then(|r| r),
            None => Err("no sound card".to_string()),
        }
    };

    let (ev_tx, ev_rx) = tokio::sync::mpsc::unbounded_channel::<Ee>();
    match built.and_then(|(audio, dsp)| {
        Engine::spawn(
            audio,
            dsp,
            move |e| {
                let _ = ev_tx.send(e);
            },
            EngineOptions::default(),
        )
        .map_err(|e| format!("audio thread: {e}"))
    }) {
        Ok(engine) => {
            let mut inner = service.inner.lock().unwrap();
            inner.engine = Some(engine);
            inner.why_no_engine.clear();
            drop(inner);
            info!("audio: engine running ({family:?} family{})", if ctx.sim { ", simulated devices" } else { "" });
            ctx.store.update::<Audio, _>(|a| a.dsp = DspStatus::Running);
            ctx.store.set_health(Subsystem::Media, HealthStatus::Ok);
        }
        Err(e) => {
            error!("audio: {e}; the phone runs without audio");
            service.inner.lock().unwrap().why_no_engine = e.clone();
            ctx.store.update::<Audio, _>(|a| a.dsp = DspStatus::Failed(e.clone()));
            ctx.store.set_health(Subsystem::Media, HealthStatus::Failed(e));
        }
    }

    tokio::spawn(events(service.clone(), ev_rx, ctx.sim));
    tokio::spawn(commands(service.clone(), ctx.store.clone()));
    tokio::spawn(hook_watch(service.clone(), ctx.store.clone()));
    register_diag(ctx, service.clone());
    service
}

/// Engine events, on the runtime.
async fn events(service: AudioService, mut rx: tokio::sync::mpsc::UnboundedReceiver<Ee>, sim: bool) {
    let mut restarts = 0u32;
    while let Some(event) = rx.recv().await {
        let call_of = |id: usize| service.inner.lock().unwrap().slots.get(id).copied().flatten();
        let tell = |e: CallMediaEvent| {
            if let Some(l) = service.listener.lock().unwrap().as_ref() {
                l(e);
            }
        };
        match event {
            Ee::Dtmf { id, edge } => {
                if let (Some(call), dphone_media::dtmf::DtmfEdge::Start(digit)) = (call_of(id), edge) {
                    tell(CallMediaEvent::Dtmf { call, digit });
                }
            }
            Ee::RtpTimeout { id } => {
                if let Some(call) = call_of(id) {
                    tell(CallMediaEvent::RtpTimeout { call });
                }
            }
            Ee::Stats { .. } => {}
            Ee::StreamError { id, error } => {
                warn!("audio: stream {id}: {error}");
                if let Some(call) = call_of(id) {
                    tell(CallMediaEvent::Failed { call, error });
                }
            }
            Ee::DspError { error, needs_restart } => {
                error!("audio: DSP: {error}{}", if needs_restart { " (restart needed)" } else { "" });
                if needs_restart && !sim {
                    restarts += 1;
                    if restarts > 5 {
                        service.store.update::<Audio, _>(|a| a.dsp = DspStatus::Failed(format!("{error}; gave up after 5 restarts")));
                        service.store.set_health(Subsystem::Media, HealthStatus::Failed(error.to_string()));
                        continue;
                    }
                    service.store.update::<Audio, _>(|a| a.dsp = DspStatus::Starting);
                    let result = contained("dsp-restart", Duration::from_secs(30), || {
                        dphone_vpm::remoteproc::Remoteproc::find().and_then(|r| r.restart()).map_err(|e| e.to_string())
                    })
                    .await
                    .and_then(|r| r);
                    match result {
                        Ok(()) => {
                            let _ = service.inner.lock().unwrap().send(Ec::RestartDsp);
                            service.store.update::<Audio, _>(|a| a.dsp = DspStatus::Running);
                            info!("audio: DSP restarted");
                        }
                        Err(e) => {
                            error!("audio: DSP restart: {e}");
                            service.store.update::<Audio, _>(|a| a.dsp = DspStatus::Failed(e.clone()));
                            service.store.set_health(Subsystem::Media, HealthStatus::Failed(e));
                        }
                    }
                }
            }
            Ee::AudioError { kind } => warn!("audio: sound card: {kind:?}"),
            Ee::ToneEnded => {
                service.inner.lock().unwrap().tone = None;
                service.refresh();
            }
        }
    }
}

/// The `Audio` command domain.
async fn commands(service: AudioService, store: Store) {
    let rx = store.handle(Domain::Audio);
    while let Some(envelope) = rx.next().await {
        let (cmd, responder) = envelope.split();
        responder.reply(service.command(cmd));
    }
}

/// Keep `Audio::off_hook` current and move a call between handset and
/// speaker when the handset is lifted or replaced.
async fn hook_watch(service: AudioService, store: Store) {
    let sub = store.subscribe_events(dphone_state::Topic::Audio);
    loop {
        let wake = sub.changed().await;
        for event in wake.events {
            if let dphone_state::Event::Hook { off_hook, .. } = event {
                store.update::<Audio, _>(|a| a.off_hook = off_hook);
                {
                    let mut inner = service.inner.lock().unwrap();
                    // Lifting the handset takes the audio there; replacing it
                    // drops an explicit handset choice (call control decides
                    // whether the call ends).
                    if off_hook {
                        if inner.path != AudioPath::None && !inner.ringing {
                            inner.requested = Some(AudioPath::Handset);
                        }
                    } else if inner.requested == Some(AudioPath::Handset) {
                        inner.requested = None;
                    }
                }
                service.refresh();
            }
        }
    }
}

fn register_diag(ctx: &Context, service: AudioService) {
    let store = ctx.store.clone();
    let svc = service.clone();
    ctx.diag.register(
        "audio",
        "audio [status] | path none|handset|headset|speaker | tone dial|ringback|busy|beep|off | ring [NAME]|off | volume TARGET LEVEL | mute on|off",
        move |args: Vec<String>, out: Output| {
            let (svc, store) = (svc.clone(), store.clone());
            async move {
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                let result: Result<String, String> = match args.as_slice() {
                    [] | ["status"] => {
                        let a = store.get::<Audio>();
                        let engine = match svc.available() {
                            Ok(()) => "running".to_string(),
                            Err(e) => format!("NOT running: {e}"),
                        };
                        let codecs: Vec<String> = svc.codecs().iter().map(|c| format!("{c:?}")).collect();
                        Ok(format!("engine: {engine}\ncodecs: {}\n{a:#?}\n", codecs.join(" ")))
                    }
                    ["path", p] => match *p {
                        "none" => Ok(AudioPath::None),
                        "handset" => Ok(AudioPath::Handset),
                        "headset" => Ok(AudioPath::Headset),
                        "speaker" => Ok(AudioPath::Speaker),
                        other => Err(format!("unknown path '{other}'")),
                    }
                    .map(|p| {
                        svc.request_path(p);
                        String::new()
                    }),
                    ["tone", t] => match *t {
                        "dial" => Ok(Some(Tone::Dial)),
                        "ringback" => Ok(Some(Tone::Ringback)),
                        "busy" => Ok(Some(Tone::Busy)),
                        "beep" => Ok(Some(Tone::Confirm)),
                        "off" => Ok(None),
                        other => Err(format!("unknown tone '{other}'")),
                    }
                    .and_then(|t| {
                        svc.available()?;
                        svc.tone(t);
                        Ok(String::new())
                    }),
                    ["ring", "off"] => {
                        svc.ring(None);
                        Ok(String::new())
                    }
                    ["ring"] | ["ring", _] => svc.available().map(|()| {
                        svc.ring(Some(args.get(1).copied().unwrap_or("Classic")));
                        String::new()
                    }),
                    ["volume", target, level] => {
                        let target = match *target {
                            "handset" => Some(VolumeTarget::Handset),
                            "headset" => Some(VolumeTarget::Headset),
                            "speaker" => Some(VolumeTarget::Speaker),
                            "ringer" => Some(VolumeTarget::Ringer),
                            "current" => Some(VolumeTarget::Current),
                            _ => None,
                        };
                        match (target, level.parse::<u8>()) {
                            (Some(t), Ok(l)) => {
                                svc.set_volume(t, l);
                                Ok(String::new())
                            }
                            _ => Err("volume handset|headset|speaker|ringer|current 0..10".into()),
                        }
                    }
                    ["mute", onoff @ ("on" | "off")] => svc
                        .command(Command::SetMute { on: *onoff == "on" })
                        .map(|_| String::new())
                        .map_err(|e| format!("{e:?}")),
                    _ => Err("usage: dphone audio [status] | path P | tone T | ring [NAME]|off | volume TARGET LEVEL | mute on|off".into()),
                };
                match result {
                    Ok(text) => {
                        if !text.is_empty() {
                            out.out(&text).await;
                        }
                        0
                    }
                    Err(e) => {
                        out.err(&format!("dphone audio: {e}\n")).await;
                        1
                    }
                }
            }
        },
    );

    let sim = ctx.sim;
    ctx.diag.register("dsp", "dsp [status] | selftest | restart", move |args: Vec<String>, out: Output| {
        let service = service.clone();
        async move {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            match args.as_slice() {
                [] | ["status"] => {
                    let mut text = match service.available() {
                        Ok(()) => "audio engine: running\n".to_string(),
                        Err(e) => format!("audio engine: NOT running: {e}\n"),
                    };
                    if !sim {
                        match dphone_vpm::remoteproc::Remoteproc::find() {
                            Ok(r) => text.push_str(&format!(
                                "remoteproc: {} state {:?} firmware {}\n",
                                r.dir().display(),
                                r.state(),
                                r.firmware().unwrap_or_default().trim()
                            )),
                            Err(e) => text.push_str(&format!("remoteproc: {e}\n")),
                        }
                    }
                    out.out(&text).await;
                    0
                }
                ["selftest"] => {
                    let report = dphone_vpm::diag::selftest();
                    let mut text = format!("{} checks, {} failures\n", report.checks, report.failures.len());
                    for f in &report.failures {
                        text.push_str(&format!("FAIL: {f}\n"));
                    }
                    out.out(&text).await;
                    if report.passed() { 0 } else { 1 }
                }
                ["restart"] => {
                    let r = contained("dsp-restart", Duration::from_secs(30), || {
                        dphone_vpm::remoteproc::Remoteproc::find().and_then(|r| r.restart()).map_err(|e| e.to_string())
                    })
                    .await
                    .and_then(|r| r);
                    match r {
                        Ok(()) => {
                            let _ = service.inner.lock().unwrap().send(Ec::RestartDsp);
                            out.out("DSP restarted\n").await;
                            0
                        }
                        Err(e) => {
                            out.err(&format!("dphone dsp: {e}\n")).await;
                            1
                        }
                    }
                }
                _ => {
                    out.err("usage: dphone dsp [status] | selftest | restart\n").await;
                    2
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gains() {
        assert_eq!(rx_gain_q8(0, 5), 0);
        assert_eq!(rx_gain_q8(-768, 6), 0);
        assert_eq!(rx_gain_q8(0, 0), -15 * 256);
        assert_eq!(ring_gain_q12(5), 4096);
        assert_eq!(ring_gain_q12(0), 0);
        assert!(ring_gain_q12(10) > ring_gain_q12(9));
    }
}
