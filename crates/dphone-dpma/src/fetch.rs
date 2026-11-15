//! The download client: a small blocking HTTP client that hands the
//! response body out as a stream, so a firmware image goes from the socket
//! to flash without being held in memory.  It follows redirects, as curl
//! does with `-L`.  curl's other schemes (FTP, TFTP) are not supported.
//!
//! How an `https://` server is checked is the caller's choice
//! ([`HttpSource::with_trust`]); without a policy no server is checked.  The
//! phone checks every download, firmware included, against its trust bundle
//! `/nvdata/certs/certs.crt` unless `allow_insecure_ssl` is set.  (Stock
//! does so for the files a configuration names by URL, through `core`'s own
//! client, but fetches firmware with `curl --insecure -L`.)  The handshake
//! is completed before the request is sent, so a refused certificate is
//! reported as such.

use crate::firmware::{FetchError, FirmwareSource};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Resolves a host name.  The default uses the system resolver.
pub type Lookup = dyn Fn(&str, u16) -> std::io::Result<std::net::SocketAddr> + Send + Sync;

/// How an `https://` server is checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trust {
    /// Not at all (`curl --insecure`): any certificate, but a real
    /// handshake.
    Any,
    /// The certificate must chain to one of these roots (PEM) and be valid
    /// for the server's name.
    Roots(Vec<u8>),
}

/// Asked at every `https://` connection, so that the answer follows the
/// configuration.
pub type TrustPolicy = dyn Fn() -> Trust + Send + Sync;

pub struct HttpSource {
    lookup: Box<Lookup>,
    trust: Option<Box<TrustPolicy>>,
    /// Connection and per-read timeout.
    pub timeout: Duration,
    pub max_redirects: u32,
}

impl Default for HttpSource {
    fn default() -> Self {
        HttpSource::new()
    }
}

impl HttpSource {
    pub fn new() -> Self {
        HttpSource::with_lookup(Box::new(|host, port| {
            (host, port)
                .to_socket_addrs()?
                .next()
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no address"))
        }))
    }

    /// With the phone's own resolver (the one fed by DHCP).
    pub fn with_lookup(lookup: Box<Lookup>) -> Self {
        HttpSource { lookup, trust: None, timeout: Duration::from_secs(30), max_redirects: 5 }
    }

    /// Checks `https://` servers as `policy` says.  Without this, none is
    /// checked ([`Trust::Any`]).
    pub fn with_trust(mut self, policy: Box<TrustPolicy>) -> Self {
        self.trust = Some(policy);
        self
    }
}

trait Stream: Read + Write + Send {}
impl<T: Read + Write + Send> Stream for T {}

struct Url {
    tls: bool,
    host: String,
    port: u16,
    path: String,
}

fn parse(url: &str) -> Result<Url, String> {
    let (tls, rest) = if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else {
        return Err(format!("unsupported URL scheme in {url} (http and https are supported)"));
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.ends_with(']') || h.starts_with('[') => match p.parse() {
            Ok(p) => (h, p),
            Err(_) => (authority, if tls { 443 } else { 80 }),
        },
        _ => (authority, if tls { 443 } else { 80 }),
    };
    if host.is_empty() {
        return Err(format!("no host in {url}"));
    }
    Ok(Url { tls, host: host.trim_matches(|c| c == '[' || c == ']').to_string(), port, path: path.to_string() })
}

fn err(message: impl Into<String>) -> FetchError {
    FetchError { status: None, message: message.into() }
}

#[cfg(feature = "https")]
mod tls {
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::crypto::CryptoProvider;
    use super::Trust;
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, RootCertStore, SignatureScheme, StreamOwned};
    use std::net::TcpStream;
    use std::sync::Arc;

    /// `curl --insecure`: any certificate, but a real handshake.
    #[derive(Debug)]
    struct Insecure(Arc<CryptoProvider>);

    impl ServerCertVerifier for Insecure {
        fn verify_server_cert(
            &self,
            _: &CertificateDer<'_>,
            _: &[CertificateDer<'_>],
            _: &ServerName<'_>,
            _: &[u8],
            _: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(
            &self,
            m: &[u8],
            c: &CertificateDer<'_>,
            d: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            rustls::crypto::verify_tls12_signature(m, c, d, &self.0.signature_verification_algorithms)
        }
        fn verify_tls13_signature(
            &self,
            m: &[u8],
            c: &CertificateDer<'_>,
            d: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            rustls::crypto::verify_tls13_signature(m, c, d, &self.0.signature_verification_algorithms)
        }
        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            self.0.signature_verification_algorithms.supported_schemes()
        }
    }

    pub fn wrap(host: &str, tcp: TcpStream, trust: &Trust) -> Result<StreamOwned<ClientConnection, TcpStream>, String> {
        let provider = Arc::new(rustls_rustcrypto::provider());
        let builder = ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?;
        let config = match trust {
            Trust::Any => {
                builder.dangerous().with_custom_certificate_verifier(Arc::new(Insecure(provider))).with_no_client_auth()
            }
            Trust::Roots(pem) => {
                let mut roots = RootCertStore::empty();
                for cert in CertificateDer::pem_slice_iter(pem).flatten() {
                    let _ = roots.add(cert);
                }
                if roots.is_empty() {
                    return Err("certificate checking is on but no trusted roots are configured".into());
                }
                builder.with_root_certificates(roots).with_no_client_auth()
            }
        };
        let name = ServerName::try_from(host.to_string())
            .or_else(|_| tcp.peer_addr().map(|a| ServerName::IpAddress(a.ip().into())))
            .map_err(|e| e.to_string())?;
        let mut conn = ClientConnection::new(Arc::new(config), name).map_err(|e| e.to_string())?;
        let mut tcp = tcp;
        while conn.is_handshaking() {
            conn.complete_io(&mut tcp).map_err(|e| e.to_string())?;
        }
        Ok(StreamOwned::new(conn, tcp))
    }
}

/// The body of a response: `Content-Length` bytes, chunks, or everything
/// until the server closes.  A body that ends early is an error, so a
/// truncated image is never mistaken for a whole one.
enum Body {
    Length(u64),
    Chunked { left: u64, done: bool },
    UntilClose,
}

/// Status, headers (lowercase names), and the body stream.
type Answer = (u16, Vec<(String, String)>, Response);

struct Response {
    stream: BufReader<Box<dyn Stream>>,
    body: Body,
}

impl Read for Response {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use std::io::{Error, ErrorKind};
        let short = || Error::new(ErrorKind::UnexpectedEof, "the download ended early");
        match &mut self.body {
            Body::UntilClose => self.stream.read(buf),
            Body::Length(left) => {
                if *left == 0 || buf.is_empty() {
                    return Ok(0);
                }
                let want = buf.len().min(*left as usize);
                let n = self.stream.read(&mut buf[..want])?;
                if n == 0 {
                    return Err(short());
                }
                *left -= n as u64;
                Ok(n)
            }
            Body::Chunked { left, done } => {
                if *done || buf.is_empty() {
                    return Ok(0);
                }
                if *left == 0 {
                    let mut line = String::new();
                    self.stream.read_line(&mut line)?;
                    if line.trim().is_empty() {
                        // The CRLF that ends the previous chunk.
                        line.clear();
                        self.stream.read_line(&mut line)?;
                    }
                    let size = line.trim().split(';').next().unwrap_or("");
                    let size = u64::from_str_radix(size, 16).map_err(|_| short())?;
                    if size == 0 {
                        *done = true;
                        return Ok(0);
                    }
                    *left = size;
                }
                let want = buf.len().min(*left as usize);
                let n = self.stream.read(&mut buf[..want])?;
                if n == 0 {
                    return Err(short());
                }
                *left -= n as u64;
                Ok(n)
            }
        }
    }
}

impl HttpSource {
    fn get(&self, url: &str, user_agent: &str) -> Result<Answer, FetchError> {
        let u = parse(url).map_err(err)?;
        let addr = match u.host.parse::<std::net::IpAddr>() {
            Ok(ip) => std::net::SocketAddr::new(ip, u.port),
            Err(_) => (self.lookup)(&u.host, u.port).map_err(|e| err(format!("cannot resolve {}: {e}", u.host)))?,
        };
        let tcp = TcpStream::connect_timeout(&addr, self.timeout).map_err(|e| err(format!("cannot connect to {addr}: {e}")))?;
        let _ = tcp.set_read_timeout(Some(self.timeout));
        let _ = tcp.set_write_timeout(Some(self.timeout));
        let mut stream: Box<dyn Stream> = if u.tls {
            #[cfg(feature = "https")]
            {
                let trust = self.trust.as_ref().map_or(Trust::Any, |policy| policy());
                Box::new(tls::wrap(&u.host, tcp, &trust).map_err(|e| err(format!("TLS handshake with {} failed: {e}", u.host)))?)
            }
            #[cfg(not(feature = "https"))]
            {
                return Err(err("https is not built in"));
            }
        } else {
            Box::new(tcp)
        };
        let host = if u.port == if u.tls { 443 } else { 80 } { u.host.clone() } else { format!("{}:{}", u.host, u.port) };
        let request = format!(
            "GET {} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: {user_agent}\r\nAccept: */*\r\nConnection: close\r\n\r\n",
            u.path
        );
        stream.write_all(request.as_bytes()).map_err(|e| err(format!("cannot send the request: {e}")))?;
        let mut stream = BufReader::with_capacity(16 * 1024, stream);
        let mut line = String::new();
        stream.read_line(&mut line).map_err(|e| err(format!("no response: {e}")))?;
        let status: u16 = line
            .split_whitespace()
            .nth(1)
            .and_then(|c| c.parse().ok())
            .ok_or_else(|| err(format!("not an HTTP response: {}", line.trim())))?;
        let mut headers = Vec::new();
        loop {
            line.clear();
            let n = stream.read_line(&mut line).map_err(|e| err(format!("bad response headers: {e}")))?;
            if n == 0 || line.trim().is_empty() {
                break;
            }
            if let Some((k, v)) = line.split_once(':') {
                headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
            }
        }
        let get = |k: &str| headers.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        let body = if get("transfer-encoding").is_some_and(|t| t.to_ascii_lowercase().contains("chunked")) {
            Body::Chunked { left: 0, done: false }
        } else if let Some(n) = get("content-length").and_then(|n| n.parse().ok()) {
            Body::Length(n)
        } else {
            Body::UntilClose
        };
        Ok((status, headers, Response { stream, body }))
    }
}

impl FirmwareSource for HttpSource {
    fn open(&self, url: &str, user_agent: &str) -> Result<Box<dyn Read + Send>, FetchError> {
        let mut url = url.to_string();
        for _ in 0..=self.max_redirects {
            let (status, headers, response) = self.get(&url, user_agent)?;
            match status {
                200 => return Ok(Box::new(response)),
                301 | 302 | 303 | 307 | 308 => {
                    let to = headers.iter().find(|(k, _)| k == "location").map(|(_, v)| v.clone());
                    let Some(to) = to else { return Err(err("redirect without a Location")) };
                    url = if to.starts_with("http://") || to.starts_with("https://") {
                        to
                    } else {
                        // A path on the same server.
                        let end = url.find("://").map(|i| i + 3).and_then(|i| url[i..].find('/').map(|j| i + j));
                        format!("{}{}{to}", &url[..end.unwrap_or(url.len())], if to.starts_with('/') { "" } else { "/" })
                    };
                }
                _ => return Err(FetchError { status: Some(status), message: format!("{url}: HTTP {status}") }),
            }
        }
        Err(err("too many redirects"))
    }
}
