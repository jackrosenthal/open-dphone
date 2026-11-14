//! Built-in icons and artwork, rasterized per skin from `assets/` at build
//! time.

/// Encodings of [`IconRec`] data.
pub(crate) const FMT_MASK4: u8 = 0;
pub(crate) const FMT_GRAY2: u8 = 1;

/// One bitmap in a skin's icon table.
#[derive(Clone, Copy, Debug)]
pub struct IconRec {
    pub name: &'static str,
    pub w: u16,
    pub h: u16,
    /// 0 ([`FMT_MASK4`]): 4-bit alpha mask (tinted when drawn); 1: four gray levels plus
    /// transparency; 2: run-length RGBA.
    pub fmt: u8,
    pub off: u32,
    pub len: u32,
}

/// A decodable icon: its record and the skin's data blob.
#[derive(Clone, Copy)]
pub struct Icon {
    pub rec: &'static IconRec,
    pub data: &'static [u8],
}

impl Icon {
    pub fn bytes(&self) -> &'static [u8] {
        &self.data[self.rec.off as usize..(self.rec.off + self.rec.len) as usize]
    }
    pub fn width(&self) -> i32 {
        self.rec.w as i32
    }
    pub fn height(&self) -> i32 {
        self.rec.h as i32
    }
}

/// Names of the built-in icons (the same set in every skin).  Icons from
/// `assets/icons/status` are monochrome and take the text color; the app
/// icons from `assets/icons/app` are named `app-<file>` (`app-contacts`)
/// and are full color on the color skin; `wordmark` and `splash` are the
/// artwork.
pub fn names() -> Vec<&'static str> {
    crate::atlas::ICONS_COLOR.iter().map(|r| r.name).collect()
}
