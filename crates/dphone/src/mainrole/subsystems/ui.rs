//! dphone-ui: the phone's screen, keys, and lamps, on their own thread.
//!
//! The display the UI draws on is wrapped in a tap that keeps a copy of
//! what was last flushed, so the web UI's screen mirror works on the phone
//! as well as in the simulator.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dphone_config::Config;
use dphone_hal::{Display, Frame, FrameInfo, Rect, Screen};
use dphone_state::{HealthStatus, Subsystem};
use dphone_ui::{Ui, UiConfig, UiHandle};
use log::{error, info};

use super::SharedConfig;
use super::hal::SharedHal;
use super::system::ConfigHub;
use crate::mainrole::Context;

/// The last flushed picture of each screen.
#[derive(Default)]
pub struct ScreenTap {
    generation: AtomicU64,
    frames: Mutex<Vec<(Screen, Frame)>>,
}

impl ScreenTap {
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    pub fn capture(&self, screen: Screen) -> Option<Frame> {
        self.frames.lock().ok()?.iter().find(|(s, _)| *s == screen).map(|(_, f)| f.clone())
    }
}

struct TapDisplay {
    inner: Box<dyn Display>,
    tap: Arc<ScreenTap>,
}

impl Display for TapDisplay {
    fn screens(&self) -> Vec<Screen> {
        self.inner.screens()
    }
    fn info(&self, screen: Screen) -> Option<FrameInfo> {
        self.inner.info(screen)
    }
    fn frame(&mut self, screen: Screen) -> dphone_hal::Result<&mut [u8]> {
        self.inner.frame(screen)
    }
    fn flush(&mut self, screen: Screen, damage: &[Rect]) -> dphone_hal::Result<()> {
        let result = self.inner.flush(screen, damage);
        if let (Some(info), Ok(pixels)) = (self.inner.info(screen), self.inner.frame(screen))
            && let Ok(mut frames) = self.tap.frames.lock()
        {
            match frames.iter_mut().find(|(s, _)| *s == screen) {
                Some((_, f)) if f.pixels.len() == pixels.len() => f.pixels.copy_from_slice(pixels),
                Some((_, f)) => *f = Frame { info, pixels: pixels.to_vec() },
                None => frames.push((screen, Frame { info, pixels: pixels.to_vec() })),
            }
            self.tap.generation.fetch_add(1, Ordering::Relaxed);
        }
        result
    }
    fn capture(&self, screen: Screen) -> dphone_hal::Result<Frame> {
        self.inner.capture(screen)
    }
    fn set_power(&mut self, screen: Screen, on: bool) -> dphone_hal::Result<()> {
        self.inner.set_power(screen, on)
    }
}

fn ui_config(ctx: &Context, cfg: Option<&Config>) -> UiConfig {
    let mut ringtones: Vec<String> = dphone_media::ring::BuiltinRing::ALL.iter().map(|r| r.name().to_string()).collect();
    // The user's own ringtones in `/nvdata/ringtones`, after the built-in
    // ones (a file named like a built-in ring is not offered twice).
    for name in super::ringtones::names(&ctx.paths.nvdata) {
        if !ringtones.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
            ringtones.push(name);
        }
    }
    let mut out = UiConfig { ringtones, ..UiConfig::default() };
    out.ringtone = out.ringtones.first().cloned().unwrap_or_default();
    let Some(cfg) = cfg else { return out };
    out.time_24h = cfg.setting("time_short_format").is_some_and(|f| f.contains('H') || f.contains('k'));
    out.utc_offset_secs = crate::tz::utc_offset(&cfg.time().time_zone, crate::sys::now_secs());
    if let Some(v) = cfg.setting_num::<u8>("brightness") {
        out.brightness = v.min(10);
    }
    out.dim_backlight = cfg.setting_bool("dim_backlight", out.dim_backlight);
    if let Some(v) = cfg.setting_num::<u8>("backlight_dim_level") {
        out.backlight_dim_level = v.min(10);
    }
    // Stock: 10 s when unset or over 600; 0 turns idle apps off.
    if let Some(v) = cfg.setting_num::<u32>("idle_cycle_seconds") {
        out.idle_cycle_secs = if v > 600 { 10 } else { v };
    }
    if let Some(v) = cfg.setting_num::<u32>("backlight_timeout") {
        out.backlight_timeout_secs = v;
    }
    if let Some(v) = cfg.setting_num::<u8>("contrast") {
        out.contrast = v;
    }
    let active = cfg.ringtones().active;
    if let Some(name) = out.ringtones.iter().find(|n| n.eq_ignore_ascii_case(&active)) {
        out.ringtone = name.clone();
    }
    out
}

pub struct UiParts {
    pub handle: UiHandle,
    pub tap: Arc<ScreenTap>,
}

pub async fn start(
    ctx: &Context,
    hal: Option<SharedHal>,
    config: Option<SharedConfig>,
    hub: &ConfigHub,
) -> Option<UiParts> {
    ctx.store.set_health(Subsystem::Ui, HealthStatus::Starting);
    // Declared before anything can return: a phone without its UI is not
    // healthy, whatever the reason, and must not confirm a trial boot.
    ctx.health.require("ui");
    let Some(hal) = hal else {
        error!("ui: no hardware layer; the UI is not started");
        ctx.store.set_health(Subsystem::Ui, HealthStatus::Failed("no hardware layer".into()));
        return None;
    };
    let tap = Arc::new(ScreenTap::default());
    let current = config.as_ref().map(|c| c.lock().unwrap().config());
    let cfg = ui_config(ctx, current.as_deref());
    let health = ctx.health.clone();
    let store = ctx.store.clone();
    // The UI is built on its own thread as well as run there: taking over
    // the display is a blocking device call (a mode set that waits for the
    // panel), and a display that does not answer must not stall the runtime
    // that SSH and the diagnostics run on.
    let (handle_tx, handle_rx) = tokio::sync::oneshot::channel();
    let thread_tap = tap.clone();
    let spawned = std::thread::Builder::new().name("ui".into()).spawn(move || {
        // The UI owns its devices and is not Sync; it lives on this thread.
        let ui_store = store.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let ui = {
                let Ok(mut hal) = hal.lock() else { return };
                let inner = std::mem::replace(&mut hal.display, Box::new(dphone_hal::Null("display")));
                hal.display = Box::new(TapDisplay { inner, tap: thread_tap });
                Ui::from_hal(&mut hal, ui_store, cfg)
            };
            let _ = handle_tx.send(ui.handle());
            ui.run()
        }));
        health.fail("ui");
        let why = if result.is_err() { "the UI thread panicked" } else { "the UI thread ended" };
        error!("ui: {why}");
        store.set_health(Subsystem::Ui, HealthStatus::Failed(why.into()));
    });
    if let Err(e) = spawned {
        error!("ui: cannot start the UI thread: {e}");
        ctx.store.set_health(Subsystem::Ui, HealthStatus::Failed(e.to_string()));
        return None;
    }
    let handle = match tokio::time::timeout(Duration::from_secs(20), handle_rx).await {
        Ok(Ok(handle)) => handle,
        Ok(Err(_)) => {
            ctx.store.set_health(Subsystem::Ui, HealthStatus::Failed("the UI did not start".into()));
            return None;
        }
        Err(_) => {
            error!("ui: the display did not answer within 20 s; continuing without the UI");
            ctx.health.fail("ui");
            ctx.store.set_health(Subsystem::Ui, HealthStatus::Failed("the display does not answer".into()));
            return None;
        }
    };
    info!("ui: running");

    // Healthy once it has stayed up for a moment.
    {
        let ctx = ctx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(3)).await;
            if !matches!(
                ctx.store.get::<dphone_state::Health>().subsystems.iter().find(|s| s.subsystem == Subsystem::Ui).map(|s| &s.status),
                Some(HealthStatus::Failed(_))
            ) {
                ctx.health.satisfy("ui");
                ctx.store.set_health(Subsystem::Ui, HealthStatus::Ok);
            }
        });
    }

    // Follow the configuration, and re-evaluate the zone's offset now and
    // then (daylight saving changes, and the clock being set by SNTP).
    {
        let handle = handle.clone();
        let ctx2 = ctx.clone();
        hub.on_change(move |cfg| handle.set_config(ui_config(&ctx2, Some(cfg))));
    }
    if let Some(config) = config {
        let handle = handle.clone();
        let ctx = ctx.clone();
        tokio::spawn(async move {
            let mut last = None;
            loop {
                let cfg = config.lock().unwrap().config();
                let next = ui_config(&ctx, Some(&cfg));
                if last.as_ref() != Some(&next) {
                    handle.set_config(next.clone());
                    last = Some(next);
                }
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        });
    }
    Some(UiParts { handle, tap })
}
