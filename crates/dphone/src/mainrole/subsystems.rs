//! Start-up of the subsystems that live in other crates.
//!
//! This is the integration point.  The order is the dependency order and
//! should be kept.  Every step is contained: a subsystem that fails to start
//! is logged, shown in `dphone health`/`dphone state`, and skipped; the SSH
//! server and the diag socket (started before this) stay up regardless.
//!
//! Every subsystem gets a clone of [`Context`] and from it:
//!
//! - `ctx.store`: the state store, event bus, and command bus;
//! - `ctx.diag.register(...)` to add `dphone <command>` diagnostics;
//! - `ctx.health.require(name)` / `satisfy(name)` for anything that must be
//!   up before a trial boot is confirmed (`net` today, `ui` to come);
//! - `ctx.request_shutdown(...)` for reboots, and `ctx.request_exit(...)`
//!   for a restart of the main role only;
//! - `ctx.recovery` to know nvdata is a stand-in tmpfs;
//! - `ctx.sim` to pick the simulator HAL backend;
//! - `ctx.safe`: safe mode, in which only the network (with defaults when the
//!   configuration cannot be read) is started here.
//!
//! All network I/O belongs on the current (single-threaded) runtime via
//! `tokio::spawn`.  The audio thread and the UI thread are plain OS threads
//! started by their subsystems.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dphone_config::{ConfigStore, Nvdata};
use dphone_state::{HealthStatus, Subsystem};
use log::{error, info, warn};

use super::{Context, Options};
use crate::diag::Output;

pub mod apps;
pub mod audio;
pub mod hal;
pub mod net;
pub mod remote;
pub mod system;
pub mod telephony;
pub mod ui;
#[cfg(feature = "web")]
pub mod web;

/// The configuration store, shared between subsystems.  Calls are short and
/// synchronous; nothing holds the lock across an await.
pub type SharedConfig = Arc<Mutex<ConfigStore>>;

pub async fn start(ctx: &Context, opts: &Options) {
    // 1. dphone-state.  The store is in the context from the start; here the
    //    log ring is bridged into it and the system facts are filled in.
    start_state(ctx);

    // 2. dphone-config: load and merge the nvdata XML layers.
    let config = start_config(ctx);
    let hub = system::ConfigHub::default();

    // 3. dphone-net: `NetManager` on this runtime.  Before the hardware, so
    //    that nothing there can delay the way in.
    let net = net::start(ctx, config.clone(), &hub);
    {
        let net = net.clone();
        let mut last = config.as_ref().map(|c| c.lock().unwrap().config().network());
        let last_cell = std::sync::Mutex::new(last.take());
        hub.on_change(move |cfg| {
            // Only a change in the network settings restarts bring-up.
            let now = cfg.network();
            let mut last = last_cell.lock().unwrap();
            if last.as_ref() != Some(&now) {
                *last = Some(now);
                net.config_changed();
            }
        });
    }

    if ctx.safe {
        warn!("safe mode: hardware, audio, telephony, UI, apps, and web are not started");
        ctx.diag.register("hal", "poke hardware (not available in safe mode)", |_a, out: Output| async move {
            out.err("dphone hal: not started (safe mode)\n").await;
            crate::diag::EXIT_UNAVAILABLE
        });
        system::start(ctx, system::Parts { config, hub, net, mtd: None });
        finish(ctx);
        return;
    }

    // The SSH switch: opens or closes the listener, now and on every later
    // configuration change.  Not consulted in safe mode or when the
    // configuration cannot be read: SSH stays on.
    if let Some(cfg) = config.as_ref().map(|c| c.lock().unwrap().config()) {
        crate::ssh::set_enabled(ssh_enabled(&cfg));
    }
    hub.on_change(|cfg| crate::ssh::set_enabled(ssh_enabled(cfg)));

    // 4. dphone-hal: `phone` backend, or `sim` when `ctx.sim`.
    let hal = hal::start(ctx).await;

    // The flash goes to the update handlers.
    let mtd: Option<system::SharedMtd> = hal.as_ref().and_then(|h| {
        let mut h = h.lock().ok()?;
        if h.missing.iter().any(|(name, _)| *name == "mtd") {
            return None;
        }
        Some(Arc::new(Mutex::new(std::mem::replace(&mut h.mtd, Box::new(dphone_hal::Null("mtd"))))))
    });

    // 5. dphone-vpm / dphone-media: the DSP, then the audio thread.
    let audio = audio::start(ctx, hal.clone()).await;
    if let Some(cfg) = config.as_ref().map(|c| c.lock().unwrap().config()) {
        audio.apply_config(&cfg);
    }
    {
        let audio = audio.clone();
        hub.on_change(move |cfg| audio.apply_config(cfg));
    }

    // 6. The main role's own command domains: Config, Network, Update, System.
    let firmware_mtd = mtd.clone();
    system::start(ctx, system::Parts { config: config.clone(), hub: hub.clone(), net: net.clone(), mtd });

    // 7. dphone-sip and dphone-dpma: accounts, provisioning.
    let telephony = telephony::start(ctx, config.clone(), &hub, net.clone(), audio.clone());
    // Provisioned firmware: downloaded over HTTP(S) and written by the updater.
    if let (Some(dpma), Some(mtd)) = (telephony.dpma.lock().unwrap().clone(), firmware_mtd) {
        let (runtime, net) = (tokio::runtime::Handle::current(), net.clone());
        let source = dphone_dpma::fetch::HttpSource::with_lookup(Box::new(move |host, port| {
            let resolver = net.resolver().ok_or_else(|| std::io::Error::other("the network is not up"))?;
            let addrs = runtime.block_on(resolver.lookup_ip(host, false, false)).map_err(|e| std::io::Error::other(format!("{e:?}")))?;
            addrs.first().map(|ip| std::net::SocketAddr::new(*ip, port)).ok_or_else(|| std::io::Error::other("no address"))
        }));
        let installer = dphone_dpma::install::MtdInstaller {
            mtd,
            model: super::state_model(ctx.model),
            cmdline: std::fs::read_to_string("/proc/cmdline").unwrap_or_default(),
            open_dir: ctx.paths.open_dir(),
            store: ctx.store.clone(),
        };
        dpma.set_firmware_updater(Arc::new(source), Arc::new(installer));
    }

    // 8. dphone-ui: the UI thread.  (It takes the display, keys, and lamps
    //    out of the hardware bundle; `dphone hal` then only reports status.)
    let ui = ui::start(ctx, hal.clone(), config.clone(), &hub).await;

    remote::register(ctx, ui.as_ref().map(|u| u.tap.clone()));

    // dphone-apps: the WASM runtime, on its own thread.
    apps::start(ctx, ui.as_ref().map(|u| u.handle.clone()), config.clone(), telephony.clone());

    // 9. dphone-web: HTTP(S) and Connect RPC.
    #[cfg(feature = "web")]
    web::start(ctx, opts, config.clone(), ui.as_ref().map(|u| u.tap.clone()));
    #[cfg(not(feature = "web"))]
    let _ = opts;

    let _ = (audio, hal, telephony, ui);
    finish(ctx);
}

/// Whether the SSH server should listen.  This is the one place that reads
/// the switch from the configuration; the SSH module only takes the answer
/// (`crate::ssh::set_enabled`).  SSH is on by default; the switch is
/// `ssh_enabled` in `/nvdata/dphone_settings.txtpb`.
fn ssh_enabled(config: &dphone_config::Config) -> bool {
    config.ssh_enabled().value
}

fn finish(ctx: &Context) {
    ctx.store.update::<dphone_state::System, _>(|s| {
        s.phase = if ctx.recovery.is_some() { dphone_state::Phase::Recovery } else { dphone_state::Phase::Running };
    });
}

fn start_state(ctx: &Context) {
    let store = ctx.store.clone();
    crate::logging::set_sink(Box::new(move |level, target, message| {
        store.log_line(level.into(), target, message.to_string());
    }));
    let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
    let boot_rev = cmdline
        .split_ascii_whitespace()
        .find_map(|w| w.strip_prefix("digium_boot_rev="))
        .and_then(|v| v.parse().ok());
    let mem_total_kib = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("MemTotal:"))
                .and_then(|v| v.split_ascii_whitespace().next().and_then(|n| n.parse().ok()))
        })
        .unwrap_or(0);
    let last_reason = std::fs::read_to_string(ctx.paths.reboot_reason()).unwrap_or_default();
    ctx.store.update::<dphone_state::System, _>(|s| {
        s.simulated = ctx.sim;
        s.boot_rev = boot_rev;
        s.mem_total_kib = mem_total_kib;
        s.last_reboot_reason = last_reason.trim().to_string();
    });
    ctx.store.set_health(Subsystem::Core, HealthStatus::Ok);

    let store = ctx.store.clone();
    ctx.diag.register("state", "dump the state store: state [TOPIC-SUBSTRING]", move |args: Vec<String>, out: Output| {
        let store = store.clone();
        async move {
            let text = format!("{:#?}\n", store.snapshot());
            match args.first() {
                None => out.out(&text).await,
                Some(filter) => out.out(&filter_section(&text, filter)).await,
            }
            0
        }
    });
}

/// From a `{:#?}` dump of the whole state, keep the top-level fields whose
/// name contains `filter` (case-insensitive).
fn filter_section(text: &str, filter: &str) -> String {
    let filter = filter.to_ascii_lowercase();
    let mut out = String::new();
    let mut keep = false;
    for line in text.lines() {
        // Top-level fields are indented by exactly four spaces.
        if line.starts_with("    ") && !line.starts_with("     ") {
            let name = line.trim().split(':').next().unwrap_or("");
            if line.trim().contains(':') {
                keep = name.to_ascii_lowercase().contains(&filter);
            }
        }
        if keep {
            out.push_str(line);
            out.push('\n');
        }
    }
    if out.is_empty() {
        out = format!("no state field matches '{filter}'\n");
    }
    out
}

fn config_model(ctx: &Context) -> dphone_config::Model {
    ctx.model.name().parse().unwrap_or(dphone_config::Model::D60)
}

/// True when this image may write files that stock also owns.
pub fn stock_writes_allowed(ctx: &Context) -> bool {
    ctx.sim || crate::paths::stock_writes_allowed()
}

fn start_config(ctx: &Context) -> Option<SharedConfig> {
    ctx.store.set_health(Subsystem::Config, HealthStatus::Starting);
    let nv = Nvdata::new(ctx.paths.nvdata.clone());
    let read_only = !stock_writes_allowed(ctx);
    let model = config_model(ctx);
    let tmp = ctx.paths.run.clone();
    // The merge is plain computation over files; run it where a panic in it
    // cannot take the main role down.
    let opened = std::thread::Builder::new()
        .name("config-open".into())
        .spawn(move || if read_only { ConfigStore::open_read_only(nv, model, tmp) } else { ConfigStore::open(nv, model, tmp) })
        .map_err(|e| e.to_string())
        .and_then(|h| h.join().map_err(|_| "panic while merging the configuration".to_string()))
        .and_then(|r| r.map_err(|e| e.to_string()));
    let store = match opened {
        Ok(s) => s,
        Err(e) => {
            error!("config: {e}; continuing with built-in defaults for the network");
            ctx.store.set_health(Subsystem::Config, HealthStatus::Failed(e.clone()));
            ctx.store.update::<dphone_state::ConfigInfo, _>(|c| c.error = e);
            ctx.diag.register("config", "configuration (failed to load)", |_a, out: Output| async move {
                out.err("dphone config: the configuration could not be loaded; see 'dphone log'\n").await;
                crate::diag::EXIT_UNAVAILABLE
            });
            return None;
        }
    };
    for w in store.warnings() {
        warn!("config: layer skipped: {w}");
    }
    let cfg = store.config();
    info!(
        "config: generation {}, {} settings{}",
        cfg.generation(),
        cfg.settings().count(),
        if read_only { " (stock files left untouched)" } else { "" }
    );
    ctx.store.update::<dphone_state::ConfigInfo, _>(|c| {
        c.generation = cfg.generation();
        c.last_merge_unix = dphone_state::clock::unix_secs();
        c.error.clear();
    });
    ctx.store.update::<dphone_state::System, _>(|s| s.timezone = cfg.time().time_zone);
    ctx.store.set_health(Subsystem::Config, HealthStatus::Ok);

    let shared: SharedConfig = Arc::new(Mutex::new(store));
    let handle = shared.clone();
    let state = ctx.store.clone();
    ctx.diag.register(
        "config",
        "config get ID | config list [SUBSTRING] | config set ID VALUE | config reload",
        move |args: Vec<String>, out: Output| {
            let handle = handle.clone();
            let state = state.clone();
            async move {
                let (code, text, err) = config_command(&handle, &state, read_only, &args);
                if !text.is_empty() {
                    out.out(&text).await;
                }
                if !err.is_empty() {
                    out.err(&err).await;
                }
                code
            }
        },
    );
    Some(shared)
}

fn config_command(
    handle: &SharedConfig,
    state: &dphone_state::Store,
    read_only: bool,
    args: &[String],
) -> (i32, String, String) {
    let Ok(mut store) = handle.lock() else {
        return (1, String::new(), "dphone config: store poisoned\n".into());
    };
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["get", id] => match store.get(id) {
            Some(v) => (0, format!("{v}\n"), String::new()),
            None => (1, String::new(), format!("dphone config: no setting '{id}'\n")),
        },
        ["list"] | ["list", _] => {
            let filter = args.get(1).copied().unwrap_or("");
            let cfg = store.config();
            let mut text = String::new();
            for (id, value) in cfg.settings().filter(|(id, _)| id.contains(filter)) {
                // Never print secrets to a terminal by default.
                let shown = if id.contains("password") || id.contains("secret") { "********" } else { value };
                text.push_str(&format!("{id}={shown}\n"));
            }
            (0, text, String::new())
        }
        ["set", id, value] => {
            if read_only && !id.starts_with("open_") {
                return (
                    1,
                    String::new(),
                    format!(
                        "dphone config: this image leaves stock-owned nvdata files untouched ({}); only open_* settings can be set\n",
                        crate::paths::NO_STOCK_WRITES_FLAG
                    ),
                );
            }
            match store.set(id, value) {
                Ok(cfg) => {
                    state.update::<dphone_state::ConfigInfo, _>(|c| {
                        c.generation = cfg.generation();
                        c.last_merge_unix = dphone_state::clock::unix_secs();
                    });
                    (0, String::new(), String::new())
                }
                Err(e) => (1, String::new(), format!("dphone config: {e}\n")),
            }
        }
        ["reload"] => match store.reload() {
            Ok(cfg) => (0, format!("generation {}\n", cfg.generation()), String::new()),
            Err(e) => (1, String::new(), format!("dphone config: {e}\n")),
        },
        _ => (
            2,
            String::new(),
            "usage: dphone config get ID | list [SUBSTRING] | set ID VALUE | reload\n".into(),
        ),
    }
}

/// Run `f` on its own thread and wait for it without blocking the runtime,
/// so that neither a panic nor a slow device open stalls the main role.
pub async fn contained<T: Send + 'static>(
    name: &str,
    limit: Duration,
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            let _ = tx.send(f());
        })
        .map_err(|e| format!("cannot start thread {name}: {e}"))?;
    match tokio::time::timeout(limit, rx).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(_)) => Err(format!("{name} panicked")),
        Err(_) => Err(format!("{name} did not finish within {} s", limit.as_secs())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_filter() {
        let text = "State {\n    system: System {\n        model: D60,\n    },\n    network: Network {\n        phase: Up,\n    },\n}\n";
        let got = filter_section(text, "net");
        assert!(got.contains("phase: Up"));
        assert!(!got.contains("model: D60"));
        assert!(filter_section(text, "zzz").contains("no state field"));
    }
}
