//! Light Up, the puzzle: the app's life under the manager with the fake
//! screen (the keys, the screens, generation in turns, what is kept, the
//! time spent, and the tables of the best times with two phones of one
//! PBX, with a PBX out of reach, and with a write whose answer does not
//! come), and the real thing drawn by the phone UI on a simulated D60
//! (color), D40, and D70 (gray).
//!
//! The pictures are written as PNG files under `light-up/` in cargo's
//! test scratch directory (`CARGO_TARGET_TMPDIR`), or in the directory
//! `LIGHT_UP_PREVIEWS` names.  The rules, the solver, the generator, and
//! the pictures themselves are tested in the crate `lightup-game` (`apps/lightup/game`).
//!
//! `cargo test --release -p dphone-apps --test lightup -- --ignored
//! --nocapture generation_times` prints how long the app takes to make
//! puzzles under the interpreter, beside the same work done natively.

mod common;

use common::{Rig, staged, temp_dir};
use dphone_apps::engine::Target;
use dphone_apps::testutil::{FakeConfig, FakePbx, FakeStore, local_value, services_with_store};
use dphone_apps::uishell::UiShell;
use dphone_apps::{AppManager, DrawOp, ManagerConfig, StoreScope};
use dphone_hal::{Hal, Screen};
use dphone_state::{
    AccountId, Call, CallState, Calls, Direction, Key, Model, Party, Phase, Store, System,
};
use dphone_ui::{Ui, UiConfig};
use lightup_game::draw::{FontSize, Fonts, Layout, Op, Painter, Palette, Rgb, TextFont};
use lightup_game::generate::{Difficulty, Generator, Params, Progress, SIZES, generate};
use lightup_game::scores::{Kind, Table, Win};
use lightup_game::solver::{Outcome, Solve, Techniques};
use lightup_game::{Act, Game, Grid};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

const APP: &str = "lightup";
/// The grids the app allows a puzzle (`MAX_GRIDS` in the app).
const MAX_GRIDS: u32 = 400;
/// The texts beside the board while a puzzle is played, after the time
/// spent ([`beside`]): the two lines that say "Solved!" and the time when
/// it comes to that, and nothing else (the keys are a picture).
const PLAYING: [&str; 2] = ["", ""];
const SOFTKEYS: [&str; 4] = ["Undo", "New", "Help", "Scores"];

fn storage(nvdata: &Path) -> PathBuf {
    nvdata.join("appdata").join(APP)
}

/// The value of one of the app's keys on the phone, or "".
fn local(nvdata: &Path, key: &str) -> String {
    local_value(&storage(nvdata), key).map(|v| String::from_utf8(v).unwrap()).unwrap_or_default()
}

/// A rig whose app makes its puzzles from this seed on.
fn rig(model: Model, seed: u64) -> Rig {
    listen();
    // The time the phone gives an app for an event, not the shorter time
    // of the other tests: a turn of making a puzzle is 50 ms of work, and
    // a busy build machine can hold a test up for longer than the rest.
    let r = Rig::with_budget(
        staged(),
        FakeConfig::default(),
        model,
        Duration::from_secs(2),
    );
    std::fs::create_dir_all(storage(&r.nvdata)).unwrap();
    std::fs::write(storage(&r.nvdata).join("seed"), seed.to_string()).unwrap();
    r
}

/// A rig on which the app finds this puzzle file.
fn rig_with(model: Model, puzzle: &str) -> Rig {
    let r = rig(model, 1);
    std::fs::write(storage(&r.nvdata).join("puzzle"), puzzle).unwrap();
    r
}

fn file(r: &Rig) -> String {
    local(&r.nvdata, "puzzle")
}

/// The grid and the cursor of a puzzle file.
fn parse(file: &str) -> (Grid, (usize, usize)) {
    let rows: Vec<&str> = file.lines().filter(|l| !l.contains('=')).collect();
    let cursor = file
        .lines()
        .find_map(|l| l.strip_prefix("cursor="))
        .expect("a cursor");
    let (x, y) = cursor.split_once(',').unwrap();
    (
        Grid::parse(&rows.join("\n")).expect("a grid"),
        (x.parse().unwrap(), y.parse().unwrap()),
    )
}

/// What the app has: something else takes the screen for a moment, which
/// makes the app write its puzzle down.
fn state(r: &mut Rig) -> (Grid, (usize, usize)) {
    r.ui.cover(APP, true);
    r.settle();
    r.ui.cover(APP, false);
    r.settle();
    parse(&file(r))
}

/// Whether a text is a time (`3:05`).
fn is_time(text: &str) -> bool {
    text.contains(':') && text.chars().all(|c| c.is_ascii_digit() || c == ':')
}

/// The time spent, as the playing screen shows it.
fn time_shown(r: &Rig) -> String {
    let texts = r.ui.texts(APP);
    assert!(texts.first().is_some_and(|t| is_time(t)), "{texts:?}");
    texts[0].clone()
}

/// The texts of the playing screen after the time spent, which is the
/// first of them and goes on.
fn beside(r: &Rig) -> Vec<String> {
    time_shown(r);
    r.ui.texts(APP).split_off(1)
}

fn generating(r: &Rig) -> bool {
    r.ui.texts(APP).iter().any(|t| t.starts_with("Generating"))
}

/// Lets the app work until its puzzle is made.
fn wait(r: &mut Rig) {
    let start = Instant::now();
    while generating(r) {
        assert!(
            start.elapsed() < Duration::from_secs(300),
            "still generating"
        );
        assert!(r.mgr.is_running(APP), "the app was stopped");
        std::thread::sleep(Duration::from_millis(1));
        r.mgr.pump();
    }
    r.settle();
}

/// Starts the puzzle of a row of the list of sizes and difficulties.
fn choose(r: &mut Rig, side: usize, difficulty: Difficulty) {
    let row = SIZES.iter().position(|s| *s == side).unwrap() * 3 + difficulty as usize;
    r.ui.highlight(APP, row as u32);
    r.softkey(APP, "Play");
}

fn native(side: usize, difficulty: Difficulty, seed: u64) -> Grid {
    generate(Params::preset(side, difficulty), seed, MAX_GRIDS).expect("a puzzle")
}

fn solution(puzzle: &Grid) -> Grid {
    let mut s = Solve::new(puzzle, Techniques::HARD);
    assert_eq!(s.run(), Outcome::Unique);
    s.solution().unwrap().clone()
}

/// The keys that take the cursor from one cell to another.
fn path(from: (usize, usize), to: (usize, usize), digits: bool) -> Vec<Key> {
    let mut keys = Vec::new();
    let (h, v) = (to.0 as i32 - from.0 as i32, to.1 as i32 - from.1 as i32);
    for _ in 0..h.abs() {
        keys.push(match (h > 0, digits) {
            (true, false) => Key::Right,
            (true, true) => Key::Digit(6),
            (false, false) => Key::Left,
            (false, true) => Key::Digit(4),
        });
    }
    for _ in 0..v.abs() {
        keys.push(match (v > 0, digits) {
            (true, false) => Key::Down,
            (true, true) => Key::Digit(5),
            (false, false) => Key::Up,
            (false, true) => Key::Digit(2),
        });
    }
    keys
}

fn keys(r: &mut Rig, keys: &[Key]) {
    for k in keys {
        r.key(APP, *k);
    }
}

/// The app's drawings as the game crate's operations.
fn drawn(r: &Rig) -> Vec<Vec<Op>> {
    r.ui.drawings(APP)
        .into_iter()
        .map(|(_, ops)| {
            ops.into_iter()
                .map(|op| match op {
                    DrawOp::Fill((rect, c)) => Op::Fill {
                        x: rect.x,
                        y: rect.y,
                        width: rect.width,
                        height: rect.height,
                        color: Rgb(c.r, c.g, c.b),
                    },
                    DrawOp::Bitmap(b) => Op::Bitmap {
                        x: b.x,
                        y: b.y,
                        width: b.width,
                        height: b.height,
                        scale: b.scale,
                        bits: b.bits.into(),
                        color: Rgb(b.color.r, b.color.g, b.color.b),
                    },
                    DrawOp::Text(t) => Op::Text {
                        x: t.box_.x,
                        y: t.box_.y,
                        width: t.box_.width,
                        height: t.box_.height,
                        font: match t.font {
                            dphone_apps::Font::Small => TextFont::Small,
                            dphone_apps::Font::Normal => TextFont::Normal,
                            dphone_apps::Font::Bold => TextFont::Bold,
                            dphone_apps::Font::Title => TextFont::Title,
                            dphone_apps::Font::Large => TextFont::Large,
                        },
                        text: {
                            assert_eq!(t.text.chars().count(), 1, "a digit");
                            t.text.chars().next().unwrap()
                        },
                        color: Rgb(t.color.r, t.color.g, t.color.b),
                    },
                })
                .collect()
        })
        .collect()
}

/// The palette of the app on a model's display under the fake screen:
/// its foreground color is (32, 32, 32), its background (16, 16, 16),
/// and a digit is 7 by 9 pixels in its bold font, 8 by 10 in the title
/// font, 12 by 16 in the large one, 6 by 8 in the normal one, and 5 by 6
/// in the small one.
fn palette(model: Model) -> Palette {
    let mut palette = if model.is_color() {
        Palette::color()
    } else {
        Palette::gray(Rgb(32, 32, 32), Rgb(16, 16, 16))
    };
    let font = |font, width, height| FontSize {
        font,
        width,
        height,
    };
    palette.fonts = Fonts::new(&[
        font(TextFont::Bold, 7, 9),
        font(TextFont::Title, 8, 10),
        font(TextFont::Large, 12, 16),
        font(TextFont::Normal, 6, 8),
        font(TextFont::Small, 5, 6),
    ]);
    palette
}

/// What the app should have drawn for a game on a model's display: a
/// list per row, and the cursor's.  The fake screen's foreground color is
/// (32, 32, 32) and its background (16, 16, 16).
fn expected(model: Model, game: &Game) -> Vec<Vec<Op>> {
    let area = model.app_content_area();
    let side = game.grid().width();
    let palette = palette(model);
    let painter = Painter::new(
        side,
        Layout::new(area.width, area.height, side).pitch,
        palette,
    );
    let mut lists: Vec<Vec<Op>> = (0..side).map(|row| painter.row(game.grid(), row)).collect();
    lists.push(painter.cursor(game));
    // The picture of the keys, until the puzzle is solved.
    if !game.solved() {
        let l = Layout::new(area.width, area.height, side);
        let line = if model.is_color() {
            25
        } else if area.height > 102 {
            17
        } else {
            15
        };
        // The fake screen's foreground color.
        let ink = Rgb(32, 32, 32);
        lists.push(
            lightup_game::draw::keymap(
                l.panel_width - 2 * (line / 2),
                // Under the line of the time spent.
                area.height - 4 - line,
                palette,
                ink,
            )
            .ops,
        );
    }
    lists
}

#[test]
fn a_puzzle_from_new_to_solved_with_every_key() {
    let mut r = rig(Model::D60, 11);
    r.launch(APP);
    assert_eq!(r.ui.top().as_deref(), Some(APP));
    // Nothing is kept yet: the sizes and difficulties come first.
    assert_eq!(r.ui.texts(APP), ["New puzzle"]);
    assert_eq!(r.ui.softkeys(APP), ["Play", "Help", "Exit", "Scores"]);
    let names: Vec<String> = r.ui.items(APP).into_iter().map(|i| i.0).collect();
    assert_eq!(
        names,
        [
            "7x7 Easy",
            "7x7 Medium",
            "7x7 Hard",
            "10x10 Easy",
            "10x10 Medium",
            "10x10 Hard",
            "14x14 Easy",
            "14x14 Medium",
            "14x14 Hard"
        ]
    );
    r.softkey(APP, "Play");
    wait(&mut r);
    assert_eq!(beside(&r), PLAYING);
    assert_eq!(r.ui.softkeys(APP), SOFTKEYS);

    // The puzzle is the one the seed gives, and it is written down.
    let puzzle = native(7, Difficulty::Easy, 11);
    assert_eq!(parse(&file(&r)), (puzzle.clone(), (0, 0)));
    assert!(
        file(&r).starts_with("size=7\ndifficulty=easy\nelapsed=0\ncursor=0,0\nundo=\n"),
        "{}",
        file(&r)
    );
    // A canvas for each row and one for the cursor, each drawn once.
    let mut game = Game::new(puzzle.clone());
    assert_eq!(drawn(&r), expected(Model::D60, &game));
    assert_eq!(r.ui.draw_count(APP), 9);
    let bounds: Vec<_> = r.ui.drawings(APP).into_iter().map(|d| d.0).collect();
    assert_eq!(
        (bounds[0].width, bounds[0].height, bounds[1].y - bounds[0].y),
        (239, 34, 34)
    );
    assert_eq!(
        (bounds[7].x, bounds[7].width, bounds[7].height),
        (bounds[0].x, 239, 239)
    );

    // The arrows and 2, 4, 5, 6 move the cursor; only the cursor is
    // drawn again.  At an edge nothing happens and nothing is drawn.
    let mut draws = 9;
    for (key, dx, dy) in [
        (Key::Right, 1, 0),
        (Key::Digit(6), 1, 0),
        (Key::Down, 0, 1),
        (Key::Digit(5), 0, 1),
        (Key::Left, -1, 0),
        (Key::Digit(4), -1, 0),
        (Key::Left, -1, 0),
        (Key::Digit(4), -1, 0),
        (Key::Up, 0, -1),
        (Key::Digit(2), 0, -1),
        (Key::Up, 0, -1),
        (Key::Digit(2), 0, -1),
    ] {
        r.key(APP, key);
        draws += game.move_cursor(dx, dy) as u32;
        assert_eq!(r.ui.draw_count(APP), draws, "{key:?}");
        assert_eq!(
            state(&mut r),
            (game.grid().clone(), game.cursor()),
            "{key:?}"
        );
        assert_eq!(drawn(&r), expected(Model::D60, &game), "{key:?}");
    }
    assert_eq!(draws, 9 + 8);
    assert_eq!(game.cursor(), (0, 0));

    // 1 and Select put a light, or take it off a cell that has one; 3
    // puts a no-light mark, or takes it off; each puts its own in place
    // of the other's.  7, 9, 0, star, and pound do nothing.  On every
    // cell of the first two rows, in turn:
    // black cells take nothing.
    let white: Vec<usize> = (0..puzzle.cells())
        .filter(|i| !puzzle.is_black(*i))
        .collect();
    let black: Vec<usize> = (0..puzzle.cells())
        .filter(|i| puzzle.is_black(*i))
        .collect();
    let at = |r: &mut Rig, game: &mut Game, i: usize| {
        let to = (i % 7, i / 7);
        keys(r, &path(game.cursor(), to, i.is_multiple_of(2)));
        game.set_cursor(to.0, to.1);
    };
    let acts = [
        (Key::Digit(1), Some(Act::Light)),
        (Key::Digit(1), Some(Act::Light)),
        (Key::Digit(3), Some(Act::Mark)),
        (Key::Digit(3), Some(Act::Mark)),
        (Key::Select, Some(Act::Light)),
        (Key::Digit(3), Some(Act::Mark)),
        (Key::Digit(1), Some(Act::Light)),
        (Key::Select, Some(Act::Light)),
        (Key::Digit(7), None),
        (Key::Digit(9), None),
        (Key::Digit(0), None),
        (Key::Star, None),
        (Key::Pound, None),
        (Key::Select, Some(Act::Light)),
    ];
    for &cell in [white[0], black[0], white[1], black[1]].iter() {
        at(&mut r, &mut game, cell);
        for (key, act) in acts {
            let before = (r.ui.draw_count(APP), game.clone());
            r.key(APP, key);
            let changed = act.is_some_and(|a| game.toggle(a));
            assert_eq!(changed, game != before.1);
            assert_eq!(
                state(&mut r),
                (game.grid().clone(), game.cursor()),
                "{key:?} at {cell}"
            );
            assert_eq!(drawn(&r), expected(Model::D60, &game), "{key:?} at {cell}");
            // Only the rows that changed were drawn again, and not the
            // cursor.
            let rows = expected(Model::D60, &game)
                .iter()
                .zip(expected(Model::D60, &before.1))
                .filter(|(now, was)| *now != was)
                .count() as u32;
            assert_eq!(r.ui.draw_count(APP) - before.0, rows, "{key:?} at {cell}");
            assert_eq!(rows > 0, changed);
            assert!(rows <= 7);
        }
        assert_eq!(puzzle.is_black(cell), !game.grid().has_light(cell));
    }
    assert!(!game.solved());

    // Now the solution: the lights on the first two white cells go if
    // they are wrong, and every light of the solution is put, with 1 and
    // with Select in turn.
    let solved = solution(&puzzle);
    for cell in [white[0], white[1]] {
        if !solved.has_light(cell) {
            at(&mut r, &mut game, cell);
            r.key(APP, Key::Digit(1));
            game.act(Act::Clear);
        }
    }
    let lights: Vec<usize> = (0..puzzle.cells())
        .filter(|i| solved.has_light(*i) && !game.grid().has_light(*i))
        .collect();
    for (n, cell) in lights.iter().enumerate() {
        assert_eq!(beside(&r), PLAYING);
        at(&mut r, &mut game, *cell);
        r.key(
            APP,
            if n.is_multiple_of(2) {
                Key::Digit(1)
            } else {
                Key::Select
            },
        );
        game.act(Act::Light);
    }
    assert!(game.solved());
    // Solved: said so, with the time taken; the solved puzzle is written
    // down at once, and the cursor is gone.
    let texts = r.ui.texts(APP);
    assert_eq!(texts.len(), 2);
    assert_eq!(texts[0], "Solved!");
    let seconds: u32 = texts[1]
        .strip_prefix("Time 0:")
        .expect(&texts[1])
        .parse()
        .unwrap();
    assert!(seconds < 60);
    assert_eq!(parse(&file(&r)).0, *game.grid());
    assert!(game.grid().is_solved());
    assert_eq!(drawn(&r), expected(Model::D60, &game));
    assert!(drawn(&r)[7].is_empty());
    assert_eq!(drawn(&r).len(), 8);
    assert_eq!(r.ui.softkeys(APP), ["New", "Help", "Exit", "Scores"]);
    // The keys do nothing any more.
    let draws = r.ui.draw_count(APP);
    keys(
        &mut r,
        &[
            Key::Left,
            Key::Digit(1),
            Key::Digit(3),
            Key::Digit(5),
            Key::Select,
            Key::Digit(8),
        ],
    );
    assert_eq!(r.ui.draw_count(APP), draws);
    assert_eq!(parse(&file(&r)).0, *game.grid());
    assert_eq!(r.ui.texts(APP), texts);

    // New: the list again, at the size and difficulty played, and Back
    // (the softkey or Cancel) is the solved puzzle as it was.
    r.softkey(APP, "New");
    assert_eq!(r.ui.softkeys(APP), ["Play", "Help", "Back", "Scores"]);
    r.softkey(APP, "Back");
    assert_eq!(r.ui.texts(APP), texts);
    r.softkey(APP, "New");
    r.key(APP, Key::Cancel);
    assert_eq!(r.ui.texts(APP), texts);
    assert_eq!(drawn(&r), expected(Model::D60, &game));
    // Another puzzle: the next seed, a different one, chosen with Select
    // on its row.
    r.softkey(APP, "New");
    r.key(APP, Key::Down);
    r.key(APP, Key::Select);
    wait(&mut r);
    assert_eq!(beside(&r), PLAYING);
    assert_eq!(
        parse(&file(&r)),
        (native(7, Difficulty::Medium, 12), (0, 0))
    );

    r.key(APP, Key::Cancel);
    assert_eq!(r.ui.top(), None);
    assert!(!r.mgr.is_running(APP), "Cancel stops the app");
}

/// A 7x7 puzzle with lights that see each other and a 1 with two lights.
const WRONG: &str = "size=7\ndifficulty=hard\nelapsed=125000\ncursor=3,1\n\
    ..#....\n\
    L.xL...\n\
    .L1L...\n\
    ...#...\n\
    ...0...\n\
    #......\n\
    ......2\n";

#[test]
fn errors_are_drawn_as_errors_in_color_and_in_black_and_white() {
    let (grid, cursor) = parse(WRONG);
    assert!(grid.light_conflicts(grid.index(0, 1)) && grid.light_conflicts(grid.index(3, 1)));
    assert!(grid.light_conflicts(grid.index(3, 2)) && !grid.light_conflicts(grid.index(1, 2)));
    assert!(grid.clue_overfull(grid.index(2, 2)) && !grid.clue_overfull(grid.index(3, 4)));
    let mut game = Game::new(grid);
    game.set_cursor(cursor.0, cursor.1);
    for model in [Model::D60, Model::D40, Model::D70] {
        let mut r = rig_with(model, WRONG);
        r.launch(APP);
        // The puzzle that was kept, with no generating.
        assert_eq!(beside(&r), PLAYING, "{model:?}");
        assert_eq!(drawn(&r), expected(model, &game), "{model:?}");
        let lists = drawn(&r);
        let colors = |ops: &[Op]| -> Vec<Rgb> {
            ops.iter()
                .map(|op| match op {
                    Op::Fill { color, .. } | Op::Bitmap { color, .. } | Op::Text { color, .. } => {
                        *color
                    }
                })
                .collect()
        };
        if model.is_color() {
            // Red: the two lights of row 1, and the 1 and a light of
            // row 2; nowhere else.
            let red = Rgb(0xE0, 0x20, 0x20);
            let reds: Vec<usize> = lists
                .iter()
                .map(|ops| colors(ops).iter().filter(|c| **c == red).count())
                .collect();
            assert_eq!(reds, [0, 2, 2, 0, 0, 0, 0, 0, 0]);
        } else {
            // The fake screen's foreground and background, and nothing
            // else.
            for ops in &lists {
                assert!(
                    colors(ops)
                        .iter()
                        .all(|c| *c == Rgb(32, 32, 32) || *c == Rgb(16, 16, 16)),
                    "{model:?}"
                );
            }
        }
        // Taking the wrong lights off takes the errors off: the rows are
        // then drawn like those of the same puzzle without them.
        keys(&mut r, &[Key::Digit(1), Key::Down, Key::Digit(1)]);
        let mut fixed = game.clone();
        fixed.act(Act::Clear);
        fixed.move_cursor(0, 1);
        fixed.act(Act::Clear);
        assert!(
            (0..49).all(|i| !fixed.grid().light_conflicts(i) && !fixed.grid().clue_overfull(i))
        );
        assert_eq!(drawn(&r), expected(model, &fixed), "{model:?}");
        assert_ne!(expected(model, &fixed)[1], expected(model, &game)[1]);
        assert_ne!(expected(model, &fixed)[2], expected(model, &game)[2]);
        r.key(APP, Key::Cancel);
    }
}

#[test]
fn the_puzzle_is_kept_and_a_call_does_not_end_it() {
    let mut r = rig_with(Model::D60, WRONG);
    r.launch(APP);
    keys(&mut r, &[Key::Right, Key::Digit(3)]);
    let shown = drawn(&r);
    let kept = WRONG
        .replace("L.xL...", "L.xLx..")
        .replace("cursor=3,1", "cursor=4,1");
    let rows = |f: &str| {
        f.lines()
            .filter(|l| !l.starts_with("elapsed=") && !l.starts_with("undo="))
            .map(String::from)
            .collect::<Vec<_>>()
    };
    let elapsed = |f: &str| {
        f.lines()
            .find_map(|l| l.strip_prefix("elapsed="))
            .unwrap()
            .parse::<u64>()
            .unwrap()
    };
    // Not written at every key.
    assert_eq!(file(&r), WRONG);

    // A call's screen goes over the puzzle.  The puzzle, the marks, the
    // cursor, and the time so far are written down, and all is there
    // when the call is over.  The time does not run meanwhile.
    r.ui.cover(APP, true);
    r.settle();
    assert!(r.mgr.is_running(APP));
    assert_eq!(rows(&file(&r)), rows(&kept));
    let before = elapsed(&file(&r));
    assert!((125_000..135_000).contains(&before), "{before}");
    std::thread::sleep(Duration::from_millis(1500));
    r.ui.cover(APP, false);
    r.settle();
    assert_eq!(drawn(&r), shown);
    assert_eq!(beside(&r), PLAYING);
    r.ui.cover(APP, true);
    r.settle();
    assert!(
        elapsed(&file(&r)) - before < 1000,
        "the time ran while the puzzle was covered"
    );
    r.ui.cover(APP, false);
    r.settle();
    r.key(APP, Key::Digit(3));
    assert_ne!(drawn(&r), shown);

    // Leaving with Cancel stops the app; the next time the puzzle is
    // where it was.
    r.key(APP, Key::Cancel);
    assert_eq!(r.ui.top(), None);
    assert!(!r.mgr.is_running(APP));
    let left = WRONG.replace("cursor=3,1", "cursor=4,1");
    assert_eq!(rows(&file(&r)), rows(&left));
    r.launch(APP);
    assert_eq!(beside(&r), PLAYING);
    let (grid, cursor) = parse(&left);
    let mut game = Game::new(grid);
    game.set_cursor(cursor.0, cursor.1);
    assert_eq!(drawn(&r), expected(Model::D60, &game));
    r.key(APP, Key::Cancel);

    // A solved puzzle is shown solved, with its time.
    let solved = native(10, Difficulty::Medium, 5);
    let mut done = solution(&solved);
    for i in 0..done.cells() {
        done.set_mark(i, false);
    }
    let puzzle = storage(&r.nvdata).join("puzzle");
    std::fs::write(
        &puzzle,
        format!(
            "size=10\ndifficulty=medium\nelapsed=3725999\ncursor=2,2\n{}",
            done.to_text()
        ),
    )
    .unwrap();
    r.launch(APP);
    assert_eq!(r.ui.texts(APP), ["Solved!", "Time 1:02:05"]);
    assert!(drawn(&r)[10].is_empty());
    r.softkey(APP, "Exit");

    // A puzzle file that makes no sense is no puzzle: the list, at the
    // size and difficulty the file names if it names any.
    for nonsense in [
        "",
        "\u{0}\u{1}",
        "size=10\ndifficulty=hard\n",
        "size=9\ndifficulty=silly\nelapsed=x\ncursor=1\n...\n...\n...\n",
        "size=7\ndifficulty=easy\nelapsed=5\ncursor=0,0\n.......\n...\n",
        "size=7\ndifficulty=easy\nelapsed=5\ncursor=0,0\n..........\n",
        "size=7\ndifficulty=easy\ncursor=0,0\n.......\n.......\n...?...\n.......\n.......\n.......\n.......\n",
    ] {
        std::fs::write(&puzzle, nonsense).unwrap();
        r.launch(APP);
        assert_eq!(r.ui.texts(APP), ["New puzzle"], "{nonsense:?}");
        assert_eq!(r.ui.softkeys(APP), ["Play", "Help", "Exit", "Scores"]);
        r.softkey(APP, "Exit");
        assert!(!r.mgr.is_running(APP));
    }
    // The cursor of a file is put inside the grid.
    std::fs::write(&puzzle, WRONG.replace("cursor=3,1", "cursor=99,x")).unwrap();
    r.launch(APP);
    assert_eq!(state(&mut r).1, (6, 0));
}

#[test]
fn undo_takes_changes_back_one_by_one_and_is_kept() {
    let (grid, cursor) = parse(WRONG);
    let mut game = Game::new(grid);
    game.set_cursor(cursor.0, cursor.1);
    let start = game.clone();
    let mut r = rig_with(Model::D60, WRONG);
    r.launch(APP);
    assert_eq!(r.ui.softkeys(APP), SOFTKEYS);
    // Nothing to take back: nothing happens, and nothing is drawn.
    let draws = r.ui.draw_count(APP);
    r.softkey(APP, "Undo");
    assert_eq!(r.ui.draw_count(APP), draws);
    assert_eq!(drawn(&r), expected(Model::D60, &game));

    // Clearing the light under the cursor takes three errors off (the
    // lights at the other end of its row and below it are no longer
    // shone on); Undo brings the light and the errors back.
    let red = |r: &Rig| {
        drawn(r)
            .iter()
            .flatten()
            .filter(|op| matches!(op, Op::Fill { color, .. } | Op::Bitmap { color, .. } | Op::Text { color, .. } if *color == Rgb(0xE0, 0x20, 0x20)))
            .count()
    };
    assert_eq!(red(&r), 4);
    r.key(APP, Key::Digit(1));
    game.act(Act::Clear);
    assert_eq!(red(&r), 1);
    r.softkey(APP, "Undo");
    assert!(game.undo());
    assert_eq!(game, start);
    assert_eq!((drawn(&r), red(&r)), (expected(Model::D60, &game), 4));

    // A light that makes an error, taken back: the error drawing goes.
    keys(&mut r, &[Key::Down, Key::Down, Key::Down, Key::Digit(1)]);
    for _ in 0..3 {
        game.move_cursor(0, 1);
    }
    assert!(!game.act(Act::Light), "a black cell: no step");
    keys(&mut r, &[Key::Right, Key::Digit(1)]);
    game.move_cursor(1, 0);
    assert!(game.act(Act::Light));
    assert!(game.grid().clue_overfull(game.grid().index(3, 4)));
    assert_eq!(red(&r), 5, "the 0 with a light beside it");
    assert_eq!(drawn(&r), expected(Model::D60, &game));
    let with_error = red(&r);
    r.softkey(APP, "Undo");
    game.undo();
    assert!(red(&r) < with_error);
    assert_eq!((drawn(&r), red(&r)), (expected(Model::D60, &game), 4));
    assert_eq!(game.grid(), start.grid());

    // Each kind of change, several on one cell and some elsewhere; the
    // cursor moves in between are no steps.
    keys(
        &mut r,
        &[
            Key::Up,
            Key::Digit(3),
            Key::Digit(1),
            Key::Digit(1),
            Key::Digit(3),
        ],
    );
    game.move_cursor(0, -1);
    for act in [Act::Mark, Act::Light, Act::Clear, Act::Mark] {
        assert!(game.act(act));
    }
    keys(
        &mut r,
        &[Key::Left, Key::Left, Key::Left, Key::Digit(1), Key::Up],
    );
    for _ in 0..3 {
        game.move_cursor(-1, 0);
    }
    assert!(game.act(Act::Light));
    game.move_cursor(0, -1);
    assert_eq!(state(&mut r), (game.grid().clone(), game.cursor()));
    assert_eq!(game.changes_text(), "25.25x25L25.22.");
    // Written down with the puzzle, as the changes and no more.
    assert!(
        file(&r).contains("\nundo=25.25x25L25.22.\n"),
        "{}",
        file(&r)
    );

    // Two steps back now; each puts the cursor on its cell and draws
    // what the board showed before.
    for _ in 0..2 {
        r.softkey(APP, "Undo");
        assert!(game.undo());
        assert_eq!(drawn(&r), expected(Model::D60, &game));
    }
    assert_eq!(game.cursor(), (4, 3));
    // The rest after the app was left and opened again.
    r.key(APP, Key::Cancel);
    assert!(!r.mgr.is_running(APP));
    assert!(file(&r).contains("\nundo=25.25x25L\n"), "{}", file(&r));
    r.launch(APP);
    assert_eq!(drawn(&r), expected(Model::D60, &game));
    while game.undo() {
        r.softkey(APP, "Undo");
        assert_eq!(drawn(&r), expected(Model::D60, &game));
    }
    assert_eq!(game.grid(), start.grid());
    let draws = r.ui.draw_count(APP);
    r.softkey(APP, "Undo");
    assert_eq!(r.ui.draw_count(APP), draws, "nothing left to take back");
    assert_eq!(state(&mut r), (start.grid().clone(), (4, 3)));
    assert!(file(&r).contains("\nundo=\n"));

    // Changes kept in a file that makes no sense are no changes.
    r.key(APP, Key::Cancel);
    std::fs::write(
        storage(&r.nvdata).join("puzzle"),
        WRONG.replace("cursor=3,1\n", "cursor=3,1\nundo=7.999L\n"),
    )
    .unwrap();
    r.launch(APP);
    let draws = r.ui.draw_count(APP);
    r.softkey(APP, "Undo");
    assert_eq!(r.ui.draw_count(APP), draws);
}

#[test]
fn key_8_does_what_the_numbers_settle_and_undo_takes_it_back_at_once() {
    let (grid, cursor) = parse(WRONG);
    let mut game = Game::new(grid);
    game.set_cursor(cursor.0, cursor.1);
    let start = game.clone();
    for model in [Model::D60, Model::D40, Model::D70] {
        let mut game = start.clone();
        let mut r = rig_with(model, WRONG);
        r.launch(APP);
        // Marks around the 0 and lights on the two cells of the 2, all
        // at once; the cursor stays.
        r.key(APP, Key::Digit(8));
        assert!(game.auto());
        let g = game.grid();
        assert!(g.is_marked(g.index(3, 5)) && g.is_marked(g.index(2, 4)));
        assert!(g.has_light(g.index(6, 5)) && g.has_light(g.index(5, 6)));
        assert_eq!(state(&mut r), (g.clone(), cursor), "{model:?}");
        assert_eq!(drawn(&r), expected(model, &game), "{model:?}");
        assert_eq!(r.ui.softkeys(APP), SOFTKEYS);
        let undo = format!("\nundo={}\n", game.changes_text());
        assert!(undo.matches('+').count() >= 4 && file(&r).contains(&undo));
        // Once more: there is nothing new to do, and nothing is drawn.
        let draws = r.ui.draw_count(APP);
        r.key(APP, Key::Digit(8));
        assert!(!game.auto());
        assert_eq!(r.ui.draw_count(APP), draws, "{model:?}");
        // A light by hand, and Undo twice: the light, and then all that
        // the key did, though the app was left and opened in between.
        keys(&mut r, &[Key::Right, Key::Digit(1)]);
        game.move_cursor(1, 0);
        assert!(game.toggle(Act::Light));
        r.softkey(APP, "Undo");
        assert!(game.undo());
        assert_eq!(drawn(&r), expected(model, &game), "{model:?}");
        r.key(APP, Key::Cancel);
        assert!(!r.mgr.is_running(APP));
        r.launch(APP);
        r.softkey(APP, "Undo");
        assert!(game.undo());
        assert_eq!(game.grid(), start.grid());
        assert_eq!(state(&mut r), (game.grid().clone(), game.cursor()));
        assert_eq!(drawn(&r), expected(model, &game), "{model:?}");
        assert!(file(&r).contains("\nundo=\n"));
        r.key(APP, Key::Cancel);
    }
}

#[test]
fn help_shows_how_to_play_and_leaves_the_puzzle_as_it_was() {
    use lightup_game::help::PAGES;
    for model in [Model::D60, Model::D40, Model::D70] {
        let mut r = rig_with(model, WRONG);
        r.launch(APP);
        keys(&mut r, &[Key::Right, Key::Digit(3)]);
        let (shown, kept) = (drawn(&r), state(&mut r));
        let elapsed = |r: &Rig| {
            file(r)
                .lines()
                .find_map(|l| l.strip_prefix("elapsed="))
                .unwrap()
                .parse::<u64>()
                .unwrap()
        };
        let before = elapsed(&r);

        r.softkey(APP, "Help");
        assert_eq!(r.ui.softkeys(APP), ["Next", "Back"]);
        let area = model.app_content_area();
        let palette = palette(model);
        let page = |r: &Rig, n: usize| {
            assert_eq!(
                r.ui.texts(APP),
                [format!("How to play ({}/5)", n + 1).as_str(), PAGES[n].text],
                "{model:?}"
            );
            // One picture, drawn as the board draws that grid, inside
            // the window below the title.
            let (mut width, mut height, mut ops) =
                lightup_game::draw::picture(&PAGES[n].grid(), area.width * 3 / 10 / 5, palette);
            if !model.is_color() {
                // Framed, where a lit cell is as white as the page.
                (width, height, ops) =
                    lightup_game::draw::framed(width, height, ops, Rgb(32, 32, 32));
            }
            assert_eq!(drawn(r), [ops], "{model:?} page {n}");
            let bounds = r.ui.drawings(APP)[0].0;
            assert_eq!((bounds.width, bounds.height), (width, height));
            assert!(
                bounds.y >= 15 && bounds.y as u16 + height <= area.height,
                "{model:?}: {bounds:?}"
            );
            assert!(bounds.x as u16 + width < area.width / 2);
        };
        page(&r, 0);
        // Next, Select, and the arrows turn the pages, around the ends.
        r.softkey(APP, "Next");
        page(&r, 1);
        r.key(APP, Key::Select);
        page(&r, 2);
        r.key(APP, Key::Right);
        page(&r, 3);
        r.softkey(APP, "Next");
        page(&r, 4);
        r.key(APP, Key::Down);
        page(&r, 0);
        r.key(APP, Key::Left);
        page(&r, 4);
        r.key(APP, Key::Up);
        page(&r, 3);
        r.key(APP, Key::Up);
        page(&r, 2);
        // The keys of the puzzle do nothing here.
        keys(
            &mut r,
            &[Key::Digit(5), Key::Digit(1), Key::Digit(3), Key::Digit(6)],
        );
        page(&r, 2);
        // The time does not run while the help is read.
        std::thread::sleep(Duration::from_millis(1500));
        r.softkey(APP, "Back");
        assert_eq!(beside(&r), PLAYING);
        assert_eq!(r.ui.softkeys(APP), SOFTKEYS);
        assert_eq!(drawn(&r), shown, "{model:?}");
        assert_eq!(state(&mut r), kept);
        assert!(elapsed(&r) - before < 1000, "the time ran during the help");
        // The change before the help can still be taken back.
        r.softkey(APP, "Undo");
        assert_ne!(drawn(&r), shown);
        // Cancel leaves the help too, and something over the help (a
        // call) leaves it where it was.
        let shown = drawn(&r);
        r.softkey(APP, "Help");
        r.softkey(APP, "Next");
        r.ui.cover(APP, true);
        r.settle();
        r.ui.cover(APP, false);
        r.settle();
        page(&r, 1);
        r.key(APP, Key::Cancel);
        assert!(r.mgr.is_running(APP));
        assert_eq!(drawn(&r), shown);
        r.key(APP, Key::Cancel);
    }
    // From the list of sizes, before any puzzle: back to the list.
    let mut r = rig(Model::D40, 3);
    r.launch(APP);
    r.softkey(APP, "Help");
    assert_eq!(r.ui.texts(APP)[0], "How to play (1/5)");
    r.softkey(APP, "Back");
    assert_eq!(r.ui.texts(APP), ["New puzzle"]);
    r.softkey(APP, "Help");
    r.key(APP, Key::Cancel);
    assert_eq!(r.ui.texts(APP), ["New puzzle"]);
    assert_eq!(r.ui.softkeys(APP), ["Play", "Help", "Exit", "Scores"]);
}

#[test]
fn generating_goes_on_in_turns_and_waits_while_a_call_has_the_screen() {
    let mut r = rig_with(Model::D60, WRONG);
    r.launch(APP);
    r.softkey(APP, "New");
    // The list is at the puzzle's size and difficulty.
    assert_eq!(r.ui.softkeys(APP), ["Play", "Help", "Back", "Scores"]);
    r.key(APP, Key::Select);
    assert_eq!(r.ui.texts(APP), ["7x7 Hard", "Generating\u{2026}"]);
    wait(&mut r);
    assert_eq!(parse(&file(&r)).0, native(7, Difficulty::Hard, 1));

    // A large hard one takes many turns.  Each is short: the app is
    // never stopped for taking too long, and softkeys are answered.
    choose_from_play(&mut r, 14, Difficulty::Hard);
    assert_eq!(r.ui.texts(APP), ["14x14 Hard", "Generating\u{2026}"]);
    assert_eq!(r.ui.softkeys(APP), ["New", "Exit"]);
    assert!(r.ui.drawings(APP).is_empty());
    // Something else takes the screen: the work stops, and nothing
    // arrives however long that lasts.
    r.ui.cover(APP, true);
    r.settle();
    let covered = Instant::now();
    while covered.elapsed() < Duration::from_millis(400) {
        r.mgr.pump();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(generating(&r));
    assert_eq!(
        parse(&file(&r)).0,
        native(7, Difficulty::Hard, 1),
        "the puzzle before is still the one kept"
    );
    r.ui.cover(APP, false);
    r.settle();
    wait(&mut r);
    assert_eq!(beside(&r), PLAYING);
    // The same puzzle as when made in one go.
    let large = native(14, Difficulty::Hard, 2);
    assert_eq!(parse(&file(&r)), (large.clone(), (0, 0)));
    assert!(file(&r).starts_with("size=14\ndifficulty=hard\nelapsed=0\n"));
    assert_eq!(r.ui.drawings(APP).len(), 16);

    // New while generating is the list again; Back from there is the
    // puzzle there was, untouched.
    r.key(APP, Key::Digit(1));
    let played = state(&mut r);
    choose_from_play(&mut r, 14, Difficulty::Hard);
    assert!(generating(&r));
    r.softkey(APP, "New");
    assert_eq!(r.ui.texts(APP), ["New puzzle"]);
    assert_eq!(r.ui.softkeys(APP), ["Play", "Help", "Back", "Scores"]);
    std::thread::sleep(Duration::from_millis(30));
    r.settle();
    assert_eq!(
        r.ui.texts(APP),
        ["New puzzle"],
        "the abandoned puzzle does not turn up"
    );
    r.softkey(APP, "Back");
    assert_eq!(beside(&r), PLAYING);
    assert_eq!(state(&mut r), played);

    // Exit while generating: the app stops, and the puzzle there was is
    // there the next time.
    choose_from_play(&mut r, 10, Difficulty::Hard);
    assert!(generating(&r));
    r.softkey(APP, "Exit");
    assert!(!r.mgr.is_running(APP));
    r.launch(APP);
    assert_eq!(beside(&r), PLAYING);
    assert_eq!(state(&mut r), played);
    // Cancel while generating leaves too.
    choose_from_play(&mut r, 10, Difficulty::Hard);
    r.key(APP, Key::Cancel);
    assert!(!r.mgr.is_running(APP));
}

fn choose_from_play(r: &mut Rig, side: usize, difficulty: Difficulty) {
    r.softkey(APP, "New");
    choose(r, side, difficulty);
}

#[test]
fn every_size_is_made_and_drawn_on_every_kind_of_display() {
    // Every size on each kind of display, drawn as the game crate draws
    // it there; the difficulties in turn, but the largest size easy (the
    // interpreter is slow in a test build; large hard ones are made in
    // the test above).
    let mut seed = 20;
    for (model, pitches) in [
        (Model::D40, [14, 10, 7]),
        (Model::D70, [19, 13, 9]),
        (Model::D60, [34, 24, 17]),
    ] {
        let mut r = rig(model, seed);
        r.launch(APP);
        for (n, side) in SIZES.into_iter().enumerate() {
            let difficulty = if side == 14 {
                Difficulty::Easy
            } else {
                Difficulty::ALL[seed as usize % 3]
            };
            if n == 0 {
                choose(&mut r, side, difficulty);
            } else {
                choose_from_play(&mut r, side, difficulty);
            }
            wait(&mut r);
            let puzzle = native(side, difficulty, seed);
            seed += 1;
            assert_eq!(
                parse(&file(&r)).0,
                puzzle,
                "{model:?} {side} {difficulty:?}"
            );
            assert_eq!(beside(&r), PLAYING);
            let game = Game::new(puzzle);
            assert_eq!(drawn(&r), expected(model, &game), "{model:?} {side}");
            let bounds: Vec<_> = r.ui.drawings(APP).into_iter().map(|d| d.0).collect();
            let board = pitches[n] * side as u16 + 1;
            assert_eq!((bounds[1].y - bounds[0].y) as u16, pitches[n]);
            assert_eq!((bounds[side].width, bounds[side].height), (board, board));
            let area = model.app_content_area();
            assert!(bounds[side].y >= 0 && bounds[side].y as u16 + board <= area.height);
        }
        r.key(APP, Key::Cancel);
    }
}

#[test]
fn it_is_a_built_in_app_that_can_be_switched_off() {
    assert_eq!(
        common::capabilities(APP),
        ["ui", "timers", "storage", "l10n", "canvas"],
        "no sound, no calls, no network but the store on the PBX; dates as the phone writes them"
    );
    let config = FakeConfig::default();
    let disabled = config.disabled.clone();
    let mut r = Rig::new(config);
    let listed = r.store.get::<dphone_state::Apps>();
    let entry = listed
        .installed
        .iter()
        .find(|a| a.name == APP)
        .expect("listed");
    assert_eq!(
        (entry.display_name.as_str(), entry.icon.as_str()),
        ("Light Up", "app-lightup")
    );
    assert!(entry.builtin);
    r.launch(APP);
    *disabled.lock().unwrap() = vec![APP.into()];
    r.store
        .emit(dphone_state::Event::ConfigApplied { generation: 2 });
    r.settle();
    assert!(!r.mgr.is_running(APP));
    assert!(r.mgr.launch(APP).is_err());
    let apps = r.store.get::<dphone_state::Apps>();
    assert!(apps.installed.iter().all(|a| a.name != APP));
    assert!(apps.disabled.iter().any(|a| a.name == APP));
}

#[test]
fn without_a_seed_file_every_puzzle_is_a_new_one() {
    // The phone's random source seeds each puzzle: two phones (or one
    // phone twice) do not get the same one.
    let mut puzzles = Vec::new();
    for _ in 0..3 {
        let mut r = Rig::with_budget(
            staged(),
            FakeConfig::default(),
            Model::D60,
            Duration::from_secs(2),
        );
        listen();
        r.launch(APP);
        choose(&mut r, 10, Difficulty::Easy);
        wait(&mut r);
        let (puzzle, _) = parse(&file(&r));
        assert_eq!(Solve::new(&puzzle, Techniques::EASY).run(), Outcome::Unique);
        puzzles.push(puzzle);
    }
    assert!(puzzles[0] != puzzles[1] && puzzles[1] != puzzles[2] && puzzles[0] != puzzles[2]);
}

// ---------------------------------------------- generation, timed

struct Lines;
static LINES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

impl log::Log for Lines {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.target() == "app" || m.level() <= log::Level::Warn
    }
    fn log(&self, record: &log::Record) {
        if record.target() == "app" {
            LINES.lock().unwrap().push(record.args().to_string());
        } else if self.enabled(record.metadata()) {
            // E.g., why the manager stopped the app.
            eprintln!("{}: {}", record.level(), record.args());
        }
    }
    fn flush(&self) {}
}

/// Keeps what the app logs, and shows what goes wrong around it.
fn listen() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        log::set_logger(&Lines).unwrap();
        log::set_max_level(log::LevelFilter::Info);
    });
}

/// How long the app takes to make puzzles under the interpreter (as it
/// reports in the log), and the same puzzles made natively.  Not a test
/// of anything: run it with `--release --ignored --nocapture` to read the
/// numbers.  `LIGHT_UP_SEEDS` sets how many puzzles of each kind.
#[test]
#[ignore]
fn generation_times() {
    let seeds: u64 = std::env::var("LIGHT_UP_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(15);
    println!(
        "size  difficulty  native ms (median, worst)  interpreted ms (median, worst)  ratio  grids (median, worst)"
    );
    for side in SIZES {
        for difficulty in Difficulty::ALL {
            let (mut fast, mut slow, mut grids) = (Vec::new(), Vec::new(), Vec::new());
            let mut r = rig(Model::D60, 1000);
            r.launch(APP);
            for seed in 1000..1000 + seeds {
                let start = Instant::now();
                let mut g = Generator::new(Params::preset(side, difficulty), seed, MAX_GRIDS);
                while g.step() == Progress::Working {}
                fast.push(start.elapsed().as_micros() as u64);
                grids.push(g.attempts());

                LINES.lock().unwrap().clear();
                if seed == 1000 {
                    choose(&mut r, side, difficulty);
                } else {
                    choose_from_play(&mut r, side, difficulty);
                }
                wait(&mut r);
                assert_eq!(parse(&file(&r)).0, *g.puzzle().unwrap());
                let line = LINES
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|l| l.contains("a puzzle after"))
                    .cloned()
                    .expect("a log line");
                let ms: u64 = line
                    .split("after ")
                    .nth(1)
                    .unwrap()
                    .split(' ')
                    .next()
                    .unwrap()
                    .parse()
                    .unwrap();
                slow.push(ms * 1000);
            }
            for v in [&mut fast, &mut slow] {
                v.sort();
            }
            grids.sort();
            let (mid, last) = (seeds as usize / 2, seeds as usize - 1);
            let ms = |us: u64| us as f64 / 1000.0;
            println!(
                "{side:>2}x{side:<2} {:<10} {:>10.2} {:>8.2} {:>16.0} {:>8.0} {:>12.0} {:>8} {:>6}",
                difficulty.name(),
                ms(fast[mid]),
                ms(fast[last]),
                ms(slow[mid]),
                ms(slow[last]),
                slow.iter().sum::<u64>() as f64 / fast.iter().sum::<u64>() as f64,
                grids[mid],
                grids[last],
            );
        }
    }
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
    /// When the time on the playing screen was last seen to change.
    ticked: Option<Instant>,
    /// Where the time spent is on the playing screen: right of this
    /// column and above this row (and some of the picture of the keys
    /// with it).
    time_box: (usize, usize),
    _dir: common::TempDir,
}

impl Phone {
    /// A phone on which the app finds this puzzle file.
    fn new(model: Model, puzzle: &str) -> Phone {
        Phone::on_pbx(model, puzzle, &FakeStore::default())
    }

    /// The same, with `app_data` as the PBX's store of app data.
    fn on_pbx(model: Model, puzzle: &str, app_data: &FakeStore) -> Phone {
        let dir = temp_dir("lightup-phone");
        let nvdata = dir.0.join("nvdata");
        std::fs::create_dir_all(storage(&nvdata)).unwrap();
        std::fs::write(storage(&nvdata).join("puzzle"), puzzle).unwrap();
        std::fs::write(storage(&nvdata).join("seed"), "7").unwrap();
        // The time is beside the board, a line in the upper half.
        let side = puzzle
            .lines()
            .find_map(|l| l.strip_prefix("size=")?.parse().ok())
            .unwrap_or(SIZES[0]);
        let area = model.app_content_area();
        let board = Layout::new(area.width, area.height, side).board_side;
        let time_box = (board as usize, (area.height as usize + 25) / 2);
        let mut hal = Hal::sim(model);
        let sim = hal.sim.clone().unwrap();
        let store = Store::new(model);
        // The first theme, whose palette is the color skin's own: the
        // colors this test names are that palette's.
        let ui = Ui::from_hal(&mut hal, store.clone(), UiConfig { theme: "harbor".into(), ..UiConfig::default() });
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
            services_with_store(&FakePbx::default(), FakeConfig::default(), app_data),
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
            ticked: None,
            time_box,
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

    /// With an unsolved puzzle on the screen, waits until the time on
    /// it has just changed (unless it did so a moment ago): what follows
    /// at once is then counted and compared without the time changing in
    /// between.
    fn steady(&mut self) {
        if self.ticked.is_some_and(|t| t.elapsed() < Duration::from_millis(300)) {
            return;
        }
        let (before, start) = (self.sim.frame_generation(), Instant::now());
        while self.sim.frame_generation() == before {
            assert!(start.elapsed() < Duration::from_secs(10), "the time stands");
            std::thread::sleep(Duration::from_millis(1));
            self.settle();
        }
        self.ticked = Some(Instant::now());
    }

    /// Presses keys; returns how many times the display was sent
    /// something, not counting the time spent going on.
    fn tap(&mut self, keys: &[Key]) -> u64 {
        let mut sent = 0;
        for key in keys {
            let (before, time) = (self.sim.frame_generation(), self.time());
            let rest = self.pixels().0;
            self.sim.tap(*key);
            self.settle();
            let mut frames = self.sim.frame_generation() - before;
            if self.time() != time {
                // The time went on meanwhile, which was sent with what
                // the key changed, or by itself.
                let least = u64::from(self.pixels().0 != rest);
                frames = frames.saturating_sub(1).max(least);
            }
            sent += frames;
        }
        sent
    }

    fn in_time_box(&self, i: usize, width: usize) -> bool {
        i % width >= self.time_box.0 && i / width < self.time_box.1
    }

    /// The pixels where the time spent is.
    fn time(&self) -> Vec<u32> {
        let (frame, width) = self.frame();
        let inside = (0..frame.len()).filter(|i| self.in_time_box(*i, width));
        inside.map(|i| frame[i]).collect()
    }

    /// The pixels of the frame as 0xRRGGBB, and the frame's width, with
    /// black where the time spent is: it goes on by itself.
    fn pixels(&self) -> (Vec<u32>, usize) {
        let (mut frame, width) = self.frame();
        for (i, pixel) in frame.iter_mut().enumerate() {
            if self.in_time_box(i, width) {
                *pixel = 0;
            }
        }
        (frame, width)
    }

    fn file(&self) -> String {
        local(&self.nvdata, "puzzle")
    }

    /// The pixels of the frame as 0xRRGGBB, and the frame's width.
    fn frame(&self) -> (Vec<u32>, usize) {
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
    /// `before`, and how many differ: (width, height, pixels).
    fn changed(&self, before: &[u32]) -> (usize, usize, usize) {
        let (now, width) = self.pixels();
        let differ: Vec<usize> = (0..now.len()).filter(|i| now[*i] != before[*i]).collect();
        if differ.is_empty() {
            return (0, 0, 0);
        }
        let xs = differ.iter().map(|i| i % width);
        let (left, right) = (xs.clone().min().unwrap(), xs.max().unwrap());
        (
            right - left + 1,
            differ[differ.len() - 1] / width - differ[0] / width + 1,
            differ.len(),
        )
    }

    /// Writes the frame as a PNG (the gray ones enlarged three times).
    fn shot(&self, name: &str) {
        let dir = match std::env::var_os("LIGHT_UP_PREVIEWS") {
            Some(d) => PathBuf::from(d),
            None => PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("light-up"),
        };
        std::fs::create_dir_all(&dir).unwrap();
        let (pixels, w) = self.frame();
        let h = pixels.len() / w;
        let scale = if self.model.is_color() { 1 } else { 3 };
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

/// A puzzle file: fresh, in progress (about half of the lights, some
/// marks, the cursor in the middle, and two wrong lights: one that
/// another light shines on, one that is one too many for a number), or
/// solved.
fn puzzle_file(side: usize, difficulty: Difficulty, seed: u64, stage: &str) -> String {
    let puzzle = native(side, difficulty, seed);
    let solved = solution(&puzzle);
    let mut game = Game::new(puzzle.clone());
    let put = |game: &mut Game, i: usize, act| {
        game.set_cursor(i % side, i / side);
        game.act(act);
    };
    let lights: Vec<usize> = (0..puzzle.cells())
        .filter(|i| solved.has_light(*i))
        .collect();
    let (elapsed, cursor) = match stage {
        "fresh" => (0, (0, 0)),
        "solved" => {
            for i in &lights {
                put(&mut game, *i, Act::Light);
            }
            (60_000 * side as u64 + 7_000, (0, 0))
        }
        _ => {
            for i in lights.iter().step_by(2) {
                put(&mut game, *i, Act::Light);
            }
            for i in (0..puzzle.cells())
                .filter(|i| solved.is_marked(*i))
                .step_by(3)
            {
                put(&mut game, i, Act::Mark);
            }
            // One light too many beside a number that is already met.
            let g = game.grid().clone();
            let full = (0..g.cells()).find_map(|c| {
                let n = g.clue(c)? as usize;
                let (cells, count) = g.neighbors(c);
                let free = cells[..count]
                    .iter()
                    .map(|j| *j as usize)
                    .find(|j| !g.is_black(*j) && !g.has_light(*j))?;
                (g.lights_beside(c) == n && n > 0).then_some(free)
            });
            put(&mut game, full.expect("a number that is met"), Act::Light);
            // A light where another light already shines, away from that.
            let g = game.grid().clone();
            let seen = (0..g.cells())
                .rev()
                .find(|i| g.is_lit(*i) && !g.has_light(*i) && !g.is_marked(*i));
            put(&mut game, seen.expect("a lit cell"), Act::Light);
            assert!((0..g.cells()).any(|i| game.grid().clue_overfull(i)));
            assert!((0..g.cells()).any(|i| game.grid().light_conflicts(i)));
            (95_000, (side / 2, side / 2 - 1))
        }
    };
    format!(
        "size={side}\ndifficulty={}\nelapsed={elapsed}\ncursor={},{}\n{}",
        difficulty.name().to_lowercase(),
        cursor.0,
        cursor.1,
        game.grid().to_text()
    )
}

/// Every size, fresh, in progress, and solved, with a picture of each;
/// and the list of sizes.  Returns the phones showing the 14x14 puzzle
/// fresh, in progress, and solved.
fn pictures(model: Model) -> [Phone; 3] {
    let mut large = Vec::new();
    for (n, side) in SIZES.into_iter().enumerate() {
        let difficulty = Difficulty::ALL[n];
        for stage in ["fresh", "play", "solved"] {
            let phone = Phone::new(model, &puzzle_file(side, difficulty, 30 + n as u64, stage));
            phone.shot(&format!("{side}x{side}-{stage}"));
            if side == 14 {
                large.push(phone);
            }
        }
    }
    let mut list = Phone::new(model, "");
    list.shot("new");
    // The first row of the list: a 7x7 easy puzzle, made on the spot.
    list.tap(&[Key::Select]);
    let start = Instant::now();
    while list.file().is_empty() {
        assert!(start.elapsed() < Duration::from_secs(120));
        std::thread::sleep(Duration::from_millis(1));
        list.settle();
    }
    list.settle();
    assert_eq!(parse(&list.file()).0, native(7, Difficulty::Easy, 7));
    // The pages of the help, from the third softkey of a puzzle.
    let mut help = Phone::new(model, &puzzle_file(7, Difficulty::Easy, 30, "play"));
    help.steady();
    let puzzle = help.pixels().0;
    help.tap(&[Key::Soft(3)]);
    for page in 1..=5 {
        help.shot(&format!("help-{page}"));
        help.tap(&[Key::Soft(1)]);
    }
    // Back, the second softkey there, is the puzzle as it was.
    help.tap(&[Key::Soft(2)]);
    assert_eq!(help.pixels().0, puzzle);
    // Undo, the first softkey, takes back a light just put: the display
    // is as before again.
    help.tap(&[Key::Right, Key::Right]);
    help.steady();
    let before = help.pixels().0;
    if help.tap(&[Key::Digit(3)]) > 0 {
        assert_ne!(help.pixels().0, before);
        help.tap(&[Key::Soft(1)]);
        assert_eq!(help.pixels().0, before);
    }
    large.try_into().ok().expect("three phones")
}

/// The display is sent something once for a key that changes something,
/// and only the part that changed; nothing for a key that changes
/// nothing.  `pitch` is the size of a cell.
fn only_what_changed_is_sent(fresh: &mut Phone, pitch: usize) {
    let model = fresh.model;
    // The cursor at an edge.
    fresh.steady();
    assert_eq!(
        fresh.tap(&[Key::Left, Key::Up, Key::Digit(4), Key::Digit(2)]),
        0,
        "{model:?}"
    );
    // One step: the two cells, and the lines around them.
    fresh.steady();
    let before = fresh.pixels().0;
    assert_eq!(fresh.tap(&[Key::Right]), 1, "{model:?}");
    let (width, height, _) = fresh.changed(&before);
    assert!(
        width <= 2 * pitch + 1 && height <= pitch + 1 && width > pitch,
        "{model:?}: {width}x{height}"
    );
    // A light: its row and its column at most.
    let (grid, cursor) = parse(&fresh.file());
    let side = grid.width();
    let white = (0..side)
        .find(|x| !grid.is_black(grid.index(*x, 0)))
        .unwrap();
    fresh.tap(&path(cursor, (1, 0), false));
    fresh.tap(&path((1, 0), (white, 0), true));
    fresh.steady();
    let before = fresh.pixels().0;
    assert_eq!(fresh.tap(&[Key::Digit(1)]), 1, "{model:?}");
    let (width, height, differ) = fresh.changed(&before);
    assert!(
        width <= side * pitch + 1 && height <= side * pitch + 1,
        "{model:?}: {width}x{height}"
    );
    let lit = grid.seen(grid.index(white, 0)).as_slice().len() + 1;
    assert!(
        differ > 0 && differ <= lit * pitch * pitch,
        "{model:?}: {differ} pixels for {lit} cells"
    );
    // Once more, and with Select: the light goes, and is back.  A mark
    // in its place, and the mark off: once each.  The keys that do
    // nothing: nothing.
    fresh.steady();
    assert_eq!(fresh.tap(&[Key::Digit(1), Key::Select]), 2, "{model:?}");
    assert_eq!(fresh.tap(&[Key::Digit(3)]), 1, "{model:?}");
    assert_eq!(fresh.tap(&[Key::Digit(3)]), 1, "{model:?}");
    assert_eq!(
        fresh.tap(&[Key::Digit(7), Key::Digit(9), Key::Digit(0)]),
        0,
        "{model:?}"
    );
    assert_eq!(fresh.pixels().0.len(), before.len());
}

#[test]
fn drawn_on_a_color_phone() {
    let [mut fresh, play, solved] = pictures(Model::D60);
    // Lit cells are yellow, errors red, and the cursor blue.  Nothing is
    // lit on the fresh puzzle, all of the solved one is, and that has no
    // cursor and no red.
    let (yellow, red, blue, unlit) = (0xFFF070, 0xE02020, 0x1060F0, 0xC4C4C4);
    // (But for the lit cell in the picture of the keys.)
    assert!(fresh.count(yellow) > 0 && fresh.count(yellow) < 46 * 46);
    assert_eq!(fresh.count(red), 0);
    assert!(fresh.count(unlit) > 100 * 16 * 16 && fresh.count(blue) > 4 * 17);
    assert!(play.count(yellow) > 30 * 16 * 16 && play.count(unlit) > 16 * 16);
    assert!(
        play.count(red) > 2 * 40 + 20,
        "a number and two lights: {}",
        play.count(red)
    );
    assert!(play.count(blue) > 4 * 17);
    assert_eq!(
        (solved.count(unlit), solved.count(red), solved.count(blue)),
        (0, 0, 0)
    );
    assert!(solved.count(yellow) > 100 * 16 * 16);

    only_what_changed_is_sent(&mut fresh, 17);

    // A call comes in: its screen takes the display, and the keys are
    // the call's.  The puzzle waits, written down, and is on the display
    // again, as it was, when the call is over.
    let mut p = play;
    p.tap(&[Key::Right, Key::Digit(3)]);
    p.steady();
    let shown = p.pixels().0;
    assert_eq!(p.ui.screen_name(), "app:lightup");
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
    assert_ne!(p.pixels().0, shown);
    let kept = p.file();
    assert!(kept.contains("cursor=8,6\n"), "{kept}");
    assert_eq!(
        parse(&kept).0.to_text().matches('x').count(),
        parse(&puzzle_file(14, Difficulty::Hard, 32, "play"))
            .0
            .to_text()
            .matches('x')
            .count()
            + 1
    );
    p.tap(&[Key::Left, Key::Digit(4), Key::Digit(1), Key::Digit(5)]);
    assert_eq!(p.file(), kept);
    p.store.update::<Calls, _>(|c| {
        c.calls.clear();
        c.focused = None;
    });
    p.settle();
    assert_eq!(p.ui.screen_name(), "app:lightup");
    assert!(p.mgr.is_running(APP));
    assert_eq!(p.pixels().0, shown);
    p.steady();
    assert_eq!(p.tap(&[Key::Left]), 1, "the puzzle goes on");
}

/// A puzzle file of the given side with, in its top left corner, a 2
/// with three lights beside it, a 1 with no cell left for its light (the
/// four beside it are lit from away, lit from away, marked, and black), a
/// 1 that is met, and a 2 that can still be met; the cursor under the
/// mark.
fn unsatisfiable_file(side: usize) -> String {
    let mut rows = vec![".".repeat(side); side];
    for (row, cells) in [
        ".L.....", "L2L.1x.", "....#..", "...1L..", ".......", ".2.....",
    ]
    .into_iter()
    .enumerate()
    {
        rows[row] = format!("{cells}{}", ".".repeat(side - 7));
    }
    format!(
        "size={side}\ndifficulty=easy\nelapsed=61000\ncursor=5,2\n{}\n",
        rows.join("\n")
    )
}

#[test]
fn numbers_that_cannot_be_met_are_drawn_as_errors() {
    let red = 0xE02020;
    for (model, pitches) in [
        (Model::D40, [14, 10, 7]),
        (Model::D70, [19, 13, 9]),
        (Model::D60, [34, 24, 17]),
    ] {
        for (side, pitch) in SIZES.into_iter().zip(pitches) {
            let what = format!("{model:?} {side}");
            let file = unsatisfiable_file(side);
            let grid = parse(&file).0;
            let wrong: Vec<usize> = (0..grid.cells())
                .filter(|i| grid.clue_unsatisfiable(*i))
                .collect();
            assert_eq!(wrong, [grid.index(1, 1), grid.index(4, 1)], "{what}");
            assert!(grid.clue_overfull(wrong[0]) && !grid.clue_overfull(wrong[1]));
            assert!((0..grid.cells()).all(|i| !grid.light_conflicts(i)));
            let mut phone = Phone::new(model, &file);
            phone.shot(&format!("unsat-{side}x{side}"));
            // The two numbers are red on the color display, and their
            // cells are not.
            let both = phone.count(red);
            if model.is_color() {
                assert!(both > 0 && both < pitch * pitch, "{what}: {both}");
            }
            // Taking the mark off leaves the 1 a cell for its light: the
            // number beside the mark is drawn anew with it, and is an
            // error no more.
            phone.tap(&[Key::Up]);
            phone.steady();
            let before = phone.pixels().0;
            assert_eq!(phone.tap(&[Key::Digit(3)]), 1, "{what}");
            let mut after = grid.clone();
            assert!(after.set_mark(after.index(5, 1), false));
            assert!(!after.clue_unsatisfiable(wrong[1]) && after.clue_unsatisfiable(wrong[0]));
            let (width, height, _) = phone.changed(&before);
            assert!(
                width > pitch + 2 && width < 2 * pitch && height < pitch,
                "{what}: {width}x{height}"
            );
            if model.is_color() {
                let one = phone.count(red);
                assert!(one > 0 && one < both, "{what}: {one} of {both}");
            }
        }
    }
}

#[test]
fn drawn_on_the_gray_phones() {
    for (model, pitch) in [(Model::D40, 7), (Model::D70, 9)] {
        let [mut fresh, play, solved] = pictures(model);
        for phone in [&fresh, &play, &solved] {
            // Black and white are there; the board uses nothing else
            // (the texts beside it may be smoothed with grays).
            let (pixels, w) = phone.pixels();
            let grays: std::collections::BTreeSet<u32> = pixels.iter().copied().collect();
            assert!(
                grays.contains(&0x000000) && grays.contains(&0xFFFFFF),
                "{model:?}: {grays:?}"
            );
            let board = 14 * pitch + 1;
            let top = pixels.len() / w - model.app_content_area().height as usize;
            let _ = (board, top);
        }
        // The solved puzzle has no dotted (unlit) cells: far fewer black
        // pixels in single dots than the fresh one.
        assert!(
            fresh.count(0x000000) > solved.count(0x000000) / 2,
            "{model:?}"
        );
        assert_ne!(fresh.pixels().0, solved.pixels().0);
        assert_ne!(play.pixels().0, solved.pixels().0);
        only_what_changed_is_sent(&mut fresh, pitch);
    }
}

// ------------------------------------------- the time and the best times

/// A puzzle file one light from solved, with the cursor on the cell of
/// that light: key 1 solves it.
fn almost(side: usize, difficulty: Difficulty, seed: u64, elapsed: u64) -> String {
    let puzzle = native(side, difficulty, seed);
    let solved = solution(&puzzle);
    let lights: Vec<usize> = (0..puzzle.cells())
        .filter(|i| solved.has_light(*i))
        .collect();
    let mut game = Game::new(puzzle);
    for i in &lights[1..] {
        game.set_cursor(i % side, i / side);
        game.act(Act::Light);
    }
    assert!(!game.solved());
    format!(
        "size={side}\ndifficulty={}\nelapsed={elapsed}\ncursor={},{}\n{}",
        difficulty.name().to_lowercase(),
        lights[0] % side,
        lights[0] / side,
        game.grid().to_text()
    )
}

fn put_puzzle(r: &Rig, puzzle: &str) {
    std::fs::create_dir_all(storage(&r.nvdata)).unwrap();
    std::fs::write(storage(&r.nvdata).join("seed"), "1").unwrap();
    std::fs::write(storage(&r.nvdata).join("puzzle"), puzzle).unwrap();
}

/// Nov 13, 2026, 2:15 PM UTC: what the wall clock of the phones of the
/// tests of the best times says, unless a test moves it.
const T: u64 = 1_794_579_300;
/// `T` as the phones show it.
const T_SHOWN: &str = "Nov 13, 2026 2:15 PM";

/// A configuration whose wall clock says `T`, and the clock.
fn clock_at_t() -> (FakeConfig, Arc<AtomicU64>) {
    let clock = Arc::new(AtomicU64::new(T));
    let config = FakeConfig {
        wall_time: Some(clock.clone()),
        ..FakeConfig::default()
    };
    (config, clock)
}

/// A phone of a PBX (`FakeStore::phone`) on which the app finds this
/// puzzle file.
fn rig_on(app_data: FakeStore, puzzle: &str) -> Rig {
    listen();
    let r = Rig::with_app_data(
        staged(),
        clock_at_t().0,
        Model::D60,
        Duration::from_secs(2),
        app_data,
    );
    put_puzzle(&r, puzzle);
    r
}

/// The times the phone has yet to write to the PBX, as it keeps them.
fn waiting(r: &Rig) -> String {
    local(&r.nvdata, "times")
}

/// The table of a kind of puzzle as the PBX has it.
fn table(pbx: &FakeStore, kind: Kind) -> Table {
    Table::parse(
        &pbx.value(APP, StoreScope::Shared, &kind.key())
            .unwrap_or_default(),
    )
}

/// The rows of a table: who, and the time in whole seconds.
fn rows(table: &Table) -> Vec<(&str, &str, u64)> {
    table
        .rows()
        .iter()
        .map(|r| (r.id.as_str(), r.name.as_str(), r.ms / 1000))
        .collect()
}

/// The lines of the table on the screen, each the place and the name,
/// when the puzzle was solved, and the time; none if it says something
/// in place of them.
fn lines(r: &Rig) -> Vec<[String; 3]> {
    // After the title, four texts a line: under it all (empty), and the
    // three.
    let texts = r.ui.texts(APP);
    let rest = &texts[1..];
    if !rest.len().is_multiple_of(4) || rest.iter().step_by(4).any(|t| !t.is_empty()) {
        return Vec::new();
    }
    rest.chunks(4)
        .map(|c| [c[1].clone(), c[2].clone(), c[3].clone()])
        .collect()
}

/// The place and the name, and the time, of each line on the screen.
fn listed(r: &Rig) -> Vec<(String, String)> {
    lines(r)
        .into_iter()
        .map(|[name, _, time]| (name, time))
        .collect()
}

/// Starts the app on its puzzle file, which is one light from solved,
/// and puts the light.
fn solve(r: &mut Rig) {
    r.launch(APP);
    assert_eq!(r.ui.softkeys(APP), SOFTKEYS);
    r.key(APP, Key::Digit(1));
    assert_eq!(r.ui.texts(APP)[0], "Solved!");
}

/// Lets the phone run for a while.
fn pass(r: &mut Rig, time: Duration) {
    let start = Instant::now();
    while start.elapsed() < time {
        std::thread::sleep(Duration::from_millis(5));
        r.mgr.pump();
    }
    r.settle();
}

fn seconds(time: &str) -> u64 {
    time.split(':').fold(0, |s, n| s * 60 + n.parse::<u64>().unwrap())
}

#[test]
fn the_time_spent_is_shown_and_runs_while_the_puzzle_is_on_the_screen() {
    let mut r = rig_with(Model::D60, WRONG);
    r.launch(APP);
    // What the file says, and then it goes on.
    assert_eq!(seconds(&time_shown(&r)), 125);
    pass(&mut r, Duration::from_millis(2100));
    let shown = seconds(&time_shown(&r));
    assert!((127..130).contains(&shown), "{shown}");
    // It is the time the puzzle file gets.
    let kept = |r: &Rig| -> u64 {
        let file = file(r);
        let ms = file.lines().find_map(|l| l.strip_prefix("elapsed="));
        ms.expect("a time").parse().unwrap()
    };
    // Not while something else has the screen (e.g., a call) ...
    r.ui.cover(APP, true);
    r.settle();
    let covered = kept(&r);
    assert!((shown..shown + 2).contains(&(covered / 1000)), "{covered}");
    pass(&mut r, Duration::from_millis(1200));
    assert!(seconds(&time_shown(&r)) <= covered / 1000);
    r.ui.cover(APP, false);
    r.settle();
    let back = seconds(&time_shown(&r));
    assert!((covered / 1000..covered / 1000 + 2).contains(&back), "{back}");
    // ... nor under the help, the list of sizes, or the best times, each
    // of which gives the puzzle back with the time it had.
    for (open, close) in [("Help", "Back"), ("New", "Back"), ("Scores", "Back")] {
        let before = seconds(&time_shown(&r));
        r.softkey(APP, open);
        assert!(r.ui.texts(APP).iter().all(|t| !is_time(t)), "{open}");
        pass(&mut r, Duration::from_millis(1200));
        r.softkey(APP, close);
        let after = seconds(&time_shown(&r));
        assert!((before..before + 2).contains(&after), "{open}: {after}");
    }
    pass(&mut r, Duration::from_millis(1100));
    assert!(seconds(&time_shown(&r)) > back);
    // Undo and key 8 are moves like any other: the time goes on.
    let before = seconds(&time_shown(&r));
    r.key(APP, Key::Digit(8));
    r.softkey(APP, "Undo");
    assert!(seconds(&time_shown(&r)) >= before);
    // A solved puzzle shows the time it took, which stands.
    let mut r = rig_with(Model::D60, &almost(7, Difficulty::Easy, 5, 59_000));
    solve(&mut r);
    let texts = r.ui.texts(APP);
    assert!(texts[1] == "Time 0:59" || texts[1] == "Time 1:00", "{texts:?}");
    pass(&mut r, Duration::from_millis(1100));
    assert_eq!(r.ui.texts(APP), texts);
}

#[test]
fn a_solved_puzzles_time_is_on_every_phones_table_with_who_solved_it() {
    let pbx = FakeStore::default();
    let easy = Kind::new(7, Difficulty::Easy);
    let mut alice = rig_on(
        pbx.phone("101", "Alice"),
        &almost(7, Difficulty::Easy, 5, 83_000),
    );
    let mut bob = rig_on(pbx.phone("102", "Bob"), "");

    // Bob has no puzzle: the list of sizes, whose fourth softkey shows
    // the table of the row.  Nobody has solved a puzzle yet.
    bob.launch(APP);
    assert_eq!(bob.ui.softkeys(APP), ["Play", "Help", "Exit", "Scores"]);
    assert!(bob.app_data.watched().is_empty());
    bob.softkey(APP, "Scores");
    assert_eq!(
        bob.ui.texts(APP),
        ["Best times: 7x7 Easy", "No times yet."]
    );
    assert_eq!(bob.ui.softkeys(APP), ["Size", "Level", "Back"]);
    assert_eq!(bob.app_data.watched(), [APP]);

    // Alice solves hers, at T.
    solve(&mut alice);
    let ms = table(&pbx, easy).rows()[0].ms;
    assert_eq!(rows(&table(&pbx, easy)), [("101", "Alice", 83)]);
    assert_eq!(table(&pbx, easy).rows()[0].when, T);
    let time = lightup_game::scores::clock(ms);
    assert_eq!(alice.ui.texts(APP), ["Solved!", &format!("Time {time}")]);
    assert_eq!(waiting(&alice), "", "nothing waits");
    // Only the table of that kind of puzzle was written.
    for kind in Kind::all().filter(|k| *k != easy) {
        assert!(table(&pbx, kind).rows().is_empty());
    }

    // Bob's phone, with the table on its screen, is told, and shows it:
    // who, when, and the time.
    let first = [("1. Alice".to_string(), time.clone())];
    bob.settle();
    assert_eq!(bob.ui.texts(APP)[0], "Best times: 7x7 Easy");
    assert_eq!(lines(&bob), [["1. Alice", T_SHOWN, time.as_str()]]);

    // The fourth softkey of the solved puzzle: the table of its kind.
    assert_eq!(alice.ui.softkeys(APP), ["New", "Help", "Exit", "Scores"]);
    alice.softkey(APP, "Scores");
    assert_eq!(alice.ui.texts(APP)[0], "Best times: 7x7 Easy");
    assert_eq!(listed(&alice), first);
    // The other kinds: the next size, the next difficulty, and left and
    // right through all of them.
    alice.softkey(APP, "Size");
    assert_eq!(
        alice.ui.texts(APP),
        ["Best times: 10x10 Easy", "No times yet."]
    );
    alice.softkey(APP, "Level");
    assert_eq!(alice.ui.texts(APP)[0], "Best times: 10x10 Medium");
    alice.softkey(APP, "Level");
    alice.softkey(APP, "Level");
    assert_eq!(alice.ui.texts(APP)[0], "Best times: 10x10 Easy");
    alice.key(APP, Key::Left);
    assert_eq!(alice.ui.texts(APP)[0], "Best times: 7x7 Hard");
    alice.softkey(APP, "Size");
    alice.softkey(APP, "Size");
    alice.softkey(APP, "Size");
    assert_eq!(alice.ui.texts(APP)[0], "Best times: 7x7 Hard");
    for _ in 0..7 {
        alice.key(APP, Key::Right);
    }
    assert_eq!(alice.ui.texts(APP)[0], "Best times: 7x7 Easy");
    assert_eq!(listed(&alice), first);
    alice.key(APP, Key::Left);
    assert_eq!(alice.ui.texts(APP)[0], "Best times: 14x14 Hard");
    // Back, the softkey or Cancel, is the solved puzzle.
    alice.softkey(APP, "Back");
    assert_eq!(alice.ui.texts(APP), ["Solved!", &format!("Time {time}")]);
    alice.softkey(APP, "Scores");
    alice.key(APP, Key::Cancel);
    assert_eq!(alice.ui.texts(APP), ["Solved!", &format!("Time {time}")]);
    assert!(alice.mgr.is_running(APP));

    // Bob goes back to the list, to the row he came from, and his phone
    // no longer asks to be told.
    bob.softkey(APP, "Level");
    assert_eq!(bob.ui.texts(APP)[0], "Best times: 7x7 Medium");
    bob.softkey(APP, "Back");
    assert_eq!(bob.ui.texts(APP), ["New puzzle"]);
    assert!(bob.app_data.watched().is_empty());
    bob.softkey(APP, "Scores");
    assert_eq!(bob.ui.texts(APP)[0], "Best times: 7x7 Easy");
    bob.softkey(APP, "Back");
    bob.softkey(APP, "Exit");

    // He plays a puzzle of the same kind.  The fourth softkey while
    // playing: the table, and back to the puzzle.
    put_puzzle(&bob, &almost(7, Difficulty::Easy, 6, 125_000));
    bob.launch(APP);
    assert_eq!(bob.ui.softkeys(APP), SOFTKEYS);
    bob.softkey(APP, "Scores");
    assert_eq!(listed(&bob), first);
    bob.softkey(APP, "Back");
    assert_eq!(beside(&bob), PLAYING);
    assert_eq!(bob.ui.softkeys(APP), SOFTKEYS);
    // He solves it, in more time than Alice: the second row.
    bob.key(APP, Key::Digit(1));
    assert_eq!(bob.ui.texts(APP)[0], "Solved!");
    assert_eq!(
        rows(&table(&pbx, easy)),
        [("101", "Alice", 83), ("102", "Bob", 125)]
    );
    alice.softkey(APP, "Scores");
    assert_eq!(
        listed(&alice),
        [first[0].clone(), ("2. Bob".to_string(), "2:05".to_string())]
    );
    alice.softkey(APP, "Back");
    alice.softkey(APP, "Exit");

    // Alice again, slower than before: each time is a row of its own, so
    // her first stays and this one is the third row.
    put_puzzle(&alice, &almost(7, Difficulty::Easy, 7, 300_000));
    solve(&mut alice);
    assert_eq!(table(&pbx, easy).rows()[0].ms, ms);
    assert_eq!(
        rows(&table(&pbx, easy)),
        [("101", "Alice", 83), ("102", "Bob", 125), ("101", "Alice", 300)]
    );
    assert_eq!(waiting(&alice), "");
    alice.softkey(APP, "Exit");
    // And faster: the first row, and her others stay.
    put_puzzle(&alice, &almost(7, Difficulty::Easy, 8, 40_000));
    solve(&mut alice);
    assert_eq!(
        rows(&table(&pbx, easy)),
        [
            ("101", "Alice", 40),
            ("101", "Alice", 83),
            ("102", "Bob", 125),
            ("101", "Alice", 300)
        ]
    );
    alice.softkey(APP, "Scores");
    assert_eq!(
        listed(&alice).iter().map(|i| i.0.as_str()).collect::<Vec<_>>(),
        ["1. Alice", "2. Alice", "3. Bob", "4. Alice"]
    );
    // Her best line is in the selection's colors; up and down move them,
    // around the ends.
    let selected = |r: &Rig| r.ui.highlighted(APP).get(1).cloned().unwrap_or_default();
    assert_eq!(selected(&alice), "1. Alice");
    assert_eq!(alice.ui.highlighted(APP).len(), 4, "the line and its three texts");
    alice.key(APP, Key::Down);
    alice.key(APP, Key::Down);
    assert_eq!(selected(&alice), "3. Bob");
    for _ in 0..3 {
        alice.key(APP, Key::Up);
    }
    assert_eq!(selected(&alice), "4. Alice");
    alice.key(APP, Key::Down);
    assert_eq!(selected(&alice), "1. Alice");
    alice.softkey(APP, "Back");
    // Bob's time on another size is in another table.
    bob.softkey(APP, "Exit");
    put_puzzle(&bob, &almost(10, Difficulty::Hard, 9, 600_000));
    solve(&mut bob);
    assert_eq!(
        rows(&table(&pbx, Kind::new(10, Difficulty::Hard))),
        [("102", "Bob", 600)]
    );
    assert_eq!(table(&pbx, easy).rows().len(), 4);
}

#[test]
fn two_phones_that_finish_at_once_are_both_on_the_table() {
    use dphone_apps::{Completion, PbxStore, StoreOp, StoreReply, StoreSignal, StoreUser};
    use std::sync::atomic::{AtomicBool, Ordering};
    // Alice's phone reads the table to put her time in it; before its
    // write arrives, Bob's phone writes the table with his.  A store that
    // slips that write in:
    struct Racing {
        inner: FakeStore,
        other: FakeStore,
        raced: AtomicBool,
    }
    impl PbxStore for Racing {
        fn request(&self, op: StoreOp, done: Completion<StoreReply>) {
            if let StoreOp::Set { key, .. } = &op
                && !self.raced.swap(true, Ordering::SeqCst)
            {
                let mut table = Table::default();
                table.add(Win { ms: 90_000, when: 1_794_579_300 }, "102", "Bob");
                let wrote = self.other.run(StoreOp::Set {
                    app: APP.into(),
                    scope: StoreScope::Shared,
                    key: key.clone(),
                    value: table.to_bytes(),
                    expect: Some(0),
                });
                assert!(matches!(wrote, StoreReply::Stored(_)));
            }
            self.inner.request(op, done);
        }
        fn user(&self) -> Option<StoreUser> {
            self.inner.user()
        }
        fn online(&self) -> bool {
            self.inner.online()
        }
        fn watch(&self, apps: &[String]) {
            self.inner.watch(apps)
        }
        fn subscribe(&self, f: Box<dyn Fn(StoreSignal) + Send + Sync>) {
            self.inner.subscribe(f)
        }
    }
    listen();
    let pbx = FakeStore::default();
    let alice_side = pbx.phone("101", "Alice");
    let racing = Racing {
        inner: alice_side.clone(),
        other: pbx.phone("102", "Bob"),
        raced: false.into(),
    };
    let mut alice = Rig::with_services(|mut services| {
        services.store = std::sync::Arc::new(racing);
        services
    });
    put_puzzle(&alice, &almost(7, Difficulty::Easy, 5, 83_000));
    solve(&mut alice);
    // Both are there: Alice's phone was shown Bob's table, and wrote
    // hers over that one.
    let easy = Kind::new(7, Difficulty::Easy);
    assert_eq!(
        rows(&table(&pbx, easy)),
        [("101", "Alice", 83), ("102", "Bob", 90)]
    );
    let writes: Vec<Option<u64>> = alice_side
        .ops()
        .into_iter()
        .filter_map(|op| match op {
            StoreOp::Set { expect, .. } => Some(expect),
            _ => None,
        })
        .collect();
    assert_eq!(writes.len(), 2, "one refused write and one over Bob's");
    assert_eq!(writes[0], Some(0));
    assert!(writes[1].is_some_and(|v| v > 0));
    assert_eq!(waiting(&alice), "");
    alice.softkey(APP, "Scores");
    assert_eq!(listed(&alice).len(), 2);

    // The same from both sides, with two phones and their apps: each
    // solves, and each table has both, in the order of the times.
    let pbx = FakeStore::default();
    let mut carol = rig_on(
        pbx.phone("103", "Carol"),
        &almost(14, Difficulty::Medium, 3, 900_000),
    );
    let mut dan = rig_on(
        pbx.phone("104", "Dan"),
        &almost(14, Difficulty::Medium, 4, 800_000),
    );
    carol.launch(APP);
    dan.launch(APP);
    carol.ui.press(APP, Key::Digit(1));
    dan.ui.press(APP, Key::Digit(1));
    for _ in 0..4 {
        carol.mgr.pump();
        dan.mgr.pump();
    }
    carol.settle();
    dan.settle();
    assert_eq!(
        rows(&table(&pbx, Kind::new(14, Difficulty::Medium))),
        [("104", "Dan", 800), ("103", "Carol", 900)]
    );
}

#[test]
fn a_time_written_without_an_answer_is_on_the_table_once() {
    use dphone_apps::{Completion, PbxStore, StoreOp, StoreReply, StoreSignal, StoreUser};
    use std::sync::atomic::{AtomicBool, Ordering};
    // The PBX does the phone's first write, and its answer does not come:
    // to the phone, the write ends out of reach.
    struct Unanswered {
        inner: FakeStore,
        lost: AtomicBool,
    }
    impl PbxStore for Unanswered {
        fn request(&self, op: StoreOp, done: Completion<StoreReply>) {
            if matches!(op, StoreOp::Set { .. }) && !self.lost.swap(true, Ordering::SeqCst) {
                assert!(matches!(self.inner.run(op), StoreReply::Stored(_)));
                done.complete(StoreReply::Unreachable("no answer".into()));
                return;
            }
            self.inner.request(op, done);
        }
        fn user(&self) -> Option<StoreUser> {
            self.inner.user()
        }
        fn online(&self) -> bool {
            self.inner.online()
        }
        fn watch(&self, apps: &[String]) {
            self.inner.watch(apps)
        }
        fn subscribe(&self, f: Box<dyn Fn(StoreSignal) + Send + Sync>) {
            self.inner.subscribe(f)
        }
    }
    listen();
    let pbx = FakeStore::default();
    let alice_side = pbx.phone("101", "Alice");
    let store = Unanswered {
        inner: alice_side.clone(),
        lost: false.into(),
    };
    let (config, clock) = clock_at_t();
    let mut alice = Rig::with_services(|mut services| {
        services.store = std::sync::Arc::new(store);
        services.config = std::sync::Arc::new(config);
        services
    });
    put_puzzle(&alice, &almost(7, Difficulty::Easy, 5, 83_000));
    solve(&mut alice);
    let easy = Kind::new(7, Difficulty::Easy);
    assert_eq!(rows(&table(&pbx, easy)), [("101", "Alice", 83)]);
    // The row has when the puzzle was solved.
    let first = table(&pbx, easy).rows()[0].clone();
    assert_eq!(first.when, T);
    // The win waits, as one the PBX does not have, with when it was
    // solved.
    assert_eq!(waiting(&alice), format!("7x7-easy {} {T}\n", first.ms));
    // When the app next starts, an hour later, the table has that win
    // already (the same time, solved at the same moment): nothing is
    // written, and nothing waits.
    alice.softkey(APP, "Exit");
    assert!(!alice.mgr.is_running(APP));
    clock.store(T + 3600, Ordering::Relaxed);
    alice.launch(APP);
    assert_eq!(rows(&table(&pbx, easy)), [("101", "Alice", 83)]);
    assert_eq!(table(&pbx, easy).rows()[0], first);
    // (As the PBX has it, not as a table reads it.)
    let value = pbx.value(APP, StoreScope::Shared, &easy.key()).unwrap();
    assert_eq!(String::from_utf8(value).unwrap().lines().count(), 1);
    assert_eq!(waiting(&alice), "");
    let writes = |side: &FakeStore| {
        let ops = side.ops();
        ops.iter().filter(|op| matches!(op, StoreOp::Set { .. })).count()
    };
    assert_eq!(writes(&alice_side), 1);
    // Her next win of that kind is a row of its own, with when it was
    // solved.
    alice.softkey(APP, "Exit");
    put_puzzle(&alice, &almost(7, Difficulty::Easy, 6, 90_000));
    solve(&mut alice);
    assert_eq!(
        rows(&table(&pbx, easy)),
        [("101", "Alice", 83), ("101", "Alice", 90)]
    );
    assert_eq!(table(&pbx, easy).rows()[1].when, T + 3600);
    assert_eq!(writes(&alice_side), 2);
    assert_eq!(waiting(&alice), "");
}

#[test]
fn two_wins_in_the_very_same_time_are_two_rows() {
    // Two wins of Alice's of one kind, in the very same time, an hour
    // apart, wait on her phone.  The PBX has the first already: it was
    // written, and the answer did not come.
    let pbx = FakeStore::default();
    let easy = Kind::new(7, Difficulty::Easy);
    let mut first = Table::default();
    first.add(Win { ms: 83_000, when: T }, "101", "Alice");
    pbx.phone("101", "Alice").run(dphone_apps::StoreOp::Set {
        app: APP.into(),
        scope: StoreScope::Shared,
        key: easy.key(),
        value: first.to_bytes(),
        expect: None,
    });
    let mut alice = rig_on(pbx.phone("101", "Alice"), "");
    let times = format!("7x7-easy 83000 {T}\n7x7-easy 83000 {}\n", T + 3600);
    std::fs::write(storage(&alice.nvdata).join("times"), times).unwrap();
    // The app starts: the first is not written again, and the second is
    // a row of its own.
    alice.launch(APP);
    assert_eq!(
        rows(&table(&pbx, easy)),
        [("101", "Alice", 83), ("101", "Alice", 83)]
    );
    let when: Vec<u64> = table(&pbx, easy).rows().iter().map(|r| r.when).collect();
    assert_eq!(when, [T, T + 3600]);
    assert_eq!(waiting(&alice), "");
    // Each line says when.
    alice.softkey(APP, "Scores");
    assert_eq!(
        lines(&alice),
        [
            ["1. Alice", T_SHOWN, "1:23"],
            ["2. Alice", "Nov 13, 2026 3:15 PM", "1:23"]
        ]
    );
}

#[test]
fn a_time_set_out_of_reach_is_kept_and_written_when_the_store_is_in_reach() {
    let pbx = FakeStore::default();
    let easy = Kind::new(7, Difficulty::Easy);
    let mut alice = rig_on(
        pbx.phone("101", "Alice"),
        &almost(7, Difficulty::Easy, 5, 83_000),
    );
    // Bob's time is on the PBX, which Alice's phone cannot reach.
    let mut theirs = Table::default();
    theirs.add(Win { ms: 70_000, when: 1_794_579_300 }, "102", "Bob");
    pbx.phone("102", "Bob").run(dphone_apps::StoreOp::Set {
        app: APP.into(),
        scope: StoreScope::Shared,
        key: easy.key(),
        value: theirs.to_bytes(),
        expect: None,
    });
    alice.app_data.set_online(false);
    solve(&mut alice);
    let time = alice.ui.texts(APP)[1].strip_prefix("Time ").unwrap().to_string();
    assert_eq!(seconds(&time), 83);
    assert_eq!(rows(&table(&pbx, easy)), [("102", "Bob", 70)]);
    // The time waits on the phone.
    let kept = waiting(&alice);
    assert!(kept.starts_with("7x7-easy 83") && kept.ends_with('\n'), "{kept}");
    // The table says so, and has the time on this phone.
    alice.softkey(APP, "Scores");
    assert_eq!(alice.ui.texts(APP)[0], "Offline: 7x7 Easy");
    assert_eq!(lines(&alice), [["1. Alice", T_SHOWN, time.as_str()]]);
    alice.softkey(APP, "Size");
    assert_eq!(
        alice.ui.texts(APP),
        ["Offline: 10x10 Easy", "The PBX cannot be reached."]
    );
    alice.softkey(APP, "Back");
    assert_eq!(alice.ui.texts(APP), ["Solved!", &format!("Time {time}")]);
    alice.softkey(APP, "Scores");

    // The PBX is in reach again: the time is written, with the times
    // that were there, and the table on the screen is the PBX's.
    alice.app_data.set_online(true);
    alice.settle();
    assert_eq!(
        rows(&table(&pbx, easy)),
        [("102", "Bob", 70), ("101", "Alice", 83)]
    );
    assert_eq!(waiting(&alice), "");
    assert!(!storage(&alice.nvdata).join("times").exists());
    assert_eq!(alice.ui.texts(APP)[0], "Best times: 7x7 Easy");
    assert_eq!(
        listed(&alice),
        [
            ("1. Bob".to_string(), "1:10".to_string()),
            ("2. Alice".to_string(), time.clone())
        ]
    );

    // Out of reach again, two puzzles of one kind and one of another are
    // solved: each time waits.  The app is left, and the PBX comes in
    // reach while it does not run.
    alice.softkey(APP, "Back");
    alice.softkey(APP, "Exit");
    alice.app_data.set_online(false);
    let mut times = Vec::new();
    for (side, seed, elapsed) in [(7, 6, 60_000), (7, 7, 95_000), (10, 8, 400_000)] {
        put_puzzle(&alice, &almost(side, Difficulty::Easy, seed, elapsed));
        solve(&mut alice);
        times.push(alice.ui.texts(APP)[1].strip_prefix("Time ").unwrap().to_string());
        if seed == 7 {
            // The table on this phone has both times that wait.
            alice.softkey(APP, "Scores");
            assert_eq!(alice.ui.texts(APP)[0], "Offline: 7x7 Easy");
            assert_eq!(
                listed(&alice),
                [
                    ("1. Alice".to_string(), times[0].clone()),
                    ("2. Alice".to_string(), times[1].clone())
                ]
            );
            alice.softkey(APP, "Back");
        }
        alice.softkey(APP, "Exit");
        assert!(!alice.mgr.is_running(APP));
    }
    let kept = waiting(&alice);
    let lines: Vec<&str> = kept.lines().collect();
    assert_eq!(lines.len(), 3, "{kept}");
    assert!(lines[0].starts_with("7x7-easy 60"), "{kept}");
    assert!(lines[1].starts_with("7x7-easy 95"), "{kept}");
    assert!(lines[2].starts_with("10x10-easy 400"), "{kept}");
    alice.app_data.set_online(true);
    alice.settle();
    assert_eq!(table(&pbx, easy).rows().len(), 2);
    assert_eq!(waiting(&alice), kept);
    // When the app next starts, it writes them.
    alice.launch(APP);
    assert_eq!(
        rows(&table(&pbx, easy)),
        [
            ("101", "Alice", 60),
            ("102", "Bob", 70),
            ("101", "Alice", 83),
            ("101", "Alice", 95)
        ]
    );
    assert_eq!(
        rows(&table(&pbx, Kind::new(10, Difficulty::Easy))),
        [("101", "Alice", 400)]
    );
    assert_eq!(waiting(&alice), "");
}

#[test]
fn a_phone_without_a_pbx_store_plays_as_ever_and_keeps_its_times() {
    listen();
    let mut r = Rig::with_services(|mut services| {
        services.store = std::sync::Arc::new(dphone_apps::NoStore);
        services
    });
    put_puzzle(&r, &almost(7, Difficulty::Easy, 5, 83_000));
    r.launch(APP);
    assert_eq!(beside(&r), PLAYING);
    // The keys do what they do: a mark, off again, and the last light.
    let (before, _) = state(&mut r);
    r.key(APP, Key::Digit(3));
    assert_ne!(state(&mut r).0, before);
    r.key(APP, Key::Digit(3));
    assert_eq!(state(&mut r).0, before);
    r.key(APP, Key::Digit(1));
    let texts = r.ui.texts(APP);
    assert_eq!(texts[0], "Solved!");
    assert_eq!(seconds(texts[1].strip_prefix("Time ").unwrap()), 83);
    assert!(waiting(&r).starts_with("7x7-easy 83"));
    // The table of this phone's times; it does not know its user's name.
    r.softkey(APP, "Scores");
    assert_eq!(r.ui.texts(APP)[0], "Offline: 7x7 Easy");
    assert_eq!(listed(&r), [("1. You".to_string(), "1:23".to_string())]);
    r.softkey(APP, "Level");
    assert_eq!(
        r.ui.texts(APP),
        ["Offline: 7x7 Medium", "The PBX cannot be reached."]
    );
    r.softkey(APP, "Back");
    assert_eq!(r.ui.texts(APP), texts);
    // The time is still there when the app runs again, and the next one
    // waits beside it.
    r.softkey(APP, "Exit");
    put_puzzle(&r, &almost(7, Difficulty::Easy, 6, 50_000));
    solve(&mut r);
    let kept = waiting(&r);
    let lines: Vec<&str> = kept.lines().collect();
    assert_eq!(lines.len(), 2, "{kept}");
    assert!(lines[0].starts_with("7x7-easy 83") && lines[1].starts_with("7x7-easy 50"));
    r.softkey(APP, "Scores");
    assert_eq!(
        listed(&r),
        [
            ("1. You".to_string(), "0:50".to_string()),
            ("2. You".to_string(), "1:23".to_string())
        ]
    );
    r.softkey(APP, "Back");
    r.softkey(APP, "New");
    assert_eq!(r.ui.texts(APP), ["New puzzle"]);
    assert!(r.mgr.is_running(APP));
}

#[test]
fn the_time_and_the_best_times_are_drawn_on_every_kind_of_display() {
    let kind = Kind::new(10, Difficulty::Medium);
    let players = [
        ("107", "Priya Raman"),
        ("101", "Alice Okafor"),
        ("112", "Bartholomew Featherstonehaugh"),
        ("104", "Dana Whitfield"),
        ("103", "Chen Wei"),
        ("110", "Front Desk"),
        ("105", "Eli Navarro"),
        ("121", "Sam"),
        ("118", "Marta Kowalczyk"),
        ("109", "Jo Lindqvist"),
    ];
    for model in [Model::D60, Model::D70, Model::D40] {
        let pbx = FakeStore::default();
        let mut table = Table::default();
        for (n, (id, name)) in players.iter().enumerate() {
            // Solved on days and at hours of all widths.
            let ms = 71_000 + 43_210 * (n * n) as u64;
            table.add(Win { ms, when: 1_794_579_300 - 190_000 * n as u64 }, id, name);
        }
        pbx.run(dphone_apps::StoreOp::Set {
            app: APP.into(),
            scope: StoreScope::Shared,
            key: kind.key(),
            value: table.to_bytes(),
            expect: None,
        });
        // Dana's phone, with a puzzle of that kind.
        let mine = pbx.phone("104", "Dana Whitfield");
        let puzzle = puzzle_file(10, Difficulty::Medium, 31, "play");
        let mut p = Phone::on_pbx(model, &puzzle, &mine);
        let area = model.app_content_area();
        let board = Layout::new(area.width, area.height, 10).board_side as usize;
        // The time goes on: every second the display is sent its new
        // digits, beside the board, and nothing else.
        p.steady();
        let (before, masked) = (p.frame().0, p.pixels().0);
        p.ticked = None;
        p.steady();
        let (now, width) = p.frame();
        let differ: Vec<usize> = (0..now.len()).filter(|i| now[*i] != before[*i]).collect();
        assert!(!differ.is_empty() && differ.len() < 400, "{model:?}");
        assert!(differ.iter().all(|i| i % width > board), "{model:?}");
        assert!(differ.iter().all(|i| p.in_time_box(*i, width)), "{model:?}");
        let height = differ[differ.len() - 1] / width - differ[0] / width + 1;
        assert!(height <= 25, "{model:?}: {height}");
        // (The tests that compare pictures leave that place out.)
        assert_eq!(p.pixels().0, masked, "{model:?}");
        p.shot("timer");
        let playing = p.pixels().0;

        // The fourth softkey: the table of the puzzle's kind.
        p.tap(&[Key::Soft(4)]);
        p.shot("scores");
        let shown = p.pixels().0;
        assert_ne!(shown, playing, "{model:?}");
        // Down the list.
        p.tap(&[Key::Down, Key::Down, Key::Down, Key::Down, Key::Down, Key::Down]);
        p.shot("scores-end");
        assert_ne!(p.pixels().0, shown, "{model:?}");
        // The next size, which nobody has solved.
        p.tap(&[Key::Soft(1)]);
        p.shot("scores-none");
        let none = p.pixels().0;
        // The next difficulty, with the PBX out of reach.
        mine.set_online(false);
        p.tap(&[Key::Soft(2)]);
        p.shot("scores-offline");
        assert_ne!(p.pixels().0, none, "{model:?}");
        mine.set_online(true);
        p.settle();
        // Back, the third softkey: the puzzle and its time as they were.
        p.tap(&[Key::Soft(3)]);
        assert_eq!(p.pixels().0, playing, "{model:?}");
    }
}
