//! Test fixtures: a DPMA server built from `dpma-proto`'s server-side
//! pieces (certificate verification, password encryption, chunked
//! encoding), an in-memory transport, and in-memory storage.
#![allow(dead_code)]

use dphone_dpma::{ConfigSink, Message, Storage, Transport};
use dpma_proto::message::{Outgoing, SendContext, Sender, Transactions};
use dpma_proto::{cert, crypto, errors, headers, types};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

struct ServerSession {
    password: Vec<u8>,
    chain: bool,
}

pub struct FakeServer {
    ca: cert::CaStore,
    sessions: HashMap<Vec<u8>, ServerSession>,
    next_session: u32,
    transactions: Transactions<Vec<u8>>,
    clock: i64,
    pub token_reqs: String,
    pub userlist_reqs: String,
    /// extension, name, PIN
    pub users: Vec<(String, String, String)>,
    pub config: Vec<u8>,
    /// Sent as `X-Digium-AppServer-MD5` on Verify and Config responses.
    pub md5: Option<String>,
    /// Request types in arrival order.
    pub log: Vec<String>,
    pub user_agent: String,
    tokens: Vec<Vec<u8>>,
}

impl FakeServer {
    pub fn new() -> FakeServer {
        FakeServer {
            ca: cert::CaStore::builtin(),
            sessions: HashMap::new(),
            next_session: 1000,
            transactions: Transactions::new(Sender::Phone),
            clock: 0,
            token_reqs: "mac,user,pass".into(),
            userlist_reqs: "none".into(),
            users: vec![("100".into(), "Desk".into(), "1234".into())],
            config: Vec::new(),
            md5: None,
            log: Vec::new(),
            user_agent: String::new(),
            tokens: Vec::new(),
        }
    }

    pub fn count(&self, request_type: &str) -> usize {
        self.log.iter().filter(|t| *t == request_type).count()
    }

    pub fn forget_sessions(&mut self) {
        self.sessions.clear();
    }

    fn ctx<'a>(s: &'a ServerSession, id: &'a [u8]) -> SendContext<'a> {
        SendContext { password: &s.password, session_id: id, origination_network: b"", chain_certificate: s.chain }
    }

    fn newest(&self) -> Option<(&Vec<u8>, &ServerSession)> {
        self.sessions.iter().max_by_key(|(id, _)| id.to_vec())
    }

    /// A push to the newest session.
    pub fn event(&self, name: &str, body: &[u8]) -> Vec<Message> {
        let Some((id, s)) = self.newest() else { return Vec::new() };
        let out = Outgoing { event_name: name.into(), body: body.to_vec(), ..Default::default() };
        out.encode(&Self::ctx(s, id)).unwrap()
    }

    /// `digium.assignment.update` as it goes out: ID `bootcReset`, -398.
    pub fn reassign(&self) -> Vec<Message> {
        let Some((id, s)) = self.newest() else { return Vec::new() };
        let out = Outgoing { id: b"bootcReset".to_vec(), error: errors::msg::SESSION_TIMEOUT, ..Default::default() };
        out.encode(&Self::ctx(s, id)).unwrap()
    }

    /// Takes one MESSAGE from a phone; returns the MESSAGEs to send back.
    pub fn handle(&mut self, m: Message) -> Vec<Message> {
        self.clock += 1;
        let raw = m.clone();
        let sid = m.header(headers::SESSION).unwrap_or_default().to_vec();
        let Some((sid, parts)) = self.transactions.add(m, (), self.clock, sid) else { return Vec::new() };
        let password = self.sessions.get(&sid).map(|s| s.password.clone());
        let id = raw.header(headers::APPSERVER_ID).unwrap_or_default().to_vec();
        let raw_type = String::from_utf8_lossy(raw.header(headers::REQUEST_TYPE).unwrap_or_default()).into_owned();
        let request = match parts.decode(Sender::Phone, password.as_deref()) {
            Ok(r) if raw_type.eq_ignore_ascii_case(types::HANDSHAKE) || password.is_some() || !types::needs_session(&raw_type) => r,
            _ => {
                // Unknown session: "Session Timeout" under a session the
                // phone does not share, as the real server sends it.
                self.log.push(format!("{raw_type}(no session)"));
                let stranger = ServerSession { password: crypto::new_common_password().unwrap(), chain: false };
                let out = Outgoing {
                    error: errors::msg::SESSION_TIMEOUT,
                    id,
                    response_type: types::response_type_for(&raw_type).unwrap_or("Error").into(),
                    ..Default::default()
                };
                return out.encode(&Self::ctx(&stranger, b"9999")).unwrap();
            }
        };
        let request_type = String::from_utf8_lossy(&request.request_type).into_owned();
        self.log.push(request_type.clone());
        if let Some(ua) = request.first.header(headers::DIGIUM_USER_AGENT) {
            self.user_agent = String::from_utf8_lossy(ua).into_owned();
        }
        let mut reply = Outgoing::new();
        reply.id = request.id.clone();
        reply.response_type = types::response_type_for(&request_type).unwrap_or("Error").into();

        if request_type.eq_ignore_ascii_case(types::HANDSHAKE) {
            let certificates = cert::restore_pem_newlines(&request.body);
            let info = cert::ClientInfo { contact: b"test", mac: Some(&request.mac), user_agent: b"" };
            let Ok(client) = cert::verify_client_chain(&self.ca, certificates, &info) else {
                // A refused certificate gets no answer.
                self.log.push("Handshake(refused)".into());
                return Vec::new();
            };
            let key = client.public_key.as_ref().expect("an RSA key");
            let password = crypto::new_common_password().unwrap();
            reply.password = crypto::encrypt_common_password(key, &password).unwrap().into_bytes();
            reply.token_auth_requirements = self.token_reqs.clone().into_bytes();
            reply.userlist_auth_requirements = self.userlist_reqs.clone().into_bytes();
            reply.append(cert::DEFAULT_APPSERVER_CERTIFICATE);
            let sid = self.next_session.to_string().into_bytes();
            self.next_session += 1;
            let session = ServerSession { password, chain: client.intermediate_serial != 0 };
            let out = reply.encode(&Self::ctx(&session, &sid)).unwrap();
            self.sessions.insert(sid, session);
            return out;
        }

        let token_ok = self.tokens.contains(&request.config_token);
        match request_type.as_str() {
            types::USERLIST_REQUEST => {
                let users: Vec<String> = self
                    .users
                    .iter()
                    .map(|(e, n, _)| format!("{{\"extension\":\"{e}\",\"name\":\"{n}\",\"assigned\":\"0\"}}"))
                    .collect();
                reply.append_str(&format!("{{\"users\":[{}]}}", users.join(",")));
            }
            types::TOKEN_REQUEST => {
                let user = String::from_utf8_lossy(&request.username).into_owned();
                let by_mac = !self.token_reqs.contains("user");
                match self.users.iter().find(|(e, _, _)| *e == user || by_mac) {
                    None => reply.error = errors::msg::USER_NOT_FOUND,
                    Some((_, _, pin)) if self.token_reqs.contains("pass") && request.password != pin.as_bytes() => {
                        reply.error = errors::msg::INVALID_PASSWORD
                    }
                    Some((ext, _, _)) => {
                        let token = format!("tok-{ext}-{}", self.tokens.len()).into_bytes();
                        self.tokens.push(token.clone());
                        reply.config_token = token;
                        reply.username = ext.clone().into_bytes();
                    }
                }
            }
            types::VERIFY_REQUEST => {
                if token_ok {
                    reply.md5 = self.md5.clone().unwrap_or_default().into_bytes();
                } else {
                    reply.error = errors::msg::INVALID_TOKEN;
                }
            }
            types::CONFIG_REQUEST => {
                if token_ok {
                    reply.body = self.config.clone();
                    reply.md5 = self.md5.clone().unwrap_or_default().into_bytes();
                } else {
                    reply.error = errors::msg::INVALID_TOKEN;
                }
            }
            types::FILE_REQUEST => {
                if request.body.starts_with(b"file://ringtone") {
                    reply.append(b"RIFFdata");
                } else {
                    reply.error = errors::msg::FILE_NOT_FOUND;
                }
            }
            types::ASTERISK_REQUEST => {
                let text = String::from_utf8_lossy(&request.body).into_owned();
                let authed = text.contains(&format!("{}100:1234", headers::DIGEST_AUTH_MARKER));
                let method = text.contains("switchvox.users.voicemail.getList");
                reply.http_status = b"200".to_vec();
                reply.append_str("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n");
                reply.append_str(if authed && method {
                    "{\"response\":{\"result\":{\"messages\":[]}}}"
                } else {
                    "{\"response\":{\"errors\":{\"error\":{\"code\":99999}}}}"
                });
            }
            _ => {}
        }
        let session = self.sessions.get(&sid).expect("checked above");
        reply.encode(&Self::ctx(session, &sid)).unwrap()
    }
}

/// Phone-side transport wired straight to a [`FakeServer`].
pub struct Mem {
    pub server: Arc<Mutex<FakeServer>>,
    pub to_phone: mpsc::UnboundedSender<Message>,
    /// While set, MESSAGEs go nowhere (an unreachable server).
    pub down: Mutex<bool>,
}

#[async_trait::async_trait]
impl Transport for Mem {
    async fn send(&self, _server_uri: &str, message: &Message) -> Result<(), String> {
        if *self.down.lock().unwrap() {
            return Err("unreachable".into());
        }
        let out = self.server.lock().unwrap().handle(message.clone());
        for m in out {
            let _ = self.to_phone.send(m);
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct MemStorage(pub Mutex<HashMap<String, Vec<u8>>>);

impl Storage for MemStorage {
    fn read(&self, name: &str) -> Option<Vec<u8>> {
        self.0.lock().unwrap().get(name).cloned()
    }
    fn write(&self, name: &str, data: &[u8]) -> Result<(), String> {
        self.0.lock().unwrap().insert(name.to_string(), data.to_vec());
        Ok(())
    }
    fn remove(&self, name: &str) {
        self.0.lock().unwrap().remove(name);
    }
}

#[derive(Default)]
pub struct MemSink {
    pub configurator: Mutex<Option<Vec<u8>>>,
    pub md5: Mutex<Option<String>>,
    pub applied: Mutex<Vec<Vec<u8>>>,
}

impl ConfigSink for MemSink {
    fn configurator(&self) -> Option<Vec<u8>> {
        self.configurator.lock().unwrap().clone()
    }
    fn write_configurator(&self, xml: &[u8]) -> Result<(), String> {
        *self.configurator.lock().unwrap() = Some(xml.to_vec());
        Ok(())
    }
    fn cached_md5(&self) -> Option<String> {
        self.md5.lock().unwrap().clone()
    }
    fn apply(&self, fetched_config: &[u8], md5: &str) -> Result<(), String> {
        self.applied.lock().unwrap().push(fetched_config.to_vec());
        *self.md5.lock().unwrap() = Some(md5.to_string());
        Ok(())
    }
    fn forget(&self) -> Result<(), String> {
        *self.configurator.lock().unwrap() = None;
        *self.md5.lock().unwrap() = None;
        Ok(())
    }
}

pub async fn wait<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    for _ in 0..400 {
        if let Some(v) = f() {
            return v;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for {what}");
}
