//! The electronic hook switch (EHS) headset adapter on UART A.
//!
//! The adapter and the serial console use the same UART and cannot share
//! it (the adapters need 9600 8N1 or 2400 8N2).  The `serial_console`
//! switch in `/nvdata/dphone_settings.txtpb` decides who has it: while it is
//! on (the default) nothing here runs; when it is off, this subsystem
//! claims the UART (the init role then takes its console shell away, and
//! kernel messages stop going to the port), and drives the adapter the
//! stock `ehs` setting names (`plantronics`, `jabra_iq`, or `auto`).
//!
//! The protocols are in `dphone_hal`.  This file is the phone's side: the
//! adapter is told when the phone rings and when call audio is on the
//! headset, and its buttons press the headset and mute keys, reject the
//! ringing call, or redial, as the stock daemon's did.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use dphone_hal::{Adapter, EhsHost, EhsMode, HeadsetAction, Identity, PhoneStatus, SerialConfig, Timing};
use dphone_state::{Audio, AudioPath, CallState, Calls, Command, Direction, HeadsetKind, Key, Store};
use log::{info, warn};

use super::hal::SharedHal;
use crate::diag::Output;
use crate::mainrole::Context;

/// How long the init role gets to take its console shell off the UART
/// after the claim appears (it looks about once a second).
const CONSOLE_HANDOVER: Duration = Duration::from_millis(2500);

#[derive(Default)]
struct Shared {
    /// The protocol to run; `None` leaves the UART to the console.
    wanted: Mutex<Option<EhsMode>>,
    changed: Condvar,
    adapter: Mutex<Option<Adapter>>,
    /// The UART is ours right now.
    claimed: AtomicBool,
}

/// Handle for configuration changes.
#[derive(Clone)]
pub struct Ehs {
    shared: Arc<Shared>,
}

/// What the configuration asks for.
pub fn wanted_mode(config: &dphone_config::Config) -> Option<EhsMode> {
    if config.serial_console().value {
        None
    } else {
        Some(EhsMode::from_setting(config.setting("ehs").unwrap_or("auto")))
    }
}

impl Ehs {
    pub fn apply_config(&self, config: &dphone_config::Config) {
        let wanted = wanted_mode(config);
        let mut current = self.shared.wanted.lock().unwrap();
        if *current != wanted {
            info!("ehs: {}", match wanted {
                Some(mode) => format!("serial console is off; adapter mode {mode:?}"),
                None => "off; the serial console keeps the UART".to_string(),
            });
            *current = wanted;
            self.shared.changed.notify_all();
        }
    }
}

struct Host {
    store: Store,
    shared: Arc<Shared>,
    mode: EhsMode,
}

impl EhsHost for Host {
    fn status(&mut self) -> PhoneStatus {
        self.store.read::<Audio, _>(|a| PhoneStatus {
            headset_active: a.path == AudioPath::Headset,
            muted: a.muted,
            ringing: a.ringing,
            bluetooth_headset: a.headset_kind == HeadsetKind::Bluetooth,
        })
    }

    fn action(&mut self, action: HeadsetAction) {
        info!("ehs: headset button: {action:?}");
        let press = |key: Key| {
            self.store.post(Command::InjectKey { key, down: true });
            self.store.post(Command::InjectKey { key, down: false });
        };
        match action {
            HeadsetAction::ToggleHeadset => press(Key::Headset),
            HeadsetAction::ToggleMute => press(Key::Mute),
            HeadsetAction::Redial => {
                self.store.post(Command::Redial);
            }
            HeadsetAction::RejectCall => {
                let ringing = self.store.read::<Calls, _>(|c| {
                    c.calls
                        .iter()
                        .find(|c| c.state == CallState::Ringing && c.direction == Direction::Incoming)
                        .map(|c| c.id)
                });
                if let Some(call) = ringing {
                    self.store.post(Command::Reject { call, busy: false });
                }
            }
        }
    }

    fn adapter(&mut self, adapter: Option<Adapter>) {
        *self.shared.adapter.lock().unwrap() = adapter;
        self.store.update::<Audio, _>(|a| match (adapter, a.headset_kind) {
            (Some(_), HeadsetKind::Wired) => a.headset_kind = HeadsetKind::Ehs,
            (None, HeadsetKind::Ehs) => a.headset_kind = HeadsetKind::Wired,
            _ => {}
        });
        if adapter.is_none() {
            info!("ehs: adapter lost");
        }
    }

    fn stop(&mut self) -> bool {
        *self.shared.wanted.lock().unwrap() != Some(self.mode)
    }
}

fn identity(ctx: &Context) -> Identity {
    let mut firmware = [0u8; 3];
    for (slot, part) in firmware.iter_mut().zip(crate::VERSION.split('.')) {
        *slot = part.parse().unwrap_or(0);
    }
    Identity { model_code: dphone_hal::jabra::model_code(ctx.model.name()), firmware }
}

/// Take the UART from the console: the init role sees the claim file and
/// removes its shell; the kernel stops printing to the port.
fn claim(ctx: &Context, shared: &Shared) {
    if let Err(e) = std::fs::write(ctx.paths.console_claimed(), b"ehs\n") {
        warn!("ehs: cannot claim the UART: {e}");
    }
    if !ctx.sim {
        crate::sys::kernel_console(false);
    }
    shared.claimed.store(true, Ordering::Relaxed);
}

fn release(ctx: &Context, shared: &Shared) {
    shared.claimed.store(false, Ordering::Relaxed);
    if !ctx.sim {
        crate::sys::kernel_console(true);
    }
    let _ = std::fs::remove_file(ctx.paths.console_claimed());
}

fn worker(ctx: Context, hal: SharedHal, shared: Arc<Shared>) {
    let mut ehs = {
        let mut hal = hal.lock().unwrap();
        std::mem::replace(&mut hal.ehs, Box::new(dphone_hal::Null("ehs")))
    };
    let identity = identity(&ctx);
    loop {
        // Wait until EHS is wanted.
        let mode = {
            let mut wanted = shared.wanted.lock().unwrap();
            loop {
                match *wanted {
                    Some(mode) => break mode,
                    None => wanted = shared.changed.wait(wanted).unwrap(),
                }
            }
        };
        if !shared.claimed.load(Ordering::Relaxed) {
            claim(&ctx, &shared);
            if !ctx.sim {
                std::thread::sleep(CONSOLE_HANDOVER);
            }
        }
        let mut host = Host { store: ctx.store.clone(), shared: shared.clone(), mode };
        match ehs.open(SerialConfig::PLANTRONICS) {
            Ok(mut port) => {
                info!("ehs: looking for an adapter ({mode:?})");
                if let Err(e) = dphone_hal::run(port.as_mut(), mode, identity, &Timing::STOCK, &mut host) {
                    warn!("ehs: {e}");
                }
                host.adapter(None);
            }
            Err(e) => warn!("ehs: cannot open the UART: {e}"),
        }
        // Dropping the port restored its line settings.  Give the UART back
        // unless EHS is still wanted (a mode change, or an error to retry).
        let still_wanted = shared.wanted.lock().unwrap().is_some();
        if still_wanted {
            if !EhsHost::stop(&mut host) {
                // Same mode: the run ended on an error; do not spin.
                std::thread::sleep(Duration::from_secs(5));
            }
        } else {
            release(&ctx, &shared);
        }
    }
}

/// Start the subsystem.  It does nothing until the configuration turns the
/// serial console off.
pub fn start(ctx: &Context, hal: Option<SharedHal>) -> Ehs {
    let shared = Arc::new(Shared::default());
    // A claim left by a main role that died belongs to nobody.
    let _ = std::fs::remove_file(ctx.paths.console_claimed());
    match hal {
        Some(hal) => {
            let (ctx2, shared2) = (ctx.clone(), shared.clone());
            if let Err(e) = std::thread::Builder::new().name("ehs".into()).spawn(move || worker(ctx2, hal, shared2)) {
                warn!("ehs: cannot start: {e}");
            }
        }
        None => warn!("ehs: no hardware layer; not started"),
    }
    let status = shared.clone();
    ctx.diag.register("ehs", "show the EHS headset adapter state", move |_args, out: Output| {
        let status = status.clone();
        async move {
            let wanted = *status.wanted.lock().unwrap();
            let adapter = *status.adapter.lock().unwrap();
            let text = match wanted {
                None => "off: the serial console has UART A (serial_console is on in dphone_settings.txtpb)\n".to_string(),
                Some(mode) => format!(
                    "mode: {mode:?}\nuart: {}\nadapter: {}\n",
                    if status.claimed.load(Ordering::Relaxed) { "claimed" } else { "not yet claimed" },
                    match adapter {
                        Some(a) => format!("{a:?}"),
                        None => "none detected".to_string(),
                    }
                ),
            };
            out.out(&text).await;
            0
        }
    });
    Ehs { shared }
}
