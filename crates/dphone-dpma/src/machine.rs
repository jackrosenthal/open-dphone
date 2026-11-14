//! The provisioning state machine (plan section 9): discover, handshake,
//! verify or obtain a token, fetch and apply the configuration, then stay
//! attached for pushes.

use crate::link::{body_of, DpmaError, Link};
use crate::xml::{self, Configurator};
use crate::{files, ConfigSink, DpmaConfig, Input, Storage};
use dphone_state::{
    clock, Command, CommandError, DiscoveredServer, DiscoverySource, Envelope, Event, ProvisionPhase,
    ProvisionUser, Provisioning, Reply, Store, Subsystem,
};
use dpma_proto::cert::PhoneIdentity;
use dpma_proto::message::Incoming;
use dpma_proto::{errors, headers, types};
use md5::{Digest, Md5};
use rsa::pkcs1::{DecodeRsaPrivateKey, EncodeRsaPrivateKey};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// How one provisioning attempt ended.
enum Outcome {
    Provisioned,
    /// Waiting for `StartProvisioning { server }`.
    NeedServer,
    /// Waiting for `SelectUser`.
    NeedUser,
}

struct Fail {
    reason: String,
    code: Option<i32>,
    /// Trying again by itself may help (timeouts, transport).
    retry: bool,
}

impl From<DpmaError> for Fail {
    fn from(e: DpmaError) -> Fail {
        match e {
            DpmaError::Server { code, text } => Fail { reason: text, code: Some(code), retry: code == errors::msg::SESSION_TIMEOUT },
            other => Fail { reason: other.to_string(), code: None, retry: true },
        }
    }
}

pub(crate) struct Machine {
    store: Store,
    cfg: DpmaConfig,
    link: Arc<Link>,
    storage: Arc<dyn Storage>,
    sink: Arc<dyn ConfigSink>,
    net_up: bool,
    option66: Option<String>,
    mdns: Vec<DiscoveredServer>,
    manual: Option<String>,
    user: Option<String>,
    pin: Option<String>,
    standalone: bool,
    attached: bool,
    server: Option<DiscoveredServer>,
}

fn sip_uri(address: &str) -> String {
    let a = address.trim();
    if a.starts_with("sip:") || a.starts_with("sips:") { a.to_string() } else { format!("sip:proxy@{a}") }
}

fn md5_hex(data: &[u8]) -> String {
    Md5::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

fn header_text(m: &Incoming, name: &str) -> String {
    m.first.header(name).map(|v| String::from_utf8_lossy(v).trim().to_string()).unwrap_or_default()
}

impl Machine {
    pub fn new(
        store: Store,
        cfg: DpmaConfig,
        link: Arc<Link>,
        storage: Arc<dyn Storage>,
        sink: Arc<dyn ConfigSink>,
    ) -> Machine {
        Machine {
            store,
            cfg,
            link,
            storage,
            sink,
            net_up: false,
            option66: None,
            mdns: Vec::new(),
            manual: None,
            user: None,
            pin: None,
            standalone: false,
            attached: false,
            server: None,
        }
    }

    fn phase(&self, phase: ProvisionPhase) {
        self.store.update::<Provisioning, _>(|p| p.phase = phase);
    }

    pub async fn run(
        mut self,
        mut input: mpsc::UnboundedReceiver<Input>,
        mut events: mpsc::UnboundedReceiver<Incoming>,
        commands: dphone_state::CommandReceiver,
    ) {
        let mut want = false;
        let mut full = true;
        let mut retry_at: Option<tokio::time::Instant> = None;
        let mut beat = tokio::time::interval(Duration::from_secs(5));
        let mut ping_at = tokio::time::Instant::now() + Duration::from_secs(self.cfg.ping_secs.max(1));
        loop {
            let retry = async {
                match retry_at {
                    Some(t) => tokio::time::sleep_until(t).await,
                    None => std::future::pending().await,
                }
            };
            let ping = async {
                if self.attached && self.cfg.ping_secs > 0 {
                    tokio::time::sleep_until(ping_at).await
                } else {
                    std::future::pending().await
                }
            };
            tokio::select! {
                i = input.recv() => match i {
                    Some(Input::Network { up, option66, mdns }) => {
                        let came_up = up && !self.net_up;
                        self.net_up = up;
                        self.option66 = option66;
                        self.mdns = mdns;
                        if !up {
                            self.attached = false;
                            self.link.forget();
                            self.store.update::<Provisioning, _>(|p| p.attached = false);
                        } else if (came_up || !self.attached) && !self.standalone {
                            want = true;
                            full = true;
                        }
                    }
                    None => break,
                },
                env = commands.next() => match env {
                    Some(env) => {
                        if self.command(env) {
                            want = true;
                            full = true;
                        }
                    }
                    None => break,
                },
                ev = events.recv() => match ev {
                    Some(ev) => {
                        if let Some(f) = self.event(ev) {
                            want = true;
                            full = f;
                        }
                    }
                    None => break,
                },
                _ = retry => {
                    retry_at = None;
                    want = true;
                }
                _ = ping => {
                    ping_at = tokio::time::Instant::now() + Duration::from_secs(self.cfg.ping_secs.max(1));
                    match self.link.request(types::PING_REQUEST, Vec::new(), |_| {}).await {
                        Ok(r) if r.error == 0 => {}
                        other => {
                            log::warn!("dpma: ping failed ({}); starting over", match other {
                                Ok(r) => format!("error {}", r.error),
                                Err(e) => e.to_string(),
                            });
                            self.attached = false;
                            self.store.update::<Provisioning, _>(|p| p.attached = false);
                            want = true;
                            full = true;
                        }
                    }
                }
                _ = beat.tick() => self.store.heartbeat(Subsystem::Dpma),
            }
            if want && self.net_up && !self.standalone {
                want = false;
                retry_at = None;
                match self.provision(full).await {
                    Ok(Outcome::Provisioned) => {
                        self.attached = true;
                        ping_at = tokio::time::Instant::now() + Duration::from_secs(self.cfg.ping_secs.max(1));
                    }
                    Ok(Outcome::NeedServer) => self.phase(ProvisionPhase::AwaitingServer),
                    Ok(Outcome::NeedUser) => self.phase(ProvisionPhase::AwaitingUser),
                    Err(f) => {
                        log::warn!("dpma: provisioning failed: {} (code {:?})", f.reason, f.code);
                        self.attached = false;
                        self.store.update::<Provisioning, _>(|p| {
                            p.attached = false;
                            p.phase = ProvisionPhase::Failed { reason: f.reason.clone(), code: f.code };
                        });
                        if f.retry {
                            retry_at = Some(tokio::time::Instant::now() + Duration::from_secs(self.cfg.retry_secs));
                        }
                    }
                }
                full = true;
            }
        }
    }

    /// Handles a command; returns whether to (re)start provisioning.
    fn command(&mut self, env: Envelope) -> bool {
        let (command, responder) = env.split();
        let mut start = false;
        let result = match command {
            Command::StartProvisioning { server } => {
                if let Some(s) = server.filter(|s| !s.trim().is_empty()) {
                    self.manual = Some(s);
                }
                self.standalone = false;
                start = true;
                Ok(Reply::Done)
            }
            Command::SelectUser { id, pin } => {
                self.user = Some(id).filter(|u| !u.is_empty());
                self.pin = Some(pin).filter(|p| !p.is_empty());
                self.standalone = false;
                start = true;
                Ok(Reply::Done)
            }
            Command::Reprovision => {
                self.standalone = false;
                start = true;
                Ok(Reply::Done)
            }
            Command::ForgetProvisioning => {
                self.link.forget();
                self.link.set_token(None);
                self.manual = None;
                self.user = None;
                self.pin = None;
                self.attached = false;
                self.server = None;
                self.storage.remove(files::IDENTITY_CERT);
                self.storage.remove(files::IDENTITY_KEY);
                let r = self.sink.forget();
                self.store.set(Provisioning::default());
                self.standalone = false;
                start = true;
                r.map(|_| Reply::Done).map_err(CommandError::Failed)
            }
            Command::SkipProvisioning => {
                self.standalone = true;
                self.attached = false;
                self.link.forget();
                self.store.update::<Provisioning, _>(|p| {
                    p.attached = false;
                    p.phase = ProvisionPhase::Standalone;
                });
                Ok(Reply::Done)
            }
            other => Err(CommandError::Invalid(format!("not a provisioning command: {other:?}"))),
        };
        responder.reply(result);
        start
    }

    /// Handles a server-initiated message.  `Some(full)` asks for another
    /// provisioning pass, with or without a new handshake.
    fn event(&mut self, ev: Incoming) -> Option<bool> {
        let name = header_text(&ev, headers::EVENT_NAME);
        if ev.error == errors::msg::SESSION_TIMEOUT {
            // Reassignment (`digium.assignment.update` arrives as ID
            // `bootcReset` with -398) or an expired session: like stock,
            // go back to the handshake.  A reassignment also invalidates
            // the token and the chosen user.
            if ev.id == b"bootcReset" {
                self.link.set_token(None);
                self.user = None;
                self.pin = None;
                self.save_configurator(|c| {
                    c.remove("config_server_token");
                    c.remove("config_server_user");
                });
            }
            self.attached = false;
            self.link.forget();
            self.store.update::<Provisioning, _>(|p| p.attached = false);
            return Some(true);
        }
        if name == types::events::CONFIG_UPDATE {
            if xml::deletes_token(&ev.payload) {
                self.link.set_token(None);
                self.save_configurator(|c| c.remove("config_server_token"));
            }
            self.store.emit(Event::Push { name, body: String::from_utf8_lossy(&ev.payload).into_owned() });
            return Some(false);
        }
        if !name.is_empty() {
            self.store.emit(Event::Push { name, body: String::from_utf8_lossy(&ev.payload).into_owned() });
        } else {
            log::debug!("dpma: unexpected message id {:?}", String::from_utf8_lossy(&ev.id));
        }
        None
    }

    fn configurator(&self) -> Configurator {
        self.sink.configurator().map(|x| Configurator::parse(&x)).unwrap_or_default()
    }

    fn save_configurator(&self, f: impl FnOnce(&mut Configurator)) {
        let mut c = self.configurator();
        f(&mut c);
        if let Err(e) = self.sink.write_configurator(&c.render()) {
            log::warn!("dpma: cannot save configurator.xml: {e}");
        }
    }

    /// Manual entry, then the saved server, then DHCP option 66 in its
    /// `sip:` form, then a single mDNS answer.
    fn pick_server(&self) -> (Option<DiscoveredServer>, Vec<DiscoveredServer>) {
        let mut found = Vec::new();
        if let Some(o) = self.option66.as_deref().map(str::trim).filter(|o| o.starts_with("sip:") || o.starts_with("sips:")) {
            found.push(DiscoveredServer { source: DiscoverySource::DhcpOption66, name: o.to_string(), address: o.to_string() });
        }
        found.extend(self.mdns.iter().cloned());
        if let Some(m) = &self.manual {
            return (Some(DiscoveredServer { source: DiscoverySource::Manual, name: m.clone(), address: m.clone() }), found);
        }
        let saved = self.configurator();
        if let Some(url) = saved.get("config_server_url") {
            // `config_server_type` other than digium/boot66 is stock's HTTP
            // provisioning, which is not this client's business.
            if matches!(saved.get("config_server_type"), Some("digium") | Some("boot66") | None) {
                return (Some(DiscoveredServer { source: DiscoverySource::Saved, name: url.to_string(), address: url.to_string() }), found);
            }
        }
        if let Some(o) = found.iter().find(|f| f.source == DiscoverySource::DhcpOption66) {
            return (Some(o.clone()), found);
        }
        let mdns: Vec<_> = found.iter().filter(|f| f.source == DiscoverySource::Mdns).collect();
        if mdns.len() == 1 {
            return (Some(mdns[0].clone()), found);
        }
        (None, found)
    }

    /// The phone identity: minted on first use with the provisioning CA
    /// built into `dpma-proto`, and kept under `/nvdata/open/`.  Stock's
    /// own `phonekey.pem`/`phonecert.pem` are never touched.
    fn identity(&self) -> Result<PhoneIdentity, Fail> {
        let fail = |reason: String| Fail { reason, code: None, retry: false };
        if let (Some(cert), Some(key)) = (self.storage.read(files::IDENTITY_CERT), self.storage.read(files::IDENTITY_KEY)) {
            if let Ok(private_key) = rsa::RsaPrivateKey::from_pkcs1_pem(&String::from_utf8_lossy(&key)) {
                return Ok(PhoneIdentity { certificate_pem: cert, private_key });
            }
            log::warn!("dpma: the saved identity key is unreadable; minting a new identity");
        }
        let id = PhoneIdentity::generate().map_err(|e| fail(format!("cannot mint an identity: {}", e.message())))?;
        let pem = id
            .private_key
            .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
            .map_err(|e| fail(format!("cannot encode the identity key: {e}")))?;
        self.storage.write(files::IDENTITY_CERT, &id.certificate_pem).map_err(fail)?;
        self.storage.write(files::IDENTITY_KEY, pem.as_bytes()).map_err(fail)?;
        Ok(id)
    }

    async fn provision(&mut self, full: bool) -> Result<Outcome, Fail> {
        let mut token_reqs = String::new();
        let mut userlist_reqs = String::new();
        if full || !self.link.has_session() {
            self.phase(ProvisionPhase::Discovering);
            let (server, found) = self.pick_server();
            self.store.update::<Provisioning, _>(|p| p.discovered = found);
            let Some(server) = server else { return Ok(Outcome::NeedServer) };
            self.link.set_server(&sip_uri(&server.address));
            self.server = Some(server.clone());
            self.store.update::<Provisioning, _>(|p| {
                p.server = Some(server);
                p.phase = ProvisionPhase::Handshaking;
                p.attached = false;
            });
            let identity = self.identity()?;
            self.link.forget();
            let resp = self.link.handshake(identity.legacy_body()).await.map_err(|e| match e {
                // A refused certificate gets no HandshakeResponse at all.
                DpmaError::Timeout => Fail {
                    reason: "no HandshakeResponse (server unreachable, or it refused the phone certificate)".into(),
                    code: None,
                    retry: true,
                },
                other => other.into(),
            })?;
            if resp.error != 0 {
                return Err(Fail { reason: errors::msg_error_text(resp.error).into(), code: Some(resp.error), retry: true });
            }
            self.link.accept_handshake(&resp, &identity.private_key)?;
            token_reqs = header_text(&resp, headers::TOKEN_AUTH_REQUIREMENTS).to_ascii_lowercase();
            userlist_reqs = header_text(&resp, headers::USERLIST_AUTH_REQUIREMENTS).to_ascii_lowercase();
            let ua = {
                let v = header_text(&resp, headers::USER_AGENT);
                if v.is_empty() { header_text(&resp, "Server") } else { v }
            };
            self.store.update::<Provisioning, _>(|p| {
                p.server_user_agent = ua;
                p.attached = true;
            });
        }
        let has = |reqs: &str, what: &str| reqs.split(',').any(|r| r.trim() == what);

        // Saved state applies only to the server it was saved for.
        let saved = self.configurator();
        let server_addr = self.server.as_ref().map(|s| s.address.clone()).unwrap_or_default();
        let same_server = saved.get("config_server_url").is_none_or(|u| sip_uri(u) == sip_uri(&server_addr));
        if self.user.is_none() && same_server {
            self.user = saved.get("config_server_user").map(str::to_string);
        }
        let saved_user_pass = saved.get("config_user_password").filter(|_| same_server).map(str::to_string);
        let saved_cfg_pass = saved.get("config_server_password").filter(|_| same_server).map(str::to_string);
        if let (true, true, Some(t)) = (self.link.token().is_none(), same_server, saved.get("config_server_token")) {
            self.link.set_token(Some(t.as_bytes()));
        }
        let user = self.user.clone().unwrap_or_default();

        // A token from an earlier boot: verify it.  Only a digest equal to
        // the cached one lets us skip the download ("md5sum_match").
        if full && self.link.token().is_some() {
            let u = user.clone();
            let resp = self.link.request(types::VERIFY_REQUEST, Vec::new(), |r| r.username = u.into_bytes()).await?;
            if resp.error == 0 {
                let md5 = header_text(&resp, headers::MD5);
                if !md5.is_empty() && self.sink.cached_md5().as_deref() == Some(md5.as_str()) {
                    self.done(&user, &md5);
                    return Ok(Outcome::Provisioned);
                }
            } else if resp.error == errors::msg::SESSION_TIMEOUT {
                return Err(Fail { reason: "session timeout".into(), code: Some(resp.error), retry: true });
            } else {
                log::info!("dpma: the saved token was refused ({}); asking for a new one", resp.error);
                self.link.set_token(None);
            }
        }

        if self.link.token().is_none() {
            let need_user = has(&token_reqs, "user");
            let need_pass = has(&token_reqs, "pass");
            let need_cfg = has(&token_reqs, "configpass");
            let pass = self.pin.clone().or(saved_user_pass.clone());
            let cfg_pass = self.pin.clone().or(saved_cfg_pass.clone());
            let missing = (need_user && user.is_empty()) || (need_pass && pass.is_none()) || (need_cfg && cfg_pass.is_none());
            if missing {
                // Offer the user list when the server gives one out.
                let mut users = Vec::new();
                let can_list = match userlist_reqs.as_str() {
                    "disabled" => false,
                    "configpass" => cfg_pass.is_some(),
                    _ => true,
                };
                if can_list {
                    let cp = cfg_pass.clone();
                    match self
                        .link
                        .request(types::USERLIST_REQUEST, Vec::new(), |r| {
                            if let Some(p) = cp {
                                r.config_password = p.into_bytes();
                            }
                        })
                        .await
                    {
                        Ok(r) if r.error == 0 => users = parse_users(&body_of(&r), need_pass || need_cfg),
                        Ok(r) => log::info!("dpma: no user list: error {}", r.error),
                        Err(e) => log::info!("dpma: no user list: {e}"),
                    }
                }
                self.store.update::<Provisioning, _>(|p| p.users = users);
                return Ok(Outcome::NeedUser);
            }
            let (u, p, c) = (user.clone(), pass.clone(), cfg_pass.clone());
            let resp = self
                .link
                .request(types::TOKEN_REQUEST, Vec::new(), |r| {
                    r.username = u.into_bytes();
                    if let (true, Some(p)) = (need_pass, p) {
                        r.password = p.into_bytes();
                    }
                    if let (true, Some(c)) = (need_cfg, c) {
                        r.config_password = c.into_bytes();
                    }
                })
                .await?;
            if resp.error != 0 {
                // Wrong PIN or user: ask again rather than looping.
                self.pin = None;
                return Err(Fail {
                    reason: errors::msg_error_text(resp.error).into(),
                    code: Some(resp.error),
                    retry: resp.error == errors::msg::SESSION_TIMEOUT,
                });
            }
            if resp.config_token.is_empty() {
                return Err(Fail { reason: "the TokenResponse has no token".into(), code: None, retry: true });
            }
            self.link.set_token(Some(&resp.config_token));
            let assigned = String::from_utf8_lossy(&resp.username).into_owned();
            if self.user.is_none() && !assigned.is_empty() {
                self.user = Some(assigned);
            }
            // Keep what was used, in stock's own settings, so both this
            // firmware and stock find it on the next boot.
            let token = String::from_utf8_lossy(&resp.config_token).into_owned();
            let user_now = self.user.clone().unwrap_or_default();
            let source = self.server.as_ref().map(|s| s.source);
            self.save_configurator(|c| {
                c.set("config_server_type", if source == Some(DiscoverySource::DhcpOption66) { "boot66" } else { "digium" });
                c.set("config_server_manual", if source == Some(DiscoverySource::Manual) { "1" } else { "0" });
                c.set("config_server_url", &server_addr);
                c.set("config_server_user", &user_now);
                if need_cfg {
                    c.set("config_server_password", cfg_pass.as_deref().unwrap_or(""));
                }
                if need_pass {
                    c.set("config_user_password", pass.as_deref().unwrap_or(""));
                }
                c.set("config_server_token", &token);
            });
        }

        // Download.
        self.phase(ProvisionPhase::FetchingConfig);
        let user = self.user.clone().unwrap_or_default();
        let u = user.clone();
        let resp = self.link.request(types::CONFIG_REQUEST, Vec::new(), |r| r.username = u.into_bytes()).await?;
        if resp.error != 0 {
            if resp.error == errors::msg::INVALID_TOKEN {
                self.link.set_token(None);
                self.save_configurator(|c| c.remove("config_server_token"));
            }
            return Err(Fail {
                reason: errors::msg_error_text(resp.error).into(),
                code: Some(resp.error),
                retry: matches!(resp.error, errors::msg::INVALID_TOKEN | errors::msg::SESSION_TIMEOUT),
            });
        }
        let config = body_of(&resp);
        if config.is_empty() {
            return Err(Fail { reason: "the ConfigResponse is empty".into(), code: None, retry: true });
        }
        // Stock stores the server's digest verbatim when it sends one, and
        // otherwise the MD5 of the downloaded bytes.
        let md5 = {
            let h = header_text(&resp, headers::MD5);
            if h.is_empty() { md5_hex(&config) } else { h }
        };
        let offered = xml::offered_firmware(&config);
        let content = if self.cfg.accept_firmware { config } else { xml::strip_firmware(&config) };
        self.store.update::<Provisioning, _>(|p| {
            p.offered_firmware = offered;
            p.phase = ProvisionPhase::Applying;
        });
        self.sink.apply(&content, &md5).map_err(|e| Fail { reason: format!("cannot apply the configuration: {e}"), code: None, retry: false })?;
        self.done(&user, &md5);
        Ok(Outcome::Provisioned)
    }

    fn done(&self, user: &str, md5: &str) {
        self.store.update::<Provisioning, _>(|p| {
            p.phase = ProvisionPhase::Provisioned;
            p.attached = true;
            p.assigned_user = user.to_string();
            p.config_md5 = md5.to_string();
            p.last_success_unix = Some(clock::unix_secs());
            p.users.clear();
        });
    }
}

/// `{"users":[{"extension":"100","name":"Desk","assigned":"0"}]}`;
/// `assigned` is a string.
fn parse_users(body: &[u8], needs_pin: bool) -> Vec<ProvisionUser> {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) else { return Vec::new() };
    let text = |x: &serde_json::Value| match x {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => String::new(),
    };
    v.get("users")
        .and_then(|u| u.as_array())
        .map(|list| {
            list.iter()
                .map(|u| {
                    let id = u.get("extension").map(text).unwrap_or_default();
                    let name = u.get("name").map(text).filter(|n| !n.is_empty()).unwrap_or_else(|| id.clone());
                    ProvisionUser { id, name, needs_pin }
                })
                .filter(|u| !u.id.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn users_json() {
        let u = parse_users(br#"{"users":[{"extension":"100","name":"Desk","assigned":"0"},{"extension":101,"name":""}]}"#, true);
        assert_eq!(u.len(), 2);
        assert_eq!((u[0].id.as_str(), u[0].name.as_str(), u[0].needs_pin), ("100", "Desk", true));
        assert_eq!((u[1].id.as_str(), u[1].name.as_str()), ("101", "101"));
        assert!(parse_users(b"not json", false).is_empty());
    }

    #[test]
    fn server_uris() {
        assert_eq!(sip_uri("10.0.0.1:5060"), "sip:proxy@10.0.0.1:5060");
        assert_eq!(sip_uri("sip:proxy@pbx;transport=tcp"), "sip:proxy@pbx;transport=tcp");
    }
}
