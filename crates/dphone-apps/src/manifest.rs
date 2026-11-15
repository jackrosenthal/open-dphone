//! The app manifest (`app.json`).
//!
//! It carries the stock fields (`name`, `id`, `displayName`, `description`,
//! `type`, `hasIdle`, `hidden`, `iconFile`) with the stock parsing rules,
//! plus two of ours: `wasm` (the component file) and `capabilities` (the
//! host interfaces the app may import).  A stock JavaScript manifest
//! (`jsFiles`, no `wasm`) is recognized so it can be skipped with a log
//! entry.

use serde_json::Value;
use std::fmt;

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
    /// Not JSON, or not an object (stock status -101).
    Unreadable(String),
    /// Name missing, too long, or malformed (stock status -102).
    BadName(&'static str),
    /// `type` starts with an unknown letter.
    BadType(String),
    /// A stock JavaScript app: it has `jsFiles` and no `wasm`.
    JavaScript,
    /// No `wasm` entry, or one with a path separator.
    BadWasm,
    /// Unknown capability name.
    BadCapability(String),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestError::Unreadable(e) => write!(f, "unreadable manifest: {e}"),
            ManifestError::BadName(why) => write!(f, "bad app name: {why}"),
            ManifestError::BadType(t) => write!(f, "bad app type {t:?}"),
            ManifestError::JavaScript => write!(f, "JavaScript apps cannot run on this firmware"),
            ManifestError::BadWasm => write!(f, "manifest names no usable wasm file"),
            ManifestError::BadCapability(c) => write!(f, "unknown capability {c:?}"),
        }
    }
}

impl std::error::Error for ManifestError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub id: String,
    /// The `fallback` (or `en_us`) display name; the app name if absent.
    pub display_name: String,
    pub description: String,
    pub kind: AppType,
    pub has_idle: bool,
    pub hidden: bool,
    /// `iconFile`.
    pub icon: String,
    /// Component file name within the package.
    pub wasm: String,
    pub capabilities: Vec<Capability>,
    /// Requested linear-memory cap in KiB (0: the default); never above
    /// the manager's hard cap.
    pub memory_kib: u32,
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

fn localized(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Object(m)) => ["en_us", "en_US", "fallback"]
            .iter()
            .find_map(|k| m.get(*k).and_then(Value::as_str))
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    }
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, ManifestError> {
        let v: Value =
            serde_json::from_str(text).map_err(|e| ManifestError::Unreadable(e.to_string()))?;
        let obj = v
            .as_object()
            .ok_or_else(|| ManifestError::Unreadable("not an object".into()))?;
        let name = obj
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        validate_name(&name)?;

        let type_text = obj.get("type").and_then(Value::as_str).unwrap_or_default();
        let kind = match type_text.chars().next() {
            None | Some('f' | 'F') => AppType::Foreground,
            Some('b' | 'B') => AppType::Background,
            Some('i' | 'I') => AppType::Idle,
            Some(_) => return Err(ManifestError::BadType(type_text.to_string())),
        };
        let has_idle =
            kind == AppType::Idle || obj.get("hasIdle").and_then(Value::as_bool).unwrap_or(false);

        let wasm = match obj.get("wasm").and_then(Value::as_str) {
            Some(w) if !w.is_empty() && !w.contains(['/', '\\']) && w != ".." => w.to_string(),
            Some(_) => return Err(ManifestError::BadWasm),
            None if obj.contains_key("jsFiles") => return Err(ManifestError::JavaScript),
            None => return Err(ManifestError::BadWasm),
        };

        let mut capabilities = Vec::new();
        if let Some(list) = obj.get("capabilities").and_then(Value::as_array) {
            for c in list {
                let s = c.as_str().unwrap_or_default();
                let cap = Capability::from_name(s)
                    .ok_or_else(|| ManifestError::BadCapability(s.to_string()))?;
                if !capabilities.contains(&cap) {
                    capabilities.push(cap);
                }
            }
        }

        let display_name = match localized(obj.get("displayName")) {
            s if s.is_empty() => name.clone(),
            s => s,
        };
        Ok(Manifest {
            id: obj
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            display_name,
            description: localized(obj.get("description")),
            kind,
            has_idle,
            hidden: obj.get("hidden").and_then(Value::as_bool).unwrap_or(false),
            icon: obj
                .get("iconFile")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            wasm,
            capabilities,
            memory_kib: obj.get("memoryKiB").and_then(Value::as_u64).unwrap_or(0) as u32,
            name,
        })
    }

    pub fn has(&self, cap: Capability) -> bool {
        self.capabilities.contains(&cap)
    }
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

    #[test]
    fn parses_stock_fields() {
        let m = Manifest::parse(
            r#"{"name":"vm","id":"x.vm","displayName":{"fallback":"Voicemail","de_de":"VM"},
               "type":"idle","iconFile":"i.png","wasm":"vm.wasm","capabilities":["ui","pbx"]}"#,
        )
        .unwrap();
        assert_eq!(m.display_name, "Voicemail");
        assert_eq!(m.kind, AppType::Idle);
        assert!(m.has_idle);
        assert!(m.has(Capability::Pbx) && !m.has(Capability::Calls));
    }

    #[test]
    fn javascript_and_bad_manifests_are_told_apart() {
        assert_eq!(
            Manifest::parse(r#"{"name":"x","jsFiles":["a.js"]}"#),
            Err(ManifestError::JavaScript)
        );
        assert_eq!(
            Manifest::parse(r#"{"name":"x"}"#),
            Err(ManifestError::BadWasm)
        );
        assert_eq!(
            Manifest::parse(r#"{"name":"x","wasm":"../x.wasm"}"#),
            Err(ManifestError::BadWasm)
        );
        assert!(matches!(
            Manifest::parse(r#"{"name":"x","wasm":"x.wasm","type":"zzz"}"#),
            Err(ManifestError::BadType(_))
        ));
        assert!(matches!(
            Manifest::parse(r#"{"name":"x","wasm":"x.wasm","capabilities":["root"]}"#),
            Err(ManifestError::BadCapability(_))
        ));
    }
}
