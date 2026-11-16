//! Light Up: put lights on the white cells of a grid until every cell is
//! lit, no light shines on another, and every numbered black cell has its
//! number of lights beside it.  The puzzle itself (rules, solver,
//! generator, pictures) is the crate `lightup-game`, in `game/` beside
//! this one; this is the app around it: the screens, the keys, and the
//! puzzle kept on the phone.
//!
//! The arrow keys, or 2, 4, 5, and 6 (up, left, down, and right), move
//! the cursor.  1 puts a light on the cell, or takes the light off a cell
//! that has one; 3 marks the cell with a square (no light here), or takes
//! the square off; each puts its own in place of the other's.  Select
//! does what 1 does.  8 does what the numbers settle as the board stands:
//! squares around each number that has its lights, and lights where a
//! number has just enough cells left.  "New" offers the sizes and
//! difficulties; each puzzle is made on the spot and has exactly one
//! solution.  "Undo" takes the last change to the board back (all of what
//! one press of 8 did), and the one before, as far as the fresh puzzle;
//! the changes are kept with the puzzle.  "Help" is five pages of how to
//! play, each a picture of a rule.  Beside the board is a picture
//! of the keys and what each does, and over it the time spent on the
//! puzzle, which runs while the unsolved puzzle is on the screen.
//!
//! The time of a solved puzzle goes to the table of the best times of
//! its kind (its size and difficulty), which the phones of a PBX share:
//! the app's store on the PBX has a key for each kind (see
//! `lightup_game::scores`).  "Scores" shows a table; there, "Size" and
//! "Level" (or left and right) go to the tables of the other kinds.  A
//! time that cannot be written, because the PBX is out of reach or keeps
//! no such store, is kept on the phone and written when the store comes
//! into reach; until then the table shows it on this phone only.
//!
//! Making a puzzle takes a while on the phone, so it is done in short
//! turns from a timer while "Generating…" is on the screen, and only
//! while the app is on the screen: a call stops it until the app is back.
//! The board is one canvas per row, and the cursor one more over them, so
//! a key draws again only the rows it changed.  The app makes no sound,
//! and like any app it leaves the screen to a call.

use dphone_app_sdk::prelude::*;
use dphone_app_sdk::{Align, Font, canvas, read_string};
use lightup_game::draw::{
    self, FontSize, Fonts, Layout, Look, Op, Painter, Palette, Rgb, TextFont,
};
use lightup_game::generate::{Difficulty, Generator, Params, Progress, SIZES};
use lightup_game::help::PAGES;
use lightup_game::scores::{Kind, Pending, Table, clock};
use lightup_game::{Act, Game, Grid};

const SK_NEW: u32 = 1;
const SK_EXIT: u32 = 2;
const SK_PLAY: u32 = 3;
const SK_BACK: u32 = 4;
const SK_RETRY: u32 = 5;
const SK_HELP: u32 = 6;
const SK_NEXT: u32 = 7;
const SK_UNDO: u32 = 8;
const SK_SCORES: u32 = 9;
const SK_SIZE: u32 = 10;
const SK_LEVEL: u32 = 11;

/// The key, in the app's storage, that holds the puzzle with its
/// lights and marks, the time spent on it, and the cursor.
const PUZZLE: &str = "puzzle";
/// A file an administrator or a test may put beside it: a decimal seed
/// for the puzzles (the first is made from it, the next from it plus 1,
/// and so on).  Without it the phone's random source seeds each puzzle.
const SEED: &str = "seed";
/// The key, in the app's storage, that holds the times of solved puzzles
/// that the tables on the PBX do not have yet.
const TIMES: &str = "times";

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
    Help,
    Scores,
}

/// The widgets of the playing screen.
struct Board {
    painter: Painter,
    /// A canvas per row, and what it shows.
    rows: Vec<(Widget, Vec<Look>)>,
    cursor: Widget,
    cursor_at: Option<(usize, usize)>,
    /// The time spent, over the picture of the keys, and what it says.
    clock: Widget,
    clock_shown: String,
    /// The picture of the keys beside the board; "Solved!" and the time
    /// take the place of both.
    keys: Widget,
    solved_shown: bool,
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
    /// The timer of the next second of the time on the screen.
    tick: Option<u32>,
    making: Option<Making>,
    /// Puzzles made since the app started.
    made: u64,
    board: Option<Board>,
    list: Option<Widget>,
    /// The page of the help on show, and whether it was opened from the
    /// list of sizes (else from the puzzle).
    page: usize,
    help_from_list: bool,
    /// What the puzzle file holds.
    saved: String,
    /// The tables of the best times as the PBX last gave them, by
    /// `Kind::index`.
    tables: Vec<Option<Table>>,
    /// The times the tables on the PBX do not have yet.
    pending: Pending,
    /// The kinds of puzzle whose table the PBX would not write; they are
    /// tried again when the app next starts.
    refused: Vec<Kind>,
    /// The time being written to its table: the kind, the time, and the
    /// request to the PBX under way (reading the table, or writing it).
    writing: Option<(Kind, u64, u32)>,
    /// The table on show, and the request reading it.
    shown: Kind,
    reading: Option<u32>,
    /// Whether the PBX answered when it was last asked for the table on
    /// show (`None` until it does).
    reached: Option<bool>,
    /// Whether the tables were opened from the list of sizes (else from
    /// the puzzle), and the row of the list to come back to.
    scores_from_list: bool,
    row: Option<u32>,
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
            tick: None,
            making: None,
            made: 0,
            board: None,
            list: None,
            page: 0,
            help_from_list: false,
            saved: String::new(),
            tables: vec![None; Kind::COUNT],
            pending: Pending::default(),
            refused: Vec::new(),
            writing: None,
            shown: Kind::new(SIZES[0], Difficulty::Easy),
            reading: None,
            reached: None,
            scores_from_list: false,
            row: None,
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

/// The colors of the pictures on this display.
/// The display's fonts, the bold ones first, as the pictures name them
/// and as the phone does.
const FONTS: [(TextFont, Font); 5] = [
    (TextFont::Bold, Font::Bold),
    (TextFont::Title, Font::Title),
    (TextFont::Large, Font::Large),
    (TextFont::Normal, Font::Normal),
    (TextFont::Small, Font::Small),
];

/// The colors of the pictures on this display, and its fonts with the
/// size of their digits: the numbers are drawn in the largest that fits.
fn palette() -> Palette {
    let mut palette = if ui::get_metrics().color {
        Palette::color()
    } else {
        let fg = canvas::theme_color(canvas::Role::Foreground);
        let bg = canvas::theme_color(canvas::Role::Background);
        Palette::gray(Rgb(fg.r, fg.g, fg.b), Rgb(bg.r, bg.g, bg.b))
    };
    palette.fonts = Fonts::new(&FONTS.map(|(font, ui_font)| FontSize {
        font,
        // The widest of the digits, and the tallest.
        width: canvas::text_size(ui_font, "8").width,
        height: canvas::text_size(ui_font, "0123456789").height,
    }));
    palette
}

fn paint(w: Widget, ops: Vec<Op>) {
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
            Op::Text {
                x,
                y,
                width,
                height,
                font,
                text,
                color: c,
            } => canvas::Op::Text(canvas::Text {
                box_: Rect::new(x, y, width, height),
                font: FONTS
                    .iter()
                    .find(|f| f.0 == font)
                    .map_or(Font::Normal, |f| f.1),
                color: color(c),
                text: text.to_string(),
            }),
        })
        .collect();
    if let Err(e) = canvas::draw(w, &ops) {
        dphone_app_sdk::log_error(&format!("draw: {e}"));
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

/// The phone's user as the tables know them: the id at the PBX and the
/// name to show.  A phone that has not reached the PBX does not know
/// its user.
fn player() -> Option<(String, String)> {
    let user = pbx_storage::user()?;
    let name = [&user.name, &user.extension, &user.id]
        .into_iter()
        .find(|n| !n.trim().is_empty())?
        .clone();
    Some((user.id, name))
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
        self.saved = read_string(PUZZLE).unwrap_or_default();
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
        game.set_changes(field("undo"));
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
                "elapsed={}\ncursor={x},{y}\nundo={}\n{}",
                self.spent(),
                game.changes_text(),
                game.grid().to_text()
            ));
        }
        if now != self.saved && storage::set(PUZZLE, now.as_bytes()).is_ok() {
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
            if let Some(tick) = self.tick.take() {
                timers::cancel(tick);
            }
        } else {
            if self.since.is_none() {
                self.since = Some(timers::now_ms());
            }
            self.show_time();
        }
    }

    /// Shows the time spent on the puzzle, and, while the clock runs,
    /// asks to be called when its next second begins.
    fn show_time(&mut self) {
        let spent = self.spent();
        if let Some(board) = &mut self.board {
            let now = clock(spent);
            if board.clock_shown != now {
                ui::set_text(board.clock, &now);
                board.clock_shown = now;
            }
        }
        if let Some(tick) = self.tick.take() {
            timers::cancel(tick);
        }
        if self.since.is_some() {
            self.tick = timers::set((1000 - spent % 1000) as u32, false).ok();
        }
    }

    /// The kind of the puzzle being played.
    fn kind(&self) -> Kind {
        Kind::new(self.side, self.difficulty)
    }

    fn leave_screen(&mut self) {
        self.run_clock(false);
        if let Some(id) = self.reading.take() {
            pbx_storage::cancel(id);
        }
        if self.screen == Screen::Scores {
            pbx_storage::watch(false);
        }
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
        // Back from the tables, the row they were opened from.
        if let Some(row) = self.row.take() {
            selected = row.min(items.len() as u32 - 1);
        }
        ui::set_items(list, &items);
        ui::set_selected(list, selected);
        // Back to the puzzle, if there is one; else out.
        let other = if self.game.is_some() {
            Softkey::new(SK_BACK, "Back")
        } else {
            Softkey::new(SK_EXIT, "Exit")
        };
        ui::set_softkeys(
            ROOT,
            &[
                Softkey::new(SK_PLAY, "Play"),
                Softkey::new(SK_HELP, "Help"),
                other,
                Softkey::new(SK_SCORES, "Scores"),
            ],
        );
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
        let seed = match read_string(SEED).and_then(|s| s.trim().parse::<u64>().ok())
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
            making.timer = timers::set(PAUSE_MS, false).ok();
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
            Progress::Working => making.timer = timers::set(PAUSE_MS, false).ok(),
            Progress::Done | Progress::Failed => {
                dphone_app_sdk::log_info(&format!(
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
        let palette = palette();
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

        // The panel beside the board: the time spent and, under it, the
        // picture of the keys, the two in the middle of its height.
        // "Solved!" and the time, when it comes to that, are two lines in
        // the middle.
        let gap = (m.line_height / 2) as i16;
        let (x, width) = (
            l.panel_x + gap,
            l.panel_width.saturating_sub(2 * gap as u16),
        );
        let ink = canvas::theme_color(canvas::Role::Foreground);
        let room = m.height.saturating_sub(4 + m.line_height);
        let keymap = draw::keymap(width, room, palette, Rgb(ink.r, ink.g, ink.b));
        let top = ((m.height - m.line_height - keymap.height) / 2) as i16;
        let shown = clock(self.elapsed);
        let time = Rect::new(x, top, width, m.line_height);
        let clock = text(time, Font::Bold, Align::Center, &shown);
        let (left, top) = (
            x + ((width - keymap.width) / 2) as i16,
            top + m.line_height as i16,
        );
        let keys = canvas::create(ROOT, Rect::new(left, top, keymap.width, keymap.height));
        paint(keys, keymap.ops);
        let top = (m.height.saturating_sub(2 * m.line_height) / 2) as i16;
        let line = |n: u16| Rect::new(x, top + (n * m.line_height) as i16, width, m.line_height);
        self.board = Some(Board {
            painter,
            rows,
            cursor,
            cursor_at: None,
            clock,
            clock_shown: shown,
            keys,
            solved_shown: false,
            status: text(line(0), Font::Bold, Align::Left, ""),
            time: text(line(1), Font::Normal, Align::Left, ""),
        });
        // Undo first, where it is quickest to reach, and the tables of
        // the best times on the fourth key.  The Cancel key leaves the
        // app; a solved puzzle, which has nothing to take back, has a
        // softkey for that as well (`update` sets its keys).
        ui::set_softkeys(
            ROOT,
            &[
                Softkey::new(SK_UNDO, "Undo"),
                Softkey::new(SK_NEW, "New"),
                Softkey::new(SK_HELP, "Help"),
                Softkey::new(SK_SCORES, "Scores"),
            ],
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
                paint(*canvas, board.painter.row(game.grid(), row));
                *shown = now;
            }
        }
        let at = (!game.solved()).then(|| game.cursor());
        if board.cursor_at != at {
            paint(board.cursor, board.painter.cursor(game));
            board.cursor_at = at;
        }
        if game.solved() && !board.solved_shown {
            // The keys do nothing any more: their picture and the
            // running time make room.
            board.solved_shown = true;
            ui::set_visible(board.keys, false);
            ui::set_visible(board.clock, false);
            ui::set_text(board.status, "Solved!");
            ui::set_text(board.time, &format!("Time {}", clock(self.elapsed)));
            ui::set_softkeys(
                ROOT,
                &[
                    Softkey::new(SK_NEW, "New"),
                    Softkey::new(SK_HELP, "Help"),
                    Softkey::new(SK_EXIT, "Exit"),
                    Softkey::new(SK_SCORES, "Scores"),
                ],
            );
        }
    }

    /// A page of how to play: a picture of a rule, drawn as the board
    /// draws it, and its sentences beside it.
    fn show_help(&mut self, page: usize) {
        if self.screen != Screen::Help {
            self.help_from_list = self.screen == Screen::Choose;
        }
        // The time does not run meanwhile.
        self.leave_screen();
        self.screen = Screen::Help;
        self.page = page % PAGES.len();
        let area = screen(&format!("How to play ({}/{})", self.page + 1, PAGES.len()));
        ui::claim_keys(
            ROOT,
            &["up", "down", "left", "right", "select", "cancel"].map(String::from),
        );
        let m = ui::get_metrics();
        let shown = &PAGES[self.page];
        let grid = shown.grid();
        // The picture takes three tenths of the width.
        let pitch = (m.width * 3 / 10 / grid.width() as u16).max(7);
        let (mut width, mut height, mut ops) = draw::picture(&grid, pitch, palette());
        if !m.color {
            let ink = canvas::theme_color(canvas::Role::Foreground);
            (width, height, ops) = draw::framed(width, height, ops, Rgb(ink.r, ink.g, ink.b));
        }
        let gap = (m.line_height / 2) as i16;
        let top = area.y + (area.height.saturating_sub(height) / 2) as i16;
        let picture = canvas::create(ROOT, Rect::new(gap, top, width, height));
        paint(picture, ops);
        let x = 2 * gap + width as i16;
        let words = text(
            Rect::new(
                x,
                area.y,
                m.width.saturating_sub(x as u16 + gap as u16),
                area.height,
            ),
            Font::Normal,
            Align::Left,
            shown.text,
        );
        ui::set_wrap(words, true);
        ui::set_softkeys(
            ROOT,
            &[Softkey::new(SK_NEXT, "Next"), Softkey::new(SK_BACK, "Back")],
        );
    }

    /// Leaves the help or the tables for where they were opened: the
    /// puzzle as it was, or the list of sizes.
    fn back(&mut self) {
        let to_list = match self.screen {
            Screen::Help => self.help_from_list,
            Screen::Scores => self.scores_from_list,
            _ => false,
        };
        if to_list {
            self.show_choose();
        } else {
            self.show_play();
        }
    }

    /// The table of the best times of a kind of puzzle: what the phone
    /// has of it at once, and what the PBX has when it answers.
    fn show_scores(&mut self, kind: Kind) {
        match self.screen {
            Screen::Scores => {}
            Screen::Choose => {
                self.scores_from_list = true;
                self.row = self.list.map(ui::selected);
            }
            _ => self.scores_from_list = false,
        }
        // The time does not run meanwhile.
        self.leave_screen();
        self.screen = Screen::Scores;
        self.shown = kind;
        self.reached = None;
        // Told of the times other phones write, the table is read again.
        pbx_storage::watch(true);
        self.read_shown();
        self.draw_scores();
    }

    /// Asks the PBX for the table on show.
    fn read_shown(&mut self) {
        if let Some(id) = self.reading.take() {
            pbx_storage::cancel(id);
        }
        self.reading = Some(pbx_storage::get(PbxScope::Shared, &self.shown.key()));
    }

    /// Draws the table on show: the rows the PBX last gave, with the
    /// time of this phone that is not written there yet in its place.
    fn draw_scores(&mut self) {
        let kind = self.shown;
        let title = match self.reached {
            Some(false) => format!("Offline: {}", kind.title()),
            _ => format!("Best times: {}", kind.title()),
        };
        let area = screen(&title);
        ui::claim_keys(ROOT, &["left", "right", "cancel"].map(String::from));
        self.list = None;
        let (id, name) = player().unwrap_or((String::new(), "You".to_string()));
        let mut table = self.tables[kind.index()].clone().unwrap_or_default();
        if let Some(ms) = self.pending.of(kind) {
            table.add(ms, &id, &name);
        }
        if table.rows().is_empty() {
            centered(
                area,
                &[match self.reached {
                    None => "Loading\u{2026}",
                    Some(true) => "No times yet.",
                    Some(false) => "The PBX cannot be reached.",
                }],
            );
        } else {
            let list = ui::create(WidgetKind::TextList, ROOT, area);
            let items: Vec<ListItem> = table
                .rows()
                .iter()
                .enumerate()
                .map(|(n, row)| {
                    let mut item = ListItem::new(format!("{}. {}", n + 1, row.name));
                    item.detail = clock(row.ms);
                    item
                })
                .collect();
            ui::set_items(list, &items);
            // The player's row, if the table has one.
            if let Some(at) = table.place(&id) {
                ui::set_selected(list, at as u32);
            }
            ui::set_focus(list);
            self.list = Some(list);
        }
        ui::set_softkeys(
            ROOT,
            &[
                Softkey::new(SK_SIZE, "Size"),
                Softkey::new(SK_LEVEL, "Level"),
                Softkey::new(SK_BACK, "Back"),
            ],
        );
    }

    /// A table as the PBX has it now.
    fn learn(&mut self, kind: Kind, table: Table) {
        self.tables[kind.index()] = Some(table);
        if self.screen == Screen::Scores && self.shown == kind {
            self.reached = Some(true);
            self.draw_scores();
        }
    }

    /// Writes the times that wait down, to outlast the app.
    fn keep_pending(&mut self) {
        let kept = if self.pending.is_empty() {
            storage::remove(TIMES)
        } else {
            storage::set(TIMES, self.pending.to_text().as_bytes())
        };
        if let Err(e) = kept {
            dphone_app_sdk::log_error(&format!("the times were not kept: {e:?}"));
        }
    }

    /// The puzzle was solved just now: its time stands, the solved
    /// puzzle is kept, and the time goes to the table of its kind.
    fn solved(&mut self) {
        self.run_clock(false);
        self.update();
        self.save();
        if self.pending.add(self.kind(), self.elapsed) {
            self.keep_pending();
        }
        self.sync();
    }

    /// Starts writing the next time that waits to its table, unless one
    /// is being written: the table is read, and written with the time in
    /// it over exactly what was read.
    fn sync(&mut self) {
        if self.writing.is_some() {
            return;
        }
        let next = self
            .pending
            .times()
            .iter()
            .find(|(kind, _)| !self.refused.contains(kind));
        if let Some(&(kind, ms)) = next {
            let id = pbx_storage::get(PbxScope::Shared, &kind.key());
            self.writing = Some((kind, ms, id));
        }
    }

    /// The answer of the PBX's store to a request.
    fn answered(&mut self, id: u32, outcome: PbxStorageOutcome) {
        if self.reading == Some(id) {
            self.reading = None;
            match outcome {
                PbxStorageOutcome::Entry(entry) => {
                    let table = entry.map(|e| Table::parse(&e.value)).unwrap_or_default();
                    self.learn(self.shown, table);
                }
                other => {
                    if !matches!(other, PbxStorageOutcome::Unreachable(_)) {
                        dphone_app_sdk::log_warn(&format!("the table was not read: {other:?}"));
                    }
                    self.reached = Some(false);
                    if self.screen == Screen::Scores {
                        self.draw_scores();
                    }
                }
            }
            return;
        }
        let Some((kind, ms, asked)) = self.writing else {
            return;
        };
        if asked != id {
            return;
        }
        self.writing = None;
        match outcome {
            // The table as it is: after a conflict, the one another phone
            // wrote meanwhile.
            PbxStorageOutcome::Entry(current) | PbxStorageOutcome::Conflict(current) => {
                let (mut table, version) = match &current {
                    Some(entry) => (Table::parse(&entry.value), entry.version),
                    None => (Table::default(), 0),
                };
                self.learn(kind, table.clone());
                let Some((player, name)) = player() else {
                    // The time waits until the store is in reach again.
                    dphone_app_sdk::log_warn("the PBX has not said who the user is");
                    return;
                };
                if table.add(ms, &player, &name) {
                    let id = pbx_storage::set(
                        PbxScope::Shared,
                        &kind.key(),
                        &table.to_bytes(),
                        Some(version),
                    );
                    self.writing = Some((kind, ms, id));
                    return;
                }
                // The table has a time of the player's as good, or ten
                // better times: there is nothing to write.
                if self.pending.done(kind, ms) {
                    self.keep_pending();
                }
            }
            PbxStorageOutcome::Stored(entry) => {
                if self.pending.done(kind, ms) {
                    self.keep_pending();
                }
                self.learn(kind, Table::parse(&entry.value));
            }
            // The time waits until the store is in reach.
            PbxStorageOutcome::Unreachable(_) => return,
            other => {
                dphone_app_sdk::log_warn(&format!(
                    "the table {} was not written: {other:?}",
                    kind.key()
                ));
                self.refused.push(kind);
            }
        }
        self.sync();
    }

    fn step(&mut self, dx: i8, dy: i8) {
        if self.game.as_mut().is_some_and(|g| g.move_cursor(dx, dy)) {
            self.update();
        }
    }

    /// Puts a light or a mark on the cell under the cursor, or takes it
    /// off a cell that has it.
    fn toggle(&mut self, act: Act) {
        self.changed(|game| game.toggle(act));
    }

    /// Changes the board, and shows and keeps what came of it.
    fn changed(&mut self, change: impl FnOnce(&mut Game) -> bool) {
        let Some(game) = &mut self.game else { return };
        if !change(game) {
            return;
        }
        if game.solved() {
            self.solved();
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
        // The times that could not be written when the app last ran.
        self.pending = Pending::parse(&read_string(TIMES).unwrap_or_default());
        self.sync();
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
                        making.timer = timers::set(PAUSE_MS, false).ok();
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
                if self.tick == Some(id) {
                    self.tick = None;
                    self.show_time();
                } else if self.making.as_ref().is_some_and(|m| m.timer == Some(id)) {
                    self.work();
                }
            }
            Event::PbxStorage((id, outcome)) => self.answered(id, outcome),
            Event::PbxStorageChanged(change) => {
                // Another phone wrote the table on show.
                if self.screen == Screen::Scores
                    && matches!(change.scope, PbxScope::Shared)
                    && change.key == self.shown.key()
                {
                    self.read_shown();
                }
            }
            Event::PbxStorageOnline => {
                self.sync();
                if self.screen == Screen::Scores {
                    self.read_shown();
                }
            }
            Event::Softkey(SK_SCORES) => {
                // Of the row of the list of sizes, or of the puzzle.
                let kind = match (&self.screen, self.list) {
                    (Screen::Choose, Some(list)) => Kind::from_index(ui::selected(list) as usize),
                    _ => self.kind(),
                };
                self.show_scores(kind);
            }
            Event::Softkey(SK_SIZE) => self.show_scores(self.shown.next_size()),
            Event::Softkey(SK_LEVEL) => self.show_scores(self.shown.next_difficulty()),
            Event::Softkey(SK_EXIT) => self.leave(),
            Event::Softkey(SK_NEW) => self.show_choose(),
            Event::Softkey(SK_BACK) => self.back(),
            Event::Softkey(SK_HELP) => self.show_help(0),
            Event::Softkey(SK_UNDO) => {
                if self.screen == Screen::Play && self.game.as_mut().is_some_and(Game::undo) {
                    if self.game.as_ref().is_some_and(Game::solved) {
                        self.solved();
                    } else {
                        self.update();
                    }
                }
            }
            Event::Softkey(SK_NEXT) => self.show_help(self.page + 1),
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
                ("down" | "5", Screen::Play) => self.step(0, 1),
                ("left" | "4", Screen::Play) => self.step(-1, 0),
                ("right" | "6", Screen::Play) => self.step(1, 0),
                ("select" | "1", Screen::Play) => self.toggle(Act::Light),
                ("3", Screen::Play) => self.toggle(Act::Mark),
                ("8", Screen::Play) => self.changed(|game| game.auto()),
                ("right" | "down" | "select", Screen::Help) => self.show_help(self.page + 1),
                ("left" | "up", Screen::Help) => self.show_help(self.page + PAGES.len() - 1),
                ("cancel", Screen::Help | Screen::Scores) => self.back(),
                ("left", Screen::Scores) => self.show_scores(self.shown.step(-1)),
                ("right", Screen::Scores) => self.show_scores(self.shown.step(1)),
                ("cancel", Screen::Choose) if self.game.is_some() => self.show_play(),
                ("cancel", _) => self.leave(),
                _ => {}
            },
            _ => {}
        }
    }
}

dphone_app_sdk::export_app! {
    app: LightUp,
    name: "lightup",
    id: "org.opendphone.lightup",
    display_name: "Light Up",
    description: "Light every cell of the grid",
    phone_icon: "lightup",
}
