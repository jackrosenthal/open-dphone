//! Test fixtures: a DPMA server built from `dpma-proto`'s server-side pieces
//! (certificate verification, password encryption, chunked encoding), an
//! in-memory transport, and the client's files in a temporary directory.
#![allow(dead_code)]

use opendeskphone_dpma::resources::{Files, Wanted};
use opendeskphone_dpma::{ClientFiles, ConfigSink, Message, Transport};
use dpma_proto::message::{Outgoing, SendContext, Sender, Transactions};
use dpma_proto::{cert, crypto, errors, headers, types};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

struct ServerSession {
    password: Vec<u8>,
    chain: bool,
    /// Answer everything with an encrypted -398.
    broken: bool,
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
    /// Send the MD5 of the configuration document's bytes there instead,
    /// 32 lowercase hex digits, as Open DPMA does (from dpma-rs 0.3.1 on).
    pub md5_of_document: bool,
    /// Request types in arrival order.
    pub log: Vec<String>,
    pub user_agent: String,
    /// Per phone (named by its extension): the MAC of its last accepted
    /// token request or token presentation, spelled as that request
    /// spelled it.
    pub last_mac: HashMap<String, Vec<u8>>,
    /// `X-Digium-AppServer-MACAddress` of every request, in arrival order.
    pub macs: Vec<Vec<u8>>,
    /// The number of certificates in each Handshake body, in arrival order.
    pub handshake_certificates: Vec<usize>,
    /// Leave a Handshake with more than one certificate unanswered, as a
    /// server that cannot read the chain form does.
    pub refuse_chain: bool,
    /// What FileRequests are answered with, by name (`file://NAME`).
    pub files: HashMap<String, Vec<u8>>,
    /// The names FileRequests asked for, in arrival order.
    pub file_requests: Vec<String>,
    /// Every TokenRequest: user name, password, and configuration password.
    pub token_requests: Vec<(String, String, String)>,
    /// The app data, kept as the real server keeps it.
    pub app_data: dpma_proto::appdata::store::MemoryBackend,
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
            md5_of_document: false,
            log: Vec::new(),
            user_agent: String::new(),
            last_mac: HashMap::new(),
            macs: Vec::new(),
            handshake_certificates: Vec::new(),
            refuse_chain: false,
            files: HashMap::new(),
            file_requests: Vec::new(),
            token_requests: Vec::new(),
            app_data: Default::default(),
        }
    }

    /// Issues a token as the real server does (`<phone>/<MAC>/<hash>/<hash>`,
    /// the MAC spelled as the request spelled it) and records the MAC.
    /// Public so that a test can stand in for another client of the same
    /// phone (stock firmware).
    pub fn issue_token(&mut self, phone: &str, mac: &[u8]) -> Vec<u8> {
        self.last_mac.insert(phone.to_string(), mac.to_vec());
        format!("{phone}/{}/2090090897/2090090897", String::from_utf8_lossy(mac)).into_bytes()
    }

    /// The real server's token check: the token must begin with
    /// `<phone>/<the phone's last MAC>`, byte for byte; a token that
    /// passes moves the phone's last MAC to the presenting request's.
    pub fn present_token(&mut self, token: &[u8], mac: &[u8]) -> bool {
        let Some(slash) = token.iter().position(|&c| c == b'/').filter(|&i| i > 0) else { return false };
        let phone = String::from_utf8_lossy(&token[..slash]).into_owned();
        let Some(last) = self.last_mac.get(&phone) else { return false };
        let mut candidate = token[..=slash].to_vec();
        candidate.extend_from_slice(last);
        if !token.starts_with(&candidate) {
            return false;
        }
        if !mac.is_empty() {
            self.last_mac.insert(phone, mac.to_vec());
        }
        true
    }

    fn digest_header(&self) -> Vec<u8> {
        if self.md5_of_document {
            use md5::{Digest, Md5};
            return Md5::digest(&self.config).iter().map(|b| format!("{b:02x}")).collect::<String>().into_bytes();
        }
        self.md5.clone().unwrap_or_default().into_bytes()
    }

    pub fn count(&self, request_type: &str) -> usize {
        self.log.iter().filter(|t| *t == request_type).count()
    }

    pub fn forget_sessions(&mut self) {
        self.sessions.clear();
    }

    /// The server keeps the newest session but changes its password, so
    /// its error replies reach the phone undecryptable (the case of a
    /// session that fails the server's MAC/contact check).
    pub fn rekey_sessions(&mut self) {
        for s in self.sessions.values_mut() {
            s.password = crypto::new_common_password().unwrap();
            s.broken = true;
        }
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
        if let Some(s) = self.sessions.get(&sid).filter(|s| s.broken) {
            let t = String::from_utf8_lossy(raw.header(headers::REQUEST_TYPE).unwrap_or_default()).into_owned();
            self.log.push(format!("{t}(failed check)"));
            let out = Outgoing {
                error: errors::msg::SESSION_TIMEOUT,
                id,
                response_type: types::response_type_for(&t).unwrap_or("Error").into(),
                ..Default::default()
            };
            return out.encode(&Self::ctx(s, &sid)).unwrap();
        }
        let raw_type = String::from_utf8_lossy(raw.header(headers::REQUEST_TYPE).unwrap_or_default()).into_owned();
        let request = match parts.decode(Sender::Phone, password.as_deref()) {
            Ok(r) if raw_type.eq_ignore_ascii_case(types::HANDSHAKE) || password.is_some() || !types::needs_session(&raw_type) => r,
            _ => {
                // Unknown session: the real server answers with a
                // plaintext HandshakeResponse carrying Error-ID -398.
                self.log.push(format!("{raw_type}(no session)"));
                let stranger = ServerSession { password: Vec::new(), chain: false, broken: false };
                let out = Outgoing {
                    error: errors::msg::SESSION_TIMEOUT,
                    id,
                    response_type: b"HandshakeResponse".to_vec(),
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
        self.macs.push(request.mac.clone());
        let mut reply = Outgoing::new();
        reply.id = request.id.clone();
        reply.response_type = types::response_type_for(&request_type).unwrap_or("Error").into();

        if request_type.eq_ignore_ascii_case(types::HANDSHAKE) {
            let certificates = cert::restore_pem_newlines(&request.body);
            let count = String::from_utf8_lossy(&certificates).matches("-----BEGIN CERTIFICATE-----").count();
            self.handshake_certificates.push(count);
            if self.refuse_chain && count > 1 {
                self.log.push("Handshake(chain unanswered)".into());
                return Vec::new();
            }
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
            let session = ServerSession { password, chain: client.intermediate_serial != 0, broken: false };
            let out = reply.encode(&Self::ctx(&session, &sid)).unwrap();
            self.sessions.insert(sid, session);
            return out;
        }

        let carries_token =
            [types::VERIFY_REQUEST, types::CONFIG_REQUEST, types::APP_DATA_REQUEST].contains(&request_type.as_str());
        let token_ok = carries_token && self.present_token(&request.config_token, &request.mac);
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
                self.token_requests.push((
                    user.clone(),
                    String::from_utf8_lossy(&request.password).into_owned(),
                    String::from_utf8_lossy(&request.config_password).into_owned(),
                ));
                let by_mac = !self.token_reqs.contains("user");
                let found = self.users.iter().find(|(e, _, _)| *e == user || by_mac).cloned();
                match found {
                    None => reply.error = errors::msg::USER_NOT_FOUND,
                    Some((_, _, pin)) if self.token_reqs.contains("pass") && request.password != pin.as_bytes() => {
                        reply.error = errors::msg::INVALID_PASSWORD
                    }
                    Some((ext, _, _)) => {
                        reply.config_token = self.issue_token(&ext, &request.mac);
                        reply.username = ext.into_bytes();
                    }
                }
            }
            types::VERIFY_REQUEST => {
                if token_ok {
                    reply.md5 = self.digest_header();
                } else {
                    reply.error = errors::msg::INVALID_TOKEN;
                }
            }
            types::CONFIG_REQUEST => {
                if token_ok {
                    reply.body = self.config.clone();
                    reply.md5 = self.digest_header();
                } else {
                    reply.error = errors::msg::INVALID_TOKEN;
                }
            }
            types::FILE_REQUEST => {
                let name = String::from_utf8_lossy(request.body.strip_prefix(b"file://").unwrap_or(&request.body)).into_owned();
                self.file_requests.push(name.clone());
                if let Some(data) = self.files.get(&name) {
                    reply.append(data);
                } else if request.body.starts_with(b"file://ringtone") {
                    reply.append(b"RIFFdata");
                } else {
                    reply.error = errors::msg::FILE_NOT_FOUND;
                }
            }
            types::APP_DATA_REQUEST => {
                use dpma_proto::appdata::{store, ErrorCode, Op, Response, User};
                // The phone is the one the token names, as on the real
                // server; these phones have no tenant key.
                let phone = String::from_utf8_lossy(&request.config_token).split('/').next().unwrap_or_default().to_string();
                let name = self.users.iter().find(|(e, _, _)| *e == phone).map(|(_, n, _)| n.clone()).unwrap_or_default();
                let user = User { id: phone.clone(), name, extension: phone.clone() };
                if !token_ok {
                    reply.error = errors::msg::INVALID_TOKEN;
                } else {
                    let response = match Op::decode(&request.body) {
                        Err(why) => Response::error(ErrorCode::Invalid, why),
                        Ok(op) => {
                            let who = store::Who { tenant: b"", phone: phone.as_bytes(), user: &user };
                            if let Op::Hello { watch } = &op {
                                store::set_watch(&mut self.app_data, phone.as_bytes(), watch);
                            }
                            let (outcome, _) = store::apply(&mut self.app_data, &who, &op, 1_794_607_500);
                            Response { user: Some(user.clone()), outcome }
                        }
                    };
                    reply.append(&response.encode());
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

/// The client's files, in a temporary directory that lasts as long as any
/// clone of this.
#[derive(Clone)]
pub struct TempFiles {
    _dir: Arc<tempfile::TempDir>,
    files: ClientFiles,
}

impl Default for TempFiles {
    fn default() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let files = ClientFiles::in_dir(dir.path());
        TempFiles { _dir: Arc::new(dir), files }
    }
}

impl TempFiles {
    /// What the client is given.
    pub fn files(&self) -> ClientFiles {
        self.files.clone()
    }
}

impl std::ops::Deref for TempFiles {
    type Target = ClientFiles;
    fn deref(&self) -> &ClientFiles {
        &self.files
    }
}

/// An nvdata tree in memory.
#[derive(Default)]
pub struct MemFiles(pub Mutex<HashMap<String, Vec<u8>>>);

impl MemFiles {
    pub fn get(&self, rel: &str) -> Option<Vec<u8>> {
        self.0.lock().unwrap().get(rel).cloned()
    }
    pub fn put(&self, rel: &str, data: &[u8]) {
        self.0.lock().unwrap().insert(rel.to_string(), data.to_vec());
    }
}

impl Files for MemFiles {
    fn read(&self, rel: &str) -> Option<Vec<u8>> {
        self.get(rel)
    }
    fn write(&self, rel: &str, data: &[u8]) -> Result<(), String> {
        self.put(rel, data);
        Ok(())
    }
    fn remove(&self, rel: &str) {
        self.0.lock().unwrap().remove(rel);
    }
    fn list(&self, dir: &str) -> Vec<String> {
        let prefix = format!("{dir}/");
        let mut names: Vec<String> = self
            .0
            .lock()
            .unwrap()
            .keys()
            .filter_map(|k| k.strip_prefix(&prefix))
            .filter(|n| !n.contains('/'))
            .map(str::to_string)
            .collect();
        names.sort();
        names
    }
}

#[derive(Default)]
pub struct MemSink {
    pub configurator: Mutex<Option<Vec<u8>>>,
    pub md5: Mutex<Option<String>>,
    pub applied: Mutex<Vec<Vec<u8>>>,
    /// What the configuration in effect names (the integrator reads it
    /// from the merged configuration).
    pub wanted: Mutex<Wanted>,
    /// Set to have resources fetched.
    pub files: Mutex<Option<Arc<MemFiles>>>,
    pub resource_changes: std::sync::atomic::AtomicUsize,
    /// Resolve "ignore provisioned firmware" from the fetched
    /// configuration, as the integrator does from the merged one.
    pub reads_ignore_setting: bool,
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
    fn ignore_provisioned_firmware(&self, fetched_config: &[u8]) -> Option<bool> {
        if !self.reads_ignore_setting {
            return None;
        }
        let text = String::from_utf8_lossy(fetched_config);
        Some(text.contains("<setting id=\"opendeskphone_ignore_provisioned_firmware\" value=\"1\""))
    }
    fn wanted_resources(&self) -> Wanted {
        self.wanted.lock().unwrap().clone()
    }
    fn files(&self) -> Option<Arc<dyn Files>> {
        self.files.lock().unwrap().clone().map(|f| f as Arc<dyn Files>)
    }
    fn resources_changed(&self) {
        self.resource_changes.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
