//! The main role: everything except PID 1's duties, in one process on one
//! single-threaded async runtime.
//!
//! Start-up order is in [`run_async`]; the subsystems that other crates
//! provide are started from [`subsystems::start`].

pub mod health;
pub mod subsystems;

use std::net::SocketAddr;
use std::os::fd::FromRawFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use log::{error, info, warn};
use tokio::sync::watch;

use crate::diag::Registry;
use crate::init::{BEAT_ALIVE, BEAT_HEALTHY, HEARTBEAT_FD_ENV, MODEL_ENV, RECOVERY_ENV};
use crate::model::Model;
use crate::paths::Paths;
use crate::sys::{self, Shutdown};

pub use health::Health;

/// How often the main role beats to the init role.
pub const BEAT_INTERVAL: Duration = Duration::from_secs(2);

/// Command-line options of `dphone main`.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Host simulator: no hardware, no PID 1 above us.
    pub sim: bool,
    pub model: Option<Model>,
    pub nvdata: Option<PathBuf>,
    pub run_dir: Option<PathBuf>,
    /// Listen address for SSH; giving one turns SSH on.
    pub ssh_listen: Option<SocketAddr>,
    pub no_ssh: bool,
    pub verbose: bool,
}

pub const USAGE: &str = "\
usage: dphone main [options]
  --sim                 run without hardware (host simulator)
  --model <d40..d70>    model to run as (default: detected, or d60 with --sim)
  --nvdata <dir>        nvdata directory (default /nvdata)
  --run-dir <dir>       runtime directory (default /run/dphone)
  --ssh-port <port>     enable SSH on this port (all addresses)
  --ssh-listen <addr>   enable SSH on this address, e.g. 127.0.0.1:2222
  --no-ssh              do not start SSH
  -v, --verbose         debug logging
";

impl Options {
    pub fn parse(args: &[String]) -> Result<Options, String> {
        let mut o = Options::default();
        let mut it = args.iter();
        let value = |it: &mut std::slice::Iter<String>, name: &str| {
            it.next().cloned().ok_or_else(|| format!("{name} needs a value"))
        };
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--sim" => o.sim = true,
                "--model" => {
                    let v = value(&mut it, "--model")?;
                    o.model = Some(v.parse().map_err(|_| format!("unknown model '{v}'"))?);
                }
                "--nvdata" => o.nvdata = Some(PathBuf::from(value(&mut it, "--nvdata")?)),
                "--run-dir" => o.run_dir = Some(PathBuf::from(value(&mut it, "--run-dir")?)),
                "--ssh-port" => {
                    let v = value(&mut it, "--ssh-port")?;
                    let port: u16 = v.parse().map_err(|_| format!("bad port '{v}'"))?;
                    o.ssh_listen = Some(SocketAddr::from(([0, 0, 0, 0], port)));
                }
                "--ssh-listen" => {
                    let v = value(&mut it, "--ssh-listen")?;
                    o.ssh_listen = Some(v.parse().map_err(|_| format!("bad address '{v}'"))?);
                }
                "--no-ssh" => o.no_ssh = true,
                "-v" | "--verbose" => o.verbose = true,
                other => return Err(format!("unknown option '{other}'")),
            }
        }
        if o.sim && o.nvdata.is_none() {
            return Err("--sim needs --nvdata <dir>".to_string());
        }
        Ok(o)
    }
}

/// Why the main role is ending.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exit {
    Running,
    /// Leave with this status; the init role restarts us.
    Code(i32),
}

/// Shared handles every subsystem gets.
#[derive(Clone)]
pub struct Context {
    pub paths: Paths,
    pub model: Model,
    /// Host simulator: skip hardware.
    pub sim: bool,
    /// Set when the init role could not mount nvdata (value: the reason).
    /// nvdata is then a tmpfs; show the recovery screen and web page, and
    /// treat nothing as persistent.
    pub recovery: Option<String>,
    /// Development mode (flag file, kernel command line, or baked-in key).
    pub dev: bool,
    pub diag: Registry,
    pub health: Health,
    pub started: Instant,
    exit_tx: Arc<watch::Sender<Exit>>,
    supervised: bool,
}

impl Context {
    /// Ask the main role to exit; the init role restarts it.
    pub fn request_exit(&self, code: i32) {
        let _ = self.exit_tx.send(Exit::Code(code));
    }

    /// Reboot, halt, or power off the phone through the init role, recording
    /// `reason` in the stock `reboot_reason` file.  In the simulator, or when
    /// nothing supervises us, this only ends the main role.
    pub fn request_shutdown(&self, kind: Shutdown, reason: &str) {
        info!("{kind:?} requested: {reason}");
        if self.recovery.is_none() {
            let _ = crate::paths::write_atomic(&self.paths.reboot_reason(), format!("{reason}\n").as_bytes(), 0o644);
        }
        if self.sim || !self.supervised {
            self.request_exit(0);
            return;
        }
        let sig = match kind {
            Shutdown::Reboot => libc::SIGTERM,
            Shutdown::Halt => libc::SIGUSR1,
            Shutdown::PowerOff => libc::SIGUSR2,
        };
        if let Err(e) = sys::kill(1, sig) {
            error!("cannot signal the init role: {e}");
        }
    }
}

/// Entry point of `dphone main`.  Returns the process exit status.
pub fn run(args: &[String]) -> i32 {
    let opts = match Options::parse(args) {
        Ok(o) => o,
        Err(e) => {
            eprint!("dphone main: {e}\n{USAGE}");
            return 2;
        }
    };
    let level = if opts.verbose { log::LevelFilter::Debug } else { log::LevelFilter::Info };
    crate::logging::init_ring(level, true);

    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            error!("cannot start the async runtime: {e}");
            return 1;
        }
    };
    let code = runtime.block_on(run_async(opts));
    // Do not wait for lingering blocking tasks (e.g., a shell session's I/O).
    runtime.shutdown_timeout(Duration::from_millis(200));
    code
}

fn resolve_model(opts: &Options) -> Model {
    if let Some(m) = opts.model {
        return m;
    }
    if let Some(m) = std::env::var(MODEL_ENV).ok().and_then(|v| v.parse().ok()) {
        return m;
    }
    if !opts.sim {
        if let Some(d) = crate::model::detect() {
            return d.model;
        }
        warn!("model not detected; assuming D60");
    }
    Model::D60
}

async fn run_async(opts: Options) -> i32 {
    let paths = Paths::new(
        opts.nvdata.clone().unwrap_or_else(|| PathBuf::from(crate::paths::NVDATA)),
        opts.run_dir.clone().unwrap_or_else(|| match (&opts.nvdata, opts.sim) {
            // Simulator default: keep the runtime directory beside nvdata.
            (Some(nv), true) => nv.join(".run"),
            _ => PathBuf::from(crate::paths::RUN_DIR),
        }),
    );
    let model = resolve_model(&opts);
    let recovery = std::env::var(RECOVERY_ENV).ok().filter(|v| !v.is_empty());
    let beat_fd = std::env::var(HEARTBEAT_FD_ENV).ok().and_then(|v| v.parse::<i32>().ok());
    // The descriptor is ours alone; children must not inherit it.
    // SAFETY: fcntl on a descriptor number the init role gave us.
    if let Some(fd) = beat_fd {
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    unsafe { std::env::remove_var(HEARTBEAT_FD_ENV) };

    let (exit_tx, mut exit_rx) = watch::channel(Exit::Running);
    let ctx = Context {
        dev: crate::ssh::dev_key().is_some() || (!opts.sim && crate::init::dev_mode(&paths)),
        paths,
        model,
        sim: opts.sim,
        recovery,
        diag: Registry::new(),
        health: Health::new(),
        started: Instant::now(),
        exit_tx: Arc::new(exit_tx),
        supervised: beat_fd.is_some(),
    };
    info!(
        "{} {} main role starting: model {}{}{}",
        crate::PRODUCT,
        crate::VERSION,
        ctx.model,
        if ctx.sim { ", simulator" } else { "" },
        if ctx.recovery.is_some() { ", RECOVERY (nvdata not mounted)" } else { "" },
    );
    for dir in [ctx.paths.run.clone(), ctx.paths.open_dir()] {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            warn!("mkdir {}: {e}", dir.display());
        }
    }

    // 1. Diagnostics socket, so `dphone log` works from the first moment.
    crate::diag::builtin::register(&ctx);
    match crate::diag::server::bind(&ctx.paths.diag_socket()) {
        Ok(listener) => {
            tokio::spawn(crate::diag::server::serve(listener, ctx.diag.clone()));
        }
        Err(e) => error!("diag socket {}: {e}", ctx.paths.diag_socket().display()),
    }

    // 2. Heartbeat to the init role.
    ctx.health.require("core");
    if let Some(fd) = beat_fd {
        tokio::spawn(heartbeat(fd, ctx.health.clone()));
    }

    // 3. Signals.
    tokio::spawn(signals(ctx.clone()));

    // 4. Subsystems from the other crates, in dependency order.
    subsystems::start(&ctx).await;

    // 5. SSH.
    if let Some(listen) = crate::ssh::listen_address(&ctx, &opts) {
        match crate::ssh::start(&ctx, listen).await {
            Ok(addr) => info!("ssh listening on {addr}"),
            Err(e) => error!("ssh: {e}"),
        }
    } else {
        info!("ssh disabled");
    }

    // 6. Our own part is up.
    ctx.health.satisfy("core");

    // 7. Run until told to stop.
    let code = loop {
        if exit_rx.changed().await.is_err() {
            break 1;
        }
        if let Exit::Code(code) = exit_rx.borrow().clone() {
            break code;
        }
    };
    info!("main role exiting with status {code}");
    let _ = std::fs::remove_file(ctx.paths.diag_socket());
    code
}

/// Write beats to the init role.  This task runs on the same single-threaded
/// runtime as everything else, so a stalled event loop stops the beats.
async fn heartbeat(fd: i32, health: Health) {
    use std::io::Write;
    // SAFETY: the init role opened this descriptor for us; nothing else in
    // this process owns it.
    let mut pipe = unsafe { std::fs::File::from_raw_fd(fd) };
    let _ = sys::set_nonblocking(fd, true);
    let mut tick = tokio::time::interval(BEAT_INTERVAL);
    loop {
        tick.tick().await;
        let beat = if health.is_healthy() { BEAT_HEALTHY } else { BEAT_ALIVE };
        // A full pipe (init busy) or a closed one is not our problem to fix.
        let _ = pipe.write(&[beat]);
    }
}

async fn signals(ctx: Context) {
    use tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut term), Ok(mut int), Ok(mut hup)) =
        (signal(SignalKind::terminate()), signal(SignalKind::interrupt()), signal(SignalKind::hangup()))
    else {
        error!("cannot install signal handlers");
        return;
    };
    loop {
        tokio::select! {
            _ = term.recv() => { info!("SIGTERM"); ctx.request_exit(0); }
            _ = int.recv() => { info!("SIGINT"); ctx.request_exit(0); }
            _ = hup.recv() => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn options() {
        let o = Options::parse(&args(&["--sim", "--model", "d40", "--nvdata", "/tmp/nv", "--ssh-port", "2222"])).unwrap();
        assert!(o.sim);
        assert_eq!(o.model, Some(Model::D40));
        assert_eq!(o.ssh_listen.unwrap().port(), 2222);
        assert!(Options::parse(&args(&["--sim"])).is_err());
        assert!(Options::parse(&args(&["--model", "d99"])).is_err());
        assert!(Options::parse(&args(&["--bogus"])).is_err());
    }
}
