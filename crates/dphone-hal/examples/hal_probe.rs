//! Prints what the phone HAL backend finds on this machine.  Read-only
//! except with `--draw` (test pattern), `--lamps` (cycle lamps), and
//! `--tone` (one second of 1 kHz on the handset).
//!
//! Usage: hal_probe [MODEL] [--draw] [--lamps] [--tone] [--keys SECONDS]

use dphone_hal::*;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let model = args
        .iter()
        .find_map(|a| a.parse::<Model>().ok())
        .or_else(|| std::fs::read_to_string("/proc/cmdline").ok().and_then(|c| Model::from_cmdline(&c)))
        .unwrap_or(Model::D60);
    let flag = |f: &str| args.iter().any(|a| a == f);
    println!("model: {model}");
    let mut hal = Hal::phone(model);
    for (name, err) in &hal.missing {
        println!("missing {name}: {err}");
    }

    for s in hal.display.screens() {
        let info = hal.display.info(s).unwrap();
        println!("display {s:?}: {}x{} stride {} {:?}", info.width, info.height, info.stride, info.format);
        if flag("--draw") {
            let bpp = info.format.bytes_per_pixel();
            let fb = hal.display.frame(s).unwrap();
            for y in 0..info.height as usize {
                for x in 0..info.width as usize {
                    let v = (((x / 8) + (y / 8)) % 4 * 85) as u8;
                    let o = y * info.stride + x * bpp;
                    fb[o..o + bpp].fill(v);
                }
            }
            println!("  power on: {:?}", hal.display.set_power(s, true));
            println!("  flush: {:?}", hal.display.flush(s, &[]));
        }
    }
    for s in [Screen::Main, Screen::Side] {
        let max = hal.backlight.max_brightness(s);
        if max > 0 {
            println!(
                "backlight {s:?}: {:?}/{max}, contrast {:?}/{}",
                hal.backlight.brightness(s),
                hal.backlight.contrast(s),
                hal.backlight.max_contrast(s)
            );
        }
    }

    println!("lamps: {:?}", hal.lamps.lamps());
    if flag("--lamps") {
        for lamp in hal.lamps.lamps() {
            for c in [LampColor::Red, LampColor::Green, LampColor::Amber, LampColor::Off] {
                let r = hal.lamps.set(lamp, c);
                if r.is_err() {
                    println!("  {lamp:?} {c:?}: {r:?}");
                }
                std::thread::sleep(Duration::from_millis(150));
            }
        }
    }

    println!("off hook: {:?}", hal.input.off_hook());
    if let Some(secs) = args.iter().position(|a| a == "--keys").and_then(|i| args.get(i + 1)) {
        let secs: u64 = secs.parse().unwrap_or(10);
        let end = std::time::Instant::now() + Duration::from_secs(secs);
        println!("press keys for {secs} s");
        while std::time::Instant::now() < end {
            if let Ok(Some(e)) = hal.input.next_event(Some(Duration::from_millis(200))) {
                println!("  {e:?}");
            }
        }
    }

    match hal.mtd.partitions() {
        Ok(parts) => {
            for p in parts {
                println!("mtd{} {:>8} {:#010x} erase {:#x}", p.index, p.name, p.size, p.erase_size);
            }
        }
        Err(e) => println!("mtd: {e}"),
    }

    println!("audio rate: {}", hal.audio.rate());
    match hal.audio.mixer() {
        Ok(mut m) => match m.controls() {
            Ok(ctls) => {
                for c in ctls {
                    let v = m.get(&c.name);
                    println!("  control {:?} {:?} {}..{} x{} = {:?} {:?}", c.name, c.kind, c.min, c.max, c.count, v, c.items);
                }
            }
            Err(e) => println!("mixer controls: {e}"),
        },
        Err(e) => println!("mixer: {e}"),
    }
    if flag("--tone") {
        let rate = hal.audio.rate();
        let r = (|| -> Result<()> {
            let mut mixer = hal.audio.mixer()?;
            apply_board_gains(mixer.as_mut(), model, PcmDevice::Handset, 0)?;
            let mut cfg = PcmConfig::voice(model);
            cfg.capture = false;
            let mut pcm = hal.audio.open(PcmDevice::Handset, cfg)?;
            println!("  pcm config: {:?}", pcm.config());
            // 1 kHz square-ish wave from a small integer table.
            let period = (rate / 1000) as usize;
            let buf: Vec<i16> = (0..pcm.config().period_frames as usize)
                .map(|i| if (i % period) < period / 2 { 6000 } else { -6000 })
                .collect();
            for _ in 0..(rate / pcm.config().period_frames) {
                pcm.write(&buf)?;
            }
            Ok(())
        })();
        println!("tone: {r:?}");
    }

    let mut b = [0u8; 8];
    println!("rng: {:?} {:02x?}", hal.rng.fill(&mut b).is_ok(), b);
}
