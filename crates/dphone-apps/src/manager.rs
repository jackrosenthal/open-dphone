//! The app manager: discovers apps, runs them, feeds them events, and
//! publishes [`dphone_state::Apps`].
//!
//! It is single-threaded: build it, then either call [`AppManager::run`]
//! on a thread of its own or call [`AppManager::pump`] from an existing
//! loop whenever the waker passed to [`AppManager::set_waker`] fires or
//! the returned delay passes.

use crate::engine::{AppEngine, EPOCH_TICK, EngineError, Target};
use crate::host::bindings::App;
use crate::host::bindings::dphone::app::types as wit;
use crate::host::{self, Action, AppCtx, Dirs, IDLE_BIT, Inbound, Inbox, Timer};
use crate::install::{InstallError, Installer};
use crate::manifest::{AppType, Capability, Manifest};
use crate::package::Package;
use crate::services::Services;
use crate::view::{Shell, WindowEvent, WindowKind};
use dphone_state::{
    AppInfo, Apps, Call, Command, CommandError, CommandReceiver, Domain, Event, Reply, Store,
    Subscription, Subsystem, Topic,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Waker};
use std::time::Duration;
use wasmtime::component::{Component, Linker};

#[derive(Clone, Debug)]
pub struct ManagerConfig {
    /// Built-in apps, read-only: `<root>/<name>/{app.json,<name>.cwasm}`.
    pub builtin_root: PathBuf,
    /// Installed apps.
    pub installed_root: PathBuf,
    /// Stock JavaScript apps; only looked at to log that they are skipped.
    pub js_root: PathBuf,
    /// Persistent app data (`nv`, `nvshared`).
    pub nv_root: PathBuf,
    /// Temporary app data (`tmp`, `tmpshared`).
    pub tmp_root: PathBuf,
    /// Shared read-only data (`data`).
    pub data_dir: PathBuf,
    /// Default linear-memory cap per app.
    pub memory_cap: usize,
    /// The most a manifest may ask for.
    pub memory_hard_cap: usize,
    /// Longest one event handler may run before the app is stopped.
    pub call_budget: Duration,
    /// Longest `start` may run.
    pub start_budget: Duration,
    pub target: Target,
}

impl ManagerConfig {
    /// Paths on the phone.
    pub fn phone() -> ManagerConfig {
        ManagerConfig {
            builtin_root: "/usr/share/dphone/apps".into(),
            installed_root: "/nvdata/wasmapps".into(),
            js_root: "/nvdata/jsapps".into(),
            nv_root: "/nvdata/appdata".into(),
            tmp_root: "/tmp/appdata".into(),
            data_dir: "/usr/share/dphone/data".into(),
            memory_cap: 4 << 20,
            memory_hard_cap: 16 << 20,
            call_budget: Duration::from_secs(2),
            start_budget: Duration::from_secs(5),
            target: Target::Host,
        }
    }

    /// The same layout under another root (simulator, tests): `nvdata` is
    /// the nvdata directory, `builtin_root` the built apps.
    pub fn rooted(builtin_root: &Path, nvdata: &Path, tmp: &Path) -> ManagerConfig {
        ManagerConfig {
            builtin_root: builtin_root.into(),
            installed_root: nvdata.join("wasmapps"),
            js_root: nvdata.join("jsapps"),
            nv_root: nvdata.join("appdata"),
            tmp_root: tmp.join("appdata"),
            data_dir: builtin_root.join("../data"),
            ..ManagerConfig::phone()
        }
    }
}

struct Known {
    dir: PathBuf,
    manifest: Manifest,
    builtin: bool,
    code: Option<Component>,
}

struct Running {
    store: wasmtime::Store<AppCtx>,
    app: App,
}

/// File holding code this phone compiled for an installed app.
const CODE_CACHE: &str = ".code.cwasm";

pub struct AppManager {
    cfg: ManagerConfig,
    engine: AppEngine,
    store: Store,
    shell: Box<dyn Shell>,
    services: Services,
    inbox: Arc<Inbox>,
    installer: Installer,
    known: BTreeMap<String, Known>,
    running: HashMap<String, Running>,
    foreground: Option<String>,
    idle_shown: HashSet<String>,
    commands: CommandReceiver,
    sub: Subscription,
    generation: u64,
    last_calls: Vec<Call>,
    last_mwi: Arc<dyn_mwi::Snapshot>,
    last_presence: Arc<dphone_state::Presence>,
    stop: Arc<AtomicBool>,
    external_waker: Option<Arc<dyn Fn() + Send + Sync>>,
}

mod dyn_mwi {
    pub type Snapshot = dphone_state::Mwi;
}

fn key_name(k: dphone_state::Key) -> String {
    k.name()
}

impl AppManager {
    pub fn new(
        cfg: ManagerConfig,
        store: Store,
        shell: Box<dyn Shell>,
        services: Services,
    ) -> Result<AppManager, EngineError> {
        let engine = AppEngine::new(cfg.target)?;
        let installer = Installer {
            builtin_root: cfg.builtin_root.clone(),
            installed_root: cfg.installed_root.clone(),
            nv_root: cfg.nv_root.clone(),
            tmp_root: cfg.tmp_root.clone(),
        };
        installer.recover();
        let commands = store.handle(Domain::Apps);
        let sub = store.subscribe_events([Topic::Calls, Topic::Mwi, Topic::Presence]);
        Ok(AppManager {
            last_calls: store.get::<dphone_state::Calls>().calls.clone(),
            last_mwi: store.get::<dphone_state::Mwi>(),
            last_presence: store.get::<dphone_state::Presence>(),
            cfg,
            engine,
            store,
            shell,
            services,
            inbox: Arc::new(Inbox::default()),
            installer,
            known: BTreeMap::new(),
            running: HashMap::new(),
            foreground: None,
            idle_shown: HashSet::new(),
            commands,
            sub,
            generation: 0,
            stop: Arc::new(AtomicBool::new(false)),
            external_waker: None,
        })
    }

    /// Set to make [`AppManager::run`] return.
    pub fn stop_flag(&self) -> Arc<AtomicBool> {
        self.stop.clone()
    }

    /// A function that wakes [`AppManager::run`] (or, with
    /// [`AppManager::set_waker`], the caller's loop) from any thread.
    pub fn waker(&self) -> Arc<dyn Fn() + Send + Sync> {
        let n = self.inbox.notifier.clone();
        let ext = self.external_waker.clone();
        Arc::new(move || {
            n.notify();
            if let Some(f) = &ext {
                f();
            }
        })
    }

    /// For callers that drive [`AppManager::pump`] from their own loop:
    /// `f` is called from any thread when there is work.
    pub fn set_waker(&mut self, f: Arc<dyn Fn() + Send + Sync>) {
        self.external_waker = Some(f);
    }

    /// Discovers built-in and installed apps and publishes the list.
    pub fn scan(&mut self) {
        let mut known = BTreeMap::new();
        for (dir, manifest) in Installer::scan_root(&self.cfg.builtin_root) {
            let code = self.known.remove(&manifest.name).and_then(|k| k.code);
            known.insert(
                manifest.name.clone(),
                Known {
                    dir,
                    manifest,
                    builtin: true,
                    code,
                },
            );
        }
        for (dir, manifest) in Installer::scan_root(&self.cfg.installed_root) {
            if known.contains_key(&manifest.name) {
                log::warn!(
                    "installed app {:?} shadows a built-in; ignored",
                    manifest.name
                );
                continue;
            }
            known.insert(
                manifest.name.clone(),
                Known {
                    dir,
                    manifest,
                    builtin: false,
                    code: None,
                },
            );
        }
        // Stock JavaScript apps cannot run; say so once per scan.
        if let Ok(rd) = std::fs::read_dir(&self.cfg.js_root) {
            for e in rd.flatten() {
                if e.path().join("app.json").is_file() {
                    log::info!(
                        "skipping JavaScript app {:?}: it cannot run on this firmware",
                        e.file_name()
                    );
                }
            }
        }
        self.known = known;
        self.publish();
    }

    /// Starts the apps that run without being opened: background apps and
    /// apps with an idle-screen window.
    pub fn autostart(&mut self) {
        let names: Vec<String> = self
            .known
            .values()
            .filter(|k| k.manifest.kind == AppType::Background || k.manifest.has_idle)
            .filter(|k| !k.builtin || self.services.config.app_enabled(&k.manifest.name))
            .map(|k| k.manifest.name.clone())
            .collect();
        for n in names {
            if let Err(e) = self.start(&n) {
                log::warn!("app {n}: could not start: {e}");
            }
        }
        self.publish();
    }

    fn publish(&mut self) {
        let installed = self
            .known
            .values()
            .filter(|k| !k.builtin || self.services.config.app_enabled(&k.manifest.name))
            .map(|k| AppInfo {
                name: k.manifest.name.clone(),
                id: k.manifest.id.clone(),
                display_name: k.manifest.display_name.clone(),
                description: k.manifest.description.clone(),
                kind: k.manifest.kind.name().to_string(),
                has_idle: k.manifest.has_idle,
                hidden: k.manifest.hidden,
                builtin: k.builtin,
                icon: k.manifest.icon.clone(),
                running: self.running.contains_key(&k.manifest.name),
            })
            .collect();
        let apps = Apps {
            installed,
            foreground: self.foreground.clone(),
            idle_app: self.idle_shown.iter().min().cloned(),
        };
        if *self.store.get::<Apps>() != apps {
            self.store.set(apps);
        }
    }

    pub fn is_running(&self, name: &str) -> bool {
        self.running.contains_key(name)
    }

    pub fn foreground(&self) -> Option<&str> {
        self.foreground.as_deref()
    }

    pub fn names(&self) -> Vec<String> {
        self.known.keys().cloned().collect()
    }

    fn load_code(&mut self, name: &str) -> Result<Component, String> {
        let known = self.known.get(name).ok_or("no such app")?;
        if let Some(c) = &known.code {
            return Ok(c.clone());
        }
        let stem = known
            .manifest
            .wasm
            .strip_suffix(".wasm")
            .unwrap_or(&known.manifest.wasm);
        let cached = if known.builtin {
            known.dir.join(format!("{stem}.cwasm"))
        } else {
            known.dir.join(CODE_CACHE)
        };
        let mut component = None;
        if cached.is_file() {
            // SAFETY: built-in code comes from the firmware image, and the
            // cache of an installed app was written by this manager (a
            // package cannot supply a dot file or a `.cwasm`).
            match unsafe { self.engine.load_precompiled(&cached) } {
                Ok(c) => component = Some(c),
                Err(e) if AppEngine::can_compile() => {
                    log::debug!("app {name}: stale compiled code ({e}); compiling again")
                }
                Err(e) => return Err(format!("compiled code does not load: {e}")),
            }
        }
        let component = match component {
            Some(c) => c,
            None => {
                let wasm = std::fs::read(known.dir.join(&known.manifest.wasm))
                    .map_err(|e| format!("reading {}: {e}", known.manifest.wasm))?;
                if !known.builtin
                    && let Ok(bytes) = self.engine.precompile(&wasm)
                {
                    let _ = std::fs::write(&cached, bytes);
                }
                self.engine.load_wasm(&wasm).map_err(|e| e.to_string())?
            }
        };
        if let Some(k) = self.known.get_mut(name) {
            k.code = Some(component.clone());
        }
        Ok(component)
    }

    fn start(&mut self, name: &str) -> Result<(), String> {
        if self.running.contains_key(name) {
            return Ok(());
        }
        let component = self.load_code(name)?;
        let known = self.known.get(name).ok_or("no such app")?;
        let manifest = known.manifest.clone();
        let dirs = Dirs {
            package: known.dir.clone(),
            nv_root: self.cfg.nv_root.clone(),
            tmp_root: self.cfg.tmp_root.clone(),
            data: self.cfg.data_dir.clone(),
        };
        let mut linker = Linker::<AppCtx>::new(self.engine.wasmtime());
        host::link(&mut linker, &manifest).map_err(|e| format!("{e:#}"))?;

        let wake = self.waker();
        let window = self.shell.open(name, WindowKind::App, wake.clone());
        let idle = manifest
            .has_idle
            .then(|| self.shell.open(name, WindowKind::Idle, wake.clone()));
        let cap = match manifest.memory_kib as usize * 1024 {
            0 => self.cfg.memory_cap,
            want => want.min(self.cfg.memory_hard_cap),
        };
        self.generation += 1;
        let ctx = AppCtx {
            name: name.to_string(),
            generation: self.generation,
            dirs,
            window,
            idle,
            idle_size: self.shell.idle_size(),
            store: self.store.clone(),
            services: self.services.clone(),
            inbox: self.inbox.clone(),
            actions: Vec::new(),
            timers: Vec::new(),
            next_timer: 0,
            next_request: 0,
            pending: HashSet::new(),
            observed: HashSet::new(),
            observe_calls: false,
            presence_subs: HashSet::new(),
            exit_after_background: true,
            foreground: false,
            limits: wasmtime::StoreLimitsBuilder::new()
                .memory_size(cap)
                .memories(1)
                .tables(4)
                .table_elements(50_000)
                .instances(8)
                .build(),
            manifest,
        };
        let mut store = wasmtime::Store::new(self.engine.wasmtime(), ctx);
        store.limiter(|c| &mut c.limits);
        store.epoch_deadline_trap();
        store.set_epoch_deadline(ticks(self.cfg.start_budget));
        let started = App::instantiate(&mut store, &component, &linker).and_then(|app| {
            app.dphone_app_guest().call_start(&mut store)?;
            Ok(app)
        });
        match started {
            Ok(app) => {
                self.running
                    .insert(name.to_string(), Running { store, app });
                log::info!("app {name}: started");
                self.after_call(name);
                Ok(())
            }
            Err(e) => {
                let has_idle = store.data().idle.is_some();
                drop(store);
                self.shell.close(name, WindowKind::App);
                if has_idle {
                    self.shell.close(name, WindowKind::Idle);
                }
                Err(format!("{e:#}"))
            }
        }
    }

    /// Opens an app on screen, starting it if needed.
    pub fn launch(&mut self, name: &str) -> Result<(), String> {
        let known = self
            .known
            .get(name)
            .ok_or_else(|| format!("no app named {name:?}"))?;
        if known.builtin && !self.services.config.app_enabled(name) {
            return Err(format!("the app {name:?} is disabled"));
        }
        self.start(name)?;
        self.shell.show(name);
        self.publish();
        Ok(())
    }

    /// Stops an app: delivers `exit`, drops the instance, closes windows.
    pub fn close(&mut self, name: &str) {
        if self.running.contains_key(name) {
            self.call(name, &wit::Event::Exit);
        }
        self.kill(name);
        self.publish();
    }

    fn kill(&mut self, name: &str) {
        if let Some(r) = self.running.remove(name) {
            let has_idle = r.store.data().idle.is_some();
            drop(r);
            self.shell.hide(name);
            self.shell.close(name, WindowKind::App);
            if has_idle {
                self.shell.close(name, WindowKind::Idle);
            }
            if self.foreground.as_deref() == Some(name) {
                self.foreground = None;
            }
            self.idle_shown.remove(name);
            log::info!("app {name}: stopped");
        }
    }

    /// Calls the app's event handler.  A trap (including running over the
    /// time budget or the memory cap) stops the app.
    fn call(&mut self, name: &str, event: &wit::Event) -> bool {
        let budget = ticks(self.cfg.call_budget);
        let Some(r) = self.running.get_mut(name) else {
            return false;
        };
        r.store.set_epoch_deadline(budget);
        match r.app.dphone_app_guest().call_handle(&mut r.store, event) {
            Ok(()) => true,
            Err(e) => {
                log::error!("app {name}: stopped after a fault: {e:#}");
                self.kill(name);
                self.publish();
                false
            }
        }
    }

    /// Delivers an event and applies what the app asked for meanwhile.
    fn deliver(&mut self, name: &str, event: wit::Event) {
        if self.call(name, &event) {
            self.after_call(name);
        }
    }

    fn after_call(&mut self, name: &str) {
        let actions = match self.running.get_mut(name) {
            Some(r) => std::mem::take(&mut r.store.data_mut().actions),
            None => return,
        };
        for a in actions {
            match a {
                Action::Background => self.shell.hide(name),
                Action::Exit => {
                    self.close(name);
                    return;
                }
                Action::Restart => {
                    let was_fg = self.foreground.as_deref() == Some(name);
                    self.close(name);
                    let r = if was_fg {
                        self.launch(name)
                    } else {
                        self.start(name)
                    };
                    if let Err(e) = r {
                        log::warn!("app {name}: restart failed: {e}");
                    }
                    return;
                }
                Action::ShowApp(other) => {
                    if let Err(e) = self.launch(&other) {
                        log::warn!("app {name}: cannot show {other}: {e}");
                    }
                }
                Action::Send {
                    app,
                    name: ev,
                    payload,
                } => {
                    if self.running.contains_key(&app) {
                        self.deliver(&app, wit::Event::Message((ev, payload)));
                    }
                }
            }
        }
    }

    /// Installs a package file.  Returns the app's name.
    pub fn install(
        &mut self,
        path: &Path,
        required_name: Option<&str>,
    ) -> Result<String, InstallError> {
        let package = Package::open(path)?;
        let name = package.manifest.name.clone();
        // Compile first: it proves the component is valid and yields the
        // code the phone will run.
        let code = self
            .engine
            .precompile(package.wasm())
            .map_err(|e| InstallError::Unloadable(e.to_string()))?;
        self.installer
            .check(&package, required_name, code.len() as u64)?;
        // The component may import only what the manifest declares.
        let component = self
            .engine
            .load_wasm(package.wasm())
            .map_err(|e| InstallError::Unloadable(e.to_string()))?;
        let mut linker = Linker::<AppCtx>::new(self.engine.wasmtime());
        host::link(&mut linker, &package.manifest)
            .and_then(|_| linker.instantiate_pre(&component).map(|_| ()))
            .map_err(|e| InstallError::Unloadable(format!("{e:#}")))?;
        let was_running = self.running.contains_key(&name);
        self.close(&name);
        self.installer
            .commit(&package, &[(CODE_CACHE.to_string(), code)])?;
        self.scan();
        self.store.emit(Event::App {
            app: name.clone(),
            name: "app_install".into(),
            payload: String::new(),
        });
        if was_running {
            let _ = self.start(&name);
            self.publish();
        }
        log::info!("app {name}: installed");
        Ok(name)
    }

    /// Removes an installed app and its data.
    pub fn remove(&mut self, name: &str) -> Result<(), InstallError> {
        if self.known.get(name).is_some_and(|k| k.builtin) {
            return Err(InstallError::ShadowsBuiltin(name.to_string()));
        }
        self.close(name);
        self.installer.remove(name)?;
        self.scan();
        Ok(())
    }

    fn handle_command(&mut self, command: Command) -> Result<Reply, CommandError> {
        match command {
            Command::LaunchApp { name } => self
                .launch(&name)
                .map(|_| Reply::Done)
                .map_err(CommandError::Failed),
            Command::CloseApp { name } => {
                self.close(&name);
                Ok(Reply::Done)
            }
            Command::InstallApp { path } => self
                .install(Path::new(&path), None)
                .map(Reply::Text)
                .map_err(|e| CommandError::Failed(format!("{e} (status {})", e.stock_status()))),
            Command::RemoveApp { name } => self
                .remove(&name)
                .map(|_| Reply::Done)
                .map_err(|e| CommandError::Failed(e.to_string())),
            other => Err(CommandError::Invalid(format!(
                "not an app command: {other:?}"
            ))),
        }
    }

    fn running_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.running.keys().cloned().collect();
        v.sort();
        v
    }

    fn apps_where(&self, f: impl Fn(&AppCtx) -> bool) -> Vec<String> {
        let mut v: Vec<String> = self
            .running
            .iter()
            .filter(|(_, r)| f(r.store.data()))
            .map(|(n, _)| n.clone())
            .collect();
        v.sort();
        v
    }

    fn on_state(&mut self, wake: dphone_state::Wake) {
        if wake.changed(Topic::Calls) {
            let calls = self.store.get::<dphone_state::Calls>().calls.clone();
            let changed: Vec<wit::CallInfo> = calls
                .iter()
                .filter(|c| self.last_calls.iter().find(|o| o.id == c.id) != Some(c))
                .map(host::call_info)
                .collect();
            self.last_calls = calls;
            if !changed.is_empty() {
                for name in self.apps_where(|c| c.observe_calls && c.has(Capability::Calls)) {
                    for info in &changed {
                        self.deliver(&name, wit::Event::Call(info.clone()));
                    }
                }
            }
        }
        if wake.changed(Topic::Mwi) {
            let mwi = self.store.get::<dphone_state::Mwi>();
            let changed: Vec<wit::Mwi> = mwi
                .accounts
                .iter()
                .filter(|m| !self.last_mwi.accounts.contains(m))
                .map(|m| wit::Mwi {
                    account: m.account.0,
                    new_messages: m.new,
                    old_messages: m.old,
                })
                .collect();
            self.last_mwi = mwi;
            for name in self.apps_where(|c| c.has(Capability::Calls)) {
                for m in &changed {
                    self.deliver(&name, wit::Event::Mwi(*m));
                }
            }
        }
        if wake.changed(Topic::Presence) {
            let now = self.store.get::<dphone_state::Presence>();
            let changed: Vec<dphone_state::BlfEntry> = now
                .entries
                .iter()
                .filter(|e| self.last_presence.get(&e.id) != Some(e))
                .cloned()
                .collect();
            self.last_presence = now;
            for name in self.apps_where(|c| !c.presence_subs.is_empty()) {
                let subs: Vec<String> = self.running[&name]
                    .store
                    .data()
                    .presence_subs
                    .iter()
                    .cloned()
                    .collect();
                for e in &changed {
                    for t in subs.iter().filter(|t| host::entry_matches(e, t)) {
                        self.deliver(&name, wit::Event::Presence(host::presence_of(e, t)));
                    }
                }
            }
        }
        for ev in wake.events {
            match ev {
                Event::Push { name: push, body } => self.on_push(&push, &body),
                Event::App {
                    app,
                    name: ev,
                    payload,
                } => {
                    if ev != "reset_idle_timer"
                        && ev != "app_install"
                        && self.running.contains_key(&app)
                    {
                        self.deliver(&app, wit::Event::Message((ev, payload)));
                    }
                }
                Event::CallEnded { call } => {
                    let info = host::call_info(&call);
                    for name in self.apps_where(|c| c.observe_calls && c.has(Capability::Calls)) {
                        self.deliver(&name, wit::Event::Call(info.clone()));
                    }
                }
                Event::ConfigApplied { .. } => {
                    self.publish();
                    for name in self.apps_where(|c| c.has(Capability::Config)) {
                        self.deliver(&name, wit::Event::NewConfig);
                    }
                }
                Event::ShuttingDown { .. } => {
                    for name in self.running_names() {
                        self.close(&name);
                    }
                }
                _ => {}
            }
        }
    }

    /// A server push.  `body` is the event data; when it is a JSON object
    /// with `type`, `data`, `method`, or `call` members (the SIP layer's
    /// envelope for pushes that arrive on an INVITE or INFO), those fill
    /// the matching fields.
    fn on_push(&mut self, push: &str, body: &str) {
        if push == "digium.phone.server_change" {
            for name in self.apps_where(|c| c.has(Capability::Events)) {
                self.deliver(&name, wit::Event::ServerChange);
            }
            return;
        }
        let targets = self.apps_where(|c| c.observed.contains(push));
        if targets.is_empty() {
            return;
        }
        let mut record = wit::Push {
            name: push.to_string(),
            kind: String::new(),
            data: body.to_string(),
            method: "MESSAGE".into(),
            call: None,
        };
        if let Ok(serde_json::Value::Object(m)) = serde_json::from_str::<serde_json::Value>(body)
            && (m.contains_key("method") || m.contains_key("call"))
        {
            let text = |k: &str| match m.get(k) {
                Some(serde_json::Value::String(s)) => s.clone(),
                Some(serde_json::Value::Null) | None => String::new(),
                Some(v) => v.to_string(),
            };
            record.kind = text("type");
            record.data = text("data");
            if let Some(method) = m.get("method").and_then(|v| v.as_str()) {
                record.method = method.to_string();
            }
            record.call = m.get("call").and_then(|v| v.as_u64()).map(|c| c as u32);
        }
        // An INVITE push names its call; if the SIP layer did not, take the
        // newest alerting call.
        if record.call.is_none() && push.starts_with("digium.incomingCall.") {
            record.method = "INVITE".into();
            record.call = self
                .store
                .get::<dphone_state::Calls>()
                .calls
                .iter()
                .filter(|c| c.state == dphone_state::CallState::Ringing)
                .max_by_key(|c| c.started_mono_ms)
                .map(|c| c.id.0);
        }
        for name in targets {
            self.deliver(&name, wit::Event::Push(record.clone()));
        }
    }

    fn on_window_event(&mut self, name: &str, idle: bool, ev: WindowEvent) {
        let bit = if idle { IDLE_BIT } else { 0 };
        let event = match ev {
            WindowEvent::Foreground if idle => {
                self.idle_shown.insert(name.to_string());
                self.publish();
                wit::Event::IdleScreenShow
            }
            WindowEvent::Background if idle => {
                self.idle_shown.remove(name);
                self.publish();
                wit::Event::IdleScreenHide
            }
            WindowEvent::Foreground => {
                self.foreground = Some(name.to_string());
                if let Some(r) = self.running.get_mut(name) {
                    r.store.data_mut().foreground = true;
                }
                self.publish();
                wit::Event::Foreground
            }
            WindowEvent::Background => {
                if self.foreground.as_deref() == Some(name) {
                    self.foreground = None;
                }
                let exit = match self.running.get_mut(name) {
                    Some(r) => {
                        let c = r.store.data_mut();
                        c.foreground = false;
                        c.exit_after_background
                            && c.manifest.kind == AppType::Foreground
                            && !c.manifest.has_idle
                    }
                    None => false,
                };
                self.deliver(name, wit::Event::Background);
                if exit && self.foreground.as_deref() != Some(name) {
                    self.close(name);
                }
                self.publish();
                return;
            }
            WindowEvent::Key(k) => wit::Event::Key(key_name(k)),
            WindowEvent::Softkey(id) => wit::Event::Softkey(id),
            WindowEvent::Highlighted { widget, row } => {
                wit::Event::Widget((widget | bit, wit::WidgetEvent::Highlighted(row)))
            }
            WindowEvent::Activated { widget, row } => {
                wit::Event::Widget((widget | bit, wit::WidgetEvent::Activated(row)))
            }
            WindowEvent::Changed { widget, value } => {
                wit::Event::Widget((widget | bit, wit::WidgetEvent::Changed(value)))
            }
            WindowEvent::Submitted { widget, value } => {
                wit::Event::Widget((widget | bit, wit::WidgetEvent::Submitted(value)))
            }
            WindowEvent::CloseRequested => {
                self.shell.hide(name);
                return;
            }
        };
        self.deliver(name, event);
    }

    /// Does all pending work.  Returns how long until a timer is due, if
    /// any timer is set.
    pub fn pump(&mut self) -> Option<Duration> {
        let waker = Waker::from(self.inbox.notifier.clone());
        let mut cx = Context::from_waker(&waker);

        while let Poll::Ready(Some(envelope)) = self.commands.poll_next(&mut cx) {
            let (command, responder) = envelope.split();
            let result = self.handle_command(command);
            responder.reply(result);
        }

        loop {
            let item = self.inbox.queue.lock().unwrap().pop_front();
            let Some(item) = item else { break };
            let (app, generation, id, event) = match item {
                Inbound::Pbx {
                    app,
                    generation,
                    id,
                    outcome,
                } => (app, generation, id, wit::Event::Pbx((id, outcome))),
                Inbound::Http {
                    app,
                    generation,
                    id,
                    result,
                } => (app, generation, id, wit::Event::Http((id, result))),
            };
            let live = self.running.get_mut(&app).is_some_and(|r| {
                let c = r.store.data_mut();
                c.generation == generation && c.pending.remove(&id)
            });
            if live {
                self.deliver(&app, event);
            }
        }

        while let Poll::Ready(wake) = self.sub.poll_changed(&mut cx) {
            self.on_state(wake);
        }

        // Window events; bounded so a chatty window cannot starve the rest.
        for _ in 0..64 {
            let mut any = false;
            for name in self.running_names() {
                let (ev, idle) = match self.running.get_mut(&name) {
                    Some(r) => {
                        let c = r.store.data_mut();
                        match c.window.poll_event() {
                            Some(e) => (Some(e), false),
                            None => (c.idle.as_mut().and_then(|w| w.poll_event()), true),
                        }
                    }
                    None => (None, false),
                };
                if let Some(ev) = ev {
                    any = true;
                    self.on_window_event(&name, idle, ev);
                }
            }
            if !any {
                break;
            }
        }

        let now = dphone_state::clock::mono_ms();
        for name in self.running_names() {
            let due: Vec<u32> = match self.running.get_mut(&name) {
                Some(r) => {
                    let c = r.store.data_mut();
                    let due: Vec<u32> = c
                        .timers
                        .iter()
                        .filter(|t| t.deadline_ms <= now)
                        .map(|t| t.id)
                        .collect();
                    c.timers.retain_mut(|t: &mut Timer| {
                        if t.deadline_ms > now {
                            return true;
                        }
                        match t.repeat_ms {
                            Some(p) => {
                                t.deadline_ms = now + p as u64;
                                true
                            }
                            None => false,
                        }
                    });
                    due
                }
                None => Vec::new(),
            };
            for id in due {
                self.deliver(&name, wit::Event::Timer(id));
            }
        }

        self.store.heartbeat(Subsystem::Apps);
        let now = dphone_state::clock::mono_ms();
        self.running
            .values()
            .flat_map(|r| r.store.data().timers.iter().map(|t| t.deadline_ms))
            .min()
            .map(|d| Duration::from_millis(d.saturating_sub(now)))
    }

    /// Runs until the stop flag is set.
    pub fn run(mut self) {
        while !self.stop.load(Ordering::Relaxed) {
            let next = self.pump().unwrap_or(Duration::from_secs(1));
            self.inbox.notifier.wait(next.min(Duration::from_secs(1)));
        }
        for name in self.running_names() {
            self.close(&name);
        }
    }
}

fn ticks(d: Duration) -> u64 {
    (d.as_millis() as u64 / EPOCH_TICK.as_millis() as u64).max(1)
}
