//! The app manifest, and the app file that carries it.
//!
//! An app is one WebAssembly component.  Its manifest (the stock fields
//! `name`, `id`, `displayName`, `description`, `type`, `hasIdle`, `hidden`,
//! and the icon, plus a memory limit and the app's data files) is the
//! protobuf message `dphone.app.v1.Manifest` in the custom section
//! `dphone-app` of the app's module, which the macro `export_app!` of
//! `dphone-app-sdk` writes (the format is `proto/manifest.proto` of that
//! crate).  This module finds the section in a module or a component and
//! reads it, without compiling anything.
//!
//! The phone keeps a record of each app in the app's directory
//! ([`RECORD`]): the manifest without the files' contents, with the
//! interfaces the component imports ([`Capability`]), which are not the
//! app's to declare.
//!
//! A stock app package (a ZIP archive with `app.json` and JavaScript) is
//! recognized only to say that it cannot run.

use std::fmt;

/// The custom section that holds the manifest.
pub const SECTION: &str = "dphone-app";
/// The manifest format this code reads.
pub const FORMAT: u32 = 1;
/// The phone's record of an app, in the app's directory.
pub const RECORD: &str = "app.pb";
/// Most data files an app may carry.
pub const MAX_FILES: usize = 64;
/// Most bytes in all of an app's data files.
pub const MAX_FILE_BYTES: u64 = 8 << 20;

/// The data files of an app: (file name, contents).
pub type Files = Vec<(String, Vec<u8>)>;

/// A host interface an app may be granted.  `runtime` is always granted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Capability {
    Ui,
    Timers,
    Storage,
    Config,
    Pbx,
    Http,
    Calls,
    Presence,
    Events,
    Audio,
    L10n,
    Canvas,
    /// Built-in apps only.
    Hotdesk,
    /// Built-in apps only.
    Diagnostics,
}

impl Capability {
    pub const ALL: [Capability; 14] = [
        Capability::Ui,
        Capability::Timers,
        Capability::Storage,
        Capability::Config,
        Capability::Pbx,
        Capability::Http,
        Capability::Calls,
        Capability::Presence,
        Capability::Events,
        Capability::Audio,
        Capability::L10n,
        Capability::Canvas,
        Capability::Hotdesk,
        Capability::Diagnostics,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Capability::Ui => "ui",
            Capability::Timers => "timers",
            Capability::Storage => "storage",
            Capability::Config => "config",
            Capability::Pbx => "pbx",
            Capability::Http => "http",
            Capability::Calls => "calls",
            Capability::Presence => "presence",
            Capability::Events => "events",
            Capability::Audio => "audio",
            Capability::L10n => "l10n",
            Capability::Canvas => "canvas",
            Capability::Hotdesk => "hotdesk",
            Capability::Diagnostics => "diagnostics",
        }
    }

    pub fn from_name(s: &str) -> Option<Capability> {
        Capability::ALL.into_iter().find(|c| c.name() == s)
    }

    /// The capability an import of a component stands for: `Ok(None)` for
    /// the interfaces every app has, an error for something that is not
    /// an interface of the phone.  `import` is the import's name, e.g.,
    /// `dphone:app/ui@3.0.0`.
    pub fn of_import(import: &str) -> Result<Option<Capability>, ManifestError> {
        let unknown = || ManifestError::UnknownImport(import.to_string());
        let interface = import.strip_prefix("dphone:app/").ok_or_else(unknown)?;
        let interface = interface.split('@').next().unwrap_or(interface);
        match interface {
            "runtime" | "types" => Ok(None),
            other => Capability::from_name(other).map(Some).ok_or_else(unknown),
        }
    }

    /// Capabilities an installed (downloaded) app can never have.
    pub fn builtin_only(self) -> bool {
        matches!(self, Capability::Hotdesk | Capability::Diagnostics)
    }
}

/// Stock manifest `type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppType {
    Foreground,
    Background,
    Idle,
}

impl AppType {
    pub fn name(self) -> &'static str {
        match self {
            AppType::Foreground => "foreground",
            AppType::Background => "background",
            AppType::Idle => "idle",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestError {
    /// The file is not WebAssembly.
    NotWasm,
    /// A stock app package: a ZIP archive with `app.json`.
    JavaScript,
    /// The WebAssembly has no manifest section (stock status -101).
    NoManifest,
    /// The section is not a `Manifest` message.
    Malformed(&'static str),
    /// A manifest format this code does not know.
    Format(u32),
    /// Name missing, too long, or malformed (stock status -102).
    BadName(&'static str),
    /// An unknown `kind`.
    BadKind(u64),
    /// A data file with a name the phone does not take.
    BadFile(String),
    /// Too many data files, or too many bytes in them.
    TooLarge,
    /// The component imports something that is not an interface of the
    /// phone.
    UnknownImport(String),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestError::NotWasm => write!(
                f,
                "not an app: an app is a WebAssembly component (a .wasm file made by cargo dphone-app)"
            ),
            ManifestError::JavaScript => write!(
                f,
                "this is a package of a stock JavaScript app; JavaScript apps cannot run on this firmware"
            ),
            ManifestError::NoManifest => write!(
                f,
                "the WebAssembly file has no app manifest (the section \"{SECTION}\", which export_app! of dphone-app-sdk writes)"
            ),
            ManifestError::Malformed(why) => write!(f, "unreadable app manifest: {why}"),
            ManifestError::Format(v) => write!(
                f,
                "the app's manifest has format {v}, and this firmware knows format {FORMAT}: the app was made for another firmware"
            ),
            ManifestError::BadName(why) => write!(f, "bad app name: {why}"),
            ManifestError::BadKind(k) => write!(f, "bad app kind {k}"),
            ManifestError::BadFile(n) => write!(f, "bad data file name {n:?}"),
            ManifestError::TooLarge => write!(
                f,
                "the app carries too much: at most {MAX_FILES} data files and {MAX_FILE_BYTES} bytes in them"
            ),
            ManifestError::UnknownImport(i) => write!(
                f,
                "the app imports {i:?}, which is not an interface of this firmware"
            ),
        }
    }
}

impl std::error::Error for ManifestError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub id: String,
    /// The display name for `en_us`, or else the unlocalized one; the app
    /// name if there is none.
    pub display_name: String,
    pub description: String,
    /// The display names and descriptions as the app gives them.
    pub display_names: Text,
    pub descriptions: Text,
    pub kind: AppType,
    pub has_idle: bool,
    pub hidden: bool,
    /// The icon: one of the phone's, or a data file.
    pub icon: String,
    /// The component's file name in the app's directory: `<name>.wasm`.
    pub wasm: String,
    /// The interfaces the component imports.  Not part of an app's own
    /// manifest: empty until the phone (or the build, for a built-in app)
    /// has looked at the component.
    pub capabilities: Vec<Capability>,
    /// Requested linear-memory cap in KiB (0: the default); never above
    /// the manager's hard cap.
    pub memory_kib: u32,
    /// The names of the app's data files.
    pub files: Vec<String>,
}

/// A text with its translations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Text {
    pub text: String,
    /// (locale, text).
    pub localized: Vec<(String, String)>,
}

impl Text {
    /// As stock chooses: `en_us`, or else the unlocalized text.
    fn shown(&self) -> &str {
        self.localized
            .iter()
            .find(|(l, _)| l.eq_ignore_ascii_case("en_us"))
            .map(|(_, t)| t.as_str())
            .unwrap_or(&self.text)
    }
}

/// Stock name rule: 1 to 64 characters, first a letter or digit, then
/// letters, digits, `_`, `.`, or `,`.
pub fn validate_name(name: &str) -> Result<(), ManifestError> {
    if name.is_empty() {
        return Err(ManifestError::BadName("empty"));
    }
    if name.len() > 64 {
        return Err(ManifestError::BadName("longer than 64 characters"));
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphanumeric() {
        return Err(ManifestError::BadName("must start with a letter or digit"));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ',')) {
        return Err(ManifestError::BadName("invalid character"));
    }
    Ok(())
}

fn validate_file_name(name: &str, app: &str) -> Result<(), ManifestError> {
    let bad = name.is_empty()
        || name.len() > 255
        || name.starts_with('.')
        || name.contains(['/', '\\', '\0'])
        // The phone's own files in an app's directory.
        || name == RECORD
        || name.ends_with(".cwasm")
        || name == format!("{app}.wasm");
    if bad {
        Err(ManifestError::BadFile(name.to_string()))
    } else {
        Ok(())
    }
}

// ------------------------------------------------------------- protobuf

/// The value of one field of a protobuf message.
enum Field<'a> {
    Number(u64),
    Bytes(&'a [u8]),
}

const CUT: ManifestError = ManifestError::Malformed("cut short");

fn varint(b: &[u8], at: &mut usize) -> Result<u64, ManifestError> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *b.get(*at).ok_or(CUT)?;
        *at += 1;
        v |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(v);
        }
    }
    Err(ManifestError::Malformed("a number too long"))
}

/// The `len` bytes at `at`, which moves past them.
fn take<'a>(b: &'a [u8], at: &mut usize, len: u64) -> Result<&'a [u8], ManifestError> {
    let end = usize::try_from(len)
        .ok()
        .and_then(|l| at.checked_add(l))
        .filter(|e| *e <= b.len())
        .ok_or(CUT)?;
    let bytes = &b[*at..end];
    *at = end;
    Ok(bytes)
}

/// Calls `f` with every field of the message `b`: (field number, value).
fn fields<'a>(
    b: &'a [u8],
    mut f: impl FnMut(u64, Field<'a>) -> Result<(), ManifestError>,
) -> Result<(), ManifestError> {
    let mut at = 0;
    while at < b.len() {
        let key = varint(b, &mut at)?;
        let value = match key & 7 {
            0 => Field::Number(varint(b, &mut at)?),
            2 => {
                let len = varint(b, &mut at)?;
                Field::Bytes(take(b, &mut at, len)?)
            }
            // Fixed-width fields: the format has none; skipped.
            1 => {
                take(b, &mut at, 8)?;
                continue;
            }
            5 => {
                take(b, &mut at, 4)?;
                continue;
            }
            _ => return Err(ManifestError::Malformed("an unknown wire type")),
        };
        f(key >> 3, value)?;
    }
    Ok(())
}

fn string(v: Field<'_>) -> Result<String, ManifestError> {
    match v {
        Field::Bytes(b) => std::str::from_utf8(b)
            .map(str::to_string)
            .map_err(|_| ManifestError::Malformed("text that is not UTF-8")),
        Field::Number(_) => Err(ManifestError::Malformed("a number where text belongs")),
    }
}

fn number(v: Field<'_>) -> Result<u64, ManifestError> {
    match v {
        Field::Number(n) => Ok(n),
        Field::Bytes(_) => Err(ManifestError::Malformed("bytes where a number belongs")),
    }
}

fn message<'a>(v: Field<'a>) -> Result<&'a [u8], ManifestError> {
    match v {
        Field::Bytes(b) => Ok(b),
        Field::Number(_) => Err(ManifestError::Malformed("a number where a message belongs")),
    }
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 & 0x7f | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn put_bytes(out: &mut Vec<u8>, field: u64, b: &[u8]) {
    put_varint(out, field << 3 | 2);
    put_varint(out, b.len() as u64);
    out.extend_from_slice(b);
}

fn put_string(out: &mut Vec<u8>, field: u64, s: &str) {
    if !s.is_empty() {
        put_bytes(out, field, s.as_bytes());
    }
}

fn put_number(out: &mut Vec<u8>, field: u64, v: u64) {
    if v != 0 {
        put_varint(out, field << 3);
        put_varint(out, v);
    }
}

fn put_text(out: &mut Vec<u8>, field: u64, t: &Text) {
    let mut m = Vec::new();
    put_string(&mut m, 1, &t.text);
    for (locale, text) in &t.localized {
        let mut l = Vec::new();
        put_string(&mut l, 1, locale);
        put_string(&mut l, 2, text);
        put_bytes(&mut m, 2, &l);
    }
    if !m.is_empty() {
        put_bytes(out, field, &m);
    }
}

fn read_text(b: &[u8], into: &mut Text) -> Result<(), ManifestError> {
    fields(b, |n, v| {
        match n {
            1 => into.text = string(v)?,
            2 => {
                let mut pair = (String::new(), String::new());
                fields(message(v)?, |n, v| {
                    match n {
                        1 => pair.0 = string(v)?,
                        2 => pair.1 = string(v)?,
                        _ => {}
                    }
                    Ok(())
                })?;
                into.localized.push(pair);
            }
            _ => {}
        }
        Ok(())
    })
}

// ------------------------------------------------------------ WebAssembly

const WASM_MAGIC: &[u8] = b"\0asm";
/// The version and layer of a core module, and of a component.
const MODULE: &[u8] = &[1, 0, 0, 0];
const COMPONENT: &[u8] = &[0x0d, 0, 1, 0];

/// Appends the contents of every manifest section in `wasm` (a module, or
/// a component with the modules in it) to `found`.
fn sections(wasm: &[u8], depth: u32, found: &mut Vec<u8>) -> Result<(), ManifestError> {
    if wasm.len() < 8 || &wasm[..4] != WASM_MAGIC {
        return Err(ManifestError::NotWasm);
    }
    let component = match &wasm[4..8] {
        v if v == MODULE => false,
        v if v == COMPONENT => true,
        _ => return Err(ManifestError::NotWasm),
    };
    let cut = |_| ManifestError::Malformed("WebAssembly that is cut short");
    let mut at = 8;
    while at < wasm.len() {
        let id = wasm[at];
        at += 1;
        let len = varint(wasm, &mut at).map_err(cut)?;
        let body = take(wasm, &mut at, len).map_err(cut)?;
        match id {
            // A custom section: a name, then the contents.
            0 => {
                let mut p = 0;
                let n = varint(body, &mut p).map_err(cut)?;
                if take(body, &mut p, n).map_err(cut)? == SECTION.as_bytes() {
                    found.extend_from_slice(&body[p..]);
                }
            }
            // In a component: a core module (1) or a component (4).
            1 | 4 if component && depth < 4 => sections(body, depth + 1, found)?,
            _ => {}
        }
    }
    Ok(())
}

/// True for a ZIP archive that holds an `app.json`: a stock app package.
fn is_stock_package(file: &[u8]) -> bool {
    file.starts_with(b"PK") && file.windows(8).any(|w| w == b"app.json")
}

impl Manifest {
    /// The manifest and the data files of an app file (a component, or the
    /// module it is made of).
    pub fn from_wasm(wasm: &[u8]) -> Result<(Manifest, Files), ManifestError> {
        let mut found = Vec::new();
        match sections(wasm, 0, &mut found) {
            Err(ManifestError::NotWasm) if is_stock_package(wasm) => {
                return Err(ManifestError::JavaScript);
            }
            other => other?,
        }
        if found.is_empty() {
            return Err(ManifestError::NoManifest);
        }
        Manifest::decode(&found, false)
    }

    /// The phone's record of an app ([`RECORD`]).
    pub fn from_record(record: &[u8]) -> Result<Manifest, ManifestError> {
        Manifest::decode(record, true).map(|(m, _)| m)
    }

    fn decode(bytes: &[u8], record: bool) -> Result<(Manifest, Files), ManifestError> {
        let mut m = Manifest {
            name: String::new(),
            id: String::new(),
            display_name: String::new(),
            description: String::new(),
            display_names: Text::default(),
            descriptions: Text::default(),
            kind: AppType::Foreground,
            has_idle: false,
            hidden: false,
            icon: String::new(),
            wasm: String::new(),
            capabilities: Vec::new(),
            memory_kib: 0,
            files: Vec::new(),
        };
        let mut format = 0;
        let mut files = Files::new();
        fields(bytes, |n, v| {
            match n {
                1 => format = number(v)?,
                2 => m.name = string(v)?,
                3 => m.id = string(v)?,
                4 => read_text(message(v)?, &mut m.display_names)?,
                5 => read_text(message(v)?, &mut m.descriptions)?,
                6 => {
                    m.kind = match number(v)? {
                        0 | 1 => AppType::Foreground,
                        2 => AppType::Background,
                        3 => AppType::Idle,
                        other => return Err(ManifestError::BadKind(other)),
                    }
                }
                7 => m.has_idle = number(v)? != 0,
                8 => m.hidden = number(v)? != 0,
                9 => m.icon = string(v)?,
                10 => m.memory_kib = number(v)?.min(u64::from(u32::MAX)) as u32,
                11 => {
                    let mut file = (String::new(), Vec::new());
                    fields(message(v)?, |n, v| {
                        match n {
                            1 => file.0 = string(v)?,
                            2 => file.1 = message(v)?.to_vec(),
                            _ => {}
                        }
                        Ok(())
                    })?;
                    files.push(file);
                }
                // Only the phone's own record says what the app may use.
                15 if record => {
                    let name = string(v)?;
                    if let Some(c) = Capability::from_name(&name)
                        && !m.capabilities.contains(&c)
                    {
                        m.capabilities.push(c);
                    }
                }
                _ => {}
            }
            Ok(())
        })?;
        if format != u64::from(FORMAT) {
            return Err(ManifestError::Format(format.min(u64::from(u32::MAX)) as u32));
        }
        validate_name(&m.name)?;
        if files.len() > MAX_FILES
            || files.iter().map(|(_, d)| d.len() as u64).sum::<u64>() > MAX_FILE_BYTES
        {
            return Err(ManifestError::TooLarge);
        }
        for (i, (name, _)) in files.iter().enumerate() {
            validate_file_name(name, &m.name)?;
            if files[..i].iter().any(|(n, _)| n == name) {
                return Err(ManifestError::BadFile(name.clone()));
            }
        }
        m.files = files.iter().map(|(n, _)| n.clone()).collect();
        m.has_idle |= m.kind == AppType::Idle;
        m.display_name = match m.display_names.shown() {
            "" => m.name.clone(),
            s => s.to_string(),
        };
        m.description = m.descriptions.shown().to_string();
        m.wasm = format!("{}.wasm", m.name);
        Ok((m, files))
    }

    /// The phone's record of the app: the manifest with the capabilities
    /// and the names of the data files, without their contents.
    pub fn record(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_number(&mut out, 1, u64::from(FORMAT));
        put_string(&mut out, 2, &self.name);
        put_string(&mut out, 3, &self.id);
        put_text(&mut out, 4, &self.display_names);
        put_text(&mut out, 5, &self.descriptions);
        put_number(
            &mut out,
            6,
            match self.kind {
                AppType::Foreground => 1,
                AppType::Background => 2,
                AppType::Idle => 3,
            },
        );
        put_number(&mut out, 7, u64::from(self.has_idle));
        put_number(&mut out, 8, u64::from(self.hidden));
        put_string(&mut out, 9, &self.icon);
        put_number(&mut out, 10, u64::from(self.memory_kib));
        for name in &self.files {
            let mut f = Vec::new();
            put_string(&mut f, 1, name);
            put_bytes(&mut out, 11, &f);
        }
        for c in &self.capabilities {
            put_string(&mut out, 15, c.name());
        }
        out
    }

    /// A manifest section with these fields and these data files: what
    /// `export_app!` writes.  For tools and tests that make one app file
    /// from another.
    pub fn section(&self, files: &[(String, Vec<u8>)]) -> Vec<u8> {
        let mut plain = self.clone();
        plain.capabilities.clear();
        plain.files.clear();
        let mut out = plain.record();
        for (name, data) in files {
            let mut f = Vec::new();
            put_string(&mut f, 1, name);
            put_bytes(&mut f, 2, data);
            put_bytes(&mut out, 11, &f);
        }
        out
    }

    pub fn has(&self, cap: Capability) -> bool {
        self.capabilities.contains(&cap)
    }
}

/// `wasm` (a module or a component) with the contents of its manifest
/// section replaced by `section`.  For tools and tests that make one app
/// file from another.
pub fn with_section(wasm: &[u8], section: &[u8]) -> Result<Vec<u8>, ManifestError> {
    if wasm.len() < 8 || &wasm[..4] != WASM_MAGIC {
        return Err(ManifestError::NotWasm);
    }
    let component = &wasm[4..8] == COMPONENT;
    let mut out = wasm[..8].to_vec();
    let mut at = 8;
    while at < wasm.len() {
        let id = wasm[at];
        at += 1;
        let len = varint(wasm, &mut at)?;
        let body = take(wasm, &mut at, len)?;
        let mut p = 0;
        let new_body = match id {
            0 => {
                let n = varint(body, &mut p)?;
                if take(body, &mut p, n)? == SECTION.as_bytes() {
                    let mut b = body[..p].to_vec();
                    b.extend_from_slice(section);
                    b
                } else {
                    body.to_vec()
                }
            }
            1 | 4 if component => with_section(body, section)?,
            _ => body.to_vec(),
        };
        out.push(id);
        put_varint(&mut out, new_body.len() as u64);
        out.extend(new_body);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_stock_rule() {
        assert!(validate_name("voicemail").is_ok());
        assert!(validate_name("a_b.c,d9").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("my-app").is_err());
        assert!(validate_name("_x").is_err());
        assert!(validate_name(&"a".repeat(65)).is_err());
        assert!(validate_name("a/b").is_err());
    }

    fn module_with(section: &[u8]) -> Vec<u8> {
        let mut body = vec![SECTION.len() as u8];
        body.extend_from_slice(SECTION.as_bytes());
        body.extend_from_slice(section);
        let mut m = b"\0asm\x01\0\0\0".to_vec();
        m.push(0);
        put_varint(&mut m, body.len() as u64);
        m.extend(body);
        m
    }

    fn sample() -> Manifest {
        let (m, _) = Manifest::decode(&[8, 1, 0x12, 2, b'v', b'm'], false).unwrap();
        m
    }

    #[test]
    fn a_record_round_trips_and_only_a_record_carries_capabilities() {
        let mut m = sample();
        m.display_names = Text {
            text: "Voicemail".into(),
            localized: vec![("de_de".into(), "VM".into())],
        };
        m.kind = AppType::Idle;
        m.icon = "i.png".into();
        m.capabilities = vec![Capability::Ui, Capability::Pbx];
        m.files = vec!["i.png".into()];
        let back = Manifest::from_record(&m.record()).unwrap();
        assert_eq!(back.display_name, "Voicemail");
        assert!(back.has_idle);
        assert!(back.has(Capability::Pbx) && !back.has(Capability::Calls));
        assert_eq!(back.files, ["i.png"]);
        // The same bytes as an app's own section: the capabilities are not
        // the app's to declare.
        let (own, _) = Manifest::from_wasm(&module_with(&m.record())).unwrap();
        assert!(own.capabilities.is_empty());
    }

    #[test]
    fn the_section_is_found_and_replaced_in_a_component_too() {
        let m = sample();
        let files = vec![("pic.png".to_string(), vec![1, 2, 3])];
        let module = module_with(&m.section(&files));
        let mut component = b"\0asm\x0d\0\x01\0".to_vec();
        component.push(1);
        put_varint(&mut component, module.len() as u64);
        component.extend(&module);
        let (found, data) = Manifest::from_wasm(&component).unwrap();
        assert_eq!(found.name, "vm");
        assert_eq!(found.wasm, "vm.wasm");
        assert_eq!(data, files);

        let mut other = found.clone();
        other.name = "other".into();
        let renamed = with_section(&component, &other.section(&[])).unwrap();
        let (found, data) = Manifest::from_wasm(&renamed).unwrap();
        assert_eq!((found.name.as_str(), data.len()), ("other", 0));
    }

    #[test]
    fn what_is_not_an_app_is_told_apart() {
        assert_eq!(Manifest::from_wasm(b"hello"), Err(ManifestError::NotWasm));
        assert_eq!(
            Manifest::from_wasm(b"PK\x03\x04....demo/app.json...."),
            Err(ManifestError::JavaScript)
        );
        assert_eq!(
            Manifest::from_wasm(b"\0asm\x01\0\0\0"),
            Err(ManifestError::NoManifest)
        );
        assert_eq!(
            Manifest::from_wasm(&module_with(&[8, 2, 0x12, 1, b'x'])),
            Err(ManifestError::Format(2))
        );
        assert!(matches!(
            Manifest::from_wasm(&module_with(&[8, 1, 0x12, 1, b'-'])),
            Err(ManifestError::BadName(_))
        ));
        let m = sample();
        for bad in ["../x", ".hidden", "app.pb", "vm.wasm", "x.cwasm", ""] {
            let files = vec![(bad.to_string(), vec![])];
            assert!(
                matches!(
                    Manifest::from_wasm(&module_with(&m.section(&files))),
                    Err(ManifestError::BadFile(_))
                ),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn imports_name_capabilities() {
        assert_eq!(
            Capability::of_import("dphone:app/ui@3.0.0"),
            Ok(Some(Capability::Ui))
        );
        assert_eq!(Capability::of_import("dphone:app/runtime@3.0.0"), Ok(None));
        assert!(Capability::of_import("wasi:io/streams@0.2.0").is_err());
        assert!(Capability::of_import("dphone:app/root@3.0.0").is_err());
    }
}
