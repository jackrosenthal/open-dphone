//! Glyph tables built by `build.rs`: pre-rasterized outline fonts with
//! 4-bit coverage (color skin), and bitmap fonts, one bit per pixel (gray
//! skins).

/// One glyph, row-major, byte-aligned at its start.  With 4-bit coverage:
/// high nibble first.  One bit per pixel: most significant bit first, each
/// row padded to a byte.
#[derive(Clone, Copy, Debug)]
pub struct GlyphRec {
    /// Offset of the glyph's data in the font's blob.
    pub off: u32,
    /// The code point (all are in the Basic Multilingual Plane).
    pub ch: u16,
    pub adv: i8,
    /// Left edge relative to the pen.
    pub xoff: i8,
    /// Top edge above the baseline.
    pub top: i8,
    pub w: u8,
    pub h: u8,
}

/// One face at one pixel size.
#[derive(Debug)]
pub struct FontData {
    pub px: u8,
    pub ascent: i8,
    pub descent: i8,
    /// Bits per pixel of the glyph data: 4 (coverage, blended) or 1 (a
    /// bitmap font: every set pixel is drawn solid).
    pub bits: u8,
    /// Where to look for a character this font lacks.
    pub fallback: Option<&'static FontData>,
    /// Sorted by `ch`.
    pub glyphs: &'static [GlyphRec],
    pub data: &'static [u8],
}

impl FontData {
    pub fn glyph(&self, ch: char) -> Option<&GlyphRec> {
        let c = u16::try_from(ch as u32).ok()?;
        self.glyphs.binary_search_by_key(&c, |g| g.ch).ok().map(|i| &self.glyphs[i])
    }

    /// The glyph for `ch` and the font that holds it: this font's, else
    /// the fallback font's, else the fallback's replacement character
    /// (U+FFFD), else this font's `?`.
    pub fn glyph_or_fallback(&self, ch: char) -> Option<(&FontData, &GlyphRec)> {
        if let Some(g) = self.glyph(ch) {
            return Some((self, g));
        }
        if let Some(f) = self.fallback
            && let Some(g) = f.glyph(ch).or_else(|| f.glyph('\u{FFFD}'))
        {
            return Some((f, g));
        }
        self.glyph('?').map(|g| (self, g))
    }

    /// Advance width of a string in pixels.
    pub fn width(&self, s: &str) -> i32 {
        s.chars().filter_map(|c| self.glyph_or_fallback(c)).map(|(_, g)| g.adv as i32).sum()
    }

    /// Ascent plus descent.
    pub fn height(&self) -> i32 {
        self.ascent as i32 + self.descent as i32
    }

    /// The longest prefix of `s` (in bytes) that fits in `max` pixels.
    pub fn fit(&self, s: &str, max: i32) -> usize {
        let mut w = 0;
        for (i, c) in s.char_indices() {
            w += self.glyph_or_fallback(c).map(|(_, g)| g.adv as i32).unwrap_or(0);
            if w > max {
                return i;
            }
        }
        s.len()
    }

    /// Greedy word wrap into lines no wider than `max`.
    pub fn wrap<'a>(&self, s: &'a str, max: i32) -> Vec<&'a str> {
        let mut lines = Vec::new();
        for para in s.split('\n') {
            let mut rest = para;
            loop {
                let n = self.fit(rest, max);
                if n == rest.len() {
                    lines.push(rest);
                    break;
                }
                // Break at the last space within the fitting prefix.
                let cut = rest[..n].rfind(' ').filter(|c| *c > 0);
                let (line, next) = match cut {
                    Some(c) => (&rest[..c], &rest[c + 1..]),
                    None => {
                        let n = if n == 0 { rest.chars().next().map(|c| c.len_utf8()).unwrap_or(0) } else { n };
                        (&rest[..n], &rest[n..])
                    }
                };
                lines.push(line);
                rest = next;
                if rest.is_empty() {
                    break;
                }
            }
        }
        lines
    }
}
