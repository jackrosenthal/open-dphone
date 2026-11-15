//! dphone-hal: open the hardware (or the simulator) and offer the `hal`
//! diagnostic command.
//!
//! Opening the hardware shows nothing: the panels and the backlight stay as
//! the kernel left them, and the UI turns them on with its first frame (the
//! boot screen), at the configured brightness.  `dphone hal pattern` puts a
//! test pattern on a display the UI has not taken.

use std::sync::{Arc, Mutex};
use std::time::Duration;

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
            ctx.diag.register("hal", "poke hardware (failed to open)", |_a, out: Output| async move {
                out.err("dphone hal: the hardware layer did not open; see 'dphone log'\n").await;
                crate::diag::EXIT_UNAVAILABLE
            });
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
    ctx.diag.register(
        "hal",
        "hal status | lamp NAME|all off|red|green|amber | backlight LEVEL | contrast LEVEL | pattern [bars|white|black]",
        move |args: Vec<String>, out: Output| {
            let handle = handle.clone();
            let inventory = inventory.clone();
            async move {
                // Hardware calls are short but blocking (SPI flush, sysfs).
                let result = tokio::task::spawn_blocking(move || command(&handle, &inventory, &args)).await;
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
        },
    );
    Some(shared)
}

/// Draws a test pattern and turns the panel on with it (the diagnostic
/// command; nothing calls this at start-up).
fn pattern(hal: &mut Hal, screen: Screen, kind: &str) -> Result<(), String> {
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
                "white" => (255, 255, 255),
                "black" => (0, 0, 0),
                _ => BARS[(x * 8 / w.max(1)).min(7)],
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

fn parse_lamp(name: &str) -> Option<Lamp> {
    let name = name.to_ascii_lowercase();
    match name.as_str() {
        "message" | "mwi" => Some(Lamp::Message),
        "mute" => Some(Lamp::Mute),
        "headset" => Some(Lamp::Headset),
        "speaker" => Some(Lamp::Speaker),
        _ => {
            if let Some(n) = name.strip_prefix("line") {
                n.parse::<u8>().ok().filter(|n| *n >= 1).map(|n| Lamp::Line(n - 1))
            } else if let Some(n) = name.strip_prefix("blf") {
                n.parse::<u8>().ok().filter(|n| *n >= 1).map(|n| Lamp::Blf(n - 1))
            } else {
                None
            }
        }
    }
}

fn command(handle: &SharedHal, inventory: &str, args: &[String]) -> (i32, String, String) {
    let Ok(mut hal) = handle.lock() else {
        return (1, String::new(), "dphone hal: lock poisoned\n".into());
    };
    let hal = &mut *hal;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let fail = |e: String| (1, String::new(), format!("dphone hal: {e}\n"));
    match args.as_slice() {
        [] | ["status"] => {
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
        ["lamp", name, color] => {
            let color = match *color {
                "off" => LampColor::Off,
                "red" => LampColor::Red,
                "green" => LampColor::Green,
                "amber" => LampColor::Amber,
                other => return fail(format!("unknown color '{other}'")),
            };
            let lamps = if *name == "all" {
                hal.lamps.lamps()
            } else {
                match parse_lamp(name) {
                    Some(l) => vec![l],
                    None => return fail(format!("unknown lamp '{name}' (lineN, blfN, message, mute, headset, speaker, all)")),
                }
            };
            for lamp in lamps {
                if let Err(e) = hal.lamps.set(lamp, color) {
                    return fail(format!("{lamp:?}: {e}"));
                }
            }
            (0, String::new(), String::new())
        }
        ["backlight", level] => match level.parse::<u8>() {
            Ok(level) => match hal.backlight.set_brightness(Screen::Main, level) {
                Ok(()) => (0, String::new(), String::new()),
                Err(e) => fail(e.to_string()),
            },
            Err(_) => fail("backlight needs a number".into()),
        },
        ["contrast", level] => match level.parse::<u8>() {
            Ok(level) => match hal.backlight.set_contrast(Screen::Main, level) {
                Ok(()) => (0, String::new(), String::new()),
                Err(e) => fail(e.to_string()),
            },
            Err(_) => fail("contrast needs a number".into()),
        },
        ["pattern"] | ["pattern", _] => {
            let kind = args.get(1).copied().unwrap_or("bars");
            for screen in hal.display.screens() {
                if let Err(e) = pattern(hal, screen, kind) {
                    return fail(format!("{screen:?}: {e}"));
                }
            }
            (0, String::new(), String::new())
        }
        _ => (
            2,
            String::new(),
            "usage: dphone hal status | lamp NAME|all COLOR | backlight LEVEL | contrast LEVEL | pattern [bars|white|black]\n".into(),
        ),
    }
}
