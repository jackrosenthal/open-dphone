//! Light Up: put lights on the white cells of a grid until every cell is
//! lit, no light shines on another, and every numbered black cell has its
//! number of lights beside it.  The puzzle itself (rules, solver,
//! generator, pictures) is the crate `lightup-game`, in `game/` beside
//! this one; this is the app around it: the screens, the keys, and the
//! puzzle kept on the phone.
//!
//! The arrow keys, or 4, 2, 6, and 8, move the cursor.  5 or Select puts a
//! light on the cell, 3 marks it with a square (no light here), and 1
//! clears it.  "New" offers the sizes and difficulties; each puzzle is
//! made on the spot and has exactly one solution.
//!
//! Making a puzzle takes a while on the phone, so it is done in short
//! turns from a timer while "Generating…" is on the screen, and only
//! while the app is on the screen: a call stops it until the app is back.
//! The board is one canvas per row, and the cursor one more over them, so
//! a key draws again only the rows it changed.  The app makes no sound,
//! and like any app it leaves the screen to a call.

use app_sdk::prelude::*;
use app_sdk::{Align, Font, canvas, read_string};
use lightup_game::draw::{Layout, Look, Op, Painter, Palette, Rgb};
use lightup_game::generate::{Difficulty, Generator, Params, Progress, SIZES};
use lightup_game::{Act, Game, Grid};

const SK_NEW: u32 = 1;
const SK_EXIT: u32 = 2;
const SK_PLAY: u32 = 3;
const SK_BACK: u32 = 4;
const SK_RETRY: u32 = 5;

/// The file, in the app's own storage, that holds the puzzle with its
/// lights and marks, the time spent on it, and the cursor.
const PUZZLE: &str = "puzzle";
/// A file an administrator or a test may put beside it: a decimal seed
/// for the puzzles (the first is made from it, the next from it plus 1,
/// and so on).  Without it the phone's random source seeds each puzzle.
const SEED: &str = "seed";

/// How long one turn of making a puzzle lasts, and the pause before the
/// next, in milliseconds: the keys, the screen, and the rest of the
/// phone get their turn in between.
const TURN_MS: u64 = 50;
const PAUSE_MS: u32 = 5;
/// How many grids a puzzle may take.  Of 3,000 puzzles of each size and
/// difficulty none took more than 81; this many means something is wrong,
/// and the user is asked.
const MAX_GRIDS: u32 = 400;

#[derive(Default, PartialEq)]
enum Screen {
    #[default]
    None,
    Choose,
    Generating,
    Failed,
    Play,
}

/// The widgets of the playing screen.
struct Board {
    painter: Painter,
    /// A canvas per row, and what it shows.
    rows: Vec<(Widget, Vec<Look>)>,
    cursor: Widget,
    cursor_at: Option<(usize, usize)>,
    status: Widget,
    time: Widget,
}

/// A puzzle being made.
struct Making {
    generator: Generator,
    side: usize,
    difficulty: Difficulty,
    timer: Option<u32>,
    /// Milliseconds worked so far.
    worked: u64,
}

struct LightUp {
    screen: Screen,
    /// The size and difficulty of the puzzle being played.
    side: usize,
    difficulty: Difficulty,
    game: Option<Game>,
    /// The size and difficulty asked for last.
    wanted: (usize, Difficulty),
    /// Milliseconds spent on the puzzle, up to `since`.
    elapsed: u64,
    /// When the clock was started, if it runs.
    since: Option<u64>,
    making: Option<Making>,
    /// Puzzles made since the app started.
    made: u64,
    board: Option<Board>,
    list: Option<Widget>,
    /// What the puzzle file holds.
    saved: String,
}

impl Default for LightUp {
    fn default() -> LightUp {
        LightUp {
            screen: Screen::None,
            side: SIZES[0],
            difficulty: Difficulty::Easy,
            game: None,
            wanted: (SIZES[0], Difficulty::Easy),
            elapsed: 0,
            since: None,
            making: None,
            made: 0,
            board: None,
            list: None,
            saved: String::new(),
        }
    }
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
                width,
                height,
                scale,
                bits,
                color: c,
            } => canvas::Op::Bitmap(canvas::Bitmap {
                x,
                y,
                width,
                height,
                scale,
                color: color(c),
                bits: bits.to_vec(),
            }),
        })
        .collect();
    if let Err(e) = canvas::draw(w, &ops) {
        app_sdk::log_error(&format!("draw: {e}"));
    }
}

fn text(bounds: Rect, font: Font, align: Align, content: &str) -> Widget {
    let w = ui::create(WidgetKind::Text, ROOT, bounds);
    ui::set_font(w, font);
    ui::set_align(w, align);
    ui::set_text(w, content);
    w
}

/// Lines of text in the middle of a rectangle.
fn centered(area: Rect, lines: &[&str]) {
    let m = ui::get_metrics();
    let spare = area
        .height
        .saturating_sub(lines.len() as u16 * m.line_height);
    for (n, line) in lines.iter().enumerate() {
        let y = area.y + (spare / 2 + n as u16 * m.line_height) as i16;
        text(
            Rect::new(area.x, y, area.width, m.line_height),
            Font::Normal,
            Align::Center,
            line,
        );
    }
}

/// A time as the solved puzzle shows it: `3:05`, or `1:02:03`.
fn clock(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

fn size_name(side: usize) -> String {
    format!("{side}x{side}")
}

fn difficulty_named(name: &str) -> Option<Difficulty> {
    Difficulty::ALL
        .into_iter()
        .find(|d| d.name().eq_ignore_ascii_case(name))
}

impl LightUp {
    /// Reads the puzzle file; what it does not say, or says wrongly, is
    /// no puzzle.
    fn restore(&mut self) {
        self.saved = read_string(Location::Nv, PUZZLE).unwrap_or_default();
        let saved = self.saved.clone();
        let field = |name: &str| {
            saved
                .lines()
                .find_map(|l| l.strip_prefix(name)?.strip_prefix('='))
                .unwrap_or("")
        };
        if let Ok(side) = field("size").parse::<usize>()
            && SIZES.contains(&side)
        {
            self.side = side;
        }
        if let Some(d) = difficulty_named(field("difficulty")) {
            self.difficulty = d;
        }
        self.wanted = (self.side, self.difficulty);
        let rows: Vec<&str> = saved
            .lines()
            .filter(|l| !l.is_empty() && !l.contains('='))
            .collect();
        let Some(grid) = Grid::parse(&rows.join("\n")) else {
            return;
        };
        if grid.width() != self.side || grid.height() != self.side {
            return;
        }
        let mut game = Game::new(grid);
        let mut at = field("cursor")
            .split(',')
            .map(|n| n.parse::<usize>().unwrap_or(0));
        game.set_cursor(at.next().unwrap_or(0), at.next().unwrap_or(0));
        self.elapsed = field("elapsed").parse().unwrap_or(0);
        self.game = Some(game);
    }

    /// Writes the puzzle file if it would change.
    fn save(&mut self) {
        let (side, difficulty) = match self.game {
            Some(_) => (self.side, self.difficulty),
            None => self.wanted,
        };
        let mut now = format!(
            "size={side}\ndifficulty={}\n",
            difficulty.name().to_lowercase()
        );
        if let Some(game) = &self.game {
            let (x, y) = game.cursor();
            now.push_str(&format!(
                "elapsed={}\ncursor={x},{y}\n{}",
                self.spent(),
                game.grid().to_text()
            ));
        }
        if now != self.saved && storage::write(Location::Nv, PUZZLE, now.as_bytes()).is_ok() {
            self.saved = now;
        }
    }

    /// Milliseconds spent on the puzzle so far.
    fn spent(&self) -> u64 {
        self.elapsed + self.since.map_or(0, |t| timers::now_ms().saturating_sub(t))
    }

    /// The clock runs while an unsolved puzzle is on the screen.
    fn run_clock(&mut self, on: bool) {
        let on =
            on && self.screen == Screen::Play && self.game.as_ref().is_some_and(|g| !g.solved());
        if !on {
            self.elapsed = self.spent();
            self.since = None;
        } else if self.since.is_none() {
            self.since = Some(timers::now_ms());
        }
    }

    fn leave_screen(&mut self) {
        self.run_clock(false);
        self.board = None;
        self.list = None;
        if let Some(timer) = self.making.as_mut().and_then(|m| m.timer.take()) {
            timers::cancel(timer);
        }
    }

    /// The sizes and difficulties to choose from.
    fn show_choose(&mut self) {
        self.leave_screen();
        self.making = None;
        self.screen = Screen::Choose;
        let area = screen("New puzzle");
        let list = ui::create(WidgetKind::TextList, ROOT, area);
        let mut items = Vec::new();
        let mut selected = 0;
        for side in SIZES {
            for d in Difficulty::ALL {
                if (side, d) == self.wanted {
                    selected = items.len() as u32;
                }
                items.push(ListItem::new(format!("{} {}", size_name(side), d.name())));
            }
        }
        ui::set_items(list, &items);
        ui::set_selected(list, selected);
        // Back to the puzzle, if there is one; else out.
        let other = if self.game.is_some() {
            Softkey::new(SK_BACK, "Back")
        } else {
            Softkey::new(SK_EXIT, "Exit")
        };
        ui::set_softkeys(ROOT, &[Softkey::new(SK_PLAY, "Play"), other]);
        ui::set_focus(list);
        self.list = Some(list);
    }

    /// Starts making the puzzle of a row of the list.
    fn choose(&mut self, row: usize) {
        let n = Difficulty::ALL.len();
        self.wanted = (
            SIZES[(row / n).min(SIZES.len() - 1)],
            Difficulty::ALL[row % n],
        );
        self.generate();
    }

    fn generate(&mut self) {
        let seed = match read_string(Location::Nv, SEED).and_then(|s| s.trim().parse::<u64>().ok())
        {
            Some(seed) => seed.wrapping_add(self.made),
            None => runtime::random(),
        };
        self.made += 1;
        let (side, difficulty) = self.wanted;
        self.making = Some(Making {
            generator: Generator::new(Params::preset(side, difficulty), seed, MAX_GRIDS),
            side,
            difficulty,
            timer: None,
            worked: 0,
        });
        self.show_generating();
    }

    fn show_generating(&mut self) {
        self.leave_screen();
        self.screen = Screen::Generating;
        ui::clear(ROOT);
        ui::claim_keys(ROOT, &["cancel".to_string()]);
        let name = format!("{} {}", size_name(self.wanted.0), self.wanted.1.name());
        centered(content_rect(), &[&name, "Generating\u{2026}"]);
        ui::set_softkeys(
            ROOT,
            &[Softkey::new(SK_NEW, "New"), Softkey::new(SK_EXIT, "Exit")],
        );
        if let Some(making) = &mut self.making {
            making.timer = Some(timers::set(PAUSE_MS, false));
        }
    }

    /// One turn of making the puzzle.
    fn work(&mut self) {
        let Some(making) = &mut self.making else {
            return;
        };
        making.timer = None;
        let start = timers::now_ms();
        let progress = loop {
            let progress = making.generator.step();
            if progress != Progress::Working || timers::now_ms().saturating_sub(start) >= TURN_MS {
                break progress;
            }
        };
        making.worked += timers::now_ms().saturating_sub(start);
        match progress {
            Progress::Working => making.timer = Some(timers::set(PAUSE_MS, false)),
            Progress::Done | Progress::Failed => {
                app_sdk::log_info(&format!(
                    "{} {}: {} after {} ms, {} grids, {} steps",
                    size_name(making.side),
                    making.difficulty.name(),
                    if progress == Progress::Done {
                        "a puzzle"
                    } else {
                        "no puzzle"
                    },
                    making.worked,
                    making.generator.attempts(),
                    making.generator.steps()
                ));
                let puzzle = making.generator.puzzle().cloned();
                let made = (making.side, making.difficulty);
                self.making = None;
                match puzzle {
                    Some(grid) => {
                        (self.side, self.difficulty) = made;
                        self.game = Some(Game::new(grid));
                        self.elapsed = 0;
                        self.since = None;
                        self.save();
                        self.show_play();
                    }
                    None => self.show_failed(),
                }
            }
        }
    }

    fn show_failed(&mut self) {
        self.leave_screen();
        self.screen = Screen::Failed;
        ui::clear(ROOT);
        ui::claim_keys(ROOT, &["cancel".to_string()]);
        centered(content_rect(), &["No puzzle was found.", "Try again."]);
        ui::set_softkeys(
            ROOT,
            &[
                Softkey::new(SK_RETRY, "Retry"),
                Softkey::new(SK_NEW, "New"),
                Softkey::new(SK_EXIT, "Exit"),
            ],
        );
    }

    fn show_play(&mut self) {
        self.leave_screen();
        let Some(game) = &self.game else {
            return self.show_choose();
        };
        self.screen = Screen::Play;
        ui::clear(ROOT);
        ui::claim_keys(
            ROOT,
            &[
                "up", "down", "left", "right", "select", "cancel", "1", "2", "3", "4", "5", "6",
                "8",
            ]
            .map(String::from),
        );
        let m = ui::get_metrics();
        let l = Layout::new(m.width, m.height, self.side);
        let palette = if m.color {
            Palette::color()
        } else {
            let fg = canvas::theme_color(canvas::Role::Foreground);
            let bg = canvas::theme_color(canvas::Role::Background);
            Palette::gray(Rgb(fg.r, fg.g, fg.b), Rgb(bg.r, bg.g, bg.b))
        };
        let painter = Painter::new(self.side, l.pitch, palette);
        let rows = (0..game.grid().height())
            .map(|row| {
                let (x, y, width, height) = painter.row_bounds(row);
                let w =
                    canvas::create(ROOT, Rect::new(l.board_x + x, l.board_y + y, width, height));
                (w, Vec::new())
            })
            .collect();
        let cursor = canvas::create(
            ROOT,
            Rect::new(l.board_x, l.board_y, l.board_side, l.board_side),
        );

        // The panel: four lines in the middle of its height.
        let gap = (m.line_height / 2) as i16;
        let (x, width) = (l.panel_x + gap, l.panel_width.saturating_sub(gap as u16));
        let top = (m.height.saturating_sub(4 * m.line_height) / 2) as i16;
        let line = |n: u16| Rect::new(x, top + (n * m.line_height) as i16, width, m.line_height);
        text(line(0), Font::Bold, Align::Left, &size_name(self.side));
        text(line(1), Font::Normal, Align::Left, self.difficulty.name());
        self.board = Some(Board {
            painter,
            rows,
            cursor,
            cursor_at: None,
            status: text(line(2), Font::Bold, Align::Left, ""),
            time: text(line(3), Font::Normal, Align::Left, ""),
        });
        ui::set_softkeys(
            ROOT,
            &[Softkey::new(SK_NEW, "New"), Softkey::new(SK_EXIT, "Exit")],
        );
        self.update();
        self.run_clock(true);
    }

    /// Draws the rows that no longer show what the puzzle has, the cursor
    /// if it moved, and the time once the puzzle is solved.
    fn update(&mut self) {
        let (Some(game), Some(board)) = (&self.game, &mut self.board) else {
            return;
        };
        for (row, (canvas, shown)) in board.rows.iter_mut().enumerate() {
            let now = board.painter.row_state(game.grid(), row);
            if *shown != now {
                draw(*canvas, board.painter.row(game.grid(), row));
                *shown = now;
            }
        }
        let at = (!game.solved()).then(|| game.cursor());
        if board.cursor_at != at {
            draw(board.cursor, board.painter.cursor(game));
            board.cursor_at = at;
        }
        if game.solved() {
            ui::set_text(board.status, "Solved!");
            ui::set_text(board.time, &format!("Time {}", clock(self.elapsed)));
        }
    }

    fn step(&mut self, dx: i8, dy: i8) {
        if self.game.as_mut().is_some_and(|g| g.move_cursor(dx, dy)) {
            self.update();
        }
    }

    fn act(&mut self, act: Act) {
        let Some(game) = &mut self.game else { return };
        if !game.act(act) {
            return;
        }
        if game.solved() {
            // The time taken stands, and the solved puzzle is kept.
            self.run_clock(false);
            self.update();
            self.save();
        } else {
            self.update();
        }
    }

    /// The user is done: the app leaves the screen and stops.
    fn leave(&mut self) {
        self.run_clock(false);
        self.save();
        runtime::set_exit_after_background(true);
        runtime::background();
    }
}

impl App for LightUp {
    fn start(&mut self) {
        // A call over the puzzle does not end it: the app stays until
        // the user leaves it.
        runtime::set_exit_after_background(false);
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Foreground => match self.screen {
                Screen::None => {
                    self.restore();
                    self.show_play();
                }
                Screen::Play => self.run_clock(true),
                Screen::Generating => {
                    if let Some(making) = &mut self.making
                        && making.timer.is_none()
                    {
                        making.timer = Some(timers::set(PAUSE_MS, false));
                    }
                }
                _ => {}
            },
            Event::Background | Event::Exit => {
                // Nothing is worked out while something else has the
                // screen (e.g., a call).
                if let Some(timer) = self.making.as_mut().and_then(|m| m.timer.take()) {
                    timers::cancel(timer);
                }
                self.run_clock(false);
                self.save();
            }
            Event::Timer(id) => {
                if self.making.as_ref().is_some_and(|m| m.timer == Some(id)) {
                    self.work();
                }
            }
            Event::Softkey(SK_EXIT) => self.leave(),
            Event::Softkey(SK_NEW) => self.show_choose(),
            Event::Softkey(SK_BACK) => self.show_play(),
            Event::Softkey(SK_RETRY) => self.generate(),
            Event::Softkey(SK_PLAY) => {
                if let Some(list) = self.list {
                    self.choose(ui::selected(list) as usize);
                }
            }
            Event::Widget((_, WidgetEvent::Activated(row))) if self.screen == Screen::Choose => {
                self.choose(row as usize)
            }
            Event::Key(key) => match (key.as_str(), &self.screen) {
                ("up" | "2", Screen::Play) => self.step(0, -1),
                ("down" | "8", Screen::Play) => self.step(0, 1),
                ("left" | "4", Screen::Play) => self.step(-1, 0),
                ("right" | "6", Screen::Play) => self.step(1, 0),
                ("select" | "5", Screen::Play) => self.act(Act::Light),
                ("3", Screen::Play) => self.act(Act::Mark),
                ("1", Screen::Play) => self.act(Act::Clear),
                ("cancel", Screen::Choose) if self.game.is_some() => self.show_play(),
                ("cancel", _) => self.leave(),
                _ => {}
            },
            _ => {}
        }
    }
}

app_sdk::export_app!(LightUp);
