//! Registration over TCP and over TLS (rustls with the RustCrypto
//! provider on both ends; the server uses a test CA and certificate made
//! for this test).
#![cfg(feature = "tls")]

mod common;

use common::{base_config, phone_with, wait, Msg};
use dphone_sip::{AccountConfig, TlsConfig};
use dphone_state::{AccountId, Accounts, Registration, Transport};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;

const CERT: &[u8] = include_bytes!("data/test-cert.pem");
const KEY: &[u8] = include_bytes!("data/test-key.pem");
const CA: &[u8] = include_bytes!("data/test-ca.pem");

/// Answers every request on a stream with 200 and logs it.
async fn serve<S: AsyncRead + AsyncWrite + Unpin>(mut s: S, log: Arc<Mutex<Vec<Msg>>>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    while let Ok(n) = s.read(&mut chunk).await {
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        while let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end + 4]).into_owned();
            let len: usize = head
                .lines()
                .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0)))
                .unwrap_or(0);
            if buf.len() < end + 4 + len {
                break;
            }
            let text = String::from_utf8_lossy(&buf[..end + 4 + len]).into_owned();
            buf.drain(..end + 4 + len);
            let Some(m) = Msg::parse(&text, "127.0.0.1:1".parse().unwrap()) else { continue };
            log.lock().unwrap().push(m.clone());
            if m.is_request() {
                let extra = m.header("Contact").map(|c| vec![("Contact", c.to_string())]).unwrap_or_default();
                if s.write_all(m.reply(200, "OK", &extra, "").as_bytes()).await.is_err() {
                    return;
                }
            }
        }
    }
}

async fn registered(transport: Transport, port: u16, tls: TlsConfig) -> bool {
    let mut cfg = base_config();
    cfg.tls = tls;
    let mut a = AccountConfig::new(AccountId(0), "100", "secret", "127.0.0.1");
    a.primary.port = port;
    a.primary.transport = transport;
    a.primary.retry_secs = 60;
    a.mwi_subscribe = false;
    cfg.accounts.push(a);
    let p = phone_with("100", cfg, 4000).await;
    for _ in 0..150 {
        match p.store.get::<Accounts>().get(AccountId(0)).map(|a| a.registration.clone()) {
            Some(Registration::Registered { .. }) => return true,
            Some(Registration::Failed { code, reason, .. }) => {
                eprintln!("registration failed: {code} {reason}");
                return false;
            }
            _ => tokio::time::sleep(std::time::Duration::from_millis(20)).await,
        }
    }
    false
}

#[tokio::test(flavor = "current_thread")]
async fn register_over_tcp() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let log = Arc::new(Mutex::new(Vec::new()));
    let l = log.clone();
    tokio::spawn(async move {
        while let Ok((s, _)) = listener.accept().await {
            tokio::spawn(serve(s, l.clone()));
        }
    });
    assert!(registered(Transport::Tcp, port, TlsConfig::default()).await);
    let reg = wait("REGISTER", || log.lock().unwrap().first().cloned()).await;
    assert!(reg.header("Via").unwrap().starts_with("SIP/2.0/TCP "), "{:?}", reg.header("Via"));
    assert!(reg.header("Contact").unwrap().to_ascii_lowercase().contains(";transport=tcp"), "{:?}", reg.header("Contact"));
}

#[tokio::test(flavor = "current_thread")]
async fn register_over_tls() {
    let certs: Vec<_> = rustls_pemfile::certs(&mut &CERT[..]).flatten().collect();
    let key = rustls_pemfile::private_key(&mut &KEY[..]).unwrap().unwrap();
    let server = rustls::ServerConfig::builder_with_provider(Arc::new(rustls_rustcrypto::provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let log = Arc::new(Mutex::new(Vec::new()));
    let l = log.clone();
    tokio::spawn(async move {
        while let Ok((s, _)) = listener.accept().await {
            let (acceptor, l) = (acceptor.clone(), l.clone());
            tokio::spawn(async move {
                if let Ok(tls) = acceptor.accept(s).await {
                    serve(tls, l).await;
                }
            });
        }
    });

    // Checking the certificate against the right root succeeds.
    let verify = TlsConfig { roots_pem: CA.to_vec(), verify: true, ..Default::default() };
    assert!(registered(Transport::Tls, port, verify).await, "verified TLS registration");
    let reg = wait("REGISTER", || log.lock().unwrap().first().cloned()).await;
    assert!(reg.header("Via").unwrap().starts_with("SIP/2.0/TLS "), "{:?}", reg.header("Via"));
    assert!(reg.header("Contact").unwrap().to_ascii_lowercase().contains(";transport=tls"), "{:?}", reg.header("Contact"));

    // Stock's default: no certificate check.
    log.lock().unwrap().clear();
    assert!(registered(Transport::Tls, port, TlsConfig::default()).await, "unverified TLS registration");

    // Checking with no roots configured cannot connect.
    assert!(dphone_sip::tls::client_config(&TlsConfig { roots_pem: Vec::new(), verify: true, ..Default::default() }).is_err());
}

/// A port nothing listens on right now.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

const OPTIONS: &str = "OPTIONS sip:100@127.0.0.1 SIP/2.0\r\nVia: SIP/2.0/TCP 127.0.0.1:9;branch=z9hG4bKin1\r\nMax-Forwards: 70\r\n\
    From: <sip:pbx@127.0.0.1>;tag=a\r\nTo: <sip:100@127.0.0.1>\r\nCall-ID: listener-test\r\nCSeq: 1 OPTIONS\r\nContent-Length: 0\r\n\r\n";

/// Sends OPTIONS on a fresh connection and returns the response's start.
async fn ask<S: AsyncRead + AsyncWrite + Unpin>(mut s: S) -> String {
    s.write_all(OPTIONS.as_bytes()).await.unwrap();
    let mut buf = vec![0u8; 4096];
    let n = tokio::time::timeout(std::time::Duration::from_secs(5), s.read(&mut buf)).await.expect("an answer").unwrap();
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

/// A registrar on TCP for the phone to register with (and so to have a
/// TCP or TLS account at all).
async fn stream_phone(transport: Transport, configure: impl FnOnce(&mut dphone_sip::SipConfig)) -> common::Phone {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((s, _)) = listener.accept().await {
            tokio::spawn(serve(s, Arc::new(Mutex::new(Vec::new()))));
        }
    });
    let mut cfg = base_config();
    let mut a = AccountConfig::new(AccountId(0), "100", "secret", "127.0.0.1");
    a.primary.port = port;
    a.primary.transport = transport;
    a.mwi_subscribe = false;
    cfg.accounts.push(a);
    configure(&mut cfg);
    phone_with("100", cfg, 4000).await
}

#[tokio::test(flavor = "current_thread")]
async fn accepts_connections_over_tcp() {
    let port = free_port();
    let p = stream_phone(Transport::Tcp, |cfg| cfg.tcp_port = Some(port)).await;
    let _ = &p;
    // The PBX's connection is gone; it opens one to the phone.
    let stream = wait("the TCP listener", || std::net::TcpStream::connect(("127.0.0.1", port)).ok()).await;
    stream.set_nonblocking(true).unwrap();
    let answer = ask(tokio::net::TcpStream::from_std(stream).unwrap()).await;
    assert!(answer.starts_with("SIP/2.0 200"), "{answer}");
    assert!(answer.contains("Call-ID: listener-test"));
    // The Contact we register names the listening port.
    wait("registration", || {
        matches!(p.store.get::<Accounts>().get(AccountId(0)).map(|a| a.registration.clone()), Some(Registration::Registered { .. }))
            .then_some(())
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn accepts_connections_over_tls() {
    let port = free_port();
    let cert = rustls_pemfile::certs(&mut &CERT[..]).next().unwrap().unwrap().to_vec();
    let key = match rustls_pemfile::private_key(&mut &KEY[..]).unwrap().unwrap() {
        rustls::pki_types::PrivateKeyDer::Pkcs8(k) => k.secret_pkcs8_der().to_vec(),
        other => panic!("the test key is not PKCS#8: {other:?}"),
    };
    // Without a certificate there is no TLS listener, and no crash.
    let quiet = free_port();
    let _p0 = stream_phone(Transport::Tcp, |cfg| cfg.tls_port = Some(quiet)).await;
    let _p = stream_phone(Transport::Tcp, |cfg| {
        // A TLS account is what turns the listener on; the registrar here
        // speaks plain TCP, so registration itself fails, which is fine.
        cfg.accounts[0].primary.transport = Transport::Tls;
        cfg.tls_port = Some(port);
        cfg.tls.server_cert_der = cert;
        cfg.tls.server_key_der = key;
    })
    .await;
    let tcp = wait("the TLS listener", || std::net::TcpStream::connect(("127.0.0.1", port)).ok()).await;
    tcp.set_nonblocking(true).unwrap();
    let mut roots = rustls::RootCertStore::empty();
    for c in rustls_pemfile::certs(&mut &CA[..]).flatten() {
        roots.add(c).unwrap();
    }
    let client = rustls::ClientConfig::builder_with_provider(Arc::new(rustls_rustcrypto::provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from("pbx.test").unwrap();
    let tls = tokio_rustls::TlsConnector::from(Arc::new(client))
        .connect(name, tokio::net::TcpStream::from_std(tcp).unwrap())
        .await
        .expect("TLS handshake with the phone");
    let answer = ask(tls).await;
    assert!(answer.starts_with("SIP/2.0 200"), "{answer}");
    assert!(std::net::TcpStream::connect(("127.0.0.1", quiet)).is_err(), "no listener without a certificate");
}

/// Installs a marker that sets a DSCP and records each socket it saw as
/// (local port, peer port or 0).
type Marks = Arc<Mutex<Vec<(u16, u16)>>>;

fn marks() -> Marks {
    static LOG: std::sync::OnceLock<Marks> = std::sync::OnceLock::new();
    LOG.get_or_init(|| {
        let log = Arc::new(Mutex::new(Vec::new()));
        let seen = log.clone();
        dphone_sip::set_socket_marker(Arc::new(move |fd| {
            let sock = socket2::SockRef::from(&fd);
            sock.set_tos_v4(26 << 2).expect("DSCP can be set");
            let port = |a: std::io::Result<socket2::SockAddr>| a.ok().and_then(|a| a.as_socket()).map(|a| a.port()).unwrap_or(0);
            seen.lock().unwrap().push((port(sock.local_addr()), port(sock.peer_addr())));
        }));
        log
    })
    .clone()
}

#[tokio::test(flavor = "current_thread")]
async fn sip_sockets_are_marked() {
    let log = marks();
    // A TCP registrar and a TLS registrar.
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tcp_port = tcp.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((s, _)) = tcp.accept().await {
            tokio::spawn(serve(s, Arc::new(Mutex::new(Vec::new()))));
        }
    });
    assert!(registered(Transport::Tcp, tcp_port, TlsConfig::default()).await);
    let seen = log.lock().unwrap().clone();
    // The UDP socket (no peer) and our TCP connection to the registrar.
    assert!(seen.iter().any(|(local, peer)| *local != 0 && *peer == 0), "the UDP socket: {seen:?}");
    assert!(seen.iter().any(|(_, peer)| *peer == tcp_port), "the TCP connection to the registrar: {seen:?}");
    // One connection, used for the challenge-free REGISTER: not one per request.
    assert_eq!(seen.iter().filter(|(_, peer)| *peer == tcp_port).count(), 1);

    // TLS toward a registrar: the connector marks before the handshake.
    let certs: Vec<_> = rustls_pemfile::certs(&mut &CERT[..]).flatten().collect();
    let key = rustls_pemfile::private_key(&mut &KEY[..]).unwrap().unwrap();
    let server = rustls::ServerConfig::builder_with_provider(Arc::new(rustls_rustcrypto::provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
    let tls = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tls_port = tls.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((s, _)) = tls.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                if let Ok(t) = acceptor.accept(s).await {
                    serve(t, Arc::new(Mutex::new(Vec::new()))).await;
                }
            });
        }
    });
    assert!(registered(Transport::Tls, tls_port, TlsConfig::default()).await);
    assert!(log.lock().unwrap().iter().any(|(_, peer)| *peer == tls_port), "the TLS connection to the registrar");
}
