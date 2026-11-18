//! NOTIFY and PUBLISH bodies: PIDF/XPIDF presence (with Asterisk's
//! `digium_presence` element), `simple-message-summary` (RFC 3842), and
//! `message/sipfrag`.

use opendeskphone_state::{BlfState, PresenceStatus};

/// A small tolerant XML walker: enough for the documents PBXes send.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Element {
    /// Local name (namespace prefix removed).
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub children: Vec<Element>,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> {
        self.children.iter().filter(move |c| c.name == name)
    }
    /// First descendant called `name`, depth first.
    pub fn find(&self, name: &str) -> Option<&Element> {
        for c in &self.children {
            if c.name == name {
                return Some(c);
            }
            if let Some(f) = c.find(name) {
                return Some(f);
            }
        }
        None
    }
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn local_name(s: &str) -> String {
    s.rsplit(':').next().unwrap_or(s).to_string()
}

/// Parses a document; returns its root element.
pub fn parse_xml(text: &str) -> Option<Element> {
    let mut stack: Vec<Element> = vec![Element::default()];
    let mut rest = text;
    while let Some(lt) = rest.find('<') {
        let txt = &rest[..lt];
        if !txt.trim().is_empty() {
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&unescape(txt.trim()));
            }
        }
        rest = &rest[lt..];
        if rest.starts_with("<?") {
            rest = &rest[rest.find("?>")? + 2..];
        } else if rest.starts_with("<!--") {
            rest = &rest[rest.find("-->")? + 3..];
        } else if rest.starts_with("<![CDATA[") {
            let end = rest.find("]]>")?;
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&rest[9..end]);
            }
            rest = &rest[end + 3..];
        } else if rest.starts_with("<!") {
            rest = &rest[rest.find('>')? + 1..];
        } else if rest.starts_with("</") {
            let end = rest.find('>')?;
            rest = &rest[end + 1..];
            if stack.len() > 1 {
                let done = stack.pop()?;
                stack.last_mut()?.children.push(done);
            }
        } else {
            // Start tag: find its end outside quotes.
            let mut quote: Option<char> = None;
            let mut end = None;
            for (i, c) in rest.char_indices() {
                match (quote, c) {
                    (Some(q), c) if c == q => quote = None,
                    (None, '"') | (None, '\'') => quote = Some(c),
                    (None, '>') => {
                        end = Some(i);
                        break;
                    }
                    _ => {}
                }
            }
            let end = end?;
            let mut body = &rest[1..end];
            let empty = body.ends_with('/');
            if empty {
                body = &body[..body.len() - 1];
            }
            let body = body.trim();
            let (name, mut attrs_text) = match body.find(char::is_whitespace) {
                Some(i) => (&body[..i], body[i..].trim_start()),
                None => (body, ""),
            };
            let mut el = Element { name: local_name(name), ..Default::default() };
            while let Some(eq) = attrs_text.find('=') {
                let key = attrs_text[..eq].trim();
                let after = attrs_text[eq + 1..].trim_start();
                let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else { break };
                let Some(close) = after[1..].find(q) else { break };
                el.attrs.push((local_name(key), unescape(&after[1..1 + close])));
                attrs_text = after[close + 2..].trim_start();
            }
            rest = &rest[end + 1..];
            if empty {
                stack.last_mut()?.children.push(el);
            } else {
                stack.push(el);
            }
        }
    }
    // Close anything left open (truncated documents).
    while stack.len() > 1 {
        let done = stack.pop()?;
        stack.last_mut()?.children.push(done);
    }
    stack.pop()?.children.into_iter().next()
}

/// A `message-summary` body.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MessageSummary {
    pub waiting: bool,
    pub account: String,
    pub new: u32,
    pub old: u32,
    pub urgent_new: u32,
    pub urgent_old: u32,
}

/// Parses `application/simple-message-summary`.
pub fn parse_message_summary(text: &str) -> MessageSummary {
    let mut m = MessageSummary::default();
    for line in text.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        let v = v.trim();
        match k.trim().to_ascii_lowercase().as_str() {
            "messages-waiting" => m.waiting = v.eq_ignore_ascii_case("yes"),
            "message-account" => m.account = v.to_string(),
            "voice-message" => {
                // new/old (urgent-new/urgent-old)
                let (counts, urgent) = match v.split_once('(') {
                    Some((c, u)) => (c.trim(), u.trim_end_matches(')').trim()),
                    None => (v, ""),
                };
                let pair = |s: &str| {
                    let (a, b) = s.split_once('/').unwrap_or((s, "0"));
                    (a.trim().parse().unwrap_or(0), b.trim().parse().unwrap_or(0))
                };
                (m.new, m.old) = pair(counts);
                if !urgent.is_empty() {
                    (m.urgent_new, m.urgent_old) = pair(urgent);
                }
            }
            _ => {}
        }
    }
    m
}

/// A presence document reduced to what the phone shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PresenceDoc {
    pub entity: String,
    /// `<basic>` (XPIDF: the `status` attribute); `None` when the document
    /// has neither.
    pub open: Option<bool>,
    /// The `<note>` text as sent: the PBX's name for the line state.
    pub note: String,
    /// The line state: the note as stock normalizes it, see
    /// [`line_state`].
    pub state: BlfState,
    /// The user's presence: the `digium_presence` element's `type`,
    /// `subtype`, and text (Asterisk adds the element for subscribers whose
    /// User-Agent names Digium), else the first RPID activity.
    pub status: PresenceStatus,
    pub subtype: String,
    pub message: String,
}

/// The line state a presence document stands for, the way stock's `core`
/// reduces it.  The note is the state when it is one of stock's six names
/// or four aliases, compared exactly as stock does; any other note starting
/// with `Hold` or `Ring` counts as that; then the RPID activity
/// `on-the-phone`; then `<basic>`.
/// Asterisk's notes are `Ready`, `Ringing`, `On the phone`, `On hold`, and
/// `Unavailable` (with `<basic>closed</basic>`).
///
/// One difference from stock: a closed document that stock would call
/// unknown is [`BlfState::Unavailable`] here.
pub fn line_state(note: &str, open: Option<bool>, on_the_phone: bool) -> BlfState {
    match note.trim() {
        "Unknown" => BlfState::Unknown,
        "Ready" | "Empty" | "Available" => BlfState::Idle,
        "Ringing" | "Proceeding" => BlfState::Ringing,
        "On hold" => BlfState::OnHold,
        "On hold ringing" => BlfState::OnHoldRinging,
        "On the phone" | "Holding" => BlfState::InUse,
        n if n.starts_with("Hold") => BlfState::OnHold,
        n if n.starts_with("Ring") => BlfState::Ringing,
        _ if on_the_phone => BlfState::InUse,
        _ => match open {
            Some(true) => BlfState::Idle,
            Some(false) => BlfState::Unavailable,
            None => BlfState::Unknown,
        },
    }
}

/// Parses `application/pidf+xml` and the older `application/xpidf+xml`.
pub fn parse_presence(text: &str) -> Option<PresenceDoc> {
    let root = parse_xml(text)?;
    if root.name != "presence" {
        return None;
    }
    let mut doc = PresenceDoc { entity: root.attr("entity").unwrap_or("").to_string(), ..Default::default() };
    // XPIDF has no note and no `<basic>`: `<status status="open"/>`, or
    // `inuse` for any call (ringing included), or `closed`.
    let mut xpidf_in_use = false;
    if let Some(basic) = root.find("basic") {
        doc.open = Some(basic.text.trim().eq_ignore_ascii_case("open"));
    } else if let Some(status) = root.find("status").and_then(|s| s.attr("status")) {
        xpidf_in_use = status.eq_ignore_ascii_case("inuse");
        doc.open = Some(!status.eq_ignore_ascii_case("closed"));
    }
    doc.note = root.find("note").map(|n| n.text.clone()).unwrap_or_default();
    // The first RPID activity, as stock reads it.
    let activity = root
        .find("activities")
        .and_then(|a| a.children.iter().find(|c| c.name != "note"))
        .map(|c| c.name.to_ascii_lowercase())
        .or_else(|| root.find("show").map(|s| s.text.trim().to_ascii_lowercase()));
    let activity = activity.as_deref();
    doc.state = line_state(&doc.note, doc.open, activity == Some("on-the-phone") || xpidf_in_use);
    if let Some(d) = root.find("digium_presence") {
        // `not_set` (a hint with no presence provider) is no status.
        doc.status = PresenceStatus::from_token(d.attr("type").unwrap_or(""));
        doc.subtype = d.attr("subtype").unwrap_or("").to_string();
        doc.message = d.text.clone();
    } else {
        // `on-the-phone` is the line's state, not the user's status.
        doc.status = match activity {
            Some("away") => PresenceStatus::Away,
            Some("busy") | Some("dnd") => PresenceStatus::Dnd,
            Some("xa") => PresenceStatus::ExtendedAway,
            Some("chat") => PresenceStatus::Chat,
            _ => match doc.open {
                Some(true) => PresenceStatus::Available,
                Some(false) => PresenceStatus::Unavailable,
                None => PresenceStatus::Unknown,
            },
        };
    }
    Some(doc)
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Our own presence as PIDF with an RPID activity, for PUBLISH.
pub fn build_pidf(entity: &str, status: PresenceStatus, note: &str) -> String {
    let (basic, activity) = match status {
        PresenceStatus::Unavailable => ("closed", ""),
        PresenceStatus::Away => ("open", "<rpid:away/>"),
        PresenceStatus::ExtendedAway => ("open", "<rpid:away/>"),
        PresenceStatus::Dnd => ("open", "<rpid:busy/>"),
        _ => ("open", ""),
    };
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n");
    s.push_str(&format!(
        "<presence xmlns=\"urn:ietf:params:xml:ns:pidf\" xmlns:dm=\"urn:ietf:params:xml:ns:pidf:data-model\" \
         xmlns:rpid=\"urn:ietf:params:xml:ns:pidf:rpid\" entity=\"{}\">\r\n",
        escape(entity)
    ));
    s.push_str(&format!("<tuple id=\"opendeskphone\"><status><basic>{basic}</basic></status>"));
    if !note.is_empty() {
        s.push_str(&format!("<note>{}</note>", escape(note)));
    }
    s.push_str("</tuple>\r\n");
    if !activity.is_empty() || status != PresenceStatus::Unknown {
        s.push_str(&format!(
            "<dm:person id=\"p1\"><rpid:activities>{activity}</rpid:activities><dm:note>{}</dm:note></dm:person>\r\n",
            escape(status.token())
        ));
    }
    s.push_str("</presence>\r\n");
    s
}

/// The status code of a `message/sipfrag` body (`SIP/2.0 200 OK`).
pub fn parse_sipfrag(text: &str) -> Option<u16> {
    let line = text.lines().next()?;
    let mut f = line.split_whitespace();
    if !f.next()?.starts_with("SIP/") {
        return None;
    }
    f.next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_summary() {
        let m = parse_message_summary(
            "Messages-Waiting: yes\r\nMessage-Account: sip:100@pbx\r\nVoice-Message: 2/8 (1/0)\r\n",
        );
        assert_eq!(m, MessageSummary { waiting: true, account: "sip:100@pbx".into(), new: 2, old: 8, urgent_new: 1, urgent_old: 0 });
        let m = parse_message_summary("Messages-Waiting: no\r\nVoice-Message: 0/3\r\n");
        assert!(!m.waiting);
        assert_eq!((m.new, m.old), (0, 3));
    }

    #[test]
    fn presence_round_trip() {
        let text = build_pidf("sip:100@pbx", PresenceStatus::Away, "Lunch & back");
        let d = parse_presence(&text).unwrap();
        assert_eq!(d.open, Some(true));
        assert_eq!(d.status, PresenceStatus::Away);
        assert_eq!(d.note, "Lunch & back");
        let closed = parse_presence(&build_pidf("sip:1@x", PresenceStatus::Unavailable, "")).unwrap();
        assert_eq!(closed.status, PresenceStatus::Unavailable);
        assert_eq!(closed.state, BlfState::Unavailable);
        assert!(parse_presence("<dialog-info/>").is_none());
    }

    /// A body Asterisk 22.11.0 sent (recorded from a local server with
    /// `res_open_dpma`; the watched extension is `auto_hint_1002`, a hint on
    /// `PJSIP/1002,CustomPresence:1002`, unless the name says otherwise).
    macro_rules! recorded {
        ($name:literal) => {
            parse_presence(include_str!(concat!("../tests/data/presence/", $name, ".xml"))).unwrap()
        };
    }

    #[test]
    fn asterisk_line_states() {
        let brief = |d: &PresenceDoc| (d.note.clone(), d.open, d.state);
        assert_eq!(brief(&recorded!("ready")), ("Ready".into(), Some(true), BlfState::Idle));
        assert_eq!(brief(&recorded!("ringing")), ("Ringing".into(), Some(true), BlfState::Ringing));
        assert_eq!(brief(&recorded!("on-the-phone")), ("On the phone".into(), Some(true), BlfState::InUse));
        assert_eq!(brief(&recorded!("on-hold")), ("On hold".into(), Some(true), BlfState::OnHold));
        assert_eq!(brief(&recorded!("unavailable")), ("Unavailable".into(), Some(false), BlfState::Unavailable));
        assert_eq!(recorded!("ready").entity, "sip:auto_hint_1002@127.0.0.1:15060");
        // To a subscriber whose User-Agent does not name Digium: the same
        // document without the Digium element.
        assert_eq!(brief(&recorded!("ringing-generic")), ("Ringing".into(), Some(true), BlfState::Ringing));
    }

    #[test]
    fn asterisk_user_presence() {
        let user = |d: &PresenceDoc| (d.status, d.subtype.clone(), d.message.clone());
        let plain = |s: PresenceStatus| (s, String::new(), String::new());
        assert_eq!(user(&recorded!("ready")), plain(PresenceStatus::Available));
        assert_eq!(
            user(&recorded!("ready-away-lunch")),
            (PresenceStatus::Away, "lunch".to_string(), "Back at 1".to_string())
        );
        // A hint with no presence provider: `not_set`.
        assert_eq!(user(&recorded!("ready-not-set")), plain(PresenceStatus::Unknown));
        // The line's state and the user's status are separate: a contact
        // that rings or is on the phone is not thereby "do not disturb",
        // though the document carries `<rpid:on-the-phone/>`; and one set
        // to do not disturb still rings.
        assert_eq!(user(&recorded!("ringing")), plain(PresenceStatus::Available));
        assert_eq!(user(&recorded!("on-the-phone")), plain(PresenceStatus::Available));
        let d = recorded!("ringing-dnd");
        assert_eq!((d.state, d.status), (BlfState::Ringing, PresenceStatus::Dnd));
        assert_eq!(user(&recorded!("ringing-generic")), plain(PresenceStatus::Available));
        // The status of an unregistered contact is still the user's.
        assert_eq!(user(&recorded!("unavailable")), plain(PresenceStatus::Available));
    }

    #[test]
    fn asterisk_xpidf() {
        // Not what Asterisk sends for our Accept header, but accepted:
        // XPIDF says `inuse` for a ringing extension as for a busy one.
        assert_eq!(recorded!("xpidf-open").state, BlfState::Idle);
        assert_eq!(recorded!("xpidf-inuse").state, BlfState::InUse);
        let closed = r#"<presence><atom id="1"><address uri="sip:1@x"><status status="closed"/></address></atom></presence>"#;
        assert_eq!(parse_presence(closed).unwrap().state, BlfState::Unavailable);
    }

    #[test]
    fn notes_as_stock_normalizes_them() {
        use BlfState::*;
        // Stock's six names.
        for (note, state) in [
            ("Unknown", Unknown),
            ("Ready", Idle),
            ("Ringing", Ringing),
            ("On hold", OnHold),
            ("On hold ringing", OnHoldRinging),
            ("On the phone", InUse),
        ] {
            assert_eq!(line_state(note, Some(true), false), state, "{note}");
            assert_eq!(line_state(note, Some(false), true), state, "{note}");
        }
        // Its aliases.
        for (note, state) in [("Holding", InUse), ("Empty", Idle), ("Proceeding", Ringing), ("Available", Idle)] {
            assert_eq!(line_state(note, None, false), state, "{note}");
        }
        // Anything else: the prefixes, the activity, then `<basic>`.
        assert_eq!(line_state("Hold, please", Some(true), true), OnHold);
        assert_eq!(line_state("Ring ring", Some(true), true), Ringing);
        assert_eq!(line_state("ringing", Some(true), false), Idle, "the names are compared exactly");
        assert_eq!(line_state("Busy", Some(true), true), InUse);
        assert_eq!(line_state("Busy", Some(true), false), Idle);
        assert_eq!(line_state("", Some(true), false), Idle);
        assert_eq!(line_state("Unavailable", Some(false), false), Unavailable);
        assert_eq!(line_state("", Some(false), false), Unavailable);
        assert_eq!(line_state("", None, false), Unknown);
        // A whole document with the one note Asterisk never sends.
        let doc = include_str!("../tests/data/presence/on-hold.xml").replace("On hold", "On hold ringing");
        assert_eq!(parse_presence(&doc).unwrap().state, OnHoldRinging);
    }

    #[test]
    fn sipfrag() {
        assert_eq!(parse_sipfrag("SIP/2.0 200 OK\r\n"), Some(200));
        assert_eq!(parse_sipfrag("SIP/2.0 100 Trying"), Some(100));
        assert_eq!(parse_sipfrag("garbage"), None);
    }
}
