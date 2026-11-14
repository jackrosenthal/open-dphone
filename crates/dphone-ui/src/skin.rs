//! The three skins: metrics, colors, fonts, and icons per screen class.

use crate::appview::{Color, Font, ThemeColor};
use crate::font::FontData;
use crate::atlas;
use crate::gfx::{rgb, Rgb, GRAY};
use crate::icons::{Icon, IconRec};
use dphone_state::{Model, Skin};

/// Everything skin-dependent.
pub struct Theme {
    pub skin: Skin,
    pub gray: bool,
    fonts: &'static [&'static FontData; 5],
    icons: &'static [IconRec],
    icon_data: &'static [u8],
    /// Main screen size.
    pub width: i32,
    pub height: i32,
    /// Softkey bar height (18, 20, 29).
    pub softkey_h: i32,
    /// Status bar at the top of native screens.
    pub status_h: i32,
    /// `Text.LINE_HEIGHT`.
    pub line_h: i32,
    /// `Text.ROW_HEIGHT`.
    pub row_h: i32,
    /// `Text.TITLE_HEIGHT`.
    pub title_h: i32,
    pub scroll_w: i32,
    /// General padding.
    pub pad: i32,
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
    pub sk_bg: Rgb,
    pub sk_fg: Rgb,
    pub good: Rgb,
    pub warn: Rgb,
    pub error: Rgb,
    pub accent: Rgb,
    /// Card background (call cards, line tabs).
    pub card: Rgb,
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
    status_h: 30,
    line_h: 25,
    row_h: 32,
    title_h: 40,
    scroll_w: 8,
    pad: 8,
    icon: 22,
    bg: rgb(0x10, 0x14, 0x1A),
    fg: rgb(0xF2, 0xF4, 0xF7),
    title_bg: rgb(0x1B, 0x24, 0x30),
    title_fg: rgb(0xFF, 0xFF, 0xFF),
    sel_bg: rgb(0x2F, 0x80, 0xED),
    sel_fg: rgb(0xFF, 0xFF, 0xFF),
    secondary: rgb(0x9A, 0xA4, 0xB2),
    border: rgb(0x2A, 0x34, 0x41),
    modal_bg: rgb(0x22, 0x2C, 0x3A),
    sk_bg: rgb(0x1B, 0x24, 0x30),
    sk_fg: rgb(0xE6, 0xEA, 0xF0),
    good: rgb(0x34, 0xC7, 0x59),
    warn: rgb(0xFF, 0xB0, 0x20),
    error: rgb(0xFF, 0x4D, 0x4F),
    accent: rgb(0x5A, 0xA2, 0xFF),
    card: rgb(0x1B, 0x24, 0x30),
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
        status_h: if large { 17 } else { 15 },
        line_h: if large { 17 } else { 15 },
        row_h: if large { 17 } else { 15 },
        title_h: if large { 17 } else { 15 },
        scroll_w: 4,
        pad: if large { 4 } else { 3 },
        icon: if large { 13 } else { 11 },
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
        sk_fg: GRAY[3],
        good: GRAY[0],
        warn: GRAY[0],
        error: GRAY[0],
        accent: GRAY[0],
        card: GRAY[3],
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
}
