//! The protocol engines against scripted headsets.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use super::jabra;
use super::plantronics::{encode, Packet, Parser};
use super::*;
use crate::Result;

fn fast() -> Timing {
    let ms = Duration::from_millis;
    Timing {
        settle: ms(1),
        round_pause: ms(1),
        poll: ms(5),
        jabra_response: ms(20),
        jabra_detect: ms(20),
        jabra_keepalive: ms(60),
        jabra_after_caps: ms(1),
        plantronics_response: ms(30),
        plantronics_keepalive: ms(80),
    }
}

#[derive(Default)]
struct World {
    status: PhoneStatus,
    actions: Vec<HeadsetAction>,
    adapters: Vec<Option<Adapter>>,
    stop: bool,
    /// Every byte the phone wrote.
    written: Vec<u8>,
    config: Option<SerialConfig>,
}

type Shared = Rc<RefCell<World>>;

/// A port whose far end is a function from written bytes to reply bytes.
struct FakePort<F: FnMut(&mut World, &[u8]) -> Vec<u8>> {
    world: Shared,
    rx: VecDeque<u8>,
    headset: F,
}

impl<F: FnMut(&mut World, &[u8]) -> Vec<u8> + Send> EhsPort for FakePort<F> {
    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize> {
        if self.rx.is_empty() {
            std::thread::sleep(timeout.min(Duration::from_millis(1)));
            return Ok(0);
        }
        let n = buf.len().min(self.rx.len());
        for b in buf[..n].iter_mut() {
            *b = self.rx.pop_front().unwrap();
        }
        Ok(n)
    }
    fn write(&mut self, data: &[u8]) -> Result<()> {
        let mut w = self.world.borrow_mut();
        w.written.extend_from_slice(data);
        let reply = (self.headset)(&mut w, data);
        self.rx.extend(reply);
        Ok(())
    }
    fn configure(&mut self, config: SerialConfig) -> Result<()> {
        self.world.borrow_mut().config = Some(config);
        self.rx.clear();
        Ok(())
    }
    fn set_lines(&mut self, _: bool, _: bool) -> Result<()> {
        Ok(())
    }
}

// The tests are single-threaded; the trait only asks for Send.
unsafe impl<F: FnMut(&mut World, &[u8]) -> Vec<u8>> Send for FakePort<F> {}

struct Host(Shared);

impl EhsHost for Host {
    fn status(&mut self) -> PhoneStatus {
        self.0.borrow().status
    }
    fn action(&mut self, action: HeadsetAction) {
        let mut w = self.0.borrow_mut();
        w.actions.push(action);
        // The phone's side of each button, as the UI would do it.
        match action {
            HeadsetAction::ToggleHeadset => {
                w.status.headset_active = !w.status.headset_active;
                w.status.ringing = false;
                if !w.status.headset_active {
                    w.status.muted = false;
                }
            }
            HeadsetAction::ToggleMute => w.status.muted = !w.status.muted,
            HeadsetAction::RejectCall => w.status.ringing = false,
            HeadsetAction::Redial => {}
        }
    }
    fn adapter(&mut self, adapter: Option<Adapter>) {
        self.0.borrow_mut().adapters.push(adapter);
    }
    fn stop(&mut self) -> bool {
        self.0.borrow().stop
    }
}

/// A Jabra headset: acknowledges everything, completes capability
/// exchanges, and presses a button when `on_command` says so.
fn jabra_headset(mut on_command: impl FnMut(&mut World, u8) -> Vec<u8>) -> impl FnMut(&mut World, &[u8]) -> Vec<u8> {
    // Inside a capability exchange: bytes still expected, and the payload
    // length to echo back.
    let mut exchange: Option<(usize, usize)> = None;
    let mut awaiting_stx = None;
    move |w, data| {
        let mut out = Vec::new();
        for &b in data {
            if let Some((left, n)) = exchange {
                if left > 0 {
                    exchange = Some((left - 1, n));
                } else {
                    assert_eq!(b, jabra::ETX);
                    exchange = None;
                    out.push(jabra::ACK);
                    out.push(jabra::STX);
                    out.extend(std::iter::repeat_n(0x11, n));
                    out.push(jabra::ETX);
                }
                continue;
            }
            if let Some(n) = awaiting_stx.take() {
                assert_eq!(b, jabra::STX);
                exchange = Some((n, n));
                continue;
            }
            match b {
                jabra::ACK | jabra::NACK => {}
                jabra::INIT => out.push(jabra::INIT_ACK),
                jabra::CAP_ID | jabra::CAP_FW | jabra::CAP_RES => {
                    out.push(jabra::ACK);
                    awaiting_stx = Some(3);
                }
                jabra::CAP_CMD | jabra::CAP_SPD => {
                    out.push(jabra::ACK);
                    awaiting_stx = Some(1);
                }
                cmd => {
                    out.push(jabra::ACK);
                    out.extend(on_command(w, cmd));
                }
            }
        }
        out
    }
}

const IDENTITY: Identity = Identity { model_code: [5, 1, 4], firmware: [3, 0, 0] };

#[test]
fn jabra_call_from_ring_to_hang_up() {
    let world: Shared = Rc::default();
    world.borrow_mut().status.ringing = true;
    let headset = jabra_headset(|w, cmd| match cmd {
        // Ringing: the user presses the hook button.
        jabra::RING => vec![jabra::EV_GO_OFF_HOOK],
        // Connected: mute.
        jabra::CALL_ACTIVE => vec![jabra::EV_MUTE_ON],
        // Muted: hang up.
        jabra::MUTE_ACTIVE => vec![jabra::EV_GO_ON_HOOK],
        jabra::CALL_INACTIVE => {
            w.stop = true;
            vec![]
        }
        _ => vec![],
    });
    let mut port = FakePort { world: world.clone(), rx: VecDeque::new(), headset };
    jabra::event_loop(&mut port, IDENTITY, &fast(), &mut Host(world.clone())).unwrap();

    let w = world.borrow();
    assert_eq!(w.actions, [HeadsetAction::ToggleHeadset, HeadsetAction::ToggleMute, HeadsetAction::ToggleHeadset]);
    // The initialization: INIT, then each capability with its payload.
    let init: Vec<u8> = [
        vec![jabra::INIT],
        vec![jabra::CAP_ID, jabra::STX, 5, 1, 4, jabra::ETX, jabra::ACK],
        vec![jabra::CAP_FW, jabra::STX, 3, 0, 0, jabra::ETX, jabra::ACK],
        vec![jabra::CAP_CMD, jabra::STX, 2, jabra::ETX, jabra::ACK],
        vec![jabra::CAP_SPD, jabra::STX, 1, jabra::ETX, jabra::ACK],
        vec![jabra::TEST],
        vec![jabra::CAP_RES, jabra::STX, 0, 0, 0, jabra::ETX, jabra::ACK],
    ]
    .concat();
    assert_eq!(&w.written[..init.len()], &init[..]);
    // Then the call: ring, acknowledge off-hook, call active, acknowledge
    // mute, mute active, acknowledge on-hook, call inactive.
    assert_eq!(
        &w.written[init.len()..],
        [jabra::RING, jabra::ACK, jabra::CALL_ACTIVE, jabra::ACK, jabra::MUTE_ACTIVE, jabra::ACK, jabra::CALL_INACTIVE]
    );
}

#[test]
fn jabra_reaction_table() {
    use jabra::{react, State::*};
    use HeadsetAction::*;
    // Idle: the hook button starts a call; redial also redials.
    let r = react(Disconnected, jabra::EV_GO_OFF_HOOK, false);
    assert_eq!((r.answer, r.action, r.then, r.state), (Some(jabra::ACK), Some(ToggleHeadset), None, Disconnected));
    let r = react(Disconnected, jabra::EV_REDIAL, false);
    assert_eq!((r.action, r.then), (Some(ToggleHeadset), Some(Redial)));
    // Ringing: answer or reject.
    let r = react(Ringing, jabra::EV_GO_OFF_HOOK, false);
    assert_eq!((r.action, r.state), (Some(ToggleHeadset), RingingConnecting));
    assert_eq!(react(Ringing, jabra::EV_REJECT_CALL, false).action, Some(RejectCall));
    assert_eq!(react(RingingConnecting, jabra::EV_REJECT_CALL, false).action, Some(RejectCall));
    // In a call: mute toggles only when it changes something.
    assert_eq!(react(Connected, jabra::EV_MUTE_ON, false).action, Some(ToggleMute));
    assert_eq!(react(Connected, jabra::EV_MUTE_OFF, false).action, None);
    assert_eq!(react(ConnectedMuted, jabra::EV_MUTE_OFF, false).action, Some(ToggleMute));
    assert_eq!(react(ConnectedMuted, jabra::EV_MUTE_ON, false).action, None);
    assert_eq!(react(Connected, jabra::EV_GO_ON_HOOK, false).action, Some(ToggleHeadset));
    assert_eq!(react(ConnectedMuted, jabra::EV_GO_ON_HOOK, false).action, Some(ToggleHeadset));
    // The ping is acknowledged anywhere.
    assert_eq!(react(Connected, jabra::TEST, false).answer, Some(jabra::ACK));
    // A known event in the wrong state is refused; an unknown byte, or INIT
    // while idle, starts over.
    assert_eq!(react(Disconnected, jabra::EV_MUTE_ON, false).answer, Some(jabra::NACK));
    assert_eq!(react(Connected, jabra::EV_REJECT_CALL, false).answer, Some(jabra::NACK));
    assert!(react(Connected, 0x99, false).reinit);
    assert!(react(Disconnected, jabra::INIT, false).reinit);
    assert_eq!(react(Connected, jabra::INIT, false).answer, Some(jabra::NACK));
    // With a Bluetooth headset in control nothing reaches the phone and the
    // adapter is put back on-hook.
    let r = react(Ringing, jabra::EV_GO_OFF_HOOK, true);
    assert_eq!((r.action, r.send, r.state), (None, Some(jabra::CALL_INACTIVE), Disconnected));
    let r = react(Connected, jabra::EV_MUTE_ON, true);
    assert_eq!((r.action, r.send, r.state), (None, Some(jabra::CALL_INACTIVE), Disconnected));
}

#[test]
fn jabra_model_codes() {
    assert_eq!(jabra::model_code("D40"), [5, 1, 1]);
    assert_eq!(jabra::model_code("d50"), [5, 1, 2]);
    assert_eq!(jabra::model_code("D70"), [5, 1, 3]);
    assert_eq!(jabra::model_code("D65"), [5, 1, 4]);
    assert_eq!(jabra::model_code("D99"), [0, 0, 0]);
}

#[test]
fn jabra_silence_is_pinged_and_a_dead_adapter_ends_the_loop() {
    let world: Shared = Rc::default();
    // Answers until it has seen two keepalive pings, then goes silent.
    let mut pings = 0;
    let mut inner = jabra_headset(|_, _| vec![]);
    let headset = move |w: &mut World, data: &[u8]| {
        if data == [jabra::TEST] {
            pings += 1;
        }
        // One ping belongs to initialization; two more are keepalives.
        if pings > 3 { vec![] } else { inner(w, data) }
    };
    let mut port = FakePort { world: world.clone(), rx: VecDeque::new(), headset };
    jabra::event_loop(&mut port, IDENTITY, &fast(), &mut Host(world.clone())).unwrap();
    // After the silent pings the daemon tried INIT again, four times.
    let w = world.borrow();
    assert!(w.written.ends_with(&[jabra::INIT; 4]), "{:02x?}", &w.written[w.written.len() - 10..]);
}

/// A Plantronics headset: acknowledges commands and answers the ping, and
/// lets `on_packet` add packets of its own.
fn plantronics_headset(mut on_packet: impl FnMut(&mut World, &Packet) -> Vec<Packet>) -> impl FnMut(&mut World, &[u8]) -> Vec<u8> {
    let mut parser = Parser::new();
    move |w, data| {
        let mut out = Vec::new();
        for p in parser.feed(data) {
            let p = p.expect("the phone writes well-formed frames");
            let (kind, code) = p.header;
            if matches!(kind, b'!' | b'?' | b'I') {
                out.extend(encode((b'+', code), b"").unwrap());
            }
            for extra in on_packet(w, &p) {
                out.extend(encode(extra.header, &extra.payload).unwrap());
            }
        }
        out
    }
}

fn sent_packets(written: &[u8]) -> Vec<(char, char, String)> {
    Parser::new()
        .feed(written)
        .into_iter()
        .map(|p| {
            let p = p.unwrap();
            (p.header.0 as char, p.header.1 as char, String::from_utf8(p.payload).unwrap())
        })
        .collect()
}

#[test]
fn plantronics_reset_ring_answer_and_queries() {
    let world: Shared = Rc::default();
    world.borrow_mut().status.ringing = true;
    let mut answered = false;
    let headset = plantronics_headset(move |w, p| {
        let text = |kind: u8, code: u8, payload: &[u8]| Packet { header: (kind, code), payload: payload.to_vec() };
        match (p.header, &p.payload[..]) {
            // Ringing: the user presses the button, and the headset asks
            // what the phone can do.
            ((b'!', b'A'), b"aRm") if !answered => {
                answered = true;
                vec![text(b'!', b'A', b"A"), text(b'?', b'C', b""), text(b'?', b'V', b"")]
            }
            // The call is on the headset and no longer ringing: done.
            ((b'!', b'A'), b"Arm") => {
                w.stop = true;
                vec![]
            }
            _ => vec![],
        }
    });
    let mut port = FakePort { world: world.clone(), rx: VecDeque::new(), headset };
    pehs::event_loop(&mut port, &fast(), &mut Host(world.clone())).unwrap();

    let w = world.borrow();
    assert_eq!(w.actions, [HeadsetAction::ToggleHeadset]);
    let sent = sent_packets(&w.written);
    let have = |kind: char, code: char, payload: &str| sent.iter().position(|p| *p == (kind, code, payload.to_string()));
    // Hard reset first: "!!", the ping, then the hook state (which already
    // shows the ringing that began meanwhile).
    assert_eq!(&sent[..3], [('!', '!', String::new()), ('?', 'P', String::new()), ('!', 'A', "aRm".into())]);
    // Ringing is reported, the button press acknowledged, and the queries
    // answered with the stock capability and version texts.
    let ring = have('!', 'A', "aRm").expect("ring reported");
    let ack = have('+', 'A', "").expect("button acknowledged");
    assert!(ring < ack);
    assert!(have('+', 'C', "").unwrap() < have('I', 'C', "0Adimrt").unwrap());
    assert!(have('+', 'V', "").unwrap() < have('I', 'V', "010103").unwrap());
    // Once the phone moved the call to the headset, the audio letter is up.
    assert!(sent.iter().any(|p| p.0 == '!' && p.1 == 'A' && p.2.starts_with('A')));
}

#[test]
fn plantronics_gives_up_after_two_failed_resets() {
    let world: Shared = Rc::default();
    // Nothing answers.
    let mut port = FakePort { world: world.clone(), rx: VecDeque::new(), headset: |_: &mut World, _: &[u8]| vec![] };
    pehs::event_loop(&mut port, &fast(), &mut Host(world.clone())).unwrap();
    let sent = sent_packets(&world.borrow().written);
    // The hard reset's "!!" went unanswered; the soft reset's ping too.
    assert_eq!(sent, [('!', '!', String::new()), ('?', 'P', String::new())]);
}

#[test]
fn auto_mode_finds_a_jabra_adapter() {
    let world: Shared = Rc::default();
    let mut jabra = jabra_headset(|w, cmd| {
        if cmd == jabra::RING {
            w.stop = true;
        }
        vec![]
    });
    // The adapter only understands its own line settings.
    let headset = move |w: &mut World, data: &[u8]| {
        if w.config == Some(SerialConfig::JABRA) { jabra(w, data) } else { vec![] }
    };
    world.borrow_mut().status.ringing = true;
    let mut port = FakePort { world: world.clone(), rx: VecDeque::new(), headset };
    run(&mut port, EhsMode::Auto, IDENTITY, &fast(), &mut Host(world.clone())).unwrap();
    let w = world.borrow();
    assert_eq!(w.adapters, [Some(Adapter::Jabra), None]);
    // Plantronics was probed first, three times, at its own settings.
    let probe = encode((b'?', b'P'), b"").unwrap();
    assert_eq!(&w.written[..probe.len() * 3], [probe.clone(), probe.clone(), probe].concat());
}

#[test]
fn modes_from_the_stock_setting() {
    assert_eq!(EhsMode::from_setting("plantronics"), EhsMode::Plantronics);
    assert_eq!(EhsMode::from_setting("Jabra_IQ"), EhsMode::Jabra);
    assert_eq!(EhsMode::from_setting("auto"), EhsMode::Auto);
    assert_eq!(EhsMode::from_setting(""), EhsMode::Auto);
}
