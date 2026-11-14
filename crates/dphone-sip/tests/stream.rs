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
    let verify = TlsConfig { roots_pem: CA.to_vec(), verify: true };
    assert!(registered(Transport::Tls, port, verify).await, "verified TLS registration");
    let reg = wait("REGISTER", || log.lock().unwrap().first().cloned()).await;
    assert!(reg.header("Via").unwrap().starts_with("SIP/2.0/TLS "), "{:?}", reg.header("Via"));
    assert!(reg.header("Contact").unwrap().to_ascii_lowercase().contains(";transport=tls"), "{:?}", reg.header("Contact"));

    // Stock's default: no certificate check.
    log.lock().unwrap().clear();
    assert!(registered(Transport::Tls, port, TlsConfig::default()).await, "unverified TLS registration");

    // Checking with no roots configured cannot connect.
    assert!(dphone_sip::tls::client_config(&TlsConfig { roots_pem: Vec::new(), verify: true }).is_err());
}
