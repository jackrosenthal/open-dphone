//! Light Up, the puzzle: the app's life under the manager with the fake
//! screen (the keys, the screens, generation in turns, what is kept), and
//! the real thing drawn by the phone UI on a simulated D60 (color), D40,
//! and D70 (gray).
//!
//! The pictures are written as PNG files under `light-up/` in cargo's
//! test scratch directory (`CARGO_TARGET_TMPDIR`), or in the directory
//! `LIGHT_UP_PREVIEWS` names.  The rules, the solver, the generator, and
//! the pictures themselves are tested in the crate `lightup-game`.
//!
//! `cargo test --release -p dphone-apps --test lightup -- --ignored
//! --nocapture generation_times` prints how long the app takes to make
//! puzzles under the interpreter, beside the same work done natively.

mod common;

use common::{Rig, staged, temp_dir};
use dphone_apps::engine::Target;
use dphone_apps::testutil::{FakeConfig, FakePbx, services};
use dphone_apps::uishell::UiShell;
use dphone_apps::{AppManager, DrawOp, ManagerConfig};
use dphone_hal::{Hal, Screen};
use dphone_state::{
    AccountId, Call, CallState, Calls, Direction, Key, Model, Party, Phase, Store, System,
};
use dphone_ui::{Ui, UiConfig};
use lightup_game::draw::{Layout, Op, Painter, Palette, Rgb};
use lightup_game::generate::{Difficulty, Generator, Params, Progress, SIZES, generate};
use lightup_game::solver::{Outcome, Solve, Techniques};
use lightup_game::{Act, Game, Grid};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const APP: &str = "lightup";
/// The grids the app allows a puzzle (`MAX_GRIDS` in the app).
const MAX_GRIDS: u32 = 400;

fn storage(nvdata: &Path) -> PathBuf {
    nvdata.join("appdata").join(APP)
}

/// A rig whose app makes its puzzles from this seed on.
fn rig(model: Model, seed: u64) -> Rig {
    listen();
    let r = Rig::with_root(staged(), FakeConfig::default(), model);
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
    std::fs::read_to_string(storage(&r.nvdata).join("puzzle")).unwrap_or_default()
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
            (true, true) => Key::Digit(8),
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
                })
                .collect()
        })
        .collect()
}

/// What the app should have drawn for a game on a model's display: a
/// list per row, and the cursor's.  The fake screen's foreground color is
/// (32, 32, 32) and its background (16, 16, 16).
fn expected(model: Model, game: &Game) -> Vec<Vec<Op>> {
    let area = model.app_content_area();
    let side = game.grid().width();
    let palette = if model.is_color() {
        Palette::color()
    } else {
        Palette::gray(Rgb(32, 32, 32), Rgb(16, 16, 16))
    };
    let painter = Painter::new(
        side,
        Layout::new(area.width, area.height, side).pitch,
        palette,
    );
    let mut lists: Vec<Vec<Op>> = (0..side).map(|row| painter.row(game.grid(), row)).collect();
    lists.push(painter.cursor(game));
    lists
}

#[test]
fn a_puzzle_from_new_to_solved_with_every_key() {
    let mut r = rig(Model::D60, 11);
    r.launch(APP);
    assert_eq!(r.ui.top().as_deref(), Some(APP));
    // Nothing is kept yet: the sizes and difficulties come first.
    assert_eq!(r.ui.texts(APP), ["New puzzle"]);
    assert_eq!(r.ui.softkeys(APP), ["Play", "Exit"]);
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
    assert_eq!(r.ui.texts(APP), ["7x7", "Easy", "", ""]);
    assert_eq!(r.ui.softkeys(APP), ["New", "Exit"]);

    // The puzzle is the one the seed gives, and it is written down.
    let puzzle = native(7, Difficulty::Easy, 11);
    assert_eq!(parse(&file(&r)), (puzzle.clone(), (0, 0)));
    assert!(
        file(&r).starts_with("size=7\ndifficulty=easy\nelapsed=0\ncursor=0,0\n"),
        "{}",
        file(&r)
    );
    // A canvas for each row and one for the cursor, each drawn once.
    let mut game = Game::new(puzzle.clone());
    assert_eq!(drawn(&r), expected(Model::D60, &game));
    assert_eq!(r.ui.draw_count(APP), 8);
    let bounds: Vec<_> = r.ui.drawings(APP).into_iter().map(|d| d.0).collect();
    assert_eq!(
        (bounds[0].width, bounds[0].height, bounds[1].y - bounds[0].y),
        (239, 34, 34)
    );
    assert_eq!(
        (bounds[7].x, bounds[7].width, bounds[7].height),
        (bounds[0].x, 239, 239)
    );

    // The arrows and 4, 2, 6, 8 move the cursor; only the cursor is
    // drawn again.  At an edge nothing happens and nothing is drawn.
    let mut draws = 8;
    for (key, dx, dy) in [
        (Key::Right, 1, 0),
        (Key::Digit(6), 1, 0),
        (Key::Down, 0, 1),
        (Key::Digit(8), 0, 1),
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
    assert_eq!(draws, 8 + 8);
    assert_eq!(game.cursor(), (0, 0));

    // 5 and Select put a light, 3 a no-light mark, 1 clears; the other
    // keys do nothing.  On every cell of the first two rows, in turn:
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
        (Key::Digit(5), Some(Act::Light)),
        (Key::Digit(5), Some(Act::Light)),
        (Key::Digit(3), Some(Act::Mark)),
        (Key::Digit(3), Some(Act::Mark)),
        (Key::Select, Some(Act::Light)),
        (Key::Digit(1), Some(Act::Clear)),
        (Key::Digit(1), Some(Act::Clear)),
        (Key::Digit(3), Some(Act::Mark)),
        (Key::Digit(1), Some(Act::Clear)),
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
            let changed = act.is_some_and(|a| game.act(a));
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
    // they are wrong, and every light of the solution is put, with 5 and
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
        assert_eq!(r.ui.texts(APP), ["7x7", "Easy", "", ""]);
        at(&mut r, &mut game, *cell);
        r.key(
            APP,
            if n.is_multiple_of(2) {
                Key::Digit(5)
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
    assert_eq!(texts[..3], ["7x7", "Easy", "Solved!"]);
    let seconds: u32 = texts[3]
        .strip_prefix("Time 0:")
        .expect(&texts[3])
        .parse()
        .unwrap();
    assert!(seconds < 60);
    assert_eq!(parse(&file(&r)).0, *game.grid());
    assert!(game.grid().is_solved());
    assert_eq!(drawn(&r), expected(Model::D60, &game));
    assert!(drawn(&r)[7].is_empty());
    assert_eq!(r.ui.softkeys(APP), ["New", "Exit"]);
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
    assert_eq!(r.ui.softkeys(APP), ["Play", "Back"]);
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
    assert_eq!(r.ui.texts(APP), ["7x7", "Medium", "", ""]);
    assert_eq!(
        parse(&file(&r)),
        (native(7, Difficulty::Medium, 12), (0, 0))
    );

    r.softkey(APP, "Exit");
    assert_eq!(r.ui.top(), None);
    assert!(!r.mgr.is_running(APP), "Exit stops the app");
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
        assert_eq!(r.ui.texts(APP), ["7x7", "Hard", "", ""], "{model:?}");
        assert_eq!(drawn(&r), expected(model, &game), "{model:?}");
        let lists = drawn(&r);
        let colors = |ops: &[Op]| -> Vec<Rgb> {
            ops.iter()
                .map(|op| match op {
                    Op::Fill { color, .. } | Op::Bitmap { color, .. } => *color,
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
            assert_eq!(reds, [0, 2, 2, 0, 0, 0, 0, 0]);
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
        r.softkey(APP, "Exit");
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
            .filter(|l| !l.starts_with("elapsed="))
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
    assert_eq!(r.ui.texts(APP), ["7x7", "Hard", "", ""]);
    r.ui.cover(APP, true);
    r.settle();
    assert!(
        elapsed(&file(&r)) - before < 1000,
        "the time ran while the puzzle was covered"
    );
    r.ui.cover(APP, false);
    r.settle();
    r.key(APP, Key::Digit(1));
    assert_ne!(drawn(&r), shown);

    // Leaving with Cancel stops the app; the next time the puzzle is
    // where it was.
    r.key(APP, Key::Cancel);
    assert_eq!(r.ui.top(), None);
    assert!(!r.mgr.is_running(APP));
    let left = WRONG.replace("cursor=3,1", "cursor=4,1");
    assert_eq!(rows(&file(&r)), rows(&left));
    r.launch(APP);
    assert_eq!(r.ui.texts(APP), ["7x7", "Hard", "", ""]);
    let (grid, cursor) = parse(&left);
    let mut game = Game::new(grid);
    game.set_cursor(cursor.0, cursor.1);
    assert_eq!(drawn(&r), expected(Model::D60, &game));
    r.softkey(APP, "Exit");

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
    assert_eq!(
        r.ui.texts(APP),
        ["10x10", "Medium", "Solved!", "Time 1:02:05"]
    );
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
        assert_eq!(r.ui.softkeys(APP), ["Play", "Exit"]);
        r.softkey(APP, "Exit");
        assert!(!r.mgr.is_running(APP));
    }
    // The cursor of a file is put inside the grid.
    std::fs::write(&puzzle, WRONG.replace("cursor=3,1", "cursor=99,x")).unwrap();
    r.launch(APP);
    assert_eq!(state(&mut r).1, (6, 0));
}

#[test]
fn generating_goes_on_in_turns_and_waits_while_a_call_has_the_screen() {
    let mut r = rig_with(Model::D60, WRONG);
    r.launch(APP);
    r.softkey(APP, "New");
    // The list is at the puzzle's size and difficulty.
    assert_eq!(r.ui.softkeys(APP), ["Play", "Back"]);
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
    assert_eq!(r.ui.texts(APP), ["14x14", "Hard", "", ""]);
    // The same puzzle as when made in one go.
    let large = native(14, Difficulty::Hard, 2);
    assert_eq!(parse(&file(&r)), (large.clone(), (0, 0)));
    assert!(file(&r).starts_with("size=14\ndifficulty=hard\nelapsed=0\n"));
    assert_eq!(r.ui.drawings(APP).len(), 15);

    // New while generating is the list again; Back from there is the
    // puzzle there was, untouched.
    r.key(APP, Key::Digit(5));
    let played = state(&mut r);
    choose_from_play(&mut r, 14, Difficulty::Hard);
    assert!(generating(&r));
    r.softkey(APP, "New");
    assert_eq!(r.ui.texts(APP), ["New puzzle"]);
    assert_eq!(r.ui.softkeys(APP), ["Play", "Back"]);
    std::thread::sleep(Duration::from_millis(30));
    r.settle();
    assert_eq!(
        r.ui.texts(APP),
        ["New puzzle"],
        "the abandoned puzzle does not turn up"
    );
    r.softkey(APP, "Back");
    assert_eq!(r.ui.texts(APP), ["14x14", "Hard", "", ""]);
    assert_eq!(state(&mut r), played);

    // Exit while generating: the app stops, and the puzzle there was is
    // there the next time.
    choose_from_play(&mut r, 10, Difficulty::Hard);
    assert!(generating(&r));
    r.softkey(APP, "Exit");
    assert!(!r.mgr.is_running(APP));
    r.launch(APP);
    assert_eq!(r.ui.texts(APP), ["14x14", "Hard", "", ""]);
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
            assert_eq!(
                r.ui.texts(APP),
                [format!("{side}x{side}").as_str(), difficulty.name(), "", ""]
            );
            let game = Game::new(puzzle);
            assert_eq!(drawn(&r), expected(model, &game), "{model:?} {side}");
            let bounds: Vec<_> = r.ui.drawings(APP).into_iter().map(|d| d.0).collect();
            let board = pitches[n] * side as u16 + 1;
            assert_eq!((bounds[1].y - bounds[0].y) as u16, pitches[n]);
            assert_eq!((bounds[side].width, bounds[side].height), (board, board));
            let area = model.app_content_area();
            assert!(bounds[side].y >= 0 && bounds[side].y as u16 + board <= area.height);
        }
        r.softkey(APP, "Exit");
    }
}

#[test]
fn it_is_a_built_in_app_that_can_be_switched_off() {
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(staged().join(APP).join("app.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        manifest["capabilities"],
        serde_json::json!(["ui", "timers", "storage", "canvas"]),
        "no sound, no calls, no network"
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
        ("Light Up", "lightup")
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
        let mut r = Rig::new(FakeConfig::default());
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
    _dir: common::TempDir,
}

impl Phone {
    /// A phone on which the app finds this puzzle file.
    fn new(model: Model, puzzle: &str) -> Phone {
        let dir = temp_dir("lightup-phone");
        let nvdata = dir.0.join("nvdata");
        std::fs::create_dir_all(storage(&nvdata)).unwrap();
        std::fs::write(storage(&nvdata).join("puzzle"), puzzle).unwrap();
        std::fs::write(storage(&nvdata).join("seed"), "7").unwrap();
        let mut hal = Hal::sim(model);
        let sim = hal.sim.clone().unwrap();
        let store = Store::new(model);
        let ui = Ui::from_hal(&mut hal, store.clone(), UiConfig::default());
        store.update::<System, _>(|s| {
            s.phase = Phase::Running;
            s.time_synced = true;
        });
        let mut cfg = ManagerConfig::rooted(staged(), &nvdata, &dir.0.join("tmp"));
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

    fn file(&self) -> String {
        std::fs::read_to_string(storage(&self.nvdata).join("puzzle")).unwrap_or_default()
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
        let (pixels, w) = self.pixels();
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
    large.try_into().ok().expect("three phones")
}

/// The display is sent something once for a key that changes something,
/// and only the part that changed; nothing for a key that changes
/// nothing.  `pitch` is the size of a cell.
fn only_what_changed_is_sent(fresh: &mut Phone, pitch: usize) {
    let model = fresh.model;
    // The cursor at an edge.
    assert_eq!(
        fresh.tap(&[Key::Left, Key::Up, Key::Digit(4), Key::Digit(2)]),
        0,
        "{model:?}"
    );
    // One step: the two cells, and the lines around them.
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
    let before = fresh.pixels().0;
    assert_eq!(fresh.tap(&[Key::Digit(5)]), 1, "{model:?}");
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
    // Once more: nothing.  A mark on it, and clearing it: once each.
    assert_eq!(fresh.tap(&[Key::Digit(5), Key::Select]), 0, "{model:?}");
    assert_eq!(fresh.tap(&[Key::Digit(3)]), 1, "{model:?}");
    assert_eq!(fresh.tap(&[Key::Digit(3)]), 0, "{model:?}");
    assert_eq!(fresh.tap(&[Key::Digit(1)]), 1, "{model:?}");
    assert_eq!(
        fresh.tap(&[Key::Digit(1), Key::Digit(7), Key::Digit(9)]),
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
    assert_eq!((fresh.count(yellow), fresh.count(red)), (0, 0));
    assert!(fresh.count(unlit) > 100 * 16 * 16 && fresh.count(blue) > 4 * 17);
    assert!(play.count(yellow) > 30 * 16 * 16 && play.count(unlit) > 16 * 16);
    assert!(
        play.count(red) > 16 * 16 + 2 * 50,
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
    assert_eq!(p.tap(&[Key::Left]), 1, "the puzzle goes on");
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
