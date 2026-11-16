//! The motion of the wallpapers that move on the idle screen: waves that
//! rise and fall, circles that wander, and a flow that runs on.
//!
//! A frame is the picture at a time, and the time is all it depends on:
//! every motion is a whole number of turns in [`CYCLE_MS`], so the
//! picture of a time is the picture one cycle later, and a frame may be
//! skipped or drawn late.
//!
//! The phone draws several frames a second on a 500 MHz ARM1176 without
//! floating point, beside everything else it does, so a frame draws only
//! what the motion changed since the frame before, and what it draws
//! comes from tables wherever a table can stand for arithmetic:
//!
//! - Waves that hide what is under them: the pixels a crest passed over.
//!   Waves that fade below the crest: the rows the waves are in.
//! - Circles: the ring at each circle's edge, where its opacity changes
//!   (inside the ring it is the same wherever the circle goes).  A
//!   circle's opacity comes from a table by the square of the distance
//!   from its center, so no pixel needs a square root.  Large circles
//!   with wide, soft edges move in steps of two frames, half of them in
//!   each frame: a step moves such an edge by less than the eye can
//!   tell from the frame before.
//! - A flow: every pixel (all of them change), from tables for the
//!   straight run, the waves, and the colors.
//!
//! The parts that do not move (the gradient under the waves and the
//! circles) are drawn once and kept, with two more bits a channel than
//! the frame buffer has, and the frame is dithered with the noise of the
//! still picture.  One difference from the still picture: a channel is
//! kept a step and a half (of the frame buffer's 255) away from 0 and
//! from 255, so that the noise can be added to all three channels of a
//! pixel at once.

use super::*;

/// The time after which every motion is where it began.
pub const CYCLE_MS: u64 = 60_000;

/// Where a motion of `turns` turns a cycle is at `t_ms`, in 1/65536 turn.
fn turn_at(t_ms: u64, turns: i32) -> i32 {
    let phase = (t_ms % CYCLE_MS) as i64 * 65536 / CYCLE_MS as i64;
    ((phase * i64::from(turns)) & 0xFFFF) as i32
}

// ----------------------------------------------------- to the frame buffer

/// A channel's least and greatest value here: with the noise and the
/// rounding (-6 to 9) it stays within its ten bits.
const LEAST: i32 = 6;
const MOST: i32 = 1014;

fn limit(c: Px) -> Px {
    c.map(|v| v.clamp(LEAST, MOST))
}

/// One in each channel of a packed color.
const EACH: u32 = 0x0010_0401;

/// What is added to a packed color before its low two bits are dropped:
/// the rounding and, by the top four bits of the noise, the dither (as
/// [`to_frame`] has it for one channel).
const NOISE: [u32; 16] = {
    let mut table = [0; 16];
    let mut i = 0;
    while i < 16 {
        table[i] = ((i as i32 - 8 + 2) as u32).wrapping_mul(EACH);
        i += 1;
    }
    table
};

/// The noise of pixel (`x`, `y`); the next pixel's is [`NOISE_STEP`] more.
#[inline]
fn noise_at(x: i32, y: i32) -> u32 {
    ((x * 49471 + y * 37345) & 0xFFFF) as u32
}
const NOISE_STEP: u32 = 49471;

/// A packed color within the limits as the frame buffer's `0x00RRGGBB`.
#[inline]
fn shown(p: u32, noise: u32, dither: bool) -> u32 {
    let q = p.wrapping_add(if dither { NOISE[((noise >> 12) & 15) as usize] } else { 2 * EACH });
    ((q >> 6) & 0xFF_0000) | ((q >> 4) & 0xFF00) | ((q >> 2) & 0xFF)
}

/// Blends `c` into the packed color with opacity `t` of [`ONE`].
#[inline]
fn blend(p: u32, c: Px, t: i32) -> u32 {
    let (r, g, b) = ((p >> 20) as i32, ((p >> 10) & 1023) as i32, (p & 1023) as i32);
    let r = r + (((c[0] - r) * t + ONE / 2) >> 12);
    let g = g + (((c[1] - g) * t + ONE / 2) >> 12);
    let b = b + (((c[2] - b) * t + ONE / 2) >> 12);
    ((r as u32) << 20) | ((g as u32) << 10) | b as u32
}

/// A wallpaper in motion: what is kept from one frame to the next.
pub(super) struct Moving {
    pub(super) id: &'static str,
    /// The frame, as the canvas takes it (`0x00RRGGBB`).
    out: Vec<u32>,
    /// The parts that do not move, as packed [`Px`] colors within the
    /// limits (empty for a flow, which has none).
    base: Vec<u32>,
    dither: bool,
    kind: Kind,
    /// A frame was drawn: `out` holds the frame before this one.
    drawn: bool,
}

enum Kind {
    Waves(Waves),
    Discs(Discs),
    Flow(Stream),
}

impl Moving {
    /// What `spec` needs to move; `None` when it does not.
    pub(super) fn new(id: &'static str, spec: &'static Spec) -> Option<Moving> {
        let moves = spec.moves.as_ref()?;
        let kind = match (&moves.motion, &spec.base) {
            (Motion::Bob(bob), _) => Kind::Waves(Waves::new(spec.layers, bob)?),
            (Motion::Wander { reach, turns }, _) => Kind::Discs(Discs::new(spec.layers, *reach, *turns, u64::from(moves.every_ms))),
            (Motion::Run(turns), Base::Flow { flow: Flow::Ribbons { ax, ay, waves }, stops }) => {
                Kind::Flow(Stream::new(*ax, *ay, waves, turns, stops))
            }
            (Motion::Run(_), _) => return None,
        };
        let base = match kind {
            Kind::Flow(_) => Vec::new(),
            _ => render_base(&spec.base).into_iter().map(|p| pack(limit(unpack(p)))).collect(),
        };
        Some(Moving { id, out: vec![0; (WIDTH * HEIGHT) as usize], base, dither: spec.dither, kind, drawn: false })
    }

    /// Draws the frame of `t_ms` and returns it.
    pub(super) fn frame(&mut self, t_ms: u64) -> &[u32] {
        let first = !std::mem::replace(&mut self.drawn, true);
        match &mut self.kind {
            Kind::Waves(w) => w.frame(t_ms, first, &self.base, &mut self.out, self.dither),
            Kind::Discs(d) => d.frame(t_ms, first, &self.base, &mut self.out, self.dither),
            Kind::Flow(s) => s.frame(t_ms, &mut self.out, self.dither),
        }
        &self.out
    }
}

// ------------------------------------------------------------------ waves

/// One wave, ready to draw.
struct Swell {
    color: Px,
    /// The opacity (of [`ONE`]) by how far under the crest a pixel's
    /// center is, in sixteenths of a pixel, from 8 above it; farther
    /// under than the table goes, the last entry.
    opacity: Vec<u16>,
    /// It hides what is under it, from 8 sixteenths below its crest on.
    hides: bool,
    /// Each column's crest where the wave rests, in sixteenths of a pixel.
    shape: Vec<i32>,
    /// The crests of the frame in `out`.
    edge: Vec<i32>,
    /// The highest and lowest of them.
    span: (i32, i32),
}

struct Waves {
    waves: Vec<Swell>,
    bob: &'static [(i32, i32, i32)],
    /// Some wave fades under its crest: a frame draws whole rows.
    fades: bool,
    line: Vec<u32>,
}

impl Waves {
    fn new(layers: &'static [Layer], bob: &'static [(i32, i32, i32)]) -> Option<Waves> {
        let [Layer::Waves(list)] = layers else { return None };
        if list.len() != bob.len() {
            return None;
        }
        let waves = list
            .iter()
            .map(|w| {
                // As `wave` has it.
                let shape: Vec<i32> = (0..WIDTH)
                    .map(|x| {
                        let mut edge = w.y * 16 + ((w.amp * 16 * sin(x * 65536 / w.period + w.phase * 256)) >> 12);
                        if w.period2 > 0 {
                            edge += (w.amp2 * 16 * sin(x * 65536 / w.period2 + w.phase * 613)) >> 12;
                        }
                        edge
                    })
                    .collect();
                let deepest = if w.fade > 0 { w.fade * 16 } else { 8 };
                let opacity = (-8..=deepest)
                    .map(|depth| {
                        let cov = (depth + 8).clamp(0, 16);
                        let a = if w.fade > 0 { w.alpha * (w.fade * 16 - depth).max(0) / (w.fade * 16) } else { w.alpha };
                        (a * cov).clamp(0, ONE) as u16
                    })
                    .collect();
                Swell { color: limit(px(w.color)), opacity, hides: w.alpha >= 256 && w.fade == 0, edge: shape.clone(), shape, span: (0, 0) }
            })
            .collect();
        Some(Waves { waves, bob, fades: list.iter().any(|w| w.fade > 0), line: vec![0; WIDTH as usize] })
    }

    /// The color of the pixel in column `x` whose center is at height
    /// `yc`, over `base`.
    #[inline]
    fn pixel(&self, base: u32, x: usize, yc: i32) -> u32 {
        // A wave that hides what is under it is where to begin.
        let mut p = base;
        let mut from = 0;
        for (i, wave) in self.waves.iter().enumerate().rev() {
            if wave.hides && yc - wave.edge[x] >= 8 {
                p = pack(wave.color);
                from = i + 1;
                break;
            }
        }
        for wave in &self.waves[from..] {
            let at = yc - wave.edge[x] + 8;
            if at > 0 {
                let t = wave.opacity[(at as usize).min(wave.opacity.len() - 1)];
                p = blend(p, wave.color, i32::from(t));
            }
        }
        p
    }

    fn frame(&mut self, t_ms: u64, first: bool, base: &[u32], out: &mut [u32], dither: bool) {
        let w = WIDTH as usize;
        // The crests of this frame, and of the frame before.
        let mut before = Vec::with_capacity(self.waves.len());
        for (wave, &(reach, turns, begin)) in self.waves.iter_mut().zip(self.bob) {
            let off = (reach * sin(turn_at(t_ms, turns) + begin * 256)) >> 12;
            let now: Vec<i32> = wave.shape.iter().map(|e| e + off).collect();
            wave.span = now.iter().fold((i32::MAX, i32::MIN), |(lo, hi), e| (lo.min(*e), hi.max(*e)));
            before.push(std::mem::replace(&mut wave.edge, now));
        }
        if first || self.fades {
            // Whole rows: all of them the first time, then those from the
            // highest crest down (a wave that fades changes all the way
            // down its fade when it moves).
            let top = if first { 0 } else { self.waves.iter().zip(&before).map(|(wave, old)| wave.span.0.min(*old.iter().min().unwrap_or(&0))).min().unwrap_or(0) };
            let top = ((top - 8) >> 4).clamp(0, HEIGHT - 1);
            let mut line = std::mem::take(&mut self.line);
            for y in top..HEIGHT {
                let yc = y * 16 + 8;
                let row = y as usize * w;
                line.copy_from_slice(&base[row..row + w]);
                for wave in &self.waves {
                    if yc + 8 <= wave.span.0 {
                        continue;
                    }
                    let last = wave.opacity.len() - 1;
                    if wave.hides && yc - wave.span.1 >= 8 {
                        line.fill(pack(wave.color));
                        continue;
                    }
                    for (p, edge) in line.iter_mut().zip(&wave.edge) {
                        let at = yc - edge + 8;
                        if at > 0 {
                            let t = wave.opacity[(at as usize).min(last)];
                            if t > 0 {
                                *p = blend(*p, wave.color, i32::from(t));
                            }
                        }
                    }
                }
                let mut noise = noise_at(0, y);
                for (o, p) in out[row..row + w].iter_mut().zip(&line) {
                    *o = shown(*p, noise, dither);
                    noise = noise.wrapping_add(NOISE_STEP);
                }
            }
            self.line = line;
            return;
        }
        // The pixels each crest passed over, column by column.
        for (i, old) in before.iter().enumerate() {
            for (x, old) in old.iter().enumerate() {
                let now = self.waves[i].edge[x];
                let (lo, hi) = (now.min(*old), now.max(*old));
                for y in ((lo - 8) >> 4).max(0)..=((hi + 8) >> 4).min(HEIGHT - 1) {
                    let at = y as usize * w + x;
                    out[at] = shown(self.pixel(base[at], x, y * 16 + 8), noise_at(x as i32, y), dither);
                }
            }
        }
    }
}

// ---------------------------------------------------------------- circles

/// A circle that wanders.
struct Disc {
    /// Its place, in sixteenths of a pixel.
    home: (i32, i32),
    /// How far it goes each way, its turns in a cycle, and where in a turn
    /// it begins, across and then up and down.
    path: [(i32, i32, i32); 2],
    /// It moves once in this many milliseconds, this long after the
    /// others that do.
    step: (u64, u64),
    color: Px,
    /// The opacity (of [`ONE`]) by the square of the distance from the
    /// center in sixteenths of a pixel, `shift` bits dropped; none beyond
    /// the table.
    opacity: Vec<u16>,
    shift: u32,
    /// By how many rows a pixel is from the center: how many columns to
    /// either side the circle reaches (`outer`), and how many columns to
    /// either side its opacity is the same wherever it goes (`inner`; 0
    /// where there is no such column).  Both allow for a move of
    /// [`MARGIN`] pixels and for the center being anywhere in its pixel.
    outer: Vec<u16>,
    inner: Vec<u16>,
    /// Where it is in the frame in `out`.
    at: (i32, i32),
}

/// The farthest a circle moves between two frames for its ring alone to
/// be drawn again; one that moved farther is drawn again whole.
const MARGIN: i32 = 3;

/// A circle whose edge is at least [`SOFT`] sixteenths of a pixel wide and
/// whose ring has more pixels than this moves in steps of two frames.
const RING: i32 = 6_000;
const SOFT: i32 = 16 * 16;

impl Disc {
    fn new(index: u32, (cx, cy, r, color, alpha): Circle, soft: i32, rim: i32, reach: (i32, i32), turns: (i32, i32), every_ms: u64) -> Disc {
        let (r, soft) = (r * 16, soft_edge(r, soft));
        let edge = r + soft / 2 + 16;
        // A table of at most 1024 entries; the steps across the edge of
        // the largest circle are a tenth of a pixel.
        let mut shift = 0;
        while ((edge * edge) >> shift) >= 1024 {
            shift += 1;
        }
        let opacity: Vec<u16> = (0..=(edge * edge) >> shift)
            .map(|i| {
                let d = (((i << shift) + ((1 << shift) >> 1)) as u32).isqrt() as i32;
                disc_opacity(d, r, soft, alpha, rim).clamp(0, ONE) as u16
            })
            .collect();
        // The radius within which the opacity is the same everywhere.
        let level = (r - soft / 2).min(if rim > 0 { r - 36 } else { i32::MAX }) - 16;
        let (far, near) = ((edge >> 4) + 2 + MARGIN, (level >> 4) - 2 - MARGIN);
        let half = |radius: i32, dy: i32| if dy < radius { ((radius * radius - dy * dy) as u32).isqrt() as i32 } else { 0 };
        let outer: Vec<u16> = (0..=far).map(|dy| (half(far, dy) + 1) as u16).collect();
        let inner: Vec<u16> = (0..=far).map(|dy| (half(near.max(0), dy + 1) - 1).max(0) as u16).collect();
        // Each circle has a path of its own, the same every time: the
        // turns across and up and down differ, so the path is not a line.
        let h = [hash(index as i32, 1, 0x51ed), hash(index as i32, 2, 0x51ed)];
        let span = (turns.1 - turns.0 + 1).max(1) as u32;
        let across = turns.0 + (h[0] % span) as i32;
        let mut up = turns.0 + ((h[0] >> 8) % span) as i32;
        if up == across && span > 1 {
            up = if across == turns.1 { turns.0 } else { across + 1 };
        }
        // Between half of the reach and all of it.
        let go = |reach: i32, h: u32| reach * 16 * (128 + (h >> 20) as i32 % 129) / 256;
        let path = [(go(reach.0, h[0]), across, (h[1] & 0xFFFF) as i32), (go(reach.1, h[1]), up, ((h[1] >> 4) & 0xFFFF) as i32)];
        // A wide ring with a soft edge is a large, faint circle: it can
        // move in steps of two frames (a step moves its edge by less than
        // can be told from the frame before), and the circles that do
        // take turns.  A sharp edge moves with every frame.
        let ring = 355 * (far * far - near.max(0) * near.max(0)) / 113;
        let frames = if soft >= SOFT && ring > RING { 2 } else { 1 };
        let step = (frames * every_ms, u64::from(index) % frames * every_ms);
        Disc { home: (cx * 16, cy * 16), path, step, color: limit(px(color)), opacity, shift, outer, inner, at: (cx * 16, cy * 16) }
    }

    fn place(&self, t_ms: u64) -> (i32, i32) {
        // The time of its last step.
        let t = t_ms % CYCLE_MS + CYCLE_MS;
        let t = t - (t + self.step.1) % self.step.0;
        let go = |(far, turns, begin): (i32, i32, i32)| (far * sin(turn_at(t, turns) + begin)) >> 12;
        (self.home.0 + go(self.path[0]), self.home.1 + go(self.path[1]))
    }

    /// The columns of row `y` the circle reaches when its center is in
    /// pixel (`cx`, `cy`): the first and the last, and how many to either
    /// side of the center are level.
    #[inline]
    fn row(&self, cx: i32, cy: i32, y: i32) -> Option<(i32, i32, i32)> {
        let dy = (y - cy).unsigned_abs() as usize;
        let half = i32::from(*self.outer.get(dy)?);
        Some((cx - half, cx + half, i32::from(self.inner[dy])))
    }
}

struct Discs {
    discs: Vec<Disc>,
    line: Vec<u32>,
}

impl Discs {
    /// The circles of the layers, in the order they are drawn.
    fn new(layers: &'static [Layer], reach: (i32, i32), turns: (i32, i32), every_ms: u64) -> Discs {
        let mut discs = Vec::new();
        for layer in layers {
            let (soft, rim, circles) = match layer {
                Layer::Circles { soft, rim, circles } => (*soft, *rim, circles.to_vec()),
                Layer::Bubbles { seed, count, r, alpha, soft, rim, colors } => (*soft, *rim, bubbles(*seed, *count, *r, *alpha, colors)),
                _ => continue,
            };
            for circle in circles {
                discs.push(Disc::new(discs.len() as u32, circle, soft, rim, reach, turns, every_ms));
            }
        }
        Discs { discs, line: vec![0; WIDTH as usize] }
    }

    fn frame(&mut self, t_ms: u64, first: bool, base: &[u32], out: &mut [u32], dither: bool) {
        let w = WIDTH as usize;
        // The circles that moved: which, the pixel its center is in, and
        // whether its ring alone changed.
        let mut moved: Vec<(usize, i32, i32, bool)> = Vec::new();
        // The rows with something to draw again.
        let (mut top, mut bottom) = (HEIGHT, -1);
        for (i, d) in self.discs.iter_mut().enumerate() {
            let at = d.place(t_ms);
            if at == d.at && !first {
                continue;
            }
            let ring = (at.0 - d.at.0).abs().max((at.1 - d.at.1).abs()) <= MARGIN * 16;
            let (cx, cy, far) = (at.0 >> 4, at.1 >> 4, d.outer.len() as i32);
            if ring {
                moved.push((i, cx, cy, true));
                (top, bottom) = (top.min(cy - far), bottom.max(cy + far));
            } else {
                // It jumped (a frame was skipped): where it was, and
                // where it is, whole.
                let (ox, oy) = (d.at.0 >> 4, d.at.1 >> 4);
                moved.push((i, ox, oy, false));
                moved.push((i, cx, cy, false));
                (top, bottom) = (top.min(cy.min(oy) - far), bottom.max(cy.max(oy) + far));
            }
            d.at = at;
        }
        if first {
            (top, bottom) = (0, HEIGHT - 1);
        }
        let mut line = std::mem::take(&mut self.line);
        let mut spans: Vec<(i32, i32)> = Vec::with_capacity(2 * moved.len() + 1);
        for y in top.max(0)..=bottom.min(HEIGHT - 1) {
            // The stretches of this row to draw again, in order, none
            // touching the next.
            spans.clear();
            if first {
                spans.push((0, WIDTH - 1));
            } else {
                for &(i, cx, cy, ring) in &moved {
                    let Some((x0, x1, level)) = self.discs[i].row(cx, cy, y) else { continue };
                    if ring && level > 0 {
                        spans.push((x0, cx - level));
                        spans.push((cx + level, x1));
                    } else {
                        spans.push((x0, x1));
                    }
                }
                spans.retain_mut(|s| {
                    *s = (s.0.max(0), s.1.min(WIDTH - 1));
                    s.0 <= s.1
                });
                if spans.is_empty() {
                    continue;
                }
                spans.sort_unstable();
                let mut merged = 0;
                for i in 1..spans.len() {
                    if spans[i].0 <= spans[merged].1 + 1 {
                        spans[merged].1 = spans[merged].1.max(spans[i].1);
                    } else {
                        merged += 1;
                        spans[merged] = spans[i];
                    }
                }
                spans.truncate(merged + 1);
            }
            let row = y as usize * w;
            for &(x0, x1) in &spans {
                line[x0 as usize..=x1 as usize].copy_from_slice(&base[row + x0 as usize..=row + x1 as usize]);
            }
            // Every circle on this row, in order, where the row is drawn
            // again.
            let yc = y * 16 + 8;
            for d in &self.discs {
                let Some((d0, d1, _)) = d.row(d.at.0 >> 4, d.at.1 >> 4, y) else { continue };
                let dy = yc - d.at.1;
                let dy2 = dy * dy;
                for &(s0, s1) in &spans {
                    let (x0, x1) = (s0.max(d0), s1.min(d1));
                    if x0 > x1 {
                        continue;
                    }
                    let mut dx = x0 * 16 + 8 - d.at.0;
                    for p in &mut line[x0 as usize..=x1 as usize] {
                        if let Some(&t) = d.opacity.get(((dx * dx + dy2) >> d.shift) as usize)
                            && t > 0
                        {
                            *p = blend(*p, d.color, i32::from(t));
                        }
                        dx += 16;
                    }
                }
            }
            for &(x0, x1) in &spans {
                let mut noise = noise_at(x0, y);
                for x in x0 as usize..=x1 as usize {
                    out[row + x] = shown(line[x], noise, dither);
                    noise = noise.wrapping_add(NOISE_STEP);
                }
            }
        }
        self.line = line;
    }
}

// ------------------------------------------------------------------- flow

/// A flow whose waves run on ([`Flow::Ribbons`]).
struct Stream {
    ax: i32,
    ay: i32,
    /// The waves, with the turns each makes in a cycle.
    waves: Vec<((i32, i32, i32, i32), i32)>,
    /// The straight run by `x * ax + y * ay`.
    straight: Vec<i32>,
    /// The color (packed, within the limits) at each position along the
    /// stops.
    colors: Vec<u32>,
    /// For each wave, its turn at each column.
    across: Vec<i32>,
    /// For each wave that varies both ways: what it adds, by its turn in
    /// 1/4096 turn.
    swing: Vec<Vec<i16>>,
}

impl Stream {
    fn new(ax: i32, ay: i32, waves: &'static [(i32, i32, i32, i32)], turns: &'static [i32], stops: Stops) -> Stream {
        let whole = WIDTH * ax + HEIGHT * ay;
        let mut across = Vec::with_capacity(waves.len() * WIDTH as usize);
        let mut swing = Vec::new();
        for &(amount, px, py, _) in waves {
            across.extend((0..WIDTH).map(|x| if px != 0 { x * 65536 / px } else { 0 }));
            swing.push(if px != 0 && py != 0 { (0..4096).map(|k| ((amount * sin(k * 16 + 8)) >> 12) as i16).collect() } else { Vec::new() });
        }
        Stream {
            ax,
            ay,
            waves: waves.iter().enumerate().map(|(i, w)| (*w, turns.get(i).copied().unwrap_or(0))).collect(),
            straight: (0..=whole).map(|v| v * ONE / whole).collect(),
            colors: (0..=ONE).map(|t| pack(limit(stops_at(stops, t)))).collect(),
            across,
            swing,
        }
    }

    /// Every pixel as [`Flow::at`] has it, with each wave moved on by the
    /// time.
    fn frame(&mut self, t_ms: u64, out: &mut [u32], dither: bool) {
        let w = WIDTH as usize;
        // What the waves that do not vary down the screen add at each
        // column; of those that vary both ways, the table, the turns at
        // the columns, and the turn at the start of the row.
        let mut columns = vec![0i32; w];
        let mut bent: Vec<(&[i16], &[i32], i32)> = Vec::new();
        for y in 0..HEIGHT {
            // What the waves that do not vary across the screen add in
            // this row.
            let mut rows = 0;
            bent.clear();
            for (i, &((amount, px, py, phase), turns)) in self.waves.iter().enumerate() {
                let turn = if py != 0 { y * 65536 / py } else { 0 } + phase * 256 + turn_at(t_ms, turns);
                let across = &self.across[i * w..(i + 1) * w];
                if px == 0 {
                    rows += (amount * sin(turn)) >> 12;
                } else if py == 0 {
                    if y == 0 {
                        for (c, a) in columns.iter_mut().zip(across) {
                            *c += (amount * sin(a + turn)) >> 12;
                        }
                    }
                } else {
                    bent.push((&self.swing[i], across, turn));
                }
            }
            let row = y as usize * w;
            let mut at = (y * self.ay) as usize;
            let mut noise = noise_at(0, y);
            for (x, o) in out[row..row + w].iter_mut().enumerate() {
                let mut t = self.straight[at] + rows + columns[x];
                for &(table, across, turn) in &bent {
                    t += i32::from(table[(((across[x] + turn) >> 4) & 4095) as usize]);
                }
                *o = shown(self.colors[t.clamp(0, ONE) as usize], noise, dither);
                at += self.ax as usize;
                noise = noise.wrapping_add(NOISE_STEP);
            }
        }
    }
}
