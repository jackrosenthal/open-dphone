//! The pictures: where the board goes on a display, and its cells as
//! filled rectangles and one-color bitmaps (what the phone's app canvas
//! draws).  Everything is computed for the size of a cell, from 7 pixels
//! (14x14 on the smallest display) to 34 (7x7 on the color display).
//!
//! The board is drawn row by row ([`Painter::row`]), so that the app
//! sends again only the rows in which something changed
//! ([`Painter::row_state`] tells), and the cursor is a list of its own
//! ([`Painter::cursor`]), drawn over the rows.
//!
//! The numbers (the clues, and the digits of the picture of the keys) are
//! in the display's own font, the tallest that fits ([`Fonts`]); where
//! none is given or none fits, they are this module's bitmaps.
//!
//! On the color display a lit cell is yellow and what is wrong is red.
//! The gray displays get black and white only, so nothing depends on
//! telling two grays apart: a cell no light shines on is dotted, a lit
//! cell is plain white, a light is a black disk, a light that another
//! light shines on is a cross, a no-light mark is a small square, and a
//! numbered cell with too many lights is white with a black number
//! instead of black with a white one.  There are no grid lines there:
//! a black cell covers the lines around it, and the cursor is a box on
//! the lines around its cell, black around a white cell and white around
//! a black one, so it never touches what the cell shows.

use crate::{Game, Grid};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// One drawing operation, in pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Fill {
        x: i16,
        y: i16,
        width: u16,
        height: u16,
        color: Rgb,
    },
    /// A picture of one color: one bit per pixel, rows from the top, each
    /// row padded to whole bytes, the leftmost pixel in the highest bit,
    /// each pixel drawn as a square of side `scale`.  Clear bits leave
    /// what is underneath.
    Bitmap {
        x: i16,
        y: i16,
        width: u16,
        height: u16,
        scale: u8,
        bits: Rc<[u8]>,
        color: Rgb,
    },
    /// A character in a font of the display, in the middle of a box.
    Text {
        x: i16,
        y: i16,
        width: u16,
        height: u16,
        font: TextFont,
        text: char,
        color: Rgb,
    },
}

/// The fonts of a display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextFont {
    Small,
    Normal,
    Bold,
    Title,
    Large,
}

/// A font of the display, and the width and height of a digit in it (of
/// the digit's own pixels).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontSize {
    pub font: TextFont,
    pub width: u16,
    pub height: u16,
}

/// The fonts a picture may use for its digits; none, and digits are
/// this module's own bitmaps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Fonts {
    fonts: [Option<FontSize>; 5],
}

impl Fonts {
    /// The fonts, the preferred one first where two are equally tall.
    pub fn new(fonts: &[FontSize]) -> Fonts {
        let mut out = Fonts::default();
        for (slot, font) in out.fonts.iter_mut().zip(fonts) {
            *slot = Some(*font);
        }
        out
    }

    /// The tallest font whose digits are no larger than this.
    fn fitting(&self, width: u16, height: u16) -> Option<FontSize> {
        let mut best: Option<FontSize> = None;
        for f in self.fonts.iter().flatten() {
            if f.width > 0
                && f.width <= width
                && f.height <= height
                && best.is_none_or(|b| f.height > b.height)
            {
                best = Some(*f);
            }
        }
        best
    }
}

/// Where things go in an app window: the board on the left, as large as
/// the height allows, and a panel for texts on the right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// From one cell to the next, in pixels: the cell and one grid line.
    pub pitch: u16,
    pub board_x: i16,
    pub board_y: i16,
    /// The board is square: the cells and the grid lines around them.
    pub board_side: u16,
    pub panel_x: i16,
    pub panel_width: u16,
}

impl Layout {
    /// The layout of a board of `cells` by `cells` in a window: e.g., 14
    /// cells get 7 pixels each in the smallest window (240x102), 9 in
    /// 320x140, and 17 in 480x243.
    pub fn new(width: u16, height: u16, cells: usize) -> Layout {
        let cells = cells.max(1) as u16;
        let pitch = (height.min(width).saturating_sub(1) / cells).max(4);
        let board_side = pitch * cells + 1;
        Layout {
            pitch,
            board_x: 0,
            board_y: (height.saturating_sub(board_side) / 2) as i16,
            board_side,
            panel_x: board_side as i16,
            panel_width: width.saturating_sub(board_side),
        }
    }
}

/// The colors of the pictures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// Black and white only, with patterns where the color display has
    /// colors.
    pub two_tone: bool,
    /// The display's fonts, for the digits.
    pub fonts: Fonts,
    pub line: Rgb,
    pub black: Rgb,
    pub number: Rgb,
    pub unlit: Rgb,
    pub lit: Rgb,
    pub light: Rgb,
    pub outline: Rgb,
    pub mark: Rgb,
    pub error: Rgb,
    pub cursor: Rgb,
}

impl Palette {
    pub fn color() -> Palette {
        Palette {
            two_tone: false,
            fonts: Fonts::default(),
            line: Rgb(0x6E, 0x6E, 0x6E),
            black: Rgb(0x1E, 0x1E, 0x1E),
            number: Rgb(0xFF, 0xFF, 0xFF),
            unlit: Rgb(0xC4, 0xC4, 0xC4),
            lit: Rgb(0xFF, 0xF0, 0x70),
            light: Rgb(0xFF, 0xA8, 0x00),
            outline: Rgb(0x1E, 0x1E, 0x1E),
            mark: Rgb(0x30, 0x30, 0x30),
            error: Rgb(0xE0, 0x20, 0x20),
            cursor: Rgb(0x10, 0x60, 0xF0),
        }
    }

    /// The palette of a gray display, from its foreground (black) and
    /// background (white).
    pub fn gray(fg: Rgb, bg: Rgb) -> Palette {
        Palette {
            two_tone: true,
            fonts: Fonts::default(),
            line: bg,
            black: fg,
            number: bg,
            unlit: bg,
            lit: bg,
            light: fg,
            outline: fg,
            mark: fg,
            error: fg,
            cursor: fg,
        }
    }
}

/// What a cell shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Black,
    Clue { number: u8, error: bool },
    Unlit { marked: bool },
    Lit { marked: bool },
    Light { error: bool },
}

impl Look {
    pub fn of(grid: &Grid, i: usize) -> Look {
        if let Some(number) = grid.clue(i) {
            Look::Clue {
                number,
                error: grid.clue_overfull(i),
            }
        } else if grid.is_black(i) {
            Look::Black
        } else if grid.has_light(i) {
            Look::Light {
                error: grid.light_conflicts(i),
            }
        } else if grid.is_lit(i) {
            Look::Lit {
                marked: grid.is_marked(i),
            }
        } else {
            Look::Unlit {
                marked: grid.is_marked(i),
            }
        }
    }
}

/// A one-color picture.
#[derive(Clone)]
struct Picture {
    width: u16,
    height: u16,
    bits: Rc<[u8]>,
}

impl Picture {
    fn new(width: u16, height: u16, set: impl Fn(i32, i32) -> bool) -> Picture {
        let stride = (width as usize).div_ceil(8);
        let mut bits = vec![0u8; stride * height as usize];
        for y in 0..height as usize {
            for x in 0..width as usize {
                if set(x as i32, y as i32) {
                    bits[y * stride + x / 8] |= 0x80 >> (x % 8);
                }
            }
        }
        Picture {
            width,
            height,
            bits: bits.into(),
        }
    }

    fn rows(rows: &[&str]) -> Picture {
        Picture::new(rows[0].len() as u16, rows.len() as u16, |x, y| {
            rows[y as usize].as_bytes()[x as usize] == b'#'
        })
    }

    fn op(&self, x: i16, y: i16, scale: u8, color: Rgb) -> Op {
        Op::Bitmap {
            x,
            y,
            width: self.width,
            height: self.height,
            scale,
            bits: self.bits.clone(),
            color,
        }
    }
}

/// The digits 0 to 8, 5 by 7 (the board shows 0 to 4; the picture of the
/// keys shows the keys' own).
const DIGITS: [[&str; 7]; 9] = [
    [
        ".###.", "#...#", "#..##", "#.#.#", "##..#", "#...#", ".###.",
    ],
    [
        "..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###.",
    ],
    [
        ".###.", "#...#", "....#", "...#.", "..#..", ".#...", "#####",
    ],
    [
        ".###.", "#...#", "....#", "..##.", "....#", "#...#", ".###.",
    ],
    [
        "...#.", "..##.", ".#.#.", "#..#.", "#####", "...#.", "...#.",
    ],
    [
        "#####", "#....", "####.", "....#", "....#", "#...#", ".###.",
    ],
    [
        ".###.", "#....", "####.", "#...#", "#...#", "#...#", ".###.",
    ],
    [
        "#####", "....#", "...#.", "..#..", "..#..", "..#..", "..#..",
    ],
    [
        ".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###.",
    ],
];

/// The digits 0 to 4, 3 by 5, for cells too small for the others.
const SMALL_DIGITS: [[&str; 5]; 5] = [
    ["###", "#.#", "#.#", "#.#", "###"],
    [".#.", "##.", ".#.", ".#.", "###"],
    ["##.", "..#", ".#.", "#..", "###"],
    ["###", "..#", ".##", "..#", "###"],
    ["#.#", "#.#", "###", "..#", "..#"],
];

/// A filled circle of this diameter.
fn disk(d: u16) -> Picture {
    let d2 = (d as i32) * (d as i32);
    Picture::new(d, d, |x, y| {
        let (cx, cy) = (2 * x + 1 - d as i32, 2 * y + 1 - d as i32);
        cx * cx + cy * cy <= d2
    })
}

/// A diagonal cross of this side and line width.
fn cross(side: u16, width: u16) -> Picture {
    let (s, w) = (side as i32, width as i32);
    Picture::new(side, side, |x, y| {
        (x - y).abs() < w || (x + y - (s - 1)).abs() < w
    })
}

/// Draws the cells of a board at one size, in one palette.
pub struct Painter {
    cells: usize,
    pitch: u16,
    palette: Palette,
    digits: Vec<Picture>,
    digit_scale: u8,
    digit_at: (i16, i16),
    /// The display's font for the digits, if one fits the cells.
    digit_font: Option<TextFont>,
    light: Picture,
    light_inner: Picture,
    light_at: i16,
    cross: Picture,
    dots: Picture,
    mark_side: u16,
    /// How many pixels of a cell the cursor's frame covers, inward from
    /// the grid line.
    cursor_inset: u16,
}

impl Painter {
    /// A painter for a board of `cells` by `cells` whose cells are
    /// `pitch` pixels apart.
    pub fn new(cells: usize, pitch: u16, palette: Palette) -> Painter {
        // The inside of a cell, between its grid lines.
        let inside = pitch - 1;
        let small = inside < 7;
        let digit_scale = if small {
            1
        } else {
            (inside / 7).clamp(1, 8) as u8
        };
        let digits: Vec<Picture> = if small {
            SMALL_DIGITS.iter().map(|d| Picture::rows(d)).collect()
        } else {
            DIGITS.iter().map(|d| Picture::rows(d)).collect()
        };
        let (dw, dh) = (
            digits[0].width * digit_scale as u16,
            digits[0].height * digit_scale as u16,
        );
        // A light fills the cell but for a margin; with an outline where
        // there is room and color for one.
        let margin = (inside / 8).max(if inside < 12 { 1 } else { 2 });
        let d = inside - 2 * margin;
        let ring = if palette.two_tone {
            0
        } else {
            (inside / 12).max(1)
        };
        let period = if inside < 12 { 3 } else { 4 };
        // A font of the display for the numbers, the tallest that fits:
        // inside the frame of a wrong number in black and white, and
        // with a pixel to spare above and below where the cell has
        // them.  (A black cell there covers the lines around it, so a
        // digit as tall as a small cell still has black around it.)
        let room = if !palette.two_tone {
            inside - 2
        } else if inside >= 12 {
            inside - 2 * (inside / 12) - 2
        } else {
            inside
        };
        let digit_font = palette.fonts.fitting(room, room).map(|f| f.font);
        Painter {
            cells,
            pitch,
            palette,
            digits,
            digit_scale,
            digit_at: (
                1 + ((inside - dw.min(inside)) / 2) as i16,
                1 + ((inside - dh.min(inside)).div_ceil(2)) as i16,
            ),
            digit_font,
            light: disk(d),
            light_inner: disk(d - 2 * ring),
            light_at: 1 + margin as i16,
            cross: cross(d, ((d + 2) / 5).max(1)),
            // Placed so that a marked dotted cell is not the picture of a
            // cross, even in 6 by 6 pixels.
            dots: Picture::new(inside, inside, |x, y| {
                x % period == period - 3 && y % period == period - 3
            }),
            mark_side: (inside / 3).max(2),
            cursor_inset: if palette.two_tone {
                (inside >= 12) as u16
            } else {
                1 + pitch / 12
            },
        }
    }

    pub fn pitch(&self) -> u16 {
        self.pitch
    }

    /// Where a row is drawn on the board: x, y, width, and height.  A row
    /// has the grid line above it, and the last row the one below too.
    pub fn row_bounds(&self, row: usize) -> (i16, i16, u16, u16) {
        let last = (row + 1 == self.cells) as u16;
        (
            0,
            (row as u16 * self.pitch) as i16,
            self.cells as u16 * self.pitch + 1,
            self.pitch + last,
        )
    }

    /// What a row shows; two rows with the same state are drawn alike.
    pub fn row_state(&self, grid: &Grid, row: usize) -> Vec<Look> {
        (0..grid.width())
            .map(|x| Look::of(grid, grid.index(x, row)))
            .collect()
    }

    /// One row, in pixels from the top left of [`Painter::row_bounds`].
    pub fn row(&self, grid: &Grid, row: usize) -> Vec<Op> {
        let p = &self.palette;
        let (_, _, width, height) = self.row_bounds(row);
        let inside = self.pitch - 1;
        // The grid lines, with the cells painted over them.
        let mut ops = vec![Op::Fill {
            x: 0,
            y: 0,
            width,
            height,
            color: p.line,
        }];
        if p.two_tone {
            // No lines, but each black cell covers the lines around it:
            // those of this row's cells, and the line under the cells of
            // the row above.
            let last = (row + 1 == self.cells) as u16;
            for x in 0..grid.width() {
                let left = (x as u16 * self.pitch) as i16;
                let mut cover = |height| {
                    ops.push(Op::Fill {
                        x: left,
                        y: 0,
                        width: self.pitch + 1,
                        height,
                        color: p.black,
                    })
                };
                if grid.is_black(grid.index(x, row)) {
                    cover(self.pitch + last);
                } else if row > 0 && grid.is_black(grid.index(x, row - 1)) {
                    cover(1);
                }
            }
        }
        for x in 0..grid.width() {
            let left = (x as u16 * self.pitch) as i16 + 1;
            let fill = |color| Op::Fill {
                x: left,
                y: 1,
                width: inside,
                height: inside,
                color,
            };
            let square = |side: u16, color| Op::Fill {
                x: left + ((inside - side) / 2) as i16,
                y: 1 + ((inside - side) / 2) as i16,
                width: side,
                height: side,
                color,
            };
            match Look::of(grid, grid.index(x, row)) {
                Look::Black => {
                    if !p.two_tone {
                        ops.push(fill(p.black));
                    }
                }
                Look::Clue { number, error } => {
                    // In the display's font where one fits the cell; else
                    // this module's own digits.
                    let digit = |color| match self.digit_font {
                        Some(font) => Op::Text {
                            x: left,
                            y: 1,
                            width: inside,
                            height: inside,
                            font,
                            text: (b'0' + number.min(9)) as char,
                            color,
                        },
                        None => self.digits[(number as usize).min(4)].op(
                            left - 1 + self.digit_at.0,
                            self.digit_at.1,
                            self.digit_scale,
                            color,
                        ),
                    };
                    if !error {
                        if !p.two_tone {
                            ops.push(fill(p.black));
                        }
                        ops.push(digit(p.number));
                    } else if p.two_tone {
                        // Inverted: black on white, in a black frame
                        // where there is room for one.
                        let frame = if inside >= 12 { inside / 12 } else { 0 };
                        ops.push(square(inside - 2 * frame, p.number));
                        ops.push(digit(p.black));
                    } else {
                        ops.push(fill(p.error));
                        ops.push(digit(p.number));
                    }
                }
                Look::Unlit { marked } => {
                    if p.two_tone {
                        ops.push(self.dots.op(left, 1, 1, p.black));
                    } else {
                        ops.push(fill(p.unlit));
                    }
                    if marked {
                        ops.push(square(self.mark_side, p.mark));
                    }
                }
                Look::Lit { marked } => {
                    if !p.two_tone {
                        ops.push(fill(p.lit));
                    }
                    if marked {
                        ops.push(square(self.mark_side, p.mark));
                    }
                }
                Look::Light { error } => {
                    if !p.two_tone {
                        ops.push(fill(p.lit));
                    }
                    let at = left - 1 + self.light_at;
                    if p.two_tone {
                        let picture = if error { &self.cross } else { &self.light };
                        ops.push(picture.op(at, self.light_at, 1, p.light));
                    } else {
                        ops.push(self.light.op(at, self.light_at, 1, p.outline));
                        let ring = ((self.light.width - self.light_inner.width) / 2) as i16;
                        let color = if error { p.error } else { p.light };
                        ops.push(
                            self.light_inner
                                .op(at + ring, self.light_at + ring, 1, color),
                        );
                    }
                }
            }
        }
        ops
    }

    /// The cursor: a frame around its cell, in pixels from the top left
    /// of the board.  Nothing once the puzzle is solved.
    pub fn cursor(&self, game: &Game) -> Vec<Op> {
        if game.solved() {
            return Vec::new();
        }
        let p = &self.palette;
        let grid = game.grid();
        let (cx, cy) = game.cursor();
        let (x, y) = (
            (cx as u16 * self.pitch) as i16,
            (cy as u16 * self.pitch) as i16,
        );
        // The frame covers the grid lines around the cell and a little
        // of the cell.
        let frame = |inset: u16, thick: u16, color: Rgb| -> [Op; 4] {
            let (i, side) = (inset as i16, self.pitch + 1 - 2 * inset);
            let bar = |x, y, width, height| Op::Fill {
                x,
                y,
                width,
                height,
                color,
            };
            [
                bar(x + i, y + i, side, thick),
                bar(x + i, y + i + (side - thick) as i16, side, thick),
                bar(x + i, y + i, thick, side),
                bar(x + i + (side - thick) as i16, y + i, thick, side),
            ]
        };
        let thick = self.cursor_inset + 1;
        if !p.two_tone {
            return frame(0, thick, p.cursor).into();
        }
        // In black and white: on the lines around the cell, black around
        // a white cell and white around a black one.
        let black = grid.is_black(grid.index(cx, cy));
        frame(0, thick, if black { p.number } else { p.black }).into()
    }
}

/// The operations moved by (`dx`, `dy`).
pub fn shifted(ops: Vec<Op>, dx: i16, dy: i16) -> Vec<Op> {
    ops.into_iter()
        .map(|op| match op {
            Op::Fill {
                x,
                y,
                width,
                height,
                color,
            } => Op::Fill {
                x: x + dx,
                y: y + dy,
                width,
                height,
                color,
            },
            Op::Bitmap {
                x,
                y,
                width,
                height,
                scale,
                bits,
                color,
            } => Op::Bitmap {
                x: x + dx,
                y: y + dy,
                width,
                height,
                scale,
                bits,
                color,
            },
            Op::Text {
                x,
                y,
                width,
                height,
                font,
                text,
                color,
            } => Op::Text {
                x: x + dx,
                y: y + dy,
                width,
                height,
                font,
                text,
                color,
            },
        })
        .collect()
}

/// A grid drawn whole, as the board draws it but without a cursor: its
/// width and height in pixels, and the operations from its top left.
pub fn picture(grid: &Grid, pitch: u16, palette: Palette) -> (u16, u16, Vec<Op>) {
    // The painter draws square boards; a row is as wide as the grid.
    let painter = Painter::new(grid.width(), pitch, palette);
    let mut ops = Vec::new();
    for row in 0..grid.height() {
        let mut list = painter.row(grid, row);
        if row + 1 == grid.height() && grid.height() != grid.width() {
            // The line under the last row, which the painter draws only
            // under the last row of a square board.
            if let Some(Op::Fill { height, .. }) = list.first_mut() {
                *height = pitch + 1;
            }
            if !palette.two_tone {
                list.insert(
                    1,
                    Op::Fill {
                        x: 0,
                        y: pitch as i16,
                        width: grid.width() as u16 * pitch + 1,
                        height: 1,
                        color: palette.line,
                    },
                );
            }
        }
        ops.extend(shifted(list, 0, (row as u16 * pitch) as i16));
    }
    (
        grid.width() as u16 * pitch + 1,
        grid.height() as u16 * pitch + 1,
        ops,
    )
}

/// A picture with a frame around it, two pixels away: for a display
/// without colors, where a lit cell is as white as the page.
pub fn framed(width: u16, height: u16, ops: Vec<Op>, ink: Rgb) -> (u16, u16, Vec<Op>) {
    let (w, h) = (width + 6, height + 6);
    let bar = |x, y, width, height| Op::Fill {
        x,
        y,
        width,
        height,
        color: ink,
    };
    let mut all = vec![
        bar(0, 0, w, 1),
        bar(0, h as i16 - 1, w, 1),
        bar(0, 0, 1, h),
        bar(w as i16 - 1, 0, 1, h),
    ];
    all.extend(shifted(ops, 3, 3));
    (w, h, all)
}

/// What a key of the keypad does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyUse {
    Up,
    Down,
    Left,
    Right,
    /// Puts a light: shown as a cell with a light.
    Light,
    /// Marks "no light": shown as a cell with the square.
    Mark,
    /// Clears: shown as an empty cell.
    Clear,
}

/// The keys of the game where they are on the keypad (column and row of
/// the three columns of digits), each with its digit and what it does.
pub const KEYS: [(u8, u8, u8, KeyUse); 7] = [
    (1, 0, 0, KeyUse::Clear),
    (2, 1, 0, KeyUse::Up),
    (3, 2, 0, KeyUse::Mark),
    (4, 0, 1, KeyUse::Left),
    (5, 1, 1, KeyUse::Light),
    (6, 2, 1, KeyUse::Right),
    (8, 1, 2, KeyUse::Down),
];

/// A picture of the keys.
pub struct Keymap {
    pub width: u16,
    pub height: u16,
    /// The size of a key, and the space between two.
    pub key: (u16, u16),
    pub gap: u16,
    pub ops: Vec<Op>,
}

/// A triangle pointing up, in a square of odd side `t`.
fn arrow(t: u16, dir: KeyUse) -> Picture {
    let t = t as i32;
    let (rows, center) = ((t + 1) / 2, (t - 1) / 2);
    let top = (t - rows) / 2;
    Picture::new(t as u16, t as u16, |x, y| {
        // Along the arrow (from its point), and across it.
        let (along, across) = match dir {
            KeyUse::Up => (y, x),
            KeyUse::Down => (t - 1 - y, x),
            KeyUse::Left => (x, y),
            _ => (t - 1 - x, y),
        };
        let r = along - top;
        (0..rows).contains(&r) && (across - center).abs() <= r
    })
}

/// The picture of the keys for a space of `width` by `height` pixels: the
/// seven keys as on the keypad, three across, each a frame with its digit
/// and, beside it, what it does: an arrow, or the cell as the board would
/// show it (with a light, with the no-light square, or empty).  `ink` is
/// the color of frames, digits, and arrows.  The keys are as large as the
/// space allows, up to a size that suits the largest display.  The digits
/// are in the display's font where the palette names one that fits.
pub fn keymap(width: u16, height: u16, palette: Palette, ink: Rgb) -> Keymap {
    /// For a height of the keys: the width of a key, the cell beside the
    /// digit, the digit's font (or the scale of this module's digits) and
    /// its width, the space around things, and between keys.
    type Sizes = (u16, u16, Option<TextFont>, u16, u16, u16, u16);
    let fits = |key_height: u16| -> Option<Sizes> {
        let scale: u16 = if key_height >= 36 { 2 } else { 1 };
        let pad = 2 + scale;
        // The cell beside the digit, with its grid lines.
        let cell = key_height.checked_sub(2 * pad + 2)?;
        // The digit in the display's font, about as tall as the cell.
        let font = palette.fonts.fitting(cell, cell.saturating_sub(2));
        let digit_width = font.map_or(5 * scale, |f| f.width);
        let key_width = digit_width + cell + 3 * pad + 2;
        let gap = 1 + scale;
        (cell >= 7 && 3 * key_width + 2 * gap <= width && 3 * key_height + 2 * gap <= height)
            .then_some((
                key_width,
                cell,
                font.map(|f| f.font),
                digit_width,
                scale,
                pad,
                gap,
            ))
    };
    let Some((key_height, (key_width, cell, font, digit_width, scale, pad, gap))) =
        (13..=46).rev().find_map(|h| Some((h, fits(h)?)))
    else {
        return Keymap {
            width: 0,
            height: 0,
            key: (0, 0),
            gap: 0,
            ops: Vec::new(),
        };
    };
    let painter = Painter::new(1, cell - 1, palette);
    let digits: Vec<Picture> = DIGITS.iter().map(|d| Picture::rows(d)).collect();
    let line = scale;
    let mut ops = Vec::new();
    for (digit, column, row, what) in KEYS {
        let x = (column as u16 * (key_width + gap)) as i16;
        let y = (row as u16 * (key_height + gap)) as i16;
        let mut bar = |x, y, width, height| {
            ops.push(Op::Fill {
                x,
                y,
                width,
                height,
                color: ink,
            })
        };
        bar(x, y, key_width, line);
        bar(x, y + (key_height - line) as i16, key_width, line);
        bar(x, y, line, key_height);
        bar(x + (key_width - line) as i16, y, line, key_height);
        let inside = (line + pad) as i16;
        ops.push(match font {
            Some(font) => Op::Text {
                x: x + inside,
                y,
                width: digit_width,
                height: key_height,
                font,
                text: (b'0' + digit) as char,
                color: ink,
            },
            None => {
                let digit_y = y + ((key_height - 7 * scale) / 2) as i16;
                digits[digit as usize].op(x + inside, digit_y, scale as u8, ink)
            }
        });
        let (cx, cy) = (
            x + inside + (digit_width + pad) as i16,
            y + ((key_height - cell) / 2) as i16,
        );
        let shown = match what {
            KeyUse::Light => "L",
            KeyUse::Mark => "x",
            KeyUse::Clear => ".",
            _ => {
                let t = if cell % 2 == 1 { cell } else { cell - 1 };
                ops.push(arrow(t, what).op(cx, cy, 1, ink));
                continue;
            }
        };
        let grid = Grid::parse(shown).expect("a cell");
        ops.extend(shifted(painter.row(&grid, 0), cx, cy));
    }
    Keymap {
        width: 3 * key_width + 2 * gap,
        height: 3 * key_height + 2 * gap,
        key: (key_width, key_height),
        gap,
        ops,
    }
}
