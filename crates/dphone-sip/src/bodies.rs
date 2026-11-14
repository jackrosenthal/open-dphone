//! NOTIFY and PUBLISH bodies: `dialog-info+xml` (RFC 4235, with the Digium
//! supplement's local/remote identities), `simple-message-summary`
//! (RFC 3842), PIDF/XPIDF presence, and `message/sipfrag`.

use dphone_state::{BlfState, Party, PresenceStatus};

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

/// What a `dialog-info` NOTIFY says about the watched extension.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DialogInfo {
    pub entity: String,
    pub state: BlfState,
    /// The ringing call's far party (from the Digium supplement or a
    /// standard `<remote>` element), for display and directed pickup.
    pub remote: Party,
    /// Call-ID and tags of the ringing dialog, for pickup with `Replaces`.
    pub call_id: String,
    pub local_tag: String,
    pub remote_tag: String,
    /// The dialog's direction as seen by the watched extension.
    pub recipient: bool,
}

fn identity(el: &Element) -> Party {
    let mut p = Party::default();
    if let Some(id) = el.child("identity") {
        p.name = id.attr("display").unwrap_or("").to_string();
        p.uri = id.text.clone();
    }
    if p.uri.is_empty() {
        if let Some(t) = el.child("target") {
            p.uri = t.attr("uri").unwrap_or("").to_string();
        }
    }
    p.number = crate::util::SipUri::parse(&p.uri).map(|u| u.user).unwrap_or_default();
    p
}

/// Parses `application/dialog-info+xml`.  With several dialogs the most
/// "interesting" state wins: ringing, then on hold, then in use.
pub fn parse_dialog_info(text: &str) -> Option<DialogInfo> {
    let root = parse_xml(text)?;
    if root.name != "dialog-info" {
        return None;
    }
    let mut info = DialogInfo {
        entity: root.attr("entity").unwrap_or("").to_string(),
        state: BlfState::Idle,
        ..Default::default()
    };
    let rank = |s: BlfState| match s {
        BlfState::Ringing => 3,
        BlfState::OnHold => 2,
        BlfState::InUse => 1,
        _ => 0,
    };
    for d in root.children_named("dialog") {
        let state_text = d.child("state").map(|s| s.text.trim().to_ascii_lowercase()).unwrap_or_default();
        let recipient = d.attr("direction") == Some("recipient");
        // Asterisk marks hold with a rendering=no parameter on the local
        // target; some PBXes use <param pname="+sip.rendering" pval="no"/>.
        let on_hold = d
            .child("local")
            .and_then(|l| l.child("target"))
            .map(|t| {
                t.children_named("param").any(|p| {
                    p.attr("pname").is_some_and(|n| n.contains("rendering")) && p.attr("pval") == Some("no")
                })
            })
            .unwrap_or(false);
        let state = match state_text.as_str() {
            "early" | "proceeding" | "trying" => BlfState::Ringing,
            "confirmed" if on_hold => BlfState::OnHold,
            "confirmed" => BlfState::InUse,
            "terminated" | "" => BlfState::Idle,
            _ => BlfState::InUse,
        };
        if rank(state) >= rank(info.state) && state != BlfState::Idle {
            info.state = state;
            info.recipient = recipient;
            info.call_id = d.attr("call-id").unwrap_or("").to_string();
            info.local_tag = d.attr("local-tag").unwrap_or("").to_string();
            info.remote_tag = d.attr("remote-tag").unwrap_or("").to_string();
            info.remote = d.child("remote").map(identity).unwrap_or_default();
        }
    }
    Some(info)
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
    pub open: bool,
    pub status: PresenceStatus,
    pub note: String,
}

/// Parses `application/pidf+xml` and the older `application/xpidf+xml`.
pub fn parse_presence(text: &str) -> Option<PresenceDoc> {
    let root = parse_xml(text)?;
    if root.name != "presence" {
        return None;
    }
    let mut doc = PresenceDoc { entity: root.attr("entity").unwrap_or("").to_string(), ..Default::default() };
    if let Some(basic) = root.find("basic") {
        doc.open = basic.text.trim().eq_ignore_ascii_case("open");
    } else if let Some(status) = root.find("status") {
        // XPIDF: <status status="open"/>.
        doc.open = status.attr("status").is_some_and(|s| s.eq_ignore_ascii_case("open"));
    }
    doc.note = root.find("note").map(|n| n.text.clone()).unwrap_or_default();
    // RPID activities, or the Digium presence element some servers add.
    let activity = root
        .find("activities")
        .and_then(|a| a.children.first())
        .map(|c| c.name.clone())
        .or_else(|| root.find("show").map(|s| s.text.trim().to_string()));
    doc.status = match activity.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("away") => PresenceStatus::Away,
        Some("busy") | Some("dnd") | Some("on-the-phone") => PresenceStatus::Dnd,
        Some("xa") => PresenceStatus::ExtendedAway,
        Some("chat") => PresenceStatus::Chat,
        _ if doc.open => PresenceStatus::Available,
        _ => PresenceStatus::Unavailable,
    };
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
    s.push_str(&format!("<tuple id=\"dphone\"><status><basic>{basic}</basic></status>"));
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

    const RINGING: &str = r#"<?xml version="1.0"?>
<dialog-info xmlns="urn:ietf:params:xml:ns:dialog-info" version="3" state="full" entity="sip:101@pbx.example">
  <dialog id="101" call-id="abc@pbx" local-tag="lt" remote-tag="rt" direction="recipient">
    <state>early</state>
    <local><identity display="Bob &amp; Co">sip:101@pbx.example</identity><target uri="sip:101@pbx.example"/></local>
    <remote><identity display="Alice">sip:100@pbx.example</identity><target uri="sip:100@pbx.example"/></remote>
  </dialog>
</dialog-info>"#;

    #[test]
    fn dialog_info_ringing_with_supplement() {
        let d = parse_dialog_info(RINGING).unwrap();
        assert_eq!(d.state, BlfState::Ringing);
        assert_eq!(d.entity, "sip:101@pbx.example");
        assert_eq!(d.remote.name, "Alice");
        assert_eq!(d.remote.number, "100");
        assert_eq!((d.call_id.as_str(), d.local_tag.as_str(), d.remote_tag.as_str()), ("abc@pbx", "lt", "rt"));
        assert!(d.recipient);
    }

    #[test]
    fn dialog_info_states() {
        let idle = r#"<dialog-info entity="sip:1@x"><dialog id="1"><state>terminated</state></dialog></dialog-info>"#;
        assert_eq!(parse_dialog_info(idle).unwrap().state, BlfState::Idle);
        let none = r#"<dialog-info entity="sip:1@x"/>"#;
        assert_eq!(parse_dialog_info(none).unwrap().state, BlfState::Idle);
        let busy = r#"<dialog-info entity="sip:1@x"><dialog id="1"><state>confirmed</state></dialog></dialog-info>"#;
        assert_eq!(parse_dialog_info(busy).unwrap().state, BlfState::InUse);
        let held = r#"<dialog-info entity="sip:1@x"><dialog id="1"><state>confirmed</state>
            <local><target uri="sip:1@x"><param pname="+sip.rendering" pval="no"/></target></local></dialog></dialog-info>"#;
        assert_eq!(parse_dialog_info(held).unwrap().state, BlfState::OnHold);
        assert!(parse_dialog_info("<presence/>").is_none());
    }

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
        assert!(d.open);
        assert_eq!(d.status, PresenceStatus::Away);
        assert_eq!(d.note, "Lunch & back");
        let closed = parse_presence(&build_pidf("sip:1@x", PresenceStatus::Unavailable, "")).unwrap();
        assert_eq!(closed.status, PresenceStatus::Unavailable);
        let xpidf = r#"<presence><presentity uri="sip:1@x"/><atom id="1"><address uri="sip:1@x"><status status="open"/></address></atom></presence>"#;
        assert!(parse_presence(xpidf).unwrap().open);
    }

    #[test]
    fn sipfrag() {
        assert_eq!(parse_sipfrag("SIP/2.0 200 OK\r\n"), Some(200));
        assert_eq!(parse_sipfrag("SIP/2.0 100 Trying"), Some(100));
        assert_eq!(parse_sipfrag("garbage"), None);
    }
}
