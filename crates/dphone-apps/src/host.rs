//! The host side of `wit/app.wit`: per-app context and the implementation
//! of every imported interface.

use crate::manifest::{Capability, Manifest};
use crate::services::{APP_VISIBLE_SETTINGS, Completion, HttpRequest, PbxRequest, Services};
use crate::view::AppWindow;
use dphone_state::{
    AccountId, AudioPath, BlfState, Call, CallId, CallState as StCallState, Command, Direction,
    Key, PresenceStatus, Reply, Store,
};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub(crate) mod bindings {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "app",
    });
}

use bindings::dphone::app as api;
use bindings::dphone::app::types::*;

/// Widget handles with this bit belong to the idle window.
pub(crate) const IDLE_BIT: u32 = 1 << 31;
/// Free space always left on a storage filesystem (stock: 2,048 KiB).
pub const FREE_RESERVE: u64 = 2048 * 1024;

/// Wakes the manager's loop from any thread.
#[derive(Default)]
pub(crate) struct Notifier {
    flag: Mutex<bool>,
    cv: Condvar,
}

impl Notifier {
    pub fn notify(&self) {
        *self.flag.lock().unwrap() = true;
        self.cv.notify_all();
    }
    /// Waits until notified or the timeout passes; clears the flag.
    pub fn wait(&self, timeout: Duration) {
        let g = self.flag.lock().unwrap();
        let (mut g, _) = self.cv.wait_timeout_while(g, timeout, |set| !*set).unwrap();
        *g = false;
    }
}

impl std::task::Wake for Notifier {
    fn wake(self: Arc<Self>) {
        self.notify();
    }
}

/// A result that arrived from another thread.
pub(crate) enum Inbound {
    Pbx {
        app: String,
        generation: u64,
        id: u32,
        outcome: PbxOutcome,
    },
    Http {
        app: String,
        generation: u64,
        id: u32,
        result: Result<HttpResponse, String>,
    },
}

#[derive(Default)]
pub(crate) struct Inbox {
    pub queue: Mutex<VecDeque<Inbound>>,
    pub notifier: Arc<Notifier>,
}

impl Inbox {
    pub fn push(&self, item: Inbound) {
        self.queue.lock().unwrap().push_back(item);
        self.notifier.notify();
    }
}

/// Something an app asked the manager to do, applied after the guest call
/// returns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Background,
    Exit,
    Restart,
    ShowApp(String),
    Send {
        app: String,
        name: String,
        payload: String,
    },
}

pub(crate) struct Timer {
    pub id: u32,
    pub deadline_ms: u64,
    pub repeat_ms: Option<u32>,
}

/// Storage roots.
#[derive(Clone, Debug)]
pub struct Dirs {
    /// The app's package directory (`app`).
    pub package: PathBuf,
    /// `/nvdata/appdata`.
    pub nv_root: PathBuf,
    /// `/tmp/appdata`.
    pub tmp_root: PathBuf,
    /// Shared read-only data (`data`).
    pub data: PathBuf,
}

pub(crate) struct AppCtx {
    pub name: String,
    pub manifest: Manifest,
    /// Distinguishes this instance from an earlier one of the same app, so
    /// late replies for a dead instance are dropped.
    pub generation: u64,
    pub dirs: Dirs,
    pub window: Box<dyn AppWindow>,
    pub idle: Option<Box<dyn AppWindow>>,
    pub idle_size: Size,
    pub store: Store,
    pub services: Services,
    pub inbox: Arc<Inbox>,
    pub actions: Vec<Action>,
    pub timers: Vec<Timer>,
    pub next_timer: u32,
    pub next_request: u32,
    pub pending: HashSet<u32>,
    pub observed: HashSet<String>,
    pub observe_calls: bool,
    pub presence_subs: HashSet<String>,
    pub exit_after_background: bool,
    pub foreground: bool,
    /// Picture names already looked for in the package, per window (idle
    /// or not), so each file is read once.
    pub images: HashSet<(bool, String)>,
    /// A sound file of this app is playing.
    pub playing: bool,
    pub limits: wasmtime::StoreLimits,
}

/// The largest picture file read from a package.
const IMAGE_FILE_MAX: u64 = 512 * 1024;

fn bad_name(name: &str) -> bool {
    name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0'])
}

fn io_error(e: std::io::Error) -> Error {
    match e.kind() {
        std::io::ErrorKind::NotFound => Error::NotFound,
        std::io::ErrorKind::StorageFull => Error::NoSpace,
        _ => Error::Failed(e.to_string()),
    }
}

/// Bytes a write may still use, honoring the reserve.
// The statvfs field widths differ between 32-bit and 64-bit targets.
#[allow(clippy::unnecessary_cast)]
pub(crate) fn usable_space(dir: &Path) -> u64 {
    match rustix::fs::statvfs(dir) {
        Ok(s) => (s.f_bavail as u64)
            .saturating_mul(s.f_frsize as u64)
            .saturating_sub(FREE_RESERVE),
        Err(_) => 0,
    }
}

pub(crate) fn call_info(c: &Call) -> CallInfo {
    let state = match c.state {
        StCallState::Dialing => CallState::Dialing,
        StCallState::Proceeding => CallState::Proceeding,
        StCallState::Ringback { .. } => CallState::Ringback,
        StCallState::Ringing => CallState::Ringing,
        StCallState::Active => CallState::Active,
        StCallState::Held { .. } => CallState::Held,
        StCallState::Transferring => CallState::Transferring,
        StCallState::Ended(_) => CallState::Ended,
    };
    let duration = c
        .connected_mono_ms
        .map(|t| (dphone_state::clock::mono_ms().saturating_sub(t) / 1000) as u32)
        .unwrap_or(0);
    CallInfo {
        id: c.id.0,
        account: c.account.0,
        incoming: c.direction == Direction::Incoming,
        state,
        remote_name: c.remote.name.clone(),
        remote_number: c.remote.number.clone(),
        duration,
    }
}

pub(crate) fn presence_of(e: &dphone_state::BlfEntry, target: &str) -> Presence {
    let line_state = match e.state {
        BlfState::Unknown => "unknown",
        BlfState::Idle => "idle",
        BlfState::Ringing => "ringing",
        BlfState::InUse => "busy",
        BlfState::OnHold => "hold",
        BlfState::OnHoldRinging => "hold-ringing",
        BlfState::Unavailable => "unavailable",
    };
    let status = match e.status {
        PresenceStatus::Unknown => "unknown",
        s => s.token(),
    };
    Presence {
        target: target.to_string(),
        status: status.to_string(),
        text: e.status_text.clone(),
        line_state: line_state.to_string(),
    }
}

/// True when a BLF entry is the one an app means by `target` (an
/// extension, a contact id, or a URI).
pub(crate) fn entry_matches(e: &dphone_state::BlfEntry, target: &str) -> bool {
    e.number == target || e.id == target || e.uri == target
}

/// The reply document of a PBX request, reduced to what the app gets.
pub(crate) fn pbx_outcome(reply: Result<Vec<u8>, String>) -> PbxOutcome {
    let bytes = match reply {
        Ok(b) => b,
        Err(e) => return PbxOutcome::Transport(e),
    };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return PbxOutcome::Transport("the server's reply was not JSON".into());
    };
    let Some(r) = v.get("response") else {
        return PbxOutcome::Transport("the server's reply had no response".into());
    };
    if let Some(e) = r.get("errors") {
        return PbxOutcome::Errors(e.to_string());
    }
    match r.get("result") {
        Some(res) => PbxOutcome::Ok(res.to_string()),
        None => PbxOutcome::Transport("the server's reply had no result".into()),
    }
}

/// Days-from-civil inverse: (year, month, day) for days since 1970-01-01.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

pub(crate) fn format_time(unix: u64, offset: i32, h24: bool, style: u8) -> String {
    let t = unix as i64 + offset as i64;
    let (days, secs) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    let (y, m, d) = civil(days);
    let (hh, mm) = (secs / 3600, secs % 3600 / 60);
    let time = if h24 {
        format!("{hh:02}:{mm:02}")
    } else {
        let h12 = match hh % 12 {
            0 => 12,
            h => h,
        };
        format!("{h12}:{mm:02} {}", if hh < 12 { "AM" } else { "PM" })
    };
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let date = format!("{} {d}, {y}", MONTHS[(m - 1) as usize]);
    match style {
        0 => time,
        1 => date,
        _ => format!("{date} {time}"),
    }
}

impl AppCtx {
    fn win(&mut self, w: u32) -> Option<(&mut (dyn AppWindow + 'static), Option<u32>)> {
        if w & IDLE_BIT != 0 {
            let id = w & !IDLE_BIT;
            self.idle
                .as_deref_mut()
                .map(|i| (i, (id != 0).then_some(id)))
        } else {
            Some((self.window.as_mut(), (w != 0).then_some(w)))
        }
    }

    /// Runs `f` on the window owning `w` when `w` names a real widget.
    fn on_widget(&mut self, w: u32, f: impl FnOnce(&mut dyn AppWindow, u32)) {
        if let Some((win, Some(id))) = self.win(w) {
            f(win, id);
        }
    }

    fn resolve(&self, loc: Location, name: &str, write: bool) -> Result<PathBuf, Error> {
        if bad_name(name) {
            return Err(Error::Invalid(
                "file names cannot contain path separators".into(),
            ));
        }
        let dir = self.location_dir(loc, write)?;
        let path = dir.join(name);
        if let Ok(meta) = std::fs::symlink_metadata(&path)
            && meta.file_type().is_symlink()
        {
            return Err(Error::Denied);
        }
        Ok(path)
    }

    fn location_dir(&self, loc: Location, write: bool) -> Result<PathBuf, Error> {
        Ok(match loc {
            Location::App | Location::Data if write => return Err(Error::Denied),
            Location::App => self.dirs.package.clone(),
            Location::Data => self.dirs.data.clone(),
            Location::Nv => self.dirs.nv_root.join(&self.name),
            Location::Nvshared => self.dirs.nv_root.join("@shared"),
            Location::Tmp => self.dirs.tmp_root.join(&self.name),
            Location::Tmpshared => self.dirs.tmp_root.join("@shared"),
        })
    }

    /// Hands the window the package's picture file called `name`, the
    /// first time the app names it.  A name that is no file of the package
    /// stays what it was: one of the phone's icons.
    fn load_image(&mut self, w: u32, name: &str) {
        let idle = w & IDLE_BIT != 0;
        if bad_name(name) || !self.images.insert((idle, name.to_string())) {
            return;
        }
        let path = self.dirs.package.join(name);
        let usable = std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_file() && m.len() <= IMAGE_FILE_MAX);
        if !usable {
            return;
        }
        let Ok(bytes) = std::fs::read(&path) else { return };
        let app = self.name.clone();
        if let Some((win, _)) = self.win(w)
            && !win.define_image(name, &bytes)
        {
            log::warn!("app {app}: {name} is not a usable picture");
        }
    }

    fn command(&self, c: Command) -> Result<Reply, Error> {
        self.store
            .send(c)
            .wait_timeout(Duration::from_millis(500))
            .map_err(|e| Error::Failed(e.to_string()))
    }

    pub fn has(&self, cap: Capability) -> bool {
        self.manifest.has(cap)
    }
}

impl api::types::Host for AppCtx {}

impl api::runtime::Host for AppCtx {
    fn background(&mut self) {
        self.actions.push(Action::Background);
    }
    fn exit(&mut self) {
        self.actions.push(Action::Exit);
    }
    fn restart(&mut self) {
        self.actions.push(Action::Restart);
    }
    fn set_exit_after_background(&mut self, exit: bool) {
        self.exit_after_background = exit;
    }
    fn show_app(&mut self, name: String) -> Result<(), String> {
        if crate::manifest::validate_name(&name).is_err() {
            return Err("bad app name".into());
        }
        self.actions.push(Action::ShowApp(name));
        Ok(())
    }
    fn reset_idle_timer(&mut self) {
        // As on stock: only the app on screen keeps the phone awake.
        if self.foreground {
            let _ = self.store.post(Command::ResetIdleTimer);
        }
    }
    fn log(&mut self, level: u8, message: String) {
        let level = match level {
            0 => log::Level::Error,
            1 => log::Level::Warn,
            2 => log::Level::Info,
            _ => log::Level::Debug,
        };
        let mut msg = message;
        if msg.len() > 512 {
            let mut end = 512;
            while !msg.is_char_boundary(end) {
                end -= 1;
            }
            msg.truncate(end);
        }
        log::log!(target: "app", level, "{}: {}", self.name, msg);
    }
    fn in_foreground(&mut self) -> bool {
        self.foreground
    }
}

impl api::ui::Host for AppCtx {
    fn get_metrics(&mut self) -> Metrics {
        self.window.metrics()
    }
    fn idle_size(&mut self) -> Size {
        self.idle_size
    }
    fn idle_root(&mut self) -> Option<u32> {
        self.idle.is_some().then_some(IDLE_BIT)
    }
    fn create(&mut self, kind: WidgetKind, parent: u32, bounds: Rect) -> u32 {
        let idle = parent & IDLE_BIT;
        match self.win(parent) {
            Some((win, p)) => win.create(kind, p, bounds) | idle,
            None => 0,
        }
    }
    fn destroy(&mut self, w: u32) {
        self.on_widget(w, |win, id| win.destroy(id));
    }
    fn clear(&mut self, w: u32) {
        if let Some((win, id)) = self.win(w) {
            win.clear(id);
        }
    }
    fn set_bounds(&mut self, w: u32, bounds: Rect) {
        self.on_widget(w, |win, id| win.set_bounds(id, bounds));
    }
    fn set_visible(&mut self, w: u32, visible: bool) {
        self.on_widget(w, |win, id| win.set_visible(id, visible));
    }
    fn set_focus(&mut self, w: u32) {
        if let Some((win, id)) = self.win(w) {
            win.set_focus(id);
        }
    }
    fn set_text(&mut self, w: u32, text: String) {
        self.on_widget(w, |win, id| win.set_text(id, &text));
    }
    fn set_align(&mut self, w: u32, align: Align) {
        self.on_widget(w, |win, id| win.set_align(id, align));
    }
    fn set_font(&mut self, w: u32, font: Font) {
        self.on_widget(w, |win, id| win.set_font(id, font));
    }
    fn set_wrap(&mut self, w: u32, wrap: bool) {
        self.on_widget(w, |win, id| win.set_wrap(id, wrap));
    }
    fn set_items(&mut self, w: u32, items: Vec<ListItem>) {
        for item in items.iter().filter(|i| !i.icon.is_empty()) {
            self.load_image(w, &item.icon);
        }
        self.on_widget(w, |win, id| win.set_items(id, &items));
    }
    fn set_selected(&mut self, w: u32, index: u32) {
        self.on_widget(w, |win, id| win.set_selected(id, index));
    }
    fn selected(&mut self, w: u32) -> u32 {
        match self.win(w) {
            Some((win, Some(id))) => win.selected(id),
            _ => 0,
        }
    }
    fn set_value(&mut self, w: u32, value: String) {
        self.on_widget(w, |win, id| win.set_value(id, &value));
    }
    fn value(&mut self, w: u32) -> String {
        match self.win(w) {
            Some((win, Some(id))) => win.value(id),
            _ => String::new(),
        }
    }
    fn set_input_mode(&mut self, w: u32, mode: InputMode) {
        self.on_widget(w, |win, id| win.set_input_mode(id, mode));
    }
    fn set_max_length(&mut self, w: u32, max: u32) {
        self.on_widget(w, |win, id| win.set_max_length(id, max));
    }
    fn set_image(&mut self, w: u32, name: String) {
        if !bad_name(&name) {
            self.load_image(w, &name);
            self.on_widget(w, |win, id| win.set_image(id, &name));
        }
    }
    fn claim_keys(&mut self, w: u32, keys: Vec<String>) {
        let keys: Vec<Key> = keys.iter().filter_map(|k| Key::from_name(k)).collect();
        if let Some((win, id)) = self.win(w) {
            win.claim_keys(id, &keys);
        }
    }
    fn set_softkeys(&mut self, w: u32, keys: Vec<Softkey>) {
        if let Some((win, id)) = self.win(w) {
            win.set_softkeys(id, &keys);
        }
    }
}

impl api::timers::Host for AppCtx {
    fn set(&mut self, delay_ms: u32, repeat: bool) -> u32 {
        // A handful of timers is plenty; refuse to grow without bound.
        if self.timers.len() >= 32 {
            return 0;
        }
        self.next_timer = self.next_timer.wrapping_add(1).max(1);
        let id = self.next_timer;
        // Repeating timers faster than 50 ms would starve the phone.
        let delay = if repeat { delay_ms.max(50) } else { delay_ms };
        self.timers.push(Timer {
            id,
            deadline_ms: dphone_state::clock::mono_ms() + delay as u64,
            repeat_ms: repeat.then_some(delay),
        });
        id
    }
    fn cancel(&mut self, id: u32) {
        self.timers.retain(|t| t.id != id);
    }
    fn now_ms(&mut self) -> u64 {
        dphone_state::clock::mono_ms()
    }
    fn wall_time(&mut self) -> u64 {
        dphone_state::clock::unix_secs()
    }
}

impl api::storage::Host for AppCtx {
    fn read(&mut self, loc: Location, name: String) -> Result<Vec<u8>, Error> {
        let path = self.resolve(loc, &name, false)?;
        let meta = std::fs::metadata(&path).map_err(io_error)?;
        // An app's memory is capped; do not even try to hand it more.
        if !meta.is_file() || meta.len() > 1 << 20 {
            return Err(Error::Invalid("not a regular file under 1 MiB".into()));
        }
        std::fs::read(&path).map_err(io_error)
    }
    fn write(&mut self, loc: Location, name: String, data: Vec<u8>) -> Result<(), Error> {
        let path = self.resolve(loc, &name, true)?;
        let dir = path.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).map_err(io_error)?;
        let old = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if (data.len() as u64).saturating_sub(old) > usable_space(&dir) {
            return Err(Error::NoSpace);
        }
        let tmp = dir.join(format!(".{name}.tmp"));
        let result = (|| {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&data)?;
            f.sync_all()?;
            std::fs::rename(&tmp, &path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result.map_err(io_error)
    }
    fn remove(&mut self, loc: Location, name: String) -> Result<(), Error> {
        let path = self.resolve(loc, &name, true)?;
        std::fs::remove_file(path).map_err(io_error)
    }
    fn exists(&mut self, loc: Location, name: String) -> bool {
        self.resolve(loc, &name, false)
            .map(|p| p.is_file())
            .unwrap_or(false)
    }
    fn list(&mut self, loc: Location) -> Result<Vec<String>, Error> {
        let dir = self.location_dir(loc, false)?;
        let mut names = Vec::new();
        match std::fs::read_dir(&dir) {
            Ok(rd) => {
                for e in rd.flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let is_file = e.file_type().map(|t| t.is_file()).unwrap_or(false);
                    if is_file && !name.starts_with('.') {
                        names.push(name);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(e)),
        }
        names.sort();
        Ok(names)
    }
}

impl api::config::Host for AppCtx {
    fn app_config(&mut self) -> String {
        self.services.config.app_config_xml(&self.name)
    }
    fn app_setting(&mut self, name: String) -> Option<String> {
        // Credentials stay with the host: it adds them to PBX requests.
        if matches!(name.as_str(), "password" | "secret") {
            return None;
        }
        if name == "account_id"
            && let Some(a) = self.services.config.app_account(&self.name)
        {
            return Some(a.account_id);
        }
        self.services.config.app_setting(&self.name, &name)
    }
    fn setting(&mut self, id: String) -> Option<String> {
        APP_VISIBLE_SETTINGS
            .contains(&id.as_str())
            .then(|| self.services.config.setting(&id))
            .flatten()
    }
    fn accounts(&mut self) -> Vec<AccountInfo> {
        self.store
            .get::<dphone_state::Accounts>()
            .accounts
            .iter()
            .map(|a| AccountInfo {
                slot: a.id.0,
                id: a.config_id.clone(),
                label: a.label.clone(),
                user: a.user.clone(),
                server: a.server.clone(),
                registered: a.registration.is_registered(),
            })
            .collect()
    }
    fn model(&mut self) -> String {
        self.store.model().name().to_string()
    }
    fn firmware_version(&mut self) -> String {
        self.store
            .get::<dphone_state::System>()
            .version
            .underscored()
    }
    fn mac_address(&mut self) -> String {
        self.services.config.mac_address()
    }
    fn contact_lists(&mut self) -> Vec<(String, String)> {
        self.services.config.contact_lists()
    }
}

impl AppCtx {
    fn next_request(&mut self) -> u32 {
        self.next_request = self.next_request.wrapping_add(1).max(1);
        self.pending.insert(self.next_request);
        self.next_request
    }
}

impl api::pbx::Host for AppCtx {
    fn request(&mut self, method: String, parameters: String) -> u32 {
        let id = self.next_request();
        let inbox = self.inbox.clone();
        let (app, generation) = (self.name.clone(), self.generation);
        let fail = |inbox: &Arc<crate::host::Inbox>, why: &str| {
            inbox.push(Inbound::Pbx {
                app: app.clone(),
                generation,
                id,
                outcome: PbxOutcome::Transport(why.to_string()),
            });
        };
        let Some(account) = self.services.config.app_account(&self.name) else {
            fail(&inbox, "this app has no application account configured");
            return id;
        };
        let params: serde_json::Value = match serde_json::from_str(&parameters) {
            Ok(v @ serde_json::Value::Object(_)) => v,
            _ => {
                fail(&inbox, "parameters must be a JSON object");
                return id;
            }
        };
        let body =
            serde_json::json!({"request": {"method": method, "parameters": params}}).to_string();
        if account.appserver.starts_with("dasp://") {
            let done = {
                let app = app.clone();
                Completion::new(move |reply| {
                    inbox.push(Inbound::Pbx {
                        app,
                        generation,
                        id,
                        outcome: pbx_outcome(reply),
                    })
                })
            };
            self.services.pbx.request(
                PbxRequest {
                    url: account.appserver,
                    username: account.username,
                    password: account.password,
                    body,
                    app: self.name.clone(),
                },
                done,
            );
        } else if account.appserver.starts_with("http://")
            || account.appserver.starts_with("https://")
        {
            let done = {
                let app = app.clone();
                Completion::new(move |reply: Result<crate::HttpReply, String>| {
                    let outcome = match reply {
                        Ok(r) if (200..300).contains(&r.status) => pbx_outcome(Ok(r.body)),
                        Ok(r) => PbxOutcome::Transport(format!("HTTP status {}", r.status)),
                        Err(e) => PbxOutcome::Transport(e),
                    };
                    inbox.push(Inbound::Pbx {
                        app,
                        generation,
                        id,
                        outcome,
                    })
                })
            };
            self.services.http.request(
                HttpRequest {
                    method: "POST".into(),
                    url: account.appserver,
                    headers: vec![("Content-Type".into(), "application/json".into())],
                    body: body.into_bytes(),
                    credentials: Some((account.username, account.password)),
                },
                done,
            );
        } else {
            fail(&inbox, "the application server address is not usable");
        }
        id
    }
    fn cancel(&mut self, id: u32) {
        self.pending.remove(&id);
    }
}

impl api::http::Host for AppCtx {
    fn request(&mut self, method: String, url: String, headers: Vec<Header>, body: Vec<u8>) -> u32 {
        let id = self.next_request();
        let inbox = self.inbox.clone();
        let (app, generation) = (self.name.clone(), self.generation);
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            inbox.push(Inbound::Http {
                app,
                generation,
                id,
                result: Err("only http and https URLs are allowed".into()),
            });
            return id;
        }
        let done = Completion::new(move |reply: Result<crate::HttpReply, String>| {
            let result = reply.map(|r| HttpResponse {
                status: r.status,
                headers: r
                    .headers
                    .into_iter()
                    .map(|(name, value)| Header { name, value })
                    .collect(),
                body: r.body,
            });
            inbox.push(Inbound::Http {
                app,
                generation,
                id,
                result,
            })
        });
        self.services.http.request(
            HttpRequest {
                method,
                url,
                headers: headers.into_iter().map(|h| (h.name, h.value)).collect(),
                body,
                credentials: None,
            },
            done,
        );
        id
    }
    fn cancel(&mut self, id: u32) {
        self.pending.remove(&id);
    }
}

impl api::calls::Host for AppCtx {
    fn dial(
        &mut self,
        number: String,
        account: Option<u8>,
        headers: Vec<Header>,
    ) -> Result<u32, Error> {
        if number.is_empty() || number.len() > 256 {
            return Err(Error::Invalid("bad number".into()));
        }
        let command = if headers.is_empty() {
            Command::Dial {
                target: number,
                account: account.map(AccountId),
                path: None::<AudioPath>,
            }
        } else {
            Command::DialWith {
                target: number,
                account: account.map(AccountId),
                headers: headers.into_iter().map(|h| (h.name, h.value)).collect(),
            }
        };
        match self.command(command)? {
            Reply::Call(id) => Ok(id.0),
            _ => Err(Error::Failed("the phone did not start a call".into())),
        }
    }
    fn answer(&mut self, call: u32) -> Result<(), Error> {
        self.command(Command::Answer {
            call: CallId(call),
            path: None,
        })
        .map(|_| ())
    }
    fn reject(&mut self, call: u32) -> Result<(), Error> {
        self.command(Command::Reject {
            call: CallId(call),
            busy: false,
        })
        .map(|_| ())
    }
    fn hangup(&mut self, call: u32) -> Result<(), Error> {
        self.command(Command::Hangup { call: CallId(call) })
            .map(|_| ())
    }
    fn send_dtmf(&mut self, call: u32, digits: String) -> Result<(), Error> {
        if digits.len() > 64 || !digits.chars().all(|c| Key::from_dial_char(c).is_some()) {
            return Err(Error::Invalid("digits must be 0-9, *, or #".into()));
        }
        for digit in digits.chars() {
            self.command(Command::Dtmf {
                call: CallId(call),
                digit,
            })?;
        }
        Ok(())
    }
    fn list(&mut self) -> Vec<CallInfo> {
        self.store
            .get::<dphone_state::Calls>()
            .calls
            .iter()
            .map(call_info)
            .collect()
    }
    fn observe(&mut self, on: bool) {
        self.observe_calls = on;
    }
    fn get_mwi(&mut self) -> Vec<Mwi> {
        self.store
            .get::<dphone_state::Mwi>()
            .accounts
            .iter()
            .map(|m| Mwi {
                account: m.account.0,
                new_messages: m.new,
                old_messages: m.old,
            })
            .collect()
    }
    fn dial_voicemail(&mut self, account: Option<u8>) -> Result<(), Error> {
        self.command(Command::DialVoicemail {
            account: account.map(AccountId),
        })
        .map(|_| ())
    }
}

impl api::presence::Host for AppCtx {
    fn subscribe(&mut self, target: String) {
        if self.presence_subs.len() < 256 {
            self.presence_subs.insert(target);
        }
    }
    fn unsubscribe(&mut self, target: String) {
        self.presence_subs.remove(&target);
    }
    fn get(&mut self, target: String) -> Option<Presence> {
        let p = self.store.get::<dphone_state::Presence>();
        p.entries
            .iter()
            .find(|e| entry_matches(e, &target))
            .map(|e| presence_of(e, &target))
    }
    fn set_own(&mut self, status: String, text: String) -> Result<(), Error> {
        let status = PresenceStatus::from_token(&status);
        if status == PresenceStatus::Unknown {
            return Err(Error::Invalid("unknown presence status".into()));
        }
        self.command(Command::SetPresence { status, text })
            .map(|_| ())
    }
    fn get_own(&mut self) -> Presence {
        let p = self.store.get::<dphone_state::Presence>();
        Presence {
            target: String::new(),
            status: match p.own_status {
                PresenceStatus::Unknown => "unknown".into(),
                s => s.token().into(),
            },
            text: p.own_status_text.clone(),
            line_state: "unknown".into(),
        }
    }
}

impl api::events::Host for AppCtx {
    fn observe(&mut self, name: String) {
        if self.observed.len() < 64 {
            self.observed.insert(name);
        }
    }
    fn stop_observing(&mut self, name: String) {
        self.observed.remove(&name);
    }
    fn send(&mut self, app: String, name: String, payload: String) -> Result<(), String> {
        if crate::manifest::validate_name(&app).is_err() {
            return Err("bad app name".into());
        }
        self.actions.push(Action::Send { app, name, payload });
        Ok(())
    }
}

impl api::audio::Host for AppCtx {
    fn play_file(&mut self, loc: Location, name: String, start: u32) -> Result<(), Error> {
        let path = self.resolve(loc, &name, false)?;
        if !path.is_file() {
            return Err(Error::NotFound);
        }
        // The audio side reads and decodes the file before it answers.
        self.store
            .send(Command::PlayFile {
                path: path.to_string_lossy().into_owned(),
                start_secs: start,
                app: self.name.clone(),
            })
            .wait_timeout(Duration::from_secs(5))
            .map(|_| self.playing = true)
            .map_err(|e| Error::Failed(e.to_string()))
    }
    fn stop(&mut self) {
        let _ = self.store.post(Command::StopSound);
    }
}

impl api::l10n::Host for AppCtx {
    fn locale(&mut self) -> String {
        "en_US".into()
    }
    fn common(&mut self, key: String) -> String {
        match key.as_str() {
            "ok" => "OK",
            "cancel" => "Cancel",
            "back" => "Back",
            "exit" => "Exit",
            "select" => "Select",
            "yes" => "Yes",
            "no" => "No",
            "more" => "More",
            "delete" => "Delete",
            "save" => "Save",
            "loading" => "Loading...",
            "error" => "Error",
            _ => return key,
        }
        .to_string()
    }
    fn format_time(&mut self, unix: u64, style: u8) -> String {
        let c = &self.services.config;
        format_time(unix, c.utc_offset_secs(), c.clock_24h(), style)
    }
}

impl api::hotdesk::Host for AppCtx {
    fn login(&mut self, extension: String, pin: String) -> Result<(), Error> {
        if extension.is_empty() || extension.len() > 64 || pin.len() > 64 {
            return Err(Error::Invalid("bad extension or PIN".into()));
        }
        self.command(Command::SelectUser { id: extension, pin })
            .map(|_| ())
    }
    fn logout(&mut self) -> Result<(), Error> {
        // An empty user means "the phone's own, MAC-keyed configuration".
        self.command(Command::SelectUser {
            id: String::new(),
            pin: String::new(),
        })
        .map(|_| ())
    }
    fn current_user(&mut self) -> String {
        self.store
            .get::<dphone_state::Provisioning>()
            .assigned_user
            .clone()
    }
}

/// Adds the interfaces the manifest grants to a linker.  `runtime` is
/// always there; an app importing anything else it did not declare fails
/// to instantiate.
pub(crate) fn link(
    linker: &mut wasmtime::component::Linker<AppCtx>,
    manifest: &Manifest,
) -> wasmtime::Result<()> {
    use wasmtime::component::HasSelf;
    type H = HasSelf<AppCtx>;
    api::types::add_to_linker::<_, H>(linker, |c| c)?;
    api::runtime::add_to_linker::<_, H>(linker, |c| c)?;
    for cap in &manifest.capabilities {
        match cap {
            Capability::Ui => api::ui::add_to_linker::<_, H>(linker, |c| c)?,
            Capability::Timers => api::timers::add_to_linker::<_, H>(linker, |c| c)?,
            Capability::Storage => api::storage::add_to_linker::<_, H>(linker, |c| c)?,
            Capability::Config => api::config::add_to_linker::<_, H>(linker, |c| c)?,
            Capability::Pbx => api::pbx::add_to_linker::<_, H>(linker, |c| c)?,
            Capability::Http => api::http::add_to_linker::<_, H>(linker, |c| c)?,
            Capability::Calls => api::calls::add_to_linker::<_, H>(linker, |c| c)?,
            Capability::Presence => api::presence::add_to_linker::<_, H>(linker, |c| c)?,
            Capability::Events => api::events::add_to_linker::<_, H>(linker, |c| c)?,
            Capability::Audio => api::audio::add_to_linker::<_, H>(linker, |c| c)?,
            Capability::L10n => api::l10n::add_to_linker::<_, H>(linker, |c| c)?,
            Capability::Hotdesk => api::hotdesk::add_to_linker::<_, H>(linker, |c| c)?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_formats() {
        // 2026-11-13 22:05:00 UTC
        let t = 1_794_607_500;
        assert_eq!(format_time(t, 0, true, 0), "22:05");
        assert_eq!(format_time(t, 0, false, 0), "10:05 PM");
        assert_eq!(format_time(t, 0, true, 1), "Nov 13, 2026");
        assert_eq!(format_time(t, -6 * 3600, false, 2), "Nov 13, 2026 4:05 PM");
        assert_eq!(format_time(0, 0, false, 2), "Jan 1, 1970 12:00 AM");
    }

    #[test]
    fn pbx_reply_reduction() {
        assert!(matches!(
            pbx_outcome(Ok(br#"{"response":{"method":"m","result":{"a":1}}}"#.to_vec())),
            PbxOutcome::Ok(s) if s == r#"{"a":1}"#
        ));
        assert!(matches!(
            pbx_outcome(Ok(
                br#"{"response":{"errors":{"error":{"code":"1"}}}}"#.to_vec()
            )),
            PbxOutcome::Errors(_)
        ));
        assert!(matches!(
            pbx_outcome(Ok(b"<html>".to_vec())),
            PbxOutcome::Transport(_)
        ));
        assert!(matches!(
            pbx_outcome(Err("x".into())),
            PbxOutcome::Transport(_)
        ));
    }

    #[test]
    fn names_with_separators_are_refused() {
        for n in ["", ".", "..", "a/b", "a\\b", "/etc/passwd"] {
            assert!(bad_name(n), "{n}");
        }
        assert!(!bad_name("settings.json"));
    }
}
