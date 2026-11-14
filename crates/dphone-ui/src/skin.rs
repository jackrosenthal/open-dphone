//! The three skins: metrics, colors, fonts, and icons per screen class.
//!
//! The geometry here is measured from stock firmware screenshots (the
//! owner's D60 captures in `screenshots/` and the per-model renders in
//! `docs/assets/` of the reverse-engineering notes), so that line-key
//! labels sit beside their physical keys and softkey labels above theirs:
//!
//! | | D40/D45/D50 | D70 | D60/D62/D65 |
//! | --- | --- | --- | --- |
//! | Line-key tab column | x 155..239 | x 206..319 | x 297..479 |
//! | Tab pitch from y = 0 | 23 | 23 | 40 |
//! | Header band (left of the tabs) | y 0..14 | y 0..16 | y 0..39 |
//! | Softkey shapes | y 103..119 | y 141..159 | y 244..268 |
//! | Softkey x: first, pitch, width | 0, 60, 60 | 0, 80, 80 | 14, 116, 104 |
//!
//! Colors of the color skin are the stock values by measurement; the
//! wallpaper and every bitmap are our own.

use crate::appview::{Color, Font, ThemeColor};
use crate::atlas;
use crate::font::FontData;
use crate::gfx::{rgb, IRect, Rgb, GRAY};
use crate::icons::{Icon, IconRec};
use dphone_state::{Model, Skin};

/// Everything skin-dependent.
pub struct Theme {
    pub skin: Skin,
    pub gray: bool,
    fonts: &'static [&'static FontData; 9],
    icons: &'static [IconRec],
    icon_data: &'static [u8],
    /// Main screen size.
    pub width: i32,
    pub height: i32,
    /// Height reserved for softkeys at the bottom (18, 20, 29).
    pub softkey_h: i32,
    /// Left edge of the first softkey shape, distance between shapes, and
    /// the width, top, and height of each shape.
    pub soft_x0: i32,
    pub soft_pitch: i32,
    pub soft_w: i32,
    pub soft_y: i32,
    pub soft_h: i32,
    /// Line-key tabs: left edge (including the border), width, and pitch.
    /// Tab `i` occupies rows `i * tab_h .. (i + 1) * tab_h` from the top of
    /// the screen, beside physical line key `i + 1`.
    pub tab_x: i32,
    pub tab_w: i32,
    pub tab_h: i32,
    /// Height of the header band at the top left of the idle and call
    /// screens (15, 17, 40).
    pub header_h: i32,
    /// `Text.LINE_HEIGHT`.
    pub line_h: i32,
    /// `Text.ROW_HEIGHT`.
    pub row_h: i32,
    /// `Text.TITLE_HEIGHT`.
    pub title_h: i32,
    pub scroll_w: i32,
    /// General padding.
    pub pad: i32,
    /// Left inset of titles and header text.
    pub title_pad: i32,
    /// Size of status icons.
    pub icon: i32,
    pub bg: Rgb,
    pub fg: Rgb,
    pub title_bg: Rgb,
    pub title_fg: Rgb,
    pub sel_bg: Rgb,
    pub sel_fg: Rgb,
    pub secondary: Rgb,
    pub border: Rgb,
    pub modal_bg: Rgb,
    /// Softkey fill, outline, and label.
    pub sk_bg: Rgb,
    pub sk_edge: Rgb,
    pub sk_fg: Rgb,
    /// Softkey fills for "go" (answer) and "stop" (end, ignore) actions.
    pub sk_go: Rgb,
    pub sk_stop: Rgb,
    pub good: Rgb,
    pub warn: Rgb,
    pub error: Rgb,
    pub accent: Rgb,
    /// Call card: fill for an active call, a ringing call, and a held one;
    /// and the dark part at its right.
    pub card: Rgb,
    pub card_ringing: Rgb,
    pub card_held: Rgb,
    pub card_dark: Rgb,
    /// Line-key tab fill.
    pub tab_bg: Rgb,
    pub tab_fg: Rgb,
}

static SMALL_GRAY: Theme = gray_theme(Skin::SmallGray);
static LARGE_GRAY: Theme = gray_theme(Skin::LargeGray);
static COLOR: Theme = Theme {
    skin: Skin::Color,
    gray: false,
    fonts: &atlas::FONTS_COLOR,
    icons: atlas::ICONS_COLOR,
    icon_data: atlas::ICON_DATA_COLOR,
    width: 480,
    height: 272,
    softkey_h: 29,
    soft_x0: 14,
    soft_pitch: 116,
    soft_w: 104,
    soft_y: 244,
    soft_h: 25,
    tab_x: 296,
    tab_w: 184,
    tab_h: 40,
    header_h: 40,
    line_h: 25,
    row_h: 32,
    title_h: 40,
    scroll_w: 8,
    pad: 8,
    title_pad: 15,
    icon: 22,
    bg: rgb(0x21, 0x21, 0x21),
    fg: rgb(0xEE, 0xEE, 0xEE),
    title_bg: rgb(0x01, 0x41, 0x74),
    title_fg: rgb(0xFF, 0xFF, 0xFF),
    sel_bg: rgb(0x01, 0x57, 0x9B),
    sel_fg: rgb(0xFF, 0xFF, 0xFF),
    secondary: rgb(0xB4, 0xB8, 0xBE),
    border: rgb(0x4D, 0x4D, 0x4D),
    modal_bg: rgb(0x2E, 0x2E, 0x2E),
    sk_bg: rgb(0x2E, 0x2E, 0x2E),
    sk_edge: rgb(0x75, 0x75, 0x75),
    sk_fg: rgb(0xEE, 0xEE, 0xEE),
    sk_go: rgb(0x38, 0x8E, 0x3C),
    sk_stop: rgb(0xB7, 0x1C, 0x1C),
    good: rgb(0x4C, 0xAF, 0x50),
    warn: rgb(0xFF, 0xB3, 0x00),
    error: rgb(0xEF, 0x53, 0x50),
    accent: rgb(0x67, 0xB4, 0xF1),
    card: rgb(0x01, 0x57, 0x9B),
    card_ringing: rgb(0x2A, 0x6B, 0x2D),
    card_held: rgb(0x45, 0x5A, 0x64),
    card_dark: rgb(0x21, 0x21, 0x21),
    tab_bg: rgb(0x42, 0x42, 0x42),
    tab_fg: rgb(0xEE, 0xEE, 0xEE),
};

const fn gray_theme(skin: Skin) -> Theme {
    let large = matches!(skin, Skin::LargeGray);
    Theme {
        skin,
        gray: true,
        fonts: if large { &atlas::FONTS_LARGE_GRAY } else { &atlas::FONTS_SMALL_GRAY },
        icons: if large { atlas::ICONS_LARGE_GRAY } else { atlas::ICONS_SMALL_GRAY },
        icon_data: if large { atlas::ICON_DATA_LARGE_GRAY } else { atlas::ICON_DATA_SMALL_GRAY },
        width: if large { 320 } else { 240 },
        height: if large { 160 } else { 120 },
        softkey_h: if large { 20 } else { 18 },
        soft_x0: 0,
        soft_pitch: if large { 80 } else { 60 },
        soft_w: if large { 80 } else { 60 },
        soft_y: if large { 141 } else { 103 },
        soft_h: if large { 19 } else { 17 },
        tab_x: if large { 206 } else { 155 },
        tab_w: if large { 114 } else { 85 },
        tab_h: 23,
        header_h: if large { 17 } else { 15 },
        line_h: if large { 17 } else { 15 },
        row_h: if large { 17 } else { 15 },
        title_h: if large { 17 } else { 15 },
        scroll_w: 4,
        pad: if large { 4 } else { 3 },
        title_pad: 3,
        icon: if large { 15 } else { 13 },
        bg: GRAY[3],
        fg: GRAY[0],
        title_bg: GRAY[0],
        title_fg: GRAY[3],
        sel_bg: GRAY[0],
        sel_fg: GRAY[3],
        secondary: GRAY[1],
        border: GRAY[1],
        modal_bg: GRAY[3],
        sk_bg: GRAY[0],
        sk_edge: GRAY[1],
        sk_fg: GRAY[3],
        sk_go: GRAY[0],
        sk_stop: GRAY[0],
        good: GRAY[0],
        warn: GRAY[0],
        error: GRAY[0],
        accent: GRAY[0],
        card: GRAY[3],
        card_ringing: GRAY[3],
        card_held: GRAY[3],
        card_dark: GRAY[3],
        tab_bg: GRAY[3],
        tab_fg: GRAY[0],
    }
}

impl Theme {
    pub fn for_skin(skin: Skin) -> &'static Theme {
        match skin {
            Skin::SmallGray => &SMALL_GRAY,
            Skin::LargeGray => &LARGE_GRAY,
            Skin::Color => &COLOR,
        }
    }
    pub fn for_model(model: Model) -> &'static Theme {
        Self::for_skin(model.skin())
    }

    pub fn font(&self, f: Font) -> &'static FontData {
        self.fonts[match f {
            Font::Normal => 0,
            Font::Bold => 1,
            Font::Small => 2,
            Font::Title => 3,
            Font::Large => 4,
        }]
    }

    /// The idle clock's digits (digits, colon, and space only).
    pub fn clock_font(&self) -> &'static FontData {
        self.fonts[5]
    }
    /// The idle screen's meridiem and date.
    pub fn date_font(&self) -> &'static FontData {
        self.fonts[6]
    }
    /// The number line of a call card (stock: 17 px on color).
    pub fn number_font(&self) -> &'static FontData {
        self.fonts[7]
    }
    /// The AM/PM beside the idle clock.
    pub fn meridiem_font(&self) -> &'static FontData {
        self.fonts[8]
    }

    pub fn text_width(&self, f: Font, s: &str) -> u16 {
        self.font(f).width(s).clamp(0, u16::MAX as i32) as u16
    }

    pub fn icon(&self, name: &str) -> Option<Icon> {
        let i = self.icons.binary_search_by(|r| r.name.cmp(name)).ok()?;
        Some(Icon { rec: &self.icons[i], data: self.icon_data })
    }

    pub fn theme_color(&self, c: ThemeColor) -> Rgb {
        match c {
            ThemeColor::Background => self.bg,
            ThemeColor::Foreground => self.fg,
            ThemeColor::TitleBackground => self.title_bg,
            ThemeColor::TitleForeground => self.title_fg,
            ThemeColor::SelectBackground => self.sel_bg,
            ThemeColor::SelectForeground => self.sel_fg,
            ThemeColor::Secondary => self.secondary,
            ThemeColor::Border => self.border,
            ThemeColor::ModalBackground => self.modal_bg,
            ThemeColor::SoftkeyBackground => self.sk_bg,
            ThemeColor::SoftkeyForeground => self.sk_fg,
            ThemeColor::Good => self.good,
            ThemeColor::Warning => self.warn,
            ThemeColor::Error => self.error,
        }
    }

    /// Resolves an app color; `None` means "do not paint".
    pub fn resolve(&self, c: Color, default: Option<Rgb>) -> Option<Rgb> {
        match c {
            Color::Default => default,
            Color::Transparent => None,
            Color::Theme(t) => Some(self.theme_color(t)),
            Color::Rgb(r, g, b) => Some(rgb(r, g, b)),
        }
    }

    /// The app content area on the main screen.
    pub fn content_h(&self) -> i32 {
        self.height - self.softkey_h
    }

    /// The outline of softkey shape `i` (0..4): the rectangle that holds
    /// it, including the pointed part on the color skin.
    pub fn softkey_rect(&self, i: usize) -> IRect {
        IRect::new(self.soft_x0 + i as i32 * self.soft_pitch, self.soft_y, self.soft_w, self.soft_h)
    }

    /// Line-key tab `i` (0 = beside line key 1), border included.
    pub fn tab_rect(&self, i: usize) -> IRect {
        IRect::new(self.tab_x, i as i32 * self.tab_h, self.tab_w, self.tab_h)
    }

    /// The header band left of the tab column.
    pub fn header_rect(&self) -> IRect {
        IRect::new(0, 0, self.tab_x - if self.gray { 1 } else { 0 }, self.header_h)
    }
}
