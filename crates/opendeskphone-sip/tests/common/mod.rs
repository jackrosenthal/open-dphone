//! Shared test fixtures: phones on loopback with a recording media
//! engine, and a hand-written UDP SIP server (deliberately not built on
//! the stack under test).
#![allow(dead_code)]

use opendeskphone_sip::{AccountConfig, NullMedia, SipConfig, SipHandle};
use opendeskphone_state::{AccountId, Model, Store};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::UdpSocket;

pub struct Phone {
    pub store: Store,
    pub handle: SipHandle,
    pub media: Arc<NullMedia>,
    pub user: String,
}

impl Phone {
    pub fn uri(&self) -> String {
        format!("sip:{}@127.0.0.1:{}", self.user, self.handle.local_port())
    }
}

pub fn base_config() -> SipConfig {
    let mut cfg = SipConfig::new(Model::D60, IpAddr::V4(Ipv4Addr::LOCALHOST));
    cfg.local_port = 0;
    cfg.udp_keepalive_secs = 0;
    cfg.ended_linger_secs = 1;
    cfg
}

/// A phone with one unregistered account, for direct calls.
pub async fn phone(user: &str, rtp: u16) -> Phone {
    let mut cfg = base_config();
    let mut a = AccountConfig::new(AccountId(0), user, "pw", "127.0.0.1");
    a.register = false;
    a.mwi_subscribe = false;
    a.display_name = format!("{user} display");
    cfg.accounts.push(a);
    phone_with(user, cfg, rtp).await
}

pub async fn phone_with(user: &str, cfg: SipConfig, rtp: u16) -> Phone {
    let store = Store::new(Model::D60);
    let media = Arc::new(NullMedia::new(
        vec![opendeskphone_state::Codec::Pcmu, opendeskphone_state::Codec::Pcma, opendeskphone_state::Codec::G722],
        rtp,
    ));
    let handle = opendeskphone_sip::start(store.clone(), cfg, media.clone(), None).await.expect("start");
    Phone { store, handle, media, user: user.to_string() }
}

/// Polls until `f` returns `Some`, or panics with `what` after 8 seconds.
pub async fn wait<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    for _ in 0..400 {
        if let Some(v) = f() {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for {what}");
}

/// A received SIP message, split just enough for a test server.
#[derive(Clone, Debug)]
pub struct Msg {
    pub start: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub from: SocketAddr,
}

impl Msg {
    pub fn parse(text: &str, from: SocketAddr) -> Option<Msg> {
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text, ""));
        let mut lines = head.split("\r\n");
        let start = lines.next()?.to_string();
        let headers = lines
            .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
            .collect();
        Some(Msg { start, headers, body: body.to_string(), from })
    }
    pub fn method(&self) -> &str {
        self.start.split(' ').next().unwrap_or("")
    }
    pub fn is_request(&self) -> bool {
        !self.start.starts_with("SIP/2.0")
    }
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
    /// A response that echoes the transaction headers.
    pub fn reply(&self, code: u16, reason: &str, extra: &[(&str, String)], body: &str) -> String {
        let mut s = format!("SIP/2.0 {code} {reason}\r\n");
        for (k, v) in &self.headers {
            let k_l = k.to_ascii_lowercase();
            if k_l == "via" || k_l == "from" || k_l == "call-id" || k_l == "cseq" {
                s.push_str(&format!("{k}: {v}\r\n"));
            } else if k_l == "to" {
                let v = if v.contains("tag=") || code == 100 { v.clone() } else { format!("{v};tag=srv") };
                s.push_str(&format!("To: {v}\r\n"));
            }
        }
        for (k, v) in extra {
            s.push_str(&format!("{k}: {v}\r\n"));
        }
        s.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
        s
    }
}

/// What a [`Server`] does with a request; `None` means no reply.
pub type Handler = Box<dyn FnMut(&Msg) -> Vec<String> + Send>;

/// A UDP SIP responder: every request goes to the handler, whose strings
/// are sent back to the source.  Everything received is logged.
pub struct Server {
    pub addr: SocketAddr,
    pub log: Arc<Mutex<Vec<Msg>>>,
    pub socket: Arc<UdpSocket>,
}

impl Server {
    pub async fn start(mut handler: Handler) -> Server {
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let addr = socket.local_addr().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let (s, l) = (socket.clone(), log.clone());
        tokio::spawn(async move {
            let mut buf = vec![0u8; 16384];
            loop {
                let Ok((n, from)) = s.recv_from(&mut buf).await else { break };
                let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                let Some(msg) = Msg::parse(&text, from) else { continue };
                l.lock().unwrap().push(msg.clone());
                if msg.is_request() {
                    for out in handler(&msg) {
                        let _ = s.send_to(out.as_bytes(), from).await;
                    }
                }
            }
        });
        Server { addr, log, socket }
    }

    pub fn requests(&self, method: &str) -> Vec<Msg> {
        self.log.lock().unwrap().iter().filter(|m| m.is_request() && m.method() == method).cloned().collect()
    }
}
