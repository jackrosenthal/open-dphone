//! What the app interface promises an app: refusals that are reported,
//! events that arrive once and with the right content, and one app's
//! doings that leave another app's alone.  The test guest `checks` does
//! what a test asks and shows what the phone answered.

mod common;

use common::{Rig, staged};
use dphone_apps::testutil::FakeConfig;
use dphone_state::{AccountId, Call, CallId, CallState, Command, Direction, EndCause, Event, Key, Model, Party};

fn rig() -> Rig {
    let mut r = Rig::new(FakeConfig::default());
    r.launch("checks");
    r
}

/// Asks the guest to do something; returns what the guest answered.
fn ask(r: &mut Rig, what: &str, arg: &str) -> String {
    r.store.emit(Event::App { app: "checks".into(), name: what.into(), payload: arg.into() });
    r.settle();
    r.ui.texts("checks")[0].clone()
}

/// The calls and pushes the guest was told of.
fn seen(r: &Rig) -> String {
    r.ui.texts("checks")[1].clone()
}

/// The commands the phone got, once the threads that answer them have
/// caught up.
fn commands(r: &Rig) -> Vec<Command> {
    std::thread::sleep(std::time::Duration::from_millis(30));
    r.commands()
}

fn hangups(r: &Rig) -> Vec<u32> {
    commands(r).iter().filter_map(|c| if let Command::Hangup { call } = c { Some(call.0) } else { None }).collect()
}

fn call(id: u32, hidden: bool) -> Call {
    let mut call = Call::new(CallId(id), AccountId(0), Direction::Incoming, Party::default());
    call.hidden = hidden;
    call
}

#[test]
fn a_limit_that_is_reached_is_reported() {
    let mut r = rig();
    let timers = ask(&mut r, "timers", "");
    assert!(timers.starts_with("32 then ") && timers.contains("Failed("), "{timers}");
    let observed = ask(&mut r, "observe", "");
    assert!(observed.starts_with("64 then ") && observed.contains("Failed("), "{observed}");
    assert_eq!(ask(&mut r, "observe-again", ""), "Ok(())");
    let watched = ask(&mut r, "subscribe", "");
    assert!(watched.starts_with("256 then ") && watched.contains("Failed("), "{watched}");
}

#[test]
fn a_drawing_for_a_widget_that_is_no_canvas_is_refused() {
    let mut r = rig();
    assert_eq!(
        ask(&mut r, "draw", ""),
        r#"canvas=Ok(()) text=Err("not a canvas") dead=Err("not a canvas") window=Err("not a canvas")"#
    );
}

#[test]
fn content_that_is_no_picture_is_refused_when_defined() {
    let mut r = rig();
    let answer = ask(&mut r, "pictures", "");
    assert!(answer.starts_with("good=Ok(()) bad=Err(Error::Invalid("), "{answer}");
    assert!(answer.contains(" again=Err(Error::Invalid("), "{answer}");
    // The window has the one picture, as first defined.
    let pic = std::fs::metadata(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/guests/probe/data/pic.png")).unwrap();
    assert_eq!(r.ui.images("checks"), [("good".to_string(), pic.len() as usize)]);
}

#[test]
fn showing_an_app_the_phone_does_not_offer_fails() {
    let config = FakeConfig::default();
    config.disabled.lock().unwrap().push("status".into());
    let mut r = Rig::new(config);
    r.launch("checks");
    let missing = ask(&mut r, "show", "nosuch");
    assert!(missing.starts_with("Err(") && missing.contains("no app named"), "{missing}");
    let off = ask(&mut r, "show", "status");
    assert!(off.starts_with("Err(") && off.contains("switched off"), "{off}");
    assert_eq!(r.ui.top().as_deref(), Some("checks"));
    assert_eq!(ask(&mut r, "show", "calendar"), "Ok(())");
    assert_eq!(r.ui.top().as_deref(), Some("calendar"));
}

#[test]
fn a_message_for_an_app_that_does_not_run_fails() {
    let mut r = rig();
    let stopped = ask(&mut r, "send", "probe");
    assert!(stopped.starts_with("Err(") && stopped.contains("not running"), "{stopped}");
    let missing = ask(&mut r, "send", "nosuch");
    assert!(missing.starts_with("Err("), "{missing}");
    r.launch("probe");
    assert_eq!(ask(&mut r, "send", "probe"), "Ok(())");
    // An app that runs may write to itself.
    assert_eq!(ask(&mut r, "send", "checks"), "unknown: hello");
}

#[test]
fn the_lamps_are_those_the_model_has() {
    let mut r = rig();
    assert_eq!(ask(&mut r, "lamps", ""), "line1,line2,message,mute,hdst,spkr");
    let mut r = Rig::with_root(staged(), FakeConfig::default(), Model::D65);
    r.launch("checks");
    assert_eq!(ask(&mut r, "lamps", ""), "line1,line2,line3,line4,line5,line6,message,mute,hdst,spkr");
}

#[test]
fn a_key_too_long_to_store_is_a_bad_argument() {
    let mut r = rig();
    let answer = ask(&mut r, "long-key", "300");
    assert!(answer.starts_with("Err(Error::Invalid("), "{answer}");
    assert_eq!(ask(&mut r, "long-key", "40"), "Ok(())");
}

#[test]
fn a_call_change_arrives_once() {
    let mut r = rig();
    ask(&mut r, "calls", "");
    r.store.update::<dphone_state::Calls, _>(|c| c.calls.push(call(9, false)));
    r.settle();
    assert_eq!(seen(&r), "call 9 ringing");
    r.store.update::<dphone_state::Calls, _>(|c| c.calls[0].state = CallState::Active);
    r.settle();
    assert_eq!(seen(&r), "call 9 ringing; call 9 active");

    // What an app does not see of a call is no change for the app.
    r.store.update::<dphone_state::Calls, _>(|c| {
        c.calls[0].muted = true;
        c.calls[0].srtp = true;
        c.calls[0].sip_call_id = "abc@example".into();
    });
    r.settle();
    assert_eq!(seen(&r), "call 9 ringing; call 9 active");

    // The end: the call's record says so, and the phone's event follows.
    let mut ended = None;
    r.store.update::<dphone_state::Calls, _>(|c| {
        c.calls[0].state = CallState::Ended(EndCause::RemoteHangup);
        ended = Some(c.calls[0].clone());
    });
    r.store.emit(Event::CallEnded { call: ended.unwrap() });
    r.settle();
    r.store.update::<dphone_state::Calls, _>(|c| c.calls.clear());
    r.settle();
    assert_eq!(seen(&r), "call 9 ringing; call 9 active; call 9 ended");
}

#[test]
fn a_push_from_the_pbx_keeps_its_payload_whatever_is_in_it() {
    let mut r = rig();
    assert_eq!(ask(&mut r, "watch", "example.event"), "Ok(())");
    // A payload that looks like the description of a call is a payload.
    let payload = r#"{"method":"refresh","call":5,"data":"inner","type":"t"}"#;
    r.store.emit(Event::Push { name: "example.event".into(), body: payload.into() });
    r.settle();
    assert_eq!(seen(&r), format!("push example.event kind=\"\" data={payload:?} method=MESSAGE call=None"));

    // A push that arrives with a call says so.
    r.store.emit(Event::CallPush {
        name: "example.event".into(),
        kind: "example.kind".into(),
        data: "with a call".into(),
        method: "INFO".into(),
        call: CallId(5),
    });
    r.settle();
    assert!(
        seen(&r).ends_with("; push example.event kind=\"example.kind\" data=\"with a call\" method=INFO call=Some(5)"),
        "{}",
        seen(&r)
    );
}

fn hidden_call_arrives(r: &mut Rig, id: u32) {
    r.store.update::<dphone_state::Calls, _>(|c| c.calls.push(call(id, true)));
    r.store.emit(Event::CallPush {
        name: "digium.incomingCall.example".into(),
        kind: String::new(),
        data: String::new(),
        method: "INVITE".into(),
        call: CallId(id),
    });
    r.settle();
}

#[test]
fn a_hidden_call_ends_with_the_app_that_got_it() {
    let mut r = rig();
    assert_eq!(ask(&mut r, "watch", "digium.incomingCall.example"), "Ok(())");
    // The app is off the screen when its call arrives.
    r.key("checks", Key::Cancel);
    assert!(r.mgr.is_running("checks"));
    assert_eq!(r.ui.top(), None);
    hidden_call_arrives(&mut r, 21);
    assert!(seen(&r).contains("call=Some(21)"), "{}", seen(&r));

    // Another app comes and goes: the call is not that app's to end.
    r.launch("calendar");
    r.key("calendar", Key::Cancel);
    assert_eq!(r.ui.top(), None);
    assert!(!r.mgr.is_running("calendar"));
    assert_eq!(hangups(&r), [] as [u32; 0]);

    // The app that got the call shows itself, and leaves: the call ends.
    r.launch("checks");
    assert_eq!(hangups(&r), [] as [u32; 0]);
    ask(&mut r, "leave", "");
    assert_eq!(r.ui.top(), None);
    assert_eq!(hangups(&r), [21]);
}

#[test]
fn a_hidden_call_ends_when_its_app_stops() {
    let mut r = rig();
    assert_eq!(ask(&mut r, "watch", "digium.incomingCall.example"), "Ok(())");
    r.key("checks", Key::Cancel);
    hidden_call_arrives(&mut r, 22);
    r.mgr.close("checks");
    assert_eq!(hangups(&r), [22]);
}

#[test]
fn a_hidden_call_no_app_got_ends_when_the_app_on_screen_leaves() {
    let mut r = rig();
    // No app observes the event of this call.
    hidden_call_arrives(&mut r, 23);
    assert_eq!(hangups(&r), [] as [u32; 0]);
    r.key("checks", Key::Cancel);
    assert_eq!(hangups(&r), [23]);
}

#[test]
fn an_app_stops_only_the_sound_the_app_started() {
    use dphone_state::{Domain, Reply};
    use std::sync::{Arc, Mutex};
    let mut r = rig();
    let audio = Arc::new(Mutex::new(Vec::new()));
    common::answer_commands(&r.store, Domain::Audio, audio.clone(), |_| Ok(Reply::Done));
    // What the audio side was asked since the last look, by kind.
    let asked = |audio: &Arc<Mutex<Vec<Command>>>| -> Vec<&'static str> {
        std::thread::sleep(std::time::Duration::from_millis(30));
        audio
            .lock()
            .unwrap()
            .drain(..)
            .map(|c| match c {
                Command::PlayData { .. } => "play",
                Command::PlaySound { .. } => "preview",
                Command::StopSound => "stop",
                _ => "other",
            })
            .collect()
    };

    // The app plays nothing: whatever sounds (another app's sound, a
    // ringtone preview of the settings) is not the app's to stop.
    ask(&mut r, "stop", "");
    ask(&mut r, "preview", "");
    assert_eq!(asked(&audio), [] as [&str; 0]);

    // The app's sound is the app's to stop.
    assert_eq!(ask(&mut r, "play", ""), "Ok(())");
    ask(&mut r, "stop", "");
    assert_eq!(asked(&audio), ["play", "stop"]);
    // The sound is over (here: another sound took its place); stopping
    // again would stop that other sound.
    r.store.emit(Event::PlaybackStopped { app: "checks".into(), position_secs: 1 });
    r.settle();
    assert_eq!(seen(&r), "stopped 1");
    ask(&mut r, "stop", "");
    assert_eq!(asked(&audio), [] as [&str; 0]);

    // The preview the app started is the app's to stop, once.
    ask(&mut r, "preview", "Classic");
    ask(&mut r, "preview", "");
    ask(&mut r, "preview", "");
    assert_eq!(asked(&audio), ["preview", "stop"]);
}
