//! Box Pusher: push every box onto a target.  The game itself (rules,
//! levels, pictures) is the crate `boxpusher-game`, in `game/` beside this
//! one; this is the app around it: the screens, the keys, and the progress
//! kept on the phone.
//!
//! The arrow keys, or 2, 4, 6, and 8, move the figure.  A level's floor,
//! walls, and targets are drawn once on one canvas; the boxes and the
//! figure are on a second canvas over it, the only thing drawn again
//! after a step.  The app makes no sound and uses no timer, and like any
//! app it leaves the screen to a call.

use dphone_app_sdk::prelude::*;
use dphone_app_sdk::{Align, Font, canvas, read_string};
use boxpusher_game::draw::{Layout, Op, Painter, Palette, Rgb};
use boxpusher_game::{Dir, Game, LEVELS, Level, Step};

const SK_UNDO: u32 = 1;
const SK_RESTART: u32 = 2;
const SK_LEVELS: u32 = 3;
const SK_EXIT: u32 = 4;
const SK_NEXT: u32 = 5;
const SK_AGAIN: u32 = 6;
const SK_PLAY: u32 = 7;
const SK_BACK: u32 = 8;

/// The file, in the app's own storage, that holds the progress: the level
/// being played, how many levels are solved, and the steps taken in the
/// level so far.
const PROGRESS: &str = "progress";
/// The most steps the progress file holds.  A level left after more than
/// this starts over the next time; the longest level takes about 120.
const STEPS_KEPT: usize = 2000;

#[derive(Default, PartialEq)]
enum Screen {
    #[default]
    None,
    Play,
    Solved,
    Levels,
}

/// The widgets of the playing screen.
struct Board {
    pieces: Widget,
    moves: Widget,
    pushes: Widget,
}

#[derive(Default)]
struct BoxPusher {
    screen: Screen,
    painter: Option<Painter>,
    /// The level being played, counted from 0.
    level: usize,
    /// How many levels are solved: the levels up to this one are open.
    solved: usize,
    game: Option<Game>,
    /// The level whose solved screen is on show.
    finished: usize,
    board: Option<Board>,
    list: Option<Widget>,
    /// What the progress file holds.
    saved: String,
}

fn color(c: Rgb) -> canvas::Color {
    canvas::Color {
        r: c.0,
        g: c.1,
        b: c.2,
    }
}

fn draw(w: Widget, ops: Vec<Op>) {
    let ops: Vec<canvas::Op> = ops
        .into_iter()
        .map(|op| match op {
            Op::Fill {
                x,
                y,
                width,
                height,
                color: c,
            } => canvas::Op::Fill((Rect::new(x, y, width, height), color(c))),
            Op::Bitmap {
                x,
                y,
                size,
                bits,
                color: c,
            } => canvas::Op::Bitmap(canvas::Bitmap {
                x,
                y,
                width: size,
                height: size,
                scale: 1,
                color: color(c),
                bits: bits.to_vec(),
            }),
        })
        .collect();
    if let Err(e) = canvas::draw(w, &ops) {
        dphone_app_sdk::log_error(&format!("draw: {e}"));
    }
}

fn load(level: usize) -> Game {
    // The levels are checked when the firmware is built (the tests of
    // `boxpusher-game`, which the `tests` check runs).
    Game::new(Level::parse(LEVELS[level].text).expect("a level"))
}

fn text(bounds: Rect, font: Font, align: Align, content: &str) -> Widget {
    let w = ui::create(WidgetKind::Text, ROOT, bounds);
    ui::set_font(w, font);
    ui::set_align(w, align);
    ui::set_text(w, content);
    w
}

impl BoxPusher {
    /// Reads the progress file; what it does not say, or says wrongly, is
    /// the beginning.
    fn restore(&mut self) {
        self.saved = read_string(Location::Nv, PROGRESS).unwrap_or_default();
        let saved = self.saved.clone();
        let field = |name: &str| {
            saved
                .lines()
                .find_map(|l| l.strip_prefix(name)?.strip_prefix('='))
                .unwrap_or("")
        };
        let number = |name: &str| field(name).parse::<usize>().unwrap_or(0);
        self.solved = number("solved").min(LEVELS.len());
        self.level = number("level").min(self.solved).min(LEVELS.len() - 1);
        let mut game = load(self.level);
        game.replay(field("moves"));
        if game.solved() {
            game.restart();
        }
        self.game = Some(game);
    }

    /// Writes the progress file if it would change.
    fn save(&mut self) {
        let mut moves = self.game.as_ref().map(Game::record).unwrap_or_default();
        if moves.len() > STEPS_KEPT {
            moves.clear();
        }
        let now = format!(
            "level={}\nsolved={}\nmoves={}\n",
            self.level, self.solved, moves
        );
        if now != self.saved && storage::write(Location::Nv, PROGRESS, now.as_bytes()).is_ok() {
            self.saved = now;
        }
    }

    /// Where things go in the window, and the pictures at that size and
    /// in the display's colors.
    fn painter(&mut self) -> (&Painter, Layout) {
        let m = ui::get_metrics();
        let layout = Layout::new(m.width, m.height);
        let painter = self.painter.get_or_insert_with(|| {
            let palette = if m.color {
                Palette::color()
            } else {
                let fg = canvas::theme_color(canvas::Role::Foreground);
                let bg = canvas::theme_color(canvas::Role::Background);
                Palette::gray(Rgb(fg.r, fg.g, fg.b), Rgb(bg.r, bg.g, bg.b))
            };
            Painter::new(layout.tile, palette)
        });
        (painter, layout)
    }

    fn show_play(&mut self) {
        self.screen = Screen::Play;
        self.list = None;
        ui::clear(ROOT);
        ui::claim_keys(
            ROOT,
            &["up", "down", "left", "right", "2", "4", "6", "8", "cancel"].map(String::from),
        );
        let m = ui::get_metrics();
        let number = self.level + 1;
        if self.game.is_none() {
            self.game = Some(load(self.level));
        }
        let (_, l) = self.painter();
        let bounds = Rect::new(l.board_x, l.board_y, l.board_width, l.board_height);
        let scenery = canvas::create(ROOT, bounds);
        let pieces = canvas::create(ROOT, bounds);
        if let (Some(painter), Some(game)) = (&self.painter, &self.game) {
            draw(scenery, painter.scenery(game.level()));
        }

        // The panel: three lines in the middle of its height.
        let gap = (l.tile / 2) as i16;
        let (x, width) = (l.panel_x + gap, l.panel_width - gap as u16);
        let top = (m.height.saturating_sub(3 * m.line_height) / 2) as i16;
        let line = |n: u16| Rect::new(x, top + (n * m.line_height) as i16, width, m.line_height);
        text(line(0), Font::Bold, Align::Left, &format!("Level {number}"));
        let board = Board {
            pieces,
            moves: text(line(1), Font::Normal, Align::Left, ""),
            pushes: text(line(2), Font::Normal, Align::Left, ""),
        };
        ui::set_softkeys(
            ROOT,
            &[
                Softkey::new(SK_UNDO, "Undo"),
                Softkey::new(SK_RESTART, "Restart"),
                Softkey::new(SK_LEVELS, "Levels"),
                Softkey::new(SK_EXIT, "Exit"),
            ],
        );
        self.board = Some(board);
        self.update(true);
    }

    /// Draws the boxes and the figure where they are, and the counts.
    fn update(&mut self, pushes_too: bool) {
        let (Some(painter), Some(game), Some(board)) = (&self.painter, &self.game, &self.board)
        else {
            return;
        };
        draw(board.pieces, painter.pieces(game));
        ui::set_text(board.moves, &format!("Moves {}", game.moves()));
        if pushes_too {
            ui::set_text(board.pushes, &format!("Pushes {}", game.pushes()));
        }
    }

    fn step(&mut self, dir: Dir) {
        let Some(game) = &mut self.game else { return };
        let step = game.step(dir);
        let solved = game.solved();
        match step {
            Step::Blocked => {}
            Step::Moved => self.update(false),
            Step::Pushed if solved => self.show_solved(),
            Step::Pushed => self.update(true),
        }
    }

    fn show_solved(&mut self) {
        let Some(game) = self.game.take() else { return };
        self.screen = Screen::Solved;
        self.board = None;
        let last = self.level + 1 == LEVELS.len();
        let area = screen(&format!("Level {} solved!", self.level + 1));
        let m = ui::get_metrics();
        let mut lines = vec![
            format!("Moves: {}", game.moves()),
            format!("Pushes: {}", game.pushes()),
            format!("Fewest pushes: {}", LEVELS[self.level].pushes),
        ];
        if last {
            lines.push("That was the last level.".into());
        }
        let spare = area
            .height
            .saturating_sub(lines.len() as u16 * m.line_height);
        let top = area.y + (spare / 2) as i16;
        for (n, line) in lines.iter().enumerate() {
            let y = top + (n as u16 * m.line_height) as i16;
            text(
                Rect::new(0, y, m.width, m.line_height),
                Font::Normal,
                Align::Center,
                line,
            );
        }
        ui::claim_keys(ROOT, &["select", "cancel"].map(String::from));
        let mut keys = vec![
            Softkey::new(SK_AGAIN, "Again"),
            Softkey::new(SK_LEVELS, "Levels"),
            Softkey::new(SK_EXIT, "Exit"),
        ];
        if !last {
            keys.insert(0, Softkey::new(SK_NEXT, "Next"));
        }
        ui::set_softkeys(ROOT, &keys);

        // The next level is open, and it is where the game goes on.
        self.finished = self.level;
        self.solved = self.solved.max(self.level + 1);
        if !last {
            self.level += 1;
        }
        self.save();
    }

    /// Starts a level from its beginning.
    fn play(&mut self, level: usize) {
        self.level = level.min(self.solved).min(LEVELS.len() - 1);
        self.game = Some(load(self.level));
        self.save();
        self.show_play();
    }

    /// The open levels, to choose from.
    fn show_levels(&mut self) {
        self.screen = Screen::Levels;
        self.board = None;
        let area = screen("Levels");
        let list = ui::create(WidgetKind::TextList, ROOT, area);
        let open = (self.solved + 1).min(LEVELS.len());
        let items: Vec<ListItem> = (0..open)
            .map(|i| {
                let state = if i < self.solved { "Solved" } else { "" };
                ListItem::with_detail(format!("Level {}", i + 1), state)
            })
            .collect();
        ui::set_items(list, &items);
        ui::set_selected(list, self.level.min(open - 1) as u32);
        ui::set_softkeys(
            ROOT,
            &[Softkey::new(SK_PLAY, "Play"), Softkey::new(SK_BACK, "Back")],
        );
        ui::set_focus(list);
        self.list = Some(list);
    }

    /// The user is done: the app leaves the screen and stops.
    fn leave(&mut self) {
        self.save();
        runtime::set_exit_after_background(true);
        runtime::background();
    }

    /// Leaves the list of levels for the game it was opened from.
    fn back(&mut self) {
        match self.game {
            Some(_) => self.show_play(),
            None => self.play(self.level),
        }
    }
}

impl App for BoxPusher {
    fn start(&mut self) {
        // A call over the game does not end it: the app stays until the
        // user leaves it.
        runtime::set_exit_after_background(false);
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Foreground => {
                if self.screen == Screen::None {
                    self.restore();
                    self.show_play();
                }
            }
            Event::Background | Event::Exit => self.save(),
            Event::Softkey(SK_EXIT) => self.leave(),
            Event::Softkey(SK_LEVELS) => self.show_levels(),
            Event::Softkey(SK_BACK) => self.back(),
            Event::Softkey(SK_PLAY) => {
                if let Some(list) = self.list {
                    self.play(ui::selected(list) as usize);
                }
            }
            Event::Widget((_, WidgetEvent::Activated(row))) if self.screen == Screen::Levels => {
                self.play(row as usize)
            }
            Event::Softkey(SK_NEXT) => self.play(self.level),
            Event::Softkey(SK_AGAIN) => self.play(self.finished),
            Event::Softkey(SK_UNDO) => {
                if self.game.as_mut().is_some_and(Game::undo) {
                    self.update(true);
                }
            }
            Event::Softkey(SK_RESTART) => {
                if let Some(game) = &mut self.game {
                    game.restart();
                    self.update(true);
                }
            }
            Event::Key(key) => match (key.as_str(), &self.screen) {
                ("up" | "2", Screen::Play) => self.step(Dir::Up),
                ("down" | "8", Screen::Play) => self.step(Dir::Down),
                ("left" | "4", Screen::Play) => self.step(Dir::Left),
                ("right" | "6", Screen::Play) => self.step(Dir::Right),
                ("select", Screen::Solved) => self.play(self.level),
                ("cancel", Screen::Levels) => self.back(),
                ("cancel", _) => self.leave(),
                _ => {}
            },
            _ => {}
        }
    }
}

dphone_app_sdk::export_app!(BoxPusher);
