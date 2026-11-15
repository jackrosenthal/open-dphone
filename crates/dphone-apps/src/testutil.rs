//! Fakes for tests and the simulator: an in-memory [`Shell`] whose widget
//! trees can be inspected and driven, a scripted PBX, and a fixed
//! configuration.

use crate::services::{
    AppAccount, Completion, ConfigSource, HttpClient, HttpReply, HttpRequest, PbxRequest,
    PbxRequests, Services,
};
use crate::view::*;
use dphone_state::Key;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct Node {
    pub id: u32,
    pub kind: WidgetKind,
    pub parent: Option<u32>,
    pub bounds: Rect,
    pub visible: bool,
    pub text: String,
    pub font: Font,
    pub wrap: bool,
    pub highlight: bool,
    pub items: Vec<ListItem>,
    pub selected: u32,
    pub value: String,
    pub mode: InputMode,
    pub max_len: u32,
    pub image: String,
    pub softkeys: Vec<Softkey>,
    pub claims: Vec<Key>,
}

#[derive(Default)]
pub struct Win {
    pub nodes: BTreeMap<u32, Node>,
    next: u32,
    pub focus: Option<u32>,
    pub softkeys: Vec<Softkey>,
    pub claims: Vec<Key>,
    /// Package pictures handed over with `define_image`: name and size.
    pub images: Vec<(String, usize)>,
    events: VecDeque<WindowEvent>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Win {
    fn push(&mut self, ev: WindowEvent) {
        self.events.push_back(ev);
        if let Some(w) = &self.wake {
            w();
        }
    }

    fn descendants(&self, w: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut stack = vec![w];
        while let Some(p) = stack.pop() {
            for n in self.nodes.values().filter(|n| n.parent == Some(p)) {
                out.push(n.id);
                stack.push(n.id);
            }
        }
        out
    }

    /// Softkeys in effect: those of the focused widget or its nearest
    /// ancestor with any, else the window's.
    pub fn effective_softkeys(&self) -> Vec<Softkey> {
        let mut cur = self.focus;
        while let Some(id) = cur {
            match self.nodes.get(&id) {
                Some(n) if !n.softkeys.is_empty() => return n.softkeys.clone(),
                Some(n) => cur = n.parent,
                None => break,
            }
        }
        self.softkeys.clone()
    }

    fn claimed(&self, key: Key) -> bool {
        let mut cur = self.focus;
        while let Some(id) = cur {
            match self.nodes.get(&id) {
                Some(n) if n.claims.contains(&key) => return true,
                Some(n) => cur = n.parent,
                None => break,
            }
        }
        self.claims.contains(&key)
    }
}

#[derive(Default)]
struct UiState {
    windows: HashMap<(String, WindowKind), Win>,
    /// Navigation stack of app names; the last is on screen.
    stack: Vec<String>,
}

/// Handle to the fake screen, shared between the test and the manager.
#[derive(Clone, Default)]
pub struct FakeUi {
    state: Arc<Mutex<UiState>>,
    model: Option<dphone_state::Model>,
}

fn metrics_for(model: dphone_state::Model) -> Metrics {
    let area = model.app_content_area();
    let (skin, line, title, off, row, sb, color) = match model.skin() {
        dphone_state::Skin::SmallGray => (Skin::SmallGray, 15, 15, 1, 15, 4, false),
        dphone_state::Skin::LargeGray => (Skin::LargeGray, 17, 17, 1, 17, 4, false),
        dphone_state::Skin::Color => (Skin::Color, 25, 40, 4, 32, 8, true),
    };
    Metrics {
        skin,
        width: area.width,
        height: area.height,
        line_height: line,
        title_height: title,
        title_offset: off,
        row_height: row,
        scroll_bar_width: sb,
        color,
    }
}

impl FakeUi {
    pub fn new(model: dphone_state::Model) -> FakeUi {
        FakeUi {
            state: Default::default(),
            model: Some(model),
        }
    }

    pub fn shell(&self) -> Box<dyn Shell> {
        Box::new(self.clone())
    }

    /// The app on screen.
    pub fn top(&self) -> Option<String> {
        self.state.lock().unwrap().stack.last().cloned()
    }

    fn with<R>(&self, app: &str, f: impl FnOnce(&mut Win) -> R) -> R {
        let mut s = self.state.lock().unwrap();
        let win = s
            .windows
            .get_mut(&(app.to_string(), WindowKind::App))
            .unwrap_or_else(|| panic!("no window for app {app}"));
        f(win)
    }

    pub fn has_window(&self, app: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .windows
            .contains_key(&(app.to_string(), WindowKind::App))
    }

    /// Texts of all visible text widgets, in creation order.
    pub fn texts(&self, app: &str) -> Vec<String> {
        self.with(app, |w| {
            w.nodes
                .values()
                .filter(|n| n.kind == WidgetKind::Text && n.visible)
                .map(|n| n.text.clone())
                .collect()
        })
    }

    /// Texts of the visible text widgets drawn highlighted.
    pub fn highlighted(&self, app: &str) -> Vec<String> {
        self.with(app, |w| {
            w.nodes
                .values()
                .filter(|n| n.kind == WidgetKind::Text && n.visible && n.highlight)
                .map(|n| n.text.clone())
                .collect()
        })
    }

    /// Texts of the app's idle-screen window.
    pub fn idle_texts(&self, app: &str) -> Vec<String> {
        let s = self.state.lock().unwrap();
        s.windows
            .get(&(app.to_string(), WindowKind::Idle))
            .map(|w| w.nodes.values().filter(|n| n.kind == WidgetKind::Text).map(|n| n.text.clone()).collect())
            .unwrap_or_default()
    }

    /// The idle screen came on screen (`true`) or was covered, as the
    /// phone UI reports it to an app's idle window.
    pub fn idle_screen(&self, app: &str, shown: bool) {
        let mut s = self.state.lock().unwrap();
        if let Some(w) = s.windows.get_mut(&(app.to_string(), WindowKind::Idle)) {
            w.push(if shown { WindowEvent::Foreground } else { WindowEvent::Background });
        }
    }

    /// One of the phone's own screens went over the app's window
    /// (`true`), or left it on top again.
    pub fn cover(&self, app: &str, covered: bool) {
        self.with(app, |w| {
            w.push(if covered {
                WindowEvent::Background
            } else {
                WindowEvent::Foreground
            })
        })
    }

    /// Package pictures the app's window was handed: (name, file size).
    pub fn images(&self, app: &str) -> Vec<(String, usize)> {
        self.with(app, |w| w.images.clone())
    }

    /// The picture names of the app's image widgets, in creation order.
    pub fn image_names(&self, app: &str) -> Vec<String> {
        self.with(app, |w| w.nodes.values().filter(|n| n.kind == WidgetKind::Image).map(|n| n.image.clone()).collect())
    }

    /// True when any visible text contains `needle`.
    pub fn shows(&self, app: &str, needle: &str) -> bool {
        self.texts(app).iter().any(|t| t.contains(needle))
            || self
                .items(app)
                .iter()
                .any(|i| i.0.contains(needle) || i.1.contains(needle))
    }

    /// (text, detail) rows of the focused list, or the first list.
    pub fn items(&self, app: &str) -> Vec<(String, String)> {
        self.with(app, |w| {
            let list = w
                .focus
                .and_then(|f| w.nodes.get(&f))
                .filter(|n| n.kind == WidgetKind::TextList)
                .or_else(|| w.nodes.values().find(|n| n.kind == WidgetKind::TextList));
            list.map(|n| {
                n.items
                    .iter()
                    .map(|i| (i.text.clone(), i.detail.clone()))
                    .collect()
            })
            .unwrap_or_default()
        })
    }

    pub fn softkeys(&self, app: &str) -> Vec<String> {
        self.with(app, |w| {
            w.effective_softkeys()
                .into_iter()
                .map(|k| k.label)
                .collect()
        })
    }

    /// Values of the input widgets, in creation order.
    pub fn inputs(&self, app: &str) -> Vec<String> {
        self.with(app, |w| {
            w.nodes
                .values()
                .filter(|n| n.kind == WidgetKind::Input)
                .map(|n| n.value.clone())
                .collect()
        })
    }

    /// Presses the softkey with this label.  Panics when it is not shown.
    pub fn press_softkey(&self, app: &str, label: &str) {
        self.with(app, |w| {
            let keys = w.effective_softkeys();
            let k = keys.iter().find(|k| k.label == label).unwrap_or_else(|| {
                panic!(
                    "no softkey {label:?}; have {:?}",
                    keys.iter().map(|k| &k.label).collect::<Vec<_>>()
                )
            });
            w.push(WindowEvent::Softkey(k.id));
        })
    }

    /// Presses a key, with the toolkit's built-in behavior for unclaimed
    /// keys.
    pub fn press(&self, app: &str, key: Key) {
        self.with(app, |w| {
            if w.claimed(key) {
                w.push(WindowEvent::Key(key));
                return;
            }
            let focused = w.focus.and_then(|f| w.nodes.get_mut(&f));
            match (key, focused) {
                (Key::Cancel, _) => w.push(WindowEvent::CloseRequested),
                (Key::Up | Key::Down, Some(n)) if n.kind == WidgetKind::TextList => {
                    let len = n.items.len() as u32;
                    if len == 0 {
                        return;
                    }
                    let row = if key == Key::Down {
                        (n.selected + 1).min(len - 1)
                    } else {
                        n.selected.saturating_sub(1)
                    };
                    if row != n.selected {
                        n.selected = row;
                        let widget = n.id;
                        w.push(WindowEvent::Highlighted { widget, row });
                    }
                }
                (Key::Select, Some(n)) if n.kind == WidgetKind::TextList => {
                    if !n.items.is_empty() {
                        let (widget, row) = (n.id, n.selected);
                        w.push(WindowEvent::Activated { widget, row });
                    }
                }
                (Key::Select, Some(n)) if n.kind == WidgetKind::Input => {
                    let (widget, value) = (n.id, n.value.clone());
                    w.push(WindowEvent::Submitted { widget, value });
                }
                (k, Some(n)) if n.kind == WidgetKind::Input && k.dial_char().is_some() => {
                    n.value.push(k.dial_char().unwrap());
                    let (widget, value) = (n.id, n.value.clone());
                    w.push(WindowEvent::Changed { widget, value });
                }
                _ => {}
            }
        })
    }

    /// Replaces the text of the focused input, as if typed.
    pub fn type_text(&self, app: &str, text: &str) {
        self.with(app, |w| {
            let n = w
                .focus
                .and_then(|f| w.nodes.get_mut(&f))
                .filter(|n| n.kind == WidgetKind::Input)
                .expect("an input has focus");
            n.value = text.to_string();
            let (widget, value) = (n.id, n.value.clone());
            w.push(WindowEvent::Changed { widget, value });
        })
    }

    /// Highlights a row of the focused list.
    pub fn highlight(&self, app: &str, row: u32) {
        self.with(app, |w| {
            let n = w
                .focus
                .and_then(|f| w.nodes.get_mut(&f))
                .filter(|n| n.kind == WidgetKind::TextList)
                .expect("a list has focus");
            n.selected = row;
            let widget = n.id;
            w.push(WindowEvent::Highlighted { widget, row });
        })
    }

    /// Number of live widgets (to check apps do not leak them).
    pub fn widget_count(&self, app: &str) -> usize {
        self.with(app, |w| w.nodes.len())
    }
}

struct FakeWindow {
    ui: FakeUi,
    key: (String, WindowKind),
}

impl FakeWindow {
    fn with<R>(&self, f: impl FnOnce(&mut Win) -> R) -> R {
        let mut s = self.ui.state.lock().unwrap();
        f(s.windows.entry(self.key.clone()).or_default())
    }
    fn node(&self, w: u32, f: impl FnOnce(&mut Node)) {
        self.with(|win| {
            if let Some(n) = win.nodes.get_mut(&w) {
                f(n)
            }
        })
    }
}

impl AppWindow for FakeWindow {
    fn metrics(&self) -> Metrics {
        metrics_for(self.ui.model.unwrap_or(dphone_state::Model::D60))
    }
    fn create(&mut self, kind: WidgetKind, parent: Option<u32>, bounds: Rect) -> u32 {
        self.with(|w| {
            w.next += 1;
            let id = w.next;
            w.nodes.insert(
                id,
                Node {
                    id,
                    kind,
                    parent,
                    bounds,
                    visible: true,
                    text: String::new(),
                    font: Font::Normal,
                    wrap: false,
                    highlight: false,
                    items: Vec::new(),
                    selected: 0,
                    value: String::new(),
                    mode: InputMode::Text,
                    max_len: 0,
                    image: String::new(),
                    softkeys: Vec::new(),
                    claims: Vec::new(),
                },
            );
            id
        })
    }
    fn destroy(&mut self, id: u32) {
        self.with(|w| {
            for d in w.descendants(id) {
                w.nodes.remove(&d);
            }
            w.nodes.remove(&id);
            if w.focus.is_some_and(|f| !w.nodes.contains_key(&f)) {
                w.focus = None;
            }
        })
    }
    fn clear(&mut self, id: Option<u32>) {
        self.with(|w| match id {
            None => {
                w.nodes.clear();
                w.focus = None;
                w.softkeys.clear();
                w.claims.clear();
            }
            Some(id) => {
                for d in w.descendants(id) {
                    w.nodes.remove(&d);
                }
                if w.focus.is_some_and(|f| !w.nodes.contains_key(&f)) {
                    w.focus = None;
                }
            }
        })
    }
    fn set_bounds(&mut self, w: u32, bounds: Rect) {
        self.node(w, |n| n.bounds = bounds)
    }
    fn set_visible(&mut self, w: u32, visible: bool) {
        self.node(w, |n| n.visible = visible)
    }
    fn set_focus(&mut self, id: Option<u32>) {
        self.with(|w| w.focus = id.filter(|i| w.nodes.contains_key(i)))
    }
    fn set_text(&mut self, w: u32, text: &str) {
        self.node(w, |n| n.text = text.to_string())
    }
    fn set_align(&mut self, _w: u32, _align: Align) {}
    fn set_font(&mut self, w: u32, font: Font) {
        self.node(w, |n| n.font = font)
    }
    fn set_wrap(&mut self, w: u32, wrap: bool) {
        self.node(w, |n| n.wrap = wrap)
    }
    fn set_highlight(&mut self, w: u32, on: bool) {
        self.node(w, |n| n.highlight = on)
    }
    fn set_items(&mut self, w: u32, items: &[ListItem]) {
        self.node(w, |n| {
            n.items = items.to_vec();
            n.selected = n.selected.min(items.len().saturating_sub(1) as u32);
        })
    }
    fn set_selected(&mut self, w: u32, index: u32) {
        self.node(w, |n| {
            n.selected = index.min(n.items.len().saturating_sub(1) as u32)
        })
    }
    fn selected(&self, w: u32) -> u32 {
        self.with(|win| win.nodes.get(&w).map(|n| n.selected).unwrap_or(0))
    }
    fn set_value(&mut self, w: u32, value: &str) {
        self.node(w, |n| n.value = value.to_string())
    }
    fn value(&self, w: u32) -> String {
        self.with(|win| {
            win.nodes
                .get(&w)
                .map(|n| n.value.clone())
                .unwrap_or_default()
        })
    }
    fn set_input_mode(&mut self, w: u32, mode: InputMode) {
        self.node(w, |n| n.mode = mode)
    }
    fn set_max_length(&mut self, w: u32, max: u32) {
        self.node(w, |n| n.max_len = max)
    }
    fn set_image(&mut self, w: u32, name: &str) {
        self.node(w, |n| n.image = name.to_string())
    }
    fn define_image(&mut self, name: &str, file: &[u8]) -> bool {
        self.with(|w| w.images.push((name.to_string(), file.len())));
        file.starts_with(b"\x89PNG")
    }
    fn set_softkeys(&mut self, id: Option<u32>, keys: &[Softkey]) {
        self.with(|w| match id {
            None => w.softkeys = keys.to_vec(),
            Some(id) => {
                if let Some(n) = w.nodes.get_mut(&id) {
                    n.softkeys = keys.to_vec()
                }
            }
        })
    }
    fn claim_keys(&mut self, id: Option<u32>, keys: &[Key]) {
        self.with(|w| match id {
            None => w.claims = keys.to_vec(),
            Some(id) => {
                if let Some(n) = w.nodes.get_mut(&id) {
                    n.claims = keys.to_vec()
                }
            }
        })
    }
    fn poll_event(&mut self) -> Option<WindowEvent> {
        self.with(|w| w.events.pop_front())
    }
}

impl Shell for FakeUi {
    fn open(
        &mut self,
        app: &str,
        kind: WindowKind,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Box<dyn AppWindow> {
        let key = (app.to_string(), kind);
        let mut s = self.state.lock().unwrap();
        s.windows.insert(
            key.clone(),
            Win {
                wake: Some(wake),
                ..Win::default()
            },
        );
        Box::new(FakeWindow {
            ui: self.clone(),
            key,
        })
    }
    fn show(&mut self, app: &str) {
        let mut s = self.state.lock().unwrap();
        if s.stack.last().map(String::as_str) == Some(app) {
            return;
        }
        if let Some(prev) = s.stack.last().cloned()
            && let Some(w) = s.windows.get_mut(&(prev, WindowKind::App))
        {
            w.push(WindowEvent::Background);
        }
        s.stack.retain(|a| a != app);
        s.stack.push(app.to_string());
        if let Some(w) = s.windows.get_mut(&(app.to_string(), WindowKind::App)) {
            w.push(WindowEvent::Foreground);
        }
    }
    fn hide(&mut self, app: &str) {
        let mut s = self.state.lock().unwrap();
        let was_top = s.stack.last().map(String::as_str) == Some(app);
        s.stack.retain(|a| a != app);
        if was_top {
            if let Some(w) = s.windows.get_mut(&(app.to_string(), WindowKind::App)) {
                w.push(WindowEvent::Background);
            }
            if let Some(next) = s.stack.last().cloned()
                && let Some(w) = s.windows.get_mut(&(next, WindowKind::App))
            {
                w.push(WindowEvent::Foreground);
            }
        }
    }
    fn close(&mut self, app: &str, kind: WindowKind) {
        let mut s = self.state.lock().unwrap();
        s.windows.remove(&(app.to_string(), kind));
        if kind == WindowKind::App {
            s.stack.retain(|a| a != app);
        }
    }
    fn idle_size(&self) -> Size {
        Size {
            width: 200,
            height: 40,
        }
    }
}

type Handler = Box<dyn Fn(&Value) -> Result<Value, Value> + Send + Sync>;

/// A scripted PBX: answers each method from a handler, at once.
#[derive(Clone, Default)]
pub struct FakePbx {
    handlers: Arc<Mutex<HashMap<String, Handler>>>,
    log: Arc<Mutex<Vec<(String, Value)>>>,
    seen: Arc<Mutex<Vec<PbxRequest>>>,
}

impl FakePbx {
    /// `f` gets the request parameters and returns `response.result`, or
    /// `Err` with `response.errors`.
    pub fn on(
        &self,
        method: &str,
        f: impl Fn(&Value) -> Result<Value, Value> + Send + Sync + 'static,
    ) {
        self.handlers
            .lock()
            .unwrap()
            .insert(method.to_string(), Box::new(f));
    }

    /// (method, parameters) of every request so far.
    pub fn requests(&self) -> Vec<(String, Value)> {
        self.log.lock().unwrap().clone()
    }

    pub fn raw_requests(&self) -> Vec<PbxRequest> {
        self.seen.lock().unwrap().clone()
    }

    /// Parameters of the last request for a method.
    pub fn last(&self, method: &str) -> Option<Value> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(m, _)| m == method)
            .map(|(_, p)| p.clone())
    }

    pub fn count(&self, method: &str) -> usize {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == method)
            .count()
    }
}

impl PbxRequests for FakePbx {
    fn request(&self, request: PbxRequest, done: Completion<Result<Vec<u8>, String>>) {
        self.seen.lock().unwrap().push(request.clone());
        let v: Value = serde_json::from_str(&request.body).unwrap_or(Value::Null);
        let method = v["request"]["method"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let params = v["request"]["parameters"].clone();
        self.log
            .lock()
            .unwrap()
            .push((method.clone(), params.clone()));
        let reply = match self.handlers.lock().unwrap().get(&method) {
            Some(h) => match h(&params) {
                Ok(result) => serde_json::json!({"response": {"method": method, "result": result}}),
                Err(errors) => {
                    serde_json::json!({"response": {"method": method, "errors": errors}})
                }
            },
            None => serde_json::json!({"response": {"method": method, "errors": {"error":
                {"code": "10001", "message": "Unknown method"}}}}),
        };
        done.complete(Ok(reply.to_string().into_bytes()));
    }
}

/// An HTTP client with no network.
pub struct NoHttp;

impl HttpClient for NoHttp {
    fn request(&self, _request: HttpRequest, done: Completion<Result<HttpReply, String>>) {
        done.complete(Err("no network in this test".into()));
    }
}

/// A fixed configuration.
#[derive(Clone, Default)]
pub struct FakeConfig {
    pub account: Option<AppAccount>,
    pub app_settings: HashMap<String, String>,
    pub contact_lists: Vec<(String, String)>,
    /// Built-in apps the configuration switches off.  A test may change
    /// the list while apps run.
    pub disabled: Arc<Mutex<Vec<String>>>,
    /// Local time's offset from UTC, in seconds.
    pub utc_offset: i32,
    /// The wall clock apps see, in seconds since the Unix epoch (`None`:
    /// the real one).  A test may change it while apps run.
    pub wall_time: Option<Arc<std::sync::atomic::AtomicU64>>,
}

impl FakeConfig {
    /// Every app shares one DPMA application account.
    pub fn dasp() -> FakeConfig {
        FakeConfig {
            account: Some(AppAccount {
                account_id: "100".into(),
                username: "100".into(),
                password: "secret".into(),
                appserver: "dasp://pbx.example/json".into(),
                apiformat: "json".into(),
            }),
            ..Default::default()
        }
    }
}

impl ConfigSource for FakeConfig {
    fn app_config_xml(&self, app: &str) -> String {
        format!("<application id=\"{app}\"/>")
    }
    fn app_account(&self, _app: &str) -> Option<AppAccount> {
        self.account.clone()
    }
    fn app_setting(&self, _app: &str, name: &str) -> Option<String> {
        self.app_settings.get(name).cloned()
    }
    fn app_enabled(&self, app: &str) -> bool {
        !self.disabled.lock().unwrap().iter().any(|d| d == app)
    }
    fn utc_offset_secs(&self) -> i32 {
        self.utc_offset
    }
    fn wall_time(&self) -> u64 {
        match &self.wall_time {
            Some(t) => t.load(std::sync::atomic::Ordering::Relaxed),
            None => dphone_state::clock::unix_secs(),
        }
    }
    fn setting(&self, _id: &str) -> Option<String> {
        None
    }
    fn mac_address(&self) -> String {
        "000fd3000001".into()
    }
    fn contact_lists(&self) -> Vec<(String, String)> {
        self.contact_lists.clone()
    }
}

pub fn services(pbx: &FakePbx, config: FakeConfig) -> Services {
    Services {
        pbx: Arc::new(pbx.clone()),
        http: Arc::new(NoHttp),
        config: Arc::new(config),
    }
}
