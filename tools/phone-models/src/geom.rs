//! Meshes: extrusions of outlines with rounded rims, lofts, and tubes.

use std::ops::{Add, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

pub const fn v3(x: f64, y: f64, z: f64) -> V3 {
    V3 { x, y, z }
}

impl V3 {
    pub fn dot(self, o: V3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: V3) -> V3 {
        v3(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
    }

    pub fn length(self) -> f64 {
        self.dot(self).sqrt()
    }

    pub fn normalized(self) -> V3 {
        let l = self.length();
        if l > 1e-12 { self * (1.0 / l) } else { self }
    }

    pub fn lerp(self, o: V3, t: f64) -> V3 {
        self + (o - self) * t
    }
}

impl Add for V3 {
    type Output = V3;
    fn add(self, o: V3) -> V3 {
        v3(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for V3 {
    type Output = V3;
    fn sub(self, o: V3) -> V3 {
        v3(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f64> for V3 {
    type Output = V3;
    fn mul(self, s: f64) -> V3 {
        v3(self.x * s, self.y * s, self.z * s)
    }
}

impl Neg for V3 {
    type Output = V3;
    fn neg(self) -> V3 {
        v3(-self.x, -self.y, -self.z)
    }
}

pub type P2 = (f64, f64);

/// A triangle mesh with a normal and a texture coordinate at each vertex
/// (texture coordinates from the top left, as glTF has them).
#[derive(Clone, Default)]
pub struct Mesh {
    pub pos: Vec<V3>,
    pub nrm: Vec<V3>,
    pub uv: Vec<P2>,
    pub idx: Vec<u32>,
}

impl Mesh {
    pub fn vertex(&mut self, p: V3, n: V3) -> u32 {
        self.vertex_uv(p, n, (0.0, 0.0))
    }

    pub fn vertex_uv(&mut self, p: V3, n: V3, uv: P2) -> u32 {
        self.pos.push(p);
        self.nrm.push(n);
        self.uv.push(uv);
        (self.pos.len() - 1) as u32
    }

    pub fn tri(&mut self, a: u32, b: u32, c: u32) {
        self.idx.extend([a, b, c]);
    }

    pub fn quad(&mut self, a: u32, b: u32, c: u32, d: u32) {
        self.idx.extend([a, b, c, a, c, d]);
    }

    pub fn append(&mut self, o: &Mesh) {
        let base = self.pos.len() as u32;
        self.pos.extend(&o.pos);
        self.nrm.extend(&o.nrm);
        self.uv.extend(&o.uv);
        self.idx.extend(o.idx.iter().map(|i| i + base));
    }

    /// Every position and normal through `f`, which must be a rotation and
    /// a translation (normals take only the rotation).
    pub fn map(mut self, f: impl Fn(V3) -> V3) -> Mesh {
        let o = f(V3::default());
        for (p, n) in self.pos.iter_mut().zip(&mut self.nrm) {
            *p = f(*p);
            *n = (f(*n) - o).normalized();
        }
        self
    }

    /// Normals from the faces around each vertex, weighted by the faces'
    /// areas.
    pub fn smooth_normals(&mut self) {
        let mut acc = vec![V3::default(); self.pos.len()];
        for t in self.idx.as_chunks::<3>().0 {
            let (a, b, c) = (self.pos[t[0] as usize], self.pos[t[1] as usize], self.pos[t[2] as usize]);
            let n = (b - a).cross(c - a);
            for &i in t {
                acc[i as usize] = acc[i as usize] + n;
            }
        }
        self.nrm = acc.into_iter().map(V3::normalized).collect();
    }
}

fn area2(pts: &[P2]) -> f64 {
    (0..pts.len())
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum()
}

/// The outline counterclockwise, without repeated points.
pub fn ccw(pts: &[P2]) -> Vec<P2> {
    let mut o: Vec<P2> = Vec::with_capacity(pts.len());
    for &p in pts {
        if o.last().is_none_or(|q: &P2| (q.0 - p.0).hypot(q.1 - p.1) > 1e-7) {
            o.push(p);
        }
    }
    while o.len() > 2 && (o[0].0 - o[o.len() - 1].0).hypot(o[0].1 - o[o.len() - 1].1) <= 1e-7 {
        o.pop();
    }
    if area2(&o) < 0.0 {
        o.reverse();
    }
    o
}

/// Triangles of a simple polygon (counterclockwise), by ear clipping.
pub fn triangulate(pts: &[P2]) -> Vec<[usize; 3]> {
    let mut idx: Vec<usize> = (0..pts.len()).collect();
    let mut out = Vec::with_capacity(pts.len().saturating_sub(2));
    let cross = |a: P2, b: P2, c: P2| (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    let inside = |p: P2, a: P2, b: P2, c: P2| cross(a, b, p) >= -1e-12 && cross(b, c, p) >= -1e-12 && cross(c, a, p) >= -1e-12;
    let mut stuck = 0;
    let mut i = 0;
    while idx.len() > 3 {
        let n = idx.len();
        let (ia, ib, ic) = (idx[(i + n - 1) % n], idx[i % n], idx[(i + 1) % n]);
        let (a, b, c) = (pts[ia], pts[ib], pts[ic]);
        let convex = cross(a, b, c) > 1e-12;
        let ear = convex
            && idx.iter().all(|&j| {
                j == ia || j == ib || j == ic || {
                    let p = pts[j];
                    // A point on the ear's corner (a repeated position) does
                    // not block the ear.
                    !inside(p, a, b, c) || [a, b, c].iter().any(|q| (q.0 - p.0).hypot(q.1 - p.1) < 1e-9)
                }
            });
        // A degenerate corner (no area) is dropped, and when nothing is an
        // ear (a slightly self-touching outline), the corner is clipped.
        if ear || cross(a, b, c).abs() <= 1e-12 || stuck > n {
            if cross(a, b, c).abs() > 1e-12 {
                out.push([ia, ib, ic]);
            }
            idx.remove(i % n);
            stuck = 0;
            i = if i == 0 { 0 } else { i - 1 };
        } else {
            i = (i + 1) % n;
            stuck += 1;
        }
    }
    if idx.len() == 3 {
        out.push([idx[0], idx[1], idx[2]]);
    }
    out
}

/// A rounded rectangle given by its box and the corners' radii (top left,
/// top right, bottom right, bottom left), in a frame with y down (the
/// panel's units).
pub fn rrect(x: f64, y: f64, w: f64, h: f64, r: [f64; 4], seg: usize) -> Vec<P2> {
    let corners = [
        (x + r[0], y + r[0], r[0], 180.0),
        (x + w - r[1], y + r[1], r[1], 270.0),
        (x + w - r[2], y + h - r[2], r[2], 0.0),
        (x + r[3], y + h - r[3], r[3], 90.0),
    ];
    let mut pts = Vec::new();
    for (cx, cy, rad, a0) in corners {
        if rad <= 0.0 {
            pts.push((cx, cy));
            continue;
        }
        for i in 0..=seg {
            let a = (a0 + 90.0 * i as f64 / seg as f64).to_radians();
            pts.push((cx + rad * a.cos(), cy + rad * a.sin()));
        }
    }
    pts
}

pub fn circle(cx: f64, cy: f64, r: f64, seg: usize) -> Vec<P2> {
    (0..seg)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / seg as f64;
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

/// Points of a quadratic curve, without the first and the last.
fn quad_pts(p0: P2, c: P2, p1: P2, n: usize) -> Vec<P2> {
    (1..n)
        .map(|i| {
            let t = i as f64 / n as f64;
            let (a, b, d) = ((1.0 - t) * (1.0 - t), 2.0 * t * (1.0 - t), t * t);
            (a * p0.0 + b * c.0 + d * p1.0, a * p0.1 + b * c.1 + d * p1.1)
        })
        .collect()
}

/// A key with straight sides and top and bottom edges that bow outward by
/// `a` at the middle (parabolas), within the box (x, y, w, h), with
/// corners rounded by `r` (the panel's units, y down).
pub fn arched(x: f64, y: f64, w: f64, h: f64, r: f64, a: f64, n: usize) -> Vec<P2> {
    let top = |u: f64| y + a * (1.0 - 2.0 * (u - x) / w).powi(2);
    let bot = |u: f64| y + h - a * (1.0 - 2.0 * (u - x) / w).powi(2);
    let mut pts = vec![(x, y + a + r)];
    pts.extend(quad_pts((x, y + a + r), (x, y + a), (x + r, top(x + r)), 6));
    pts.extend((1..n).map(|i| {
        let u = x + r + (w - 2.0 * r) * i as f64 / n as f64;
        (u, top(u))
    }));
    pts.push((x + w - r, top(x + w - r)));
    pts.extend(quad_pts((x + w - r, top(x + w - r)), (x + w, y + a), (x + w, y + a + r), 6));
    pts.push((x + w, y + h - a - r));
    pts.extend(quad_pts((x + w, y + h - a - r), (x + w, y + h - a), (x + w - r, bot(x + w - r)), 6));
    pts.extend((1..n).map(|i| {
        let u = x + w - r - (w - 2.0 * r) * i as f64 / n as f64;
        (u, bot(u))
    }));
    pts.push((x + r, bot(x + r)));
    pts.extend(quad_pts((x + r, bot(x + r)), (x, y + h - a), (x, y + h - a - r), 6));
    pts
}

/// The outline `pts` (its plane's x and y) from height z0 to z1 along z,
/// with both rims rounded: a quarter circle of radius `bevel` in `segs`
/// segments.  A corner sharper than 40 degrees keeps a sharp edge.
pub fn prism(pts: &[P2], z0: f64, z1: f64, bevel: f64, segs: usize) -> Mesh {
    let pts = ccw(pts);
    let n = pts.len();
    let (mut lo, mut hi) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
    for p in &pts {
        lo = (lo.0.min(p.0), lo.1.min(p.1));
        hi = (hi.0.max(p.0), hi.1.max(p.1));
    }
    let b = bevel.min((z1 - z0) * 0.49).min((hi.0 - lo.0).min(hi.1 - lo.1) * 0.45).max(0.0);
    let segs = if b > 0.0 { segs.max(1) } else { 0 };

    // Each corner: the outward normal of the edge before and after, and the
    // direction and distance by which a point moves inward per unit inset.
    struct Corner {
        n_in: (f64, f64),
        n_out: (f64, f64),
        miter: (f64, f64),
        split: bool,
    }
    let edge_normal = |a: P2, b: P2| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let l = dx.hypot(dy).max(1e-12);
        (dy / l, -dx / l)
    };
    let corners: Vec<Corner> = (0..n)
        .map(|i| {
            let (p, q, r) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
            let (a, c) = (edge_normal(p, q), edge_normal(q, r));
            let (mx, my) = (a.0 + c.0, a.1 + c.1);
            let ml = mx.hypot(my);
            let m = if ml > 1e-9 { (mx / ml, my / ml) } else { a };
            let cos_half = (m.0 * a.0 + m.1 * a.1).max(0.25);
            let split = a.0 * c.0 + a.1 * c.1 < 40f64.to_radians().cos();
            Corner { n_in: a, n_out: c, miter: (m.0 / cos_half, m.1 / cos_half), split }
        })
        .collect();

    // Rings from the bottom cap up to the top cap: (inset, z, horizontal
    // and vertical parts of the normal).
    let mut rings: Vec<(f64, f64, f64, f64)> = Vec::new();
    if segs == 0 {
        rings.push((0.0, z0, 1.0, 0.0));
        rings.push((0.0, z1, 1.0, 0.0));
    } else {
        for k in 0..=segs {
            let f = std::f64::consts::FRAC_PI_2 * k as f64 / segs as f64;
            rings.push((b * (1.0 - f.sin()), z0 + b * (1.0 - f.cos()), f.sin(), -f.cos()));
        }
        for k in 0..=segs {
            let f = std::f64::consts::FRAC_PI_2 * k as f64 / segs as f64;
            rings.push((b * (1.0 - f.cos()), z1 - b + b * f.sin(), f.cos(), f.sin()));
        }
    }

    let mut m = Mesh::default();
    let at = |i: usize, inset: f64| (pts[i].0 - corners[i].miter.0 * inset, pts[i].1 - corners[i].miter.1 * inset);
    // Per ring, per corner: the vertex that the edge into the corner uses,
    // and the one that the edge out of the corner uses.
    let mut cols: Vec<Vec<(u32, u32)>> = Vec::with_capacity(rings.len());
    for &(inset, z, nh, nz) in &rings {
        let mut row = Vec::with_capacity(n);
        for (i, c) in corners.iter().enumerate() {
            let (x, y) = at(i, inset);
            let p = v3(x, y, z);
            if c.split && nh.abs() > 1e-9 {
                let a = m.vertex(p, v3(c.n_in.0 * nh, c.n_in.1 * nh, nz).normalized());
                let b = m.vertex(p, v3(c.n_out.0 * nh, c.n_out.1 * nh, nz).normalized());
                row.push((a, b));
            } else {
                let (mx, my) = (c.n_in.0 + c.n_out.0, c.n_in.1 + c.n_out.1);
                let ml = mx.hypot(my).max(1e-12);
                let v = m.vertex(p, v3(mx / ml * nh, my / ml * nh, nz).normalized());
                row.push((v, v));
            }
        }
        cols.push(row);
    }
    for k in 0..rings.len() - 1 {
        for i in 0..n {
            let j = (i + 1) % n;
            m.quad(cols[k][i].1, cols[k][j].0, cols[k + 1][j].0, cols[k + 1][i].1);
        }
    }
    // The caps.
    let tris = triangulate(&(0..n).map(|i| at(i, b)).collect::<Vec<_>>());
    let cap = |m: &mut Mesh, z: f64, up: bool| {
        let base = m.pos.len() as u32;
        for i in 0..n {
            let (x, y) = at(i, b);
            m.vertex(v3(x, y, z), v3(0.0, 0.0, if up { 1.0 } else { -1.0 }));
        }
        for t in &tris {
            let [a, b_, c] = t.map(|i| base + i as u32);
            if up { m.tri(a, b_, c) } else { m.tri(a, c, b_) }
        }
    };
    cap(&mut m, z1, true);
    cap(&mut m, z0, false);
    m
}

/// A tube of radius `r` along `path`, with `sides` sides, open at the
/// ends.  The cross-section is carried along the path without twist.
pub fn tube(path: &[V3], r: f64, sides: usize) -> Mesh {
    let n = path.len();
    let mut m = Mesh::default();
    let tangent = |i: usize| (path[(i + 1).min(n - 1)] - path[i.saturating_sub(1)]).normalized();
    let t0 = tangent(0);
    let helper = if t0.z.abs() < 0.9 { v3(0.0, 0.0, 1.0) } else { v3(1.0, 0.0, 0.0) };
    let mut side = t0.cross(helper).normalized();
    for (i, &p) in path.iter().enumerate() {
        let t = tangent(i);
        side = (side - t * side.dot(t)).normalized();
        let up = t.cross(side);
        for k in 0..sides {
            let a = std::f64::consts::TAU * k as f64 / sides as f64;
            let d = side * a.cos() + up * a.sin();
            m.vertex(p + d * r, d);
        }
    }
    for i in 0..n - 1 {
        for k in 0..sides {
            let k1 = (k + 1) % sides;
            let s = sides as u32;
            let (a, b, c, d) = (i as u32 * s + k as u32, i as u32 * s + k1 as u32, (i as u32 + 1) * s + k1 as u32, (i as u32 + 1) * s + k as u32);
            m.quad(a, b, c, d);
        }
    }
    m
}
