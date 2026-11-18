//! The electronic hook switch (EHS) headset adapter on UART A.
//!
//! The adapter and the serial console use the same UART and cannot share it
//! (the adapters need 9600 8N1 or 2400 8N2).  The `serial_console` switch in
//! `/nvdata/opendeskphone/settings.txtpb` decides who has it.  Off (the
//! default), the UART is the adapter's: this subsystem drives the adapter the
//! stock `ehs` setting names (`plantronics`, `jabra_iq`, or `auto`), and
//! nothing else writes to the port.  The phone boots that way: the kernel
//! keeps its messages off the port, the init role runs no shell on it, and the
//! main role's own output goes nowhere.  On, this subsystem hands the port to
//! the console: kernel messages and the main role's log copies go to it, and
//! the init role runs its shell there
//! ([`crate::paths::Paths::serial_console`]).
//!
//! The protocols are in `opendeskphone_hal`.  This file is the phone's side:
//! the adapter is told when the phone rings and when call audio is on the
//! headset, and its buttons press the headset and mute keys, reject the
//! ringing call, or redial, as the stock daemon's did.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use opendeskphone_hal::{Adapter, EhsHost, EhsMode, HeadsetAction, Identity, PhoneStatus, SerialConfig, Timing};
use opendeskphone_state::{Audio, AudioPath, CallState, Calls, Command, Direction, HeadsetKind, Key, Store};
use log::{info, warn};

use super::hal::SharedHal;
use crate::diag::Output;
use crate::mainrole::Context;

/// How long the init role gets to take its console shell off the UART
/// after the console loses it (it looks about once a second).
const CONSOLE_HANDOVER: Duration = Duration::from_millis(2500);

/// Who the configuration gives the UART to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Wanted {
    /// No configuration has been applied yet: the UART is left alone.
    #[default]
    Undecided,
    Console,
    /// The EHS adapter, with the protocol to run.
    Ehs(EhsMode),
}

#[derive(Default)]
struct Shared {
    wanted: Mutex<Wanted>,
    changed: Condvar,
    adapter: Mutex<Option<Adapter>>,
    /// The serial console has the UART right now.
    console: AtomicBool,
}

/// Handle for configuration changes.
#[derive(Clone)]
pub struct Ehs {
    shared: Arc<Shared>,
}

/// What the configuration asks for.
fn wanted(config: &opendeskphone_config::Config) -> Wanted {
    if config.serial_console().value {
        Wanted::Console
    } else {
        Wanted::Ehs(EhsMode::from_setting(config.setting("ehs").unwrap_or("auto")))
    }
}

impl Ehs {
    pub fn apply_config(&self, config: &opendeskphone_config::Config) {
        self.set(wanted(config));
    }

    /// Without a configuration nothing says an adapter is there, and the
    /// serial console is a way in: it gets the UART.
    pub fn no_config(&self) {
        self.set(Wanted::Console);
    }

    fn set(&self, wanted: Wanted) {
        let mut current = self.shared.wanted.lock().unwrap();
        if *current != wanted {
            info!("ehs: {}", match wanted {
                Wanted::Ehs(mode) => format!("the adapter has the UART; mode {mode:?}"),
                _ => "off; the serial console has the UART (serial_console is on)".to_string(),
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
        // The ringer also sounds with no call (a ringtone tried in the
        // settings, the volume keys); that is nothing for the headset.
        let alerting = self.store.read::<Calls, _>(|c| c.ringing().next().is_some());
        self.store.read::<Audio, _>(|a| PhoneStatus {
            headset_active: a.path == AudioPath::Headset,
            muted: a.muted,
            ringing: a.ringing && alerting,
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
        *self.shared.wanted.lock().unwrap() != Wanted::Ehs(self.mode)
    }
}

fn identity(ctx: &Context) -> Identity {
    let mut firmware = [0u8; 3];
    for (slot, part) in firmware.iter_mut().zip(crate::VERSION.split('.')) {
        *slot = part.parse().unwrap_or(0);
    }
    Identity { model_code: opendeskphone_hal::jabra::model_code(ctx.model.name()), firmware }
}

/// Give the UART to the console: kernel messages and this process's log
/// copies go to it, and the init role, seeing the file, runs its shell
/// there.
fn give_console(ctx: &Context, shared: &Shared) {
    if let Err(e) = std::fs::write(ctx.paths.serial_console(), b"main\n") {
        warn!("ehs: cannot hand the UART to the console: {e}");
    }
    if !ctx.sim {
        crate::sys::serial_console_output(true);
        crate::sys::stdio_to_console(true);
    }
    shared.console.store(true, Ordering::Relaxed);
}

/// Take the UART for the adapter.  Returns whether the console had it (the
/// init role then needs a moment to take its shell away).
fn take_uart(ctx: &Context, shared: &Shared) -> bool {
    // The file may also be the init role's, from after a main role failed.
    let had = shared.console.swap(false, Ordering::Relaxed) | ctx.paths.serial_console().exists();
    if !ctx.sim {
        crate::sys::stdio_to_console(false);
        crate::sys::serial_console_output(false);
    }
    let _ = std::fs::remove_file(ctx.paths.serial_console());
    had
}

fn worker(ctx: Context, hal: SharedHal, shared: Arc<Shared>) {
    let mut ehs = {
        let mut hal = hal.lock().unwrap();
        std::mem::replace(&mut hal.ehs, Box::new(opendeskphone_hal::Null("ehs")))
    };
    let identity = identity(&ctx);
    // The UART is ours: nothing else may be written to it.
    let mut ours = false;
    loop {
        // Wait until EHS is wanted; meanwhile the console may be.
        let mode = {
            let mut wanted = shared.wanted.lock().unwrap();
            loop {
                match *wanted {
                    Wanted::Ehs(mode) => break mode,
                    Wanted::Console => {
                        if !shared.console.load(Ordering::Relaxed) {
                            give_console(&ctx, &shared);
                            ours = false;
                        }
                    }
                    Wanted::Undecided => {}
                }
                wanted = shared.changed.wait(wanted).unwrap();
            }
        };
        if !ours {
            if take_uart(&ctx, &shared) && !ctx.sim {
                std::thread::sleep(CONSOLE_HANDOVER);
            }
            ours = true;
        }
        let mut host = Host { store: ctx.store.clone(), shared: shared.clone(), mode };
        match ehs.open(SerialConfig::PLANTRONICS) {
            Ok(mut port) => {
                info!("ehs: looking for an adapter ({mode:?})");
                if let Err(e) = opendeskphone_hal::run(port.as_mut(), mode, identity, &Timing::STOCK, &mut host) {
                    warn!("ehs: {e}");
                }
                host.adapter(None);
            }
            Err(e) => warn!("ehs: cannot open the UART: {e}"),
        }
        // Dropping the port restored its line settings.  The top of the
        // loop gives the UART to the console if that is wanted now.
        if !EhsHost::stop(&mut host) {
            // Same mode: the run ended on an error; do not spin.
            std::thread::sleep(Duration::from_secs(5));
        }
    }
}

/// Start the subsystem.  It leaves the UART as it is until a configuration
/// is applied.
pub fn start(ctx: &Context, hal: Option<SharedHal>) -> Ehs {
    let shared = Arc::new(Shared::default());
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
    ctx.diag.register(move |_: EhsCommand, out: Output| {
        let status = status.clone();
        async move {
            let wanted = *status.wanted.lock().unwrap();
            let adapter = *status.adapter.lock().unwrap();
            let text = match wanted {
                Wanted::Undecided => "waiting for the configuration\n".to_string(),
                Wanted::Console => "off: the serial console has UART A (serial_console is on)\n".to_string(),
                Wanted::Ehs(mode) => format!(
                    "mode: {mode:?}\nuart: {}\nadapter: {}\n",
                    if status.console.load(Ordering::Relaxed) { "still the console's" } else { "claimed" },
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

/// Show the state of the EHS headset adapter
#[derive(Debug, clap::Parser)]
#[command(name = "ehs")]
pub struct EhsCommand {}
