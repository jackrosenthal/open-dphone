//! The command domains the main role itself answers: `Config`, `Network`,
//! `Update`, and `System`; and the hub that tells every subsystem when the
//! merged configuration changes.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dphone_config::Config;
use dphone_hal::Mtd;
use dphone_state::{Command, CommandError, Domain, Event, Reply, Store, Subsystem, UpdatePhase};
use dphone_update::{Bank, Updater, trial};
use log::{error, info, warn};

use super::SharedConfig;
use super::net::NetHandle;
use crate::mainrole::{Context, state_model};
use crate::sys::Shutdown;

type ConfigListener = Box<dyn Fn(&Arc<Config>) + Send + Sync>;

/// Listeners for configuration changes.  Subsystems register a closure when
/// they start; it runs on the runtime thread after every merge that changed
/// something, and must be quick.
#[derive(Clone, Default)]
pub struct ConfigHub {
    listeners: Arc<Mutex<Vec<ConfigListener>>>,
}

impl ConfigHub {
    pub fn on_change(&self, f: impl Fn(&Arc<Config>) + Send + Sync + 'static) {
        self.listeners.lock().unwrap().push(Box::new(f));
    }

    /// A merge finished: publish the generation and tell everyone.
    pub fn changed(&self, store: &Store, cfg: &Arc<Config>) {
        let generation = cfg.generation();
        store.update::<dphone_state::ConfigInfo, _>(|c| {
            c.generation = generation;
            c.last_merge_unix = dphone_state::clock::unix_secs();
            c.error.clear();
        });
        store.update::<dphone_state::System, _>(|s| s.timezone = cfg.time().time_zone);
        for l in self.listeners.lock().unwrap().iter() {
            l(cfg);
        }
        store.emit(Event::ConfigApplied { generation });
    }
}

/// The flash, shared between the update command handler and diagnostics.
pub type SharedMtd = Arc<Mutex<Box<dyn Mtd>>>;

pub struct Parts {
    pub config: Option<SharedConfig>,
    pub hub: ConfigHub,
    pub net: NetHandle,
    pub mtd: Option<SharedMtd>,
}

pub fn start(ctx: &Context, parts: Parts) {
    let Parts { config, hub, net, mtd } = parts;
    tokio::spawn(config_domain(ctx.clone(), config.clone(), hub));
    tokio::spawn(network_domain(ctx.store.clone(), net));
    tokio::spawn(system_domain(ctx.clone(), config));
    tokio::spawn(update_domain(ctx.clone(), mtd.clone()));
    tokio::spawn(confirm_trial_when_healthy(ctx.clone()));

    // Seed the update state: running bank and trial status.
    if let Some(mtd) = mtd {
        let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
        if let Ok(m) = mtd.lock() {
            trial::seed_state(&ctx.store, &ctx.paths.open_dir(), &cmdline, m.as_ref());
        }
    }
}

async fn config_domain(ctx: Context, config: Option<SharedConfig>, hub: ConfigHub) {
    let rx = ctx.store.handle(Domain::Config);
    let read_only = !super::stock_writes_allowed(&ctx);
    while let Some(envelope) = rx.next().await {
        let (cmd, responder) = envelope.split();
        let Some(config) = &config else {
            responder.reply(Err(CommandError::Failed("the configuration could not be loaded".into())));
            continue;
        };
        let result = match cmd {
            Command::ReloadConfig => config.lock().unwrap().reload().map_err(|e| CommandError::Failed(e.to_string())),
            Command::SetSettings { settings } => {
                if read_only && settings.iter().any(|(id, _)| !id.starts_with("open_")) {
                    Err(CommandError::Denied(
                        "this image leaves stock-owned nvdata files untouched; settings cannot be saved".into(),
                    ))
                } else {
                    let pairs: Vec<(&str, &str)> = settings.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
                    config.lock().unwrap().set_many(&pairs).map_err(|e| CommandError::Failed(e.to_string()))
                }
            }
            Command::ReloadContacts => {
                // Contact lists are read from disk by their users; a reload
                // of the merge picks up changed sources.
                config.lock().unwrap().reload().map_err(|e| CommandError::Failed(e.to_string()))
            }
            other => Err(CommandError::Invalid(format!("not a configuration command: {other:?}"))),
        };
        match result {
            Ok(cfg) => {
                hub.changed(&ctx.store, &cfg);
                responder.reply(Ok(Reply::Done));
            }
            Err(e) => {
                if let CommandError::Failed(text) = &e {
                    warn!("config: {text}");
                    ctx.store.update::<dphone_state::ConfigInfo, _>(|c| c.error = text.clone());
                }
                responder.reply(Err(e));
            }
        }
    }
}

async fn network_domain(store: Store, net: NetHandle) {
    let rx = store.handle(Domain::Network);
    while let Some(envelope) = rx.next().await {
        let (cmd, responder) = envelope.split();
        let result = match cmd {
            // A renewal is a fresh bring-up with the same settings.
            Command::RenewDhcp | Command::ReconfigureNetwork => {
                net.reconfigure();
                Ok(Reply::Done)
            }
            Command::SyncTime => {
                net.sync_time();
                Ok(Reply::Done)
            }
            other => Err(CommandError::Invalid(format!("not a network command: {other:?}"))),
        };
        responder.reply(result);
    }
}

async fn system_domain(ctx: Context, config: Option<SharedConfig>) {
    let rx = ctx.store.handle(Domain::System);
    while let Some(envelope) = rx.next().await {
        let (cmd, responder) = envelope.split();
        let result = match cmd {
            Command::Reboot { reason } => {
                let reason = if reason.is_empty() { "requested".to_string() } else { reason };
                shutdown_soon(&ctx, reason);
                Ok(Reply::Done)
            }
            Command::FactoryReset => {
                if !super::stock_writes_allowed(&ctx) {
                    Err(CommandError::Denied("this image leaves stock-owned nvdata files untouched".into()))
                } else {
                    // Stock's marker file: the reset runs at the next start,
                    // before anything reads the configuration.
                    let nv = match &config {
                        Some(c) => c.lock().unwrap().nvdata().clone(),
                        None => dphone_config::Nvdata::new(ctx.paths.nvdata.clone()),
                    };
                    match nv.request_factory_reset() {
                        Ok(()) => {
                            shutdown_soon(&ctx, "factory reset".to_string());
                            Ok(Reply::Done)
                        }
                        Err(e) => Err(CommandError::Failed(e.to_string())),
                    }
                }
            }
            Command::SetTime { unix_secs } => match crate::sys::set_clock(unix_secs as i64) {
                Ok(()) => {
                    ctx.store.emit(Event::TimeChanged);
                    Ok(Reply::Done)
                }
                Err(e) => Err(CommandError::Failed(e.to_string())),
            },
            other => Err(CommandError::Invalid(format!("not a system command: {other:?}"))),
        };
        responder.reply(result);
    }
}

/// Announce the shutdown, give listeners (UI, SIP unregister) a moment, then
/// go down through the init role.
fn shutdown_soon(ctx: &Context, reason: String) {
    ctx.store.update::<dphone_state::System, _>(|s| s.phase = dphone_state::Phase::ShuttingDown);
    ctx.store.emit(Event::ShuttingDown { reason: reason.clone() });
    let ctx = ctx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        ctx.request_shutdown(Shutdown::Reboot, &format!("dphone: {reason}"));
    });
}

fn cmdline() -> String {
    std::fs::read_to_string("/proc/cmdline").unwrap_or_default()
}

async fn update_domain(ctx: Context, mtd: Option<SharedMtd>) {
    let rx = ctx.store.handle(Domain::Update);
    let model = state_model(ctx.model);
    while let Some(envelope) = rx.next().await {
        let (cmd, responder) = envelope.split();
        let Some(mtd) = mtd.clone() else {
            responder.reply(Err(CommandError::Failed("flash is not available".into())));
            continue;
        };
        match cmd {
            Command::InstallFirmware { path, allow_stock } => {
                if matches!(
                    ctx.store.get::<dphone_state::Update>().phase,
                    UpdatePhase::Idle | UpdatePhase::Complete | UpdatePhase::Failed(_)
                ) {
                    // The write takes minutes: do it on its own thread and
                    // answer when it is done.  Progress is in the store.
                    let store = ctx.store.clone();
                    let open_dir = ctx.paths.open_dir();
                    let spawned = std::thread::Builder::new().name("update".into()).spawn(move || {
                        responder.reply(install(&store, &mtd, model, &path, allow_stock, &open_dir));
                    });
                    if let Err(e) = spawned {
                        error!("update: cannot start the update thread: {e}");
                    }
                } else {
                    responder.reply(Err(CommandError::Invalid("an update is already running".into())));
                }
            }
            Command::ConfirmTrial => {
                let r = trial::mark_healthy(&ctx.paths.open_dir(), Some(&ctx.store));
                responder.reply(r.map(|()| Reply::Done).map_err(|e| CommandError::Failed(e.to_string())));
            }
            Command::Rollback => {
                let result = (|| {
                    let mut m = mtd.lock().map_err(|_| "flash lock poisoned".to_string())?;
                    let parts = m.partitions().map_err(|e| e.to_string())?;
                    let running = dphone_update::bank::running_bank(&cmdline(), &parts)
                        .ok_or("not running from either bank; use 'dphone update switch-bank'")?;
                    let other = match running {
                        Bank::A => Bank::B,
                        Bank::B => Bank::A,
                    };
                    dphone_update::select_bank(m.as_mut(), model, other).map_err(|e| e.to_string())?;
                    let _ = trial::clear(&ctx.paths.open_dir());
                    Ok::<(), String>(())
                })();
                match result {
                    Ok(()) => {
                        shutdown_soon(&ctx, "switching to the other firmware bank".into());
                        responder.reply(Ok(Reply::Done));
                    }
                    Err(e) => responder.reply(Err(CommandError::Failed(e))),
                }
            }
            other => responder.reply(Err(CommandError::Invalid(format!("not an update command: {other:?}")))),
        }
    }
}

fn install(
    store: &Store,
    mtd: &SharedMtd,
    model: dphone_state::Model,
    path: &str,
    allow_stock: bool,
    open_dir: &std::path::Path,
) -> Result<Reply, CommandError> {
    let open = || std::fs::File::open(path).map(|f| std::io::BufReader::with_capacity(64 * 1024, f));
    // Look at the label first: a stock image is only written when asked for.
    let header = eff::EffReader::new(open().map_err(|e| CommandError::Failed(format!("{path}: {e}")))?)
        .map_err(|e| CommandError::Failed(format!("{path}: {e}")))?
        .header()
        .clone();
    if dphone_update::is_stock_version(&header.version) && !allow_stock {
        return Err(CommandError::Denied(format!(
            "{} is stock firmware; installing it replaces Open DPhone and must be asked for explicitly",
            header.version
        )));
    }
    let reader = open().map_err(|e| CommandError::Failed(format!("{path}: {e}")))?;
    let mut m = mtd.lock().map_err(|_| CommandError::Failed("flash lock poisoned".into()))?;
    let installed = Updater::new(m.as_mut(), model, &cmdline())
        .store(store.clone())
        .install_eff(reader)
        .map_err(|e| CommandError::Failed(e.to_string()))?;
    info!("update: installed {} into bank {:?}", installed.version, installed.bank);
    if let Err(e) = trial::begin(open_dir, &installed) {
        warn!("update: could not record the trial boot: {e}");
    }
    Ok(Reply::Text(installed.version))
}

/// The main role's half of trial confirmation (the init role does the same
/// on a healthy heartbeat; either may come first).
async fn confirm_trial_when_healthy(ctx: Context) {
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        if ctx.health.is_healthy() {
            if !ctx.sim && ctx.recovery.is_none() {
                let _ = trial::mark_healthy(&ctx.paths.open_dir(), Some(&ctx.store));
            }
            ctx.store.heartbeat(Subsystem::Core);
            return;
        }
    }
}
