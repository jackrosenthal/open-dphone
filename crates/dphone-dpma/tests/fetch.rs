//! The firmware download client against a local HTTP server: streaming,
//! redirects, statuses, and bodies that end early.

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
