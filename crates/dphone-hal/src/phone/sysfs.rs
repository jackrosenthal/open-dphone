//! Backlight, contrast, and lamps through sysfs class devices.

use super::sys::*;
use crate::*;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Name the gray-LCD driver gives the D70 side display's backlight.
const SIDE_BACKLIGHT: &str = "digium-dss";

fn class_entries(class: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(Path::new("/sys/class").join(class))
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

fn file_name(p: &Path) -> &str {
    p.file_name().and_then(|n| n.to_str()).unwrap_or("")
}

/// Backlight class and LCD class (contrast) devices.
pub struct SysBacklight {
    model: Model,
    main_bl: Option<PathBuf>,
    side_bl: Option<PathBuf>,
    main_lcd: Option<PathBuf>,
    side_lcd: Option<PathBuf>,
}

/// Top of the stock percent-to-value contrast table for the main gray
/// panels (`DEFAULT_MAIN_CONTRAST_TABLE`: 0, 5, 10, ... 506).
const MAIN_CONTRAST_TOP: u32 = 506;

impl SysBacklight {
    pub fn open(model: Model) -> Result<SysBacklight> {
        let bls = class_entries("backlight");
        let lcds = class_entries("lcd");
        let b = SysBacklight {
            model,
            main_bl: bls.iter().find(|p| file_name(p) != SIDE_BACKLIGHT).cloned(),
            side_bl: bls.iter().find(|p| file_name(p) == SIDE_BACKLIGHT).cloned(),
            main_lcd: lcds.iter().find(|p| !file_name(p).ends_with("-dss")).cloned(),
            side_lcd: lcds.iter().find(|p| file_name(p).ends_with("-dss")).cloned(),
        };
        if b.main_bl.is_none() {
            return Err(HalError::Unavailable("no backlight class device".into()));
        }
        Ok(b)
    }

    fn bl(&self, screen: Screen) -> Result<&Path> {
        match screen {
            Screen::Main => self.main_bl.as_deref(),
            Screen::Side => self.side_bl.as_deref(),
        }
        .ok_or(HalError::Unsupported("no backlight for that screen"))
    }

    fn lcd(&self, screen: Screen) -> Result<&Path> {
        match screen {
            Screen::Main => self.main_lcd.as_deref(),
            Screen::Side => self.side_lcd.as_deref(),
        }
        .ok_or(HalError::Unsupported("no contrast control for that screen"))
    }

    /// Raw value that 100 percent maps to.
    fn contrast_top(&self, screen: Screen) -> Result<u32> {
        let p = self.lcd(screen)?.join("max_contrast");
        let max = read_u32(&p).map_err(|e| HalError::io(p.display().to_string(), e))?;
        Ok(if max == 511 { MAIN_CONTRAST_TOP } else { max })
    }
}

fn write_value(path: &Path, v: u32) -> Result<()> {
    std::fs::write(path, v.to_string()).map_err(|e| HalError::io(path.display().to_string(), e))
}

impl Backlight for SysBacklight {
    fn max_brightness(&self, screen: Screen) -> u8 {
        match self.bl(screen) {
            Ok(p) => read_u32(&p.join("max_brightness")).map(|v| v.min(255) as u8).unwrap_or(0),
            Err(_) => 0,
        }
    }

    fn brightness(&self, screen: Screen) -> Result<u8> {
        let p = self.bl(screen)?.join("brightness");
        read_u32(&p).map(|v| v.min(255) as u8).map_err(|e| HalError::io(p.display().to_string(), e))
    }

    fn set_brightness(&mut self, screen: Screen, level: u8) -> Result<()> {
        let dir = self.bl(screen)?;
        let max = self.max_brightness(screen);
        let level = level.min(max);
        // The side backlight registers powered off (as the vendor driver
        // leaves it); `bl_power` 0 is on, 4 is off.
        let _ = std::fs::write(dir.join("bl_power"), if level == 0 { "4" } else { "0" });
        write_value(&dir.join("brightness"), level as u32)
    }

    /// Contrast is a percentage (0..=100), as in the stock settings; it is
    /// scaled to the panel's raw range the way the stock tables do for the
    /// main panels (linear to 506) and linearly for the D70 side display
    /// (whose stock table is not monotonic and is not reproduced).
    fn max_contrast(&self, screen: Screen) -> u8 {
        if self.lcd(screen).is_ok() { 100 } else { 0 }
    }

    fn contrast(&self, screen: Screen) -> Result<u8> {
        let top = self.contrast_top(screen)?.max(1);
        let p = self.lcd(screen)?.join("contrast");
        let raw = read_u32(&p).map_err(|e| HalError::io(p.display().to_string(), e))?;
        Ok(((raw * 100 + top / 2) / top).min(100) as u8)
    }

    fn set_contrast(&mut self, screen: Screen, level: u8) -> Result<()> {
        let top = self.contrast_top(screen)?;
        let raw = (level.min(100) as u32 * top + 50) / 100;
        write_value(&self.lcd(screen)?.join("contrast"), raw)
    }
}

impl SysBacklight {
    /// The model this was opened for.
    pub fn model(&self) -> Model {
        self.model
    }
}

/// Parses an LED class name of the port's devicetrees:
/// `<color>:<function>[-<n>]`, e.g., `green:line-0`, `red:mail`.
pub(crate) fn parse_led_name(name: &str) -> Option<(Lamp, bool)> {
    let (color, func) = name.split_once(':')?;
    let red = match color {
        "red" => true,
        "green" => false,
        _ => return None,
    };
    let (func, n) = match func.rsplit_once('-') {
        Some((f, n)) if n.bytes().all(|b| b.is_ascii_digit()) && !n.is_empty() => {
            (f, n.parse::<u8>().ok())
        }
        _ => (func, None),
    };
    let lamp = match (func, n) {
        ("line", Some(n)) => Lamp::Line(n),
        ("blf", Some(n)) => Lamp::Blf(n),
        ("mail", _) => Lamp::Message,
        ("mute", _) => Lamp::Mute,
        ("headset", _) => Lamp::Headset,
        ("speaker", _) => Lamp::Speaker,
        _ => return None,
    };
    Some((lamp, red))
}

struct SysLamp {
    lamp: Lamp,
    red: Option<File>,
    green: Option<File>,
}

/// LED class lamps.
pub struct SysLamps {
    lamps: Vec<SysLamp>,
}

impl SysLamps {
    pub fn open() -> Result<SysLamps> {
        let mut lamps: Vec<SysLamp> = Vec::new();
        for dir in class_entries("leds") {
            let Some((lamp, red)) = parse_led_name(file_name(&dir)) else { continue };
            let Ok(f) = std::fs::OpenOptions::new().write(true).open(dir.join("brightness")) else {
                continue;
            };
            let idx = match lamps.iter().position(|l| l.lamp == lamp) {
                Some(i) => i,
                None => {
                    lamps.push(SysLamp { lamp, red: None, green: None });
                    lamps.len() - 1
                }
            };
            if red {
                lamps[idx].red = Some(f);
            } else {
                lamps[idx].green = Some(f);
            }
        }
        if lamps.is_empty() {
            return Err(HalError::Unavailable("no phone lamps under /sys/class/leds".into()));
        }
        lamps.sort_by_key(|l| l.lamp);
        Ok(SysLamps { lamps })
    }
}

fn led_write(f: &mut Option<File>, on: bool) -> Result<()> {
    if let Some(f) = f {
        // The LED core clamps to max_brightness; these lamps are on/off.
        f.write_all(if on { b"255" } else { b"0" }).map_err(|e| HalError::io("LED brightness", e))?;
    }
    Ok(())
}

impl Lamps for SysLamps {
    fn lamps(&self) -> Vec<Lamp> {
        self.lamps.iter().map(|l| l.lamp).collect()
    }

    fn set(&mut self, lamp: Lamp, color: LampColor) -> Result<()> {
        let Some(l) = self.lamps.iter_mut().find(|l| l.lamp == lamp) else { return Ok(()) };
        let (red, green) = match color {
            LampColor::Off => (false, false),
            LampColor::Red => (true, false),
            LampColor::Green => (false, true),
            LampColor::Amber => (true, true),
        };
        led_write(&mut l.red, red)?;
        led_write(&mut l.green, green)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn led_names() {
        assert_eq!(parse_led_name("green:line-0"), Some((Lamp::Line(0), false)));
        assert_eq!(parse_led_name("red:line-5"), Some((Lamp::Line(5), true)));
        assert_eq!(parse_led_name("red:blf-9"), Some((Lamp::Blf(9), true)));
        assert_eq!(parse_led_name("green:mail"), Some((Lamp::Message, false)));
        assert_eq!(parse_led_name("red:mute"), Some((Lamp::Mute, true)));
        assert_eq!(parse_led_name("green:headset"), Some((Lamp::Headset, false)));
        assert_eq!(parse_led_name("red:speaker"), Some((Lamp::Speaker, true)));
        assert_eq!(parse_led_name("mmc0::"), None);
        assert_eq!(parse_led_name("blue:line-0"), None);
        assert_eq!(parse_led_name("green:line"), None);
    }
}
