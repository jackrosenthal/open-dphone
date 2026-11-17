//! Box Pusher, the puzzle: the app's life under the manager with the fake
//! screen, and the real thing drawn by the phone UI on a simulated D60
//! (color), D40, and D70 (gray).
//!
//! The pictures are written as PNG files under `box-pusher/` in cargo's
//! test scratch directory (`CARGO_TARGET_TMPDIR`), or in the directory
//! `BOX_PUSHER_PREVIEWS` names.  The rules, the levels, and the solver
//! that proves them are tested in the crate `boxpusher-game` (`apps/boxpusher/game`).
//!
//! The progress is kept on the PBX too, whose store of app data is a
//! `FakeStore` here: the last tests before the pictures are of a second
//! phone, a PBX out of reach, and a phone and a PBX that differ.

mod common;

use common::{Rig, staged, temp_dir};
use dphone_apps::engine::Target;
use dphone_apps::testutil::{FakeConfig, FakePbx, FakeStore, local_value, services_with_store};
use dphone_apps::uishell::UiShell;
use dphone_apps::{AppManager, DrawOp, ManagerConfig, StoreOp, StoreReply, StoreScope};
use dphone_hal::{Hal, Screen};
use dphone_state::{AccountId, Call, CallState, Calls, Direction, Key, Model, Party, Phase, Store, System};
use dphone_ui::{Ui, UiConfig};
use std::path::{Path, PathBuf};

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

/// The value of one of the game's keys on the phone.
fn local(nvdata: &Path, key: &str) -> Option<String> {
    let value = local_value(&nvdata.join("appdata").join(APP), key)?;
    Some(String::from_utf8(value).unwrap())
}

fn progress(r: &Rig) -> String {
    local(&r.nvdata, "progress").unwrap_or_default()
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

    // A progress file that makes no sense is the beginning, on a phone
    // whose PBX is out of reach.
    r.app_data.set_online(false);
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
    r.softkey(APP, "Back");
    r.softkey(APP, "Exit");

    // With the PBX in reach, a progress file that was lost is the PBX's
    // progress, and the levels the phone has solved go to the PBX.
    r.app_data.set_online(true);
    std::fs::write(&file, "").unwrap();
    r.launch(APP);
    assert_eq!(panel(&r), ["Level 2", "Moves 2", "Pushes 1"]);
    std::fs::write(&file, "level=27\nsolved=28\nmoves=\n").unwrap();
    r.softkey(APP, "Exit");
    r.launch(APP);
    assert_eq!(panel(&r)[0], "Level 28");
    let on_pbx = r.app_data.value(APP, StoreScope::User, "progress").unwrap();
    assert_eq!(String::from_utf8(on_pbx).unwrap(), "level=27\nsolved=28\nmoves=\n");
}

#[test]
fn it_plays_on_the_gray_screens_in_black_and_white() {
    for (model, tile) in [(Model::D40, 14), (Model::D70, 20)] {
        let mut r = Rig::with_root(staged(), FakeConfig::default(), model);
        r.launch(APP);
        let drawn = r.ui.drawings(APP);
        assert_eq!((drawn[0].0.width, drawn[0].0.height), (11 * tile, 7 * tile), "{model:?}");
        // The fake screen's foreground and background colors, and no
        // others.
        let fg = dphone_apps::Color { r: 32, g: 32, b: 32 };
        let bg = dphone_apps::Color { r: 16, g: 16, b: 16 };
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

// ------------------------------------------------- progress on the PBX

const LEVEL_1: [Key; 3] = [Key::Right, Key::Right, Key::Right];
const LEVEL_2: [Key; 5] = [Key::Up, Key::Right, Key::Up, Key::Right, Key::Down];

/// A phone of `user` on the PBX of `pbx`, with nothing of the game on it.
fn phone(pbx: &FakeStore, user: &str) -> Rig {
    Rig::on_pbx(pbx.phone(user, user))
}

/// What the PBX has of a user's progress, read from another phone of the
/// user (one that is in reach when the user's is not).
fn on_pbx(pbx: &FakeStore, user: &str) -> Option<String> {
    let value = pbx.phone(user, user).value(APP, StoreScope::User, "progress");
    value.map(|v| String::from_utf8(v).unwrap())
}

/// Writes a user's progress on the PBX as another phone of the user
/// does; returns the entry's version.
fn write_on_pbx(pbx: &FakeStore, user: &str, progress: &str) -> u64 {
    let reply = pbx.phone(user, user).run(StoreOp::Set {
        app: APP.into(),
        scope: StoreScope::User,
        key: "progress".into(),
        value: progress.as_bytes().to_vec(),
        expect: None,
    });
    match reply {
        StoreReply::Stored(entry) => entry.version,
        other => panic!("{other:?}"),
    }
}

fn storage(r: &Rig) -> PathBuf {
    r.nvdata.join("appdata").join(APP)
}

/// Puts a file in the game's storage on the phone.
fn put(r: &Rig, key: &str, value: &str) {
    std::fs::create_dir_all(storage(r)).unwrap();
    std::fs::write(storage(r).join(key), value).unwrap();
}

/// The game's storage of one phone, copied to another: the same phone
/// with another user provisioned.
fn carry(from: &Rig, to: &Rig) {
    std::fs::create_dir_all(storage(to)).unwrap();
    for file in std::fs::read_dir(storage(from)).unwrap() {
        let file = file.unwrap();
        std::fs::copy(file.path(), storage(to).join(file.file_name())).unwrap();
    }
}

/// The requests to write that the phone made.
fn writes(r: &Rig) -> usize {
    let ops = r.app_data.ops();
    ops.iter().filter(|op| matches!(op, StoreOp::Set { .. })).count()
}

#[test]
fn a_reset_phone_and_a_second_phone_go_on_where_the_user_was() {
    let pbx = FakeStore::default();
    let mut first = phone(&pbx, "101");
    first.launch(APP);
    // Nothing is written for a game that has not begun.
    assert_eq!(on_pbx(&pbx, "101"), None);
    keys(&mut first, &LEVEL_1);
    assert_eq!(on_pbx(&pbx, "101").as_deref(), Some("level=1\nsolved=1\nmoves=\n"));
    first.softkey(APP, "Next");
    keys(&mut first, &[Key::Up, Key::Right]);
    // The steps in a level go when the game leaves the screen.
    assert_eq!(on_pbx(&pbx, "101").as_deref(), Some("level=1\nsolved=1\nmoves=\n"));
    first.softkey(APP, "Exit");
    assert_eq!(on_pbx(&pbx, "101").as_deref(), Some("level=1\nsolved=1\nmoves=ur\n"));

    // The phone after a reset, or another phone of the user: nothing of
    // the game is on it.
    let mut second = phone(&pbx, "101");
    second.launch(APP);
    assert_eq!(panel(&second), ["Level 2", "Moves 2", "Pushes 1"]);
    assert_eq!(progress(&second), "level=1\nsolved=1\nmoves=ur\n");
    assert_eq!(writes(&second), 0, "the PBX has it already");
    // The steps can be undone there too.
    second.softkey(APP, "Undo");
    assert_eq!(panel(&second), ["Level 2", "Moves 1", "Pushes 1"]);
    second.softkey(APP, "Levels");
    assert_eq!(second.ui.items(APP).len(), 2);
    second.softkey(APP, "Back");
    // The user leaves the second phone a step earlier in the level than
    // the first, and finds the first there too: what the first sent as
    // the game was left counts as arrived.
    second.softkey(APP, "Exit");
    assert_eq!(on_pbx(&pbx, "101").as_deref(), Some("level=1\nsolved=1\nmoves=u\n"));
    first.launch(APP);
    assert_eq!(panel(&first), ["Level 2", "Moves 1", "Pushes 1"]);
    first.softkey(APP, "Exit");
    second.launch(APP);
    assert_eq!(panel(&second), ["Level 2", "Moves 1", "Pushes 1"]);

    // Another user starts at the beginning, and has keys of their own.
    let mut other = phone(&pbx, "102");
    other.launch(APP);
    assert_eq!(panel(&other), ["Level 1", "Moves 0", "Pushes 0"]);
    other.softkey(APP, "Exit");
    assert_eq!(on_pbx(&pbx, "102"), None);

    // The user goes on at the second phone, and then at the first.
    keys(&mut second, &[Key::Right, Key::Up, Key::Right, Key::Down]);
    assert_eq!(panel(&second)[0], "Level 2 solved!");
    second.softkey(APP, "Exit");
    assert_eq!(on_pbx(&pbx, "101").as_deref(), Some("level=2\nsolved=2\nmoves=\n"));
    first.launch(APP);
    assert_eq!(panel(&first), ["Level 3", "Moves 0", "Pushes 0"]);
    assert_eq!(progress(&first), "level=2\nsolved=2\nmoves=\n");
    assert_eq!(on_pbx(&pbx, "102"), None);
}

#[test]
fn a_game_left_running_takes_up_what_was_played_on_another_phone() {
    let pbx = FakeStore::default();
    let mut r = phone(&pbx, "101");
    r.launch(APP);
    r.key(APP, Key::Right);
    // A call's screen is over the game, and meanwhile the user solves
    // five levels at another phone.
    r.ui.cover(APP, true);
    r.settle();
    assert_eq!(on_pbx(&pbx, "101").as_deref(), Some("level=0\nsolved=0\nmoves=r\n"));
    write_on_pbx(&pbx, "101", "level=5\nsolved=5\nmoves=\n");
    r.ui.cover(APP, false);
    r.settle();
    assert_eq!(panel(&r), ["Level 6", "Moves 0", "Pushes 0"]);
    assert_eq!(progress(&r), "level=5\nsolved=5\nmoves=\n");
    assert_eq!(on_pbx(&pbx, "101").as_deref(), Some("level=5\nsolved=5\nmoves=\n"));
}

#[test]
fn what_is_played_out_of_reach_goes_to_the_pbx_when_it_is_in_reach() {
    let pbx = FakeStore::default();
    let mut r = phone(&pbx, "101");
    r.app_data.set_online(false);
    r.launch(APP);
    keys(&mut r, &LEVEL_1);
    r.softkey(APP, "Next");
    r.key(APP, Key::Up);
    // The game goes on as ever, and keeps its progress on the phone.
    assert_eq!(panel(&r), ["Level 2", "Moves 1", "Pushes 1"]);
    assert_eq!(progress(&r), "level=1\nsolved=1\nmoves=\n");
    assert_eq!(writes(&r), 0, "nothing is asked of a PBX out of reach");

    // The PBX comes into reach while the game is on screen.
    r.app_data.set_online(true);
    r.settle();
    assert_eq!(on_pbx(&pbx, "101").as_deref(), Some("level=1\nsolved=1\nmoves=u\n"));
    assert_eq!(panel(&r), ["Level 2", "Moves 1", "Pushes 1"]);

    // Out of reach again until after the game was left: the progress
    // goes the next time the game is started.
    r.app_data.set_online(false);
    keys(&mut r, &[Key::Right, Key::Up, Key::Right, Key::Down]);
    assert_eq!(panel(&r)[0], "Level 2 solved!");
    r.softkey(APP, "Exit");
    assert_eq!(progress(&r), "level=2\nsolved=2\nmoves=\n");
    r.app_data.set_online(true);
    r.settle();
    assert_eq!(on_pbx(&pbx, "101").as_deref(), Some("level=1\nsolved=1\nmoves=u\n"));
    r.launch(APP);
    assert_eq!(panel(&r), ["Level 3", "Moves 0", "Pushes 0"]);
    assert_eq!(on_pbx(&pbx, "101").as_deref(), Some("level=2\nsolved=2\nmoves=\n"));
}

#[test]
fn a_phone_whose_pbx_keeps_no_app_data_plays_and_keeps_its_progress() {
    let mut r = Rig::with_services(|mut services| {
        services.store = std::sync::Arc::new(dphone_apps::NoStore);
        services
    });
    r.launch(APP);
    keys(&mut r, &LEVEL_1);
    assert_eq!(panel(&r)[0], "Level 1 solved!");
    r.softkey(APP, "Next");
    keys(&mut r, &LEVEL_2);
    assert_eq!(panel(&r)[0], "Level 2 solved!");
    r.softkey(APP, "Levels");
    assert_eq!(r.ui.items(APP).len(), 3);
    r.softkey(APP, "Back");
    r.key(APP, Key::Down);
    r.softkey(APP, "Exit");
    assert_eq!(progress(&r), "level=2\nsolved=2\nmoves=d\n");
    r.launch(APP);
    assert_eq!(panel(&r), ["Level 3", "Moves 1", "Pushes 0"]);
}

/// A phone of the user 101 on which the game has `progress`, and that
/// last knew the PBX to have `known`; meanwhile the PBX has `theirs`.
/// The game is started.
fn differing(progress: &str, known: Option<&str>, theirs: Option<&str>) -> (FakeStore, Rig) {
    let pbx = FakeStore::default();
    let r = phone(&pbx, "101");
    put(&r, "progress", progress);
    if let Some(known) = known {
        let version = write_on_pbx(&pbx, "101", known);
        put(&r, "pbx", &format!("user=101\nversion={version}\n{known}"));
    }
    if let Some(theirs) = theirs {
        write_on_pbx(&pbx, "101", theirs);
    }
    let mut r = r;
    r.launch(APP);
    (pbx, r)
}

#[test]
fn a_phone_and_a_pbx_that_differ_agree_on_the_right_progress() {
    let was = "level=1\nsolved=1\nmoves=\n";
    let mine = "level=3\nsolved=3\nmoves=\n";

    // The phone played out of reach, and the PBX has what it had: the
    // phone's progress goes to the PBX.
    let (pbx, r) = differing(mine, Some(was), None);
    assert_eq!(panel(&r)[0], "Level 4");
    assert_eq!(on_pbx(&pbx, "101").as_deref(), Some(mine));

    // The phone has nothing new, and the PBX has: the PBX's progress is
    // gone on with, even to an earlier level, and no level closes.
    let (pbx, r) = differing(mine, Some(mine), Some("level=5\nsolved=5\nmoves=\n"));
    assert_eq!(panel(&r)[0], "Level 6");
    assert_eq!(progress(&r), "level=5\nsolved=5\nmoves=\n");
    assert_eq!(writes(&r), 0);
    assert_eq!(on_pbx(&pbx, "101").as_deref(), Some("level=5\nsolved=5\nmoves=\n"));
    let (pbx, mut r) = differing(mine, Some(mine), Some("level=0\nsolved=2\nmoves=r\n"));
    assert_eq!(panel(&r), ["Level 1", "Moves 1", "Pushes 0"]);
    r.softkey(APP, "Levels");
    assert_eq!(r.ui.items(APP).len(), 4);
    assert_eq!(on_pbx(&pbx, "101").as_deref(), Some("level=0\nsolved=3\nmoves=r\n"));

    // Both have something new.  The PBX has more levels solved: the
    // PBX's.
    let (pbx, r) = differing(mine, Some(was), Some("level=4\nsolved=7\nmoves=\n"));
    assert_eq!(panel(&r)[0], "Level 5");
    assert_eq!(progress(&r), "level=4\nsolved=7\nmoves=\n");
    assert_eq!(on_pbx(&pbx, "101").as_deref(), Some("level=4\nsolved=7\nmoves=\n"));
    // The phone has more, or as many: the phone's.
    for theirs in ["level=2\nsolved=2\nmoves=u\n", "level=0\nsolved=3\nmoves=r\n"] {
        let (pbx, r) = differing(mine, Some(was), Some(theirs));
        assert_eq!(panel(&r), ["Level 4", "Moves 0", "Pushes 0"], "{theirs:?}");
        assert_eq!(on_pbx(&pbx, "101").as_deref(), Some(mine));
    }

    // A phone from before the progress went to the PBX: the same, with
    // a PBX that has nothing yet, less, or more.
    let (pbx, r) = differing(mine, None, None);
    assert_eq!(panel(&r)[0], "Level 4");
    assert_eq!(on_pbx(&pbx, "101").as_deref(), Some(mine));
    let (pbx, r) = differing(mine, None, Some(was));
    assert_eq!(panel(&r)[0], "Level 4");
    assert_eq!(on_pbx(&pbx, "101").as_deref(), Some(mine));
    let (pbx, r) = differing(mine, None, Some("level=9\nsolved=9\nmoves=\n"));
    assert_eq!(panel(&r)[0], "Level 10");
    assert_eq!(on_pbx(&pbx, "101").as_deref(), Some("level=9\nsolved=9\nmoves=\n"));

    // The PBX lost what it had: it gets the phone's again.
    let (pbx, r) = differing(mine, None, None);
    put(&r, "pbx", &format!("user=101\nversion=77\n{mine}"));
    let reply = pbx.phone("101", "101").run(StoreOp::Remove {
        app: APP.into(),
        scope: StoreScope::User,
        key: "progress".into(),
        expect: None,
    });
    assert!(matches!(reply, StoreReply::Removed), "{reply:?}");
    let mut r = r;
    r.softkey(APP, "Exit");
    r.launch(APP);
    assert_eq!(panel(&r)[0], "Level 4");
    assert_eq!(on_pbx(&pbx, "101").as_deref(), Some(mine));

    // A value on the PBX that makes no sense closes no level.
    let (pbx, r) = differing(mine, Some(mine), Some("\u{0}nonsense"));
    assert_eq!(panel(&r)[0], "Level 1");
    assert_eq!(on_pbx(&pbx, "101").as_deref(), Some("level=0\nsolved=3\nmoves=\n"));
}

#[test]
fn a_player_who_is_playing_is_not_moved_to_another_level() {
    // The PBX answers after the player's first step (here: it comes into
    // reach then).  It has more levels solved.
    let pbx = FakeStore::default();
    write_on_pbx(&pbx, "101", "level=5\nsolved=5\nmoves=\n");
    let mut r = phone(&pbx, "101");
    r.app_data.set_online(false);
    r.launch(APP);
    r.key(APP, Key::Right);
    r.app_data.set_online(true);
    r.settle();
    assert_eq!(panel(&r), ["Level 1", "Moves 1", "Pushes 0"]);
    // The levels are open all the same, and the PBX is told where the
    // player is.
    r.softkey(APP, "Levels");
    assert_eq!(r.ui.items(APP).len(), 6);
    assert_eq!(on_pbx(&pbx, "101").as_deref(), Some("level=0\nsolved=5\nmoves=r\n"));

    // The same with the list of levels on screen: it is the longer list.
    let pbx = FakeStore::default();
    write_on_pbx(&pbx, "101", "level=5\nsolved=5\nmoves=\n");
    let mut r = phone(&pbx, "101");
    r.app_data.set_online(false);
    r.launch(APP);
    r.softkey(APP, "Levels");
    assert_eq!(r.ui.items(APP).len(), 1);
    r.app_data.set_online(true);
    r.settle();
    assert_eq!(r.ui.items(APP).len(), 6);
    assert_eq!(r.ui.softkeys(APP), ["Play", "Back"]);
}

#[test]
fn a_phone_that_gets_another_user_keeps_each_users_progress_apart() {
    let pbx = FakeStore::default();
    let mut desk = phone(&pbx, "101");
    desk.launch(APP);
    keys(&mut desk, &LEVEL_1);
    desk.softkey(APP, "Next");
    // The PBX goes out of reach, and 101 solves another level.
    desk.app_data.set_online(false);
    keys(&mut desk, &LEVEL_2);
    desk.softkey(APP, "Exit");
    assert_eq!(on_pbx(&pbx, "101").as_deref(), Some("level=1\nsolved=1\nmoves=\n"));

    // The phone is provisioned for 102, who has not played.
    let mut desk_102 = phone(&pbx, "102");
    carry(&desk, &desk_102);
    desk_102.launch(APP);
    assert_eq!(panel(&desk_102), ["Level 1", "Moves 0", "Pushes 0"]);
    desk_102.softkey(APP, "Levels");
    assert_eq!(desk_102.ui.items(APP).len(), 1);
    desk_102.softkey(APP, "Back");
    keys(&mut desk_102, &LEVEL_1);
    desk_102.softkey(APP, "Exit");
    assert_eq!(on_pbx(&pbx, "102").as_deref(), Some("level=1\nsolved=1\nmoves=\n"));
    // Neither user's progress went to the other's keys.
    assert_eq!(on_pbx(&pbx, "101").as_deref(), Some("level=1\nsolved=1\nmoves=\n"));

    // 101 is back at the phone: the level solved out of reach is there,
    // and goes to the PBX.
    let mut desk_101 = phone(&pbx, "101");
    carry(&desk_102, &desk_101);
    desk_101.launch(APP);
    assert_eq!(panel(&desk_101), ["Level 3", "Moves 0", "Pushes 0"]);
    assert_eq!(on_pbx(&pbx, "101").as_deref(), Some("level=2\nsolved=2\nmoves=\n"));
    assert_eq!(on_pbx(&pbx, "102").as_deref(), Some("level=1\nsolved=1\nmoves=\n"));
    // Nothing is set aside any longer.
    let mut files: Vec<String> = std::fs::read_dir(storage(&desk_101))
        .unwrap()
        .map(|f| f.unwrap().file_name().into_string().unwrap())
        .filter(|name| !name.starts_with('.'))
        .collect();
    files.sort();
    assert_eq!(files, ["pbx", "progress"]);
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
            services_with_store(&FakePbx::default(), FakeConfig::default(), &FakeStore::default()),
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
        local(&solved.nvdata, "progress").unwrap(),
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
    assert_eq!(local(&p.nvdata, "progress").unwrap(), "level=13\nsolved=13\nmoves=ur\n");
    p.tap(&[Key::Left, Key::Digit(4)]);
    assert_eq!(local(&p.nvdata, "progress").unwrap(), "level=13\nsolved=13\nmoves=ur\n");
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
