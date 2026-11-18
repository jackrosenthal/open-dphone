//! The EXP100/EXP150 expansion module protocol (new family).
//!
//! A module hangs off the PC Ethernet port and talks plain text over UDP on
//! a private tagged VLAN.  Everything here follows the stock `phone` binary
//! of the D60: message names, their exact text, and what the phone
//! answers.  It is the wire format only: no sockets, no state.  Nothing
//! here has been run against a real module.
//!
//! The network, as stock sets it up:
//!
//! - VLAN [`VLAN_ID`] on the phone's Ethernet device; the phone's address is
//!   [`private_address`] with a 16-bit prefix.
//! - A module announces itself to [`DISCOVERY_GROUP`]:[`DISCOVERY_PORT`]
//!   with `EXP_NOTIFY`; the phone answers to
//!   [`ANNOUNCE_GROUP`]:[`ANNOUNCE_PORT`] with the node number it gave the
//!   module.
//! - After that, phone and module exchange datagrams between their own
//!   addresses on [`NODE_PORT`].
//!
//! A message is lines separated by LF.  The first line is the command
//! (compared without regard to case).  A datagram is at most
//! [`MAX_DATAGRAM`] bytes.
//!
//! Not recovered (see also the notes on the items below): the module
//! firmware update (`CHECK_VERSION_EVENT` with `HAS_VERSION:1`, the
//! `/tmp/extra/fw*.rom` files), the BLF icon and photo transfer
//! (`EXP_BLF_GET_ICONS`, `GET_EXP_BLF_PHOTO`), and the meaning of the
//! `BLF_STATUS` numbers.

use std::net::Ipv4Addr;

use opendeskphone_state::{Blink, LampColor, LampState};

pub const VLAN_ID: u16 = 3182;
pub const DISCOVERY_GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 2, 1);
pub const DISCOVERY_PORT: u16 = 10024;
pub const ANNOUNCE_GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 3, 1);
pub const ANNOUNCE_PORT: u16 = 10026;
pub const NODE_PORT: u16 = 10025;
pub const MAX_DATAGRAM: usize = 2048;

/// Modules in a chain.
pub const MAX_NODES: usize = 6;
/// Key pages on each module.
pub const PAGES: usize = 2;
/// Keys on each page.
pub const KEYS_PER_PAGE: usize = 20;

/// Key background colors when the configuration gives none: the outer
/// columns and the inner columns of the four-column key grid.
pub const KEY_BG_OUTER: u32 = 0x616161;
pub const KEY_BG_INNER: u32 = 0x212121;
/// Key text color when the configuration gives none.
pub const KEY_FG: u32 = 0xeeeeee;
/// Title bar colors.
pub const TITLE_BG: u32 = 0x01579b;
pub const TITLE_FG: u32 = 0xffffff;

/// The phone's address on the expansion VLAN: `172.20.<MAC byte 4>.<MAC
/// byte 5>`, with a 16-bit prefix.
pub fn private_address(mac: [u8; 6]) -> Ipv4Addr {
    Ipv4Addr::new(172, 20, mac[4], mac[5])
}

/// The default background of key `index` on its page.
pub fn default_key_bg(index: usize) -> u32 {
    if matches!(index & 3, 0 | 3) { KEY_BG_OUTER } else { KEY_BG_INNER }
}

/// Splits a message into its lines the way stock does: LF ends a field,
/// NUL ends the message, and a byte with the high bit set makes the rest
/// unusable.
pub fn fields(data: &[u8]) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = data;
    while !rest.is_empty() {
        let end = rest.iter().position(|b| *b == b'\n' || *b == 0 || *b & 0x80 != 0).unwrap_or(rest.len());
        // The bytes are ASCII by the test above.
        out.push(std::str::from_utf8(&rest[..end]).unwrap_or(""));
        match rest.get(end) {
            Some(b'\n') => rest = &rest[end + 1..],
            _ => break,
        }
    }
    out
}

/// A message from a module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Discovery: `EXP_NOTIFY`, then `Version:<text>`.
    Notify { version: String },
    SocketDetect,
    SynAck,
    VoicemailGet,
    DndGet,
    ForwardGet,
    RegisterGet,
    BacklightGet,
    /// The module asks for its key configuration.
    CfgGet,
    /// The module's firmware versions.
    Version { image: String, boot: String, rom: String },
    /// The module stored the configuration it was sent.
    CfgSaveAck,
    CheckVersion,
    /// A key was pressed.  `echo` is what the acknowledgement repeats.
    Key { page: u8, key: u8, long: bool, echo: (String, String) },
    /// The module switched to key page `page`.
    Page { page: u8 },
    BlfPhotoGet,
    BlfStatusAck,
    Unknown(String),
}

fn after<'a>(field: Option<&&'a str>, prefix: &str) -> Option<&'a str> {
    field.and_then(|f| f.strip_prefix(prefix))
}

fn between(text: &str, marker: &str) -> String {
    match text.find(marker) {
        Some(i) => {
            let rest = &text[i + marker.len()..];
            rest.split(|c: char| c.is_whitespace() || c == ',' || c == ';').next().unwrap_or("").to_string()
        }
        None => String::new(),
    }
}

/// Parses a datagram from a module.  `None` when it has no command line.
pub fn parse(data: &[u8]) -> Option<Request> {
    let f = fields(data);
    let command = f.first()?.trim();
    let is = |name: &str| command.eq_ignore_ascii_case(name);
    Some(if is("EXP_NOTIFY") {
        // Stock keeps what follows "Version:" in the second line.
        let version = f.get(1).and_then(|l| l.find("Version:").map(|i| l[i + 8..].to_string())).unwrap_or_default();
        Request::Notify { version }
    } else if is("SOCKET_DETECT_EVENT") {
        Request::SocketDetect
    } else if is("EXP_SYN_ACK") {
        Request::SynAck
    } else if is("VOICEMAIL_GET_EVENT") {
        Request::VoicemailGet
    } else if is("DND_GET_EVENT") {
        Request::DndGet
    } else if is("FORWARD_GET_EVENT") {
        Request::ForwardGet
    } else if is("REGISTER_GET_EVENT") {
        Request::RegisterGet
    } else if is("BACKLIGHT_GET_EVENT") {
        Request::BacklightGet
    } else if is("CFG_GET_EVENT") {
        Request::CfgGet
    } else if is("EXP_VERSION_EVENT") {
        // Stock reads the three versions from the fourth line; they are
        // looked for in the whole message here.
        let text = f[1..].join(" ");
        Request::Version {
            image: between(&text, "IMG_VERSION:IMG--"),
            boot: between(&text, "BOOT_VERSION:BOOT--"),
            rom: between(&text, "ROM_VERSION:ROM--"),
        }
    } else if is("EXP_CFG_SAVE_EVENT_ACK") {
        Request::CfgSaveAck
    } else if is("CHECK_VERSION_EVENT") {
        Request::CheckVersion
    } else if is("EXP_KEYEVENT") || is("EXP_LONG_PRESS_KEYEVENT") {
        // Lines: command, an event line, one more, EXP_PAGE:<n>, EXP_KEYNO:<n>.
        let page = after(f.get(3), "EXP_PAGE:").and_then(|v| v.trim().parse().ok());
        let key = after(f.get(4), "EXP_KEYNO:").and_then(|v| v.trim().parse().ok());
        match (page, key) {
            (Some(page), Some(key)) => Request::Key {
                page,
                key,
                long: is("EXP_LONG_PRESS_KEYEVENT"),
                echo: (f.get(1).unwrap_or(&"").to_string(), f.get(4).unwrap_or(&"").to_string()),
            },
            _ => Request::Unknown(command.to_string()),
        }
    } else if is("EXP_PAGE_EVENT") {
        // Lines: command, an event line, EXP_PAGE:<n>, EXP_PAGE_KEY:<n>.
        match after(f.get(2), "EXP_PAGE:").and_then(|v| v.trim().parse().ok()) {
            Some(page) => Request::Page { page },
            None => Request::Unknown(command.to_string()),
        }
    } else if is("GET_EXP_BLF_PHOTO") {
        Request::BlfPhotoGet
    } else if is("EXP_BLF_STATUS_ACK") {
        Request::BlfStatusAck
    } else {
        Request::Unknown(command.to_string())
    })
}

// ------------------------------------------------------------- phone side

/// Answer to discovery, sent to the announce group.  The node number is
/// one character.
pub fn notify_ack(node: u8) -> String {
    format!("EXP_NOTIFY_ACK\nEXP_NO:{node}")
}

pub fn socket_detect_ack(node: u8) -> String {
    format!("SOCKET_DETECT_EVENT_ACK\nEXP_NO:{node}\n")
}

pub fn syn_ack_ok(node: u8) -> String {
    format!("EXP_SYN_ACK_OK\nEXP_NO:{node}\n")
}

/// The three status answers stock gives without looking at the phone's
/// state: always zero.
pub const VOICEMAIL_STATUS: &str = "EXP_VOICEMAIL_STATUS\nVOICEMAIL_STATUS:0";
pub const DND_STATUS: &str = "EXP_DND_STATUS\nDND_FLAG:0\n";
pub const FORWARD_STATUS: &str = "EXP_FORWARD_STATUS\nFORWARD_FLAG:0\nFORWARD_NUMBER:0\n";

/// Answer to `REGISTER_GET_EVENT`: one status number per account.
pub fn register_status(accounts: &[u8]) -> String {
    let mut s = format!("EXP_REGISTER_STATUS\nACCOUNT_NUM:{}\n", accounts.len());
    for (i, status) in accounts.iter().enumerate() {
        s.push_str(&format!("ACCOUNT_{i}_STATUS:{status}\n"));
    }
    s
}

/// Answer to `BACKLIGHT_GET_EVENT`.
pub fn backlight_status(level: u8) -> String {
    format!("NOTIFY_BACKLIGHT\nEXP_BACKLIGHT_LEVEL:{level}\n")
}

/// Tells a module the backlight level.
pub fn backlight_notify(level: u8) -> String {
    format!("NOTIFY_BACKLIGHT\nBACKLIGHT_LEVEL:{level}\n")
}

/// Answer to `CHECK_VERSION_EVENT`.  `has_version` asks the module to fetch
/// new firmware; the update itself is not recovered, so callers pass false.
pub fn check_version_ack(has_version: bool, node: u8) -> String {
    format!("CHECK_VERSION_EVENT_ACK\nHAS_VERSION:{}\nEXP_NO:{node}\n", has_version as u8)
}

/// Acknowledges a key press: the header, then two lines of the request
/// run together, exactly as stock builds it.
pub fn key_ack(echo: &(String, String)) -> String {
    format!("EXP_KEYEVENT_ACK\n{}{}", echo.0, echo.1)
}

/// What a key does, as far as the module needs to know (`DssType`).  Stock
/// sends 0 for an empty key and 3 for a contact or application key; other
/// values exist in the module but are never sent by the D60 firmware.
pub const DSS_NONE: u8 = 0;
pub const DSS_KEY: u8 = 3;

/// One key of a page in the configuration sent to a module.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyConfig {
    pub dss_type: u8,
    pub value: String,
    pub label: String,
    /// Background and text colors (`0xRRGGBB`); `None` takes the defaults.
    pub bg: Option<u32>,
    pub fg: Option<u32>,
}

/// The module-wide settings at the end of the configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleSettings {
    /// `<PASS_WORD:...>` beside the fixed user name `admin`.
    pub password: String,
    /// `<BACKLIGHT_ACTIVE_LEVEL:...>`.
    pub backlight_level: u8,
    /// Show the dots beside the keys.
    pub key_dots: bool,
}

fn one_line(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(80).collect()
}

/// The configuration for one module: [`PAGES`] pages of
/// [`KEYS_PER_PAGE`] keys, then the module settings.  `pages` may be
/// shorter than that; missing keys are empty.  As the answer to
/// `CFG_GET_EVENT` it follows an `EXP_CFG_GET_ACK` line (`answer`); pushed
/// on the phone's own initiative it does not.
pub fn config(answer: bool, event_id: u32, node: u8, pages: &[Vec<KeyConfig>], settings: &ModuleSettings) -> String {
    let mut s = String::with_capacity(4096);
    if answer {
        s.push_str("EXP_CFG_GET_ACK\n");
    }
    s.push_str(&format!("EXP_CFG_SAVE_EVENT\nEVENT_ID:{{{event_id}}}\nEXP_NO:{node}\n"));
    let empty = KeyConfig::default();
    for page in 0..PAGES {
        s.push_str(&format!("<EXP_CFG_PAGE_{}>\n", page + 1));
        for index in 0..KEYS_PER_PAGE {
            let key = pages.get(page).and_then(|p| p.get(index)).unwrap_or(&empty);
            let pickup = if key.dss_type == DSS_KEY { "**" } else { "" };
            s.push_str(&format!(
                "<KEY_{index}>\nKeyNo:{node}\nDssType:{}\nValue:{}\naid:0\nPickUpCode:{pickup}\nLabel:{}\nKEY_BG_COLOR:{}\nKEY_FG_COLOR:{}\n",
                key.dss_type,
                one_line(&key.value),
                one_line(&key.label),
                key.bg.unwrap_or(default_key_bg(index)),
                key.fg.unwrap_or(KEY_FG),
            ));
        }
    }
    s.push_str(&format!("<USER_NAME:admin>\n<PASS_WORD:{}>\n", one_line(&settings.password)));
    s.push_str("<SELF_HOST:1>\n<HOST_ADDR:>\n<SCREEN_SAVER_TIME:0>\n<BACKLIGHT_TIME:0>\n");
    s.push_str(&format!("<BACKLIGHT_ACTIVE_LEVEL:{}>\n", settings.backlight_level));
    s.push_str("<FONT_SIZE:1>\n<EXP_PHOTO_SERVER_PATH:0>\n");
    s.push_str(&format!("<TITLE_BG_COLOR:{TITLE_BG}>\n<TITLE_FG_COLOR:{TITLE_FG}>\n"));
    s.push_str(&format!("<DISABLE_KEY_DOTS:{}>\n", !settings.key_dots as u8));
    s.push_str("<EXPM_LONG_PRESS_TIME:550>\n<END>");
    s
}

/// The `BLF_STATUS` number stock sends for a state it does not otherwise
/// map.  What the other numbers (3 to 7) show on the module is not
/// recovered, so this is the only one used.
pub const BLF_STATUS_DEFAULT: u8 = 8;

/// The `BLF_LED_STATUS` number for a lamp: off and amber are 0; red is 1
/// solid, 4 slow, 5 otherwise; green is 2 solid, 8 slow, 9 otherwise.
pub fn led_status(lamp: LampState) -> u8 {
    let red = match lamp.color {
        LampColor::Off | LampColor::Amber => return 0,
        LampColor::Red => true,
        LampColor::Green => false,
    };
    match (lamp.blink, red) {
        (Blink::Solid, true) => 1,
        (Blink::Solid, false) => 2,
        (Blink::Slow, true) => 4,
        (Blink::Slow, false) => 8,
        (_, true) => 5,
        (_, false) => 9,
    }
}

/// Updates one key's state on a module.
#[allow(clippy::too_many_arguments)]
pub fn blf_status(event_id: u32, node: u8, page: u8, key: u8, status: u8, led: u8, bg: Option<u32>, fg: Option<u32>) -> String {
    format!(
        "EXP_BLF_STATUS\nEVENT_ID:{{{event_id}}}\nEXP_NO:{node}\nEXP_PAGE:{page}\nEXP_KEYNO:{key}\nBLF_STATUS:{status}\nBLF_LED_STATUS:{led}\nKEY_BG_COLOR:{}\nKEY_FG_COLOR:{}\n",
        bg.unwrap_or(default_key_bg(key as usize)),
        fg.unwrap_or(KEY_FG),
    )
}

/// Sets the text of a module's title bar.  Stock shows the page, as
/// "Page 1/2".
pub fn set_title(text: &str) -> String {
    format!("EXP_SET_TITLE\nTITLE_TEXT:{}\nTITLE_BG_COLOR:{TITLE_BG}\nTITLE_FG_COLOR:{TITLE_FG}\n", one_line(text))
}

/// Sends a module back to its first key page.
pub fn set_first_page(node: u8) -> String {
    format!("EXP_SET_PAGE_EVENT\nEXP_NO:{node}\nEXP_PAGE:0\n")
}

/// Restarts a module.
pub fn reboot(node: u8) -> String {
    format!("EXP_REBOOT\nEXP_NO:{node}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(private_address([0x00, 0x0f, 0xd3, 0x01, 0x02, 0xfe]), Ipv4Addr::new(172, 20, 2, 254));
        assert_eq!([0, 1, 2, 3, 4, 7].map(default_key_bg), [KEY_BG_OUTER, KEY_BG_INNER, KEY_BG_INNER, KEY_BG_OUTER, KEY_BG_OUTER, KEY_BG_OUTER]);
    }

    #[test]
    fn field_rules() {
        assert_eq!(fields(b"A\nB:1\n"), ["A", "B:1"]);
        assert_eq!(fields(b"A\nB"), ["A", "B"]);
        // NUL ends the message; a high-bit byte ends it too.
        assert_eq!(fields(b"A\nB\0C\n"), ["A", "B"]);
        assert_eq!(fields(b"A\nB\xc3\xa9\nC\n"), ["A", "B"]);
    }

    #[test]
    fn requests() {
        assert_eq!(parse(b"EXP_NOTIFY\nVersion:v5.0\n"), Some(Request::Notify { version: "v5.0".into() }));
        assert_eq!(parse(b"exp_notify\n"), Some(Request::Notify { version: String::new() }));
        assert_eq!(parse(b"SOCKET_DETECT_EVENT\nEXP_NO:0\n"), Some(Request::SocketDetect));
        assert_eq!(parse(b"CFG_GET_EVENT\n"), Some(Request::CfgGet));
        assert_eq!(
            parse(b"EXP_KEYEVENT\nEVENT_ID:{77}\nEXP_NO:0\nEXP_PAGE:1\nEXP_KEYNO:13\n"),
            Some(Request::Key { page: 1, key: 13, long: false, echo: ("EVENT_ID:{77}".into(), "EXP_KEYNO:13".into()) })
        );
        assert!(matches!(
            parse(b"EXP_LONG_PRESS_KEYEVENT\nEVENT_ID:{78}\nEXP_NO:0\nEXP_PAGE:0\nEXP_KEYNO:2\n"),
            Some(Request::Key { page: 0, key: 2, long: true, .. })
        ));
        // A key event without its page and key lines is not a key event.
        assert_eq!(parse(b"EXP_KEYEVENT\nEVENT_ID:{77}\n"), Some(Request::Unknown("EXP_KEYEVENT".into())));
        assert_eq!(parse(b"EXP_PAGE_EVENT\nEVENT_ID:{5}\nEXP_PAGE:1\nEXP_PAGE_KEY:0\n"), Some(Request::Page { page: 1 }));
        assert_eq!(
            parse(b"EXP_VERSION_EVENT\nEVENT_ID:{1}\nEXP_NO:0\nIMG_VERSION:IMG--3.0.0.8 BOOT_VERSION:BOOT--1.2 ROM_VERSION:ROM--9\n"),
            Some(Request::Version { image: "3.0.0.8".into(), boot: "1.2".into(), rom: "9".into() })
        );
        assert_eq!(parse(b"SOMETHING_NEW\n"), Some(Request::Unknown("SOMETHING_NEW".into())));
        assert_eq!(parse(b""), None);
    }

    #[test]
    fn answers() {
        assert_eq!(notify_ack(2), "EXP_NOTIFY_ACK\nEXP_NO:2");
        assert_eq!(socket_detect_ack(0), "SOCKET_DETECT_EVENT_ACK\nEXP_NO:0\n");
        assert_eq!(register_status(&[1, 0]), "EXP_REGISTER_STATUS\nACCOUNT_NUM:2\nACCOUNT_0_STATUS:1\nACCOUNT_1_STATUS:0\n");
        assert_eq!(check_version_ack(false, 1), "CHECK_VERSION_EVENT_ACK\nHAS_VERSION:0\nEXP_NO:1\n");
        assert_eq!(key_ack(&("EVENT_ID:{77}".into(), "EXP_KEYNO:13".into())), "EXP_KEYEVENT_ACK\nEVENT_ID:{77}EXP_KEYNO:13");
        assert_eq!(set_first_page(3), "EXP_SET_PAGE_EVENT\nEXP_NO:3\nEXP_PAGE:0\n");
        assert_eq!(
            blf_status(9, 0, 1, 5, BLF_STATUS_DEFAULT, 4, None, None),
            "EXP_BLF_STATUS\nEVENT_ID:{9}\nEXP_NO:0\nEXP_PAGE:1\nEXP_KEYNO:5\nBLF_STATUS:8\nBLF_LED_STATUS:4\nKEY_BG_COLOR:2171169\nKEY_FG_COLOR:15658734\n"
        );
    }

    #[test]
    fn led_numbers() {
        let lamp = |color, blink| LampState { color, blink };
        assert_eq!(led_status(lamp(LampColor::Off, Blink::Solid)), 0);
        assert_eq!(led_status(lamp(LampColor::Amber, Blink::Fast)), 0);
        assert_eq!(led_status(lamp(LampColor::Red, Blink::Solid)), 1);
        assert_eq!(led_status(lamp(LampColor::Green, Blink::Solid)), 2);
        assert_eq!(led_status(lamp(LampColor::Red, Blink::Slow)), 4);
        assert_eq!(led_status(lamp(LampColor::Green, Blink::Slow)), 8);
        assert_eq!(led_status(lamp(LampColor::Red, Blink::Fast)), 5);
        assert_eq!(led_status(lamp(LampColor::Green, Blink::Fast)), 9);
    }

    #[test]
    fn configuration_text() {
        let keys = vec![vec![
            KeyConfig { dss_type: DSS_KEY, value: "1002".into(), label: "Ann\nLee".into(), bg: None, fg: Some(0x00ff00) },
        ]];
        let settings = ModuleSettings { password: "pw".into(), backlight_level: 7, key_dots: true };
        let text = config(true, 42, 1, &keys, &settings);
        assert!(text.starts_with("EXP_CFG_GET_ACK\nEXP_CFG_SAVE_EVENT\nEVENT_ID:{42}\nEXP_NO:1\n<EXP_CFG_PAGE_1>\n<KEY_0>\nKeyNo:1\nDssType:3\nValue:1002\naid:0\nPickUpCode:**\nLabel:AnnLee\nKEY_BG_COLOR:6381921\nKEY_FG_COLOR:65280\n<KEY_1>\nKeyNo:1\nDssType:0\nValue:\naid:0\nPickUpCode:\nLabel:\nKEY_BG_COLOR:2171169\nKEY_FG_COLOR:15658734\n"));
        assert_eq!(text.matches("<KEY_").count(), PAGES * KEYS_PER_PAGE);
        assert!(text.contains("<KEY_19>\n") && text.contains("<EXP_CFG_PAGE_2>\n") && !text.contains("<EXP_CFG_PAGE_3>"));
        assert!(text.ends_with(
            "<USER_NAME:admin>\n<PASS_WORD:pw>\n<SELF_HOST:1>\n<HOST_ADDR:>\n<SCREEN_SAVER_TIME:0>\n<BACKLIGHT_TIME:0>\n<BACKLIGHT_ACTIVE_LEVEL:7>\n<FONT_SIZE:1>\n<EXP_PHOTO_SERVER_PATH:0>\n<TITLE_BG_COLOR:87963>\n<TITLE_FG_COLOR:16777215>\n<DISABLE_KEY_DOTS:0>\n<EXPM_LONG_PRESS_TIME:550>\n<END>"
        ));
        // Pushed without being asked: no acknowledgement line.
        assert!(config(false, 1, 0, &[], &settings).starts_with("EXP_CFG_SAVE_EVENT\n"));
    }
}
