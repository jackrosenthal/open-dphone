//! The UI thread: input, navigation, rendering, lamps, and backlight.

use crate::appview::AppView;
use crate::gfx::{Canvas, HAlign, IRect};
use crate::screens::app::AppScreen;
use crate::screens::calls::{CallScreen, Dialer, Purpose};
use crate::screens::{boot, idle, menus, side, Cx, Nav, Screen, Soft, Tag};
use crate::skin::Theme;
use crate::strings as tr;
use crate::widgets;
use dphone_hal::{Backlight, Blinker, Display, Hal, Input, InputEvent, Lamp, Lamps, Null, BLINK_TICK_MS};
use dphone_state::{
    self as ds, clock, AudioPath, BlfState, CallId, CallState, Command, CommandError, Domain, Envelope, Event,
    InputSource, Key, LampColor, LampState, LineKeyBinding, Model, Notice, Phase, ProvisionPhase, Reply, Store,
    Subsystem, Topic, TopicSet, UpdatePhase, VolumeTarget,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

/// Settings the integrator maps from the merged configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiConfig {
    /// 24-hour clock.
    pub time_24h: bool,
    /// Local time offset from UTC in seconds (the UI has no zone database).
    pub utc_offset_secs: i32,
    /// Backlight brightness 0..=10 (stock `brightness`).
    pub brightness: u8,
    /// Dim (rather than switch off) the backlight when idle (stock
    /// `dim_backlight`).
    pub dim_backlight: bool,
    /// Seconds of inactivity before the backlight dims or goes off; 0
    /// keeps it on (stock `backlight_timeout`).
    pub backlight_timeout_secs: u32,
    /// Gray panels: contrast level (stock `contrast`).
    pub contrast: u8,
    /// Highest contrast level; filled in by the UI from the HAL.
    pub contrast_max: u8,
    /// Name of the selected ringtone and the names available.
    pub ringtone: String,
    pub ringtones: Vec<String>,
    /// Off-hook dialing: milliseconds after the last digit before the
    /// number is dialed.
    pub dial_timeout_ms: u32,
}

impl Default for UiConfig {
    fn default() -> Self {
        UiConfig {
            time_24h: false,
            utc_offset_secs: 0,
            brightness: 5,
            dim_backlight: true,
            backlight_timeout_secs: 60,
            contrast: 0,
            contrast_max: 0,
            ringtone: String::new(),
            ringtones: Vec::new(),
            dial_timeout_ms: 4000,
        }
    }
}

enum Msg {
    Input(InputEvent),
    State,
    Command(Envelope),
    OpenApp(AppView),
    IdleView(Option<AppView>),
    CloseApp(String),
    Config(UiConfig),
    Wake,
    Quit,
}

/// Thread-safe access to a running [`Ui`].
#[derive(Clone)]
pub struct UiHandle {
    tx: Sender<Msg>,
}

impl UiHandle {
    /// Puts an app's view on top of the navigation stack (or raises it if
    /// already there).  The app gets `AppEvent::Foreground`.
    pub fn open_app(&self, view: AppView) {
        let _ = self.tx.send(Msg::OpenApp(view));
    }
    /// Shows an app's view in the idle screen's app area (stock `hasIdle`
    /// apps), or removes it with `None`.  The view is display-only: it
    /// gets no keys.  Its size is [`crate::idle_app_area`].
    pub fn set_idle_view(&self, view: Option<AppView>) {
        let _ = self.tx.send(Msg::IdleView(view));
    }
    /// Removes an app's view from the stack.
    pub fn close_app(&self, name: &str) {
        let _ = self.tx.send(Msg::CloseApp(name.to_string()));
    }
    /// Replaces the UI configuration.
    pub fn set_config(&self, cfg: UiConfig) {
        let _ = self.tx.send(Msg::Config(cfg));
    }
    /// Requests a repaint.
    pub fn wake(&self) {
        let _ = self.tx.send(Msg::Wake);
    }
    /// Makes [`Ui::run`] return.
    pub fn quit(&self) {
        let _ = self.tx.send(Msg::Quit);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BaseKind {
    Boot,
    Recovery,
    Provision,
    Update,
    Idle,
}

fn base_kind(st: &ds::State) -> BaseKind {
    if !matches!(st.update.phase, UpdatePhase::Idle | UpdatePhase::Failed(_)) {
        return BaseKind::Update;
    }
    match st.system.phase {
        Phase::Recovery => BaseKind::Recovery,
        Phase::Starting | Phase::NetworkWait | Phase::ShuttingDown => BaseKind::Boot,
        Phase::Provisioning => match st.provisioning.phase {
            ProvisionPhase::Provisioned | ProvisionPhase::Standalone => BaseKind::Idle,
            _ => BaseKind::Provision,
        },
        Phase::Running => BaseKind::Idle,
    }
}

fn make_base(kind: BaseKind) -> Box<dyn Screen> {
    match kind {
        BaseKind::Boot => Box::new(boot::Boot),
        BaseKind::Recovery => Box::new(boot::Recovery),
        BaseKind::Provision => Box::new(boot::Provision::default()),
        BaseKind::Update => Box::new(boot::UpdateScreen),
        BaseKind::Idle => Box::new(idle::Idle),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Light {
    On,
    Dim,
    Off,
}

struct Popup {
    until_ms: u64,
}

/// The navigation stack.
struct Stack {
    kind: BaseKind,
    base: Box<dyn Screen>,
    stack: Vec<Box<dyn Screen>>,
    soft_page: u8,
    seen_ringing: Vec<CallId>,
}

/// The four softkey positions of a native screen: action id (0: none) and
/// label.  More than four actions page three at a time behind "More".
fn soft_slots(keys: &[Soft], page: u8) -> [(u32, String); 4] {
    const MORE: u32 = u32::MAX;
    let mut out = [(0, String::new()), (0, String::new()), (0, String::new()), (0, String::new())];
    if keys.len() <= 4 {
        for (i, k) in keys.iter().enumerate() {
            out[i] = k.clone();
        }
    } else {
        let pages = keys.len().div_ceil(3);
        let page = page as usize % pages;
        for (i, k) in keys.iter().skip(page * 3).take(3).enumerate() {
            out[i] = k.clone();
        }
        out[3] = (MORE, tr::MORE.to_string());
    }
    out
}

impl Stack {
    fn top(&mut self) -> &mut Box<dyn Screen> {
        self.stack.last_mut().unwrap_or(&mut self.base)
    }
    fn top_ref(&self) -> &dyn Screen {
        self.stack.last().unwrap_or(&self.base).as_ref()
    }

    fn push(&mut self, cx: &Cx, s: Box<dyn Screen>) {
        self.top().leave(cx);
        self.stack.push(s);
        self.soft_page = 0;
        self.top().enter(cx);
    }
    fn pop(&mut self, cx: &Cx) {
        if let Some(mut s) = self.stack.pop() {
            s.leave(cx);
            self.soft_page = 0;
            self.top().enter(cx);
        }
    }
    /// Removes the screens above index `len` (top first).
    fn truncate(&mut self, cx: &Cx, len: usize) {
        if self.stack.len() <= len {
            return;
        }
        while self.stack.len() > len {
            if let Some(mut s) = self.stack.pop() {
                s.leave(cx);
            }
        }
        self.soft_page = 0;
        self.top().enter(cx);
    }

    fn apply(&mut self, cx: &Cx, nav: Nav) {
        match nav {
            Nav::Ignored | Nav::Stay => {}
            Nav::Push(s) => self.push(cx, s),
            Nav::Pop => self.pop(cx),
            Nav::Home => {
                // Back to the base, but never hide a live call.
                let keep = self.stack.iter().position(|s| s.tag() == Tag::Call).map(|i| i + 1).unwrap_or(0);
                self.truncate(cx, keep);
            }
        }
    }

    fn call_screen_pos(&self) -> Option<usize> {
        self.stack.iter().position(|s| s.tag() == Tag::Call)
    }

    /// Makes the call screen the top screen.
    fn raise_call_screen(&mut self, cx: &Cx) {
        match self.call_screen_pos() {
            Some(i) => self.truncate(cx, i + 1),
            None => self.push(cx, Box::new(CallScreen::default())),
        }
    }

    /// Follows the state: base screen kind, and the call screen.
    fn sync(&mut self, cx: &Cx) {
        let kind = base_kind(cx.st);
        if kind != self.kind {
            self.truncate(cx, 0);
            self.base.leave(cx);
            self.kind = kind;
            self.base = make_base(kind);
            self.base.enter(cx);
        }
        if kind == BaseKind::Update {
            return;
        }
        let live = !cx.st.calls.is_idle();
        let ringing: Vec<CallId> = cx.st.calls.ringing().map(|c| c.id).collect();
        let new_ring = ringing.iter().any(|id| !self.seen_ringing.contains(id));
        self.seen_ringing = ringing;
        match (live, self.call_screen_pos()) {
            (true, None) => {
                // A live dialer stays on top of the new call screen only
                // if it is for a transfer or conference; a plain dialer has
                // done its job.
                if self.top_ref().tag() == Tag::Dialer {
                    self.pop(cx);
                }
                self.push(cx, Box::new(CallScreen::default()));
            }
            (true, Some(i)) => {
                if new_ring && self.stack.len() > i + 1 && self.top_ref().tag() != Tag::Dialer {
                    self.truncate(cx, i + 1);
                }
            }
            (false, Some(i)) => self.truncate(cx, i),
            (false, None) => {}
        }
    }

    fn open_app(&mut self, cx: &Cx, view: AppView) {
        if let Some(i) = self.stack.iter().position(|s| s.app_name() == Some(view.name())) {
            let mut s = self.stack.remove(i);
            s.leave(cx);
        }
        self.push(cx, Box::new(AppScreen::new(view)));
    }

    fn close_app(&mut self, cx: &Cx, name: &str) {
        if let Some(i) = self.stack.iter().position(|s| s.app_name() == Some(name)) {
            let was_top = i + 1 == self.stack.len();
            let mut s = self.stack.remove(i);
            s.leave(cx);
            if was_top {
                self.soft_page = 0;
                self.top().enter(cx);
            }
        }
    }

    fn blf_press(&mut self, cx: &Cx, entry: &str, number: &str) {
        let e = cx.st.presence.get(entry);
        if e.is_some_and(|e| e.state == BlfState::Ringing) {
            cx.post(Command::Pickup { blf_entry: entry.to_string() });
            return;
        }
        let number = if number.is_empty() { e.map(|e| e.number.as_str()).unwrap_or("") } else { number };
        if !number.is_empty() {
            cx.post(Command::Dial { target: number.to_string(), account: e.and_then(|e| e.account), path: None });
        }
    }

    /// Keys no screen took.
    fn global_key(&mut self, cx: &Cx, key: Key) {
        let st = cx.st;
        let calls = &st.calls;
        let live = !calls.is_idle();
        let ringing = calls.ringing().next().map(|c| c.id);
        let focused = calls.focused().filter(|c| !c.state.is_ended()).or_else(|| calls.live().next());
        let idle_base = self.kind == BaseKind::Idle;
        match key {
            Key::Mute if live => cx.post(Command::SetMute { on: !st.audio.muted }),
            Key::Speaker | Key::Headset => {
                let path = if key == Key::Speaker { AudioPath::Speaker } else { AudioPath::Headset };
                if let Some(call) = ringing {
                    cx.post(Command::Answer { call, path: Some(path) });
                } else if let (true, Some(call)) = (live, focused) {
                    if st.audio.path == path {
                        // Leaving the path: back to the handset if it is
                        // lifted, else the call ends.
                        if cx.off_hook {
                            cx.post(Command::SetAudioPath { path: AudioPath::Handset });
                        } else {
                            cx.post(Command::Hangup { call: call.id });
                        }
                    } else {
                        cx.post(Command::SetAudioPath { path });
                    }
                } else if idle_base {
                    // Either key alone goes off-hook on its path, as on
                    // the stock firmware (observed on a D60: the Headset
                    // key at idle opens New Call, attaches the headset,
                    // and plays the dial tone there).
                    let d = Dialer::off_hook(cx, path, None);
                    self.push(cx, Box::new(d));
                } else if key == Key::Headset {
                    cx.post(Command::SetHeadsetMode { on: !st.audio.headset_mode });
                }
            }
            Key::Hold | Key::Transfer | Key::Conference if live => {
                if self.top_ref().tag() != Tag::Call {
                    self.raise_call_screen(cx);
                    let nav = self.top().key(cx, key);
                    self.apply(cx, nav);
                }
            }
            Key::Redial if !live => cx.post(Command::Redial),
            Key::Messages => {
                if st.apps.installed.iter().any(|a| a.name == "voicemail") {
                    cx.post(Command::LaunchApp { name: "voicemail".into() });
                } else {
                    cx.post(Command::DialVoicemail { account: None });
                }
            }
            Key::Line(n) => {
                let Some(k) = st.keys.line_keys.iter().find(|k| k.index + 1 == n) else {
                    // No key table yet: line 1 goes home.
                    if n == 1 {
                        self.apply(cx, Nav::Home);
                    }
                    return;
                };
                match &k.binding {
                    LineKeyBinding::Account(a) => {
                        if let Some(call) = k.calls.first() {
                            let c = calls.get(*call);
                            match c.map(|c| &c.state) {
                                Some(CallState::Ringing) => {
                                    cx.post(Command::Answer { call: *call, path: cx.off_hook.then_some(AudioPath::Handset) })
                                }
                                Some(CallState::Held { .. }) => cx.post(Command::Resume { call: *call }),
                                _ => cx.post(Command::Focus { call: *call }),
                            }
                            self.raise_call_screen(cx);
                        } else if idle_base && !live {
                            let path = if cx.off_hook {
                                AudioPath::Handset
                            } else if st.audio.headset_mode {
                                AudioPath::Headset
                            } else {
                                AudioPath::Speaker
                            };
                            let d = Dialer::off_hook(cx, path, Some(*a));
                            self.apply(cx, Nav::Home);
                            self.push(cx, Box::new(d));
                        } else if live {
                            let mut d = Dialer::new(Purpose::Conference(focused.map(|c| c.id).unwrap_or(CallId(0))), Some(*a));
                            d.text.clear();
                            self.push(cx, Box::new(d));
                        }
                    }
                    LineKeyBinding::Blf(entry) => self.blf_press(cx, entry, ""),
                    LineKeyBinding::Unused => {}
                }
            }
            Key::Blf(n) => {
                let page = st.keys.rapid_page;
                if let Some(k) = st.keys.rapid_dial.iter().find(|k| k.index + 1 == n && k.page == page) {
                    self.blf_press(cx, &k.entry, &k.number);
                }
            }
            Key::BlfPageLeft | Key::BlfPageRight => {
                let pages = st.keys.rapid_pages.max(1) as i32;
                let d = if key == Key::BlfPageLeft { -1 } else { 1 };
                let page = (st.keys.rapid_page as i32 + d).rem_euclid(pages) as u8;
                cx.post(Command::SetRapidDialPage { page });
            }
            Key::Menu if idle_base => self.push(cx, menus::main_menu()),
            Key::Apps if idle_base => self.push(cx, menus::launcher()),
            Key::Contacts => cx.post(Command::LaunchApp { name: "contacts".into() }),
            Key::Status | Key::Info if idle_base => self.push(cx, menus::about()),
            Key::Cancel => self.pop(cx),
            _ => {}
        }
    }

    fn hook(&mut self, cx: &Cx, off_hook: bool) {
        let nav = self.top().hook(cx, off_hook);
        if !matches!(nav, Nav::Ignored) {
            self.apply(cx, nav);
            return;
        }
        let calls = &cx.st.calls;
        if off_hook {
            if let Some(call) = calls.ringing().next() {
                cx.post(Command::Answer { call: call.id, path: Some(AudioPath::Handset) });
            } else if !calls.is_idle() {
                cx.post(Command::SetAudioPath { path: AudioPath::Handset });
            } else if self.kind == BaseKind::Idle {
                let d = Dialer::off_hook(cx, AudioPath::Handset, None);
                self.push(cx, Box::new(d));
            }
        } else if !calls.is_idle() {
            // Hanging up ends the call that is on the handset; calls on
            // the speaker or headset continue.
            if matches!(cx.st.audio.path, AudioPath::Handset | AudioPath::None) {
                let target = calls
                    .focused()
                    .filter(|c| !c.state.is_ended() && c.state != CallState::Ringing && !c.state.is_held())
                    .or_else(|| calls.live().find(|c| c.state != CallState::Ringing && !c.state.is_held()));
                if let Some(c) = target {
                    cx.post(Command::Hangup { call: c.id });
                }
            }
        } else if cx.st.audio.path == AudioPath::Handset {
            cx.post(Command::SetAudioPath { path: AudioPath::None });
        }
    }
}

/// The phone UI.  See the crate docs.
pub struct Ui {
    store: Store,
    model: Model,
    th: &'static Theme,
    display: Box<dyn Display>,
    backlight: Box<dyn Backlight>,
    lamps: Box<dyn Lamps>,
    input: Option<Box<dyn Input>>,
    main: Canvas,
    side: Option<Canvas>,
    cfg: Arc<UiConfig>,
    nav: Stack,
    off_hook: bool,
    popup: Option<Popup>,
    notice_deadlines: Vec<(u64, u64)>,
    blinker: Blinker,
    lamp_test: bool,
    light: Light,
    level: u8,
    last_activity_ms: u64,
    fixed_time: Option<(u64, u64)>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    commands: Option<ds::CommandReceiver>,
    state_pending: Arc<AtomicBool>,
    _guard: ds::CallbackGuard,
    needs_draw: bool,
    quit: bool,
    last_failed_update: bool,
    /// When the top screen next needs a tick or repaint (set at each draw).
    screen_deadline: Option<u64>,
    /// An app's view shown on the idle screen (`hasIdle`).
    idle_view: Option<AppView>,
}

impl Ui {
    /// Builds the UI from the HAL bundle, taking the display, backlight,
    /// input, and lamps out of it (null devices are left in their place).
    pub fn from_hal(hal: &mut Hal, store: Store, cfg: UiConfig) -> Ui {
        let display = std::mem::replace(&mut hal.display, Box::new(Null("display")));
        let backlight = std::mem::replace(&mut hal.backlight, Box::new(Null("backlight")));
        let input = std::mem::replace(&mut hal.input, Box::new(Null("input")));
        let lamps = std::mem::replace(&mut hal.lamps, Box::new(Null("lamps")));
        Ui::new(display, backlight, input, lamps, store, cfg)
    }

    /// Builds the UI from its devices.  The model comes from the store.
    pub fn new(
        display: Box<dyn Display>,
        backlight: Box<dyn Backlight>,
        mut input: Box<dyn Input>,
        lamps: Box<dyn Lamps>,
        store: Store,
        mut cfg: UiConfig,
    ) -> Ui {
        let model = store.model();
        let th = Theme::for_model(model);
        let size = model.main_screen();
        let main = Canvas::new(size.width, size.height, th.gray);
        let side = model.side_screen().map(|s| Canvas::new(s.width, s.height, th.gray));
        cfg.contrast_max = backlight.max_contrast(dphone_hal::Screen::Main);
        let (tx, rx) = mpsc::channel();
        let state_pending = Arc::new(AtomicBool::new(false));
        // Everything except our own topic and the log.
        let topics: TopicSet = Topic::ALL.into_iter().filter(|t| !matches!(t, Topic::Ui | Topic::Log)).collect();
        let guard = {
            let tx = tx.clone();
            let pending = state_pending.clone();
            store.on_change(topics, move |_| {
                if !pending.swap(true, Ordering::AcqRel) {
                    let _ = tx.send(Msg::State);
                }
            })
        };
        let commands = Some(store.handle(Domain::Ui));
        let off_hook = input.off_hook().unwrap_or(false);
        let kind = base_kind(&store.snapshot());
        let mut ui = Ui {
            store,
            model,
            th,
            display,
            backlight,
            lamps,
            input: Some(input),
            main,
            side,
            cfg: Arc::new(cfg),
            nav: Stack { kind, base: make_base(kind), stack: Vec::new(), soft_page: 0, seen_ringing: Vec::new() },
            off_hook,
            popup: None,
            notice_deadlines: Vec::new(),
            blinker: Blinker::new(),
            lamp_test: false,
            light: Light::On,
            level: 0,
            last_activity_ms: clock::mono_ms(),
            fixed_time: None,
            tx,
            rx,
            commands,
            state_pending,
            _guard: guard,
            needs_draw: true,
            quit: false,
            last_failed_update: false,
            screen_deadline: None,
            idle_view: None,
        };
        for s in ui.display.screens() {
            let _ = ui.display.set_power(s, true);
        }
        ui.apply_light(Light::On);
        let c = ui.cfg.contrast;
        if ui.cfg.contrast_max > 0 {
            let _ = ui.backlight.set_contrast(dphone_hal::Screen::Main, c.min(ui.cfg.contrast_max));
        }
        ui.store.update::<ds::Ui, _>(|u| u.contrast = c);
        ui.store.set_health(Subsystem::Ui, ds::HealthStatus::Ok);
        ui
    }

    /// A handle for other threads.
    pub fn handle(&self) -> UiHandle {
        UiHandle { tx: self.tx.clone() }
    }

    /// Freezes the clock the screens see (snapshot tests): monotonic
    /// milliseconds and Unix seconds.
    pub fn set_fixed_time(&mut self, time: Option<(u64, u64)>) {
        self.fixed_time = time;
        self.needs_draw = true;
    }

    fn now(&self) -> (u64, u64) {
        self.fixed_time.unwrap_or_else(|| (clock::mono_ms(), clock::unix_secs()))
    }

    /// Runs `f` with a screen context and the navigation stack.
    fn with_cx<R>(&mut self, f: impl FnOnce(&mut Stack, &Cx) -> R) -> R {
        let st = self.store.snapshot();
        let cfg = self.cfg.clone();
        let store = self.store.clone();
        let (now_ms, unix) = self.now();
        let idle_view = self.idle_view.clone();
        let cx = Cx {
            store: &store,
            st: &st,
            th: self.th,
            model: self.model,
            cfg: &cfg,
            now_ms,
            unix,
            off_hook: self.off_hook,
            idle_view: idle_view.as_ref(),
        };
        let r = f(&mut self.nav, &cx);
        self.needs_draw = true;
        r
    }

    // ------------------------------------------------------------ light

    fn hw_level(&self, brightness: u8) -> u8 {
        let max = self.backlight.max_brightness(dphone_hal::Screen::Main) as u32;
        let l = brightness.min(10) as u32 * max / 10;
        if brightness > 0 { l.max(1) as u8 } else { 0 }
    }

    fn apply_light(&mut self, light: Light) {
        let on = if self.level > 0 { self.level } else { self.hw_level(self.cfg.brightness) };
        let level = match light {
            Light::On => on,
            Light::Dim => (on / 4).max(1).min(on),
            Light::Off => 0,
        };
        self.light = light;
        for s in self.display.screens() {
            let _ = self.backlight.set_brightness(s, level);
        }
        self.store.update::<ds::Ui, _>(|u| {
            u.backlight_on = light != Light::Off;
            u.backlight_level = on;
        });
    }

    fn activity(&mut self) {
        self.last_activity_ms = clock::mono_ms();
        if self.light != Light::On {
            self.apply_light(Light::On);
        }
    }

    fn light_deadline(&self) -> Option<u64> {
        let t = self.cfg.backlight_timeout_secs as u64;
        (t > 0 && self.light == Light::On).then(|| self.last_activity_ms + t * 1000)
    }

    fn check_light(&mut self) {
        let st_calls_idle = self.store.read::<ds::Calls, _>(|c| c.is_idle());
        if !st_calls_idle {
            self.activity();
            return;
        }
        if let Some(d) = self.light_deadline()
            && clock::mono_ms() >= d {
                self.apply_light(if self.cfg.dim_backlight { Light::Dim } else { Light::Off });
            }
    }

    // ------------------------------------------------------------ input

    /// Feeds a key event as if from the keypad.
    pub fn key(&mut self, key: Key, down: bool, source: InputSource) {
        self.store.emit(Event::Key { key, down, source });
        if !down {
            return;
        }
        self.activity();
        if self.nav.kind == BaseKind::Update {
            return;
        }
        // A notice on screen takes Select, Cancel, and the softkeys.
        if matches!(key, Key::Select | Key::Cancel | Key::Soft(_)) {
            let top = self.store.read::<ds::Ui, _>(|u| u.notices.last().map(|n| n.id));
            if let Some(id) = top {
                self.dismiss_notice(id);
                return;
            }
        }
        let captures = self.nav.top_ref().captures_all_keys();
        if !captures && matches!(key, Key::VolumeUp | Key::VolumeDown) {
            let delta = if key == Key::VolumeUp { 1 } else { -1 };
            self.store.post(Command::AdjustVolume { target: VolumeTarget::Current, delta });
            self.popup = Some(Popup { until_ms: self.now().0 + 2000 });
            self.needs_draw = true;
            return;
        }
        self.with_cx(|nav, cx| {
            if let (Key::Soft(n @ 1..=4), None, false) = (key, nav.top_ref().raw_softkeys(cx), captures) {
                let keys = nav.top_ref().softkeys(cx);
                let (id, _) = soft_slots(&keys, nav.soft_page)[n as usize - 1].clone();
                if id == u32::MAX {
                    nav.soft_page = (nav.soft_page + 1) % keys.len().div_ceil(3).max(1) as u8;
                } else if id != 0 {
                    let r = nav.top().softkey(cx, id);
                    nav.apply(cx, r);
                }
                return;
            }
            let r = nav.top().key(cx, key);
            if matches!(r, Nav::Ignored) {
                if !captures {
                    nav.global_key(cx, key);
                }
            } else {
                nav.apply(cx, r);
            }
        });
    }

    /// Feeds a hookswitch change.
    pub fn hook(&mut self, off_hook: bool, source: InputSource) {
        self.off_hook = off_hook;
        self.store.emit(Event::Hook { off_hook, source });
        self.activity();
        if self.nav.kind == BaseKind::Update {
            return;
        }
        self.with_cx(|nav, cx| nav.hook(cx, off_hook));
    }

    fn input_event(&mut self, ev: InputEvent, source: InputSource) {
        match ev {
            InputEvent::Key { key, down } => self.key(key, down, source),
            InputEvent::Hook { off_hook } => self.hook(off_hook, source),
        }
    }

    // --------------------------------------------------------- commands

    fn dismiss_notice(&mut self, id: u64) {
        self.store.update::<ds::Ui, _>(|u| u.notices.retain(|n| n.id != id));
        self.notice_deadlines.retain(|(i, _)| *i != id);
        self.needs_draw = true;
    }

    fn notify(&mut self, level: ds::NoticeLevel, title: String, text: String, timeout_ms: u32) -> u64 {
        let id = self.store.next_notice_id();
        self.store.update::<ds::Ui, _>(|u| {
            u.notices.push(Notice { id, level, title, text, timeout_ms });
            // Bound the backlog.
            if u.notices.len() > 8 {
                u.notices.remove(0);
            }
        });
        if timeout_ms > 0 {
            self.notice_deadlines.push((id, self.now().0 + timeout_ms as u64));
        }
        self.activity();
        self.needs_draw = true;
        id
    }

    fn command(&mut self, env: Envelope) {
        let (cmd, responder) = env.split();
        let result = match cmd {
            Command::InjectKey { key, down } => {
                self.key(key, down, InputSource::Injected);
                Ok(Reply::Done)
            }
            Command::InjectHook { off_hook } => {
                self.hook(off_hook, InputSource::Injected);
                Ok(Reply::Done)
            }
            Command::Notify { level, title, text, timeout_ms } => Ok(Reply::Id(self.notify(level, title, text, timeout_ms))),
            Command::DismissNotice { id } => {
                self.dismiss_notice(id);
                Ok(Reply::Done)
            }
            Command::SetBacklight { level } => {
                self.level = level.min(self.backlight.max_brightness(dphone_hal::Screen::Main));
                self.last_activity_ms = clock::mono_ms();
                self.apply_light(Light::On);
                Ok(Reply::Done)
            }
            Command::SetContrast { level } => {
                let level = level.min(self.cfg.contrast_max);
                match self.backlight.set_contrast(dphone_hal::Screen::Main, level) {
                    Ok(()) => {
                        self.store.update::<ds::Ui, _>(|u| u.contrast = level);
                        Ok(Reply::Done)
                    }
                    Err(e) => Err(CommandError::Failed(e.to_string())),
                }
            }
            Command::ShowScreen { name } => {
                let screen: Option<Option<Box<dyn Screen>>> = match name.as_str() {
                    "idle" | "home" => Some(None),
                    "menu" => Some(Some(menus::main_menu())),
                    "call_log" => Some(Some(menus::call_log())),
                    "apps" => Some(Some(menus::launcher())),
                    "forward" => Some(Some(menus::forward_menu())),
                    "paging" => Some(Some(menus::paging())),
                    "bluetooth" => Some(Some(crate::screens::bluetooth::menu())),
                    "settings" => Some(Some(menus::settings())),
                    "network" => Some(Some(menus::network_menu())),
                    "status" | "about" => Some(Some(menus::about())),
                    "diagnostics" => Some(Some(crate::screens::diag::menu())),
                    "dialer" => Some(Some(Box::new(Dialer::new(Purpose::Call, None)))),
                    _ => None,
                };
                match screen {
                    None => Err(CommandError::Invalid(format!("no screen named {name:?}"))),
                    Some(s) => {
                        self.activity();
                        self.with_cx(|nav, cx| {
                            nav.apply(cx, Nav::Home);
                            if let Some(s) = s {
                                nav.push(cx, s);
                            }
                        });
                        Ok(Reply::Done)
                    }
                }
            }
            other => Err(CommandError::Invalid(format!("not a UI command: {other:?}"))),
        };
        responder.reply(result);
    }

    // ------------------------------------------------------------ lamps

    fn sync_lamps(&mut self) {
        let keys = self.store.get::<ds::Keys>();
        let test = self.nav.top_ref().lamp_test();
        if test {
            let (now, _) = self.now();
            let color = [LampColor::Red, LampColor::Green, LampColor::Amber, LampColor::Off][(now / 500 % 4) as usize];
            for lamp in self.lamps.lamps() {
                self.blinker.set(lamp, LampState::solid(color));
            }
        } else {
            for lamp in self.lamps.lamps() {
                let state = match lamp {
                    Lamp::Line(i) => keys.line_keys.iter().find(|k| k.index == i).map(|k| k.lamp),
                    Lamp::Blf(i) => {
                        keys.rapid_dial.iter().find(|k| k.index == i && k.page == keys.rapid_page).map(|k| k.lamp)
                    }
                    Lamp::Message => Some(keys.message_lamp),
                    Lamp::Mute => Some(keys.mute_lamp),
                    Lamp::Headset => Some(keys.headset_lamp),
                    Lamp::Speaker => Some(keys.speaker_lamp),
                };
                self.blinker.set(lamp, state.unwrap_or(LampState::OFF));
            }
        }
        self.lamp_test = test;
        let _ = self.blinker.tick(clock::mono_ms(), self.lamps.as_mut());
    }

    // ------------------------------------------------------------- draw

    fn draw(&mut self) {
        let st = self.store.snapshot();
        let cfg = self.cfg.clone();
        let store = self.store.clone();
        let (now_ms, unix) = self.now();
        let th = self.th;
        let idle_view = self.idle_view.clone();
        if let Some(v) = &idle_view {
            v.take_dirty();
        }
        let cx = Cx {
            store: &store,
            st: &st,
            th,
            model: self.model,
            cfg: &cfg,
            now_ms,
            unix,
            off_hook: self.off_hook,
            idle_view: idle_view.as_ref(),
        };
        let c = &mut self.main;
        let bounds = c.bounds();
        let top = self.nav.stack.last_mut().unwrap_or(&mut self.nav.base);
        if top.full_screen() {
            top.draw(&cx, c, bounds);
        } else {
            // The whole frame gets a background first: the softkey shapes
            // and the line-key tabs sit on it.
            if top.wallpaper() {
                c.wallpaper(th.bg);
            } else {
                c.clear(th.bg);
            }
            let (area, _) = bounds.split_bottom(th.softkey_h);
            top.draw(&cx, c, area);
            let labels = match top.raw_softkeys(&cx) {
                Some(l) => l,
                None => soft_slots(&top.softkeys(&cx), self.nav.soft_page).map(|(_, l)| l),
            };
            widgets::softkey_bar(c, th, &labels);
        }
        self.screen_deadline = top.deadline(&cx);
        let name = top.name();
        let dial = top.dial_buffer().unwrap_or("").to_string();

        // Volume popup.
        if self.popup.is_some() {
            let a = &st.audio;
            let (label, icon) = match a.path {
                AudioPath::Handset => (tr::VOL_HANDSET, "handset"),
                AudioPath::Headset => (tr::VOL_HEADSET, "headset"),
                AudioPath::Speaker => (tr::VOL_SPEAKER, "speaker"),
                AudioPath::None => (tr::VOL_RINGER, "ring"),
            };
            let (w, h) = if th.gray { (bounds.w * 3 / 4, th.line_h * 2 + 12) } else { (300, 84) };
            let inner = widgets::modal(c, th, bounds, w, h);
            let (head, rest) = inner.split_top(th.line_h);
            let (ic, text) = head.split_left(th.icon + th.pad);
            if let Some(i) = th.icon(icon) {
                c.icon_in(i, IRect { w: th.icon, ..ic }, th.fg);
            }
            c.text_in(th.font(crate::appview::Font::Bold), text, label, th.fg, HAlign::Left);
            let mh = if th.gray { 8 } else { 18 };
            widgets::meter(
                c,
                th,
                IRect::new(rest.x, rest.y + (rest.h - mh) / 2, rest.w, mh),
                a.volume(VolumeTarget::Current) as u32,
                ds::VOLUME_MAX as u32,
            );
        }

        // The newest notice.
        if let Some(n) = st.ui.notices.last() {
            let f = th.font(crate::appview::Font::Normal);
            let w = if th.gray { bounds.w - 12 } else { 380 };
            let lines = f.wrap(&n.text, w - 2 * th.pad).len().clamp(1, 4) as i32;
            let h = (lines + 1) * th.line_h + 2 * th.pad + 2;
            let inner = widgets::modal(c, th, bounds, w, h);
            let (head, rest) = inner.split_top(th.line_h);
            let (icon, tint) = match n.level {
                ds::NoticeLevel::Info => ("info", th.accent),
                ds::NoticeLevel::Warning => ("warning", th.warn),
                ds::NoticeLevel::Error => ("error", th.error),
            };
            let (ic, text) = head.split_left(th.icon + th.pad);
            if let Some(i) = th.icon(icon) {
                c.icon_in(i, IRect { w: th.icon, ..ic }, tint);
            }
            c.text_in(th.font(crate::appview::Font::Bold), text, &n.title, th.fg, HAlign::Left);
            c.text_wrapped(f, rest, &n.text, th.fg, HAlign::Left, th.line_h);
        }

        if let Err(e) = c.present(self.display.as_mut(), dphone_hal::Screen::Main) {
            log::warn!("display: {e}");
        }
        if let Some(sc) = &mut self.side {
            side::draw(&cx, sc);
            if let Err(e) = sc.present(self.display.as_mut(), dphone_hal::Screen::Side) {
                log::warn!("side display: {e}");
            }
        }
        self.store.update::<ds::Ui, _>(|u| {
            if u.screen != name {
                u.screen = name;
            }
            if u.dial_buffer != dial {
                u.dial_buffer = dial;
            }
        });
        self.needs_draw = false;
    }

    // ------------------------------------------------------------- loop

    fn on_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Input(ev) => self.input_event(ev, InputSource::Physical),
            Msg::State => {
                self.state_pending.store(false, Ordering::Release);
                self.state_changed();
            }
            Msg::Command(env) => self.command(env),
            Msg::OpenApp(view) => {
                let tx = self.tx.clone();
                view.set_ui_waker(Some(Box::new(move || {
                    let _ = tx.send(Msg::Wake);
                })));
                self.activity();
                self.with_cx(|nav, cx| nav.open_app(cx, view));
            }
            Msg::IdleView(view) => {
                if let Some(old) = self.idle_view.take() {
                    old.set_ui_waker(None);
                }
                if let Some(v) = &view {
                    let tx = self.tx.clone();
                    v.set_ui_waker(Some(Box::new(move || {
                        let _ = tx.send(Msg::Wake);
                    })));
                }
                self.idle_view = view;
                self.needs_draw = true;
            }
            Msg::CloseApp(name) => self.with_cx(|nav, cx| nav.close_app(cx, &name)),
            Msg::Config(mut cfg) => {
                cfg.contrast_max = self.cfg.contrast_max;
                let brightness_changed = cfg.brightness != self.cfg.brightness;
                let contrast_changed = cfg.contrast != self.cfg.contrast;
                self.cfg = Arc::new(cfg);
                if brightness_changed {
                    self.level = 0;
                    self.apply_light(self.light);
                }
                if contrast_changed && self.cfg.contrast_max > 0 {
                    let c = self.cfg.contrast.min(self.cfg.contrast_max);
                    let _ = self.backlight.set_contrast(dphone_hal::Screen::Main, c);
                    self.store.update::<ds::Ui, _>(|u| u.contrast = c);
                }
                self.needs_draw = true;
            }
            Msg::Wake => self.needs_draw = true,
            Msg::Quit => self.quit = true,
        }
    }

    fn state_changed(&mut self) {
        // A failed update becomes a notice (the update screen goes away).
        let failed = self.store.read::<ds::Update, _>(|u| match &u.phase {
            UpdatePhase::Failed(e) => Some(e.clone()),
            _ => None,
        });
        match (failed, self.last_failed_update) {
            (Some(e), false) => {
                self.last_failed_update = true;
                self.notify(ds::NoticeLevel::Error, tr::UPD_FAILED.into(), e, 0);
            }
            (None, true) => self.last_failed_update = false,
            _ => {}
        }
        let ringing = self.store.read::<ds::Calls, _>(|c| c.ringing().next().is_some());
        if ringing {
            self.activity();
        }
        self.with_cx(|nav, cx| nav.sync(cx));
    }

    /// Deadline-driven work: screen ticks, popups, notices, lamps, light.
    fn timers(&mut self) {
        let (now_ms, _) = self.now();
        let real_now = now_ms;
        if self.popup.as_ref().is_some_and(|p| real_now >= p.until_ms) {
            self.popup = None;
            self.needs_draw = true;
        }
        let expired: Vec<u64> = self.notice_deadlines.iter().filter(|(_, d)| real_now >= *d).map(|(i, _)| *i).collect();
        for id in expired {
            self.dismiss_notice(id);
        }
        // The deadline the top screen gave when it was last drawn.
        let due = self.screen_deadline.is_some_and(|d| now_ms >= d);
        if due {
            self.screen_deadline = None;
            self.with_cx(|nav, cx| {
                let r = nav.top().tick(cx);
                nav.apply(cx, r);
            });
        }
        if self.nav.top_ref().dirty() {
            self.needs_draw = true;
        }
        self.check_light();
    }

    fn next_deadline(&self) -> Option<u64> {
        let now = clock::mono_ms();
        let mut d: Option<u64> = None;
        let mut add = |t: Option<u64>| {
            if let Some(t) = t {
                d = Some(d.map_or(t, |x: u64| x.min(t)));
            }
        };
        add(self.popup.as_ref().map(|p| p.until_ms));
        add(self.notice_deadlines.iter().map(|(_, t)| *t).min());
        add(self.light_deadline());
        if self.blinker.needs_ticks() || self.lamp_test {
            add(Some(now + BLINK_TICK_MS));
        }
        if self.fixed_time.is_none() {
            add(self.screen_deadline);
        }
        d
    }

    /// Processes everything that is pending without blocking, then
    /// repaints if needed.  [`Ui::run`] is this in a loop; tests and the
    /// snapshot harness call it directly.  While the UI still owns the
    /// input device (before `run`), it is polled here too.
    pub fn pump(&mut self) {
        loop {
            let mut any = false;
            loop {
                let ev = match &mut self.input {
                    Some(input) => input.next_event(Some(Duration::ZERO)).ok().flatten(),
                    None => None,
                };
                let Some(ev) = ev else { break };
                self.input_event(ev, InputSource::Physical);
                any = true;
            }
            while let Some(env) = self.commands.as_ref().and_then(|rx| rx.try_recv()) {
                self.command(env);
                any = true;
            }
            while let Ok(msg) = self.rx.try_recv() {
                self.on_msg(msg);
                any = true;
            }
            if !any {
                break;
            }
        }
        self.finish_turn();
    }

    fn finish_turn(&mut self) {
        self.timers();
        self.sync_lamps();
        if self.needs_draw {
            self.draw();
        }
        self.store.heartbeat(Subsystem::Ui);
    }

    /// Runs the UI until [`UiHandle::quit`].  Spawns two small helper
    /// threads: one blocks on the input device, one on the command queue.
    pub fn run(mut self) {
        if let Some(mut input) = self.input.take() {
            let tx = self.tx.clone();
            let _ = std::thread::Builder::new().name("ui-input".into()).stack_size(64 * 1024).spawn(move || loop {
                match input.next_event(None) {
                    Ok(Some(ev)) => {
                        if tx.send(Msg::Input(ev)).is_err() {
                            break;
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        log::error!("input: {e}");
                        std::thread::sleep(Duration::from_secs(5));
                    }
                }
            });
        }
        if let Some(commands) = self.commands.take() {
            let tx = self.tx.clone();
            let _ = std::thread::Builder::new().name("ui-commands".into()).stack_size(64 * 1024).spawn(move || {
                while let Some(env) = commands.recv() {
                    if tx.send(Msg::Command(env)).is_err() {
                        break;
                    }
                }
            });
        }
        self.state_changed();
        self.finish_turn();
        while !self.quit {
            let now = clock::mono_ms();
            // Wake at least every 10 s for the heartbeat.
            let wait = self.next_deadline().map_or(10_000, |d| d.saturating_sub(now).min(10_000));
            match self.rx.recv_timeout(Duration::from_millis(wait.max(1))) {
                Ok(msg) => {
                    self.on_msg(msg);
                    while let Ok(msg) = self.rx.try_recv() {
                        self.on_msg(msg);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.finish_turn();
        }
        self.store.set_health(Subsystem::Ui, ds::HealthStatus::Stopped);
    }

    /// The main canvas (tests and tools).
    pub fn canvas(&self) -> &Canvas {
        &self.main
    }
    /// The D70 side canvas.
    pub fn side_canvas(&self) -> Option<&Canvas> {
        self.side.as_ref()
    }
    /// Name of the top screen (`idle`, `call`, `app:voicemail`, ...).
    pub fn screen_name(&self) -> String {
        self.nav.top_ref().name()
    }
}
