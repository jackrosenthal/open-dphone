//! What the EHS protocol engines share: the phone-side inputs and outputs,
//! timing, and the detection loop that picks an adapter.
//!
//! Everything here is recovered from the stock `ehs` daemon (new-family
//! build; function addresses in the comments are ELF virtual addresses in
//! that binary).  The daemon listened to four audio-bridge signals and sent
//! four requests to the phone UI; [`PhoneStatus`] and [`HeadsetAction`] are
//! those, without the bus.

use std::time::{Duration, Instant};

use super::{EhsPort, SerialConfig};
use crate::Result;

/// The phone state an adapter is told about.  The engines compare each new
/// value with the last one and act on the differences, as the stock daemon
/// acted on the `audio_status`, `audio_wire_status`, `audio_mute_status`,
/// `ring`, and `bt_hs_connect`/`bt_hs_disconnect` signals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PhoneStatus {
    /// Call audio is on the headset path.
    pub headset_active: bool,
    /// The microphone is muted.
    pub muted: bool,
    /// The phone is ringing.
    pub ringing: bool,
    /// A Bluetooth headset is connected and has control; the wired adapter
    /// is then kept on-hook and its buttons do nothing.
    pub bluetooth_headset: bool,
}

/// What a button on the headset asks of the phone.  These are the stock
/// daemon's four UI requests (`fn hdst`, `fn mute`, `call rjct`, `fn redial`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeadsetAction {
    /// Press the headset key: answer, hang up, or start a call on the headset.
    ToggleHeadset,
    /// Press the mute key.
    ToggleMute,
    /// Reject the ringing call.
    RejectCall,
    /// Redial the last number.
    Redial,
}

/// Which adapter protocol to speak: the stock `ehs` setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EhsMode {
    /// Try Plantronics, then Jabra, repeatedly (`auto`, and any other value).
    #[default]
    Auto,
    /// `plantronics`.
    Plantronics,
    /// `jabra_iq`.
    Jabra,
}

impl EhsMode {
    /// Parses the stock setting value, case-insensitively.  Unknown values
    /// mean auto-detection, as in stock.
    pub fn from_setting(value: &str) -> EhsMode {
        if value.eq_ignore_ascii_case("plantronics") {
            EhsMode::Plantronics
        } else if value.eq_ignore_ascii_case("jabra_iq") {
            EhsMode::Jabra
        } else {
            EhsMode::Auto
        }
    }
}

/// A detected adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Adapter {
    Plantronics,
    Jabra,
}

/// The engines' view of the phone.
pub trait EhsHost {
    /// The current phone state.  Called often; must be cheap.
    fn status(&mut self) -> PhoneStatus;
    /// A headset button was used.
    fn action(&mut self, action: HeadsetAction);
    /// An adapter was detected (`Some`) or lost (`None`).
    fn adapter(&mut self, adapter: Option<Adapter>);
    /// True when the engine should return (shutdown, or the UART goes back
    /// to the console).
    fn stop(&mut self) -> bool;
}

/// Protocol timing.  [`Timing::STOCK`] holds the recovered values; tests
/// shrink them.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    /// Wait after changing the line settings before talking (stock: 2 s).
    pub settle: Duration,
    /// Pause at the end of each detection round (stock: 2 s).
    pub round_pause: Duration,
    /// Longest wait for input in the event loops (stock: 500 ms).
    pub poll: Duration,
    /// Jabra: wait for the answer to a command byte (stock: 150 ms).
    pub jabra_response: Duration,
    /// Jabra: wait for the answer to the detection ping (stock: 500 ms).
    pub jabra_detect: Duration,
    /// Jabra: silence after which the adapter is pinged (stock: 4 s).
    pub jabra_keepalive: Duration,
    /// Jabra: pause between the capability exchange and the final ping
    /// (stock: 160 ms).
    pub jabra_after_caps: Duration,
    /// Plantronics: wait for the answer to a packet (stock: 500 ms).
    pub plantronics_response: Duration,
    /// Plantronics: silence after which the adapter is queried (stock: 8 s).
    pub plantronics_keepalive: Duration,
}

impl Timing {
    pub const STOCK: Timing = Timing {
        settle: Duration::from_secs(2),
        round_pause: Duration::from_secs(2),
        poll: Duration::from_millis(500),
        jabra_response: Duration::from_millis(150),
        jabra_detect: Duration::from_millis(500),
        jabra_keepalive: Duration::from_secs(4),
        jabra_after_caps: Duration::from_millis(160),
        plantronics_response: Duration::from_millis(500),
        plantronics_keepalive: Duration::from_secs(8),
    };
}

/// Identity the Jabra capability exchange reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    /// The three-byte model code (see [`super::jabra::model_code`]).
    pub model_code: [u8; 3],
    /// Firmware version: major, minor, patch.
    pub firmware: [u8; 3],
}

/// Sleeps for `d` in short slices; false if the host asked to stop.
pub(super) fn nap(host: &mut dyn EhsHost, d: Duration) -> bool {
    let end = Instant::now() + d;
    loop {
        if host.stop() {
            return false;
        }
        let now = Instant::now();
        if now >= end {
            return true;
        }
        std::thread::sleep((end - now).min(Duration::from_millis(50)));
    }
}

/// Discards pending input (the stock `tcflush` after the settle time).
pub(super) fn drain(port: &mut dyn EhsPort) {
    let mut buf = [0u8; 64];
    while matches!(port.read(&mut buf, Duration::ZERO), Ok(n) if n > 0) {}
}

/// Runs EHS on an open port until the host asks to stop: the stock main
/// loop (`0xaa50`).  Each round sets the line for one protocol, waits,
/// probes for the adapter, and runs that protocol's event loop for as long
/// as the adapter answers; then the next protocol; then a pause.
pub fn run(port: &mut dyn EhsPort, mode: EhsMode, identity: Identity, timing: &Timing, host: &mut dyn EhsHost) -> Result<()> {
    loop {
        if matches!(mode, EhsMode::Plantronics | EhsMode::Auto) {
            port.configure(SerialConfig::PLANTRONICS)?;
            if !nap(host, timing.settle) {
                return Ok(());
            }
            drain(port);
            if super::pehs::detect(port, timing, host)? {
                log::info!("ehs: Plantronics adapter detected");
                host.adapter(Some(Adapter::Plantronics));
                let result = super::pehs::event_loop(port, timing, host);
                host.adapter(None);
                result?;
            }
        }
        if host.stop() {
            return Ok(());
        }
        if matches!(mode, EhsMode::Jabra | EhsMode::Auto) {
            port.configure(SerialConfig::JABRA)?;
            if !nap(host, timing.settle) {
                return Ok(());
            }
            drain(port);
            if super::jabra::detect(port, timing)? {
                log::info!("ehs: Jabra adapter detected");
                host.adapter(Some(Adapter::Jabra));
                let result = super::jabra::event_loop(port, identity, timing, host);
                host.adapter(None);
                result?;
            }
        }
        if !nap(host, timing.round_pause) {
            return Ok(());
        }
    }
}
