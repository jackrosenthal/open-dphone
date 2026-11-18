//! Accounts: registration (the stack's `Registration`, with our failover
//! between the primary and alternate host around it), keepalives, event
//! subscriptions on the stack's client subscription dialogs, NOTIFY
//! bodies, and out-of-dialog MESSAGE.

use crate::bodies;
use crate::config::{AccountConfig, SipConfig};
use crate::core::{transport_param, Acct, Core, Ev, RegOutcome, Sub, SubKind, Timer};
use crate::util::{self, NameAddr, SipUri};
use crate::{IncomingMessage, OutMessage};
use opendeskphone_state::{
    clock, Account, AccountId, Accounts, ActiveHost, BlfEntry, BlfState, Event, Mwi, MwiAccount, Party, Presence,
    Registration as RegState, Transport,
};
use rsipstack::dialog::dialog::{Dialog, TransactionHandle};
use rsipstack::dialog::registration::Registration;
use rsipstack::dialog::DialogId;
use rsipstack::sip::prelude::HeadersExt;
use rsipstack::sip::{Header, Method, Request, Response};
use rsipstack::transaction::transaction::Transaction;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;

/// What a presence SUBSCRIBE accepts and the expiry it asks for, both as
/// stock's `core` sends them.  Asterisk answers this `Accept` with PIDF.
const PRESENCE_ACCEPT: &str = "application/pidf+xml, application/xpidf+xml";
const PRESENCE_EXPIRES_SECS: u32 = 600;

const ALLOW: &str = "INVITE, ACK, CANCEL, BYE, OPTIONS, NOTIFY, REFER, MESSAGE, INFO, UPDATE";

impl Core {
    pub fn allow() -> Header {
        Header::Allow(ALLOW.to_string().into())
    }

    fn new_registration(&self, cfg: &AccountConfig, alt: bool) -> Registration {
        let (auth_user, password) = cfg.credentials(alt);
        let cred = rsipstack::dialog::authenticate::Credential {
            username: cfg.user.clone(),
            password,
            realm: None,
            auth_username: Some(auth_user),
        };
        let mut reg = Registration::new(self.inner.clone(), Some(cred));
        reg.allow = Some(ALLOW.to_string().into());
        // Over UDP the stack writes the Contact itself, from the address
        // the registrar sees.  Over a stream it leaves out the transport
        // parameter, so the Contact is given (the stack still rewrites
        // its host once it learns the public address).
        let transport = cfg.host(alt).transport;
        if transport != Transport::Udp {
            reg.contact = Some(rsipstack::sip::typed::Contact {
                display_name: None,
                uri: self.contact_uri(&cfg.user, None, transport),
                params: vec![],
            });
        }
        reg
    }

    fn new_acct(&self, cfg: AccountConfig) -> Acct {
        let reg = Arc::new(tokio::sync::Mutex::new(self.new_registration(&cfg, false)));
        Acct { cfg, alt: false, generation: 0, fails: 0, registered: false, reg, public: None, served_by_alt: None }
    }

    /// Applies a configuration: accounts that changed are unregistered and
    /// set up again; subscriptions are rebuilt.
    pub fn configure(&mut self, cfg: SipConfig) {
        let old = std::mem::replace(&mut self.cfg, cfg);
        // A new address changes every Contact: register again.
        let moved = old.local_ip != self.cfg.local_ip;
        let max = self.cfg.model.max_accounts() as usize;
        let wanted: Vec<AccountConfig> = self.cfg.accounts.iter().take(max).cloned().collect();
        let mut keep = Vec::new();
        for a in std::mem::take(&mut self.accts) {
            if wanted.contains(&a.cfg) {
                keep.push(a);
            } else {
                if a.registered {
                    self.send_register(&a, true);
                }
                self.drop_subs(a.cfg.id);
            }
        }
        self.accts = keep;
        let mut fresh = Vec::new();
        for w in wanted {
            if self.acct(w.id).is_none() {
                fresh.push(w.id);
                let a = self.new_acct(w);
                self.accts.push(a);
            }
        }
        self.accts.sort_by_key(|a| a.cfg.id);
        self.publish_accounts(true);
        // BLF entries follow the configuration; subscriptions restart.
        // One entry per contact, however many keys show it; blanks only
        // reserve keys.
        let mut seen = std::collections::HashSet::new();
        let entries: Vec<BlfEntry> = self
            .cfg
            .blf
            .iter()
            .filter(|b| !b.blank && seen.insert(b.id.clone()))
            .map(|b| {
                let prev = self.store.get::<Presence>().get(&b.id).cloned();
                BlfEntry {
                    id: b.id.clone(),
                    label: b.label.clone(),
                    number: b.number.clone(),
                    uri: self.blf_uri(b.account, &b.subscribe).unwrap_or_default(),
                    account: Some(b.account),
                    state: prev.as_ref().map(|p| p.state).unwrap_or(BlfState::Unknown),
                    status: prev.as_ref().map(|p| p.status).unwrap_or_default(),
                    status_text: prev.map(|p| p.status_text).unwrap_or_default(),
                    pickup_ringing: b.pickup_ringing,
                    subscribed: false,
                }
            })
            .collect();
        self.store.update::<Presence, _>(|p| p.entries = entries);
        let ids: Vec<AccountId> = self.accts.iter().map(|a| a.cfg.id).collect();
        for id in &ids {
            self.drop_subs(*id);
        }
        for id in ids {
            if fresh.contains(&id) || moved {
                self.start_registration(id);
            } else if self.acct(id).is_some_and(|a| a.registered || !a.cfg.register) {
                self.start_subs(id);
            }
        }
        self.ensure_listeners();
        self.sync_keys();
    }

    /// Starts accepting connections over TCP and TLS once an account uses
    /// that transport (stock listens on both by default).  The PBX normally
    /// reuses the connection we opened to register; the listeners take
    /// over when that connection has gone away.
    fn ensure_listeners(&mut self) {
        let uses = |t: Transport| self.accts.iter().any(|a| a.cfg.primary.transport == t || a.cfg.alternate.as_ref().is_some_and(|h| h.transport == t));
        let any: std::net::IpAddr = match self.cfg.local_ip {
            ip if ip.is_loopback() => ip,
            std::net::IpAddr::V4(_) => std::net::Ipv4Addr::UNSPECIFIED.into(),
            std::net::IpAddr::V6(_) => std::net::Ipv6Addr::UNSPECIFIED.into(),
        };
        if let (None, Some(port), true) = (self.tcp_listening, self.cfg.tcp_port, uses(Transport::Tcp)) {
            self.tcp_listening = Some(port);
            let inner = self.inner.clone();
            let (bind, external) = (std::net::SocketAddr::new(any, port), std::net::SocketAddr::new(self.cfg.local_ip, port));
            tokio::spawn(async move {
                use rsipstack::transport::tcp_listener::TcpListenerConnection;
                // The stack's own TCP listener.
                let listener = match TcpListenerConnection::new(bind, Some(external)).await {
                    Ok(l) => l,
                    Err(e) => return log::error!("sip: TCP listener: {e}"),
                };
                inner.transport_layer.add_transport(rsipstack::transport::SipConnection::TcpListener(listener.clone()));
                match listener.serve_listener(inner.transport_layer.inner.clone()).await {
                    Ok(()) => log::info!("sip: accepting TCP on port {port}"),
                    Err(e) => log::error!("sip: cannot accept TCP on port {port}: {e}"),
                }
            });
        }
        #[cfg(feature = "tls")]
        if let (None, Some(port), true) = (self.tls_listening, self.cfg.tls_port, uses(Transport::Tls)) {
            let tls = &self.cfg.tls;
            if tls.server_cert_der.is_empty() || tls.server_key_der.is_empty() {
                log::info!("sip: no certificate for the TLS listener yet; not accepting TLS");
            } else {
                self.tls_listening = Some(port);
                let (cert, key) = (tls.server_cert_der.clone(), tls.server_key_der.clone());
                let inner = self.inner.clone();
                tokio::spawn(async move {
                    match crate::tls::listen(std::net::SocketAddr::new(any, port), cert, key, inner).await {
                        Ok(()) => {}
                        Err(e) => log::error!("sip: cannot accept TLS on port {port}: {e}"),
                    }
                });
            }
        }
    }

    fn blf_uri(&self, acct: AccountId, subscribe: &str) -> Option<String> {
        if subscribe.is_empty() {
            return None;
        }
        if subscribe.starts_with("sip:") || subscribe.starts_with("sips:") {
            return Some(subscribe.to_string());
        }
        Some(Core::host_uri(self.acct(acct)?, Some(subscribe)))
    }

    /// Writes the account list (and optionally resets registration
    /// status) to the store.
    pub fn publish_accounts(&self, reset: bool) {
        let prev = self.store.get::<Accounts>();
        let accounts: Vec<Account> = self
            .accts
            .iter()
            .map(|a| {
                let h = a.cfg.host(a.alt);
                let registration = if !a.cfg.register || h.server.is_empty() {
                    RegState::Disabled
                } else {
                    match prev.get(a.cfg.id) {
                        Some(p) if !reset || a.registered => p.registration.clone(),
                        _ => RegState::Unregistered,
                    }
                };
                Account {
                    id: a.cfg.id,
                    config_id: a.cfg.config_id.clone(),
                    label: a.cfg.label.clone(),
                    display_name: a.cfg.display_name.clone(),
                    user: a.cfg.user.clone(),
                    auth_user: a.cfg.credentials(a.alt).0,
                    server: h.server.clone(),
                    port: h.effective_port(),
                    transport: h.transport,
                    alternate_server: a.cfg.alternate.as_ref().map(|h| h.server.clone()).unwrap_or_default(),
                    outbound_proxy: a.cfg.outbound_proxy.clone(),
                    voicemail_number: a.cfg.voicemail_number.clone(),
                    dpma: a.cfg.dpma,
                    srtp: h.srtp,
                    registration,
                }
            })
            .collect();
        let default_account = self.accts.iter().find(|a| a.registered).or(self.accts.first()).map(|a| a.cfg.id);
        self.store.update::<Accounts, _>(|s| {
            s.accounts = accounts;
            s.default_account = default_account;
        });
    }

    fn set_registration(&self, id: AccountId, r: RegState) {
        self.store.update::<Accounts, _>(|s| {
            if let Some(a) = s.get_mut(id) {
                a.registration = r;
            }
        });
        self.publish_accounts(false);
    }

    /// Starts (or restarts) the registration cycle of an account.
    pub fn start_registration(&mut self, id: AccountId) {
        let Some(a) = self.acct_mut(id) else { return };
        a.generation += 1;
        if !a.cfg.register || a.cfg.host(a.alt).server.is_empty() {
            // Nothing to register; subscriptions can still run.
            self.start_subs(id);
            return;
        }
        self.register_now(id);
    }

    fn register_now(&mut self, id: AccountId) {
        let Some(a) = self.acct(id) else { return };
        if !a.registered {
            self.set_registration(id, RegState::Registering);
        }
        if let Some(a) = self.acct(id) {
            self.send_register(a, false);
        }
    }

    /// One REGISTER (or un-REGISTER) through the stack's `Registration`;
    /// the outcome comes back as [`Ev::Registered`].
    fn send_register(&self, a: &Acct, unregister: bool) {
        let h = a.cfg.host(a.alt);
        let expires = if unregister { 0 } else { h.expires_secs };
        let server = match crate::core::uri(&Core::host_uri(a, None)) {
            Ok(u) => u,
            Err(e) => return log::warn!("sip: account {}: {e}", a.cfg.user),
        };
        let (reg, post, resolver) = (a.reg.clone(), self.tx.clone(), self.resolver.clone());
        let (acct, generation) = (a.cfg.id, a.generation);
        let (proxy, transport) = (a.cfg.outbound_proxy.clone(), h.transport);
        let (inner, streams, host, port) = (self.inner.clone(), self.streams.clone(), h.server.clone(), h.effective_port());
        tokio::spawn(async move {
            let mut reg = reg.lock().await;
            // The stack opens TCP connections itself and gives no access to
            // the socket, but it uses a connection it is handed.  So for
            // TCP the connection to the registrar is opened and marked
            // here first.
            if transport == Transport::Tcp {
                let target = match SipUri::parse(&format!("sip:{proxy}")) {
                    Some(u) if !proxy.is_empty() => (u.host, u.port.unwrap_or(5060)),
                    _ => (host, port),
                };
                if let Ok(addr) = resolver.resolve(&target.0, target.1, transport).await {
                    let key = rsipstack::transport::SipAddr { r#type: Some(rsipstack::sip::Transport::Tcp), addr: addr.into() }.to_string();
                    if !streams.lock().unwrap().contains(&key) {
                        let dial = tokio::time::timeout(Duration::from_secs(10), tokio::net::TcpStream::connect(addr)).await;
                        if let Ok(Ok(stream)) = dial {
                            crate::mark(&stream);
                            let local = stream.local_addr().unwrap_or(addr);
                            let local = rsipstack::transport::SipAddr {
                                r#type: Some(rsipstack::sip::Transport::Tcp),
                                addr: local.into(),
                            };
                            if let Ok(conn) = rsipstack::transport::tcp::TcpConnection::from_stream(stream, local, None) {
                                inner.transport_layer.add_connection(rsipstack::transport::SipConnection::Tcp(conn));
                                streams.lock().unwrap().insert(key);
                            }
                        }
                    }
                }
            }
            if !proxy.is_empty() {
                let u = SipUri::parse(&format!("sip:{proxy}"));
                let port = u.as_ref().and_then(|u| u.port).unwrap_or(if transport == Transport::Tls { 5061 } else { 5060 });
                let host = u.map(|u| u.host).unwrap_or(proxy);
                reg.outbound_proxy = resolver.resolve(&host, port, transport).await.ok();
            }
            let outcome = match reg.register(server, Some(expires)).await {
                Ok(resp) => RegOutcome {
                    code: resp.status_code.code(),
                    reason: resp.reason_phrase().map(str::to_string).unwrap_or_else(|| resp.status_code.to_string()),
                    granted: reg.expires(),
                    min_expires: util::header(&resp.headers, "Min-Expires").and_then(|m| m.trim().parse().ok()),
                    public: reg.discovered_public_address(),
                },
                Err(e) => RegOutcome { code: 0, reason: e.to_string(), granted: 0, min_expires: None, public: None },
            };
            if !unregister {
                let _ = post.send(Ev::Registered { acct, generation, outcome });
            }
        });
    }

    pub fn on_registered(&mut self, id: AccountId, generation: u64, o: RegOutcome) {
        let Some(a) = self.acct_mut(id) else { return };
        if a.generation != generation {
            return;
        }
        let h = a.cfg.host(a.alt).clone();
        if (200..300).contains(&o.code) {
            let was = a.registered;
            a.registered = true;
            a.fails = 0;
            a.public = o.public;
            let (alt, user) = (a.alt, a.cfg.user.clone());
            // Registered with the other host than last time: the server
            // changed under the apps (stock `digium.phone.server_change`).
            if a.served_by_alt.replace(alt).is_some_and(|before| before != alt) {
                // Stock's event data: the account and which of its hosts.
                let body = format!("{{\"accountSlot\":{},\"accountSubslot\":{}}}", id.0, alt as u8);
                self.store.emit(Event::Push { name: "digium.phone.server_change".into(), body });
            }
            let granted = o.granted.max(10);
            // Stock refreshes at half the lifetime.
            self.after(Duration::from_secs((granted / 2).max(5) as u64), Timer::Register { acct: id, generation });
            self.set_registration(
                id,
                RegState::Registered {
                    host: if alt { ActiveHost::Alternate } else { ActiveHost::Primary },
                    expires_mono_ms: clock::mono_ms() + granted as u64 * 1000,
                },
            );
            if !was {
                log::info!("sip: account {user} registered with {}", h.server);
                self.start_subs(id);
                if self.cfg.udp_keepalive_secs > 0 && h.transport == Transport::Udp {
                    self.after(
                        Duration::from_secs(self.cfg.udp_keepalive_secs as u64),
                        Timer::Keepalive { acct: id, generation },
                    );
                }
            }
            return;
        }
        // 423 Interval Too Brief: ask for what the registrar accepts.
        if let (423, Some(min)) = (o.code, o.min_expires) {
            if a.alt {
                if let Some(h) = a.cfg.alternate.as_mut() {
                    h.expires_secs = min;
                }
            } else {
                a.cfg.primary.expires_secs = min;
            }
            return self.register_now(id);
        }
        let was = a.registered;
        a.registered = false;
        a.fails += 1;
        let mut delay = h.retry_secs.max(1) as u64;
        let switch = a.cfg.alternate.is_some() && a.fails >= h.num_retries.max(1);
        if switch {
            a.alt = !a.alt;
            a.fails = 0;
            delay = 1;
            log::warn!(
                "sip: account {}: {} failed ({} {}); switching to {}",
                a.cfg.user,
                h.server,
                o.code,
                o.reason,
                a.cfg.host(a.alt).server
            );
            let (cfg, alt) = (a.cfg.clone(), a.alt);
            // A different registrar is a different binding: new Call-ID.
            let fresh = Arc::new(tokio::sync::Mutex::new(self.new_registration(&cfg, alt)));
            if let Some(a) = self.acct_mut(id) {
                a.reg = fresh;
            }
        } else {
            log::warn!("sip: account {}: registration failed: {} {}", a.cfg.user, o.code, o.reason);
        }
        if was {
            self.drop_subs(id);
        }
        self.set_registration(
            id,
            RegState::Failed { code: o.code, reason: o.reason, retry_mono_ms: clock::mono_ms() + delay * 1000 },
        );
        self.after(Duration::from_secs(delay), Timer::Register { acct: id, generation });
    }

    pub fn on_register_timer(&mut self, id: AccountId, generation: u64) {
        let Some(a) = self.acct_mut(id) else { return };
        if a.generation != generation {
            return;
        }
        // On the alternate host, a refresh first offers the primary one
        // more chance: one failure there falls straight back.
        if a.registered && a.alt {
            a.alt = false;
            a.fails = a.cfg.primary.num_retries.max(1) - 1;
            let cfg = a.cfg.clone();
            let fresh = Arc::new(tokio::sync::Mutex::new(self.new_registration(&cfg, false)));
            if let Some(a) = self.acct_mut(id) {
                a.reg = fresh;
            }
        }
        self.register_now(id);
    }

    /// Runs a request that belongs to no dialog and discards the answer.
    fn fire(&self, req: Result<Request, String>, dest: rsipstack::transport::SipAddr, a: &Acct, what: &'static str) {
        let (inner, cred) = (self.inner.clone(), Some(Core::credential(a)));
        match req {
            Ok(req) => {
                tokio::spawn(async move {
                    if let Err(e) = Core::transact(inner, req, dest, cred).await {
                        log::debug!("sip: {what}: {e}");
                    }
                });
            }
            Err(e) => log::warn!("sip: {what}: {e}"),
        }
    }

    /// OPTIONS toward the registrar keeps a UDP NAT binding open.
    pub fn on_keepalive_timer(&mut self, id: AccountId, generation: u64) {
        let Some(a) = self.acct(id) else { return };
        if a.generation != generation || !a.registered || self.cfg.udp_keepalive_secs == 0 {
            return;
        }
        let target = Core::host_uri(a, None);
        let aor = Core::host_uri(a, Some(&a.cfg.user));
        if let Ok((dest, t)) = self.request_dest(a, &target) {
            let req = self.make_request(
                Method::Options,
                &target,
                &aor,
                &util::token(10),
                &aor,
                None,
                &format!("{}@opendeskphone", util::token(20)),
                1,
                t,
                None,
                vec![Header::Accept("application/sdp".to_string().into())],
                Vec::new(),
            );
            self.fire(req, dest, a, "keepalive");
        }
        self.after(Duration::from_secs(self.cfg.udp_keepalive_secs as u64), Timer::Keepalive { acct: id, generation });
    }

    /// PUBLISH of our own presence on every registered account.
    pub fn publish_presence(&mut self, status: opendeskphone_state::PresenceStatus, text: &str) {
        for a in self.accts.iter().filter(|a| a.registered) {
            let aor = Core::host_uri(a, Some(&a.cfg.user));
            let Ok((dest, t)) = self.request_dest(a, &aor) else { continue };
            let entity = format!("sip:{}@{}", a.cfg.user, a.cfg.host(a.alt).server);
            let req = self.make_request(
                Method::Publish,
                &aor,
                &aor,
                &util::token(10),
                &aor,
                None,
                &format!("{}@opendeskphone", util::token(20)),
                1,
                t,
                None,
                vec![
                    Header::Event("presence".to_string().into()),
                    Header::Expires(3600.into()),
                    Header::ContentType("application/pidf+xml".to_string().into()),
                ],
                bodies::build_pidf(&entity, status, text).into_bytes(),
            );
            self.fire(req, dest, a, "PUBLISH");
        }
    }

    /// Un-registers every account (shutdown).
    pub fn unregister_all(&mut self) {
        for a in self.accts.iter().filter(|a| a.registered) {
            self.send_register(a, true);
        }
        for a in self.accts.iter_mut() {
            a.registered = false;
            a.generation += 1;
        }
    }

    // ----------------------------------------------------- subscriptions

    fn drop_sub_dialog(&self, s: &Sub) {
        self.layer.remove_dialog(&DialogId {
            call_id: s.call_id.clone(),
            local_tag: s.local_tag.clone(),
            remote_tag: s.remote_tag.clone(),
        });
    }

    fn drop_subs(&mut self, acct: AccountId) {
        // Subscriptions simply stop being refreshed; the server expires
        // them.
        for s in self.subs.iter().filter(|s| s.acct == acct) {
            self.drop_sub_dialog(s);
        }
        self.subs.retain(|s| s.acct != acct);
        self.store.update::<Presence, _>(|p| {
            for e in p.entries.iter_mut().filter(|e| e.account == Some(acct)) {
                e.subscribed = false;
            }
        });
    }

    /// Creates and starts the subscriptions of an account.
    pub fn start_subs(&mut self, acct: AccountId) {
        if self.subs.iter().any(|s| s.acct == acct) {
            return;
        }
        let Some(a) = self.acct(acct) else { return };
        let mut wanted: Vec<(SubKind, String)> = Vec::new();
        if a.cfg.mwi_subscribe && a.cfg.register {
            let uri = if a.cfg.mwi_uri.is_empty() { Core::host_uri(a, Some(&a.cfg.user)) } else { a.cfg.mwi_uri.clone() };
            wanted.push((SubKind::Mwi, uri));
        }
        let mut seen = std::collections::HashSet::new();
        for b in self.cfg.blf.iter().filter(|b| b.account == acct && !b.blank && seen.insert(b.id.clone())) {
            if let Some(uri) = self.blf_uri(acct, &b.subscribe) {
                wanted.push((SubKind::Blf(b.id.clone()), uri));
            }
        }
        for (kind, uri) in wanted {
            self.next_sub += 1;
            let id = self.next_sub;
            self.subs.push(Sub {
                id,
                kind,
                acct,
                uri,
                call_id: String::new(),
                local_tag: String::new(),
                remote_tag: String::new(),
                dialog: None,
                generation: 0,
            });
            self.subscribe(id);
        }
    }

    /// Puts a client subscription dialog for `sub` into the layer.  The
    /// layer's subscription dialog models an established dialog only: it
    /// never learns the notifier's tag from the 200 or the first NOTIFY.
    /// So the dialog is seeded without a tag for the initial SUBSCRIBE and
    /// seeded again, with the tag, as soon as it is known.
    fn seed_sub(&mut self, id: u32, remote_tag: &str, target: Option<String>, cseq: u32) -> Result<(), String> {
        let Some(s) = self.subs.iter().find(|s| s.id == id) else { return Err("no such subscription".into()) };
        let a = self.acct(s.acct).ok_or("no such account")?;
        let aor = Core::host_uri(a, Some(&a.cfg.user));
        let target = target.unwrap_or_else(|| s.uri.clone());
        let transport = self.request_dest(a, &target).map(|d| d.1).unwrap_or(a.cfg.host(a.alt).transport);
        let contact = self.account_contact(a);
        let seed = self.make_request(
            Method::Subscribe,
            &target,
            &aor,
            &s.local_tag,
            &s.uri,
            (!remote_tag.is_empty()).then_some(remote_tag),
            &s.call_id,
            cseq,
            transport,
            None,
            Vec::new(),
            Vec::new(),
        )?;
        self.drop_sub_dialog(s);
        let dialog = self
            .layer
            .get_or_create_client_subscription(
                s.call_id.clone(),
                s.local_tag.clone(),
                remote_tag.to_string(),
                seed,
                self.dialog_tx.clone(),
                Some(Core::credential(a)),
                Some(contact),
            )
            .map_err(|e| e.to_string())?;
        let s = self.subs.iter_mut().find(|s| s.id == id).expect("found above");
        s.remote_tag = remote_tag.to_string();
        s.dialog = Some(dialog);
        Ok(())
    }

    /// Sends the initial SUBSCRIBE or a refresh.
    pub fn subscribe(&mut self, id: u32) {
        let mwi_expires = self.cfg.subscribe_expires_secs;
        let Some(s) = self.subs.iter_mut().find(|s| s.id == id) else { return };
        s.generation += 1;
        let generation = s.generation;
        if s.dialog.is_none() {
            s.call_id = format!("{}@opendeskphone", util::token(20));
            s.local_tag = util::token(10);
            s.remote_tag.clear();
            if let Err(e) = self.seed_sub(id, "", None, 0) {
                log::warn!("sip: SUBSCRIBE: {e}");
                return self.after(Duration::from_secs(60), Timer::Subscribe { sub: id, generation });
            }
        }
        let Some(s) = self.subs.iter().find(|s| s.id == id) else { return };
        let (event, accept, expires) = match s.kind {
            SubKind::Blf(_) => ("presence", PRESENCE_ACCEPT, PRESENCE_EXPIRES_SECS),
            SubKind::Mwi => ("message-summary", "application/simple-message-summary", mwi_expires),
        };
        let headers = vec![
            Header::Event(event.to_string().into()),
            Header::Accept(accept.to_string().into()),
            Header::Expires(expires.into()),
        ];
        let (dialog, post) = (s.dialog.clone().expect("seeded above"), self.tx.clone());
        tokio::spawn(async move {
            let result = dialog.subscribe(Some(headers), None).await.map_err(|e| e.to_string());
            let _ = post.send(Ev::Subscribed { sub: id, generation, result });
        });
    }

    fn mark_subscribed(&self, kind: &SubKind, on: bool) {
        if let SubKind::Blf(entry) = kind {
            self.store.update::<Presence, _>(|p| {
                if let Some(e) = p.entries.iter_mut().find(|e| e.id == *entry) {
                    e.subscribed = on;
                    if !on {
                        e.state = BlfState::Unknown;
                    }
                }
            });
            self.sync_keys();
        }
    }

    pub fn on_subscribed(&mut self, id: u32, generation: u64, result: Result<Option<Response>, String>) {
        let Some(s) = self.subs.iter().find(|s| s.id == id) else { return };
        if s.generation != generation {
            return;
        }
        let expires_default = match s.kind {
            SubKind::Blf(_) => PRESENCE_EXPIRES_SECS,
            SubKind::Mwi => self.cfg.subscribe_expires_secs,
        };
        let resp = match result {
            Ok(Some(r)) => Some(r),
            Ok(None) => None,
            Err(e) => {
                log::debug!("sip: SUBSCRIBE failed: {e}");
                None
            }
        };
        let code = resp.as_ref().map(|r| r.status_code.code()).unwrap_or(408);
        if (200..300).contains(&code) {
            let resp = resp.expect("a 2xx is a response");
            // The first 2xx (unless a NOTIFY got here first) tells us the
            // notifier's tag and Contact: make the dialog a real one.
            if s.remote_tag.is_empty() {
                let tag = util::header(&resp.headers, "To").and_then(|t| NameAddr::parse(t).tag().map(str::to_string));
                let contact = util::header(&resp.headers, "Contact").map(|c| NameAddr::parse(c).uri);
                let cseq = resp.cseq_header().ok().and_then(|c| c.seq().ok()).unwrap_or(2);
                if let Some(tag) = tag {
                    if let Err(e) = self.seed_sub(id, &tag, contact, cseq) {
                        log::warn!("sip: subscription: {e}");
                    }
                }
            }
            let granted = util::header(&resp.headers, "Expires")
                .and_then(|e| e.trim().parse::<u32>().ok())
                .unwrap_or(expires_default)
                .max(30);
            if let Some(s) = self.subs.iter().find(|s| s.id == id) {
                self.mark_subscribed(&s.kind, true);
            }
            self.after(Duration::from_secs(granted as u64 * 4 / 5), Timer::Subscribe { sub: id, generation });
        } else {
            // 481: the server lost the dialog; anything else: try again
            // later.  Either way with a new dialog.
            self.drop_sub_dialog(s);
            self.mark_subscribed(&s.kind, false);
            if let Some(s) = self.subs.iter_mut().find(|s| s.id == id) {
                s.dialog = None;
            }
            let delay = if code == 481 { 1 } else { 60 };
            self.after(Duration::from_secs(delay), Timer::Subscribe { sub: id, generation });
        }
    }

    pub fn on_subscribe_timer(&mut self, id: u32, generation: u64) {
        if self.subs.iter().any(|s| s.id == id && s.generation == generation) {
            self.subscribe(id);
        }
    }

    /// A NOTIFY that matched none of the layer's dialogs: the first NOTIFY
    /// of a subscription whose tag we did not know yet, or unsolicited MWI.
    pub fn on_stray_notify(&mut self, tx: Transaction) {
        let req = &tx.original;
        let call_id = util::header(&req.headers, "Call-ID").unwrap_or("").to_string();
        let event = util::header(&req.headers, "Event").unwrap_or("").split(';').next().unwrap_or("").trim().to_ascii_lowercase();
        let from = NameAddr::parse(util::header(&req.headers, "From").unwrap_or(""));
        let pending = self.subs.iter().find(|s| s.call_id == call_id && s.remote_tag.is_empty()).map(|s| s.id);
        if let (Some(id), Some(tag)) = (pending, from.tag()) {
            let contact = util::header(&req.headers, "Contact").map(|c| NameAddr::parse(c).uri);
            // Our SUBSCRIBE may have been re-sent with credentials, so the
            // next CSeq must clear both attempts.
            if self.seed_sub(id, tag, contact, 8).is_ok() {
                if let Some(d) = self.subs.iter().find(|s| s.id == id).and_then(|s| s.dialog.clone()) {
                    return self.dispatch(Dialog::ClientSubscription(d), tx);
                }
            }
        }
        if event == "check-sync" {
            // Polycom-style "fetch your configuration again" (stock
            // `enable_check_sync`).
            if self.cfg.check_sync {
                log::info!("sip: check-sync received; asking for the configuration again");
                self.store.post(opendeskphone_state::Command::Reprovision);
            }
            return self.reply(tx, 200, Vec::new());
        }
        if event == "message-summary" {
            if let Some(acct) = self.account_for(req) {
                let body = String::from_utf8_lossy(&req.body).into_owned();
                self.apply_mwi(acct, &body);
            }
            return self.reply(tx, 200, Vec::new());
        }
        self.reply(tx, 481, Vec::new());
    }

    /// A NOTIFY on one of our subscription dialogs.
    pub fn on_sub_notify(&mut self, idx: usize, req: Request, handle: TransactionHandle) {
        self.respond(handle, 200, Vec::new(), None);
        let body = String::from_utf8_lossy(&req.body).into_owned();
        let state = util::header(&req.headers, "Subscription-State").unwrap_or("").to_ascii_lowercase();
        let terminated = state.starts_with("terminated");
        let (id, acct) = (self.subs[idx].id, self.subs[idx].acct);
        match &self.subs[idx].kind {
            SubKind::Blf(entry) => {
                // One document says both what the contact's line does and
                // what status the user has set.
                let entry = entry.clone();
                if let Some(doc) = bodies::parse_presence(&body) {
                    self.store.update::<Presence, _>(|p| {
                        if let Some(e) = p.entries.iter_mut().find(|e| e.id == entry) {
                            e.state = doc.state;
                            e.status = doc.status;
                            e.status_text = doc.message.clone();
                            e.subscribed = !terminated;
                        }
                    });
                    self.sync_keys();
                }
            }
            SubKind::Mwi => self.apply_mwi(acct, &body),
        }
        if terminated {
            self.drop_sub_dialog(&self.subs[idx]);
            let s = &mut self.subs[idx];
            s.dialog = None;
            s.generation += 1;
            let generation = s.generation;
            let delay = if state.contains("reason=rejected") || state.contains("reason=noresource") { 300 } else { 5 };
            self.after(Duration::from_secs(delay), Timer::Subscribe { sub: id, generation });
        }
    }

    fn apply_mwi(&mut self, acct: AccountId, body: &str) {
        let m = bodies::parse_message_summary(body);
        self.store.update::<Mwi, _>(|s| {
            let entry = MwiAccount {
                account: acct,
                waiting: m.waiting,
                new: m.new,
                old: m.old,
                urgent_new: m.urgent_new,
                urgent_old: m.urgent_old,
            };
            match s.accounts.iter_mut().find(|a| a.account == acct) {
                Some(a) => *a = entry,
                None => s.accounts.push(entry),
            }
        });
        self.sync_keys();
    }

    // ----------------------------------------------------------- MESSAGE

    /// Sends an out-of-dialog MESSAGE; `reply` gets the final status.
    pub fn send_message(&mut self, msg: OutMessage, reply: oneshot::Sender<Result<u16, String>>) {
        let acct = msg.account.and_then(|id| self.acct(id));
        let Some(to) = SipUri::parse(&msg.to) else {
            let _ = reply.send(Err(format!("bad destination {}", msg.to)));
            return;
        };
        let fallback = acct.map(|a| a.cfg.host(a.alt).transport).unwrap_or(Transport::Udp);
        let transport = match to.param("transport").map(str::to_ascii_lowercase).as_deref() {
            Some("tcp") => Transport::Tcp,
            Some("tls") => Transport::Tls,
            Some("udp") => Transport::Udp,
            _ if to.secure => Transport::Tls,
            _ => fallback,
        };
        let port = to.port.unwrap_or(if transport == Transport::Tls { 5061 } else { 5060 });
        let from_user =
            msg.from_user.clone().or_else(|| acct.map(|a| a.cfg.user.clone())).unwrap_or_else(|| "phone".into());
        // Like stock, a MESSAGE that is not tied to an account is from the
        // phone's own address, which is also where replies go.
        let from_uri = match (acct, &msg.from_user) {
            (Some(a), None) => Core::host_uri(a, Some(&a.cfg.user)),
            _ => format!("sip:{from_user}@{}{}", self.local_hostport(), transport_param(transport)),
        };
        let contact = self.contact_uri(&from_user, acct.and_then(|a| a.public.clone()), transport);
        let mut headers: Vec<Header> =
            msg.headers.iter().map(|(k, v)| Header::Other(k.clone(), v.clone())).collect();
        let content_type = if msg.content_type.is_empty() { "text/plain".to_string() } else { msg.content_type.clone() };
        headers.push(Header::ContentType(content_type.into()));
        let built = Core::addr(&to.host, port, transport).and_then(|dest| {
            let req = self.make_request(
                Method::Message,
                &msg.to,
                &from_uri,
                &util::token(10),
                &msg.to,
                None,
                &format!("{}@opendeskphone", util::token(20)),
                1,
                transport,
                Some(contact),
                headers,
                msg.body,
            )?;
            Ok((req, dest))
        });
        let (req, dest) = match built {
            Ok(x) => x,
            Err(e) => {
                let _ = reply.send(Err(e));
                return;
            }
        };
        let (inner, cred) = (self.inner.clone(), acct.map(Core::credential));
        tokio::spawn(async move {
            let result = Core::transact(inner, req, dest, cred).await.map(|r| r.status_code.code());
            let _ = reply.send(result);
        });
    }

    /// An incoming MESSAGE: DPMA traffic goes to the registered listeners,
    /// anything else becomes a store event.
    pub fn deliver_message(&mut self, req: &Request) {
        let headers = util::header_pairs(&req.headers);
        let from = NameAddr::parse(util::header(&req.headers, "From").unwrap_or(""));
        let account = self.account_for(req);
        let is_app = headers
            .iter()
            .any(|(k, _)| k.to_ascii_lowercase().starts_with("x-digium-appserver") || k.eq_ignore_ascii_case("X-Event-Name"));
        let content_type = util::header(&req.headers, "Content-Type").unwrap_or("text/plain").to_string();
        if is_app && !self.listeners.is_empty() {
            let m = IncomingMessage {
                account,
                from_uri: from.uri.clone(),
                contact_uri: util::header(&req.headers, "Contact").map(|c| NameAddr::parse(c).uri).unwrap_or_default(),
                headers,
                content_type,
                body: req.body.clone(),
            };
            self.listeners.retain(|l| l.send(m.clone()).is_ok());
        } else {
            let number = from.user();
            self.store.emit(Event::Message {
                account: account.unwrap_or(AccountId(0)),
                from: Party { name: from.display.clone(), number, uri: from.uri.clone() },
                content_type,
                body: String::from_utf8_lossy(&req.body).into_owned(),
            });
        }
    }
}
