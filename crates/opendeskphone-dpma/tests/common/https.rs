//! A local HTTPS server for the download client's tests.

/// An HTTPS server presenting the test certificate (names `pbx.test` and
/// 127.0.0.1, issued by `opendeskphone-sip/tests/data/test-ca.pem`) that
/// answers every request with `body`.  Returns its port.
pub fn https_server(body: Vec<u8>) -> u16 {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use std::io::{BufRead, BufReader, Write};
    let certs: Vec<_> =
        CertificateDer::pem_slice_iter(include_bytes!("../../../opendeskphone-sip/tests/data/test-cert.pem")).flatten().collect();
    let key = PrivateKeyDer::from_pem_slice(include_bytes!("../../../opendeskphone-sip/tests/data/test-key.pem")).unwrap();
    let config = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(rustls_rustcrypto::provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap();
    let config = std::sync::Arc::new(config);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for tcp in listener.incoming() {
            let Ok(tcp) = tcp else { break };
            let conn = rustls::ServerConnection::new(config.clone()).unwrap();
            let mut tls = rustls::StreamOwned::new(conn, tcp);
            // A client that refuses the certificate ends the handshake, and
            // one may hang up before the whole body: both are ignored.
            let mut reader = BufReader::new(&mut tls);
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|n| n > 0) && !line.trim().is_empty() {
                line.clear();
            }
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = tls.write_all(head.as_bytes()).and_then(|_| tls.write_all(&body));
            tls.conn.send_close_notify();
            let _ = tls.flush();
        }
    });
    port
}
