//! dphone-hal: open the hardware (or the simulator) and offer the `hal`
//! diagnostic command.
//!
//! Opening the hardware shows nothing: the panels and the backlight stay as
//! the kernel left them, and the UI turns them on with its first frame (the
//! boot screen), at the configured brightness.  `dphone hal pattern` puts a
//! test pattern on a display the UI has not taken.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use dphone_hal::{Hal, Lamp, LampColor, PixelFormat, Screen};
use dphone_state::{HealthStatus, Subsystem};
use log::{error, info, warn};

use super::contained;
use crate::diag::Output;
use crate::mainrole::{Context, state_model};

pub type SharedHal = Arc<Mutex<Hal>>;

/// The hardware random number generator, checked once at start.
///
/// Nothing in the main role draws from `/dev/hwrng` itself: the kernel's
/// `hwrng` thread feeds the device into the kernel pool (seen on the D60:
/// the `[hwrng]` thread runs, `rng_quality` is 1024, and "crng init done"
/// is logged), and every key the phone makes comes from that pool through
/// the operating system's generator.  What is worth knowing is whether the
/// device is there and answers, since the hardware layer falls back to
/// `/dev/urandom` without saying so.
fn rng_line(hal: &mut Hal, sim: bool) -> String {
    let mut probe = [0u8; 16];
    let read = hal.rng.fill(&mut probe);
    if sim {
        return "rng: simulated\n".to_string();
    }
    let present = std::path::Path::new("/dev/hwrng").exists();
    match (present, read) {
        (true, Ok(())) => "rng: /dev/hwrng\n".to_string(),
        (true, Err(e)) => {
            warn!("hal: /dev/hwrng does not answer: {e}");
            format!("rng: /dev/hwrng does not answer: {e}\n")
        }
        (false, _) => {
            warn!("hal: no /dev/hwrng; the kernel pool has no hardware source");
            "rng: no /dev/hwrng (the kernel pool has no hardware source)\n".to_string()
        }
    }
}

pub async fn start(ctx: &Context) -> Option<SharedHal> {
    let model = state_model(ctx.model);
    let sim = ctx.sim;
    // Device opens can block or (in a driver nobody has run yet) misbehave;
    // do them off the runtime and give up after a while.
    let built = contained("hal-open", Duration::from_secs(20), move || {
        if sim { Hal::sim(model) } else { Hal::phone(model) }
    })
    .await;
    let mut hal = match built {
        Ok(hal) => hal,
        Err(e) => {
            error!("hal: {e}; continuing without hardware access");
            ctx.store.set_health(Subsystem::Core, HealthStatus::Degraded(format!("hal: {e}")));
            ctx.diag.register_unavailable::<HalCommand>("the hardware layer did not open; see 'dphone log'");
            return None;
        }
    };
    if hal.missing.is_empty() {
        info!("hal: all devices opened");
    }
    // (The hardware layer has already logged each device it could not open.)
    // What the phone has, noted now: the UI takes the display, keys, and
    // lamps out of the bundle when it starts.
    let mut inventory = String::new();
    for screen in hal.display.screens() {
        if let Some(i) = hal.display.info(screen) {
            inventory.push_str(&format!(
                "display {screen:?}: {}x{} {:?}, backlight max {}\n",
                i.width,
                i.height,
                i.format,
                hal.backlight.max_brightness(screen)
            ));
        }
    }
    inventory.push_str(&rng_line(&mut hal, sim));
    let lamps: Vec<String> = hal.lamps.lamps().iter().map(|l| format!("{l:?}")).collect();
    inventory.push_str(&format!("lamps: {}\n", lamps.join(" ")));
    let inventory: Arc<str> = inventory.into();
    let shared: SharedHal = Arc::new(Mutex::new(hal));
    let handle = shared.clone();
    ctx.diag.register(move |args: HalCommand, out: Output| {
        let handle = handle.clone();
        let inventory = inventory.clone();
        async move {
            // Hardware calls are short but blocking (SPI flush, sysfs).
            let result = tokio::task::spawn_blocking(move || command(&handle, &inventory, args.action)).await;
            match result {
                Ok((code, text, err)) => {
                    if !text.is_empty() {
                        out.out(&text).await;
                    }
                    if !err.is_empty() {
                        out.err(&err).await;
                    }
                    code
                }
                Err(e) => {
                    out.err(&format!("dphone hal: {e}\n")).await;
                    1
                }
            }
        }
    });
    Some(shared)
}

/// Poke the hardware: lamps, backlight, contrast, and display patterns
///
/// Without a subcommand: status.  Once the UI has started it owns the
/// display, the keys, and the lamps; lamp, backlight, contrast, and pattern
/// act only before that (e.g., in a main role started without the UI).
#[derive(Debug, Parser)]
#[command(name = "hal")]
pub struct HalCommand {
    #[command(subcommand)]
    action: Option<HalAction>,
}

#[derive(Debug, Subcommand)]
enum HalAction {
    /// Show the model, the devices found, and the devices missing
    Status,
    /// Set a lamp
    Lamp {
        /// lineN, blfN, message, mute, headset, speaker, or all
        #[arg(value_parser = parse_lamps)]
        lamp: Lamps,
        color: Color,
    },
    /// Set the backlight of the main display
    Backlight { level: u8 },
    /// Set the contrast of the main display
    Contrast { level: u8 },
    /// Draw a test pattern on every display and turn it on
    Pattern {
        #[arg(default_value = "bars")]
        kind: Pattern,
    },
}

/// One lamp, or every lamp the phone has.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Lamps {
    One(Lamp),
    All,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Color {
    Off,
    Red,
    Green,
    Amber,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Pattern {
    /// Eight vertical color bars
    Bars,
    White,
    Black,
}

/// Draws a test pattern and turns the panel on with it (the diagnostic
/// command; nothing calls this at start-up).
fn pattern(hal: &mut Hal, screen: Screen, kind: Pattern) -> Result<(), String> {
    let info = hal.display.info(screen).ok_or("no such screen")?;
    let frame = hal.display.frame(screen).map_err(|e| e.to_string())?;
    let (w, h) = (info.width as usize, info.height as usize);
    // Eight vertical bars: white, yellow, cyan, green, magenta, red, blue, black.
    const BARS: [(u8, u8, u8); 8] = [
        (255, 255, 255),
        (255, 255, 0),
        (0, 255, 255),
        (0, 255, 0),
        (255, 0, 255),
        (255, 0, 0),
        (0, 0, 255),
        (0, 0, 0),
    ];
    for y in 0..h {
        for x in 0..w {
            let (r, g, b) = match kind {
                Pattern::White => (255, 255, 255),
                Pattern::Black => (0, 0, 0),
                Pattern::Bars => BARS[(x * 8 / w.max(1)).min(7)],
            };
            match info.format {
                PixelFormat::Xrgb8888 => {
                    let o = y * info.stride + x * 4;
                    if let Some(px) = frame.get_mut(o..o + 4) {
                        px.copy_from_slice(&[b, g, r, 0]);
                    }
                }
                PixelFormat::R8 => {
                    let gray = ((r as u32 * 77 + g as u32 * 150 + b as u32 * 29) >> 8) as u8;
                    if let Some(px) = frame.get_mut(y * info.stride + x) {
                        *px = gray;
                    }
                }
            }
        }
    }
    // On, for a panel nobody has turned on yet (that shows the pattern);
    // the flush is for one that was on already.
    hal.display.set_power(screen, true).map_err(|e| e.to_string())?;
    hal.display.flush(screen, &[]).map_err(|e| e.to_string())
}

fn parse_lamps(name: &str) -> Result<Lamps, String> {
    let number = |n: &str| n.parse::<u8>().ok().filter(|n| *n >= 1).map(|n| n - 1);
    let lamp = match name {
        "all" => return Ok(Lamps::All),
        "message" => Some(Lamp::Message),
        "mute" => Some(Lamp::Mute),
        "headset" => Some(Lamp::Headset),
        "speaker" => Some(Lamp::Speaker),
        _ => match (name.strip_prefix("line"), name.strip_prefix("blf")) {
            (Some(n), _) => number(n).map(Lamp::Line),
            (_, Some(n)) => number(n).map(Lamp::Blf),
            _ => None,
        },
    };
    lamp.map(Lamps::One).ok_or_else(|| "not a lamp (lineN, blfN, message, mute, headset, speaker, or all)".to_string())
}

fn command(handle: &SharedHal, inventory: &str, action: Option<HalAction>) -> (i32, String, String) {
    let Ok(mut hal) = handle.lock() else {
        return (1, String::new(), "dphone hal: lock poisoned\n".into());
    };
    let hal = &mut *hal;
    let fail = |e: String| (1, String::new(), format!("dphone hal: {e}\n"));
    match action.unwrap_or(HalAction::Status) {
        HalAction::Status => {
            let mut text = format!("model {:?}{}\n", hal.model, if hal.sim.is_some() { " (simulator)" } else { "" });
            text.push_str(inventory);
            if hal.display.screens().is_empty() {
                text.push_str("(the UI owns the display, keys, and lamps; lamp, backlight, and pattern act only before it starts)\n");
            }
            if hal.missing.is_empty() {
                text.push_str("missing: none\n");
            }
            for (name, why) in &hal.missing {
                text.push_str(&format!("missing: {name}: {why}\n"));
            }
            (0, text, String::new())
        }
        HalAction::Lamp { lamp, color } => {
            let color = match color {
                Color::Off => LampColor::Off,
                Color::Red => LampColor::Red,
                Color::Green => LampColor::Green,
                Color::Amber => LampColor::Amber,
            };
            let lamps = match lamp {
                Lamps::All => hal.lamps.lamps(),
                Lamps::One(l) => vec![l],
            };
            for lamp in lamps {
                if let Err(e) = hal.lamps.set(lamp, color) {
                    return fail(format!("{lamp:?}: {e}"));
                }
            }
            (0, String::new(), String::new())
        }
        HalAction::Backlight { level } => match hal.backlight.set_brightness(Screen::Main, level) {
            Ok(()) => (0, String::new(), String::new()),
            Err(e) => fail(e.to_string()),
        },
        HalAction::Contrast { level } => match hal.backlight.set_contrast(Screen::Main, level) {
            Ok(()) => (0, String::new(), String::new()),
            Err(e) => fail(e.to_string()),
        },
        HalAction::Pattern { kind } => {
            for screen in hal.display.screens() {
                if let Err(e) = pattern(hal, screen, kind) {
                    return fail(format!("{screen:?}: {e}"));
                }
            }
            (0, String::new(), String::new())
        }
    }
}
