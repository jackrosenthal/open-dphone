//! Settings that only Open DPhone has: `/nvdata/dphone_settings.txtpb`.
//!
//! The file is a `dphone.settings.v1.DphoneSettings` message (see
//! `proto/dphone/settings/v1/settings.proto`) in protobuf text format, read
//! and written with buffa's text-format support.  Stock never reads it, and
//! stock's factory reset deletes it along with every other file it does not
//! keep, which puts these settings back to their defaults.
//!
//! Every field has explicit presence.  Only fields that are set are written;
//! an absent field takes its default, which the accessors below apply.
//!
//! Fields this firmware does not know (written by a newer one) are kept:
//! their text is carried through a read-modify-write untouched.  Comments
//! are not kept, apart from those that follow such a field.

use std::io;

use buffa::text::{TextDecoder, TextFormat, encode_to_string_pretty};

use crate::nvdata::Nvdata;

#[allow(unused_imports, clippy::all)]
mod pb {
    include!(concat!(env!("OUT_DIR"), "/dphone.settings.v1.rs"));
}

pub use pb::DphoneSettings;

/// The settings file, relative to the nvdata root.
pub const SETTINGS_FILE: &str = "dphone_settings.txtpb";
/// Where a settings file that does not parse is moved to.
pub const SETTINGS_FILE_BAD: &str = "dphone_settings.txtpb.bad";

/// Field names of [`DphoneSettings`]; anything else in the file is carried
/// through as text.  A test checks this list against the generated code.
const KNOWN_FIELDS: &[&str] =
    &["ssh_enabled", "ignore_provisioned_firmware", "serial_console", "metrics_enabled", "idle_softkeys"];

const HEADER: &str = "# Open DPhone settings (dphone.settings.v1.DphoneSettings, protobuf text format).\n\
                      # A field that is absent takes its default.\n";

/// Where an effective setting value came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingSource {
    /// Neither set locally nor provisioned.
    Default,
    /// Set on the phone (`/nvdata/dphone_settings.txtpb`).
    Local,
    /// Supplied by the provisioning server.
    Provisioned,
}

/// An effective setting: its value, where it came from, and whether the
/// provisioning server froze it (local changes then have no effect).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolved<T> {
    pub value: T,
    pub source: SettingSource,
    pub locked: bool,
}

/// Combine the sources the way stock layers a local UI setting over a
/// provisioned one: a frozen provisioned value wins, then the local value,
/// then an ordinary provisioned value, then the default.
pub(crate) fn resolve<T: Copy>(local: Option<T>, provisioned: Option<T>, frozen: bool, default: T) -> Resolved<T> {
    match (provisioned, local) {
        (Some(p), _) if frozen => Resolved { value: p, source: SettingSource::Provisioned, locked: true },
        (_, Some(l)) => Resolved { value: l, source: SettingSource::Local, locked: false },
        (Some(p), None) => Resolved { value: p, source: SettingSource::Provisioned, locked: false },
        (None, None) => Resolved { value: default, source: SettingSource::Default, locked: false },
    }
}

/// Setting IDs a provisioning server uses for the [`DphoneSettings`] fields.
pub const PROVISIONED_SSH_ENABLED: &str = "dphone_ssh_enabled";
pub const PROVISIONED_IGNORE_FIRMWARE: &str = "dphone_ignore_provisioned_firmware";
pub const PROVISIONED_SERIAL_CONSOLE: &str = "dphone_serial_console";
pub const PROVISIONED_METRICS_ENABLED: &str = "dphone_metrics_enabled";
pub const PROVISIONED_IDLE_SOFTKEYS: &str = "dphone_idle_softkeys";

/// The names an idle softkey list may hold, besides `app:<name>`.
pub const IDLE_SOFTKEY_NAMES: &[&str] =
    &["call_log", "contacts", "status", "info", "menu", "apps", "dnd", "forward", "redial"];

/// One softkey of the idle screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdleSoftkey {
    /// One of [`IDLE_SOFTKEY_NAMES`], or `app:<name>`.
    pub name: String,
    /// The label a keymap entry carries (`label1`): text, or a stock string
    /// token such as `_CONTACTS`.
    pub label: Option<String>,
}

/// The idle screen's softkeys in effect, where they came from, and whether
/// the provisioning server froze them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdleSoftkeys {
    pub keys: Vec<IdleSoftkey>,
    pub source: SettingSource,
    pub locked: bool,
}

/// Parse an idle softkey list (`call_log,contacts,app:parking`).  Blank
/// items are skipped; an unknown name is an error that names it.
pub fn parse_idle_softkeys(list: &str) -> Result<Vec<IdleSoftkey>, String> {
    list.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| {
            let app = item.strip_prefix("app:").is_some_and(|name| {
                !name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
            });
            if app || IDLE_SOFTKEY_NAMES.contains(&item) {
                Ok(IdleSoftkey { name: item.to_owned(), label: None })
            } else {
                Err(format!("unknown softkey {item:?}"))
            }
        })
        .collect()
}

/// The idle softkey a stock keymap action stands for, if it has a place on
/// the idle screen (`action` and the first `<arg>` of a `<key>`).
pub fn idle_softkey_for_action(action: &str, arg: Option<&str>, label: Option<&str>) -> Option<IdleSoftkey> {
    let name = match action {
        "missed_or_call_log" => "call_log".to_owned(),
        "contacts" | "phonebooks" | "directory" => "contacts".to_owned(),
        "forward_calls" => "forward".to_owned(),
        // The contacts app is the Contacts softkey.
        "show_application" if arg == Some("contacts") => "contacts".to_owned(),
        "show_application" => format!("app:{}", arg.filter(|a| !a.is_empty())?),
        a if IDLE_SOFTKEY_NAMES.contains(&a) => a.to_owned(),
        _ => return None,
    };
    Some(IdleSoftkey { name, label: label.map(str::to_owned) })
}
/// Setting IDs a provisioning server uses for a static IPv4 arrangement:
/// the address as `a.b.c.d/prefix length`, the gateway, and the DNS servers
/// as a comma-separated list.  Stock firmware does not know them.  (Stock's
/// own `network_*` IDs cannot carry a provisioned address: every phone
/// holds them in `ui-config.xml`, which is merged after the fetched
/// configuration.)
pub const PROVISIONED_STATIC_ADDRESS: &str = "dphone_static_address";
pub const PROVISIONED_STATIC_GATEWAY: &str = "dphone_static_gateway";
pub const PROVISIONED_STATIC_DNS: &str = "dphone_static_dns";
/// Element a provisioning server uses for SSH public keys:
/// `<dphone_ssh_authorized_keys><key id="1" value="ssh-ed25519 AAAA... name"/></dphone_ssh_authorized_keys>`.
pub const PROVISIONED_SSH_KEYS: &str = "dphone_ssh_authorized_keys";
/// Where the provisioned SSH keys are mirrored for the SSH server, relative
/// to the nvdata root, in `authorized_keys` format.  Rewritten from the
/// merged configuration on every merge and removed when there are none.
pub const PROVISIONED_SSH_KEYS_FILE: &str = "open/provisioned_authorized_keys";

/// True for a plain public-key line: `<type> <base64> [comment]`, one line,
/// no options field.
pub fn valid_public_key_line(line: &str) -> bool {
    let mut f = line.split(' ');
    let kind = f.next().unwrap_or("");
    let blob = f.next().unwrap_or("");
    !line.contains(['\n', '\r', '\0'])
        && (kind.starts_with("ssh-") || kind.starts_with("ecdsa-sha2-") || kind.starts_with("sk-"))
        && blob.len() >= 16
        && blob.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/' | b'='))
}

/// The settings file as read: the typed message plus the text of fields
/// this firmware does not know.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsDocument {
    pub settings: DphoneSettings,
    /// Verbatim text of unknown top-level fields, one per entry.
    unknown: Vec<String>,
}

impl SettingsDocument {
    /// Parse the file's text.
    pub fn parse(text: &str) -> Result<Self, buffa::text::ParseError> {
        let mut settings = DphoneSettings::default();
        settings.merge_text(&mut TextDecoder::new(text))?;
        // Second pass over the top level to find where each field starts
        // (names are borrowed from the input), so unknown ones can be kept.
        let mut dec = TextDecoder::new(text);
        let mut fields: Vec<(usize, bool)> = Vec::new();
        while let Some(name) = dec.read_field_name()? {
            let start = name.as_ptr() as usize - text.as_ptr() as usize;
            fields.push((start, KNOWN_FIELDS.contains(&name)));
            dec.skip_value()?;
        }
        let mut unknown = Vec::new();
        for (i, (start, known)) in fields.iter().enumerate() {
            if !known {
                let end = fields.get(i + 1).map_or(text.len(), |f| f.0);
                unknown.push(text[*start..end].trim_end().to_owned());
            }
        }
        Ok(SettingsDocument { settings, unknown })
    }

    /// Text of the fields this firmware does not know.
    pub fn unknown_fields(&self) -> &[String] {
        &self.unknown
    }

    /// The file's text: a header comment, the set fields, then the unknown
    /// fields as they were read.
    pub fn to_text(&self) -> String {
        let mut out = String::from(HEADER);
        out.push_str(&encode_to_string_pretty(&self.settings));
        for u in &self.unknown {
            out.push_str(u);
            out.push('\n');
        }
        out
    }

    /// Read the file.  A missing file gives the defaults.  A file that does
    /// not parse is logged, moved aside to [`SETTINGS_FILE_BAD`], and
    /// replaced by the defaults, so a bad file never blocks configuration.
    pub fn load(nv: &Nvdata) -> io::Result<Self> {
        let Some(text) = nv.read_string(SETTINGS_FILE)? else { return Ok(Self::default()) };
        match Self::parse(&text) {
            Ok(doc) => Ok(doc),
            Err(e) => {
                log::error!(
                    "{SETTINGS_FILE} does not parse ({e}); kept as {SETTINGS_FILE_BAD}, using defaults"
                );
                if let Err(e) = std::fs::rename(nv.path(SETTINGS_FILE), nv.path(SETTINGS_FILE_BAD)) {
                    log::error!("could not move {SETTINGS_FILE} aside: {e}");
                }
                Ok(Self::default())
            }
        }
    }

    /// Write the file atomically (temp file, `fsync`, rename).
    pub fn save(&self, nv: &Nvdata) -> io::Result<()> {
        nv.write(SETTINGS_FILE, self.to_text().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_field_list_matches_the_message() {
        let all = DphoneSettings {
            ssh_enabled: Some(true),
            ignore_provisioned_firmware: Some(true),
            serial_console: Some(true),
            metrics_enabled: Some(true),
            idle_softkeys: Some("menu".into()),
        };
        let text = encode_to_string_pretty(&all);
        let names: Vec<&str> = text.lines().map(|l| l.split(':').next().unwrap()).collect();
        assert_eq!(names, KNOWN_FIELDS);
    }

    #[test]
    fn presence_and_defaults() {
        let d = SettingsDocument::parse("").unwrap();
        assert_eq!(d.settings, DphoneSettings::default());
        // Nothing set: nothing but the header is written.
        assert_eq!(d.to_text(), HEADER);

        let d = SettingsDocument::parse("ssh_enabled: false\n").unwrap();
        assert_eq!(d.settings.ssh_enabled, Some(false));
        assert_eq!(d.settings.ignore_provisioned_firmware, None);
        assert_eq!(d.to_text(), format!("{HEADER}ssh_enabled: false\n"));
    }

    #[test]
    fn unknown_fields_survive() {
        let text = "# c\nfuture_number: 7  # why\nssh_enabled: false\nfuture_msg {\n  a: \"x y\"\n  b: [1, 2]\n}\n";
        let mut d = SettingsDocument::parse(text).unwrap();
        assert_eq!(d.unknown_fields(), ["future_number: 7  # why", "future_msg {\n  a: \"x y\"\n  b: [1, 2]\n}"]);
        d.settings.ignore_provisioned_firmware = Some(true);
        let again = SettingsDocument::parse(&d.to_text()).unwrap();
        assert_eq!(again, d);
        assert_eq!(
            d.to_text(),
            format!("{HEADER}ssh_enabled: false\nignore_provisioned_firmware: true\nfuture_number: 7  # why\nfuture_msg {{\n  a: \"x y\"\n  b: [1, 2]\n}}\n")
        );
    }

    #[test]
    fn precedence() {
        use SettingSource::*;
        let r = |l, p, f| resolve(l, p, f, true);
        assert_eq!(r(None, None, false), Resolved { value: true, source: Default, locked: false });
        assert_eq!(r(None, Some(false), false), Resolved { value: false, source: Provisioned, locked: false });
        assert_eq!(r(Some(true), Some(false), false), Resolved { value: true, source: Local, locked: false });
        assert_eq!(r(Some(true), Some(false), true), Resolved { value: false, source: Provisioned, locked: true });
        assert_eq!(r(Some(false), None, true), Resolved { value: false, source: Local, locked: false });
    }

    #[test]
    fn idle_softkey_lists() {
        let names = |l: &str| parse_idle_softkeys(l).map(|v| v.into_iter().map(|k| k.name).collect::<Vec<_>>());
        assert_eq!(names("call_log, contacts,,app:parking ,dnd").unwrap(), ["call_log", "contacts", "app:parking", "dnd"]);
        assert_eq!(names("").unwrap(), Vec::<String>::new());
        assert_eq!(names("menu,lunch").unwrap_err(), "unknown softkey \"lunch\"");
        assert!(names("app:").is_err() && names("app:a b").is_err());
        let key = |a, arg| idle_softkey_for_action(a, arg, None).map(|k| k.name);
        assert_eq!(key("missed_or_call_log", None).as_deref(), Some("call_log"));
        assert_eq!(key("show_application", Some("queues")).as_deref(), Some("app:queues"));
        assert_eq!(key("show_application", Some("contacts")).as_deref(), Some("contacts"));
        assert_eq!(key("forward_calls", None).as_deref(), Some("forward"));
        assert_eq!(key("show_application", None), None);
        assert_eq!(key("none", None), None);
        assert_eq!(key("end_call", None), None);
    }

    #[test]
    fn public_key_lines() {
        assert!(valid_public_key_line("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJ7x alice@laptop"));
        assert!(valid_public_key_line("ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTY="));
        assert!(!valid_public_key_line("command=\"x\" ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJ7x"));
        assert!(!valid_public_key_line("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJ7x\nssh-rsa AAAA"));
        assert!(!valid_public_key_line("ssh-ed25519"));
        assert!(!valid_public_key_line(""));
    }

    #[test]
    fn malformed_text_is_an_error() {
        assert!(SettingsDocument::parse("ssh_enabled: maybe").is_err());
        assert!(SettingsDocument::parse("ssh_enabled: true\nfuture {").is_err());
        assert!(SettingsDocument::parse("<config/>").is_err());
    }
}
