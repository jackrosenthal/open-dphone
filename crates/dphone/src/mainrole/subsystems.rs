//! Start-up of the subsystems that live in other crates.
//!
//! This is the integration point.  Each step below is a placeholder until
//! its crate is wired in; the order is the dependency order and should be
//! kept.  Every subsystem gets a clone of [`Context`] and from it:
//!
//! - `ctx.diag.register(...)` to add `dphone <command>` diagnostics (the
//!   placeholders `state`, `config`, `hal`, and `net` are replaced by
//!   registering the same name);
//! - `ctx.health.require(name)` / `satisfy(name)` for anything that must be
//!   up before a trial boot is confirmed (at least `net` and `ui`);
//! - `ctx.request_shutdown(...)` for reboots, and `ctx.request_exit(...)`
//!   for a restart of the main role only;
//! - `ctx.recovery` to know nvdata is a stand-in tmpfs;
//! - `ctx.sim` to pick the simulator HAL backend.
//!
//! All network I/O belongs on the current (single-threaded) runtime via
//! `tokio::spawn`.  The audio thread and the UI thread are plain OS threads
//! started by their subsystems.

use super::Context;

pub async fn start(ctx: &Context) {
    let _ = ctx;

    // 1. dphone-state: the typed store, event bus, command bus, and log
    //    ring.  Then replace the `state` and `log` diag commands, and bridge
    //    `crate::logging` into the store's ring.

    // 2. dphone-config: load and merge the nvdata XML layers (skip the stock
    //    files when `ctx.recovery` is set).  Replace the `config` diag command.

    // 3. dphone-hal: `phone` backend, or `sim` when `ctx.sim`.  Replace the
    //    `hal` diag command.  Claim the UART for EHS by creating
    //    `ctx.paths.console_claimed()` when configuration says so.

    // 4. dphone-net: `NetManager` on this runtime: MAC from enet.cfg, links,
    //    switch, VLAN, DHCP, DNS, SNTP, LLDP-MED, mDNS.  Register the `net`
    //    diag command.  `ctx.health.require("net")`, satisfied on first lease.

    // 5. dphone-vpm / dphone-media: start the DSP remoteproc, then the audio
    //    thread (SCHED_FIFO).

    // 6. dphone-sip and dphone-dpma: accounts, provisioning.

    // 7. dphone-ui and dphone-apps: UI thread, WASM runtime.
    //    `ctx.health.require("ui")`, satisfied on first frame.

    // 8. dphone-web: HTTP(S) and Connect RPC.

    // 9. dphone-update: firmware service.  Its rollback function also goes
    //    into `init::Hooks` in `cli::run` (the init role calls it, not us).
}
