//! Small helpers: random tokens, header lookup, and name-addr/URI parsing.

use rsipstack::sip::{Header, Headers};

/// `n` random lowercase alphanumerics.
pub fn token(n: usize) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut buf = vec![0u8; n];
    if getrandom::fill(&mut buf).is_err() {
        // No entropy source: fall back to the clock; uniqueness is all
        // that tags and branches need.
        let mut x = dphone_state::clock::mono_ms() as u64 ^ 0x9e37_79b9_7f4a_7c15;
        for b in buf.iter_mut() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *b = x as u8;
        }
    }
    buf.iter().map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char).collect()
}

/// The value of the first header called `name` (case-insensitive; the
/// stack has already expanded compact forms it knows).
pub fn header<'a>(headers: &'a Headers, name: &str) -> Option<&'a str> {
    headers.iter().find(|h| h.name().eq_ignore_ascii_case(name)).map(|h| h.value())
}

/// Every value of the headers called `name`, in order.
pub fn headers_all<'a>(headers: &'a Headers, name: &str) -> Vec<&'a str> {
    headers
        .iter()
        .filter(|h| h.name().eq_ignore_ascii_case(name))
        .map(|h| h.value())
        .collect()
}

/// `(name, value)` of every header, for callers outside the stack.
pub fn header_pairs(headers: &Headers) -> Vec<(String, String)> {
    headers.iter().map(|h: &Header| (h.name().to_string(), h.value().to_string())).collect()
}

/// A parsed `name-addr` or `addr-spec` header value (From, To, Contact,
/// Refer-To, Route).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NameAddr {
    pub display: String,
    pub uri: String,
    /// Header parameters (after the `>`), lowercased names.
    pub params: Vec<(String, String)>,
}

impl NameAddr {
    pub fn parse(value: &str) -> NameAddr {
        let value = value.trim();
        let (display, uri, rest) = if let Some(lt) = find_unquoted(value, '<') {
            let display = value[..lt].trim().trim_matches('"').to_string();
            match value[lt + 1..].find('>') {
                Some(gt) => (display, &value[lt + 1..lt + 1 + gt], &value[lt + 2 + gt..]),
                None => (display, &value[lt + 1..], ""),
            }
        } else {
            // addr-spec: parameters after the first ';' belong to the header.
            match value.find(';') {
                Some(sc) => (String::new(), &value[..sc], &value[sc..]),
                None => (String::new(), value, ""),
            }
        };
        let params = rest
            .split(';')
            .filter(|p| !p.trim().is_empty())
            .map(|p| match p.split_once('=') {
                Some((k, v)) => (k.trim().to_ascii_lowercase(), v.trim().trim_matches('"').to_string()),
                None => (p.trim().to_ascii_lowercase(), String::new()),
            })
            .collect();
        NameAddr { display, uri: uri.trim().to_string(), params }
    }

    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn tag(&self) -> Option<&str> {
        self.param("tag")
    }

    /// The user part of the URI.
    pub fn user(&self) -> String {
        SipUri::parse(&self.uri).map(|u| u.user).unwrap_or_default()
    }
}

fn find_unquoted(s: &str, c: char) -> Option<usize> {
    let mut quoted = false;
    for (i, ch) in s.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            ch if ch == c && !quoted => return Some(i),
            _ => {}
        }
    }
    None
}

/// A parsed SIP URI.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SipUri {
    pub secure: bool,
    pub user: String,
    pub host: String,
    pub port: Option<u16>,
    /// URI parameters, lowercased names.
    pub params: Vec<(String, String)>,
    /// URI headers (`?Replaces=...`), raw.
    pub headers: String,
}

impl SipUri {
    pub fn parse(uri: &str) -> Option<SipUri> {
        let uri = uri.trim();
        let (scheme, rest) = uri.split_once(':')?;
        let secure = match scheme.to_ascii_lowercase().as_str() {
            "sip" => false,
            "sips" => true,
            _ => return None,
        };
        let (rest, headers) = match rest.split_once('?') {
            Some((r, h)) => (r, h.to_string()),
            None => (rest, String::new()),
        };
        let (user, hostpart) = match rest.rsplit_once('@') {
            Some((u, h)) => (u.split(':').next().unwrap_or("").to_string(), h),
            None => (String::new(), rest),
        };
        let mut parts = hostpart.split(';');
        let hostport = parts.next().unwrap_or("");
        let params = parts
            .filter(|p| !p.is_empty())
            .map(|p| match p.split_once('=') {
                Some((k, v)) => (k.to_ascii_lowercase(), v.to_string()),
                None => (p.to_ascii_lowercase(), String::new()),
            })
            .collect();
        let (host, port) = if let Some(end) = hostport.strip_prefix('[').and_then(|h| h.find(']')) {
            let host = hostport[..end + 2].to_string();
            let port = hostport[end + 2..].strip_prefix(':').and_then(|p| p.parse().ok());
            (host, port)
        } else {
            match hostport.rsplit_once(':') {
                Some((h, p)) => (h.to_string(), p.parse().ok()),
                None => (hostport.to_string(), None),
            }
        };
        if host.is_empty() {
            return None;
        }
        Some(SipUri { secure, user, host, port, params, headers })
    }

    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// Percent-decodes a URI component (for `Replaces` in Refer-To headers).
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encodes everything outside the unreserved set.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_addr_forms() {
        let n = NameAddr::parse("\"Alice <A>\" <sip:alice@example.com;transport=tcp>;tag=abc;x");
        assert_eq!(n.display, "Alice <A>");
        assert_eq!(n.uri, "sip:alice@example.com;transport=tcp");
        assert_eq!(n.tag(), Some("abc"));
        let n = NameAddr::parse("sip:bob@10.0.0.1:5070;tag=q");
        assert_eq!(n.uri, "sip:bob@10.0.0.1:5070");
        assert_eq!(n.tag(), Some("q"));
        assert_eq!(n.user(), "bob");
        let n = NameAddr::parse("Bob <sip:bob@h>");
        assert_eq!(n.display, "Bob");
    }

    #[test]
    fn uri_forms() {
        let u = SipUri::parse("sips:100:pw@pbx.example:5061;transport=tls?Replaces=a%40b").unwrap();
        assert!(u.secure);
        assert_eq!((u.user.as_str(), u.host.as_str(), u.port), ("100", "pbx.example", Some(5061)));
        assert_eq!(u.param("transport"), Some("tls"));
        assert_eq!(u.headers, "Replaces=a%40b");
        let u = SipUri::parse("sip:[2001:db8::1]:5060").unwrap();
        assert_eq!((u.host.as_str(), u.port), ("[2001:db8::1]", Some(5060)));
        assert!(SipUri::parse("tel:123").is_none());
    }

    #[test]
    fn percent() {
        assert_eq!(percent_decode("a%40b%3Btag%3D1"), "a@b;tag=1");
        assert_eq!(percent_encode("a@b;c=1"), "a%40b%3Bc%3D1");
        assert_eq!(percent_decode("100%"), "100%");
    }
}
