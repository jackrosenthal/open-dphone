//! The Diagnostics app's hardware tests, drawn by the phone UI on a
//! simulated D60 (color), D40, and D70 (gray), with the simulator's keys,
//! hookswitch, and lamps.
//!
//! The pictures are written as PNG files under `diagnostics/` in cargo's
//! test scratch directory (`CARGO_TARGET_TMPDIR`), or in the directory
//! `DIAGNOSTICS_PREVIEWS` names.  The menu, the status pages, and the tone
//! are tested in `apps.rs`.

mod common;

use common::{staged, temp_dir};
use dphone_apps::engine::Target;
use dphone_apps::testutil::{FakeConfig, FakePbx, services};
use dphone_apps::uishell::UiShell;
use dphone_apps::{AppManager, ManagerConfig};
use dphone_hal::{Hal, Lamp, Screen};
use dphone_state::{Domain, Key, Keys, LampColor, LampState, Model, Phase, Store, System};
use dphone_ui::{Ui, UiConfig};
use std::path::PathBuf;

const APP: &str = "diagnostics";
const WHITE: u32 = 0xFF_FFFF;
const BLACK: u32 = 0;

/// A simulated phone with the app on screen.
struct Phone {
    model: Model,
    ui: Ui,
    sim: dphone_hal::sim::SimHandle,
    mgr: AppManager,
    audio: dphone_state::CommandReceiver,
    calls: dphone_state::CommandReceiver,
    _dir: common::TempDir,
}

impl Phone {
    fn new(model: Model) -> Phone {
        let dir = temp_dir("diagnostics-phone");
        let mut hal = Hal::sim(model);
        let sim = hal.sim.clone().unwrap();
        let store = Store::new(model);
        let ui = Ui::from_hal(&mut hal, store.clone(), UiConfig::default());
        store.update::<System, _>(|s| {
            s.phase = Phase::Running;
            s.time_synced = true;
        });
        // The phone's own lamp, to tell it from the test's.
        store.update::<Keys, _>(|k| k.message_lamp = LampState::solid(LampColor::Red));
        let (audio, calls) = (store.handle(Domain::Audio), store.handle(Domain::Calls));
        let mut cfg = ManagerConfig::rooted(staged(), &dir.0.join("nvdata"), &dir.0.join("tmp"));
        cfg.target = Target::Pulley64;
        let mut mgr = AppManager::new(
            cfg,
            store,
            Box::new(UiShell::new(ui.handle(), model)),
            services(&FakePbx::default(), FakeConfig::default()),
        )
        .expect("engine");
        mgr.scan();
        let mut phone = Phone {
            model,
            ui,
            sim,
            mgr,
            audio,
            calls,
            _dir: dir,
        };
        phone.settle();
        phone.mgr.launch(APP).unwrap();
        phone.settle();
        assert_eq!(phone.ui.screen_name(), "app:diagnostics");
        phone
    }

    /// The UI and the manager take turns until both are done.
    fn settle(&mut self) {
        for _ in 0..4 {
            self.ui.pump();
            self.mgr.pump();
        }
        self.ui.pump();
    }

    fn tap(&mut self, keys: &[Key]) {
        for key in keys {
            self.sim.tap(*key);
            self.settle();
        }
    }

    fn hook(&mut self, off_hook: bool) {
        self.sim.set_hook(off_hook);
        self.settle();
    }

    /// Waits for the app's next timer.
    fn timer(&mut self) {
        let wait = self.mgr.pump().expect("a timer is pending");
        std::thread::sleep(wait + std::time::Duration::from_millis(20));
        self.settle();
    }

    /// How many commands the phone's audio and call sides were sent.
    fn commands(&self) -> usize {
        let mut n = 0;
        for rx in [&self.audio, &self.calls] {
            while let Some(env) = rx.try_recv() {
                env.done();
                n += 1;
            }
        }
        n
    }

    /// The pixels of the frame as 0xRRGGBB, and the frame's width.
    fn pixels(&self) -> (Vec<u32>, usize) {
        let f = self.sim.frame(Screen::Main).expect("main frame");
        let bpp = f.info.format.bytes_per_pixel();
        let mut out = Vec::new();
        for y in 0..f.info.height as usize {
            for x in 0..f.info.width as usize {
                let i = y * f.info.stride + x * bpp;
                out.push(if bpp == 4 {
                    u32::from_le_bytes([f.pixels[i], f.pixels[i + 1], f.pixels[i + 2], 0])
                } else {
                    u32::from(f.pixels[i]) * 0x01_0101
                });
            }
        }
        (out, f.info.width as usize)
    }

    /// The colors on the display, each once, darkest first.
    fn colors(&self) -> Vec<u32> {
        let mut colors = self.pixels().0;
        colors.sort_unstable();
        colors.dedup();
        colors
    }

    /// The color every lamp shows.
    fn lamps(&self) -> LampColor {
        let lamps = self.sim.lamps();
        assert_eq!(lamps.len(), Lamp::for_model(self.model).len());
        assert!(lamps.iter().all(|(_, c)| *c == lamps[0].1), "{lamps:?}");
        lamps[0].1
    }

    /// Writes the frame as a PNG (the gray ones doubled in size).
    fn shot(&self, name: &str) {
        let dir = match std::env::var_os("DIAGNOSTICS_PREVIEWS") {
            Some(d) => PathBuf::from(d),
            None => PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("diagnostics"),
        };
        std::fs::create_dir_all(&dir).unwrap();
        let (pixels, w) = self.pixels();
        let h = pixels.len() / w;
        let scale = if self.model.is_color() { 1 } else { 2 };
        let mut rgb = Vec::with_capacity(w * h * 3 * scale * scale);
        for y in 0..h * scale {
            for x in 0..w * scale {
                let p = pixels[(y / scale) * w + x / scale];
                rgb.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
            }
        }
        let path = dir.join(format!("{}-{name}.png", self.model.name()));
        let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
        let mut enc = png::Encoder::new(file, (w * scale) as u32, (h * scale) as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&rgb).unwrap();
    }
}

fn display_test(model: Model) {
    let mut p = Phone::new(model);
    let size = model.main_screen();
    let (w, h) = (size.width as usize, size.height as usize);
    p.shot("menu");
    p.tap(&[Key::Digit(1)]);
    // The whole display, the softkey bar's part too.
    p.shot("display-1-white");
    assert_eq!(p.colors(), [WHITE]);
    // Any key shows the next pattern: the phone's own keys too.
    p.tap(&[Key::VolumeUp]);
    p.shot("display-2-black");
    assert_eq!(p.colors(), [BLACK]);
    p.tap(&[Key::Line(1)]);
    p.shot("display-3-bars");
    let (px, _) = p.pixels();
    let bars: Vec<u32> = if model.is_color() {
        vec![0xFF_0000, 0x00_FF00, 0x00_00FF, 0xFF_FF00, 0x00_FFFF, 0xFF_00FF, WHITE, BLACK]
    } else {
        vec![BLACK, 0x55_5555, 0xAA_AAAA, WHITE]
    };
    let bar = w / bars.len();
    for (i, color) in bars.iter().enumerate() {
        for x in [i * bar, i * bar + bar - 1] {
            for y in [0, h - 1] {
                assert_eq!(px[y * w + x], *color, "bar {i} at {x},{y}");
            }
        }
    }
    p.tap(&[Key::Speaker]);
    p.shot("display-4-pixels");
    let (px, _) = p.pixels();
    for y in 0..h {
        for x in 0..w {
            let want = if x % 2 == 0 && y % 2 == 0 { BLACK } else { WHITE };
            assert_eq!(px[y * w + x], want, "{x},{y}");
        }
    }
    p.tap(&[Key::Soft(2)]);
    p.shot("display-5-frame");
    let (px, _) = p.pixels();
    let at = |x: usize, y: usize| px[y * w + x];
    assert_eq!([at(0, 0), at(w - 1, h - 1), at(0, h - 1), at(w - 1, 0)], [BLACK; 4]);
    assert_eq!([at(1, 1), at(w - 2, h - 2)], [WHITE; 2]);
    assert_eq!([at(4, 4), at(w - 5, h - 5), at(4, h - 5)], [0x55_5555; 3]);
    assert_eq!([at(w / 2, 2), at(2, h / 2)], [BLACK; 2]);
    p.tap(&[Key::Mute]);
    p.shot("display-6-fonts");
    assert!(p.colors().len() > 1);
    // The hookswitch is not a key, and the phone does nothing with it.
    p.hook(true);
    p.hook(false);
    assert_eq!(p.ui.screen_name(), "app:diagnostics");
    assert_eq!(p.commands(), 0, "the phone did nothing with the keys");
    // Any key after the last pattern: the menu.
    let fonts = p.pixels().0;
    p.tap(&[Key::Hold]);
    assert_ne!(p.pixels().0, fonts);
    // Cancel leaves at once, and the keys are the phone's again.
    p.tap(&[Key::Digit(1)]);
    assert_eq!(p.colors(), [WHITE]);
    p.tap(&[Key::Cancel]);
    assert!(p.colors().len() > 1);
    p.tap(&[Key::VolumeUp]);
    assert_eq!(p.commands(), 1, "the volume key is the phone's");
    p.tap(&[Key::Line(1)]);
    assert_eq!(p.ui.screen_name(), "idle");
    p.settle();
    assert!(!p.mgr.is_running(APP));
}

fn key_test(model: Model) {
    let mut p = Phone::new(model);
    p.tap(&[Key::Digit(2)]);
    let empty = p.pixels().0;
    p.tap(&[Key::Line(1), Key::VolumeUp, Key::VolumeDown, Key::Speaker, Key::Soft(1), Key::Digit(5), Key::Cancel, Key::Hold]);
    p.hook(true);
    p.hook(false);
    p.shot("keys");
    assert_ne!(p.pixels().0, empty);
    assert_eq!(p.ui.screen_name(), "app:diagnostics");
    assert_eq!(p.commands(), 0, "the phone did nothing with the keys");
    // Cancel twice in a row: the menu, and the keys are the phone's again.
    p.tap(&[Key::Cancel, Key::Cancel]);
    assert_eq!(p.ui.screen_name(), "app:diagnostics");
    p.tap(&[Key::VolumeUp]);
    assert_eq!(p.commands(), 1, "the volume key is the phone's");
    p.tap(&[Key::Line(1)]);
    assert_eq!(p.ui.screen_name(), "idle");
}

fn lamp_test(model: Model) {
    let mut p = Phone::new(model);
    assert_eq!(p.sim.lamp(Lamp::Message), LampColor::Red);
    assert_eq!(p.sim.lamp(Lamp::Line(0)), LampColor::Off);
    p.tap(&[Key::Digit(3)]);
    p.shot("lamps");
    assert_eq!(p.lamps(), LampColor::Red);
    for color in [LampColor::Green, LampColor::Amber, LampColor::Off, LampColor::Red, LampColor::Green] {
        p.timer();
        assert_eq!(p.lamps(), color);
    }
    // Left with Select: the phone's lamps.
    p.tap(&[Key::Select]);
    assert_eq!(p.ui.screen_name(), "app:diagnostics");
    assert_eq!((p.sim.lamp(Lamp::Message), p.sim.lamp(Lamp::Line(0))), (LampColor::Red, LampColor::Off));
    // The app left in the middle of the test (line key 1): the same.
    p.tap(&[Key::Digit(3)]);
    p.timer();
    assert_eq!(p.lamps(), LampColor::Green);
    p.tap(&[Key::Line(1)]);
    assert_eq!(p.ui.screen_name(), "idle");
    assert_eq!((p.sim.lamp(Lamp::Message), p.sim.lamp(Lamp::Line(0))), (LampColor::Red, LampColor::Off));
    p.settle();
    assert!(!p.mgr.is_running(APP));
}

#[test]
fn the_display_test_on_every_skin() {
    for model in [Model::D60, Model::D40, Model::D70] {
        display_test(model);
    }
}

#[test]
fn the_key_test_on_every_skin() {
    for model in [Model::D60, Model::D40, Model::D70] {
        key_test(model);
    }
}

#[test]
fn the_lamp_test_on_every_skin() {
    for model in [Model::D60, Model::D40, Model::D70] {
        lamp_test(model);
    }
}
