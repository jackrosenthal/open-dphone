//! Box Pusher, the puzzle: the app's life under the manager with the fake
//! screen, and the real thing drawn by the phone UI on a simulated D60
//! (color), D40, and D70 (gray).
//!
//! The pictures are written as PNG files under `box-pusher/` in cargo's
//! test scratch directory (`CARGO_TARGET_TMPDIR`), or in the directory
//! `BOX_PUSHER_PREVIEWS` names.  The rules, the levels, and the solver
//! that proves them are tested in the crate `boxpusher-game` (`apps/boxpusher/game`).

mod common;

use common::{Rig, staged, temp_dir};
use dphone_apps::engine::Target;
use dphone_apps::testutil::{FakeConfig, FakePbx, services};
use dphone_apps::uishell::UiShell;
use dphone_apps::{AppManager, DrawOp, ManagerConfig};
use dphone_hal::{Hal, Screen};
use dphone_state::{AccountId, Call, CallState, Calls, Direction, Key, Model, Party, Phase, Store, System};
use dphone_ui::{Ui, UiConfig};
use std::path::PathBuf;

const APP: &str = "boxpusher";

fn rig() -> Rig {
    Rig::new(FakeConfig::default())
}

/// The level number and the two counts beside the board.
fn panel(r: &Rig) -> Vec<String> {
    r.ui.texts(APP)
}

/// What the app's two canvases show: the level, and the boxes and the
/// figure.
fn canvases(r: &Rig) -> (String, String) {
    let drawn = r.ui.drawings(APP);
    assert_eq!(drawn.len(), 2);
    (format!("{:?}", drawn[0].1), format!("{:?}", drawn[1].1))
}

fn progress(r: &Rig) -> String {
    std::fs::read_to_string(r.nvdata.join("appdata").join(APP).join("progress")).unwrap_or_default()
}

fn keys(r: &mut Rig, keys: &[Key]) {
    for k in keys {
        r.key(APP, *k);
    }
}

#[test]
fn a_level_from_the_first_step_to_solved() {
    let mut r = rig();
    r.launch(APP);
    assert_eq!(r.ui.top().as_deref(), Some(APP));
    assert_eq!(r.ui.softkeys(APP), ["Undo", "Restart", "Levels", "Exit"]);
    assert_eq!(panel(&r), ["Level 1", "Moves 0", "Pushes 0"]);

    // Two canvases, one over the other: the level, and the boxes and the
    // figure.  The board has room for 11 by 7 squares of 32 pixels.
    let drawn = r.ui.drawings(APP);
    assert_eq!(drawn.len(), 2);
    assert_eq!(format!("{:?}", drawn[0].0), format!("{:?}", drawn[1].0));
    assert_eq!((drawn[0].0.width, drawn[0].0.height), (11 * 32, 7 * 32));
    let (scenery, pieces) = canvases(&r);
    // Level 1 is a corridor of five squares inside sixteen of wall, with
    // one target: a floor for each, bricks on each wall, and the target.
    assert_eq!(drawn[0].1.len(), 5 + 2 * 16 + 1);
    // One box (not on a target: two operations) and the figure (three).
    assert_eq!(drawn[1].1.len(), 2 + 3);
    assert_eq!(r.ui.draw_count(APP), 2);

    // A step draws the boxes and the figure again, and nothing else.
    r.key(APP, Key::Right);
    assert_eq!(panel(&r), ["Level 1", "Moves 1", "Pushes 0"]);
    assert_eq!(r.ui.draw_count(APP), 3);
    assert_eq!(canvases(&r).0, scenery);
    assert_ne!(canvases(&r).1, pieces);

    // A wall: nothing happens, and nothing is drawn.
    keys(&mut r, &[Key::Up, Key::Down, Key::Digit(2), Key::Digit(8)]);
    assert_eq!(panel(&r), ["Level 1", "Moves 1", "Pushes 0"]);
    assert_eq!(r.ui.draw_count(APP), 3);

    // 4 and 6 are Left and Right.
    r.key(APP, Key::Digit(4));
    assert_eq!(panel(&r), ["Level 1", "Moves 2", "Pushes 0"]);
    assert_eq!(canvases(&r).1, pieces, "back where it started");
    keys(&mut r, &[Key::Digit(6), Key::Digit(6)]);
    assert_eq!(panel(&r), ["Level 1", "Moves 4", "Pushes 1"]);

    // Undo takes the push back; Restart takes everything back.
    r.softkey(APP, "Undo");
    assert_eq!(panel(&r), ["Level 1", "Moves 3", "Pushes 0"]);
    r.key(APP, Key::Right);
    assert_eq!(panel(&r), ["Level 1", "Moves 4", "Pushes 1"]);
    r.softkey(APP, "Restart");
    assert_eq!(panel(&r), ["Level 1", "Moves 0", "Pushes 0"]);
    assert_eq!(canvases(&r).1, pieces);
    r.softkey(APP, "Undo");
    assert_eq!(panel(&r), ["Level 1", "Moves 0", "Pushes 0"]);

    // Three steps to the right solve the level.
    keys(&mut r, &[Key::Right, Key::Right, Key::Right]);
    assert_eq!(
        panel(&r),
        ["Level 1 solved!", "Moves: 3", "Pushes: 2", "Fewest pushes: 2"]
    );
    assert_eq!(r.ui.softkeys(APP), ["Next", "Again", "Levels", "Exit"]);
    assert!(r.ui.drawings(APP).is_empty());
    assert_eq!(progress(&r), "level=1\nsolved=1\nmoves=\n");
    // The arrows do nothing here.
    r.key(APP, Key::Right);
    assert_eq!(panel(&r)[0], "Level 1 solved!");

    r.softkey(APP, "Again");
    assert_eq!(panel(&r), ["Level 1", "Moves 0", "Pushes 0"]);
    keys(&mut r, &[Key::Right, Key::Right, Key::Right]);
    r.softkey(APP, "Next");
    assert_eq!(panel(&r), ["Level 2", "Moves 0", "Pushes 0"]);
    // Level 1 again, and Select on its solved screen goes on too.
    r.softkey(APP, "Levels");
    r.softkey(APP, "Play");
    assert_eq!(panel(&r), ["Level 2", "Moves 0", "Pushes 0"]);
    r.softkey(APP, "Levels");
    r.key(APP, Key::Up);
    r.key(APP, Key::Select);
    assert_eq!(panel(&r), ["Level 1", "Moves 0", "Pushes 0"]);
    keys(&mut r, &[Key::Right, Key::Right, Key::Right, Key::Select]);
    assert_eq!(panel(&r), ["Level 2", "Moves 0", "Pushes 0"]);

    r.softkey(APP, "Exit");
    assert_eq!(r.ui.top(), None);
    assert!(!r.mgr.is_running(APP), "Exit stops the app");
}

#[test]
fn the_levels_open_one_by_one() {
    let mut r = rig();
    r.launch(APP);
    r.softkey(APP, "Levels");
    assert_eq!(r.ui.softkeys(APP), ["Play", "Back"]);
    assert_eq!(r.ui.items(APP), [("Level 1".to_string(), String::new())]);
    // Back, by the softkey or by Cancel, is the game as it was.
    r.softkey(APP, "Back");
    r.key(APP, Key::Right);
    r.softkey(APP, "Levels");
    r.key(APP, Key::Cancel);
    assert_eq!(panel(&r), ["Level 1", "Moves 1", "Pushes 0"]);

    keys(&mut r, &[Key::Right, Key::Right]);
    assert_eq!(panel(&r)[0], "Level 1 solved!");
    r.softkey(APP, "Levels");
    assert_eq!(
        r.ui.items(APP),
        [
            ("Level 1".to_string(), "Solved".to_string()),
            ("Level 2".to_string(), String::new())
        ]
    );
    // From the solved screen, Back goes on to the next level.
    r.softkey(APP, "Back");
    assert_eq!(panel(&r), ["Level 2", "Moves 0", "Pushes 0"]);
    // Level 2: up, right, up, right, down.
    keys(&mut r, &[Key::Up, Key::Right, Key::Up, Key::Right, Key::Down]);
    assert_eq!(
        panel(&r),
        ["Level 2 solved!", "Moves: 5", "Pushes: 2", "Fewest pushes: 2"]
    );
    r.softkey(APP, "Levels");
    assert_eq!(r.ui.items(APP).len(), 3);
    assert_eq!(progress(&r), "level=2\nsolved=2\nmoves=\n");
}

#[test]
fn progress_is_kept_and_a_call_does_not_end_the_game() {
    let mut r = rig();
    r.launch(APP);
    keys(&mut r, &[Key::Right, Key::Right, Key::Right]);
    r.softkey(APP, "Next");
    keys(&mut r, &[Key::Up, Key::Right]);
    assert_eq!(panel(&r), ["Level 2", "Moves 2", "Pushes 1"]);
    let pieces = canvases(&r).1;

    // A call's screen goes over the game.  The steps so far are written
    // down, and the game is there when the call is over.
    r.ui.cover(APP, true);
    r.settle();
    assert!(r.mgr.is_running(APP));
    assert_eq!(progress(&r), "level=1\nsolved=1\nmoves=ur\n");
    r.ui.cover(APP, false);
    r.settle();
    assert_eq!(panel(&r), ["Level 2", "Moves 2", "Pushes 1"]);
    assert_eq!(canvases(&r).1, pieces);
    r.key(APP, Key::Up);
    assert_eq!(panel(&r), ["Level 2", "Moves 3", "Pushes 1"]);

    // Leaving with Cancel stops the app; the next time it is where it
    // was, and the steps can still be undone.
    r.key(APP, Key::Cancel);
    assert_eq!(r.ui.top(), None);
    assert!(!r.mgr.is_running(APP));
    assert_eq!(progress(&r), "level=1\nsolved=1\nmoves=uru\n");
    r.launch(APP);
    assert_eq!(panel(&r), ["Level 2", "Moves 3", "Pushes 1"]);
    r.softkey(APP, "Undo");
    assert_eq!(canvases(&r).1, pieces);
    r.softkey(APP, "Exit");

    // A progress file that makes no sense is the beginning.
    let file = r.nvdata.join("appdata").join(APP).join("progress");
    for nonsense in ["", "level=99\nsolved=3\nmoves=zz\n", "solved=x\nlevel=5\n", "\u{0}\u{1}"] {
        std::fs::write(&file, nonsense).unwrap();
        r.launch(APP);
        let level = if nonsense.contains("solved=3") { "Level 4" } else { "Level 1" };
        assert_eq!(panel(&r), [level, "Moves 0", "Pushes 0"], "{nonsense:?}");
        r.softkey(APP, "Exit");
    }
    // The last level solved: there is no next one.
    std::fs::write(&file, "level=27\nsolved=28\nmoves=\n").unwrap();
    r.launch(APP);
    assert_eq!(panel(&r)[0], "Level 28");
    r.softkey(APP, "Levels");
    assert_eq!(r.ui.items(APP).len(), 28);
}

#[test]
fn it_plays_on_the_gray_screens_in_black_and_white() {
    for (model, tile) in [(Model::D40, 14), (Model::D70, 20)] {
        let mut r = Rig::with_root(staged(), FakeConfig::default(), model);
        r.launch(APP);
        let drawn = r.ui.drawings(APP);
        assert_eq!((drawn[0].0.width, drawn[0].0.height), (11 * tile, 7 * tile), "{model:?}");
        // The fake screen's foreground and background colors, and no
        // others; no floor is painted.
        let fg = dphone_apps::Color { r: 32, g: 32, b: 32 };
        let bg = dphone_apps::Color { r: 16, g: 16, b: 16 };
        assert_eq!(drawn[0].1.len(), 2 * 16 + 1);
        for (_, ops) in &drawn {
            for op in ops {
                let (color, size) = match op {
                    DrawOp::Fill((rect, color)) => (*color, (rect.width, rect.height)),
                    DrawOp::Bitmap(b) => (b.color, (b.width, b.height)),
                    DrawOp::Text(t) => panic!("no text in this game: {t:?}"),
                };
                assert!(
                    (color.r, color.g, color.b) == (fg.r, fg.g, fg.b) || (color.r, color.g, color.b) == (bg.r, bg.g, bg.b),
                    "{model:?}"
                );
                assert!(size.0 <= tile && size.1 <= tile);
            }
        }
        keys(&mut r, &[Key::Right, Key::Right, Key::Right]);
        assert_eq!(panel(&r)[0], "Level 1 solved!", "{model:?}");
        r.softkey(APP, "Exit");
    }
}

#[test]
fn it_is_a_built_in_app_that_can_be_switched_off() {
    assert_eq!(
        common::capabilities(APP),
        ["ui", "storage", "canvas"],
        "no sound, no calls, no network, no timers"
    );
    let config = FakeConfig::default();
    let disabled = config.disabled.clone();
    let mut r = Rig::new(config);
    let listed = r.store.get::<dphone_state::Apps>();
    let entry = listed.installed.iter().find(|a| a.name == APP).expect("listed");
    assert_eq!((entry.display_name.as_str(), entry.icon.as_str()), ("Box Pusher", "boxpusher"));
    assert!(entry.builtin);
    r.launch(APP);
    *disabled.lock().unwrap() = vec![APP.into()];
    r.store.emit(dphone_state::Event::ConfigApplied { generation: 2 });
    r.settle();
    assert!(!r.mgr.is_running(APP));
    assert!(r.mgr.launch(APP).is_err());
    let apps = r.store.get::<dphone_state::Apps>();
    assert!(apps.installed.iter().all(|a| a.name != APP));
    assert!(apps.disabled.iter().any(|a| a.name == APP));
}

// --------------------------------------------------- with the phone UI

/// A simulated phone: the app under the manager, drawn by the phone UI on
/// the simulator's display, with keys from the simulator's keypad.
struct Phone {
    model: Model,
    store: Store,
    ui: Ui,
    sim: dphone_hal::sim::SimHandle,
    mgr: AppManager,
    nvdata: PathBuf,
    _dir: common::TempDir,
}

impl Phone {
    /// A phone on which the game has this progress file.
    fn new(model: Model, progress: &str) -> Phone {
        let dir = temp_dir("boxpusher-phone");
        let nvdata = dir.0.join("nvdata");
        std::fs::create_dir_all(nvdata.join("appdata").join(APP)).unwrap();
        std::fs::write(nvdata.join("appdata").join(APP).join("progress"), progress).unwrap();
        let mut hal = Hal::sim(model);
        let sim = hal.sim.clone().unwrap();
        let store = Store::new(model);
        let ui = Ui::from_hal(&mut hal, store.clone(), UiConfig::default());
        store.update::<System, _>(|s| {
            s.phase = Phase::Running;
            s.time_synced = true;
        });
        let mut cfg = ManagerConfig::rooted(staged(), &nvdata);
        cfg.target = Target::Pulley64;
        let mut mgr = AppManager::new(
            cfg,
            store.clone(),
            Box::new(UiShell::new(ui.handle(), model)),
            services(&FakePbx::default(), FakeConfig::default()),
        )
        .expect("engine");
        mgr.scan();
        let mut phone = Phone {
            model,
            store,
            ui,
            sim,
            mgr,
            nvdata,
            _dir: dir,
        };
        phone.settle();
        phone.mgr.launch(APP).unwrap();
        phone.settle();
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

    /// Presses keys; returns how many times the display was sent
    /// something.
    fn tap(&mut self, keys: &[Key]) -> u64 {
        let before = self.sim.frame_generation();
        for key in keys {
            self.sim.tap(*key);
            self.settle();
        }
        self.sim.frame_generation() - before
    }

    /// Plays recorded steps (`u`, `d`, `l`, `r`).
    fn play(&mut self, steps: &str) {
        for c in steps.chars() {
            let key = match c {
                'u' => Key::Up,
                'd' => Key::Down,
                'l' => Key::Left,
                _ => Key::Right,
            };
            self.tap(&[key]);
        }
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

    fn count(&self, color: u32) -> usize {
        self.pixels().0.iter().filter(|p| **p == color).count()
    }

    /// The smallest rectangle that holds every pixel that differs from
    /// `before`: (width, height).
    fn changed(&self, before: &[u32]) -> (usize, usize) {
        let (now, width) = self.pixels();
        let differ: Vec<usize> = (0..now.len()).filter(|i| now[*i] != before[*i]).collect();
        if differ.is_empty() {
            return (0, 0);
        }
        let xs = differ.iter().map(|i| i % width);
        let (left, right) = (xs.clone().min().unwrap(), xs.max().unwrap());
        (right - left + 1, differ[differ.len() - 1] / width - differ[0] / width + 1)
    }

    /// Writes the frame as a PNG (the gray ones doubled in size).
    fn shot(&self, name: &str) {
        let dir = match std::env::var_os("BOX_PUSHER_PREVIEWS") {
            Some(d) => PathBuf::from(d),
            None => PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("box-pusher"),
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

/// Level 14's solution, from `apps/boxpusher/game/tests/levels.rs`.
const LEVEL_14: &str = "drurrdrulllluurdldrrrdrrulllluurrrdullldddlurrrdrruluurd";

/// A first level, a level in the middle, that level nearly solved, its
/// solved screen, and the list of levels, with a picture of each.  Returns the phone at the first
/// level, the phone at level 14 after one more step, and what the display
/// showed before that step.
fn pictures(model: Model) -> (Phone, Phone, Vec<u32>) {
    let mut first = Phone::new(model, "");
    first.shot("level-1");

    let mut middle = Phone::new(model, "level=13\nsolved=13\nmoves=\n");
    middle.shot("level-14");
    // The same level one push from solved, as the game finds it when it
    // was left there.
    let mut solved = Phone::new(model, &format!("level=13\nsolved=13\nmoves={}\n", &LEVEL_14[..LEVEL_14.len() - 1]));
    solved.shot("level-14-played");
    solved.play(&LEVEL_14[LEVEL_14.len() - 1..]);
    solved.shot("level-14-solved");
    solved.tap(&[Key::Soft(3)]);
    solved.shot("levels");
    assert_eq!(
        std::fs::read_to_string(solved.nvdata.join("appdata").join(APP).join("progress")).unwrap(),
        "level=14\nsolved=14\nmoves=\n"
    );

    // One step of the figure, with no box: the display is sent the two
    // squares and the count beside the board, once.
    middle.play("u");
    let before = middle.pixels().0;
    assert_eq!(middle.tap(&[Key::Digit(6)]), 1, "one step, one update of the display");
    // A step into a wall: the display is sent nothing.
    let mut wall = Phone::new(model, "");
    assert_eq!(wall.tap(&[Key::Up, Key::Digit(2), Key::Left]), 0);
    assert_eq!(first.tap(&[Key::Right]), 1);
    (first, middle, before)
}

#[test]
fn drawn_on_a_color_phone() {
    let (first, middle, before) = pictures(Model::D60);
    // The colors of the pictures are on the display: wall, floor, box,
    // target, and the figure's body.
    for color in [0x5F6E8C, 0xD9D0BA, 0xE09A38, 0xC63A32, 0x1F6FD0] {
        assert!(first.count(color) > 20, "{color:06X}");
    }
    // No box of level 1 is on a target yet.
    assert_eq!(first.count(0x3EA655), 0);
    // The step changed two squares side by side (32 pixels each) and the
    // count of moves, which is on the same lines of the display or not;
    // whatever else is on the display stayed.
    let (width, height) = middle.changed(&before);
    assert!(width >= 64 && height >= 32, "{width}x{height}");
    let (pixels, w) = middle.pixels();
    let differ = (0..pixels.len()).filter(|i| pixels[*i] != before[*i]).count();
    assert!(differ < 2 * 32 * 32 + 40 * 25, "{differ} pixels of {}x{}", w, pixels.len() / w);

    // A call comes in: its screen takes the display, and the keys are
    // the call's.  The game waits, with its steps written down, and is
    // on the display again, as it was, when the call is over.
    let mut p = middle;
    let game = p.pixels().0;
    assert_eq!(p.ui.screen_name(), "app:boxpusher");
    let id = p.store.next_call_id();
    p.store.update::<Calls, _>(|c| {
        let from = Party {
            number: "1002".into(),
            ..Default::default()
        };
        let mut call = Call::new(id, AccountId(0), Direction::Incoming, from);
        call.state = CallState::Ringing;
        c.calls.push(call);
        c.focused = Some(id);
    });
    p.settle();
    assert_eq!(p.ui.screen_name(), "call");
    assert_ne!(p.pixels().0, game);
    let file = p.nvdata.join("appdata").join(APP).join("progress");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "level=13\nsolved=13\nmoves=ur\n");
    p.tap(&[Key::Left, Key::Digit(4)]);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "level=13\nsolved=13\nmoves=ur\n");
    p.store.update::<Calls, _>(|c| {
        c.calls.clear();
        c.focused = None;
    });
    p.settle();
    assert_eq!(p.ui.screen_name(), "app:boxpusher");
    assert!(p.mgr.is_running(APP));
    assert_eq!(p.pixels().0, game);
    assert_eq!(p.tap(&[Key::Left]), 1, "the game goes on");
}

#[test]
fn drawn_on_the_gray_phones() {
    for model in [Model::D40, Model::D70] {
        let (first, middle, before) = pictures(model);
        // Black and white, and nothing in between, on the board: the
        // left 11 squares of the app's part of the display.
        let (pixels, w) = first.pixels();
        let grays: std::collections::BTreeSet<u32> = pixels.iter().copied().collect();
        assert!(grays.contains(&0x000000) && grays.contains(&0xFFFFFF), "{model:?}: {grays:?}");
        let (width, height) = middle.changed(&before);
        let tile = if model == Model::D40 { 14 } else { 20 };
        assert!(width >= 2 * tile && height >= tile, "{model:?}: {width}x{height}");
        let now = middle.pixels().0;
        let differ = (0..now.len()).filter(|i| now[*i] != before[*i]).count();
        assert!(differ < 2 * tile * tile + 300, "{model:?}: {differ} pixels of {}x{}", w, now.len() / w);
    }
}
