//! The HTTP client behind the apps' `http` interface and behind PBX
//! requests whose application server is an HTTP(S) URL.
//!
//! HTTP/1.1 over TCP or TLS, one request per connection.  TLS is rustls
//! with the RustCrypto provider, under the caller's policy (as on stock:
//! the server is checked against the trust bundle unless
//! `allow_insecure_ssl` is set).  Up to five redirects are followed, as
//! stock's libcurl does.  Credentials are sent only in answer to a
//! challenge (stock: `CURLAUTH_ANY`): Digest (RFC 7616: MD5, SHA-256, their
//! `-sess` forms, `qop=auth`) when the server offers it, else Basic.
//! Credentials come from the request or from the URL's `user:password@`
//! part.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use opendeskphone_apps::services::{HttpReply, HttpRequest};
use opendeskphone_sip::TlsConfig;
use md5::{Digest as _, Md5};
use sha2::Sha256;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use super::net::NetHandle;

/// The largest reply (head and body) an app is handed.
const LIMIT: usize = 1 << 20;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_REDIRECTS: usize = 5;

#[derive(Debug, PartialEq, Eq)]
struct Url {
    tls: bool,
    /// Host without brackets.
    host: String,
    port: u16,
    /// The `Host` header value.
    authority: String,
    /// Path and query; never empty.
    target: String,
    credentials: Option<(String, String)>,
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(hex) = s.get(i + 1..i + 3)
            && let Ok(v) = u8::from_str_radix(hex, 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_url(url: &str) -> Result<Url, String> {
    let (tls, rest) = if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else {
        return Err("only http:// and https:// URLs are supported".into());
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let tail = tail.split('#').next().unwrap_or("");
    let target = match tail.chars().next() {
        Some('/') => tail.to_string(),
        Some(_) => format!("/{tail}"),
        None => "/".to_string(),
    };
    let (userinfo, hostport) = match authority.rfind('@') {
        Some(i) => (Some(&authority[..i]), &authority[i + 1..]),
        None => (None, authority),
    };
    let credentials = userinfo.map(|u| {
        let (user, pass) = u.split_once(':').unwrap_or((u, ""));
        (percent_decode(user), percent_decode(pass))
    });
    let (host, port) = if let Some(r) = hostport.strip_prefix('[') {
        let close = r.find(']').ok_or("malformed IPv6 address in the URL")?;
        let port = match &r[close + 1..] {
            "" => None,
            p => Some(p.strip_prefix(':').ok_or("malformed port in the URL")?),
        };
        (&r[..close], port)
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (hostport, None),
        }
    };
    if host.is_empty() || host.contains(|c: char| c.is_whitespace() || c.is_control()) {
        return Err("the URL has no usable host".into());
    }
    let port = match port {
        Some(p) => p.parse::<u16>().map_err(|_| "malformed port in the URL".to_string())?,
        None if tls => 443,
        None => 80,
    };
    if target.contains(|c: char| c.is_whitespace() || c.is_control()) {
        return Err("the URL contains white space".into());
    }
    Ok(Url { tls, host: host.to_string(), port, authority: hostport.to_string(), target, credentials })
}

// ------------------------------------------------------------------------
// Authentication.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Algorithm {
    Md5,
    Sha256,
}

/// One challenge of a `WWW-Authenticate` header.
#[derive(Debug, Default, PartialEq, Eq)]
struct Challenge {
    scheme: String,
    params: Vec<(String, String)>,
}

impl Challenge {
    fn get(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

/// Splits a `WWW-Authenticate` value into its challenges.  A token
/// followed by white space and something other than `=` starts a new
/// challenge; `name=value` and `name="value"` pairs belong to the current
/// one.
fn parse_challenges(value: &str) -> Vec<Challenge> {
    let b = value.as_bytes();
    let mut out: Vec<Challenge> = Vec::new();
    let mut i = 0;
    let token_end = |mut j: usize| {
        while j < b.len() && !b[j].is_ascii_whitespace() && !matches!(b[j], b',' | b'=' | b'"') {
            j += 1;
        }
        j
    };
    while i < b.len() {
        if b[i].is_ascii_whitespace() || b[i] == b',' {
            i += 1;
            continue;
        }
        let end = token_end(i);
        if end == i {
            // A stray `=` or quote: skip it.
            i += 1;
            continue;
        }
        let word = &value[i..end];
        let mut j = end;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if j < b.len() && b[j] == b'=' {
            // A parameter.
            j += 1;
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            let text = if j < b.len() && b[j] == b'"' {
                j += 1;
                let mut bytes = Vec::new();
                while j < b.len() && b[j] != b'"' {
                    if b[j] == b'\\' && j + 1 < b.len() {
                        j += 1;
                    }
                    bytes.push(b[j]);
                    j += 1;
                }
                j += 1;
                String::from_utf8_lossy(&bytes).into_owned()
            } else {
                let e = token_end(j);
                let t = value[j..e].to_string();
                j = e;
                t
            };
            if let Some(c) = out.last_mut() {
                c.params.push((word.to_ascii_lowercase(), text));
            }
            i = j;
        } else {
            out.push(Challenge { scheme: word.to_ascii_lowercase(), params: Vec::new() });
            i = end;
        }
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash(alg: Algorithm, data: &str) -> String {
    match alg {
        Algorithm::Md5 => hex(&Md5::digest(data.as_bytes())),
        Algorithm::Sha256 => hex(&Sha256::digest(data.as_bytes())),
    }
}

fn quote(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The `Authorization` value answering a Digest challenge, or `None` when
/// the challenge asks for something this client does not do.
fn digest_authorization(c: &Challenge, method: &str, uri: &str, user: &str, pass: &str, cnonce: &str) -> Option<String> {
    let name = c.get("algorithm").unwrap_or("MD5");
    let upper = name.to_ascii_uppercase();
    let (alg, sess) = match upper.as_str() {
        "MD5" => (Algorithm::Md5, false),
        "MD5-SESS" => (Algorithm::Md5, true),
        "SHA-256" => (Algorithm::Sha256, false),
        "SHA-256-SESS" => (Algorithm::Sha256, true),
        _ => return None,
    };
    let realm = c.get("realm").unwrap_or("");
    let nonce = c.get("nonce")?;
    let qop = match c.get("qop") {
        None => None,
        Some(list) if list.split(',').any(|q| q.trim().eq_ignore_ascii_case("auth")) => Some("auth"),
        // Only `auth-int`, which needs the body hashed: not offered.
        Some(_) => return None,
    };
    let nc = "00000001";
    let mut ha1 = hash(alg, &format!("{user}:{realm}:{pass}"));
    if sess {
        ha1 = hash(alg, &format!("{ha1}:{nonce}:{cnonce}"));
    }
    let ha2 = hash(alg, &format!("{method}:{uri}"));
    let response = match qop {
        Some(q) => hash(alg, &format!("{ha1}:{nonce}:{nc}:{cnonce}:{q}:{ha2}")),
        None => hash(alg, &format!("{ha1}:{nonce}:{ha2}")),
    };
    let mut v = format!(
        "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", response=\"{response}\"",
        quote(user),
        quote(realm),
        quote(nonce),
        quote(uri)
    );
    if c.get("algorithm").is_some() {
        v.push_str(&format!(", algorithm={name}"));
    }
    if let Some(q) = qop {
        v.push_str(&format!(", qop={q}, nc={nc}, cnonce=\"{cnonce}\""));
    } else if sess {
        v.push_str(&format!(", cnonce=\"{cnonce}\""));
    }
    if let Some(o) = c.get("opaque") {
        v.push_str(&format!(", opaque=\"{}\"", quote(o)));
    }
    Some(v)
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

fn cnonce() -> String {
    use rand_core::RngCore;
    let mut bytes = [0u8; 16];
    rand_core::OsRng.fill_bytes(&mut bytes);
    hex(&bytes)
}

/// The `Authorization` value for the strongest challenge on offer:
/// Digest with SHA-256, Digest with MD5, then Basic.
fn authorization(reply: &HttpReply, method: &str, uri: &str, user: &str, pass: &str) -> Option<String> {
    let challenges: Vec<Challenge> = reply
        .headers
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case("www-authenticate"))
        .flat_map(|(_, v)| parse_challenges(v))
        .collect();
    let cnonce = cnonce();
    let digest = |want_sha: bool| {
        challenges
            .iter()
            .filter(|c| c.scheme == "digest")
            .filter(|c| c.get("algorithm").is_some_and(|a| a.to_ascii_uppercase().starts_with("SHA-256")) == want_sha)
            .find_map(|c| digest_authorization(c, method, uri, user, pass, &cnonce))
    };
    digest(true).or_else(|| digest(false)).or_else(|| {
        challenges
            .iter()
            .any(|c| c.scheme == "basic")
            .then(|| format!("Basic {}", base64(format!("{user}:{pass}").as_bytes())))
    })
}

// ------------------------------------------------------------------------
// One exchange.

fn dechunk(mut data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let end = data.windows(2).position(|w| w == b"\r\n")?;
        let size_text = std::str::from_utf8(&data[..end]).ok()?;
        let size = usize::from_str_radix(size_text.split(';').next()?.trim(), 16).ok()?;
        data = &data[end + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(data.get(..size)?);
        data = data.get(size + 2..)?;
    }
}

struct Head {
    status: u16,
    headers: Vec<(String, String)>,
    /// Offset of the body in the raw reply.
    body_at: usize,
}

fn parse_head(raw: &[u8]) -> Result<Option<Head>, String> {
    let Some(split) = raw.windows(4).position(|w| w == b"\r\n\r\n") else { return Ok(None) };
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .filter(|l| l.starts_with("HTTP/"))
        .and_then(|l| l.split_ascii_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or("malformed status line")?;
    let headers =
        lines.filter_map(|l| l.split_once(':')).map(|(n, v)| (n.trim().to_string(), v.trim().to_string())).collect();
    Ok(Some(Head { status, headers, body_at: split + 4 }))
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
}

/// Sends one request and reads its reply: `Content-Length` bytes, chunks
/// up to the last one, or everything until the server closes.
async fn exchange<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    method: &str,
    head: &str,
    body: &[u8],
) -> Result<HttpReply, String> {
    let io = |e: std::io::Error| e.to_string();
    stream.write_all(head.as_bytes()).await.map_err(io)?;
    stream.write_all(body).await.map_err(io)?;
    stream.flush().await.map_err(io)?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    let mut parsed: Option<Head> = None;
    loop {
        // Skip interim (1xx) replies.
        while parsed.is_none() {
            match parse_head(&raw)? {
                Some(h) if (100..200).contains(&h.status) => {
                    raw.drain(..h.body_at);
                }
                Some(h) => parsed = Some(h),
                None => break,
            }
        }
        if let Some(h) = &parsed {
            let got = raw.len() - h.body_at;
            let no_body = method.eq_ignore_ascii_case("HEAD") || h.status == 204 || h.status == 304;
            let chunked = header(&h.headers, "transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
            let length = header(&h.headers, "content-length").and_then(|v| v.parse::<usize>().ok());
            if no_body {
                raw.truncate(h.body_at);
                break;
            } else if chunked {
                if let Some(decoded) = dechunk(&raw[h.body_at..]) {
                    raw.truncate(h.body_at);
                    raw.extend_from_slice(&decoded);
                    break;
                }
            } else if let Some(n) = length {
                if n > LIMIT {
                    return Err("reply too large".into());
                }
                if got >= n {
                    raw.truncate(h.body_at + n);
                    break;
                }
            }
        }
        let n = match stream.read(&mut buf).await {
            Ok(n) => n,
            // A TLS peer that closes without a closing alert.
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => 0,
            Err(e) => return Err(e.to_string()),
        };
        if n == 0 {
            let Some(h) = &parsed else { return Err("the server closed the connection without a reply".into()) };
            let framed = header(&h.headers, "content-length").is_some()
                || header(&h.headers, "transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
            if framed {
                return Err("the reply was cut short".into());
            }
            break;
        }
        raw.extend_from_slice(&buf[..n]);
        if raw.len() > LIMIT {
            return Err("reply too large".into());
        }
    }
    let h = parsed.ok_or("malformed reply")?;
    let body = raw.split_off(h.body_at);
    Ok(HttpReply { status: h.status, headers: h.headers, body })
}

async fn resolve(net: Option<&NetHandle>, host: &str, port: u16) -> Result<SocketAddr, String> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, port));
    }
    if let Some(resolver) = net.and_then(|n| n.resolver()) {
        return match resolver.lookup_ip(host, false, false).await {
            Ok(addrs) => addrs.first().map(|ip| SocketAddr::new(*ip, port)).ok_or_else(|| format!("{host} has no address")),
            Err(e) => Err(format!("cannot resolve {host}: {e:?}")),
        };
    }
    // Simulator, or the network manager is not up: the host's resolver.
    tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| format!("cannot resolve {host}: {e}"))?
        .next()
        .ok_or_else(|| format!("{host} has no address"))
}

async fn once(
    net: Option<&NetHandle>,
    tls: &TlsConfig,
    url: &Url,
    request: &HttpRequest,
    authorization: Option<&str>,
) -> Result<HttpReply, String> {
    let method = if request.method.is_empty() { "GET" } else { request.method.as_str() };
    if !method.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Err("bad request method".into());
    }
    let mut head = format!("{method} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n", url.target, url.authority);
    let mut has_agent = false;
    for (name, value) in &request.headers {
        if name.is_empty() || name.contains(['\r', '\n', ':', ' ']) || value.contains(['\r', '\n']) {
            return Err("a header is malformed".into());
        }
        // These are the client's to write.
        if ["host", "connection", "content-length", "transfer-encoding"].iter().any(|h| name.eq_ignore_ascii_case(h)) {
            continue;
        }
        if authorization.is_some() && name.eq_ignore_ascii_case("authorization") {
            continue;
        }
        has_agent |= name.eq_ignore_ascii_case("user-agent");
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if !has_agent {
        head.push_str("User-Agent: Open Desk Phone\r\n");
    }
    if let Some(a) = authorization {
        head.push_str(&format!("Authorization: {a}\r\n"));
    }
    head.push_str(&format!("Content-Length: {}\r\n\r\n", request.body.len()));

    let addr = resolve(net, &url.host, url.port).await?;
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| format!("connecting to {addr} timed out"))?
        .map_err(|e| format!("{addr}: {e}"))?;
    let _ = tcp.set_nodelay(true);
    let run = async {
        if url.tls {
            use tokio_rustls::rustls::pki_types::ServerName;
            let config = opendeskphone_sip::tls::client_config(tls)?;
            let name = ServerName::try_from(url.host.clone()).unwrap_or(ServerName::IpAddress(addr.ip().into()));
            let mut tls = tokio_rustls::TlsConnector::from(std::sync::Arc::new(config))
                .connect(name, tcp)
                .await
                .map_err(|e| format!("TLS handshake with {addr} failed: {e}"))?;
            let reply = exchange(&mut tls, method, &head, &request.body).await;
            let _ = tls.shutdown().await;
            reply
        } else {
            let mut tcp = tcp;
            exchange(&mut tcp, method, &head, &request.body).await
        }
    };
    tokio::time::timeout(EXCHANGE_TIMEOUT, run).await.map_err(|_| "the request timed out".to_string())?
}

/// One request to one URL, answering one authentication challenge when
/// there are credentials to answer it with.
async fn authenticated(net: Option<&NetHandle>, tls: &TlsConfig, url: &Url, request: &HttpRequest) -> Result<HttpReply, String> {
    let reply = once(net, tls, url, request, None).await?;
    let credentials = request.credentials.as_ref().or(url.credentials.as_ref());
    if reply.status != 401 {
        return Ok(reply);
    }
    let Some((user, pass)) = credentials else { return Ok(reply) };
    let method = if request.method.is_empty() { "GET" } else { request.method.as_str() };
    match authorization(&reply, method, &url.target, user, pass) {
        Some(value) => once(net, tls, url, request, Some(&value)).await,
        None => Ok(reply),
    }
}

/// Where a redirect leads: an absolute URL, or a path on the same server.
fn redirect_target(from: &Url, location: &str) -> Option<String> {
    let location = location.trim();
    if location.starts_with("http://") || location.starts_with("https://") {
        return Some(location.to_string());
    }
    let scheme = if from.tls { "https" } else { "http" };
    if let Some(rest) = location.strip_prefix("//") {
        return Some(format!("{scheme}://{rest}"));
    }
    if location.starts_with('/') {
        return Some(format!("{scheme}://{}{location}", from.authority));
    }
    // Relative to the directory of the current path.
    let path = from.target.split('?').next().unwrap_or("/");
    let dir = &path[..path.rfind('/').map_or(0, |i| i + 1)];
    (!location.is_empty()).then(|| format!("{scheme}://{}{dir}{location}", from.authority))
}

/// Runs a request, following redirects.
pub async fn request(net: Option<&NetHandle>, tls: &TlsConfig, mut request: HttpRequest) -> Result<HttpReply, String> {
    let mut url = parse_url(&request.url)?;
    for _ in 0..=MAX_REDIRECTS {
        let reply = authenticated(net, tls, &url, &request).await?;
        let location = match reply.status {
            301 | 302 | 303 | 307 | 308 => header(&reply.headers, "location").and_then(|l| redirect_target(&url, l)),
            _ => None,
        };
        let Some(next) = location else { return Ok(reply) };
        let next_url = parse_url(&next)?;
        // As browsers and curl do: these three turn anything but a HEAD
        // into a GET without a body.
        if matches!(reply.status, 301..=303) && !request.method.eq_ignore_ascii_case("HEAD") {
            request.method = "GET".into();
            request.body.clear();
            request.headers.retain(|(n, _)| !n.eq_ignore_ascii_case("content-type"));
        }
        // Credentials are for the server they were given for.
        if next_url.host != url.host || next_url.port != url.port || next_url.tls != url.tls {
            request.credentials = None;
            request.headers.retain(|(n, _)| !n.eq_ignore_ascii_case("authorization") && !n.eq_ignore_ascii_case("cookie"));
        }
        request.url = next;
        url = next_url;
    }
    Err("too many redirects".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn urls() {
        let u = parse_url("http://pbx.example/json?x=1#frag").unwrap();
        assert_eq!((u.tls, u.host.as_str(), u.port, u.target.as_str()), (false, "pbx.example", 80, "/json?x=1"));
        assert_eq!(u.authority, "pbx.example");
        let u = parse_url("https://al%40ice:p%3Aw@[2001:db8::1]:8443").unwrap();
        assert_eq!((u.tls, u.host.as_str(), u.port, u.target.as_str()), (true, "2001:db8::1", 8443, "/"));
        assert_eq!(u.authority, "[2001:db8::1]:8443");
        assert_eq!(u.credentials, Some(("al@ice".into(), "p:w".into())));
        assert_eq!(parse_url("https://host?q").unwrap().target, "/?q");
        assert_eq!(parse_url("https://host").unwrap().port, 443);
        assert!(parse_url("ftp://host/").is_err());
        assert!(parse_url("http:///path").is_err());
        assert!(parse_url("http://host:port/").is_err());
        assert!(parse_url("http://host/a b").is_err());
    }

    #[test]
    fn encodings() {
        assert_eq!(base64(b"user:pass"), "dXNlcjpwYXNz");
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(dechunk(b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n").unwrap(), b"Wikipedia");
        assert!(dechunk(b"zz\r\n").is_none());
        assert!(dechunk(b"4\r\nWi").is_none());
    }

    #[test]
    fn challenges() {
        let c = parse_challenges(
            r#"Digest realm="a, b", qop="auth,auth-int", nonce="n\"1", algorithm=SHA-256, Basic realm="simple""#,
        );
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].scheme, "digest");
        assert_eq!(c[0].get("realm"), Some("a, b"));
        assert_eq!(c[0].get("qop"), Some("auth,auth-int"));
        assert_eq!(c[0].get("nonce"), Some("n\"1"));
        assert_eq!(c[0].get("algorithm"), Some("SHA-256"));
        assert_eq!(c[1].scheme, "basic");
        assert_eq!(c[1].get("realm"), Some("simple"));
        assert_eq!(parse_challenges("Negotiate"), vec![Challenge { scheme: "negotiate".into(), params: vec![] }]);
    }

    /// The worked examples of RFC 7616 section 3.9.1 and RFC 2617 section 3.5.
    #[test]
    fn digest_vectors() {
        let nonce = "7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v";
        let opaque = "FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS";
        let cnonce = "f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ";
        let challenge = |alg: &str| {
            parse_challenges(&format!(
                r#"Digest realm="http-auth@example.org", qop="auth, auth-int", algorithm={alg}, nonce="{nonce}", opaque="{opaque}""#
            ))
            .remove(0)
        };
        let md5 = digest_authorization(&challenge("MD5"), "GET", "/dir/index.html", "Mufasa", "Circle of Life", cnonce).unwrap();
        assert!(md5.contains(r#"response="8ca523f5e9506fed4657c9700eebdbec""#), "{md5}");
        assert!(md5.contains("algorithm=MD5") && md5.contains("qop=auth, nc=00000001") && md5.contains(opaque), "{md5}");
        let sha =
            digest_authorization(&challenge("SHA-256"), "GET", "/dir/index.html", "Mufasa", "Circle of Life", cnonce).unwrap();
        assert!(sha.contains(r#"response="753927fa0e85d155564e2e272a28d1802ca10daf4496794697cf8db5856cb6c1""#), "{sha}");

        // RFC 2617: no algorithm parameter means MD5.
        let old = parse_challenges(
            r#"Digest realm="testrealm@host.com", qop="auth,auth-int", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093", opaque="5ccc069c403ebaf9f0171e9517f40e41""#,
        )
        .remove(0);
        let v = digest_authorization(&old, "GET", "/dir/index.html", "Mufasa", "Circle Of Life", "0a4f113b").unwrap();
        assert!(v.contains(r#"response="6629fae49393a05397450978507c4ef1""#), "{v}");
        assert!(!v.contains("algorithm="), "{v}");

        // Without qop (RFC 2069 form).
        let bare = parse_challenges(r#"Digest realm="r", nonce="abc""#).remove(0);
        let v = digest_authorization(&bare, "POST", "/x", "u", "p", "c").unwrap();
        let ha1 = hash(Algorithm::Md5, "u:r:p");
        let ha2 = hash(Algorithm::Md5, "POST:/x");
        assert!(v.contains(&hash(Algorithm::Md5, &format!("{ha1}:abc:{ha2}"))), "{v}");
        assert!(!v.contains("qop="), "{v}");

        // What this client does not do is declined, not guessed at.
        let int_only = parse_challenges(r#"Digest realm="r", nonce="abc", qop="auth-int""#).remove(0);
        assert!(digest_authorization(&int_only, "GET", "/", "u", "p", "c").is_none());
        let odd = parse_challenges(r#"Digest realm="r", nonce="abc", algorithm=SHA-512-256"#).remove(0);
        assert!(digest_authorization(&odd, "GET", "/", "u", "p", "c").is_none());
    }

    #[test]
    fn strongest_challenge_wins() {
        let reply = |values: &[&str]| HttpReply {
            status: 401,
            headers: values.iter().map(|v| ("WWW-Authenticate".to_string(), v.to_string())).collect(),
            body: Vec::new(),
        };
        let both = reply(&[r#"Basic realm="r""#, r#"Digest realm="r", nonce="n", qop="auth""#]);
        assert!(authorization(&both, "GET", "/", "u", "p").unwrap().starts_with("Digest "));
        let algs = reply(&[r#"Digest realm="r", nonce="n", algorithm=MD5, Digest realm="r", nonce="n", algorithm=SHA-256"#]);
        assert!(authorization(&algs, "GET", "/", "u", "p").unwrap().contains("algorithm=SHA-256"));
        let basic = reply(&[r#"Basic realm="r""#]);
        assert_eq!(authorization(&basic, "GET", "/", "user", "pass").unwrap(), "Basic dXNlcjpwYXNz");
        assert!(authorization(&reply(&["Negotiate"]), "GET", "/", "u", "p").is_none());
    }

    use tokio::net::TcpListener;

    const CERT: &[u8] = include_bytes!("../../../../opendeskphone-sip/tests/data/test-cert.pem");
    const KEY: &[u8] = include_bytes!("../../../../opendeskphone-sip/tests/data/test-key.pem");

    /// Reads one request (head and `Content-Length` body) as text.
    async fn read_request<S: AsyncRead + Unpin>(s: &mut S) -> String {
        let mut raw = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            let n = s.read(&mut buf).await.unwrap();
            raw.extend_from_slice(&buf[..n]);
            if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&raw[..end]).to_ascii_lowercase();
                let len = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if raw.len() >= end + 4 + len {
                    break;
                }
            }
            assert!(n > 0, "the client closed early");
        }
        String::from_utf8_lossy(&raw).into_owned()
    }

    /// A server that wants Digest: 401 with a challenge, then 200 with a
    /// chunked body when the answer checks out.
    async fn digest_server<S: AsyncRead + AsyncWrite + Unpin>(mut s: S) {
        let req = read_request(&mut s).await;
        let reply = match req.lines().find_map(|l| l.strip_prefix("Authorization: ")) {
            None => "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Digest realm=\"pbx\", nonce=\"n0nce\", qop=\"auth\", algorithm=MD5\r\nContent-Length: 0\r\n\r\n".to_string(),
            Some(auth) => {
                let c = parse_challenges(auth).remove(0);
                let ha1 = hash(Algorithm::Md5, "app:pbx:secret");
                let ha2 = hash(Algorithm::Md5, &format!("POST:{}", c.get("uri").unwrap()));
                let want = hash(
                    Algorithm::Md5,
                    &format!("{ha1}:n0nce:{}:{}:auth:{ha2}", c.get("nc").unwrap(), c.get("cnonce").unwrap()),
                );
                assert_eq!(c.get("response"), Some(want.as_str()), "{auth}");
                assert_eq!(c.get("username"), Some("app"));
                assert!(req.ends_with("{\"request\":1}"), "{req}");
                "HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n".to_string()
            }
        };
        s.write_all(reply.as_bytes()).await.unwrap();
        let _ = s.shutdown().await;
    }

    fn post(url: String, credentials: Option<(&str, &str)>) -> HttpRequest {
        HttpRequest {
            method: "POST".into(),
            url,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: b"{\"request\":1}".to_vec(),
            credentials: credentials.map(|(u, p)| (u.to_string(), p.to_string())),
        }
    }

    #[tokio::test]
    async fn digest_over_plain_http() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let (s, _) = listener.accept().await.unwrap();
                tokio::spawn(digest_server(s));
            }
        });
        let reply = request(None, &TlsConfig::default(), post(format!("http://127.0.0.1:{port}/json"), Some(("app", "secret")))).await.unwrap();
        assert_eq!((reply.status, reply.body.as_slice()), (200, &b"hello world"[..]));
        // Credentials in the URL work the same way.
        let reply = request(None, &TlsConfig::default(), post(format!("http://app:secret@127.0.0.1:{port}/json"), None)).await.unwrap();
        assert_eq!(reply.status, 200);
        // Without credentials the challenge is the answer.
        let reply = request(None, &TlsConfig::default(), post(format!("http://127.0.0.1:{port}/json"), None)).await.unwrap();
        assert_eq!(reply.status, 401);
    }

    #[tokio::test]
    async fn digest_over_https() {
        let certs: Vec<_> = rustls_pemfile::certs(&mut &CERT[..]).flatten().collect();
        let key = rustls_pemfile::private_key(&mut &KEY[..]).unwrap().unwrap();
        let server = tokio_rustls::rustls::ServerConfig::builder_with_provider(Arc::new(rustls_rustcrypto::provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let (s, _) = listener.accept().await.unwrap();
                let acceptor = acceptor.clone();
                tokio::spawn(async move { digest_server(acceptor.accept(s).await.unwrap()).await });
            }
        });
        let reply = request(None, &TlsConfig::default(), post(format!("https://127.0.0.1:{port}/json"), Some(("app", "secret")))).await.unwrap();
        assert_eq!((reply.status, reply.body.as_slice()), (200, &b"hello world"[..]));
    }

    #[tokio::test]
    async fn framing() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let replies: [&[u8]; 4] = [
                // Content-Length; the connection stays open afterward.
                b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nX-A: b\r\n\r\nabcEXTRA",
                // No framing: the body runs to the close.
                b"HTTP/1.0 200 OK\r\n\r\nuntil close",
                // Cut short.
                b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc",
                // Basic.
                b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"r\"\r\nContent-Length: 0\r\n\r\n",
            ];
            for (i, reply) in replies.into_iter().enumerate() {
                let (mut s, _) = listener.accept().await.unwrap();
                read_request(&mut s).await;
                s.write_all(reply).await.unwrap();
                if i == 0 {
                    // Leave the first connection open: the client must not wait for the close.
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(20)).await;
                        drop(s);
                    });
                }
            }
            let (mut s, _) = listener.accept().await.unwrap();
            let req = read_request(&mut s).await;
            assert!(req.contains("Authorization: Basic dTpw\r\n"), "{req}");
            s.write_all(b"HTTP/1.1 204 No Content\r\n\r\n").await.unwrap();
        });
        let get = |credentials: Option<(&str, &str)>| HttpRequest {
            method: String::new(),
            url: format!("http://127.0.0.1:{port}/"),
            headers: Vec::new(),
            body: Vec::new(),
            credentials: credentials.map(|(u, p)| (u.to_string(), p.to_string())),
        };
        let r = request(None, &TlsConfig::default(), get(None)).await.unwrap();
        assert_eq!((r.status, r.body.as_slice()), (200, &b"abc"[..]));
        assert_eq!(header(&r.headers, "x-a"), Some("b"));
        let r = request(None, &TlsConfig::default(), get(None)).await.unwrap();
        assert_eq!(r.body, b"until close");
        assert_eq!(request(None, &TlsConfig::default(), get(None)).await.unwrap_err(), "the reply was cut short");
        let r = request(None, &TlsConfig::default(), get(Some(("u", "p")))).await.unwrap();
        assert_eq!(r.status, 204);
    }

    #[tokio::test]
    async fn redirects_and_certificate_checks() {
        // Redirects: a relative one, then one that turns the POST into a GET.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let mut seen = Vec::new();
            loop {
                let (mut s, _) = listener.accept().await.unwrap();
                let req = read_request(&mut s).await;
                let line = req.lines().next().unwrap().to_string();
                let reply = match line.as_str() {
                    "POST /a/start HTTP/1.1" => "HTTP/1.1 307 Temporary Redirect\r\nLocation: next\r\nContent-Length: 0\r\n\r\n".to_string(),
                    "POST /a/next HTTP/1.1" => {
                        assert!(req.ends_with("{\"request\":1}"), "a 307 keeps the body: {req}");
                        "HTTP/1.1 302 Found\r\nLocation: /done\r\nContent-Length: 0\r\n\r\n".to_string()
                    }
                    "GET /done HTTP/1.1" => format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}", seen.len(), "x".repeat(seen.len())),
                    "GET /loop HTTP/1.1" => "HTTP/1.1 301 Moved\r\nLocation: /loop\r\nContent-Length: 0\r\n\r\n".to_string(),
                    other => panic!("unexpected request {other}"),
                };
                seen.push(line);
                s.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        let tls = TlsConfig::default();
        let r = request(None, &tls, post(format!("http://127.0.0.1:{port}/a/start"), None)).await.unwrap();
        assert_eq!((r.status, r.body.len()), (200, 2));
        let mut again = post(format!("http://127.0.0.1:{port}/loop"), None);
        again.method = "GET".into();
        assert_eq!(request(None, &tls, again).await.unwrap_err(), "too many redirects");

        // With certificate checking on, a server whose certificate does
        // not chain to the trusted roots is refused; with its issuer
        // trusted, it is accepted.
        let certs: Vec<_> = rustls_pemfile::certs(&mut &CERT[..]).flatten().collect();
        let key = rustls_pemfile::private_key(&mut &KEY[..]).unwrap().unwrap();
        let server = tokio_rustls::rustls::ServerConfig::builder_with_provider(Arc::new(rustls_rustcrypto::provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let (s, _) = listener.accept().await.unwrap();
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    if let Ok(mut s) = acceptor.accept(s).await {
                        read_request(&mut s).await;
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok").await;
                        let _ = s.shutdown().await;
                    }
                });
            }
        });
        let get = |host: &str| HttpRequest { url: format!("https://{host}:{port}/"), ..Default::default() };
        let strict = TlsConfig { verify: true, roots_pem: CERT.to_vec(), ..Default::default() };
        let e = request(None, &strict, get("127.0.0.1")).await.unwrap_err();
        assert!(e.contains("TLS handshake"), "{e}");
        let trusting = TlsConfig {
            verify: true,
            roots_pem: include_bytes!("../../../../opendeskphone-sip/tests/data/test-ca.pem").to_vec(),
            ..Default::default()
        };
        assert_eq!(request(None, &trusting, get("127.0.0.1")).await.unwrap().body, b"ok");
        assert_eq!(request(None, &TlsConfig::default(), get("127.0.0.1")).await.unwrap().body, b"ok");
    }

    #[test]
    fn redirect_targets() {
        let from = parse_url("https://pbx.example:8443/api/v1/call?x=1").unwrap();
        assert_eq!(redirect_target(&from, "http://other/").unwrap(), "http://other/");
        assert_eq!(redirect_target(&from, "//other/p").unwrap(), "https://other/p");
        assert_eq!(redirect_target(&from, "/root").unwrap(), "https://pbx.example:8443/root");
        assert_eq!(redirect_target(&from, "next?y=2").unwrap(), "https://pbx.example:8443/api/v1/next?y=2");
        assert_eq!(redirect_target(&from, " "), None);
    }
}
