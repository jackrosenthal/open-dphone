//! Desk Sparrow: a sparrow that has to be kept in the air with one key and
//! flown through the openings between pipes.  Select, or any key of the
//! keypad (the digits, star, and pound), starts a game and flaps; each
//! opening is a point; a pipe or the floor ends the game.  The best score
//! is kept in the app's storage.
//!
//! The game has the whole display: there are no softkeys.  Cancel, which
//! the app leaves to the phone, closes it, as does line key 1.
//!
//! The game is drawn on a canvas, once for every tick of a repeating timer
//! and in one call, so the phone repaints once a tick.  Between games
//! nothing moves and no timer runs.  The speed of the game is chosen for
//! the screen (see [`game::Rules`]): twenty ticks a second on the color
//! screens and ten on the gray ones, whose panels are slow.  There is no
//! sound.

mod game;

use app_sdk::canvas::{self, Bitmap, Color, Op, Role};
use app_sdk::prelude::*;
use app_sdk::{Align, Font};
use game::{Game, Pipe, Rules};

/// The keys that start a game and flap: Select and the keypad.  Claimed,
/// so that a digit does not start a number to dial behind the game.
const PLAY_KEYS: [&str; 13] = [
    "select", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "*", "#",
];

/// The file of the best score: a decimal number.
const BEST_FILE: &str = "best";
/// A file an administrator or a test may put beside it: a decimal seed
/// that makes every game the same game.
const SEED_FILE: &str = "seed";
/// How to play and how to leave, said once, on the start screen.
const START_HINT: &str = "Any key flaps.  Cancel leaves.";
/// A key just after the end of a game is the last flap of that game, not
/// the start of the next one.
const RESTART_AFTER_MS: u64 = 700;

// ----------------------------------------------------------------- art

/// The sparrow, beak to the right: `o` dark feathers, `b` brown, `c` the
/// pale belly, `k` the beak, `e` the eye.
const SPARROW: [&str; 12] = [
    "......oooo......",
    "....oobbbbo.....",
    "o..obbbbbebo....",
    "oo.obbbbbbbkkk..",
    "ooobbbbbbbbkk...",
    ".oobbbbbbbcco...",
    "..obbbbbbccco...",
    "..obbbbbcccco...",
    "...obbccccco....",
    "....oocccco.....",
    "......oooo......",
    ".......o.o......",
];
/// The wing, drawn over the body: low while gliding, high after a flap.
const WING: [&str; 4] = ["ooooo.", "oooooo", ".ooooo", "..ooo."];
const WING_AT: (i32, i32) = (4, 5);
const WING_UP_AT: (i32, i32) = (4, 2);

/// The digits of the score, five by seven.
const DIGITS: [[u8; 7]; 10] = [
    [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
    [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
    [0x0E, 0x11, 0x01, 0x06, 0x08, 0x10, 0x1F],
    [0x1F, 0x02, 0x04, 0x06, 0x01, 0x11, 0x0E],
    [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
    [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
    [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
    [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
    [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
    [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
];

/// The pixels of `art` marked with `mark`, as a one-bit picture.
fn layer(art: &[&str], mark: u8) -> Vec<u8> {
    let stride = art[0].len().div_ceil(8);
    let mut bits = vec![0u8; stride * art.len()];
    for (y, row) in art.iter().enumerate() {
        for (x, c) in row.bytes().enumerate() {
            if c == mark {
                bits[y * stride + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    bits
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color { r, g, b }
}

/// `pct` percent of the way from `a` to `b`.
fn mix(a: Color, b: Color, pct: u32) -> Color {
    let ch = |a: u8, b: u8| ((u32::from(a) * (100 - pct) + u32::from(b) * pct + 50) / 100) as u8;
    rgb(ch(a.r, b.r), ch(a.g, b.g), ch(a.b, b.b))
}

/// The colors of the pipes: the skin's own.
struct PipeColors {
    body: Color,
    light: Color,
    dark: Color,
    /// The clamping bands.
    band: Color,
}

/// Everything about how the game looks on this phone.
struct Look {
    rules: Rules,
    /// The window.
    width: i32,
    height: i32,
    /// Size of one pixel of the artwork: 2 on the color screens.
    unit: i32,
    pipe: PipeColors,
    /// The sparrow's layers, back to front: the mark in the art and the
    /// color.
    feathers: [(u8, Color); 5],
    wing: Color,
    background: Color,
    foreground: Color,
    floor: Color,
    floor_edge: Color,
}

impl Look {
    fn new() -> Look {
        let m = ui::get_metrics();
        let rules = match m.skin {
            Skin::Color => Rules::COLOR,
            Skin::LargeGray => Rules::LARGE_GRAY,
            Skin::SmallGray => Rules::SMALL_GRAY,
        };
        let highlight = canvas::theme_color(Role::Highlight);
        let highlight_text = canvas::theme_color(Role::HighlightText);
        let (black, white) = (rgb(0, 0, 0), rgb(255, 255, 255));
        let (pipe, feathers, wing) = if m.color {
            (
                // The skin's blues.
                {
                    let dark = mix(canvas::theme_color(Role::TitleBackground), black, 25);
                    PipeColors {
                        body: highlight,
                        light: mix(highlight, highlight_text, 45),
                        dark,
                        band: dark,
                    }
                },
                [
                    (b'o', rgb(0x6E, 0x45, 0x22)),
                    (b'b', rgb(0xBC, 0x86, 0x4E)),
                    (b'c', rgb(0xF3, 0xE6, 0xCC)),
                    (b'k', rgb(0xF5, 0xA6, 0x23)),
                    (b'e', rgb(0x14, 0x14, 0x14)),
                ],
                rgb(0x6E, 0x45, 0x22),
            )
        } else {
            (
                // Four gray levels: black pipes with a light gray glint
                // and light gray bands.
                {
                    let light = mix(highlight, highlight_text, 66);
                    PipeColors {
                        body: highlight,
                        light,
                        dark: highlight,
                        band: light,
                    }
                },
                [
                    (b'o', black),
                    (b'b', black),
                    (b'c', white),
                    (b'k', black),
                    (b'e', white),
                ],
                rgb(0xAA, 0xAA, 0xAA),
            )
        };
        Look {
            rules,
            width: i32::from(m.width),
            height: i32::from(m.height),
            unit: if m.color { 2 } else { 1 },
            pipe,
            feathers,
            wing,
            background: canvas::theme_color(Role::Background),
            foreground: canvas::theme_color(Role::Foreground),
            floor: canvas::theme_color(Role::Border),
            floor_edge: canvas::theme_color(Role::Secondary),
        }
    }
}

// ------------------------------------------------------------- drawing

fn fill(ops: &mut Vec<Op>, x: i32, y: i32, w: i32, h: i32, color: Color) {
    if w > 0 && h > 0 {
        ops.push(Op::Fill((
            Rect::new(x as i16, y as i16, w as u16, h as u16),
            color,
        )));
    }
}

/// One obstacle: a pipe down from the top of the field and a pipe up from
/// the floor, with the opening between them.  Everything about what an
/// obstacle looks like is here and in [`pipe`].
fn obstacle(ops: &mut Vec<Op>, look: &Look, p: &Pipe) {
    let r = &look.rules;
    if p.x >= look.width || p.x + r.pipe_w <= 0 {
        return;
    }
    pipe(ops, look, p.x, 0, p.gap_y, true);
    pipe(ops, look, p.x, p.gap_y + r.gap, r.height, false);
}

/// A slim pipe from `top` to `bottom` whose open end, at the bottom if
/// `opens_down`, has a flat bolted flange; clamping bands sit along it at
/// even distances from the flange.  It is lit from the left.
fn pipe(ops: &mut Vec<Op>, look: &Look, x: i32, top: i32, bottom: i32, opens_down: bool) {
    let (u, w, c) = (look.unit, look.rules.pipe_w, &look.pipe);
    let len = bottom - top;
    if len <= 0 {
        return;
    }
    // The pipe itself is narrower than its flange.
    let inset = 2 * u;
    fill(ops, x + inset, top, w - 2 * inset, len, c.body);
    fill(ops, x + inset + u, top, 2 * u, len, c.light);
    fill(ops, x + w - inset - 2 * u, top, 2 * u, len, c.dark);

    let flange = (5 * u).min(len);
    // Distance from the open end to a row of the pipe.
    let at = |d: i32, h: i32| if opens_down { bottom - d - h } else { top + d };
    let (band, pitch) = (3 * u, 20 * u);
    let mut d = flange + 9 * u;
    while d + band + u <= len {
        fill(ops, x + u, at(d, band), w - 2 * u, band, c.band);
        fill(ops, x + u, at(d, band), w - 2 * u, u, c.light);
        d += pitch;
    }

    let fy = at(0, flange);
    fill(ops, x, fy, w, flange, c.body);
    fill(ops, x, fy, w, u, c.light);
    fill(ops, x, fy + flange - u, w, u, c.dark);
    fill(ops, x + w - u, fy, u, flange, c.dark);
    // Two bolts.
    let by = fy + (flange - 2 * u) / 2;
    if flange >= 4 * u {
        fill(ops, x + 3 * u, by, 2 * u, 2 * u, c.light);
        fill(ops, x + w - 6 * u, by, 2 * u, 2 * u, c.light);
    }
}

/// The sparrow with its top-left corner at (`x`, `y`).
fn sparrow(ops: &mut Vec<Op>, look: &Look, art: &Art, x: i32, y: i32, wing_up: bool) {
    let bitmap = |bits: &Vec<u8>, dx: i32, dy: i32, w: usize, h: usize, color: Color| {
        Op::Bitmap(Bitmap {
            x: (x + dx * look.unit) as i16,
            y: (y + dy * look.unit) as i16,
            width: w as u16,
            height: h as u16,
            scale: look.unit as u8,
            color,
            bits: bits.clone(),
        })
    };
    for (bits, (_, color)) in art.sparrow.iter().zip(look.feathers) {
        ops.push(bitmap(bits, 0, 0, SPARROW[0].len(), SPARROW.len(), color));
    }
    let (dx, dy) = if wing_up { WING_UP_AT } else { WING_AT };
    ops.push(bitmap(
        &art.wing,
        dx,
        dy,
        WING[0].len(),
        WING.len(),
        look.wing,
    ));
}

/// A plate with a frame: something to read on, whatever is behind.
fn plate(ops: &mut Vec<Op>, look: &Look, x: i32, y: i32, w: i32, h: i32) {
    let u = look.unit;
    fill(ops, x, y, w, h, look.foreground);
    fill(ops, x + u, y + u, w - 2 * u, h - 2 * u, look.background);
}

/// The score during a game: digits on a plate at the top of the field.
fn score(ops: &mut Vec<Op>, look: &Look, value: u32) {
    let text = value.to_string();
    let scale = if look.unit == 2 { 3 } else { 2 };
    let n = text.len() as i32;
    let (dw, dh, pad) = (5 * scale, 7 * scale, 2 * scale);
    let w = n * dw + (n - 1) * scale + 2 * pad;
    let (x, y) = ((look.width - w) / 2, look.unit);
    plate(ops, look, x, y, w, dh + 2 * pad);
    for (i, d) in text.bytes().enumerate() {
        // Five-bit rows, moved to the top of their bytes.
        let bits = DIGITS[(d - b'0') as usize]
            .iter()
            .map(|row| row << 3)
            .collect();
        ops.push(Op::Bitmap(Bitmap {
            x: (x + pad + i as i32 * (dw + scale)) as i16,
            y: (y + pad) as i16,
            width: 5,
            height: 7,
            scale: scale as u8,
            color: look.foreground,
            bits,
        }));
    }
}

// ----------------------------------------------------------------- app

/// The sparrow's pictures, one for each color.
struct Art {
    sparrow: Vec<Vec<u8>>,
    wing: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    /// The start screen.
    Ready,
    Flying,
    /// The end of a game, since this time.
    Over(u64),
}

/// The window: a canvas under two lines of text on a plate.
struct Screen {
    look: Look,
    art: Art,
    canvas: Widget,
    title: Widget,
    line: Widget,
    /// The plate behind the two lines.
    plate: Rect,
}

impl Screen {
    fn build() -> Screen {
        let look = Look::new();
        let m = ui::get_metrics();
        ui::clear(ROOT);
        ui::claim_keys(ROOT, &PLAY_KEYS.map(String::from));
        let canvas = canvas::create(ROOT, Rect::new(0, 0, m.width, m.height));

        let (title_h, pad) = if m.color {
            (m.title_height, 8)
        } else {
            (m.line_height, 4)
        };
        let w = if m.color { 300 } else { m.width * 3 / 4 };
        let h = title_h + m.line_height + 2 * pad;
        let plate = Rect::new(
            ((m.width - w) / 2) as i16,
            ((look.rules.height as u16 - h) / 2 + look.rules.bird_h as u16 / 2) as i16,
            w,
            h,
        );
        let text = |y: u16, h: u16, font: Font| {
            let t = ui::create(
                WidgetKind::Text,
                ROOT,
                Rect::new(plate.x, plate.y + y as i16, w, h),
            );
            ui::set_font(t, font);
            ui::set_align(t, Align::Center);
            t
        };
        let title = text(pad, title_h, if m.color { Font::Title } else { Font::Bold });
        let line = text(pad + title_h, m.line_height, Font::Normal);
        let art = Art {
            sparrow: look
                .feathers
                .iter()
                .map(|(mark, _)| layer(&SPARROW, *mark))
                .collect(),
            wing: layer(&WING, b'o'),
        };
        Screen {
            look,
            art,
            canvas,
            title,
            line,
            plate,
        }
    }

    fn texts(&self, lines: Option<(&str, &str)>) {
        for (w, text) in [self.title, self.line]
            .into_iter()
            .zip(lines.map(|l| [l.0, l.1]).unwrap_or_default())
        {
            ui::set_text(w, text);
        }
        ui::set_visible(self.title, lines.is_some());
        ui::set_visible(self.line, lines.is_some());
    }

    /// Draws the whole picture in one call.
    fn draw(&self, state: State, game: &Game) {
        let look = &self.look;
        let r = &look.rules;
        let mut ops = Vec::with_capacity(96);
        fill(
            &mut ops,
            0,
            r.height,
            look.width,
            look.height - r.height,
            look.floor,
        );
        fill(
            &mut ops,
            0,
            r.height,
            look.width,
            look.unit,
            look.floor_edge,
        );
        let p = &self.plate;
        let (px, py) = (i32::from(p.x), i32::from(p.y));
        match state {
            State::Ready => {
                // The sparrow waits on the plate.
                plate(
                    &mut ops,
                    look,
                    px,
                    py,
                    i32::from(p.width),
                    i32::from(p.height),
                );
                let x = px + (i32::from(p.width) - r.bird_w) / 2;
                sparrow(&mut ops, look, &self.art, x, py - r.bird_h, false);
            }
            State::Flying | State::Over(_) => {
                for pipe in &game.pipes {
                    obstacle(&mut ops, look, pipe);
                }
                let wing_up = game.flapped_at.is_some_and(|t| game.ticks - t < 2);
                sparrow(&mut ops, look, &self.art, r.bird_x, game.bird_y(), wing_up);
                if state == State::Flying {
                    score(&mut ops, look, game.score);
                } else {
                    plate(
                        &mut ops,
                        look,
                        px,
                        py,
                        i32::from(p.width),
                        i32::from(p.height),
                    );
                }
            }
        }
        if let Err(e) = canvas::draw(self.canvas, &ops) {
            app_sdk::log_warn(&format!("drawing refused: {e}"));
        }
    }
}

struct Sparrow {
    screen: Option<Screen>,
    state: State,
    game: Game,
    best: u32,
    /// Games started, so that two games begun in the same millisecond of
    /// two runs still differ.
    games: u32,
    timer: Option<u32>,
}

impl Default for Sparrow {
    fn default() -> Sparrow {
        Sparrow {
            screen: None,
            state: State::Ready,
            game: Game::new(Rules::COLOR, 1),
            best: 0,
            games: 0,
            timer: None,
        }
    }
}

impl Sparrow {
    fn stop_timer(&mut self) {
        if let Some(t) = self.timer.take() {
            timers::cancel(t);
        }
    }

    /// Shows the state: the texts and the picture.
    fn show(&mut self) {
        let screen = self.screen.get_or_insert_with(Screen::build);
        match self.state {
            State::Ready => screen.texts(Some(("Desk Sparrow", START_HINT))),
            State::Flying => screen.texts(None),
            State::Over(_) => {
                let line = format!("Score {}   Best {}", self.game.score, self.best);
                screen.texts(Some(("Game over", &line)));
            }
        }
        screen.draw(self.state, &self.game);
    }

    fn seed(&self) -> u32 {
        app_sdk::read_string(Location::Nv, SEED_FILE)
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or_else(|| (timers::now_ms() as u32) ^ self.games.wrapping_mul(0x9E37_79B9))
    }

    /// A score counts as the best as soon as it is one, and is written
    /// down then: once a game at most.
    fn keep_best(&mut self) {
        if self.game.score > self.best {
            self.best = self.game.score;
            if let Err(e) =
                storage::write(Location::Nv, BEST_FILE, self.best.to_string().as_bytes())
            {
                app_sdk::log_warn(&format!("the best score was not saved: {e:?}"));
            }
        }
    }

    /// One of the keys that play.
    fn play(&mut self) {
        let Some(rules) = self.screen.as_ref().map(|s| s.look.rules) else {
            return;
        };
        match self.state {
            State::Flying => self.game.flap(),
            State::Over(at) if timers::now_ms() < at + RESTART_AFTER_MS => {}
            State::Ready | State::Over(_) => {
                self.games += 1;
                self.game = Game::new(rules, self.seed());
                // The key that starts the game is its first flap.
                self.game.flap();
                self.state = State::Flying;
                self.stop_timer();
                self.timer = Some(timers::set(rules.tick_ms, true));
                self.show();
            }
        }
    }

    fn tick(&mut self) {
        if self.state != State::Flying {
            return;
        }
        self.game.step();
        if self.game.over {
            self.stop_timer();
            self.keep_best();
            self.state = State::Over(timers::now_ms());
            self.show();
        } else if let Some(screen) = &self.screen {
            screen.draw(self.state, &self.game);
        }
    }

    /// The game leaves the screen (a call, another screen, or Cancel): a
    /// game being played is over as it stands.
    fn leave(&mut self) {
        self.stop_timer();
        if self.state == State::Flying {
            self.keep_best();
        }
        self.state = State::Ready;
    }
}

impl App for Sparrow {
    fn start(&mut self) {
        // The whole display, from the first frame.
        ui::set_full_screen(true);
        self.best = app_sdk::read_string(Location::Nv, BEST_FILE)
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Foreground => self.show(),
            Event::Background | Event::Exit => self.leave(),
            Event::Timer(id) if self.timer == Some(id) => self.tick(),
            Event::Key(k) if PLAY_KEYS.contains(&k.as_str()) => self.play(),
            _ => {}
        }
    }
}

app_sdk::export_app!(Sparrow);
