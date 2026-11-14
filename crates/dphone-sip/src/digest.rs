//! HTTP Digest authentication for SIP (RFC 2617 / RFC 3261 section 22):
//! MD5 and MD5-sess, with or without `qop=auth`.

use md5::{Digest, Md5};

/// A parsed `WWW-Authenticate` / `Proxy-Authenticate` challenge.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Challenge {
    pub realm: String,
    pub nonce: String,
    pub opaque: Option<String>,
    pub algorithm: String,
    /// The server offers `qop=auth`.
    pub qop_auth: bool,
    pub stale: bool,
}

impl Challenge {
    /// Parses a challenge header value; `None` unless it is a Digest
    /// challenge with a realm and a nonce.
    pub fn parse(value: &str) -> Option<Challenge> {
        let value = value.trim();
        let rest = value.strip_prefix("Digest").or_else(|| value.strip_prefix("digest"))?;
        let mut c = Challenge { algorithm: "MD5".into(), ..Default::default() };
        for (k, v) in params(rest) {
            match k.as_str() {
                "realm" => c.realm = v,
                "nonce" => c.nonce = v,
                "opaque" => c.opaque = Some(v),
                "algorithm" => c.algorithm = v,
                "qop" => c.qop_auth = v.split(',').any(|q| q.trim().eq_ignore_ascii_case("auth")),
                "stale" => c.stale = v.eq_ignore_ascii_case("true"),
                _ => {}
            }
        }
        (!c.nonce.is_empty()).then_some(c)
    }
}

/// Splits `k=v, k="v, v"` lists.
fn params(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for ch in s.chars().chain(std::iter::once(',')) {
        match ch {
            '"' => {
                quoted = !quoted;
                cur.push(ch);
            }
            ',' if !quoted => {
                if let Some((k, v)) = cur.split_once('=') {
                    out.push((k.trim().to_ascii_lowercase(), v.trim().trim_matches('"').to_string()));
                }
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    out
}

fn md5_hex(parts: &[&str]) -> String {
    let mut h = Md5::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            h.update(b":");
        }
        h.update(p.as_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Builds the `Authorization` / `Proxy-Authorization` header value.
/// `nc` is the nonce count for this nonce (1 for its first use).
pub fn authorize(
    c: &Challenge,
    user: &str,
    password: &str,
    method: &str,
    uri: &str,
    cnonce: &str,
    nc: u32,
) -> String {
    let mut ha1 = md5_hex(&[user, &c.realm, password]);
    if c.algorithm.eq_ignore_ascii_case("MD5-sess") {
        ha1 = md5_hex(&[&ha1, &c.nonce, cnonce]);
    }
    let ha2 = md5_hex(&[method, uri]);
    let nc_text = format!("{nc:08x}");
    let response = if c.qop_auth {
        md5_hex(&[&ha1, &c.nonce, &nc_text, cnonce, "auth", &ha2])
    } else {
        md5_hex(&[&ha1, &c.nonce, &ha2])
    };
    let mut v = format!(
        "Digest username=\"{user}\", realm=\"{}\", nonce=\"{}\", uri=\"{uri}\", response=\"{response}\", algorithm={}",
        c.realm, c.nonce, c.algorithm
    );
    if c.qop_auth {
        v.push_str(&format!(", qop=auth, nc={nc_text}, cnonce=\"{cnonce}\""));
    }
    if let Some(o) = &c.opaque {
        v.push_str(&format!(", opaque=\"{o}\""));
    }
    v
}

/// Server-side check, used by the in-test registrar: recomputes the
/// response of an `Authorization` value.
pub fn verify(authorization: &str, password: &str, method: &str) -> bool {
    let Some(rest) = authorization.trim().strip_prefix("Digest") else { return false };
    let p = params(rest);
    let get = |k: &str| p.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str()).unwrap_or("");
    let c = Challenge {
        realm: get("realm").into(),
        nonce: get("nonce").into(),
        opaque: None,
        algorithm: if get("algorithm").is_empty() { "MD5".into() } else { get("algorithm").into() },
        qop_auth: get("qop") == "auth",
        stale: false,
    };
    let nc = u32::from_str_radix(get("nc"), 16).unwrap_or(1);
    let expect = authorize(&c, get("username"), password, method, get("uri"), get("cnonce"), nc);
    let expect = params(expect.strip_prefix("Digest").unwrap_or(""));
    expect.iter().find(|(n, _)| n == "response").map(|(_, v)| v.as_str()) == Some(get("response"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worked example of RFC 2617 section 3.5.
    #[test]
    fn rfc2617_vector() {
        let c = Challenge::parse(
            "Digest realm=\"testrealm@host.com\", qop=\"auth,auth-int\", \
             nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\", opaque=\"5ccc069c403ebaf9f0171e9517f40e41\"",
        )
        .unwrap();
        assert!(c.qop_auth);
        let v = authorize(&c, "Mufasa", "Circle Of Life", "GET", "/dir/index.html", "0a4f113b", 1);
        assert!(v.contains("response=\"6629fae49393a05397450978507c4ef1\""), "{v}");
        assert!(v.contains("opaque=\"5ccc069c403ebaf9f0171e9517f40e41\""));
        assert!(verify(&v, "Circle Of Life", "GET"));
        assert!(!verify(&v, "wrong", "GET"));
    }

    #[test]
    fn without_qop() {
        let c = Challenge::parse("Digest realm=\"asterisk\", nonce=\"abc\", algorithm=MD5").unwrap();
        assert!(!c.qop_auth);
        let v = authorize(&c, "100", "secret", "REGISTER", "sip:pbx", "x", 1);
        assert!(!v.contains("qop"));
        assert!(verify(&v, "secret", "REGISTER"));
    }

    #[test]
    fn rejects_non_digest() {
        assert!(Challenge::parse("Basic realm=\"x\"").is_none());
        assert!(Challenge::parse("Digest realm=\"x\"").is_none());
    }
}
