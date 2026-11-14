//! A minimal XML reader for the phone's configuration documents (contact
//! lists, app configuration): elements, attributes, and nesting.  Text
//! content, comments, and declarations are skipped.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Element>,
}

impl Element {
    /// Attribute value, or "".
    pub fn attr(&self, name: &str) -> &str {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    }

    /// Direct children with this name.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// All descendants with this name, depth first.
    pub fn find_all<'a>(&'a self, name: &str, out: &mut Vec<&'a Element>) {
        for c in &self.children {
            if c.name == name {
                out.push(c);
            }
            c.find_all(name, out);
        }
    }
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Parses a document; returns a synthetic root whose children are the
/// top-level elements.  Malformed input yields what was read so far.
pub fn parse(text: &str) -> Element {
    let b = text.as_bytes();
    let mut stack: Vec<Element> = vec![Element::default()];
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &text[i..];
        if rest.starts_with("<!--") {
            i += rest.find("-->").map(|e| e + 3).unwrap_or(rest.len());
            continue;
        }
        if rest.starts_with("<?") || rest.starts_with("<!") {
            i += rest.find('>').map(|e| e + 1).unwrap_or(rest.len());
            continue;
        }
        if rest.starts_with("</") {
            i += rest.find('>').map(|e| e + 1).unwrap_or(rest.len());
            if stack.len() > 1 {
                let done = stack.pop().unwrap();
                stack.last_mut().unwrap().children.push(done);
            }
            continue;
        }
        // Start tag: name, then attributes, up to `>` outside quotes.
        let mut j = i + 1;
        while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' && b[j] != b'/' {
            j += 1;
        }
        let mut el = Element {
            name: text[i + 1..j].to_string(),
            ..Default::default()
        };
        let mut self_closing = false;
        loop {
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= b.len() {
                break;
            }
            if b[j] == b'>' {
                j += 1;
                break;
            }
            if b[j] == b'/' {
                self_closing = true;
                j += 1;
                continue;
            }
            let start = j;
            while j < b.len() && b[j] != b'=' && b[j] != b'>' && !b[j].is_ascii_whitespace() {
                j += 1;
            }
            let key = text[start..j].to_string();
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < b.len() && b[j] == b'=' {
                j += 1;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < b.len() && (b[j] == b'"' || b[j] == b'\'') {
                    let q = b[j];
                    let vstart = j + 1;
                    j = vstart;
                    while j < b.len() && b[j] != q {
                        j += 1;
                    }
                    el.attrs
                        .push((key, unescape(&text[vstart..j.min(b.len())])));
                    j = (j + 1).min(b.len());
                }
            } else if !key.is_empty() {
                el.attrs.push((key, String::new()));
            }
        }
        i = j;
        if self_closing {
            stack.last_mut().unwrap().children.push(el);
        } else if stack.len() < 32 {
            stack.push(el);
        }
    }
    while stack.len() > 1 {
        let done = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(done);
    }
    stack.pop().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contacts_document() {
        let doc = parse(
            r#"<?xml version="1.0"?>
            <contacts group_name="Internal Extensions" editable="0" >
              <!-- note -->
              <contact first_name="Ann &amp; Co" last_name="Lee" contact_type="sip" subscribe_to="auto_hint_101">
                <numbers><number dial="101" label="Extension" primary="1" /></numbers>
              </contact>
              <contact first_name='Bo' last_name=""/>
            </contacts>"#,
        );
        let contacts = &doc.children[0];
        assert_eq!(contacts.attr("group_name"), "Internal Extensions");
        let list: Vec<_> = contacts.children_named("contact").collect();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].attr("first_name"), "Ann & Co");
        let mut nums = Vec::new();
        list[0].find_all("number", &mut nums);
        assert_eq!(nums[0].attr("dial"), "101");
        assert_eq!(list[1].attr("first_name"), "Bo");
    }

    #[test]
    fn tolerates_truncation() {
        let doc = parse("<a x=\"1\"><b y=\"2\"");
        assert_eq!(doc.children[0].name, "a");
    }
}
