//! The little XML handling provisioning needs: the stock
//! `configurator.xml` settings file, and scanning a fetched configuration
//! for firmware entries.

/// `/nvdata/configurator.xml`: a flat list of `<setting id value>`.
/// Unknown settings are preserved in order, so stock finds its own values
/// intact after a flash back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Configurator {
    pub settings: Vec<(String, String)>,
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The value of attribute `name` in a start tag's text.
pub fn attr(tag: &str, name: &str) -> Option<String> {
    let mut rest = tag;
    while let Some(i) = rest.find(name) {
        let before_ok = i == 0 || rest[..i].ends_with(char::is_whitespace);
        let after = rest[i + name.len()..].trim_start();
        if before_ok && after.starts_with('=') {
            let v = after[1..].trim_start();
            let q = v.chars().next()?;
            if q == '"' || q == '\'' {
                let end = v[1..].find(q)?;
                return Some(unescape(&v[1..1 + end]));
            }
        }
        rest = &rest[i + name.len()..];
    }
    None
}

/// Every start tag called `name`, as its inner text.
pub fn tags<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let open = format!("<{name}");
    let mut rest = xml;
    while let Some(i) = rest.find(&open) {
        let after = &rest[i + open.len()..];
        let boundary = after.chars().next().is_some_and(|c| c.is_whitespace() || c == '>' || c == '/');
        let Some(end) = after.find('>') else { break };
        if boundary {
            out.push(after[..end].trim_end_matches('/'));
        }
        rest = &after[end..];
    }
    out
}

impl Configurator {
    pub fn parse(xml: &[u8]) -> Configurator {
        let text = String::from_utf8_lossy(xml);
        let settings = tags(&text, "setting")
            .into_iter()
            .filter_map(|t| Some((attr(t, "id")?, attr(t, "value").unwrap_or_default())))
            .collect();
        Configurator { settings }
    }

    pub fn get(&self, id: &str) -> Option<&str> {
        self.settings.iter().find(|(k, _)| k == id).map(|(_, v)| v.as_str()).filter(|v| !v.is_empty() && *v != "null")
    }

    /// The DPMA server this file saves, if any: `config_server_url`, which
    /// is written once a server has provisioned the phone.  A
    /// `config_server_type` other than digium/boot66 is stock's HTTP
    /// provisioning, which is not the DPMA client's business.
    pub fn saved_server(&self) -> Option<&str> {
        let url = self.get("config_server_url")?;
        matches!(self.get("config_server_type"), Some("digium") | Some("boot66") | None).then_some(url)
    }

    pub fn set(&mut self, id: &str, value: &str) {
        match self.settings.iter_mut().find(|(k, _)| k == id) {
            Some(s) => s.1 = value.to_string(),
            None => self.settings.push((id.to_string(), value.to_string())),
        }
    }

    pub fn remove(&mut self, id: &str) {
        self.settings.retain(|(k, _)| k != id);
    }

    /// The stock serialization.
    pub fn render(&self) -> Vec<u8> {
        let mut s = String::from("<config>\n");
        for (k, v) in &self.settings {
            s.push_str(&format!("    <setting id=\"{}\" value=\"{}\" />\n", escape(k), escape(v)));
        }
        s.push_str("</config>\n");
        s.into_bytes()
    }
}

/// The firmware version a fetched configuration asks for, if any.
pub fn offered_firmware(config: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(config);
    tags(&text, "firmware").into_iter().find_map(|t| attr(t, "version")).filter(|v| !v.is_empty())
}

/// Removes the `<firmwares>` and `<public_firmwares>` sections, so the
/// rest of the firmware never sees a server-chosen image.
pub fn strip_firmware(config: &[u8]) -> Vec<u8> {
    let mut text = String::from_utf8_lossy(config).into_owned();
    for name in ["firmwares", "public_firmwares"] {
        let (open, close) = (format!("<{name}"), format!("</{name}>"));
        while let Some(start) = text.find(&open) {
            let after = &text[start + open.len()..];
            if !after.starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/') {
                break;
            }
            let end = match (after.find(&close), after.find("/>")) {
                (Some(c), _) => start + open.len() + c + close.len(),
                (None, Some(e)) => start + open.len() + e + 2,
                (None, None) => break,
            };
            text.replace_range(start..end, "");
        }
    }
    text.into_bytes()
}

/// Whether a `digium.config.update` body asks to drop the token
/// (`<option id="delete_token" value="true"/>`).
pub fn deletes_token(body: &[u8]) -> bool {
    let text = String::from_utf8_lossy(body);
    tags(&text, "option")
        .into_iter()
        .any(|t| attr(t, "id").as_deref() == Some("delete_token") && attr(t, "value").as_deref() == Some("true"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configurator_round_trip() {
        let xml = b"<config>\n    <setting id=\"keep_old\" value=\"1\" />\n    <setting id=\"config_server_url\" value=\"sip:proxy@10.0.0.1:5060\" />\n    <setting id=\"config_server_token\" value=\"a&amp;b\" />\n</config>\n";
        let mut c = Configurator::parse(xml);
        assert_eq!(c.get("config_server_url"), Some("sip:proxy@10.0.0.1:5060"));
        assert_eq!(c.get("config_server_token"), Some("a&b"));
        assert_eq!(c.render(), xml.to_vec());
        c.set("config_server_user", "100");
        c.remove("config_server_token");
        let again = Configurator::parse(&c.render());
        assert_eq!(again.get("config_server_user"), Some("100"));
        assert_eq!(again.get("config_server_token"), None);
        assert_eq!(again.get("keep_old"), Some("1"));
    }

    #[test]
    fn firmware_sections() {
        let cfg = b"<config><setting id=\"a\" value=\"1\"/>\n<firmwares>\n<firmware model=\"D60\" version=\"2_9_28\" url=\"http://x/f.eff\" network_id=\"n\" />\n</firmwares>\n<public_firmwares>\n</public_firmwares>\n<accounts/></config>";
        assert_eq!(offered_firmware(cfg).as_deref(), Some("2_9_28"));
        let stripped = String::from_utf8(strip_firmware(cfg)).unwrap();
        assert!(!stripped.contains("firmware"), "{stripped}");
        assert!(stripped.contains("<setting id=\"a\" value=\"1\"/>") && stripped.contains("<accounts/>"));
        assert_eq!(offered_firmware(b"<config/>"), None);
    }

    #[test]
    fn delete_token_option() {
        assert!(deletes_token(b"<config>\t<section id=\"all\"/>\t<option id=\"delete_token\" value=\"true\"/></config>"));
        assert!(!deletes_token(b"<config></config>"));
    }
}
