//! The app's manifest: what the phone has to know about an app before it
//! runs it.  It is written in Rust, in [`export_app!`](crate::export_app),
//! and travels inside the app's WebAssembly file, in the custom section
//! `dphone-app`, as the protobuf message `dphone.app.v1.Manifest`
//! (`proto/manifest.proto` in this crate).
//!
//! The section is made when the app is compiled, so everything here is
//! `const`: a mistake in the manifest is a compile error.

/// The custom section that holds the manifest.
pub const SECTION: &str = "dphone-app";

/// The version of the manifest format this crate writes.
pub const FORMAT: u32 = 1;

/// When an app runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppKind {
    /// While it is on screen (the default).
    Foreground,
    /// From the phone's start, without being on screen.
    Background,
    /// As the idle screen.
    Idle,
}

/// The fields of [`export_app!`](crate::export_app), after `app`.  Only
/// `name` and `display_name` must be given.
#[derive(Clone, Copy, Debug)]
pub struct Manifest {
    /// The app's name on the phone: 1 to 64 characters, the first a letter
    /// or digit, the rest letters, digits, `_`, `.`, or `,`.  It cannot be
    /// the name of a built-in app.
    pub name: &'static str,
    /// An identifier of the app, e.g., in reverse domain form.
    pub id: &'static str,
    /// What the phone calls the app on screen.
    pub display_name: &'static str,
    /// The same in other languages: (locale, text), e.g.,
    /// `&[("de_de", "Kalender")]`.
    pub display_name_in: &'static [(&'static str, &'static str)],
    /// A line about the app.
    pub description: &'static str,
    /// The same in other languages.
    pub description_in: &'static [(&'static str, &'static str)],
    pub kind: AppKind,
    /// The app draws on the idle screen.
    pub has_idle: bool,
    /// The app is marked as hidden.
    pub hidden: bool,
    /// The app's icon: the content of a PNG file, e.g.,
    /// `include_bytes!("hello.png")`.  At most 256 KiB.
    pub icon: &'static [u8],
    /// Icons for particular models of phone, each a PNG file's content:
    /// (model, icon), e.g., `&[("D60", include_bytes!("color.png"))]`.  A
    /// phone shows its model's icon if there is one, else `icon`.  The
    /// models are D40, D45, D50, D60, D62, D65, and D70.
    pub icon_for: &'static [(&'static str, &'static [u8])],
    /// The name of one of the phone's icons, for an app without an icon.
    pub phone_icon: &'static str,
    /// The most memory the app may have, in KiB.  The default (0) is
    /// 4 MiB, and the phone gives no app more than 16 MiB.
    pub memory_kib: u32,
}

/// The largest icon.
pub const MAX_ICON_BYTES: usize = 256 << 10;

/// The models of phone, as `icon_for` names them.
pub const MODELS: [&str; 7] = ["D40", "D45", "D50", "D60", "D62", "D65", "D70"];

const fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

const fn model_ok(model: &str) -> bool {
    let mut i = 0;
    while i < MODELS.len() {
        if same(MODELS[i], model) {
            return true;
        }
        i += 1;
    }
    false
}

const fn png_ok(icon: &[u8]) -> bool {
    icon.len() > 8 && icon[0] == 0x89 && icon[1] == b'P' && icon[2] == b'N' && icon[3] == b'G'
}

const fn model_icon_len(i: &(&str, &[u8])) -> usize {
    string_len(i.0) + delimited_len(i.1.len())
}

const fn varint_len(mut v: u64) -> usize {
    let mut n = 1;
    while v >= 0x80 {
        v >>= 7;
        n += 1;
    }
    n
}

/// A length-delimited field of `len` bytes, with its one-byte tag.
const fn delimited_len(len: usize) -> usize {
    1 + varint_len(len as u64) + len
}

const fn string_len(s: &str) -> usize {
    if s.is_empty() {
        0
    } else {
        delimited_len(s.len())
    }
}

const fn localized_len(l: &(&str, &str)) -> usize {
    string_len(l.0) + string_len(l.1)
}

const fn text_len(text: &str, localized: &[(&str, &str)]) -> usize {
    let mut n = string_len(text);
    let mut i = 0;
    while i < localized.len() {
        n += delimited_len(localized_len(&localized[i]));
        i += 1;
    }
    n
}

/// A cursor over the section being written.
struct Out<const N: usize> {
    bytes: [u8; N],
    at: usize,
}

impl<const N: usize> Out<N> {
    const fn varint(mut self, mut v: u64) -> Self {
        while v >= 0x80 {
            self.bytes[self.at] = (v as u8 & 0x7f) | 0x80;
            self.at += 1;
            v >>= 7;
        }
        self.bytes[self.at] = v as u8;
        self.at += 1;
        self
    }

    /// The tag of a length-delimited field and its length.
    const fn open(self, field: u32, len: usize) -> Self {
        self.varint((field as u64) << 3 | 2).varint(len as u64)
    }

    const fn bytes(mut self, field: u32, data: &[u8]) -> Self {
        self = self.open(field, data.len());
        let (_, rest) = self.bytes.split_at_mut(self.at);
        let (dest, _) = rest.split_at_mut(data.len());
        dest.copy_from_slice(data);
        self.at += data.len();
        self
    }

    const fn string(self, field: u32, s: &str) -> Self {
        if s.is_empty() {
            self
        } else {
            self.bytes(field, s.as_bytes())
        }
    }

    const fn number(self, field: u32, v: u64) -> Self {
        if v == 0 {
            self
        } else {
            self.varint((field as u64) << 3).varint(v)
        }
    }

    const fn text(mut self, field: u32, text: &str, localized: &[(&str, &str)]) -> Self {
        let len = text_len(text, localized);
        if len == 0 {
            return self;
        }
        self = self.open(field, len).string(1, text);
        let mut i = 0;
        while i < localized.len() {
            let l = &localized[i];
            self = self.open(2, localized_len(l)).string(1, l.0).string(2, l.1);
            i += 1;
        }
        self
    }
}

const fn name_ok(name: &str) -> bool {
    let b = name.as_bytes();
    if b.is_empty() || b.len() > 64 || !b[0].is_ascii_alphanumeric() {
        return false;
    }
    let mut i = 1;
    while i < b.len() {
        if !(b[i].is_ascii_alphanumeric() || matches!(b[i], b'_' | b'.' | b',')) {
            return false;
        }
        i += 1;
    }
    true
}

const fn locales_ok(localized: &[(&str, &str)]) -> bool {
    let mut i = 0;
    while i < localized.len() {
        if localized[i].0.is_empty() || localized[i].1.is_empty() {
            return false;
        }
        i += 1;
    }
    true
}

impl Manifest {
    /// Every field at its default; `name` and `display_name` are missing.
    pub const EMPTY: Manifest = Manifest {
        name: "",
        id: "",
        display_name: "",
        display_name_in: &[],
        description: "",
        description_in: &[],
        kind: AppKind::Foreground,
        has_idle: false,
        hidden: false,
        icon: &[],
        icon_for: &[],
        phone_icon: "",
        memory_kib: 0,
    };

    /// The manifest, if the phone will take it; otherwise the compilation
    /// fails with what is wrong.
    pub const fn checked(self) -> Manifest {
        assert!(!self.name.is_empty(), "export_app!: `name` is missing");
        assert!(
            name_ok(self.name),
            "export_app!: `name` must be 1 to 64 characters, the first a letter or digit, the rest letters, digits, '_', '.', or ','"
        );
        assert!(
            !self.display_name.is_empty(),
            "export_app!: `display_name` is missing"
        );
        assert!(
            locales_ok(self.display_name_in) && locales_ok(self.description_in),
            "export_app!: a localized text needs a locale and a text, e.g., (\"de_de\", \"Kalender\")"
        );
        assert!(
            self.memory_kib <= 16 << 10,
            "export_app!: `memory_kib` is more than the phone gives any app (16384)"
        );
        assert!(
            self.icon.len() <= MAX_ICON_BYTES,
            "export_app!: `icon` is larger than 256 KiB"
        );
        assert!(
            self.icon.is_empty() || png_ok(self.icon),
            "export_app!: `icon` is not a PNG file's content (for one of the phone's icons, give its name as `phone_icon`)"
        );
        let mut i = 0;
        while i < self.icon_for.len() {
            let (model, icon) = self.icon_for[i];
            assert!(
                model_ok(model),
                "export_app!: a model in `icon_for` is not one of D40, D45, D50, D60, D62, D65, D70"
            );
            assert!(
                icon.len() <= MAX_ICON_BYTES && png_ok(icon),
                "export_app!: an icon in `icon_for` is not a PNG file's content of at most 256 KiB"
            );
            let mut j = 0;
            while j < i {
                assert!(
                    !same(self.icon_for[j].0, model),
                    "export_app!: `icon_for` names a model twice"
                );
                j += 1;
            }
            i += 1;
        }
        self
    }

    /// The size of the manifest's section.
    pub const fn encoded_len(&self) -> usize {
        let mut n = 1 + varint_len(FORMAT as u64);
        n += string_len(self.name) + string_len(self.id);
        let t = text_len(self.display_name, self.display_name_in);
        if t > 0 {
            n += delimited_len(t);
        }
        let t = text_len(self.description, self.description_in);
        if t > 0 {
            n += delimited_len(t);
        }
        // `kind` is always written; the flags only when set.
        n += 2;
        n += self.has_idle as usize * 2 + self.hidden as usize * 2;
        n += string_len(self.phone_icon);
        if self.memory_kib > 0 {
            n += 1 + varint_len(self.memory_kib as u64);
        }
        if !self.icon.is_empty() {
            n += delimited_len(self.icon.len());
        }
        let mut i = 0;
        while i < self.icon_for.len() {
            n += delimited_len(model_icon_len(&self.icon_for[i]));
            i += 1;
        }
        n
    }

    /// The manifest's section; `N` is [`Manifest::encoded_len`].
    pub const fn encode<const N: usize>(&self) -> [u8; N] {
        let kind = match self.kind {
            AppKind::Foreground => 1,
            AppKind::Background => 2,
            AppKind::Idle => 3,
        };
        let mut out = Out {
            bytes: [0; N],
            at: 0,
        }
        .number(1, FORMAT as u64)
        .string(2, self.name)
        .string(3, self.id)
        .text(4, self.display_name, self.display_name_in)
        .text(5, self.description, self.description_in)
        .number(6, kind)
        .number(7, self.has_idle as u64)
        .number(8, self.hidden as u64)
        .string(9, self.phone_icon)
        .number(10, self.memory_kib as u64);
        if !self.icon.is_empty() {
            out = out.bytes(11, self.icon);
        }
        let mut i = 0;
        while i < self.icon_for.len() {
            let m = &self.icon_for[i];
            out = out.open(12, model_icon_len(m)).string(1, m.0).bytes(2, m.1);
            i += 1;
        }
        assert!(out.at == N, "the manifest's section has another length");
        out.bytes
    }
}
