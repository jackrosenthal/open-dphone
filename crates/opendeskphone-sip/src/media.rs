//! The media engine as call control sees it.  `opendeskphone-media`'s audio
//! engine sits behind this trait (the integrator adapts its command
//! channel); tests use [`NullMedia`].
//!
//! All methods must return quickly: they are called from the SIP task and
//! are expected to post to the audio thread, not wait for it.

use crate::sdp::SrtpSuite;
use opendeskphone_state::{AudioPath, CallId, Codec, MediaStats, Tone};
use std::net::SocketAddr;
use std::sync::Mutex;

/// One direction-complete stream description, from SDP negotiation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamParams {
    pub codec: Codec,
    pub tx_pt: u8,
    pub rx_pt: u8,
    pub remote: SocketAddr,
    pub ptime_ms: u16,
    pub dtmf_tx_pt: Option<u8>,
    pub dtmf_rx_pt: Option<u8>,
    /// SDES keys: 16 bytes of master key then 14 of salt, per direction.
    pub srtp: Option<StreamSrtp>,
    /// Send our audio / play theirs.  Both false while held both ways.
    pub send: bool,
    pub recv: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamSrtp {
    pub suite: SrtpSuite,
    pub tx_key: [u8; 30],
    pub rx_key: [u8; 30],
}

/// What the ringer should do for an alerting call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RingRequest {
    pub call: CallId,
    /// `Alert-Info` value, if the INVITE carried one (distinctive ring).
    pub alert_info: String,
    /// Another call is up: play the call-waiting tone in its audio rather
    /// than ringing.
    pub call_waiting: bool,
}

/// Reports from the media engine, passed in through
/// [`crate::SipHandle::media_event`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaEvent {
    /// A telephone event (or in-band digit) arrived.
    Dtmf { call: CallId, digit: char },
    /// No RTP for the engine's timeout while the stream should receive.
    RtpTimeout { call: CallId },
    Stats { call: CallId, stats: MediaStats },
}

/// Call control's view of the audio engine.
pub trait MediaControl: Send + Sync + 'static {
    /// Codecs that can run now (G.711 always; DSP codecs whose init body
    /// is known).  Negotiation never offers or accepts anything else.
    fn codecs(&self) -> Vec<Codec>;
    /// Reserves an RTP socket for a call; returns its local port (RTCP is
    /// port + 1).
    fn open(&self, call: CallId) -> Result<u16, String>;
    /// Starts or reconfigures the call's stream.
    fn start(&self, call: CallId, params: &StreamParams) -> Result<(), String>;
    /// Stops the stream and releases the socket.
    fn close(&self, call: CallId);
    /// Sends one DTMF digit as RFC 4733 events on the call's stream.
    fn send_dtmf(&self, call: CallId, digit: char, duration_ms: u32);
    /// Connects these calls' streams to the audio path: none (all held or
    /// idle), one (normal), or several (local conference mix).
    fn connect(&self, calls: &[CallId]);
    /// Plays a call-progress tone on the current path, or stops it.
    fn tone(&self, tone: Option<Tone>);
    /// Starts the ringer (or call-waiting beep) for a call, or stops it.
    fn ring(&self, ring: Option<RingRequest>);
    /// Asks for an audio path because a call needs one (`Dial`/`Answer`
    /// with an explicit path).  The audio subsystem owns the policy
    /// otherwise.
    fn request_path(&self, path: AudioPath);
}

/// What a [`NullMedia`] was asked to do, for assertions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaCall {
    Open(CallId),
    Start(CallId, StreamParams),
    Close(CallId),
    Dtmf(CallId, char),
    Connect(Vec<CallId>),
    Tone(Option<Tone>),
    Ring(Option<RingRequest>),
    Path(AudioPath),
}

/// A media engine that does nothing but hand out port numbers and record
/// calls.  For tests and for running signaling without audio hardware.
pub struct NullMedia {
    codecs: Vec<Codec>,
    next_port: Mutex<u16>,
    log: Mutex<Vec<MediaCall>>,
}

impl NullMedia {
    pub fn new(codecs: Vec<Codec>, first_port: u16) -> Self {
        NullMedia { codecs, next_port: Mutex::new(first_port), log: Mutex::new(Vec::new()) }
    }
    /// Everything asked so far.
    pub fn calls(&self) -> Vec<MediaCall> {
        self.log.lock().unwrap().clone()
    }
    /// The latest `Start` parameters for a call.
    pub fn last_start(&self, call: CallId) -> Option<StreamParams> {
        self.log.lock().unwrap().iter().rev().find_map(|c| match c {
            MediaCall::Start(id, p) if *id == call => Some(p.clone()),
            _ => None,
        })
    }
    fn push(&self, c: MediaCall) {
        self.log.lock().unwrap().push(c);
    }
}

impl Default for NullMedia {
    fn default() -> Self {
        NullMedia::new(vec![Codec::Pcmu, Codec::Pcma, Codec::G722, Codec::G726_32], 4000)
    }
}

impl MediaControl for NullMedia {
    fn codecs(&self) -> Vec<Codec> {
        self.codecs.clone()
    }
    fn open(&self, call: CallId) -> Result<u16, String> {
        self.push(MediaCall::Open(call));
        let mut p = self.next_port.lock().unwrap();
        let port = *p;
        *p = p.wrapping_add(2).max(1024);
        Ok(port)
    }
    fn start(&self, call: CallId, params: &StreamParams) -> Result<(), String> {
        self.push(MediaCall::Start(call, params.clone()));
        Ok(())
    }
    fn close(&self, call: CallId) {
        self.push(MediaCall::Close(call));
    }
    fn send_dtmf(&self, call: CallId, digit: char, _duration_ms: u32) {
        self.push(MediaCall::Dtmf(call, digit));
    }
    fn connect(&self, calls: &[CallId]) {
        self.push(MediaCall::Connect(calls.to_vec()));
    }
    fn tone(&self, tone: Option<Tone>) {
        self.push(MediaCall::Tone(tone));
    }
    fn ring(&self, ring: Option<RingRequest>) {
        self.push(MediaCall::Ring(ring));
    }
    fn request_path(&self, path: AudioPath) {
        self.push(MediaCall::Path(path));
    }
}
