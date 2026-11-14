//! The download client against a local HTTP server: streaming, redirects,
//! statuses, and bodies that end early; and against an HTTPS server: the
//! certificate check.

use dphone_dpma::fetch::HttpSource;
use dphone_dpma::firmware::FirmwareSource;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

/// Serves a few fixed paths; records each request's path and User-Agent.
type Log = Arc<Mutex<Vec<(String, String)>>>;

fn server(body: Vec<u8>) -> (u16, Log) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let log = Arc::new(Mutex::new(Vec::new()));
    let seen = log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
            let mut ua = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                    break;
                }
                if let Some(v) = line.strip_prefix("User-Agent: ") {
                    ua = v.trim().to_string();
                }
            }
            seen.lock().unwrap().push((path.clone(), ua));
            let head = |status: &str, extra: &str, len: usize| {
                format!("HTTP/1.1 {status}\r\n{extra}Content-Length: {len}\r\nConnection: close\r\n\r\n")
            };
            // The client may hang up early (the header peek): ignore write errors.
            let _ = match path.as_str() {
                "/fw.eff" => stream.write_all(head("200 OK", "", body.len()).as_bytes()).and_then(|_| stream.write_all(&body)),
                "/moved" => stream.write_all(head("302 Found", "Location: /fw.eff\r\n", 0).as_bytes()),
                "/busy" => stream.write_all(head("503 Service Unavailable", "", 0).as_bytes()),
                "/short" => stream
                    .write_all(head("200 OK", "", body.len()).as_bytes())
                    .and_then(|_| stream.write_all(&body[..body.len() / 2])),
                "/chunked" => {
                    let mut out = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
                    for chunk in body.chunks(1000) {
                        out.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
                        out.extend_from_slice(chunk);
                        out.extend_from_slice(b"\r\n");
                    }
                    out.extend_from_slice(b"0\r\n\r\n");
                    stream.write_all(&out)
                }
                _ => stream.write_all(head("404 Not Found", "", 0).as_bytes()),
            };
        }
    });
    (port, log)
}

#[test]
fn streams_redirects_and_reports_failures() {
    let body: Vec<u8> = (0..300_000u32).map(|i| (i * 7 + i / 251) as u8).collect();
    let (port, log) = server(body.clone());
    let source = HttpSource::new();
    let url = |path: &str| format!("http://127.0.0.1:{port}{path}");
    let read_all = |path: &str| {
        let mut out = Vec::new();
        source.open(&url(path), "Digium-D60/3_0_0").map_err(|e| e.message)?.read_to_end(&mut out).map_err(|e| e.to_string())?;
        Ok::<_, String>(out)
    };

    assert_eq!(read_all("/fw.eff").unwrap(), body);
    assert_eq!(log.lock().unwrap()[0], ("/fw.eff".to_string(), "Digium-D60/3_0_0".to_string()));
    // curl -L: redirects are followed.
    assert_eq!(read_all("/moved").unwrap(), body);
    assert_eq!(read_all("/chunked").unwrap(), body);
    // Statuses come back as errors, with the code for the 503 retry.
    assert_eq!(source.open(&url("/busy"), "x").err().unwrap().status, Some(503));
    assert_eq!(source.open(&url("/nothing"), "x").err().unwrap().status, Some(404));
    // A body shorter than announced is an error, never a short image.
    let short = read_all("/short");
    assert!(short.is_err(), "a truncated download must not look complete");
    // Reading only the start and dropping the stream is clean, and the
    // server keeps serving afterwards.
    let mut head = [0u8; 700];
    source.open(&url("/fw.eff"), "x").unwrap().read_exact(&mut head).unwrap();
    assert_eq!(&head[..], &body[..700]);
    assert_eq!(read_all("/fw.eff").unwrap(), body);
    // Unsupported schemes and dead servers are plain errors.
    assert!(source.open("ftp://127.0.0.1/fw.eff", "x").is_err());
    assert!(source.open("http://127.0.0.1:1/fw.eff", "x").is_err());
}

/// An HTTPS server presenting the test certificate (`pbx.test`, 127.0.0.1)
/// that answers every request with `ok`.
#[cfg(feature = "https")]
fn https_server() -> u16 {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let certs: Vec<_> =
        CertificateDer::pem_slice_iter(include_bytes!("../../dphone-sip/tests/data/test-cert.pem")).flatten().collect();
    let key = PrivateKeyDer::from_pem_slice(include_bytes!("../../dphone-sip/tests/data/test-key.pem")).unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls_rustcrypto::provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap();
    let config = Arc::new(config);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for tcp in listener.incoming() {
            let Ok(tcp) = tcp else { break };
            let conn = rustls::ServerConnection::new(config.clone()).unwrap();
            let mut tls = rustls::StreamOwned::new(conn, tcp);
            // A client that refuses the certificate ends the handshake.
            let mut reader = BufReader::new(&mut tls);
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|n| n > 0) && !line.trim().is_empty() {
                line.clear();
            }
            let _ = tls.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
            tls.conn.send_close_notify();
            let _ = tls.flush();
        }
    });
    port
}

#[cfg(feature = "https")]
#[test]
fn https_servers_are_checked_as_the_trust_policy_says() {
    use dphone_dpma::fetch::Trust;
    const CA: &[u8] = include_bytes!("../../dphone-sip/tests/data/test-ca.pem");
    const OTHER_CA: &[u8] = include_bytes!("../../dphone-sip/tests/data/wild-ca.pem");
    let port = https_server();
    let get = |source: &HttpSource, host: &str| {
        let mut out = Vec::new();
        source
            .open(&format!("https://{host}:{port}/list.xml"), "x")
            .map_err(|e| e.message)?
            .read_to_end(&mut out)
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(out)
    };
    // Every name resolves to the server.
    let with = |trust: Trust| {
        HttpSource::with_lookup(Box::new(|_, port| Ok(([127, 0, 0, 1], port).into()))).with_trust(Box::new(move || trust.clone()))
    };
    // No policy (firmware, as stock's `curl --insecure`), and a policy that
    // says so: any certificate.
    assert_eq!(get(&HttpSource::new(), "127.0.0.1").unwrap(), b"ok");
    assert_eq!(get(&with(Trust::Any), "127.0.0.1").unwrap(), b"ok");

    // Checked: the issuer must be among the roots.  The roots are a bundle
    // as the phone builds it: other authorities, text between the blocks,
    // and the issuer twice.
    let mut bundle = b"# base bundle\n\nSome other CA\n".to_vec();
    bundle.extend_from_slice(OTHER_CA);
    bundle.extend_from_slice(b"\nOpen DPhone test CA\n");
    bundle.extend_from_slice(CA);
    bundle.extend_from_slice(b"\n");
    bundle.extend_from_slice(CA);
    assert_eq!(get(&with(Trust::Roots(bundle.clone())), "127.0.0.1").unwrap(), b"ok");
    let e = get(&with(Trust::Roots(OTHER_CA.to_vec())), "127.0.0.1").unwrap_err();
    assert!(e.contains("UnknownIssuer"), "{e}");
    // The name must match as well.
    assert_eq!(get(&with(Trust::Roots(bundle.clone())), "pbx.test").unwrap(), b"ok");
    let e = get(&with(Trust::Roots(bundle)), "other.test").unwrap_err();
    assert!(e.contains("not valid for name"), "{e}");
    // Checking without roots refuses every server.
    let e = get(&with(Trust::Roots(Vec::new())), "127.0.0.1").unwrap_err();
    assert!(e.contains("no trusted roots"), "{e}");
}
