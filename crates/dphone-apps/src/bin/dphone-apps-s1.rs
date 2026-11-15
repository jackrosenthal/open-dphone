//! Spike S1 measurement: loads the precompiled Status app under the real
//! manager with fake services and times instantiation, event handling,
//! and host calls.  Run it on the phone or under `qemu-arm`:
//!
//!     s1 <built-in-apps-root>

use dphone_apps::testutil::{FakeConfig, FakePbx, FakeUi, services};
use dphone_apps::{AppManager, ManagerConfig};
use dphone_state::{Model, Store};
use serde_json::json;
use std::time::Instant;

fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

/// Measure the app runtime (spike S1)
#[derive(clap::Parser)]
struct Args {
    /// The staged apps root (e.g., /usr/share/dphone/apps)
    apps_root: std::path::PathBuf,
}

fn main() {
    let root = <Args as clap::Parser>::parse().apps_root;
    let tmp = std::env::temp_dir().join(format!("s1-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = Store::new(Model::D40);
    let ui = FakeUi::new(Model::D40);
    let pbx = FakePbx::default();
    pbx.on("switchvox.users.presence.options.getList", |_| {
        Ok(json!({"presence_options": {"presence_option": [
            {"id": "1", "presence": "available", "sub_presence": "", "owner": "system", "active": "1"},
            {"id": "2", "presence": "away", "sub_presence": "Lunch", "owner": "system", "active": "0"},
            {"id": "3", "presence": "dnd", "sub_presence": "", "owner": "system", "active": "0"},
        ]}}))
    });
    pbx.on("switchvox.users.presence.getInfo", |_| {
        Ok(json!({"presence": {"id": "1", "presence": "available", "sub_presence": "", "message": "hi"}}))
    });

    let rss0 = rss_kib();
    let t = Instant::now();
    let mut cfg = ManagerConfig::rooted(
        std::path::Path::new(&root),
        &tmp.join("nvdata"),
        &tmp.join("tmp"),
    );
    // On a 64-bit workstation, `S1_TARGET=pulley64` runs the interpreter
    // instead of native code (apps staged with `--target pulley64`).
    if std::env::var("S1_TARGET").as_deref() == Ok("pulley64") {
        cfg.target = dphone_apps::engine::Target::Pulley64;
    }
    let mut mgr = AppManager::new(cfg, store, ui.shell(), services(&pbx, FakeConfig::dasp()))
        .expect("engine");
    let engine_ms = t.elapsed().as_secs_f64() * 1e3;
    let rss1 = rss_kib();

    mgr.scan();
    let t = Instant::now();
    mgr.launch("status").expect("launch");
    let launch_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    mgr.pump();
    mgr.pump();
    let first_screen_ms = t.elapsed().as_secs_f64() * 1e3;
    let rss2 = rss_kib();
    assert_eq!(ui.items("status").len(), 3, "status list shown");

    // Each round trip rebuilds two screens: about 24 host calls with
    // strings and lists crossing the boundary, in 2 guest calls.
    let rounds = 200;
    let t = Instant::now();
    for _ in 0..rounds {
        ui.press_softkey("status", "Message");
        mgr.pump();
        ui.press_softkey("status", "Back");
        mgr.pump();
    }
    let per_round_us = t.elapsed().as_secs_f64() * 1e6 / rounds as f64;

    println!(
        "engine creation:        {engine_ms:8.1} ms   (+{} KiB RSS)",
        rss1.saturating_sub(rss0)
    );
    println!("load+instantiate+start: {launch_ms:8.1} ms");
    println!("first screen (2 PBX):   {first_screen_ms:8.1} ms");
    println!(
        "instance RSS:           {:8} KiB",
        rss2.saturating_sub(rss1)
    );
    println!(
        "screen rebuild x2:      {per_round_us:8.0} us  (~{:.0} us per host call)",
        per_round_us / 24.0
    );
    let _ = std::fs::remove_dir_all(&tmp);
}
