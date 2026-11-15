//! The pictures: the sizes on each display, what the canvas accepts, and
//! that everything a cell can show looks different from everything else,
//! in color and in black and white.

use lightup_game::draw::{Layout, Look, Op, Painter, Palette, Rgb};
use lightup_game::{Game, Grid};
use std::collections::BTreeSet;

const BLACK: Rgb = Rgb(0, 0, 0);
const WHITE: Rgb = Rgb(255, 255, 255);

/// The three app windows: small gray, large gray, and color.
const WINDOWS: [(u16, u16, bool); 3] = [(240, 102, false), (320, 140, false), (480, 243, true)];

fn palette(color: bool) -> Palette {
    if color {
        Palette::color()
    } else {
        Palette::gray(BLACK, WHITE)
    }
}

/// The pixels a list of operations paints on a `width` by `height`
/// canvas that starts out in `ground`.
fn paint(ops: &[Op], width: usize, height: usize, ground: Rgb) -> Vec<Rgb> {
    let mut px = vec![ground; width * height];
    let mut set = |x: i32, y: i32, c: Rgb| {
        assert!(
            x >= 0 && y >= 0 && (x as usize) < width && (y as usize) < height,
            "({x}, {y}) is outside"
        );
        px[y as usize * width + x as usize] = c;
    };
    for op in ops {
        match op {
            Op::Fill {
                x,
                y,
                width,
                height,
                color,
            } => {
                for yy in 0..*height as i32 {
                    for xx in 0..*width as i32 {
                        set(*x as i32 + xx, *y as i32 + yy, *color);
                    }
                }
            }
            Op::Bitmap {
                x,
                y,
                width,
                height,
                scale,
                bits,
                color,
            } => {
                let stride = (*width as usize).div_ceil(8);
                assert_eq!(bits.len(), stride * *height as usize);
                assert!((1..=8).contains(scale) && *width <= 256 && *height <= 256);
                for yy in 0..*height as usize {
                    for xx in 0..*width as usize {
                        if bits[yy * stride + xx / 8] & (0x80 >> (xx % 8)) != 0 {
                            for sy in 0..*scale as i32 {
                                for sx in 0..*scale as i32 {
                                    let s = *scale as i32;
                                    set(
                                        *x as i32 + xx as i32 * s + sx,
                                        *y as i32 + yy as i32 * s + sy,
                                        *color,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    px
}

#[test]
fn the_board_fills_the_height_of_each_window() {
    let pitch = |w, h, n| {
        let l = Layout::new(w, h, n);
        assert_eq!(l.board_side, l.pitch * n as u16 + 1);
        assert!(l.board_side <= h && l.board_y >= 0 && l.board_y as u16 + l.board_side <= h);
        assert_eq!(l.panel_x as u16 + l.panel_width, w);
        // Room for the texts beside the board.
        assert!(l.panel_width >= 139, "{w}x{h} {n}: {}", l.panel_width);
        l.pitch
    };
    assert_eq!([7, 10, 14].map(|n| pitch(240, 102, n)), [14, 10, 7]);
    assert_eq!([7, 10, 14].map(|n| pitch(320, 140, n)), [19, 13, 9]);
    assert_eq!([7, 10, 14].map(|n| pitch(480, 243, n)), [34, 24, 17]);
}

/// A grid of the given side that shows every look: a row of
/// `# 0 1 2 3 4`, an overfull 0 with its light, two lights that see each
/// other, a lone light, marks on lit and unlit cells.
fn sampler(side: usize) -> Grid {
    let mut rows = vec![".".repeat(side); side];
    rows[0] = format!("#01234{}", ".".repeat(side - 6));
    rows[1] = format!("xL{}", ".".repeat(side - 2));
    rows[2] = format!("{}#", "#".repeat(side - 1));
    rows[3] = format!("L.x.L#{}", ".".repeat(side - 6));
    rows[4] = format!("{}#", "#".repeat(side - 1));
    rows[5] = format!("x.{}", "#".repeat(side - 2));
    Grid::parse(&rows.join("\n")).unwrap()
}

#[test]
fn every_look_is_its_own_picture() {
    for (w, h, color) in WINDOWS {
        for side in [7, 10, 14] {
            let what = format!("{w}x{h} {side}");
            let l = Layout::new(w, h, side);
            let painter = Painter::new(side, l.pitch, palette(color));
            let g = sampler(side);
            let pitch = l.pitch as usize;
            let mut seen: Vec<(Look, Vec<Rgb>)> = Vec::new();
            for row in 0..side {
                let (x, y, width, height) = painter.row_bounds(row);
                assert_eq!(
                    (x, y as usize, width),
                    (0, row * pitch, l.board_side),
                    "{what}"
                );
                assert_eq!(height as usize, pitch + (row + 1 == side) as usize);
                let ops = painter.row(&g, row);
                // What the canvas takes in one call.
                assert!(ops.len() <= 100, "{what}: {} operations", ops.len());
                let bytes: usize = ops
                    .iter()
                    .map(|op| match op {
                        Op::Bitmap { bits, .. } => bits.len(),
                        _ => 0,
                    })
                    .sum();
                assert!(bytes <= 4096, "{what}: {bytes} bytes of bitmaps");
                let px = paint(&ops, width as usize, height as usize, Rgb(1, 2, 3));
                assert!(
                    !px.contains(&Rgb(1, 2, 3)),
                    "{what}: a row paints all of itself"
                );
                if !color {
                    let colors: BTreeSet<(u8, u8, u8)> =
                        px.iter().map(|c| (c.0, c.1, c.2)).collect();
                    assert!(
                        colors
                            .iter()
                            .all(|c| *c == (0, 0, 0) || *c == (255, 255, 255)),
                        "{what}: {colors:?}"
                    );
                }
                // The inside of each cell, between the grid lines.
                for col in 0..side {
                    let look = Look::of(&g, g.index(col, row));
                    let inside: Vec<Rgb> = (1..pitch)
                        .flat_map(|yy| (1..pitch).map(move |xx| (xx, yy)))
                        .map(|(xx, yy)| px[yy * width as usize + col * pitch + xx])
                        .collect();
                    match seen.iter().find(|(l, _)| *l == look) {
                        Some((_, picture)) => {
                            assert_eq!(*picture, inside, "{what}: {look:?} is drawn one way")
                        }
                        None => seen.push((look, inside)),
                    }
                }
                assert_eq!(
                    painter.row_state(&g, row),
                    (0..side)
                        .map(|c| Look::of(&g, g.index(c, row)))
                        .collect::<Vec<_>>()
                );
            }
            // Black, five numbers, the overfull 0, unlit and lit, each
            // plain and marked, a light, and a light in error.
            assert_eq!(
                seen.len(),
                12,
                "{what}: {:?}",
                seen.iter().map(|s| s.0).collect::<Vec<_>>()
            );
            assert!(seen.iter().any(|s| s.0
                == Look::Clue {
                    number: 0,
                    error: true
                }));
            assert!(seen.iter().any(|s| s.0 == Look::Light { error: true }));
            for (i, a) in seen.iter().enumerate() {
                for b in &seen[i + 1..] {
                    let differ = a.1.iter().zip(&b.1).filter(|(p, q)| p != q).count();
                    // Told apart by more than a pixel or two, even in the
                    // 6 by 6 pixels of the smallest cell.
                    assert!(
                        differ >= 3,
                        "{what}: {:?} and {:?} differ in {differ} pixels",
                        a.0,
                        b.0
                    );
                }
            }
        }
    }
}

#[test]
fn the_cursor_is_a_frame_that_stays_clear_of_what_the_cell_shows() {
    for (w, h, color) in WINDOWS {
        for side in [7, 10, 14] {
            let what = format!("{w}x{h} {side}");
            let l = Layout::new(w, h, side);
            let painter = Painter::new(side, l.pitch, palette(color));
            let (pitch, board) = (l.pitch as usize, l.board_side as usize);
            let mut game = Game::new(sampler(side));
            assert!(!game.solved());
            for (col, row) in [
                (0, 0),
                (1, 0),
                (1, 1),
                (0, 3),
                (2, 3),
                (1, 5),
                (side - 1, side - 1),
            ] {
                game.set_cursor(col, row);
                let ops = painter.cursor(&game);
                assert!(!ops.is_empty() && ops.len() <= 8);
                let ground = Rgb(1, 2, 3);
                let px = paint(&ops, board, board, ground);
                // Within the cell and the grid lines around it.
                let (x0, y0) = (col * pitch, row * pitch);
                let mut painted = 0;
                for y in 0..board {
                    for x in 0..board {
                        if px[y * board + x] != ground {
                            painted += 1;
                            assert!(
                                (x0..=x0 + pitch).contains(&x) && (y0..=y0 + pitch).contains(&y),
                                "{what}"
                            );
                        }
                    }
                }
                assert!(painted >= 4 * pitch, "{what}: {painted} pixels");
                // What the cell shows is not touched: the frame leaves
                // the middle alone, and in black and white, where a
                // light is as black as the frame, a clear pixel between.
                let row_px = paint(
                    &painter.row(game.grid(), row),
                    board,
                    pitch + (row + 1 == side) as usize,
                    ground,
                );
                let cell = Look::of(game.grid(), game.grid().index(col, row));
                if !color && matches!(cell, Look::Light { .. }) {
                    for y in 1..pitch {
                        for x in 1..pitch {
                            if row_px[y * board + x0 + x] == BLACK {
                                for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
                                    assert_eq!(
                                        px[(y0 + ny) * board + x0 + nx],
                                        ground,
                                        "{what}: the frame touches the light"
                                    );
                                }
                            }
                        }
                    }
                }
                // In black and white the frame is black around a white
                // cell and white around a black one.
                if !color {
                    let frame = if game.grid().is_black(game.grid().index(col, row)) {
                        WHITE
                    } else {
                        BLACK
                    };
                    assert!(px.iter().all(|p| *p == ground || *p == frame), "{what}");
                }
            }
        }
    }
    // A solved puzzle has no cursor.
    let painter = Painter::new(2, 14, palette(false));
    assert!(
        painter
            .cursor(&Game::new(Grid::parse("L.\n.L").unwrap()))
            .is_empty()
    );
}

#[test]
fn the_picture_of_the_keys_fits_beside_every_board() {
    use lightup_game::draw::{KEYS, keymap};
    // The line height of each display: the panel has half a line of
    // margin on each side, and two pixels above and below.
    for ((w, h, color), line) in WINDOWS.into_iter().zip([15u16, 17, 25]) {
        for side in [7, 10, 14] {
            let what = format!("{w}x{h} {side}");
            let l = Layout::new(w, h, side);
            let room = (l.panel_width - 2 * (line / 2), h - 4);
            let map = keymap(
                room.0,
                room.1,
                palette(color),
                if color { WHITE } else { BLACK },
            );
            assert!(
                map.width > 0 && map.width <= room.0 && map.height <= room.1,
                "{what}: {}x{}",
                map.width,
                map.height
            );
            // Every key is at least 24 pixels high, even beside the
            // largest board on the smallest display.
            assert!(map.height >= 3 * 24, "{what}: {}", map.height);
            assert!(map.ops.len() <= 100);
            let ground = Rgb(1, 2, 3);
            let px = paint(&map.ops, map.width as usize, map.height as usize, ground);
            // Seven keys where the keypad has them: something is drawn
            // in each of their places and nothing in the two empty ones.
            let (kw, kh, gap) = (map.key.0 as usize, map.key.1 as usize, map.gap as usize);
            for row in 0..3 {
                for column in 0..3 {
                    let (x0, y0) = (column * (kw + gap), row * (kh + gap));
                    let painted = (0..kh)
                        .flat_map(|y| (0..kw).map(move |x| (x, y)))
                        .filter(|(x, y)| px[(y0 + y) * map.width as usize + x0 + x] != ground)
                        .count();
                    let key = KEYS
                        .iter()
                        .any(|k| (k.1 as usize, k.2 as usize) == (column, row));
                    assert_eq!(
                        painted > 40,
                        key,
                        "{what}: key at {column}, {row}: {painted}"
                    );
                    assert!(key || painted == 0, "{what}");
                }
            }
            if !color {
                assert!(
                    px.iter()
                        .all(|p| *p == ground || *p == BLACK || *p == WHITE),
                    "{what}"
                );
            } else {
                // The light's key shows the board's lit cell and light.
                let c = Palette::color();
                assert!(
                    px.contains(&c.lit)
                        && px.contains(&c.light)
                        && px.contains(&c.mark)
                        && px.contains(&c.unlit)
                );
            }
        }
    }
    assert_eq!(KEYS.map(|k| k.0), [1, 2, 3, 4, 5, 6, 8]);
    // No room, no picture.
    assert!(keymap(30, 30, palette(false), BLACK).ops.is_empty());
}

#[test]
fn the_help_pictures_show_what_their_pages_say() {
    use lightup_game::draw::picture;
    use lightup_game::help::PAGES;
    let grids: Vec<Grid> = PAGES.iter().map(|p| p.grid()).collect();
    // A light, and a cell its light does not reach behind a black cell.
    let g = &grids[0];
    assert!(g.is_lit(g.index(0, 1)) && g.is_lit(g.index(1, 2)) && !g.is_lit(g.index(4, 1)));
    assert!((0..15).all(|i| !g.light_conflicts(i)));
    // Two lights in error.
    let g = &grids[1];
    assert_eq!((0..15).filter(|i| g.light_conflicts(*i)).count(), 2);
    // A 1 that is met and a 1 with two lights, and no other error.
    let g = &grids[2];
    assert!(!g.clue_overfull(g.index(1, 1)) && g.lights_beside(g.index(1, 1)) == 1);
    assert!(g.clue_overfull(g.index(3, 1)));
    assert!((0..15).all(|i| !g.light_conflicts(i)));
    // Marks beside a 0.
    let g = &grids[3];
    assert_eq!(g.clue(2), Some(0));
    assert!(g.is_marked(1) && g.is_marked(3) && g.is_marked(7));
    for page in &PAGES {
        // Short enough for five lines of the smallest display.
        assert!(page.text.len() <= 105, "{}", page.text.len());
        let g = page.grid();
        assert_eq!((g.width(), g.height()), (5, 3));
        for (pitch, color) in [(14, false), (19, false), (28, true)] {
            let (w, h, ops) = picture(&g, pitch, palette(color));
            assert_eq!((w, h), (5 * pitch + 1, 3 * pitch + 1));
            let px = paint(&ops, w as usize, h as usize, Rgb(1, 2, 3));
            assert!(
                !px.contains(&Rgb(1, 2, 3)),
                "the picture paints all of itself"
            );
            // Each cell is drawn as the board's painter draws that look
            // (the first row of a board of the same width).
            let board = Painter::new(5, pitch, palette(color));
            let first = paint(&board.row(&g, 0), w as usize, pitch as usize, Rgb(1, 2, 3));
            assert_eq!(px[..first.len()], first[..]);
        }
    }
}
