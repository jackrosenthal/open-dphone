//! Shared fixture: the built-in apps and the test guest, built to
//! WebAssembly and compiled for the host interpreter once per test run.
//!
//! With the feature `test-inputs/required` (the Nix check, whose sandbox
//! cannot build them), they are `test-inputs/apps-staged` instead, staged in
//! an earlier step (see the `test-inputs` crate).

#![allow(dead_code)]

use opendeskphone_apps::engine::Target;
use opendeskphone_apps::testutil::{FakeConfig, FakePbx, FakeStore, FakeUi, services_with_store};
use opendeskphone_apps::{AppManager, ManagerConfig};
use opendeskphone_state::{Command, CommandResult, Domain, Model, Reply, Store};
use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::sync::{Arc, Mutex, OnceLock};

fn run(cmd: &mut Process) {
    let out = cmd.output().expect("spawn");
    assert!(
        out.status.success(),
        "{:?} failed:\n{}\n{}",
        cmd,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The staged apps root.
pub fn staged() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        if test_inputs::REQUIRED {
            return test_inputs::input("apps-staged").unwrap();
        }
        let tool = env!("CARGO_BIN_EXE_opendeskphone-apps-build");
        let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let work = Path::new(env!("CARGO_TARGET_TMPDIR")).join("apps-fixture");
        let out = work.join("staged");
        let _ = std::fs::remove_dir_all(&out);
        for src in [crate_dir.join("../../apps"), crate_dir.join("tests/guests")] {
            run(Process::new(tool)
                .env("CARGO_TARGET_DIR", work.join("guest-target"))
                // Do not let the outer cargo's settings leak into the wasm build.
                .env_remove("CARGO_ENCODED_RUSTFLAGS")
                .env_remove("RUSTFLAGS")
                .args(["build", "--target", "pulley64", "--apps"])
                .arg(&src)
                .arg("--out")
                .arg(&out));
        }
        out
    })
}

/// The phone's record of a staged app.
#[allow(dead_code)]
pub fn record(app: &str) -> opendeskphone_apps::Manifest {
    let bytes = std::fs::read(staged().join(app).join(opendeskphone_apps::manifest::RECORD)).unwrap();
    opendeskphone_apps::Manifest::from_record(&bytes).unwrap()
}

/// The interfaces of the phone a staged app uses: its component's imports.
#[allow(dead_code)]
pub fn capabilities(app: &str) -> Vec<&'static str> {
    record(app).capabilities.iter().map(|c| c.name()).collect()
}

pub struct Rig {
    pub mgr: AppManager,
    pub ui: FakeUi,
    pub pbx: FakePbx,
    /// The PBX's store of app data, as this phone reaches it.
    pub app_data: FakeStore,
    pub store: Store,
    pub commands: Arc<Mutex<Vec<Command>>>,
    pub nvdata: PathBuf,
    _dir: TempDir,
}

pub struct TempDir(pub PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn temp_dir(tag: &str) -> TempDir {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    TempDir(d)
}

/// Answers commands of a domain on a thread, recording them.
pub fn answer_commands(
    store: &Store,
    domain: Domain,
    log: Arc<Mutex<Vec<Command>>>,
    reply: impl Fn(&Command) -> CommandResult + Send + 'static,
) {
    let rx = store.handle(domain);
    std::thread::spawn(move || {
        while let Some(env) = rx.recv() {
            let (cmd, responder) = env.split();
            let r = reply(&cmd);
            log.lock().unwrap().push(cmd);
            responder.reply(r);
        }
    });
}

impl Rig {
    pub fn new(config: FakeConfig) -> Rig {
        Rig::with_root(staged(), config, Model::D60)
    }

    pub fn with_root(root: &Path, config: FakeConfig, model: Model) -> Rig {
        Rig::with_budget(root, config, model, std::time::Duration::from_millis(300))
    }

    /// A rig whose apps get `budget` for each event (the phone gives
    /// them 2 seconds).
    pub fn with_budget(
        root: &Path,
        config: FakeConfig,
        model: Model,
        budget: std::time::Duration,
    ) -> Rig {
        Rig::with_app_data(root, config, model, budget, FakeStore::default())
    }

    /// A phone whose PBX store of app data is `app_data`: with
    /// `FakeStore::phone` of another rig's, a second phone of the same
    /// PBX.
    pub fn on_pbx(app_data: FakeStore) -> Rig {
        let budget = std::time::Duration::from_millis(300);
        Rig::with_app_data(staged(), FakeConfig::default(), Model::D60, budget, app_data)
    }

    /// A rig with other services than the fakes: `change` gets the
    /// rig's, and returns what the apps are to have.
    pub fn with_services(
        change: impl FnOnce(opendeskphone_apps::Services) -> opendeskphone_apps::Services,
    ) -> Rig {
        let budget = std::time::Duration::from_millis(300);
        let config = FakeConfig::default();
        Rig::build(staged(), config, Model::D60, budget, FakeStore::default(), change, |_| {})
    }

    /// A rig whose manager has other settings than a test's usual ones:
    /// `edit` changes them.
    pub fn with_manager(edit: impl FnOnce(&mut ManagerConfig)) -> Rig {
        let budget = std::time::Duration::from_millis(300);
        let config = FakeConfig::default();
        Rig::build(staged(), config, Model::D60, budget, FakeStore::default(), |s| s, edit)
    }

    pub fn with_app_data(
        root: &Path,
        config: FakeConfig,
        model: Model,
        budget: std::time::Duration,
        app_data: FakeStore,
    ) -> Rig {
        Rig::build(root, config, model, budget, app_data, |services| services, |_| {})
    }

    fn build(
        root: &Path,
        config: FakeConfig,
        model: Model,
        budget: std::time::Duration,
        app_data: FakeStore,
        change: impl FnOnce(opendeskphone_apps::Services) -> opendeskphone_apps::Services,
        edit: impl FnOnce(&mut ManagerConfig),
    ) -> Rig {
        let dir = temp_dir("rig");
        let store = Store::new(model);
        let ui = FakeUi::new(model);
        let pbx = FakePbx::default();
        let commands = Arc::new(Mutex::new(Vec::new()));
        answer_commands(&store, Domain::Calls, commands.clone(), |c| match c {
            Command::Dial { .. } => Ok(Reply::Call(opendeskphone_state::CallId(7))),
            Command::DialWith { .. } => Ok(Reply::Call(opendeskphone_state::CallId(8))),
            _ => Ok(Reply::Done),
        });
        answer_commands(&store, Domain::Provisioning, commands.clone(), |_| {
            Ok(Reply::Done)
        });
        let nvdata = dir.0.join("nvdata");
        let mut cfg = ManagerConfig::rooted(root, &nvdata);
        cfg.target = Target::Pulley64;
        cfg.call_budget = budget;
        edit(&mut cfg);
        let services = change(services_with_store(&pbx, config, &app_data));
        let mut mgr = AppManager::new(cfg, store.clone(), ui.shell(), services).expect("engine");
        mgr.scan();
        Rig {
            mgr,
            ui,
            pbx,
            app_data,
            store,
            commands,
            nvdata,
            _dir: dir,
        }
    }

    /// Runs the manager until nothing more happens.
    pub fn settle(&mut self) {
        for _ in 0..8 {
            self.mgr.pump();
        }
        // A compilation on the worker thread (an install, or an installed
        // app's code compiled again) is waited for.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while self.mgr.compiling() {
            assert!(std::time::Instant::now() < deadline, "the compiler thread did not finish");
            std::thread::sleep(std::time::Duration::from_millis(5));
            self.mgr.pump();
        }
        for _ in 0..8 {
            self.mgr.pump();
        }
    }

    pub fn launch(&mut self, app: &str) {
        self.mgr
            .launch(app)
            .unwrap_or_else(|e| panic!("launch {app}: {e}"));
        self.settle();
    }

    pub fn softkey(&mut self, app: &str, label: &str) {
        self.ui.press_softkey(app, label);
        self.settle();
    }

    pub fn key(&mut self, app: &str, key: opendeskphone_state::Key) {
        self.ui.press(app, key);
        self.settle();
    }

    pub fn commands(&self) -> Vec<Command> {
        self.commands.lock().unwrap().clone()
    }
}
