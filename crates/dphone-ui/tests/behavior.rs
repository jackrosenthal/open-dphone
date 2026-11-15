//! Behavior tests: key dispatch, hook handling, navigation, softkey
//! paging, the app view contract, lamps, backlight, and damage tracking,
//! all through the HAL simulator.

use dphone_hal::{Hal, Lamp, Screen};
use dphone_state::*;
use dphone_ui::appview::{self as av, AppEvent, AppView, KeyClaims, WidgetKind};
use dphone_ui::{Ui, UiConfig};

struct Rig {
    store: Store,
    ui: Ui,
    sim: dphone_hal::sim::SimHandle,
    calls: CommandReceiver,
    audio: CommandReceiver,
    apps: CommandReceiver,
    config: CommandReceiver,
}

impl Rig {
    fn new(model: Model) -> Rig {
        let mut hal = Hal::sim(model);
        let sim = hal.sim.clone().unwrap();
        let store = Store::new(model);
        store.update::<System, _>(|s| s.phase = Phase::Running);
        let calls = store.handle(Domain::Calls);
        let audio = store.handle(Domain::Audio);
        let apps = store.handle(Domain::Apps);
        let config = store.handle(Domain::Config);
        let mut ui = Ui::from_hal(&mut hal, store.clone(), UiConfig { backlight_timeout_secs: 0, ..Default::default() });
        ui.pump();
        Rig { store, ui, sim, calls, audio, apps, config }
    }
    fn tap(&mut self, key: Key) {
        self.sim.tap(key);
        self.ui.pump();
    }
    fn hook(&mut self, off: bool) {
        self.sim.set_hook(off);
        self.ui.pump();
    }
    fn drain(rx: &CommandReceiver) -> Vec<Command> {
        let mut v = Vec::new();
        while let Some(env) = rx.try_recv() {
            v.push(env.command.clone());
            env.done();
        }
        v
    }
    fn call_cmds(&self) -> Vec<Command> {
        Self::drain(&self.calls)
    }
    fn audio_cmds(&self) -> Vec<Command> {
        Self::drain(&self.audio)
    }
    fn add_call(&mut self, state: CallState) -> CallId {
        let id = self.store.next_call_id();
        self.store.update::<Calls, _>(|c| {
            let mut call = Call::new(id, AccountId(0), Direction::Incoming, Party { number: "1002".into(), ..Default::default() });
            call.state = state;
            c.calls.push(call);
            c.focused = Some(id);
        });
        self.ui.pump();
        id
    }
    fn screen(&self) -> String {
        self.ui.screen_name()
    }
}

#[test]
fn on_hook_dialing() {
    let mut r = Rig::new(Model::D60);
    assert_eq!(r.screen(), "idle");
    for d in [1, 0, 0, 2] {
        r.tap(Key::Digit(d));
    }
    assert_eq!(r.screen(), "dialer");
    assert_eq!(r.store.get::<dphone_state::Ui>().dial_buffer, "1002");
    // On-hook digits play no tone and do not dial by themselves.
    assert!(r.audio_cmds().is_empty());
    assert!(r.call_cmds().is_empty());
    r.tap(Key::Left); // backspace
    r.tap(Key::Soft(1)); // Dial
    assert_eq!(r.call_cmds(), vec![Command::Dial { target: "100".into(), account: None, path: None }]);
    assert_eq!(r.screen(), "idle");
    assert_eq!(r.store.get::<dphone_state::Ui>().screen, "idle");
    // Events were re-emitted for other listeners.
}

#[test]
fn off_hook_dialing_times_out_into_a_call() {
    let mut r = Rig::new(Model::D40);
    r.hook(true);
    assert_eq!(r.screen(), "dialer");
    assert_eq!(
        r.audio_cmds(),
        vec![
            Command::SetAudioPath { path: AudioPath::Handset },
            Command::PlayTone { tone: Tone::Dial, duration_ms: 0 }
        ]
    );
    r.ui.set_fixed_time(Some((1_000_000, 1_794_607_087)));
    r.tap(Key::Digit(7));
    let a = r.audio_cmds();
    assert_eq!(a[0], Command::StopTone);
    assert_eq!(a[1], Command::PlayTone { tone: Tone::Dtmf('7'), duration_ms: 100 });
    // Before the inter-digit timeout nothing is dialed.
    r.ui.set_fixed_time(Some((1_003_000, 1_794_607_090)));
    r.ui.pump();
    assert!(r.call_cmds().is_empty());
    r.ui.set_fixed_time(Some((1_004_100, 1_794_607_091)));
    r.ui.pump();
    assert_eq!(
        r.call_cmds(),
        vec![Command::Dial { target: "7".into(), account: None, path: Some(AudioPath::Handset) }]
    );
    assert_eq!(r.screen(), "idle");
}

#[test]
fn speaker_and_headset_keys_go_off_hook_on_their_path() {
    for (key, path) in [(Key::Speaker, AudioPath::Speaker), (Key::Headset, AudioPath::Headset)] {
        let mut r = Rig::new(Model::D60);
        r.tap(key);
        assert_eq!(r.screen(), "dialer", "{key:?}");
        assert_eq!(
            r.audio_cmds(),
            vec![Command::SetAudioPath { path }, Command::PlayTone { tone: Tone::Dial, duration_ms: 0 }],
            "{key:?}"
        );
    }
}

#[test]
fn hanging_up_an_empty_dialer_closes_it() {
    let mut r = Rig::new(Model::D60);
    r.hook(true);
    r.audio_cmds();
    r.hook(false);
    assert_eq!(r.screen(), "idle");
    assert_eq!(r.audio_cmds(), vec![Command::StopTone, Command::SetAudioPath { path: AudioPath::None }]);
}

#[test]
fn incoming_call_keys() {
    let mut r = Rig::new(Model::D60);
    // Bury the idle screen under a menu: a ringing call still comes up.
    r.tap(Key::Select);
    assert_eq!(r.screen(), "menu");
    let id = r.add_call(CallState::Ringing);
    assert_eq!(r.screen(), "call");
    r.tap(Key::Soft(2)); // Ignore
    assert_eq!(r.call_cmds(), vec![Command::Silence { call: id }]);
    r.tap(Key::Cancel); // stock: Cancel rejects
    assert_eq!(r.call_cmds(), vec![Command::Reject { call: id, busy: true }]);
    r.tap(Key::Speaker);
    assert_eq!(r.call_cmds(), vec![Command::Answer { call: id, path: Some(AudioPath::Speaker) }]);
    r.hook(true);
    assert_eq!(r.call_cmds(), vec![Command::Answer { call: id, path: Some(AudioPath::Handset) }]);
    // Stock layout: Answer, Ignore, Transfer, and an empty fourth key.
    r.tap(Key::Soft(4));
    assert!(r.call_cmds().is_empty());
    assert_eq!(r.screen(), "call");
    r.tap(Key::Soft(3)); // Transfer the ringing call
    assert_eq!(r.screen(), "dialer");
    r.tap(Key::Cancel);
    assert_eq!(r.screen(), "call");
    // The call ends: back to where the user was (the menu was replaced by
    // the call; the stack below the call screen is kept).
    r.store.update::<Calls, _>(|c| c.calls.clear());
    r.ui.pump();
    assert_eq!(r.screen(), "menu");
}

#[test]
fn active_call_keys() {
    let mut r = Rig::new(Model::D50);
    let id = r.add_call(CallState::Active);
    r.store.update::<Audio, _>(|a| a.path = AudioPath::Handset);
    r.hook(true);
    r.call_cmds();
    r.audio_cmds();
    r.tap(Key::Digit(5));
    assert_eq!(r.call_cmds(), vec![Command::Dtmf { call: id, digit: '5' }]);
    r.tap(Key::Hold);
    assert_eq!(r.call_cmds(), vec![Command::Hold { call: id }]);
    r.tap(Key::Mute);
    assert_eq!(r.audio_cmds(), vec![Command::SetMute { on: true }]);
    r.tap(Key::VolumeUp);
    assert_eq!(r.audio_cmds(), vec![Command::AdjustVolume { target: VolumeTarget::Current, delta: 1 }]);
    r.tap(Key::Speaker);
    assert_eq!(r.audio_cmds(), vec![Command::SetAudioPath { path: AudioPath::Speaker }]);
    // Transfer opens the transfer dialer; its first softkey is a blind
    // transfer.
    r.tap(Key::Transfer);
    assert_eq!(r.screen(), "dialer");
    r.tap(Key::Digit(9));
    r.tap(Key::Soft(1));
    assert_eq!(r.call_cmds(), vec![Command::TransferBlind { call: id, target: "9".into() }]);
    assert_eq!(r.screen(), "call");
    // With a second call, the Conference key joins them.
    let second = r.add_call(CallState::Active);
    r.store.update::<Calls, _>(|c| c.get_mut(id).unwrap().state = CallState::Held { local: true, remote: false });
    r.ui.pump();
    r.tap(Key::Conference);
    assert_eq!(r.call_cmds(), vec![Command::Conference { a: second, b: id }]);
    r.tap(Key::Transfer);
    assert_eq!(r.call_cmds(), vec![Command::TransferAttended { call: id, consult: second }]);
    // Hanging up the handset ends the call on the handset.
    r.hook(false);
    assert_eq!(r.call_cmds(), vec![Command::Hangup { call: second }]);
    // On the speaker, the hook does nothing and the speaker key hangs up.
    r.store.update::<Audio, _>(|a| a.path = AudioPath::Speaker);
    r.hook(true);
    r.audio_cmds();
    r.hook(false);
    assert!(r.call_cmds().is_empty());
    r.tap(Key::Speaker);
    assert_eq!(r.call_cmds(), vec![Command::Hangup { call: second }]);
}

#[test]
fn menu_shortcuts_and_settings() {
    let mut r = Rig::new(Model::D60);
    // Idle softkeys page behind "More...": Call Log, Contacts, Status,
    // then Menu, Apps, DND.
    r.tap(Key::Soft(4));
    r.tap(Key::Soft(1)); // Menu
    assert_eq!(r.screen(), "menu");
    r.tap(Key::Digit(4)); // Do Not Disturb toggles in place
    assert_eq!(r.call_cmds(), vec![Command::SetDnd { on: true }]);
    r.tap(Key::Digit(3)); // Call Forward
    assert_eq!(r.screen(), "forward");
    r.tap(Key::Select); // Always
    for d in [1, 0, 9, 9] {
        r.tap(Key::Digit(d));
    }
    r.tap(Key::Select);
    assert_eq!(
        r.call_cmds(),
        vec![Command::SetForward { kind: ForwardKind::Always, target: "1099".into(), no_answer_secs: None }]
    );
    assert_eq!(r.screen(), "forward");
    r.tap(Key::Cancel);
    r.tap(Key::Cancel);
    assert_eq!(r.screen(), "idle");
    // Settings write stock setting IDs.
    r.tap(Key::Select);
    r.tap(Key::Digit(6)); // Settings
    r.tap(Key::Digit(1)); // Network
    r.tap(Key::Digit(1)); // Addressing: DHCP -> static
    let cmds = Rig::drain(&r.config);
    assert_eq!(cmds, vec![Command::SetSettings { settings: vec![("network_enable_dhcp".into(), "0".into())] }]);
    // Brightness adjusts live and is persisted when the screen closes.
    let p = r.store.send(Command::ShowScreen { name: "settings".into() });
    r.ui.pump();
    p.wait().unwrap();
    r.tap(Key::Digit(3));
    r.tap(Key::Digit(1));
    r.tap(Key::Right);
    r.ui.pump();
    assert_eq!(r.sim.brightness(Screen::Main), Some(20)); // the stock level of brightness 6
    assert!(Rig::drain(&r.config).is_empty());
    r.tap(Key::Cancel);
    assert_eq!(Rig::drain(&r.config), vec![Command::SetSettings { settings: vec![("brightness".into(), "6".into())] }]);
}

#[test]
fn injected_keys_and_events() {
    let mut r = Rig::new(Model::D60);
    let sub = r.store.subscribe_events(TopicSet::NONE);
    let p = r.store.send(Command::InjectKey { key: Key::Digit(4), down: true });
    r.ui.pump();
    assert_eq!(p.wait(), Ok(Reply::Done));
    assert_eq!(r.screen(), "dialer");
    let wake = sub.try_take().unwrap();
    assert!(wake.events.contains(&Event::Key { key: Key::Digit(4), down: true, source: InputSource::Injected }));
    let p = r.store.send(Command::ShowScreen { name: "nonsense".into() });
    r.ui.pump();
    assert!(matches!(p.wait(), Err(CommandError::Invalid(_))));
}

#[test]
fn messages_key_prefers_the_voicemail_app() {
    let mut r = Rig::new(Model::D60);
    r.tap(Key::Messages);
    assert_eq!(r.call_cmds(), vec![Command::DialVoicemail { account: None }]);
    r.store.update::<Apps, _>(|a| a.installed.push(AppInfo { name: "voicemail".into(), ..Default::default() }));
    r.tap(Key::Messages);
    assert_eq!(Rig::drain(&r.apps), vec![Command::LaunchApp { name: "voicemail".into() }]);
}

#[test]
fn rapid_dial_keys_dial_or_pick_up() {
    let mut r = Rig::new(Model::D70);
    r.store.update::<Presence, _>(|p| {
        p.entries.push(BlfEntry { id: "a".into(), number: "201".into(), state: BlfState::Idle, ..Default::default() });
        // Both ring; only "b" is a contact whose calls may be picked up.
        p.entries.push(BlfEntry {
            id: "b".into(),
            number: "202".into(),
            state: BlfState::Ringing,
            pickup_ringing: true,
            ..Default::default()
        });
        p.entries.push(BlfEntry { id: "c".into(), number: "203".into(), state: BlfState::Ringing, ..Default::default() });
    });
    r.store.update::<Keys, _>(|k| {
        k.rapid_pages = 2;
        for (i, e) in ["a", "b", "c"].into_iter().enumerate() {
            k.rapid_dial.push(RapidDialKey {
                index: i as u8,
                page: 0,
                entry: e.into(),
                label: e.into(),
                number: String::new(),
                lamp: LampState::solid(LampColor::Green),
            });
        }
    });
    r.ui.pump();
    r.tap(Key::Blf(1));
    r.tap(Key::Blf(2));
    r.tap(Key::Blf(3));
    r.tap(Key::BlfPageRight);
    assert_eq!(
        r.call_cmds(),
        vec![
            Command::Dial { target: "201".into(), account: None, path: None },
            Command::Pickup { blf_entry: "b".into() },
            Command::Dial { target: "203".into(), account: None, path: None },
            Command::SetRapidDialPage { page: 1 },
        ]
    );
    // Lamps follow the key state; the side display was drawn.
    assert_eq!(r.sim.lamp(Lamp::Blf(0)), LampColor::Green);
    assert_eq!(r.sim.lamp(Lamp::Blf(5)), LampColor::Off);
    assert!(r.sim.frame(Screen::Side).is_some());
}

#[test]
fn lamps_follow_key_state() {
    let mut r = Rig::new(Model::D60);
    r.store.update::<Keys, _>(|k| {
        k.line_keys.push(LineKey {
            index: 0,
            binding: LineKeyBinding::Account(AccountId(0)),
            label: "100".into(),
            lamp: LampState::solid(LampColor::Green),
            calls: vec![],
        });
        k.message_lamp = LampState::solid(LampColor::Red);
        k.mute_lamp = LampState::solid(LampColor::Red);
    });
    r.ui.pump();
    assert_eq!(r.sim.lamp(Lamp::Line(0)), LampColor::Green);
    assert_eq!(r.sim.lamp(Lamp::Message), LampColor::Red);
    assert_eq!(r.sim.lamp(Lamp::Mute), LampColor::Red);
    assert_eq!(r.sim.lamp(Lamp::Speaker), LampColor::Off);
    r.store.update::<Keys, _>(|k| k.message_lamp = LampState::OFF);
    r.ui.pump();
    assert_eq!(r.sim.lamp(Lamp::Message), LampColor::Off);
}

#[test]
fn app_view_contract() {
    let mut r = Rig::new(Model::D60);
    let (view, events) = AppView::new("demo", Model::D60);
    let m = view.metrics();
    assert_eq!((m.width, m.height, m.row_height), (480, 243, 32));
    let group = view.create(WidgetKind::Group, None, av::Rect::new(0, 0, m.width, m.height));
    let list = view.create(WidgetKind::List, Some(group), av::Rect::new(0, 0, m.width, 100));
    view.list_set_rows(list, (0..5).map(|i| vec![av::Cell::text(format!("row {i}"))]).collect());
    let input = view.create(WidgetKind::Input, Some(group), av::Rect::new(0, 110, m.width, 36));
    view.focus(Some(list));
    view.set_softkeys(None, (1..=6).map(|i| av::Softkey::new(i, format!("A{i}"))).collect());
    view.set_softkeys(Some(group), vec![av::Softkey::new(100, "Group")]);
    r.ui.handle().open_app(view.clone());
    r.ui.pump();
    assert_eq!(r.screen(), "app:demo");
    assert_eq!(r.store.get::<dphone_state::Ui>().screen, "app:demo");
    assert_eq!(events.try_recv(), Some(AppEvent::Foreground));

    // List navigation and activation.
    r.tap(Key::Down);
    r.tap(Key::Select);
    assert_eq!(events.try_recv(), Some(AppEvent::ListSelected { widget: list, row: 1 }));
    assert_eq!(events.try_recv(), Some(AppEvent::ListActivated { widget: list, row: 1 }));
    r.tap(Key::Up);
    r.tap(Key::Up); // wraps
    assert_eq!(view.list_selected(list), Some(4));
    while events.try_recv().is_some() {}

    // Softkeys resolve by walking up from the focused widget: the group's
    // set wins over the window's.
    r.tap(Key::Soft(1));
    assert_eq!(events.try_recv(), Some(AppEvent::Softkey { id: 100 }));
    // Without the group's set, the window's six actions page behind More.
    view.set_softkeys(Some(group), vec![]);
    r.tap(Key::Soft(3));
    assert_eq!(events.try_recv(), Some(AppEvent::Softkey { id: 3 }));
    r.tap(Key::Soft(4)); // More
    r.tap(Key::Soft(1));
    assert_eq!(events.try_recv(), Some(AppEvent::Softkey { id: 4 }));

    // Claims: the group claims Right; the window claims the digits.
    view.set_key_claims(group, KeyClaims::keys([Key::Right]));
    view.set_window_key_claims(KeyClaims { digits: true, ..Default::default() });
    r.tap(Key::Right);
    r.tap(Key::Digit(3));
    assert_eq!(events.try_recv(), Some(AppEvent::Key { widget: Some(group), key: Key::Right }));
    assert_eq!(events.try_recv(), Some(AppEvent::Key { widget: None, key: Key::Digit(3) }));
    view.set_window_key_claims(KeyClaims::none());

    // Input: multi-tap text, softkey 3 deletes, Select submits.
    view.focus(Some(input));
    view.set_softkeys(None, vec![av::Softkey::new(1, "OK")]);
    r.tap(Key::Digit(2));
    r.tap(Key::Digit(2)); // a -> b
    r.tap(Key::Digit(3)); // d
    assert_eq!(view.text(input).as_deref(), Some("bd"));
    r.tap(Key::Soft(3)); // Delete
    assert_eq!(view.text(input).as_deref(), Some("b"));
    r.tap(Key::Select);
    let got: Vec<AppEvent> = std::iter::from_fn(|| events.try_recv()).collect();
    assert_eq!(
        got,
        vec![
            AppEvent::InputChanged { widget: input, value: "a".into() },
            AppEvent::InputChanged { widget: input, value: "b".into() },
            AppEvent::InputChanged { widget: input, value: "bd".into() },
            AppEvent::InputChanged { widget: input, value: "b".into() },
            AppEvent::InputSubmitted { widget: input, value: "b".into() },
        ]
    );
    // Up from the input moves focus to the list.
    r.tap(Key::Up);
    assert_eq!(view.focused(), Some(list));
    assert_eq!(events.try_recv(), Some(AppEvent::Focused { widget: list }));

    // Unclaimed Cancel asks the app to close; volume keys never reach it.
    r.tap(Key::Cancel);
    assert_eq!(events.try_recv(), Some(AppEvent::CloseRequested));
    r.tap(Key::VolumeDown);
    assert_eq!(events.try_recv(), None);
    assert_eq!(r.audio_cmds(), vec![Command::AdjustVolume { target: VolumeTarget::Current, delta: -1 }]);

    // An incoming call covers the app; ending it brings the app back.
    let _ = r.add_call(CallState::Ringing);
    assert_eq!(r.screen(), "call");
    assert_eq!(events.try_recv(), Some(AppEvent::Background));
    r.store.update::<Calls, _>(|c| c.calls.clear());
    r.ui.pump();
    assert_eq!(r.screen(), "app:demo");
    assert_eq!(events.try_recv(), Some(AppEvent::Foreground));

    // Line 1 backgrounds a nonprivileged app (stock rule).
    r.tap(Key::Line(1));
    assert_eq!(r.screen(), "idle");
    assert_eq!(events.try_recv(), Some(AppEvent::Background));

    // Destroying a widget destroys its children and drops focus.
    view.destroy(group);
    assert!(!view.exists(list) && !view.exists(input));
    assert_eq!(view.focused(), None);
}

#[test]
fn damage_is_tracked() {
    let mut r = Rig::new(Model::D40);
    r.ui.pump();
    let g0 = r.sim.frame_generation();
    // Nothing changed: no flush.
    r.ui.pump();
    assert_eq!(r.sim.frame_generation(), g0);
    // A state change that alters pixels flushes once.
    r.store.update::<Features, _>(|f| f.dnd = true);
    r.ui.pump();
    assert!(r.sim.frame_generation() > g0);
    // The gray panel only ever receives the four levels.
    let frame = r.sim.frame(Screen::Main).unwrap();
    assert!(frame.pixels.iter().all(|p| matches!(p, 0x00 | 0x55 | 0xAA | 0xFF)));
}

#[test]
fn backlight_dims_and_wakes() {
    let model = Model::D60;
    let mut hal = Hal::sim(model);
    let sim = hal.sim.clone().unwrap();
    let store = Store::new(model);
    store.update::<System, _>(|s| s.phase = Phase::Running);
    let mut ui = Ui::from_hal(&mut hal, store.clone(), UiConfig { backlight_timeout_secs: 1, brightness: 10, ..Default::default() });
    ui.pump();
    assert_eq!(sim.brightness(Screen::Main), Some(32));
    assert!(store.get::<dphone_state::Ui>().backlight_on);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    ui.pump();
    assert_eq!(sim.brightness(Screen::Main), Some(6)); // the stock level of dim level 2
    sim.tap(Key::Up);
    ui.pump();
    assert_eq!(sim.brightness(Screen::Main), Some(32));
    // Dim level 0 is off.
    ui.handle().set_config(UiConfig { backlight_timeout_secs: 1, brightness: 10, backlight_dim_level: 0, ..Default::default() });
    ui.pump();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    ui.pump();
    assert_eq!(sim.brightness(Screen::Main), Some(0));
    assert!(!store.get::<dphone_state::Ui>().backlight_on);
    // With dimming off the backlight is lit again and stays lit, as in stock.
    ui.handle().set_config(UiConfig { backlight_timeout_secs: 1, brightness: 10, dim_backlight: false, ..Default::default() });
    ui.pump();
    assert_eq!(sim.brightness(Screen::Main), Some(32));
    std::thread::sleep(std::time::Duration::from_millis(1100));
    ui.pump();
    assert_eq!(sim.brightness(Screen::Main), Some(32));
    assert!(store.get::<dphone_state::Ui>().backlight_on);
    // A ringing call wakes a dimmed backlight.
    ui.handle().set_config(UiConfig { backlight_timeout_secs: 1, brightness: 10, ..Default::default() });
    ui.pump();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    ui.pump();
    assert_eq!(sim.brightness(Screen::Main), Some(6));
    store.update::<Calls, _>(|c| {
        let mut call = Call::new(CallId(1), AccountId(0), Direction::Incoming, Party::default());
        call.state = CallState::Ringing;
        c.calls.push(call);
    });
    ui.pump();
    assert_eq!(sim.brightness(Screen::Main), Some(32));
}

#[test]
fn clock_and_call_timer_repaint_on_their_own() {
    let mut r = Rig::new(Model::D60);
    // 21:58:07 UTC: the idle screen shows 9:58.
    r.ui.set_fixed_time(Some((1_000_000, 1_794_607_087)));
    r.ui.pump();
    let g0 = r.sim.frame_generation();
    // Same minute: nothing to repaint.
    r.ui.set_fixed_time(Some((1_010_000, 1_794_607_097)));
    r.ui.pump();
    let before = r.sim.frame(Screen::Main).unwrap();
    // The next minute: the clock changes without any input.
    r.ui.set_fixed_time(Some((1_060_000, 1_794_607_147)));
    r.ui.pump();
    let after = r.sim.frame(Screen::Main).unwrap();
    assert!(r.sim.frame_generation() > g0);
    assert_ne!(before.pixels, after.pixels);
}

#[test]
fn run_loop_on_its_own_thread() {
    let model = Model::D60;
    let mut hal = Hal::sim(model);
    let sim = hal.sim.clone().unwrap();
    let store = Store::new(model);
    store.update::<System, _>(|s| s.phase = Phase::Running);
    let ui = Ui::from_hal(&mut hal, store.clone(), UiConfig::default());
    let handle = ui.handle();
    let thread = std::thread::spawn(move || ui.run());
    let wait_screen = |want: &str| {
        for _ in 0..200 {
            if store.get::<dphone_state::Ui>().screen == want {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("screen never became {want:?}: {:?}", store.get::<dphone_state::Ui>().screen);
    };
    wait_screen("idle");
    // A physical key through the blocking input thread.
    sim.tap(Key::Digit(1));
    wait_screen("dialer");
    // A command through the command thread.
    assert_eq!(store.send(Command::ShowScreen { name: "menu".into() }).wait(), Ok(Reply::Done));
    wait_screen("menu");
    // A state change.
    store.update::<Calls, _>(|c| {
        let mut call = Call::new(CallId(7), AccountId(0), Direction::Incoming, Party::default());
        call.state = CallState::Ringing;
        c.calls.push(call);
    });
    wait_screen("call");
    assert!(store.last_heartbeat(Subsystem::Ui).is_some());
    handle.quit();
    thread.join().unwrap();
}

#[test]
fn paging_screen_pages_a_group_and_stops_a_page() {
    let mut r = Rig::new(Model::D60);
    let paging = r.store.handle(Domain::Paging);
    r.store.update::<Paging, _>(|p| {
        p.broadcasts = vec![PageGroup { id: "dock".into(), name: "Loading dock".into(), addr: "239.1.1.1".into(), port: 5000 }];
    });
    let shown = r.store.send(Command::ShowScreen { name: "paging".into() });
    r.ui.pump();
    assert_eq!(shown.wait(), Ok(Reply::Done));
    assert_eq!(r.screen(), "paging");
    // Selecting the group pages it.
    r.tap(Key::Select);
    assert_eq!(Rig::drain(&paging), [Command::StartPage { id: "dock".into() }]);
    // While a page is in progress the first row stops it, whether this
    // phone is paging or listening.
    for activity in [
        PageActivity::Sending { group: "dock".into(), name: "Loading dock".into() },
        PageActivity::Receiving { group: "1".into(), name: "Everyone".into() },
    ] {
        r.store.update::<Paging, _>(|p| p.activity = activity.clone());
        r.ui.pump();
        r.tap(Key::Select);
        assert_eq!(Rig::drain(&paging), [Command::StopPage]);
    }
}

#[test]
fn bluetooth_application_asks_before_pairing() {
    let mut r = Rig::new(Model::D65);
    let bt = r.store.handle(Domain::Bluetooth);
    let show = |r: &mut Rig| {
        let shown = r.store.send(Command::ShowScreen { name: "bluetooth".into() });
        r.ui.pump();
        assert_eq!(shown.wait(), Ok(Reply::Done));
    };
    // Off: the only row turns it on.
    r.store.update::<Bluetooth, _>(|b| b.present = true);
    show(&mut r);
    assert_eq!(r.screen(), "bluetooth");
    r.tap(Key::Select);
    assert_eq!(Rig::drain(&bt), [Command::SetBluetooth { on: true }]);

    // A headset asks to pair: the question is the first row, with the six
    // digits to compare; selecting it accepts, the next row refuses.
    let addr = "00:1B:DC:0F:A1:02".to_string();
    r.store.update::<Bluetooth, _>(|b| {
        b.enabled = true;
        b.pairing = Some(BluetoothPairing { addr: addr.clone(), name: "Headset".into(), passkey: Some(42), pin_needed: false });
    });
    r.ui.pump();
    r.tap(Key::Select);
    assert_eq!(Rig::drain(&bt), [Command::BluetoothAnswerPairing { addr: addr.clone(), accept: true, pin: None }]);
    r.tap(Key::Down);
    r.tap(Key::Select);
    assert_eq!(Rig::drain(&bt), [Command::BluetoothAnswerPairing { addr: addr.clone(), accept: false, pin: None }]);

    // An old device needs a PIN: it is typed, not assumed.
    r.store.update::<Bluetooth, _>(|b| b.pairing.as_mut().unwrap().pin_needed = true);
    r.ui.pump();
    r.tap(Key::Up);
    r.tap(Key::Select);
    assert_eq!(r.screen(), "bluetooth-pin");
    for d in [1, 2, 3, 4] {
        r.tap(Key::Digit(d));
    }
    r.tap(Key::Select);
    assert_eq!(Rig::drain(&bt), [Command::BluetoothAnswerPairing { addr, accept: true, pin: Some("1234".into()) }]);
    assert_eq!(r.screen(), "bluetooth");
}

/// A view with one text widget, for the idle screen.
fn idle_view(name: &str, model: Model) -> (dphone_ui::AppView, dphone_ui::AppEvents) {
    let (view, events) = dphone_ui::AppView::new(name, model);
    let line = view.create(dphone_ui::appview::WidgetKind::Text, None, dphone_ui::appview::Rect::new(0, 0, 100, 16));
    view.set_text(line, name);
    (view, events)
}

fn lifecycle(events: &dphone_ui::AppEvents) -> Vec<dphone_ui::AppEvent> {
    std::iter::from_fn(|| events.try_recv()).collect()
}

#[test]
fn idle_views_are_told_when_the_idle_screen_shows_and_hides() {
    use dphone_ui::AppEvent::{Background, Foreground};
    let mut r = Rig::new(Model::D60);
    let (view, events) = idle_view("status", Model::D60);
    r.ui.handle().add_idle_view(view.clone());
    r.ui.pump();
    assert_eq!(lifecycle(&events), [Foreground], "the idle screen is showing");
    // A menu covers the idle screen; leaving it uncovers it.
    r.tap(Key::Menu);
    assert_eq!(lifecycle(&events), [Background]);
    r.tap(Key::Cancel);
    assert_eq!(r.screen(), "idle");
    assert_eq!(lifecycle(&events), [Foreground]);
    // So does a call.
    let call = r.add_call(CallState::Ringing);
    assert_eq!(lifecycle(&events), [Background]);
    r.store.update::<Calls, _>(|c| c.calls.retain(|x| x.id != call));
    r.ui.pump();
    assert_eq!(lifecycle(&events), [Foreground]);
    // An app's hidden call has no screen: the idle screen stays.
    r.store.update::<Calls, _>(|c| {
        let mut hidden = Call::new(CallId(900), AccountId(0), Direction::Incoming, Party::default());
        hidden.hidden = true;
        c.calls.push(hidden);
    });
    r.ui.pump();
    assert_eq!(r.screen(), "idle");
    assert_eq!(lifecycle(&events), []);
    // A view with nothing in it is not shown; withdrawn, it hears nothing more.
    view.clear();
    r.ui.pump();
    assert_eq!(lifecycle(&events), [Background]);
    r.ui.handle().remove_idle_view("status");
    r.ui.pump();
    assert_eq!(lifecycle(&events), []);
}

#[test]
fn several_idle_views_take_turns() {
    use dphone_ui::AppEvent::{Background, Foreground};
    let model = Model::D60;
    let mut hal = Hal::sim(model);
    let store = Store::new(model);
    store.update::<System, _>(|s| s.phase = Phase::Running);
    let cfg = UiConfig { backlight_timeout_secs: 0, idle_cycle_secs: 1, ..Default::default() };
    let mut ui = Ui::from_hal(&mut hal, store.clone(), cfg.clone());
    ui.pump();
    let (a, a_events) = idle_view("a", model);
    let (b, b_events) = idle_view("b", model);
    let (empty, empty_events) = dphone_ui::AppView::new("empty", model);
    ui.handle().add_idle_view(a);
    ui.handle().add_idle_view(empty);
    ui.handle().add_idle_view(b);
    ui.pump();
    assert_eq!((lifecycle(&a_events), lifecycle(&b_events)), (vec![Foreground], vec![]));
    std::thread::sleep(std::time::Duration::from_millis(1100));
    ui.pump();
    assert_eq!((lifecycle(&a_events), lifecycle(&b_events)), (vec![Background], vec![Foreground]));
    std::thread::sleep(std::time::Duration::from_millis(1100));
    ui.pump();
    assert_eq!((lifecycle(&a_events), lifecycle(&b_events)), (vec![Foreground], vec![Background]));
    // A cycle time of 0 shows no idle view at all.
    ui.handle().set_config(UiConfig { idle_cycle_secs: 0, ..cfg });
    ui.pump();
    assert_eq!((lifecycle(&a_events), lifecycle(&b_events)), (vec![Background], vec![]));
    assert_eq!(lifecycle(&empty_events), [], "a view without widgets never shows");
}

#[test]
fn an_app_can_keep_the_backlight_on() {
    let model = Model::D60;
    let mut hal = Hal::sim(model);
    let sim = hal.sim.clone().unwrap();
    let store = Store::new(model);
    store.update::<System, _>(|s| s.phase = Phase::Running);
    let mut ui = Ui::from_hal(&mut hal, store.clone(), UiConfig { backlight_timeout_secs: 1, brightness: 10, ..Default::default() });
    ui.pump();
    for _ in 0..3 {
        std::thread::sleep(std::time::Duration::from_millis(600));
        store.post(Command::ResetIdleTimer);
        ui.pump();
        assert_eq!(sim.brightness(Screen::Main), Some(32), "kept awake past the 1 s timeout");
    }
    std::thread::sleep(std::time::Duration::from_millis(1100));
    ui.pump();
    assert_eq!(sim.brightness(Screen::Main), Some(6));
    store.post(Command::ResetIdleTimer);
    ui.pump();
    assert_eq!(sim.brightness(Screen::Main), Some(32), "and woken");
}
