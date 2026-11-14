//! The Plantronics EHS protocol ("pehs" in the stock daemon's log): framed,
//! checksummed packets at 9600 baud, 8N1.  The framing is in
//! [`super::plantronics`]; this is what the packets mean and when they are
//! sent.
//!
//! Recovered from the stock daemon's detection routine (`0xdaa4`), event
//! loop (`0xde84`), and phone-signal handler (`0xf124`).  A packet is a type
//! byte, a code byte, and a text payload:
//!
//! | Type | Meaning |
//! | --- | --- |
//! | `!` | Command or event.  `!A` carries the hook state; `!!` starts a hard reset |
//! | `?` | Query.  `?P` is the ping; the headset may ask `?C` (capabilities) and `?V` (version) |
//! | `+` | Acknowledgement of the packet with the same code |
//! | `-`, `%` | Negative acknowledgement: send the packet again |
//! | `*` | The packet with the same code is not supported: do not send it again |
//! | `I` | Information (the answers to `?C` and `?V`) |
//! | `E` | The headset asks for a reset |
//!
//! The payload of `!A` is three letters, `a`, `r`, and `m`; a capital means
//! "on".  `A` is the audio link (headset off-hook) and `R` is ringing.  The
//! stock daemon never changes the third letter.  The headset sends `!A`
//! with the `a` it wants when its button is pressed.
//!
//! None of this has been run against a real adapter.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::link::{EhsHost, HeadsetAction, PhoneStatus, Timing};
use super::plantronics::{encode, Packet, Parser};
use super::EhsPort;
use crate::Result;

const CMD: u8 = b'!';
const QUERY: u8 = b'?';
const ACK: u8 = b'+';
const NACK: u8 = b'-';
const NACK_ALT: u8 = b'%';
const UNSUPPORTED: u8 = b'*';
const INFO: u8 = b'I';
const RESET_REQUEST: u8 = b'E';
/// Answer to the headset's `?C`.
pub const CAPABILITIES: &[u8] = b"0Adimrt";
/// Answer to the headset's `?V`.
pub const VERSION: &[u8] = b"010103";

fn packet(kind: u8, code: u8, payload: &[u8]) -> Packet {
    Packet { header: (kind, code), payload: payload.to_vec() }
}

fn write(port: &mut dyn EhsPort, p: &Packet) -> Result<()> {
    // Payloads here are a few bytes; encode cannot fail.
    match encode(p.header, &p.payload) {
        Some(frame) => port.write(&frame),
        None => Ok(()),
    }
}

/// Reads until a complete packet, a framing error, or the deadline.
/// `Ok(None)` on timeout; `Ok(Some(Err(())))` on a malformed frame.
fn read_packet(port: &mut dyn EhsPort, parser: &mut Parser, pending: &mut VecDeque<std::result::Result<Packet, ()>>, timeout: Duration) -> Result<Option<std::result::Result<Packet, ()>>> {
    let end = Instant::now() + timeout;
    loop {
        if let Some(p) = pending.pop_front() {
            return Ok(Some(p));
        }
        let left = end.saturating_duration_since(Instant::now());
        let mut buf = [0u8; 64];
        let n = port.read(&mut buf, left)?;
        if n == 0 {
            return Ok(None);
        }
        pending.extend(parser.feed(&buf[..n]).into_iter().map(|r| r.map_err(|_| ())));
    }
}

/// Probes for a Plantronics adapter (`0xdaa4`): `?P`, answered by `+P`,
/// up to three times.
pub fn detect(port: &mut dyn EhsPort, timing: &Timing, host: &mut dyn EhsHost) -> Result<bool> {
    let mut parser = Parser::new();
    let mut pending = VecDeque::new();
    for _ in 0..3 {
        if host.stop() {
            return Ok(false);
        }
        write(port, &packet(QUERY, b'P', b""))?;
        match read_packet(port, &mut parser, &mut pending, timing.plantronics_response)? {
            Some(Ok(p)) if p.header == (ACK, b'P') => return Ok(true),
            _ => {}
        }
    }
    Ok(false)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Reset {
    Soft,
    Hard,
    None,
}

/// A queued item: a packet, or the marker that ends a reset.
enum Item {
    Packet(Packet),
    /// Reached once the reset's own packets were sent and answered.
    ResetDone { hard: bool },
}

/// Runs the protocol with a detected adapter until two resets in a row
/// fail, or the host asks to stop (`0xde84`).
pub fn event_loop(port: &mut dyn EhsPort, timing: &Timing, host: &mut dyn EhsHost) -> Result<()> {
    let mut parser = Parser::new();
    let mut received = VecDeque::new();
    let mut commands: VecDeque<Item> = VecDeque::new();
    let mut acks: VecDeque<Packet> = VecDeque::new();
    // The command sent last that still waits for its answer.
    let mut awaiting: Option<(Packet, Instant)> = None;
    let mut reset = Reset::Hard;
    let mut resets = 0u32;
    let mut keepalive = Instant::now();
    let mut state = *b"arm";
    // Stock asks the audio bridge to repeat its status at the start; the
    // equivalent is to act on the whole current state once.
    let mut seen = PhoneStatus::default();
    let hook = |state: &[u8; 3]| packet(CMD, b'A', state);

    loop {
        if host.stop() {
            return Ok(());
        }
        if reset != Reset::None {
            if resets > 1 {
                log::info!("ehs: plantronics: reset failed; leaving");
                return Ok(());
            }
            commands.clear();
            acks.clear();
            awaiting = None;
            parser = Parser::new();
            received.clear();
            if reset == Reset::Hard {
                commands.push_back(Item::Packet(packet(CMD, b'!', b"")));
            }
            commands.push_back(Item::Packet(packet(QUERY, b'P', b"")));
            commands.push_back(Item::ResetDone { hard: reset == Reset::Hard });
            reset = Reset::None;
            resets += 1;
        }

        // Phone state changes (stock `0xf124`).
        let now = host.status();
        let old = std::mem::replace(&mut seen, now);
        if now.bluetooth_headset && !old.bluetooth_headset {
            state[0] = b'a';
            commands.push_back(Item::Packet(hook(&state)));
        }
        if now.headset_active != old.headset_active {
            state[0] = if now.headset_active && !now.bluetooth_headset { b'A' } else { b'a' };
            commands.push_back(Item::Packet(hook(&state)));
        }
        if now.ringing != old.ringing && !now.bluetooth_headset {
            state[1] = if now.ringing { b'R' } else { b'r' };
            commands.push_back(Item::Packet(hook(&state)));
        }

        // What to write next: acknowledgements first, then one command at a
        // time (the next waits until the last is answered).
        let mut out = acks.pop_front();
        if out.is_none() && awaiting.is_none() {
            while let Some(Item::ResetDone { hard }) = commands.front() {
                let hard = *hard;
                commands.clear();
                acks.clear();
                parser = Parser::new();
                received.clear();
                resets = 0;
                // The hook state follows a reset; twice after a hard one.
                if hard {
                    commands.push_back(Item::Packet(hook(&state)));
                }
                commands.push_back(Item::Packet(hook(&state)));
            }
            if let Some(Item::Packet(p)) = commands.pop_front() {
                out = Some(p);
            }
        }
        if let Some(p) = out {
            write(port, &p)?;
            if matches!(p.header.0, CMD | QUERY | b'C' | INFO) {
                awaiting = Some((p, Instant::now() + timing.plantronics_response));
            }
            keepalive = Instant::now() + timing.plantronics_keepalive;
        }

        let wait = if acks.is_empty() { timing.poll.min(Duration::from_millis(50)) } else { Duration::ZERO };
        match read_packet(port, &mut parser, &mut received, wait)? {
            Some(Ok(p)) => {
                let (kind, code) = p.header;
                let answers_last = awaiting.as_ref().is_some_and(|(sent, _)| sent.header.1 == code);
                match kind {
                    CMD if code == b'A' => {
                        if !seen.bluetooth_headset {
                            acks.push_back(packet(ACK, code, b""));
                            for want in *b"aA" {
                                if p.payload.contains(&want) && state[0] != want {
                                    state[0] = want;
                                    host.action(HeadsetAction::ToggleHeadset);
                                    break;
                                }
                            }
                        } else if p.payload.contains(&b'A') {
                            // A Bluetooth headset has control: force the
                            // wired adapter back on-hook.
                            state[0] = b'a';
                            reset = Reset::Hard;
                            resets = 0;
                        }
                    }
                    CMD => acks.push_back(packet(UNSUPPORTED, code, b"")),
                    NACK | NACK_ALT => match awaiting.take() {
                        Some((sent, _)) if answers_last => commands.push_back(Item::Packet(sent)),
                        _ => reset = Reset::Soft,
                    },
                    UNSUPPORTED => {
                        if answers_last {
                            awaiting = None;
                        } else {
                            reset = Reset::Soft;
                        }
                    }
                    ACK => {
                        if answers_last {
                            awaiting = None;
                        }
                    }
                    QUERY => {
                        acks.push_back(packet(ACK, code, b""));
                        match code {
                            b'C' => acks.push_back(packet(INFO, b'C', CAPABILITIES)),
                            b'V' => acks.push_back(packet(INFO, b'V', VERSION)),
                            _ => {}
                        }
                    }
                    RESET_REQUEST => reset = Reset::Hard,
                    INFO => acks.push_back(packet(ACK, code, b"")),
                    _ => reset = Reset::Soft,
                }
                keepalive = Instant::now() + timing.plantronics_keepalive;
            }
            Some(Err(())) => reset = Reset::Soft,
            None => {
                let now = Instant::now();
                match &awaiting {
                    Some((_, deadline)) if now > *deadline => reset = Reset::Soft,
                    _ if now >= keepalive && commands.is_empty() && awaiting.is_none() => {
                        commands.push_back(Item::Packet(packet(QUERY, b'P', b"")));
                    }
                    _ => {}
                }
            }
        }
    }
}
