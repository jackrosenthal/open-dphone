//! Palette files: the colors of every screen of a color model, as a
//! provisioning server supplies them beside a wallpaper picture
//! (`dphone_palette_file`).  Protobuf text format; the message and the
//! file's documentation are in `proto/dphone/palette/v1/palette.proto`.
//!
//! This module reads the file.  What the colors mean, and how a whole
//! palette is derived from `primary` and `surface`, is the UI's business.

use buffa::text::{TextDecoder, TextFormat as _};

#[allow(unused_imports, clippy::all)]
mod pb {
    include!(concat!(env!("OUT_DIR"), "/dphone.palette.v1.rs"));
}

pub use pb::Palette as PaletteMessage;

/// Where a provisioned wallpaper picture is kept, relative to the nvdata
/// root (stock's name for it), and the setting that names its source.
pub const WALLPAPER_FILE: &str = "user_wallpaper.png";
pub const WALLPAPER_SETTING: &str = "wallpaper_file";
/// Where the palette file that came with it is kept, and its setting.
pub const PALETTE_FILE: &str = "open/wallpaper_palette.txtpb";
pub const PALETTE_SETTING: &str = "dphone_palette_file";

/// The most a palette file may be (a complete one is under 1 KiB).
pub const MAX_PALETTE_FILE: usize = 16 * 1024;

/// The roles a palette file can set, by field name.
pub const ROLES: [&str; 27] = [
    "bg",
    "fg",
    "title_bg",
    "title_fg",
    "sel_bg",
    "sel_fg",
    "secondary",
    "border",
    "modal_bg",
    "sk_bg",
    "sk_edge",
    "sk_fg",
    "sk_go",
    "sk_stop",
    "good",
    "warn",
    "error",
    "accent",
    "card",
    "card_ringing",
    "card_held",
    "card_dark",
    "tab_bg",
    "tab_fg",
    "load_track",
    "load_fill",
    "wallpaper_fg",
];

/// A palette file as read.  Colors are `0x00RRGGBB`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaletteFile {
    /// Derive a light palette from the two colors below.
    pub light: bool,
    pub primary: Option<u32>,
    pub surface: Option<u32>,
    /// The roles set one by one: (one of [`ROLES`], color).
    pub roles: Vec<(&'static str, u32)>,
    /// What was skipped, for the log: fields this firmware does not know
    /// and colors that are not `#RRGGBB`.
    pub skipped: Vec<String>,
}

/// `#RRGGBB` as `0x00RRGGBB`.
pub fn parse_color(s: &str) -> Option<u32> {
    let hex = s.trim().strip_prefix('#')?;
    (hex.len() == 6 && hex.bytes().all(|c| c.is_ascii_hexdigit())).then(|| u32::from_str_radix(hex, 16).ok())?
}

impl PaletteFile {
    /// Reads a palette file.  Fails only when the text is not protobuf
    /// text format (or gives a field the wrong kind of value); unknown
    /// fields and malformed colors are skipped and listed in `skipped`.
    pub fn parse(text: &str) -> Result<PaletteFile, String> {
        if text.len() > MAX_PALETTE_FILE {
            return Err(format!("{} bytes, and {MAX_PALETTE_FILE} are allowed", text.len()));
        }
        let mut msg = PaletteMessage::default();
        msg.merge_text(&mut TextDecoder::new(text)).map_err(|e| e.to_string())?;
        let mut out = PaletteFile { light: msg.light.unwrap_or(false), ..PaletteFile::default() };
        // The top-level field names, for the ones this firmware skips.
        let mut dec = TextDecoder::new(text);
        while let Some(name) = dec.read_field_name().map_err(|e| e.to_string())? {
            if !ROLES.contains(&name) && !["light", "primary", "surface"].contains(&name) {
                out.skipped.push(format!("unknown field {name:?}"));
            }
            dec.skip_value().map_err(|e| e.to_string())?;
        }
        let mut color = |name: &str, value: &Option<String>| -> Option<u32> {
            let value = value.as_deref()?;
            let c = parse_color(value);
            if c.is_none() {
                out.skipped.push(format!("{name}: {value:?} is not a color (\"#RRGGBB\")"));
            }
            c
        };
        let primary = color("primary", &msg.primary);
        let surface = color("surface", &msg.surface);
        let roles = [
            ("bg", color("bg", &msg.bg)),
            ("fg", color("fg", &msg.fg)),
            ("title_bg", color("title_bg", &msg.title_bg)),
            ("title_fg", color("title_fg", &msg.title_fg)),
            ("sel_bg", color("sel_bg", &msg.sel_bg)),
            ("sel_fg", color("sel_fg", &msg.sel_fg)),
            ("secondary", color("secondary", &msg.secondary)),
            ("border", color("border", &msg.border)),
            ("modal_bg", color("modal_bg", &msg.modal_bg)),
            ("sk_bg", color("sk_bg", &msg.sk_bg)),
            ("sk_edge", color("sk_edge", &msg.sk_edge)),
            ("sk_fg", color("sk_fg", &msg.sk_fg)),
            ("sk_go", color("sk_go", &msg.sk_go)),
            ("sk_stop", color("sk_stop", &msg.sk_stop)),
            ("good", color("good", &msg.good)),
            ("warn", color("warn", &msg.warn)),
            ("error", color("error", &msg.error)),
            ("accent", color("accent", &msg.accent)),
            ("card", color("card", &msg.card)),
            ("card_ringing", color("card_ringing", &msg.card_ringing)),
            ("card_held", color("card_held", &msg.card_held)),
            ("card_dark", color("card_dark", &msg.card_dark)),
            ("tab_bg", color("tab_bg", &msg.tab_bg)),
            ("tab_fg", color("tab_fg", &msg.tab_fg)),
            ("load_track", color("load_track", &msg.load_track)),
            ("load_fill", color("load_fill", &msg.load_fill)),
            ("wallpaper_fg", color("wallpaper_fg", &msg.wallpaper_fg)),
        ];
        out.primary = primary;
        out.surface = surface;
        out.roles = roles.into_iter().filter_map(|(name, c)| Some((name, c?))).collect();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        assert_eq!(parse_color("#1F6E4E"), Some(0x1F6E4E));
        assert_eq!(parse_color(" #a0b0c0 "), Some(0xA0B0C0));
        for bad in ["", "1F6E4E", "#1F6E4", "#1F6E4E0", "#GGGGGG", "#+12345", "red"] {
            assert_eq!(parse_color(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_complete_file() {
        // The example of `palette.proto`.
        let f = PaletteFile::parse(
            "# Palette for lobby.png\nprimary: \"#1F6E4E\"\nsurface: \"#0F1E18\"\ntitle_bg: \"#14503A\"\naccent: \"#8FD6B4\"\n",
        )
        .unwrap();
        assert_eq!(
            f,
            PaletteFile {
                light: false,
                primary: Some(0x1F6E4E),
                surface: Some(0x0F1E18),
                roles: vec![("title_bg", 0x14503A), ("accent", 0x8FD6B4)],
                skipped: vec![],
            }
        );
    }

    #[test]
    fn every_role_is_a_field() {
        let text: String = ROLES.iter().enumerate().map(|(i, r)| format!("{r}: \"#0000{i:02X}\"\n")).collect();
        let f = PaletteFile::parse(&text).unwrap();
        assert_eq!(f.roles, ROLES.iter().enumerate().map(|(i, r)| (*r, i as u32)).collect::<Vec<_>>());
        assert!(f.skipped.is_empty() && f.primary.is_none() && f.surface.is_none() && !f.light);
    }

    #[test]
    fn partial_and_empty_files() {
        assert_eq!(PaletteFile::parse("").unwrap(), PaletteFile::default());
        assert_eq!(PaletteFile::parse("# nothing\n").unwrap(), PaletteFile::default());
        let f = PaletteFile::parse("light: true\nsurface: \"#ECE9E4\"\n").unwrap();
        assert_eq!((f.light, f.primary, f.surface, f.roles.len()), (true, None, Some(0xECE9E4), 0));
    }

    #[test]
    fn unknown_roles_and_bad_colors_are_skipped() {
        let f = PaletteFile::parse(
            "bg: \"#101010\"\nsparkle: \"#FFFFFF\"\nfg: \"white\"\nfuture { a: 1 }\nprimary: \"#12345\"\nsel_bg: \"#203040\"\n",
        )
        .unwrap();
        assert_eq!(f.roles, vec![("bg", 0x101010), ("sel_bg", 0x203040)]);
        assert_eq!(f.primary, None);
        assert_eq!(
            f.skipped,
            vec![
                "unknown field \"sparkle\"".to_string(),
                "unknown field \"future\"".to_string(),
                "primary: \"#12345\" is not a color (\"#RRGGBB\")".to_string(),
                "fg: \"white\" is not a color (\"#RRGGBB\")".to_string(),
            ]
        );
    }

    #[test]
    fn malformed_files_are_refused() {
        for bad in ["bg: \"#101010", "bg = \"#101010\"", "<palette/>", "bg: 5", "light: \"yes\"", "\u{0}\u{1}PNG"] {
            assert!(PaletteFile::parse(bad).is_err(), "{bad:?}");
        }
        assert!(PaletteFile::parse(&"# x\n".repeat(MAX_PALETTE_FILE)).is_err());
    }
}
