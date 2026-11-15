//! Integer-only drawing: fills, lines, glyph and icon blits into a back
//! buffer, and damage-tracked presentation to a HAL display.

use crate::font::FontData;
use crate::icons::{Icon, FMT_GRAY2, FMT_MASK4};
use dphone_hal::{Display, PixelFormat, Screen};

/// A color as `0x00RRGGBB`.
pub type Rgb = u32;

pub const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
    ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

/// The four gray levels as colors (0 = black .. 3 = white).
pub const GRAY: [Rgb; 4] = [0x000000, 0x555555, 0xAAAAAA, 0xFFFFFF];

/// A rectangle in screen pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct IRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl IRect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        IRect { x, y, w, h }
    }
    pub const fn right(&self) -> i32 {
        self.x + self.w
    }
    pub const fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }
    pub fn intersect(&self, o: &IRect) -> IRect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        IRect { x, y, w: (r - x).max(0), h: (b - y).max(0) }
    }
    /// Shrinks by `d` on every side.
    pub fn inset(&self, d: i32) -> IRect {
        IRect { x: self.x + d, y: self.y + d, w: (self.w - 2 * d).max(0), h: (self.h - 2 * d).max(0) }
    }
    pub fn inset_xy(&self, dx: i32, dy: i32) -> IRect {
        IRect { x: self.x + dx, y: self.y + dy, w: (self.w - 2 * dx).max(0), h: (self.h - 2 * dy).max(0) }
    }
    /// Splits off the top `h` pixels: (top, rest).
    pub fn split_top(&self, h: i32) -> (IRect, IRect) {
        let h = h.clamp(0, self.h);
        (IRect { h, ..*self }, IRect { y: self.y + h, h: self.h - h, ..*self })
    }
    /// Splits off the bottom `h` pixels: (rest, bottom).
    pub fn split_bottom(&self, h: i32) -> (IRect, IRect) {
        let h = h.clamp(0, self.h);
        (IRect { h: self.h - h, ..*self }, IRect { y: self.bottom() - h, h, ..*self })
    }
    /// Splits off the left `w` pixels: (left, rest).
    pub fn split_left(&self, w: i32) -> (IRect, IRect) {
        let w = w.clamp(0, self.w);
        (IRect { w, ..*self }, IRect { x: self.x + w, w: self.w - w, ..*self })
    }
    /// Splits off the right `w` pixels: (rest, right).
    pub fn split_right(&self, w: i32) -> (IRect, IRect) {
        let w = w.clamp(0, self.w);
        (IRect { w: self.w - w, ..*self }, IRect { x: self.right() - w, w, ..*self })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum HAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// A back buffer for one screen.
pub struct Canvas {
    w: i32,
    h: i32,
    gray: bool,
    pix: Vec<u32>,
    front: Vec<u32>,
    /// False until the first present: everything is damaged.
    presented: bool,
    clip: IRect,
}

fn quantize(c: Rgb) -> Rgb {
    let lum = (((c >> 16) & 255) * 77 + ((c >> 8) & 255) * 150 + (c & 255) * 29) >> 8;
    GRAY[((lum * 3 + 127) / 255) as usize]
}

#[inline]
fn blend(dst: Rgb, src: Rgb, a: u32, max: u32) -> Rgb {
    if a >= max {
        return src;
    }
    let na = max - a;
    let ch = |shift: u32| {
        let d = (dst >> shift) & 255;
        let s = (src >> shift) & 255;
        ((d * na + s * a + max / 2) / max) << shift
    };
    ch(16) | ch(8) | ch(0)
}

impl Canvas {
    /// A canvas for a `width` x `height` screen; `gray` reduces every pixel
    /// to four levels.
    pub fn new(width: u16, height: u16, gray: bool) -> Canvas {
        let n = width as usize * height as usize;
        Canvas {
            w: width as i32,
            h: height as i32,
            gray,
            pix: vec![0; n],
            front: vec![0; n],
            presented: false,
            clip: IRect::new(0, 0, width as i32, height as i32),
        }
    }

    pub fn width(&self) -> i32 {
        self.w
    }
    pub fn height(&self) -> i32 {
        self.h
    }
    pub fn bounds(&self) -> IRect {
        IRect::new(0, 0, self.w, self.h)
    }
    pub fn is_gray(&self) -> bool {
        self.gray
    }
    /// The pixels as `0x00RRGGBB`, row-major.
    pub fn pixels(&self) -> &[u32] {
        &self.pix
    }
    pub fn pixel(&self, x: i32, y: i32) -> Rgb {
        self.pix[(y * self.w + x) as usize]
    }

    /// Restricts drawing to `r` (intersected with the screen); returns the
    /// previous clip for [`Canvas::restore_clip`].
    pub fn set_clip(&mut self, r: IRect) -> IRect {
        let old = self.clip;
        self.clip = r.intersect(&self.bounds());
        old
    }
    /// Narrows the clip to its intersection with `r`.
    pub fn push_clip(&mut self, r: IRect) -> IRect {
        let old = self.clip;
        self.clip = r.intersect(&old);
        old
    }
    pub fn restore_clip(&mut self, old: IRect) {
        self.clip = old;
    }
    pub fn clip(&self) -> IRect {
        self.clip
    }

    fn color(&self, c: Rgb) -> Rgb {
        if self.gray { quantize(c) } else { c & 0xFF_FFFF }
    }

    pub fn fill(&mut self, r: IRect, c: Rgb) {
        let r = r.intersect(&self.clip);
        if r.is_empty() {
            return;
        }
        let c = self.color(c);
        for y in r.y..r.bottom() {
            let row = (y * self.w) as usize;
            self.pix[row + r.x as usize..row + r.right() as usize].fill(c);
        }
    }

    pub fn clear(&mut self, c: Rgb) {
        self.fill(self.bounds(), c);
    }

    pub fn hline(&mut self, x: i32, y: i32, w: i32, c: Rgb) {
        self.fill(IRect::new(x, y, w, 1), c);
    }
    pub fn vline(&mut self, x: i32, y: i32, h: i32, c: Rgb) {
        self.fill(IRect::new(x, y, 1, h), c);
    }

    /// A one-pixel outline.
    pub fn frame(&mut self, r: IRect, c: Rgb) {
        if r.is_empty() {
            return;
        }
        self.hline(r.x, r.y, r.w, c);
        self.hline(r.x, r.bottom() - 1, r.w, c);
        self.vline(r.x, r.y, r.h, c);
        self.vline(r.right() - 1, r.y, r.h, c);
    }

    /// A filled rectangle with rounded corners of radius `rad`.
    pub fn fill_round(&mut self, r: IRect, rad: i32, c: Rgb) {
        let rad = rad.min(r.w / 2).min(r.h / 2).max(0);
        if rad == 0 {
            return self.fill(r, c);
        }
        self.fill(IRect::new(r.x, r.y + rad, r.w, r.h - 2 * rad), c);
        for i in 0..rad {
            // Row i from the top/bottom edge: inset by the circle.
            let dy = rad - i;
            let mut dx = 0;
            while (dx + 1) * (dx + 1) + dy * dy <= rad * rad + rad / 2 {
                dx += 1;
            }
            let inset = rad - dx;
            self.hline(r.x + inset, r.y + i, r.w - 2 * inset, c);
            self.hline(r.x + inset, r.bottom() - 1 - i, r.w - 2 * inset, c);
        }
    }

    /// A filled rectangle with rounded corners of radius `rad`, its edge
    /// anti-aliased by pixel coverage.  (A gray canvas shows the coverage
    /// through its two middle levels.)  A radius of half the height makes
    /// a bar with round ends.
    pub fn fill_round_aa(&mut self, r: IRect, rad: i32, c: Rgb) {
        /// One pixel in the fixed-point unit of the distances below.
        const ONE: i32 = 256;
        if r.is_empty() {
            return;
        }
        let rad = rad.min(r.w / 2).min(r.h / 2).max(0);
        let c = self.color(c);
        // The straight middle needs no blending.
        self.fill(IRect::new(r.x + rad, r.y, r.w - 2 * rad, r.h), c);
        // Half the size, and half the size less the radius: the distance
        // from the center to the centers of the corner circles.
        let (hw, hh) = (r.w * ONE / 2, r.h * ONE / 2);
        let (iw, ih) = (hw - rad * ONE, hh - rad * ONE);
        for j in 0..r.h {
            let qy = (j * ONE + ONE / 2 - hh).abs() - ih;
            for i in (0..rad).chain(r.w - rad..r.w) {
                let qx = (i * ONE + ONE / 2 - hw).abs() - iw;
                // The signed distance from the pixel's center to the edge.
                let d = if qx > 0 && qy > 0 {
                    ((qx * qx + qy * qy) as u32).isqrt() as i32 - rad * ONE
                } else {
                    qx.max(qy) - rad * ONE
                };
                let coverage = (ONE / 2 - d).clamp(0, ONE) as u32;
                self.put_blend(r.x + i, r.y + j, c, coverage, ONE as u32);
            }
        }
    }

    #[inline]
    fn put_blend(&mut self, x: i32, y: i32, c: Rgb, a: u32, max: u32) {
        if a == 0 || x < self.clip.x || y < self.clip.y || x >= self.clip.right() || y >= self.clip.bottom() {
            return;
        }
        let i = (y * self.w + x) as usize;
        let v = blend(self.pix[i], c, a, max);
        self.pix[i] = if self.gray { quantize(v) } else { v };
    }

    /// Draws `s` with its baseline at `y`, starting at `x`.  Returns the
    /// advance.
    pub fn text(&mut self, font: &FontData, x: i32, y: i32, s: &str, c: Rgb) -> i32 {
        let c = self.color(c);
        let mut pen = x;
        for ch in s.chars() {
            let Some((font, g)) = font.glyph_or_fallback(ch) else { continue };
            let gx = pen + g.xoff as i32;
            let gy = y - g.top as i32;
            pen += g.adv as i32;
            if g.w == 0
                || gx >= self.clip.right()
                || gx + g.w as i32 <= self.clip.x
                || gy >= self.clip.bottom()
                || gy + g.h as i32 <= self.clip.y
            {
                continue;
            }
            let data = &font.data[g.off as usize..];
            if font.bits == 1 {
                // A bitmap font: its pixels as they are, in one solid
                // level.
                let row_bytes = (g.w as usize).div_ceil(8);
                for row in 0..g.h as usize {
                    let bits = &data[row * row_bytes..];
                    for col in 0..g.w as usize {
                        if bits[col / 8] & (0x80 >> (col % 8)) != 0 {
                            self.put_blend(gx + col as i32, gy + row as i32, c, 1, 1);
                        }
                    }
                }
                continue;
            }
            let mut n = 0usize;
            for row in 0..g.h as i32 {
                for col in 0..g.w as i32 {
                    let byte = data[n / 2];
                    let a = if n.is_multiple_of(2) { byte >> 4 } else { byte & 15 };
                    n += 1;
                    self.put_blend(gx + col, gy + row, c, a as u32, 15);
                }
            }
        }
        pen - x
    }

    /// Draws one line of text inside `r`, vertically centered, aligned
    /// horizontally, truncated with an ellipsis when too long.
    pub fn text_in(&mut self, font: &FontData, r: IRect, s: &str, c: Rgb, align: HAlign) {
        if r.is_empty() {
            return;
        }
        let mut buf;
        let mut s = s;
        let mut w = font.width(s);
        if w > r.w {
            let ell = font.width("\u{2026}");
            let n = font.fit(s, r.w - ell);
            buf = String::with_capacity(n + 3);
            buf.push_str(s[..n].trim_end());
            buf.push('\u{2026}');
            w = font.width(&buf);
            s = &buf;
        }
        let x = match align {
            HAlign::Left => r.x,
            HAlign::Center => r.x + (r.w - w) / 2,
            HAlign::Right => r.right() - w,
        };
        let y = r.y + (r.h - font.height()) / 2 + font.ascent as i32;
        let old = self.push_clip(r);
        self.text(font, x, y, s, c);
        self.restore_clip(old);
    }

    /// Word-wrapped text from the top of `r`; returns the height used.
    pub fn text_wrapped(&mut self, font: &FontData, r: IRect, s: &str, c: Rgb, align: HAlign, line_h: i32) -> i32 {
        let lines = font.wrap(s, r.w);
        let mut y = r.y;
        for line in &lines {
            if y + line_h > r.bottom() + line_h / 2 {
                break;
            }
            self.text_in(font, IRect::new(r.x, y, r.w, line_h), line, c, align);
            y += line_h;
        }
        y - r.y
    }

    /// Draws an icon with its top-left at (`x`, `y`).  `tint` colors
    /// alpha-mask icons; it is ignored by color and gray-level icons.
    pub fn icon(&mut self, icon: Icon, x: i32, y: i32, tint: Rgb) {
        let (w, h) = (icon.width(), icon.height());
        let data = icon.bytes();
        match icon.rec.fmt {
            FMT_MASK4 | FMT_GRAY2 => {
                let mask = icon.rec.fmt == FMT_MASK4;
                let tint = self.color(tint);
                let mut n = 0usize;
                for row in 0..h {
                    for col in 0..w {
                        let byte = data[n / 2];
                        let v = if n.is_multiple_of(2) { byte >> 4 } else { byte & 15 };
                        n += 1;
                        if mask {
                            self.put_blend(x + col, y + row, tint, v as u32, 15);
                        } else if v < 4 {
                            self.put_blend(x + col, y + row, GRAY[v as usize], 1, 1);
                        }
                    }
                }
            }
            _ => {
                let (mut col, mut row) = (0, 0);
                for run in data.as_chunks::<5>().0 {
                    let c = rgb(run[1], run[2], run[3]);
                    let a = run[4] as u32;
                    for _ in 0..run[0] {
                        if a != 0 {
                            self.put_blend(x + col, y + row, c, a, 255);
                        }
                        col += 1;
                        if col == w {
                            col = 0;
                            row += 1;
                        }
                    }
                }
            }
        }
    }

    /// Draws an icon centered in `r`.
    pub fn icon_in(&mut self, icon: Icon, r: IRect, tint: Rgb) {
        self.icon(icon, r.x + (r.w - icon.width()) / 2, r.y + (r.h - icon.height()) / 2, tint);
    }

    /// Draws raw RGBA pixels (app images) with their top-left at (`x`, `y`).
    pub fn rgba(&mut self, x: i32, y: i32, w: i32, h: i32, pixels: &[u8]) {
        if pixels.len() < (w * h * 4) as usize {
            return;
        }
        for row in 0..h {
            for col in 0..w {
                let p = &pixels[((row * w + col) * 4) as usize..];
                self.put_blend(x + col, y + row, rgb(p[0], p[1], p[2]), p[3] as u32, 255);
            }
        }
    }

    /// The bounding box of the pixels that differ from what was last
    /// presented.
    pub fn damage(&self) -> Option<IRect> {
        if !self.presented {
            return Some(self.bounds());
        }
        let w = self.w as usize;
        let rows = self.h as usize;
        let differs = |y: usize| self.pix[y * w..(y + 1) * w] != self.front[y * w..(y + 1) * w];
        let top = (0..rows).find(|&y| differs(y))?;
        let bottom = (top..rows).rev().find(|&y| differs(y)).unwrap_or(top);
        let (mut left, mut right) = (w, 0);
        for y in top..=bottom {
            let (a, b) = (&self.pix[y * w..(y + 1) * w], &self.front[y * w..(y + 1) * w]);
            if let Some(l) = (0..left).find(|&x| a[x] != b[x]) {
                left = l;
            }
            if let Some(r) = (right..w).rev().find(|&x| a[x] != b[x]) {
                right = r + 1;
            }
        }
        Some(IRect::new(left as i32, top as i32, (right.max(left + 1) - left) as i32, (bottom - top + 1) as i32))
    }

    /// Copies the damaged area to the display and flushes it.  Returns the
    /// damage, or `None` when nothing changed (no flush happens).
    pub fn present(&mut self, display: &mut dyn Display, screen: Screen) -> dphone_hal::Result<Option<IRect>> {
        let Some(d) = self.damage() else { return Ok(None) };
        let Some(info) = display.info(screen) else { return Ok(None) };
        let d = d.intersect(&IRect::new(0, 0, info.width as i32, info.height as i32));
        if d.is_empty() {
            return Ok(None);
        }
        {
            let frame = display.frame(screen)?;
            let w = self.w as usize;
            for y in d.y as usize..d.bottom() as usize {
                let src = &self.pix[y * w + d.x as usize..y * w + d.right() as usize];
                let row = y * info.stride;
                match info.format {
                    PixelFormat::Xrgb8888 => {
                        let dst = &mut frame[row + d.x as usize * 4..row + d.right() as usize * 4];
                        for (o, p) in dst.as_chunks_mut::<4>().0.iter_mut().zip(src) {
                            o.copy_from_slice(&p.to_le_bytes());
                        }
                    }
                    PixelFormat::R8 => {
                        let dst = &mut frame[row + d.x as usize..row + d.right() as usize];
                        for (o, p) in dst.iter_mut().zip(src) {
                            // Gray canvases hold r == g == b.
                            *o = (*p & 255) as u8;
                        }
                    }
                }
            }
        }
        display.flush(
            screen,
            &[dphone_hal::Rect::new(d.x as u16, d.y as u16, d.w as u16, d.h as u16)],
        )?;
        self.front.copy_from_slice(&self.pix);
        self.presented = true;
        Ok(Some(d))
    }

    /// Forgets what was presented, so the next present repaints everything
    /// (after the panel was powered off, for example).
    pub fn invalidate(&mut self) {
        self.presented = false;
    }
}

// ------------------------------------------------------------- wallpaper

/// The color skin's wallpaper: a deep blue vertical gradient crossed by
/// broad diagonal facets.  Drawn procedurally (our own design in the stock
/// firmware's palette); computed once and kept.
fn wallpaper_pixels(w: i32, h: i32) -> &'static [u32] {
    static PIXELS: std::sync::OnceLock<Vec<u32>> = std::sync::OnceLock::new();
    PIXELS.get_or_init(|| {
        let mut v = Vec::with_capacity((w * h) as usize);
        // Vertical gradient stops: (row, r, g, b).
        const STOPS: [(i32, i32, i32, i32); 4] =
            [(0, 0x15, 0x1E, 0x2C), (56, 0x10, 0x24, 0x44), (150, 0x0D, 0x20, 0x3E), (268, 0x0A, 0x14, 0x26)];
        // Facets: (slant: +1 leans right going down, -1 left; start and
        // width along the slanted axis; strength in 1/64).  Each facet is
        // brightest at its leading edge and fades across its width.
        const FACETS: [(i32, i32, i32, i32); 6] =
            [(1, 60, 170, 30), (1, 330, 150, 22), (1, 560, 190, 32), (-1, -120, 160, 24), (-1, 130, 140, 18), (-1, 300, 170, 28)];
        for y in 0..h {
            let (mut r, mut g, mut b) = (0x20, 0x21, 0x22);
            if y <= 268 {
                let i = STOPS.iter().rposition(|s| s.0 <= y).unwrap_or(0).min(STOPS.len() - 2);
                let (a, z) = (STOPS[i], STOPS[i + 1]);
                let (t, span) = (y - a.0, z.0 - a.0);
                r = a.1 + (z.1 - a.1) * t / span;
                g = a.2 + (z.2 - a.2) * t / span;
                b = a.3 + (z.3 - a.3) * t / span;
            }
            for x in 0..w {
                let (mut pr, mut pg, mut pb) = (r, g, b);
                if y <= 268 {
                    let mut light = 0;
                    for (slant, start, width, strength) in FACETS {
                        // Slope 3 across to 5 down.
                        let u = x + slant * y * 3 / 5 - start;
                        if (0..width).contains(&u) {
                            light += strength * (width - u) / width;
                        }
                    }
                    // Lighten toward a brighter blue.
                    pr += 0x06 * light / 64;
                    pg += 0x16 * light / 64;
                    pb += 0x38 * light / 64;
                }
                v.push(rgb(pr.clamp(0, 255) as u8, pg.clamp(0, 255) as u8, pb.clamp(0, 255) as u8));
            }
        }
        v
    })
}

impl Canvas {
    /// Paints the idle/call background over the whole screen: the blue
    /// wallpaper on the color skin, the plain background on gray.
    pub fn wallpaper(&mut self, plain: Rgb) {
        if self.gray || self.w != 480 || self.h != 272 {
            self.clear(plain);
            return;
        }
        let src = wallpaper_pixels(self.w, self.h);
        let clip = self.clip;
        for y in clip.y..clip.bottom() {
            let row = (y * self.w) as usize;
            let (a, b) = (row + clip.x as usize, row + clip.right() as usize);
            self.pix[a..b].copy_from_slice(&src[a..b]);
        }
    }

    /// Fills one row span `x0..x1` (exclusive) at `y`.
    pub fn span(&mut self, y: i32, x0: i32, x1: i32, c: Rgb) {
        self.fill(IRect::new(x0, y, x1 - x0, 1), c);
    }

    /// A softkey shape in `r`.  On the color skin: a rectangle whose
    /// bottom edge comes to a shallow point at the center, aimed at the
    /// physical key below (the last five rows of `r` are the point), with
    /// rounded corners and an optional one-pixel outline, all
    /// anti-aliased by coverage.  On gray: a cell with rounded top corners
    /// and a one-pixel gap at each side.
    pub fn softkey_shape(&mut self, r: IRect, fill: Rgb, edge: Option<Rgb>) {
        if self.gray {
            for j in 0..r.h {
                let inset = match j {
                    0 => 3,
                    1 => 2,
                    _ => 1,
                };
                self.span(r.y + j, r.x + inset, r.right() - inset, fill);
            }
            return;
        }
        /// Rows of the point.
        const POINT: i32 = 5;
        /// One pixel in the fixed-point unit of the distances below.
        const ONE: i32 = 256;
        /// The radius of the outline's corners.
        const RADIUS: i32 = 3 * ONE;
        if r.w < 2 || r.h <= POINT {
            return;
        }
        let body = r.h - POINT;
        let edge = edge.unwrap_or(fill);
        // The outline is the top edge, the two sides, and two slanted
        // edges from the lower corners (at `body`) to the center of the
        // bottom of `r`.  The length of a slanted edge's direction vector
        // (w, 2 * POINT), in fixed point, turns its line equation into a
        // distance.
        let slant_len = ((r.w * r.w + 4 * POINT * POINT) as u32 * (ONE * ONE) as u32).isqrt() as i32;
        // Two edges that meet at a corner, rounded: where both distances
        // are within the radius, the distance is from the corner's circle.
        let round = |a: i32, b: i32| {
            if a > -RADIUS && b > -RADIUS {
                (((a + RADIUS) * (a + RADIUS) + (b + RADIUS) * (b + RADIUS)) as u32).isqrt() as i32 - RADIUS
            } else {
                a.max(b)
            }
        };
        // How much of a pixel lies inside an outline whose edge is `d`
        // from the pixel's center (negative inside), out of `ONE`.
        let coverage = |d: i32| (ONE / 2 - d).clamp(0, ONE) as u32;
        for j in 0..r.h {
            let py = j * ONE + ONE / 2;
            for x in 0..r.w {
                // The shape is symmetric: fold onto the left half.
                let px = x.min(r.w - 1 - x) * ONE + ONE / 2;
                let side = -px;
                let top = -py;
                let slant = ((py - body * ONE) * r.w - 2 * POINT * px) * ONE / slant_len;
                // The signed distance from the pixel's center to the
                // outline, and to the inner edge of the one-pixel border.
                let d = round(side, top).max(round(side, slant));
                let outer = coverage(d);
                if outer == 0 {
                    continue;
                }
                let inner = coverage(d + ONE);
                self.put_blend(r.x + x, r.y + j, blend(edge, fill, inner, outer), outer, ONE as u32);
            }
        }
    }
}

/// Mixes `n` eighths of `b` into `a`.
pub fn mix(a: Rgb, b: Rgb, n: u32) -> Rgb {
    blend(a, b, n.min(8), 8)
}
