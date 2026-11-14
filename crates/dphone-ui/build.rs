//! Build-time asset pipeline: rasterizes the TrueType fonts in
//! `assets/fonts` into glyph atlases and the SVG artwork in `assets/icons`
//! and `assets/art` into per-skin bitmaps, so the phone renders with
//! integer blits only.  (Floating point is used here, on the build host.)
//!
//! Output: `$OUT_DIR/assets_gen.rs` plus one blob per table.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

const SKINS: [&str; 3] = ["SMALL_GRAY", "LARGE_GRAY", "COLOR"];

/// (font file, pixel size, digits only) per skin, in role order: Normal,
/// Bold, Small, Title, Large, then the two idle-screen roles: Clock (the
/// big time; digits and the colon only), Sub (meridiem and date), Number
/// (the number line of a call card), and Meridiem (AM/PM beside the clock).
///
/// Color sizes follow the stock screenshots: 14 px body text (cap height
/// 10), a 16 px title, and a clock whose digits are 64 px tall.
const FONTS: [[(&str, f32, bool); 9]; 3] = [
    // Gray body sizes are the stock `default_fontsize`: 10 and 11.
    [
        ("DroidSans.ttf", 10.0, false),
        ("DroidSans-Bold.ttf", 10.0, false),
        ("DroidSans.ttf", 9.0, false),
        ("DroidSans.ttf", 10.0, false),
        ("DroidSans-Bold.ttf", 19.0, false),
        ("DroidSans-Bold.ttf", 36.0, true),
        ("DroidSans.ttf", 10.0, false),
        ("DroidSans.ttf", 10.0, false),
        ("DroidSans.ttf", 10.0, false),
    ],
    [
        ("DroidSans.ttf", 11.0, false),
        ("DroidSans-Bold.ttf", 11.0, false),
        ("DroidSans.ttf", 10.0, false),
        ("DroidSans.ttf", 11.0, false),
        ("DroidSans-Bold.ttf", 24.0, false),
        ("DroidSans-Bold.ttf", 52.0, true),
        ("DroidSans.ttf", 11.0, false),
        ("DroidSans.ttf", 11.0, false),
        ("DroidSans.ttf", 11.0, false),
    ],
    [
        ("Roboto-Regular.ttf", 14.0, false),
        ("Roboto-Medium.ttf", 15.0, false),
        ("Roboto-Regular.ttf", 12.0, false),
        ("Roboto-Bold.ttf", 16.0, false),
        ("Roboto-Regular.ttf", 30.0, false),
        ("Roboto-Thin.ttf", 89.0, true),
        ("Roboto-Light.ttf", 22.0, false),
        ("Roboto-Regular.ttf", 17.0, false),
        ("Roboto-Regular.ttf", 22.0, false),
    ],
];

/// Asset classes: directory, pixel height per skin, and whether the color
/// skin keeps color (else an alpha mask tinted at draw time).
/// The last flag inverts luminance on the gray skins (artwork drawn for a
/// dark background, shown on the light gray screens).
const CLASSES: [(&str, [u32; 3], bool, bool); 3] = [
    ("icons/status", [13, 15, 22], false, false),
    ("icons/app", [24, 28, 48], true, false),
    ("art", [44, 60, 104], true, true),
];

const FMT_MASK4: u8 = 0;
const FMT_GRAY2: u8 = 1;
const FMT_RGBA_RLE: u8 = 2;

fn charset() -> Vec<char> {
    let mut v: Vec<char> = (32u32..127).chain(160..256).filter_map(char::from_u32).collect();
    v.extend(['\u{2013}', '\u{2014}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}', '\u{2026}']);
    v.sort();
    v
}

struct Nibbles(Vec<u8>, bool);
impl Nibbles {
    fn new() -> Self {
        Nibbles(Vec::new(), false)
    }
    fn push(&mut self, n: u8) {
        if self.1 {
            *self.0.last_mut().unwrap() |= n & 15;
        } else {
            self.0.push((n & 15) << 4);
        }
        self.1 = !self.1;
    }
    /// Byte-aligns and returns the bytes.
    fn finish(self) -> Vec<u8> {
        self.0
    }
}

#[allow(clippy::too_many_arguments)]
fn gen_font(out: &mut String, blobs: &Path, assets: &Path, name: &str, file: &str, px: f32, gray: bool, digits: bool) {
    let data = fs::read(assets.join("fonts").join(file)).expect("font file");
    let font = fontdue::Font::from_bytes(data, fontdue::FontSettings { scale: px, ..Default::default() })
        .expect("font parse");
    let lm = font.horizontal_line_metrics(px).expect("line metrics");
    let ascent = lm.ascent.round() as i32;
    let descent = (-lm.descent).round() as i32;
    let mut blob: Vec<u8> = Vec::new();
    let mut recs = String::new();
    for ch in charset() {
        if digits && !(ch.is_ascii_digit() || matches!(ch, ':' | ' ' | '-')) {
            continue;
        }
        if font.lookup_glyph_index(ch) == 0 && ch != ' ' {
            continue;
        }
        let (m, cov) = font.rasterize(ch, px);
        let off = blob.len();
        let mut nib = Nibbles::new();
        for &c in &cov {
            // Small gray text has no hinting; steepen the coverage curve so
            // stems land on solid levels instead of smearing over two.
            let c = if gray && px < 20.0 {
                let v = (c as i32 - 40) * 255 / 150;
                v.clamp(0, 255) as u8
            } else {
                c
            };
            nib.push(((c as u32 * 15 + 127) / 255) as u8);
        }
        blob.extend(nib.finish());
        let top = m.ymin + m.height as i32; // above baseline
        let adv = m.advance_width.round() as i32;
        writeln!(
            recs,
            "    GlyphRec {{ ch: {}, adv: {}, xoff: {}, top: {}, w: {}, h: {}, off: {} }},",
            ch as u32, adv, m.xmin, top, m.width, m.height, off
        )
        .unwrap();
    }
    let blob_path = blobs.join(format!("{name}.bin"));
    fs::write(&blob_path, &blob).unwrap();
    writeln!(
        out,
        "pub static {name}: FontData = FontData {{ px: {}, ascent: {ascent}, descent: {descent}, glyphs: &[\n{recs}], data: include_bytes!({:?}) }};",
        px as u32,
        blob_path
    )
    .unwrap();
}

fn render_svg(path: &Path, height: u32, opt: &resvg::usvg::Options) -> (u32, u32, Vec<u8>) {
    let data = fs::read(path).unwrap();
    let tree = resvg::usvg::Tree::from_data(&data, opt)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let size = tree.size();
    let scale = height as f32 / size.height();
    let width = (size.width() * scale).round().max(1.0) as u32;
    let mut pm = resvg::tiny_skia::Pixmap::new(width, height).unwrap();
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pm.as_mut());
    // Un-premultiply.
    let mut px = pm.take();
    for p in px.as_chunks_mut::<4>().0 {
        let a = p[3] as u32;
        if a != 0 && a != 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    (width, height, px)
}

fn encode(rgba: &[u8], fmt: u8, invert: bool) -> Vec<u8> {
    match fmt {
        FMT_MASK4 => {
            let mut n = Nibbles::new();
            for p in rgba.as_chunks::<4>().0 {
                n.push(((p[3] as u32 * 15 + 127) / 255) as u8);
            }
            n.finish()
        }
        FMT_GRAY2 => {
            let mut n = Nibbles::new();
            for p in rgba.as_chunks::<4>().0 {
                if p[3] < 128 {
                    n.push(15);
                } else {
                    let lum = (p[0] as u32 * 77 + p[1] as u32 * 150 + p[2] as u32 * 29) >> 8;
                    let lum = if invert { 255 - lum } else { lum };
                    n.push(((lum * 3 + 127) / 255) as u8);
                }
            }
            n.finish()
        }
        _ => {
            let mut out = Vec::new();
            let mut it = rgba.as_chunks::<4>().0.iter().peekable();
            while let Some(p) = it.next() {
                let mut run = 1u8;
                while run < 255 && it.peek().is_some_and(|q| *q == p || (p[3] == 0 && q[3] == 0)) {
                    it.next();
                    run += 1;
                }
                out.push(run);
                out.extend_from_slice(if p[3] == 0 { &[0, 0, 0, 0] } else { p });
            }
            out
        }
    }
}

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let assets = manifest.join("../../assets");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", assets.display());

    let mut out = String::from("// Generated by build.rs; do not edit.\nuse crate::font::{FontData, GlyphRec};\nuse crate::icons::IconRec;\n");

    // Fonts (deduplicated by file and size within a skin).
    for (si, skin) in SKINS.iter().enumerate() {
        let mut names: Vec<String> = Vec::new();
        let mut done: Vec<(&str, u32, bool, String)> = Vec::new();
        for (file, px, digits) in FONTS[si] {
            let key = (file, px as u32, digits);
            let name = match done.iter().find(|d| (d.0, d.1, d.2) == key) {
                Some(d) => d.3.clone(),
                None => {
                    let name = format!("FONT_{skin}_{}", done.len());
                    gen_font(&mut out, &out_dir, &assets, &name, file, px, si < 2, digits);
                    done.push((file, px as u32, digits, name.clone()));
                    name
                }
            };
            names.push(format!("&{name}"));
        }
        writeln!(out, "pub static FONTS_{skin}: [&FontData; 9] = [{}];", names.join(", ")).unwrap();
    }

    // Icons and art.
    let mut opt = resvg::usvg::Options::default();
    {
        let db = opt.fontdb_mut();
        for f in fs::read_dir(assets.join("fonts")).unwrap().flatten() {
            if f.path().extension().is_some_and(|e| e == "ttf") {
                db.load_font_data(fs::read(f.path()).unwrap());
            }
        }
    }
    for (si, skin) in SKINS.iter().enumerate() {
        let mut blob: Vec<u8> = Vec::new();
        let mut recs = String::new();
        let mut entries: Vec<(String, PathBuf, u32, u8, bool)> = Vec::new();
        for (dir, heights, colored, invert) in CLASSES {
            let d = assets.join(dir);
            let Ok(rd) = fs::read_dir(&d) else { continue };
            for f in rd.flatten() {
                let p = f.path();
                if p.extension().is_some_and(|e| e == "svg") {
                    let stem = p.file_stem().unwrap().to_string_lossy().into_owned();
                    // App icons get their own namespace: `app-contacts` beside
                    // the monochrome `contacts`.
                    let name = if dir == "icons/app" { format!("app-{stem}") } else { stem };
                    let fmt = match (colored, si) {
                        (false, _) => FMT_MASK4,
                        (true, 2) => FMT_RGBA_RLE,
                        (true, _) => FMT_GRAY2,
                    };
                    entries.push((name, p, heights[si], fmt, invert));
                }
            }
        }
        entries.sort();
        for (name, path, h, fmt, invert) in entries {
            let (w, h, rgba) = render_svg(&path, h, &opt);
            let enc = encode(&rgba, fmt, invert);
            writeln!(
                recs,
                "    IconRec {{ name: {name:?}, w: {w}, h: {h}, fmt: {fmt}, off: {}, len: {} }},",
                blob.len(),
                enc.len()
            )
            .unwrap();
            blob.extend(enc);
        }
        let blob_path = out_dir.join(format!("icons_{skin}.bin"));
        fs::write(&blob_path, &blob).unwrap();
        writeln!(
            out,
            "pub static ICONS_{skin}: &[IconRec] = &[\n{recs}];\npub static ICON_DATA_{skin}: &[u8] = include_bytes!({blob_path:?});"
        )
        .unwrap();
    }
    fs::write(out_dir.join("assets_gen.rs"), out).unwrap();
}
