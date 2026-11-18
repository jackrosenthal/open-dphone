//! The Jabra EHS protocol ("jehs" in the stock daemon's log): single bytes
//! at 2400 baud, 8 data bits, two stop bits.
//!
//! The command names and values, the exchanges, and the capability payloads
//! are the stock daemon's.  None of this has been run against a real
//! adapter.

use std::time::{Duration, Instant};

use super::link::{nap, EhsHost, HeadsetAction, Identity, PhoneStatus, Timing};
use super::EhsPort;
use crate::Result;

// Framing and answers.
pub const STX: u8 = 0x02;
pub const ETX: u8 = 0x03;
pub const ACK: u8 = 0x06;
pub const INIT_ACK: u8 = 0x07;
pub const NACK: u8 = 0x15;

// Phone to headset.
pub const INIT: u8 = 0x21;
pub const CAP_ID: u8 = 0x22;
pub const CAP_FW: u8 = 0x23;
pub const CAP_CMD: u8 = 0x24;
pub const CAP_SPD: u8 = 0x25;
pub const CAP_RES: u8 = 0x26;
pub const MUTE_INACTIVE: u8 = 0x2f;
pub const MUTE_ACTIVE: u8 = 0x37;
/// The ping; either side may send it.
pub const TEST: u8 = 0x55;
/// Named in the stock table; the stock daemon never sends it.
pub const INCOMING_CALL: u8 = 0x69;
pub const CALL_INACTIVE: u8 = 0xbb;
pub const CALL_ACTIVE: u8 = 0xdd;
pub const RING: u8 = 0xee;

// Headset to phone.
pub const EV_MUTE_OFF: u8 = 0x37;
pub const EV_REJECT_CALL: u8 = 0x65;
pub const EV_REDIAL: u8 = 0x73;
pub const EV_MUTE_ON: u8 = 0xbb;
pub const EV_GO_OFF_HOOK: u8 = 0xdd;
pub const EV_GO_ON_HOOK: u8 = 0xee;

/// Payload of the `CAP_CMD` exchange.
pub const CAP_CMD_PAYLOAD: [u8; 1] = [0x02];
/// Payload of the `CAP_SPD` exchange.
pub const CAP_SPD_PAYLOAD: [u8; 1] = [0x01];
/// Payload of the `CAP_RES` exchange.
pub const CAP_RES_PAYLOAD: [u8; 3] = [0, 0, 0];

/// Attempts for every command byte and for the ping.
const ATTEMPTS: u32 = 4;

/// The three bytes the stock daemon sends in the `CAP_ID` exchange for each
/// model name (its `DG_VERSION`).  Unknown names get zeros, as in stock.
pub fn model_code(model: &str) -> [u8; 3] {
    match model.to_ascii_uppercase().as_str() {
        "D40" | "D45" => [5, 1, 1],
        "D50" | "D55" => [5, 1, 2],
        "D70" => [5, 1, 3],
        "D60" | "D62" | "D65" => [5, 1, 4],
        _ => [0, 0, 0],
    }
}

/// The audio state the daemon tracks for the adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Disconnected = 0,
    Ringing = 1,
    RingingConnecting = 2,
    Connected = 3,
    ConnectedMuted = 4,
}

/// Events the daemon accepts from a headset; anything else forces a new
/// initialization.
fn known_event(b: u8) -> bool {
    matches!(b, EV_REJECT_CALL | EV_MUTE_OFF | TEST | INIT | EV_MUTE_ON | EV_REDIAL | EV_GO_OFF_HOOK | EV_GO_ON_HOOK)
}

/// The answer to a headset event, from the stock event loop's table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reaction {
    /// The byte answered at once: `ACK`, `NACK`, or nothing.
    pub answer: Option<u8>,
    /// A request to the phone.
    pub action: Option<HeadsetAction>,
    /// A second request to the phone (redial follows the headset key).
    pub then: Option<HeadsetAction>,
    /// A command byte sent to the headset after the answer.
    pub send: Option<u8>,
    pub state: State,
    /// Start over with `INIT`.
    pub reinit: bool,
}

/// What the stock daemon does with `event` in `state`.  With a Bluetooth
/// headset in control (`bluetooth`), button presses are acknowledged and
/// the adapter is put back on-hook instead of acting on them.
pub fn react(state: State, event: u8, bluetooth: bool) -> Reaction {
    use HeadsetAction::*;
    use State::*;
    let mut r = Reaction { answer: Some(ACK), action: None, then: None, send: None, state, reinit: false };
    let hang_up = |r: &mut Reaction| {
        r.send = Some(CALL_INACTIVE);
        r.state = Disconnected;
    };
    match (state, event) {
        (Disconnected, INIT) => {
            r.answer = None;
            r.reinit = true;
        }
        (Disconnected, EV_GO_OFF_HOOK) => {
            if bluetooth {
                r.send = Some(CALL_INACTIVE);
            } else {
                r.action = Some(ToggleHeadset);
            }
        }
        (Disconnected, EV_REDIAL) => {
            if bluetooth {
                r.send = Some(CALL_INACTIVE);
            } else {
                r.action = Some(ToggleHeadset);
                r.then = Some(Redial);
            }
        }
        (Disconnected, EV_GO_ON_HOOK) => {}
        (Ringing | RingingConnecting, EV_REJECT_CALL) => {
            if bluetooth {
                r.state = Disconnected;
            } else {
                r.action = Some(RejectCall);
            }
        }
        (Ringing, EV_GO_OFF_HOOK) => {
            if bluetooth {
                hang_up(&mut r);
            } else {
                r.action = Some(ToggleHeadset);
                r.state = RingingConnecting;
            }
        }
        (Ringing | RingingConnecting, EV_GO_ON_HOOK) => {}
        (Connected, EV_MUTE_OFF) | (ConnectedMuted, EV_MUTE_ON) => {}
        (Connected, EV_MUTE_ON) | (ConnectedMuted, EV_MUTE_OFF) => {
            if bluetooth {
                hang_up(&mut r);
            } else {
                r.action = Some(ToggleMute);
            }
        }
        (Connected | ConnectedMuted, EV_GO_OFF_HOOK) => {
            if bluetooth {
                hang_up(&mut r);
            }
        }
        (Connected | ConnectedMuted, EV_GO_ON_HOOK) => {
            if bluetooth {
                r.state = Disconnected;
            } else {
                r.action = Some(ToggleHeadset);
            }
        }
        (_, TEST) => {}
        (_, e) if known_event(e) => r.answer = Some(NACK),
        _ => {
            r.answer = None;
            r.reinit = true;
        }
    }
    r
}

/// The serial side of the protocol.
struct Line<'a> {
    port: &'a mut dyn EhsPort,
    response: Duration,
    /// When the adapter was last heard from.
    last_rx: Instant,
}

impl Line<'_> {
    fn put(&mut self, byte: u8) -> Result<()> {
        self.port.write(&[byte])
    }

    /// One byte, or `None` after `timeout`.
    fn get(&mut self, timeout: Duration) -> Result<Option<u8>> {
        let mut b = [0u8; 1];
        if self.port.read(&mut b, timeout)? == 1 {
            self.last_rx = Instant::now();
            Ok(Some(b[0]))
        } else {
            Ok(None)
        }
    }

    /// Waits up to `timeout` for one of `wanted`, skipping other bytes.
    fn response(&mut self, wanted: &[u8], timeout: Duration) -> Result<Option<u8>> {
        let end = Instant::now() + timeout;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            match self.get(left)? {
                Some(b) if wanted.contains(&b) => return Ok(Some(b)),
                Some(_) if Instant::now() < end => continue,
                _ => return Ok(None),
            }
        }
    }

    /// Sends a command byte and waits for its acknowledgement, up to four
    /// times.  `INIT` is acknowledged with `INIT_ACK`.
    fn send(&mut self, cmd: u8) -> Result<bool> {
        for _ in 0..ATTEMPTS {
            self.put(cmd)?;
            let wanted: &[u8] = if cmd == INIT { &[INIT_ACK] } else { &[ACK, NACK] };
            match self.response(wanted, self.response)? {
                Some(INIT_ACK) | Some(ACK) => return Ok(true),
                _ => {}
            }
        }
        log::debug!("ehs: jabra: no acknowledgement for 0x{cmd:02x}");
        Ok(false)
    }

    /// The ping: `TEST`, answered by `ACK` or `NACK`.
    fn ping(&mut self, timeout: Duration) -> Result<bool> {
        for _ in 0..ATTEMPTS {
            self.put(TEST)?;
            if self.response(&[ACK, NACK], timeout)?.is_some() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// One capability exchange: the command byte, then
    /// `STX payload ETX` from us, then `STX`, as many bytes, and `ETX` from
    /// the headset, which we acknowledge.  The headset's bytes are not
    /// interpreted (stock only logs them).
    fn exchange(&mut self, cmd: u8, payload: &[u8]) -> Result<bool> {
        if !self.send(cmd)? {
            return Ok(false);
        }
        self.put(STX)?;
        for b in payload {
            self.put(*b)?;
        }
        if !self.send(ETX)? {
            return Ok(false);
        }
        if self.get(self.response)? != Some(STX) {
            return Ok(false);
        }
        for _ in payload {
            if self.get(self.response)?.is_none() {
                return Ok(false);
            }
        }
        if self.get(self.response)? != Some(ETX) {
            return Ok(false);
        }
        self.put(ACK)?;
        Ok(true)
    }
}

/// Probes for a Jabra adapter: a ping with the detection timeout.
pub fn detect(port: &mut dyn EhsPort, timing: &Timing) -> Result<bool> {
    Line { port, response: timing.jabra_response, last_rx: Instant::now() }.ping(timing.jabra_detect)
}

struct Engine<'a> {
    line: Line<'a>,
    state: State,
    /// The phone state last acted on.
    seen: PhoneStatus,
}

impl Engine<'_> {
    /// Sends `cmd`; on acknowledgement moves to `next`.
    fn step(&mut self, cmd: u8, next: State) -> Result<bool> {
        let ok = self.line.send(cmd)?;
        if ok {
            self.state = next;
        }
        Ok(ok)
    }

    /// The audio path or mute changed (stock `audio_status`).
    fn audio_changed(&mut self, now: PhoneStatus) -> Result<()> {
        use State::*;
        let before_call = matches!(self.state, Disconnected | Ringing | RingingConnecting);
        if now.headset_active && !now.bluetooth_headset {
            if now.muted {
                if before_call {
                    self.step(CALL_ACTIVE, Connected)?;
                }
                if self.state == Connected {
                    self.step(MUTE_ACTIVE, ConnectedMuted)?;
                }
            } else if !(before_call && self.step(CALL_ACTIVE, Connected)?) && self.state == ConnectedMuted {
                self.step(MUTE_INACTIVE, Connected)?;
            }
        } else if self.state != Disconnected {
            self.step(CALL_INACTIVE, Disconnected)?;
        }
        Ok(())
    }

    /// Ringing started or stopped (stock `ring`).
    fn ring_changed(&mut self, now: PhoneStatus) -> Result<()> {
        use State::*;
        if now.bluetooth_headset {
            return Ok(());
        }
        if now.ringing {
            if self.state == Disconnected {
                self.step(RING, Ringing)?;
            }
        } else if self.state == Ringing {
            self.step(CALL_INACTIVE, Disconnected)?;
        } else if self.state == RingingConnecting {
            self.step(CALL_ACTIVE, Connected)?;
        }
        Ok(())
    }

    /// Acts on the differences between the phone state and the one last
    /// acted on.
    fn sync(&mut self, now: PhoneStatus) -> Result<()> {
        let old = std::mem::replace(&mut self.seen, now);
        if now.bluetooth_headset && !old.bluetooth_headset && self.state != State::Disconnected {
            self.step(CALL_INACTIVE, State::Disconnected)?;
        }
        if (now.headset_active, now.muted) != (old.headset_active, old.muted) {
            self.audio_changed(now)?;
        }
        if now.ringing != old.ringing {
            self.ring_changed(now)?;
        }
        Ok(())
    }

    /// `INIT` and the capability exchanges.  `Ok(false)` when the adapter
    /// does not answer `INIT`, or the host asked to stop.
    fn init(&mut self, identity: Identity, timing: &Timing, host: &mut dyn EhsHost) -> Result<bool> {
        loop {
            if host.stop() || !self.line.send(INIT)? {
                return Ok(false);
            }
            let caps = self.line.exchange(CAP_ID, &identity.model_code)?
                && self.line.exchange(CAP_FW, &identity.firmware)?
                && self.line.exchange(CAP_CMD, &CAP_CMD_PAYLOAD)?
                && self.line.exchange(CAP_SPD, &CAP_SPD_PAYLOAD)?;
            if caps {
                if !nap(host, timing.jabra_after_caps) {
                    return Ok(false);
                }
                if self.line.ping(timing.jabra_response)? && self.line.exchange(CAP_RES, &CAP_RES_PAYLOAD)? {
                    return Ok(true);
                }
            }
        }
    }
}

/// Runs the protocol with a detected adapter until it stops answering or
/// the host asks to stop.
pub fn event_loop(port: &mut dyn EhsPort, identity: Identity, timing: &Timing, host: &mut dyn EhsHost) -> Result<()> {
    let line = Line { port, response: timing.jabra_response, last_rx: Instant::now() };
    let mut e = Engine { line, state: State::Disconnected, seen: PhoneStatus::default() };
    let mut needs_init = true;
    loop {
        if host.stop() {
            return Ok(());
        }
        if needs_init {
            if !e.init(identity, timing, host)? {
                log::info!("ehs: jabra: adapter does not initialize; leaving");
                return Ok(());
            }
            needs_init = false;
            e.line.last_rx = Instant::now();
            e.state = State::Disconnected;
            // Stock asks the audio bridge to repeat its status here; the
            // equivalent is to act on the whole current state.
            e.seen = PhoneStatus::default();
        }
        e.sync(host.status())?;
        match e.line.get(timing.poll.min(Duration::from_millis(50)))? {
            Some(event) => {
                let bluetooth = e.seen.bluetooth_headset;
                let r = react(e.state, event, bluetooth);
                log::debug!("ehs: jabra: state {:?}, event 0x{event:02x}", e.state);
                if let Some(answer) = r.answer {
                    e.line.put(answer)?;
                }
                e.state = r.state;
                if let Some(cmd) = r.send {
                    e.line.send(cmd)?;
                }
                for action in [r.action, r.then].into_iter().flatten() {
                    host.action(action);
                }
                needs_init = r.reinit;
            }
            None => {
                if e.line.last_rx.elapsed() > timing.jabra_keepalive {
                    if e.line.ping(timing.jabra_response)? {
                        e.line.last_rx = Instant::now();
                    } else {
                        needs_init = true;
                    }
                }
            }
        }
    }
}
