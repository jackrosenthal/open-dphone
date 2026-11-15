//! Shared fixture: the built-in apps and the test guest, built to
//! WebAssembly and compiled for the host interpreter once per test run.
//!
//! Set `DPHONE_APPS_STAGED` to a directory staged with
//! `dphone-apps-build stage --target pulley64` to skip the build (for
//! sandboxed builds that prepare it in an earlier step).

#![allow(dead_code)]

use dphone_apps::engine::Target;
use dphone_apps::testutil::{FakeConfig, FakePbx, FakeUi, services};
use dphone_apps::{AppManager, ManagerConfig};
use dphone_state::{Command, CommandResult, Domain, Model, Reply, Store};
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
        if let Ok(d) = std::env::var("DPHONE_APPS_STAGED") {
            return PathBuf::from(d);
        }
        let tool = env!("CARGO_BIN_EXE_dphone-apps-build");
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

pub struct Rig {
    pub mgr: AppManager,
    pub ui: FakeUi,
    pub pbx: FakePbx,
    pub store: Store,
    pub commands: Arc<Mutex<Vec<Command>>>,
    pub nvdata: PathBuf,
    pub tmp: PathBuf,
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
        let dir = temp_dir("rig");
        let store = Store::new(model);
        let ui = FakeUi::new(model);
        let pbx = FakePbx::default();
        let commands = Arc::new(Mutex::new(Vec::new()));
        answer_commands(&store, Domain::Calls, commands.clone(), |c| match c {
            Command::Dial { .. } => Ok(Reply::Call(dphone_state::CallId(7))),
            Command::DialWith { .. } => Ok(Reply::Call(dphone_state::CallId(8))),
            _ => Ok(Reply::Done),
        });
        answer_commands(&store, Domain::Provisioning, commands.clone(), |_| {
            Ok(Reply::Done)
        });
        let (nvdata, tmp) = (dir.0.join("nvdata"), dir.0.join("tmp"));
        let mut cfg = ManagerConfig::rooted(root, &nvdata, &tmp);
        cfg.target = Target::Pulley64;
        cfg.call_budget = std::time::Duration::from_millis(300);
        let mut mgr = AppManager::new(cfg, store.clone(), ui.shell(), services(&pbx, config))
            .expect("engine");
        mgr.scan();
        Rig {
            mgr,
            ui,
            pbx,
            store,
            commands,
            nvdata,
            tmp,
            _dir: dir,
        }
    }

    /// Runs the manager until nothing more happens.
    pub fn settle(&mut self) {
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

    pub fn key(&mut self, app: &str, key: dphone_state::Key) {
        self.ui.press(app, key);
        self.settle();
    }

    pub fn commands(&self) -> Vec<Command> {
        self.commands.lock().unwrap().clone()
    }
}
