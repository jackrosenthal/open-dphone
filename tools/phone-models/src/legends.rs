//! The key legends as SVG, rasterized: one picture over the whole face
//! (`Panel::extent`), transparent but for the ink.  The icons are those of
//! `web/src/lib/phoneIcons.tsx`; the type is Roboto Bold.

use crate::panel::{Legend, Panel};
use resvg::{tiny_skia, usvg};
use std::fmt::Write;
use std::path::Path;

/// An RGBA picture, row by row, not premultiplied.
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

const CUT: &str = "#17181a";

/// A handset as the phones print it: a C-shaped handle opening to the
/// right, and the earpiece and the mouthpiece as two caps at the handle's
/// ends, each set apart from the handle by a narrow gap.  20 high and 10
/// wide, centered on the origin, placed by `transform`.
fn handset(c: &str, transform: &str) -> String {
    format!(
        r#"<g transform="{transform}"><path d="M1.7-8.05C-4.63-8.05-4.63 8.05 1.7 8.05" fill="none" stroke="{c}" stroke-width="4.5"/><rect x="2.5" y="-10" width="2.7" height="5.4" rx="1.1" fill="{c}"/><rect x="2.5" y="4.6" width="2.7" height="5.4" rx="1.1" fill="{c}"/></g>"#
    )
}

/// The handset leaning back, earpiece up and to the left (hold, redial).
const LEANING: &str = "translate(9.6 14.2) rotate(-45) scale(.9)";

/// `body` crossed by the line `slash`, with a narrow gap cut out of the
/// body on each side of the line.
fn slashed(name: &str, body: &str, slash: &str, c: &str) -> String {
    format!(
        r##"<mask id="{name}-cut" maskUnits="userSpaceOnUse" x="-2" y="-2" width="28" height="28"><rect x="-2" y="-2" width="28" height="28" fill="#fff"/><path d="{slash}" stroke="#000" stroke-width="4.6" stroke-linecap="round"/></mask><g mask="url(#{name}-cut)">{body}</g><path d="{slash}" stroke="{c}" stroke-width="2" stroke-linecap="round"/>"##
    )
}

/// An icon in a 24 x 24 box.
fn icon(name: &str, c: &str) -> String {
    let s = format!(r#"fill="none" stroke="{c}" stroke-linecap="round" stroke-linejoin="round""#);
    match name {
        "check" => format!(r#"<path d="M4 13l5.5 5.5L20 6" {s} stroke-width="4"/>"#),
        "x" => format!(r#"<path d="M5 5l14 14M19 5L5 19" {s} stroke-width="4.5"/>"#),
        "up" => format!(r#"<path d="M5 16l7-7 7 7" {s} stroke-width="4"/>"#),
        "down" => format!(r#"<path d="M5 8l7 7 7-7" {s} stroke-width="4"/>"#),
        "left" => format!(r#"<path d="M16 5l-7 7 7 7" {s} stroke-width="4"/>"#),
        "right" => format!(r#"<path d="M8 5l7 7-7 7" {s} stroke-width="4"/>"#),
        "page-left" => format!(r#"<path d="M17 4.5L6 12l11 7.5z" fill="{c}"/>"#),
        "page-right" => format!(r#"<path d="M7 4.5L18 12 7 19.5z" fill="{c}"/>"#),
        "minus" => format!(r#"<path d="M4 12h16" {s} stroke-width="4"/>"#),
        "plus" => format!(r#"<path d="M4 12h16M12 4v16" {s} stroke-width="4"/>"#),
        // The dial pad's star: six square-ended arms.
        "asterisk" => format!(r#"<path d="M12 2v20M3.34 7l17.32 10M3.34 17l17.32-10" fill="none" stroke="{c}" stroke-width="3.4"/>"#),
        "msgs" => format!(
            r#"<rect x="1" y="5" width="22" height="14" rx="1.5" fill="{c}"/><path d="M1.5 6l10.5 8L22.5 6" fill="none" stroke="{CUT}" stroke-width="1.6"/>"#
        ),
        "xfer" => format!(
            r#"{}<path d="M9.6 12h5.6M13 8.8l3.2 3.2-3.2 3.2" {s} stroke-width="2.2"/>{}"#,
            handset(c, "translate(3 12) scale(.9)"),
            handset(c, "translate(21 12) scale(.9)")
        ),
        "redial" => format!(
            r#"{}<path d="M14.4 6.06A4.2 4.2 0 1 0 19.8 3.56" fill="none" stroke="{c}" stroke-width="2"/><path d="M16.15 3.3L19.8.9v4.8z" fill="{c}"/>"#,
            handset(c, LEANING)
        ),
        "hold" => format!(r#"{}<path d="M16.3 2.8v6.4M20.1 2.8v6.4" fill="none" stroke="{c}" stroke-width="2.2"/>"#, handset(c, LEANING)),
        "conf" => format!(
            "{}{}{}",
            handset(c, "translate(3.8 12) scale(.8)"),
            handset(c, "translate(12 12) scale(.8)"),
            handset(c, "translate(20.2 12) scale(.8)")
        ),
        "info" => format!(
            r#"<circle cx="12" cy="12" r="10" fill="{c}"/><path d="M12 10.5v7" stroke="{CUT}" stroke-width="3.2" stroke-linecap="round"/><circle cx="12" cy="6.2" r="1.9" fill="{CUT}"/>"#
        ),
        "apps" => format!(
            r#"<path d="M4 10.5l8-3 8 3v8.5l-8 3-8-3z" fill="{c}"/><path d="M4 10.5L1 6.5l8-3 3 4M20 10.5l3-4-8-3-3 4" {s} stroke-width="1.8"/><path d="M12 13.5V22" stroke="{CUT}" stroke-width="1.4"/>"#
        ),
        "dir" => format!(
            r#"<circle cx="7.5" cy="7" r="3.4" fill="{c}"/><circle cx="16.5" cy="7" r="3.4" fill="{c}"/><path d="M1.5 21q0-9 6-9t6 9zM10.5 21q0-9 6-9t6 9z" fill="{c}"/>"#
        ),
        "menu" => format!(
            r#"<path d="M5 11.5h14M5 16h14M5 20.5h14" fill="none" stroke="{c}" stroke-width="2.6"/><path d="M10 5l2.5 2.5L17.5 2" {s} stroke-width="2.2"/>"#
        ),
        "status" => format!(r#"<circle cx="12" cy="12" r="9" {s} stroke-width="2.4"/><path d="M7 12h10" {s} stroke-width="3"/>"#),
        // New family (D60, D65): drawn face on.
        "headset" => format!(
            r#"<path d="M5.4 11.5v-1a6.6 7.4 0 0 1 13.2 0v1" {s} stroke-width="1.5"/><rect x="3.6" y="9.4" width="3.6" height="7.8" rx="1.3" fill="{c}"/><rect x="16.8" y="9.4" width="3.6" height="7.8" rx="1.3" fill="{c}"/><path d="M3.1 11.5L1 13.3l2.1 1.8zM20.9 11.5l2.1 1.8-2.1 1.8z" fill="{c}"/><path d="M18.6 17.2q0 2.9-3.3 3.5" {s} stroke-width="1.4"/><circle cx="14.2" cy="20.8" r="1.25" fill="none" stroke="{c}" stroke-width="1.1"/>"#
        ),
        "speaker" => format!(
            r#"<path d="M2.2 10.7l1.5-1.9h2.4v6.4H3.7l-1.5-1.9z" fill="{c}"/><path d="M6.1 10.1Q7.2 4.8 9.6 4.8a4.4 7.2 0 0 1 0 14.4Q7.2 19.2 6.1 13.9z" {s} stroke-width="1.5"/><path d="M15.6 6.6a8.6 8.6 0 0 1 0 10.8M19 4a12 12 0 0 1 0 16" {s} stroke-width="1.8"/>"#
        ),
        "mute" => slashed(
            "mute",
            &format!(
                r#"<path d="M8.7 11.2V5.2q0-3.2 3.3-3.2t3.3 3.2v6z" fill="{c}"/><rect x="8.7" y="12.1" width="6.6" height="2.5" fill="{c}"/><path d="M6.9 13.2q0 4.6 5.1 4.6t5.1-4.6M8.3 21.1q3.7 1.9 7.4 0" {s} stroke-width="1.6"/>"#
            ),
            "M3.6 14.6L20.6 4.4",
            c,
        ),
        // Old family (D40, D50, D70): drawn turned, as on those phones.
        "headset-turned" => format!(
            r#"<path d="M5.3 14.6C4.1 7.3 10.1 2.4 15.6 5.4" {s} stroke-width="1.5"/><circle cx="7.6" cy="16" r="3.5" fill="{c}"/><circle cx="16.4" cy="8.9" r="3" fill="{c}"/><path d="M8.1 19.9q4.3 2.4 9.2-.6" {s} stroke-width="1.5"/>"#
        ),
        "speaker-turned" => format!(
            r#"<ellipse cx="9.6" cy="14.6" rx="4.4" ry="5.2" transform="rotate(-35 9.6 14.6)" {s} stroke-width="1.5"/><circle cx="6.7" cy="17.3" r="2.1" {s} stroke-width="1.5"/><path d="M9.1 5.6a9 9 0 0 1 9.4 9.4M10.2 1.9a12.6 12.6 0 0 1 11.9 12" {s} stroke-width="1.6"/>"#
        ),
        "mute-turned" => slashed(
            "mute-turned",
            &format!(
                r#"<rect x="9.3" y="2.4" width="5.4" height="10.8" rx="2.7" fill="{c}"/><path d="M7.3 10.6q0 5.9 4.7 5.9t4.7-5.9M12 16.5v2.6M8.7 20.6q3.3 1.8 6.6 0" {s} stroke-width="1.6"/>"#
            ),
            "M4.4 14.8L19.8 5.4",
            c,
        ),
        _ => panic!("no icon {name}"),
    }
}

/// The legends alone, on a transparent ground, over `p.extent()`: every
/// key's but the page rocker's, or (`rocker`) the page rocker's alone,
/// which the model prints on the rocker's surface.
pub fn svg(p: &Panel, px_per_unit: f64, rocker: bool) -> String {
    let [x0, y0, x1, y1] = p.extent();
    let (w, h) = (x1 - x0, y1 - y0);
    let mut o = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{x0} {y0} {w} {h}" width="{}" height="{}">"#,
        (w * px_per_unit).round(),
        (h * px_per_unit).round()
    );
    let text = r#"font-family="Roboto" font-weight="700" text-anchor="middle""#;
    for k in &p.keys {
        let Some(l) = &k.legend else { continue };
        if matches!(l, Legend::Ends(..)) != rocker {
            continue;
        }
        let ink = k.ink;
        match l {
            Legend::Ends(a, b) => {
                // An icon toward each end of a wide key, and a fine line
                // across the middle, where the key pivots.
                let s = k.size.unwrap();
                for (name, cx) in [(a, k.cx - k.w / 2.0 + 11.0), (b, k.cx + k.w / 2.0 - 11.0)] {
                    let _ = write!(o, r#"<g transform="translate({} {}) scale({})">{}</g>"#, cx - s / 2.0, k.cy - s / 2.0, s / 24.0, icon(name, ink));
                }
                let _ = write!(o, r##"<rect x="{}" y="{}" width="0.9" height="{}" fill="#9aa1a8"/>"##, k.cx - 0.45, k.cy - k.h / 2.0, k.h);
            }
            Legend::Icon(name) => {
                let s = k.size.unwrap_or(16.0);
                let _ = write!(o, r#"<g transform="translate({} {}) scale({})">{}</g>"#, k.cx - s / 2.0, k.cy - s / 2.0, s / 24.0, icon(name, ink));
            }
            Legend::Text(t) => {
                let fs = k.size.unwrap();
                let _ = write!(o, r#"<text x="{}" y="{}" font-size="{fs}" fill="{ink}" {text}>{t}</text>"#, k.cx, k.cy + fs * 0.355);
            }
            Legend::Digit(d, letters) => {
                if let Some(letters) = letters.filter(|l| !l.is_empty()) {
                    let fs = k.h * 0.23;
                    let y = k.cy - k.h * 0.27 + fs * 0.355;
                    let _ = write!(o, r#"<text x="{}" y="{y}" font-size="{fs}" fill="{ink}" {text}>{letters}</text>"#, k.cx);
                }
                // A key with a row for letters (empty on the 1 key) has the
                // digit below the row; a key without, in the middle.
                let fs = k.h * 0.52;
                let yc = k.cy + if letters.is_some() { k.h * 0.16 } else { 0.0 };
                let _ = write!(o, r#"<text x="{}" y="{}" font-size="{fs}" fill="{ink}" {text}>{d}</text>"#, k.cx, yc + fs * 0.355);
            }
        }
    }
    o.push_str("</svg>");
    o
}

/// Draws `svg` with the repository's Roboto Bold.
pub fn rasterize(svg: &str, fonts: &Path) -> Result<Rgba, String> {
    let mut opt = usvg::Options::default();
    let font = fonts.join("Roboto-Bold.ttf");
    opt.fontdb_mut().load_font_data(std::fs::read(&font).map_err(|e| format!("{}: {e}", font.display()))?);
    let tree = usvg::Tree::from_str(svg, &opt).map_err(|e| e.to_string())?;
    let size = tree.size().to_int_size();
    let mut pm = tiny_skia::Pixmap::new(size.width(), size.height()).ok_or("an empty picture")?;
    resvg::render(&tree, tiny_skia::Transform::default(), &mut pm.as_mut());
    let mut px = pm.take();
    for p in px.as_chunks_mut::<4>().0 {
        let a = p[3] as u32;
        if a != 0 && a != 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    Ok(Rgba { width: size.width(), height: size.height(), pixels: px })
}
