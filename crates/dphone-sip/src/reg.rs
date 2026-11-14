//! Registration with primary/alternate failover, keepalives, event
//! subscriptions (BLF `dialog`, `presence`, `message-summary`), NOTIFY
//! handling, and out-of-dialog MESSAGE.

use crate::bodies;
use crate::config::{AccountConfig, SipConfig};
use crate::core::{Acct, Core, Ctx, Dialog, Owner, Spec, Sub, SubKind, Timer};
use crate::util::{self, NameAddr, SipUri};
use crate::{IncomingMessage, OutMessage};
use dphone_state::{
    clock, Account, AccountId, Accounts, ActiveHost, BlfEntry, BlfState, Event, Mwi, MwiAccount, Party, Presence,
    Registration, Transport,
};
use rsipstack::sip::Response;
use rsipstack::transaction::transaction::Transaction;
use std::time::Duration;
use tokio::sync::oneshot;

const ALLOW: &str = "INVITE, ACK, CANCEL, BYE, OPTIONS, NOTIFY, REFER, MESSAGE, INFO, UPDATE";

impl Core {
    pub fn allow() -> &'static str {
        ALLOW
    }

    /// `"Display" <sip:user@server>` for an account on its current host.
    pub fn aor(&self, a: &Acct) -> String {
        let h = a.cfg.host(a.alt);
        let uri = format!("sip:{}@{}", a.cfg.user, h.server);
        if a.cfg.display_name.is_empty() {
            format!("<{uri}>")
        } else {
            format!("\"{}\" <{uri}>", a.cfg.display_name.replace('"', "'"))
        }
    }

    fn new_acct(cfg: AccountConfig) -> Acct {
        Acct {
            cfg,
            alt: false,
            call_id: format!("{}@dphone", util::token(20)),
            tag: util::token(10),
            cseq: 0,
            generation: 0,
            fails: 0,
            registered: false,
            last_ok_mono_ms: 0,
        }
    }

    /// Applies a configuration: accounts that changed are unregistered and
    /// set up again; subscriptions are rebuilt.
    pub fn configure(&mut self, cfg: SipConfig) {
        let old = std::mem::replace(&mut self.cfg, cfg);
        // A new address changes every Contact: register again.
        let moved = old.local_ip != self.cfg.local_ip;
        let max = self.cfg.model.max_accounts() as usize;
        let wanted: Vec<AccountConfig> = self.cfg.accounts.iter().take(max).cloned().collect();
        // Drop accounts that are gone or different.
        let mut keep = Vec::new();
        for a in std::mem::take(&mut self.accts) {
            if wanted.contains(&a.cfg) {
                keep.push(a);
            } else {
                if a.registered {
                    if let Some((spec, dest, ctx, cred)) = self.register_parts(&a, true) {
                        let _ = self.send(spec, dest, ctx, cred, Owner::None);
                    }
                }
                self.drop_subs(a.cfg.id);
            }
        }
        self.accts = keep;
        let mut fresh = Vec::new();
        for w in wanted {
            if self.acct(w.id).is_none() {
                fresh.push(w.id);
                self.accts.push(Self::new_acct(w));
            }
        }
        self.accts.sort_by_key(|a| a.cfg.id);
        self.publish_accounts(true);
        // BLF entries follow the configuration; subscriptions restart.
        let entries: Vec<BlfEntry> = self
            .cfg
            .blf
            .iter()
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
                    remote: Party::default(),
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
        self.sync_keys();
    }

    fn blf_uri(&self, acct: AccountId, subscribe: &str) -> Option<String> {
        if subscribe.is_empty() {
            return None;
        }
        if subscribe.starts_with("sip:") || subscribe.starts_with("sips:") {
            return Some(subscribe.to_string());
        }
        let a = self.acct(acct)?;
        Some(format!("sip:{}@{}", subscribe, a.cfg.host(a.alt).server))
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
                    Registration::Disabled
                } else {
                    match prev.get(a.cfg.id) {
                        Some(p) if !reset || a.registered => p.registration.clone(),
                        _ => Registration::Unregistered,
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
        let default_account = self
            .accts
            .iter()
            .find(|a| a.registered)
            .or(self.accts.first())
            .map(|a| a.cfg.id);
        self.store.update::<Accounts, _>(|s| {
            s.accounts = accounts;
            s.default_account = default_account;
        });
    }

    fn set_registration(&self, id: AccountId, r: Registration) {
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
        let Some(a) = self.acct_mut(id) else { return };
        a.cseq += 1;
        let registered = a.registered;
        if !registered {
            self.set_registration(id, Registration::Registering);
        }
        let parts = self.acct(id).and_then(|a| self.register_parts(a, false));
        if let Some((spec, dest, ctx, cred)) = parts {
            if let Err(e) = self.send(spec, dest, ctx, cred, Owner::Registration(id)) {
                log::warn!("sip: REGISTER: {e}");
            }
        }
    }

    /// The REGISTER (or un-REGISTER) for `a`, ready to send.
    #[allow(clippy::type_complexity)]
    fn register_parts(
        &self,
        a: &Acct,
        unregister: bool,
    ) -> Option<(Spec, rsipstack::transport::SipAddr, Ctx, Option<(String, String)>)> {
        let h = a.cfg.host(a.alt);
        let expires = if unregister { 0 } else { h.expires_secs };
        let aor = self.aor(a);
        let spec = Spec {
            method: "REGISTER".into(),
            ruri: format!("sip:{}", h.server),
            from: format!("{aor};tag={}", a.tag),
            to: aor,
            call_id: a.call_id.clone(),
            cseq: if unregister { a.cseq + 1 } else { a.cseq },
            routes: Vec::new(),
            headers: vec![
                ("Contact".into(), format!("{};expires={expires}", self.contact(&a.cfg.user, h.transport))),
                ("Expires".into(), expires.to_string()),
                ("Allow".into(), ALLOW.into()),
            ],
            content_type: String::new(),
            body: Vec::new(),
            transport: h.transport,
        };
        let dest = match self.account_dest(a) {
            Ok((d, _)) => d,
            Err(e) => {
                log::warn!("sip: account {}: {e}", a.cfg.user);
                return None;
            }
        };
        let ctx = Ctx::Register { acct: a.cfg.id, generation: a.generation, unregister };
        Some((spec, dest, ctx, Some(a.cfg.credentials(a.alt))))
    }

    pub fn on_register_final(
        &mut self,
        id: AccountId,
        generation: u64,
        unregister: bool,
        code: u16,
        reason: &str,
        resp: Option<&Response>,
    ) {
        if unregister {
            return;
        }
        let Some(a) = self.acct_mut(id) else { return };
        if a.generation != generation {
            return;
        }
        let h = a.cfg.host(a.alt).clone();
        if (200..300).contains(&code) {
            let was = a.registered;
            a.registered = true;
            a.fails = 0;
            a.last_ok_mono_ms = clock::mono_ms();
            let alt = a.alt;
            let user = a.cfg.user.clone();
            // The granted lifetime: our Contact's expires parameter, else
            // the Expires header, else what we asked for.
            let granted = resp
                .and_then(|r| {
                    util::headers_all(&r.headers, "Contact")
                        .into_iter()
                        .flat_map(|v| v.split(',').map(NameAddr::parse).collect::<Vec<_>>())
                        .find(|c| c.user() == user)
                        .and_then(|c| c.param("expires").and_then(|e| e.parse::<u32>().ok()))
                        .or_else(|| util::header(&r.headers, "Expires").and_then(|e| e.trim().parse().ok()))
                })
                .unwrap_or(h.expires_secs)
                .max(10);
            // Stock refreshes at half the lifetime.
            self.after(Duration::from_secs((granted / 2).max(5) as u64), Timer::Register { acct: id, generation });
            self.set_registration(
                id,
                Registration::Registered {
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
        // Failure.
        if code == 423 {
            if let Some(min) = resp.and_then(|r| util::header(&r.headers, "Min-Expires")).and_then(|m| m.trim().parse::<u32>().ok()) {
                let alt = a.alt;
                if alt {
                    if let Some(h) = a.cfg.alternate.as_mut() {
                        h.expires_secs = min;
                    }
                } else {
                    a.cfg.primary.expires_secs = min;
                }
                self.register_now(id);
                return;
            }
        }
        let was = a.registered;
        a.registered = false;
        a.fails += 1;
        let mut delay = h.retry_secs.max(1) as u64;
        if a.cfg.alternate.is_some() && a.fails >= h.num_retries.max(1) {
            a.alt = !a.alt;
            a.fails = 0;
            a.call_id = format!("{}@dphone", util::token(20));
            a.cseq = 0;
            delay = 1;
            log::warn!(
                "sip: account {}: {} failed ({code} {reason}); switching to {}",
                a.cfg.user,
                h.server,
                a.cfg.host(a.alt).server
            );
        } else {
            log::warn!("sip: account {}: registration failed: {code} {reason}", a.cfg.user);
        }
        if was {
            self.drop_subs(id);
        }
        self.set_registration(
            id,
            Registration::Failed {
                code: if resp.is_some() { code } else { 0 },
                reason: reason.to_string(),
                retry_mono_ms: clock::mono_ms() + delay * 1000,
            },
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
            a.call_id = format!("{}@dphone", util::token(20));
            a.cseq = 0;
        }
        self.register_now(id);
    }

    pub fn on_keepalive_timer(&mut self, id: AccountId, generation: u64) {
        let Some(a) = self.acct(id) else { return };
        if a.generation != generation || !a.registered || self.cfg.udp_keepalive_secs == 0 {
            return;
        }
        let h = a.cfg.host(a.alt);
        let aor = self.aor(a);
        let spec = Spec {
            method: "OPTIONS".into(),
            ruri: format!("sip:{}", h.server),
            from: format!("{aor};tag={}", util::token(10)),
            to: aor,
            call_id: format!("{}@dphone", util::token(20)),
            cseq: 1,
            routes: Vec::new(),
            headers: vec![("Accept".into(), "application/sdp".into())],
            content_type: String::new(),
            body: Vec::new(),
            transport: h.transport,
        };
        if let Ok((dest, _)) = self.account_dest(a) {
            let _ = self.send(spec, dest, Ctx::Other("keepalive"), None, Owner::None);
        }
        self.after(Duration::from_secs(self.cfg.udp_keepalive_secs as u64), Timer::Keepalive { acct: id, generation });
    }

    /// Un-registers every account (shutdown).
    pub fn unregister_all(&mut self) {
        for i in 0..self.accts.len() {
            if self.accts[i].registered {
                if let Some((spec, dest, ctx, cred)) = self.register_parts(&self.accts[i], true) {
                    let _ = self.send(spec, dest, ctx, cred, Owner::None);
                }
            }
            self.accts[i].registered = false;
            self.accts[i].generation += 1;
        }
    }

    // ----------------------------------------------------- subscriptions

    fn drop_subs(&mut self, acct: AccountId) {
        // Subscriptions simply stop being refreshed; the server expires
        // them.  (An explicit Expires: 0 would need the dialog state of
        // subscriptions that may not have completed.)
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
            let uri = if a.cfg.mwi_uri.is_empty() {
                format!("sip:{}@{}", a.cfg.user, a.cfg.host(a.alt).server)
            } else {
                a.cfg.mwi_uri.clone()
            };
            wanted.push((SubKind::Mwi, uri));
        }
        for b in self.cfg.blf.iter().filter(|b| b.account == acct) {
            if let Some(uri) = self.blf_uri(acct, &b.subscribe) {
                wanted.push((SubKind::Blf(b.id.clone()), uri.clone()));
                if b.presence {
                    wanted.push((SubKind::Presence(b.id.clone()), uri));
                }
            }
        }
        for (kind, uri) in wanted {
            self.next_sub += 1;
            let id = self.next_sub;
            self.subs.push(Sub { id, kind, acct, uri, dialog: Dialog::default(), generation: 0, established: false });
            self.subscribe(id);
        }
    }

    /// Sends the initial SUBSCRIBE or a refresh.
    pub fn subscribe(&mut self, id: u32) {
        let expires = self.cfg.subscribe_expires_secs;
        let Some(s) = self.subs.iter_mut().find(|s| s.id == id) else { return };
        let Some(a) = self.accts.iter().find(|a| a.cfg.id == s.acct) else { return };
        s.generation += 1;
        let generation = s.generation;
        let h = a.cfg.host(a.alt);
        let (event, accept) = match s.kind {
            SubKind::Blf(_) => ("dialog", "application/dialog-info+xml"),
            SubKind::Presence(_) => ("presence", "application/pidf+xml"),
            SubKind::Mwi => ("message-summary", "application/simple-message-summary"),
        };
        if !s.established {
            let uri = format!("sip:{}@{}", a.cfg.user, h.server);
            s.dialog = Dialog {
                call_id: format!("{}@dphone", util::token(20)),
                local_tag: util::token(10),
                remote_tag: String::new(),
                local: format!("<{uri}>"),
                remote: format!("<{}>", s.uri),
                target: s.uri.clone(),
                routes: Vec::new(),
                cseq: 0,
            };
        }
        let cseq = s.dialog.next_cseq();
        let d = s.dialog.clone();
        let established = s.established;
        let acct = s.acct;
        let (user, transport) = (a.cfg.user.clone(), h.transport);
        let cred = Some(a.cfg.credentials(a.alt));
        let dest = if established { Core::dialog_dest(&d, transport).map(|d| d.0) } else { self.request_dest(a, &d.target).map(|d| d.0) };
        let spec = Spec {
            method: "SUBSCRIBE".into(),
            ruri: d.target.clone(),
            from: d.local_header(),
            to: d.remote_header(),
            call_id: d.call_id.clone(),
            cseq,
            routes: d.routes.clone(),
            headers: vec![
                ("Contact".into(), self.contact(&user, transport)),
                ("Event".into(), event.into()),
                ("Accept".into(), accept.into()),
                ("Expires".into(), expires.to_string()),
            ],
            content_type: String::new(),
            body: Vec::new(),
            transport,
        };
        let _ = acct;
        match dest {
            Ok(dest) => {
                if let Err(e) = self.send(spec, dest, Ctx::Subscribe { sub: id, generation }, cred, Owner::Sub(id)) {
                    log::warn!("sip: SUBSCRIBE: {e}");
                }
            }
            Err(e) => log::warn!("sip: SUBSCRIBE: {e}"),
        }
    }

    pub fn on_subscribe_final(&mut self, id: u32, generation: u64, code: u16, resp: Option<&Response>) {
        let expires_default = self.cfg.subscribe_expires_secs;
        let Some(s) = self.subs.iter_mut().find(|s| s.id == id) else { return };
        if s.generation != generation {
            return;
        }
        if (200..300).contains(&code) {
            if let Some(r) = resp {
                if let Some(tag) = util::header(&r.headers, "To").and_then(|t| NameAddr::parse(t).tag().map(str::to_string)) {
                    s.dialog.remote_tag = tag;
                }
                if let Some(c) = util::header(&r.headers, "Contact") {
                    s.dialog.target = NameAddr::parse(c).uri;
                }
                if !s.established {
                    let mut routes: Vec<String> = util::headers_all(&r.headers, "Record-Route")
                        .into_iter()
                        .flat_map(|v| v.split(',').map(|x| x.trim().to_string()).collect::<Vec<_>>())
                        .collect();
                    routes.reverse();
                    s.dialog.routes = routes;
                }
            }
            s.established = true;
            let granted = resp
                .and_then(|r| util::header(&r.headers, "Expires"))
                .and_then(|e| e.trim().parse::<u32>().ok())
                .unwrap_or(expires_default)
                .max(30);
            if let SubKind::Blf(entry) | SubKind::Presence(entry) = &s.kind {
                let entry = entry.clone();
                self.store.update::<Presence, _>(|p| {
                    if let Some(e) = p.entries.iter_mut().find(|e| e.id == entry) {
                        e.subscribed = true;
                    }
                });
            }
            self.after(Duration::from_secs(granted as u64 * 4 / 5), Timer::Subscribe { sub: id, generation });
        } else {
            // 481: the server lost the dialog; anything else: try again
            // later with a new one.
            s.established = false;
            let delay = if code == 481 { 1 } else { 60 };
            if let SubKind::Blf(entry) | SubKind::Presence(entry) = &s.kind {
                let entry = entry.clone();
                self.store.update::<Presence, _>(|p| {
                    if let Some(e) = p.entries.iter_mut().find(|e| e.id == entry) {
                        e.subscribed = false;
                        e.state = BlfState::Unknown;
                    }
                });
                self.sync_keys();
            }
            self.after(Duration::from_secs(delay), Timer::Subscribe { sub: id, generation });
        }
    }

    pub fn on_subscribe_timer(&mut self, id: u32, generation: u64) {
        if self.subs.iter().any(|s| s.id == id && s.generation == generation) {
            self.subscribe(id);
        }
    }

    /// An incoming NOTIFY that is not for a call's REFER.
    pub fn on_notify(&mut self, tx: Transaction) {
        let req = &tx.original;
        let call_id = util::header(&req.headers, "Call-ID").unwrap_or("").to_string();
        let event = util::header(&req.headers, "Event").unwrap_or("").split(';').next().unwrap_or("").trim().to_ascii_lowercase();
        let body = String::from_utf8_lossy(&req.body).into_owned();
        let state = util::header(&req.headers, "Subscription-State").unwrap_or("").to_ascii_lowercase();
        let from_tag = util::header(&req.headers, "From").and_then(|f| NameAddr::parse(f).tag().map(str::to_string));
        let contact = util::header(&req.headers, "Contact").map(|c| NameAddr::parse(c).uri);

        let Some(idx) = self.subs.iter().position(|s| s.dialog.call_id == call_id) else {
            if event == "message-summary" {
                // Unsolicited MWI.
                if let Some(acct) = self.account_for(req) {
                    self.apply_mwi(acct, &body);
                }
                return self.respond_code(tx, 200);
            }
            return self.respond_code(tx, 481);
        };
        let s = &mut self.subs[idx];
        if let Some(t) = from_tag {
            if s.dialog.remote_tag.is_empty() {
                s.dialog.remote_tag = t;
            }
        }
        if let (Some(c), false) = (contact, s.established) {
            s.dialog.target = c;
        }
        let (id, acct) = (s.id, s.acct);
        let terminated = state.starts_with("terminated");
        match &s.kind {
            SubKind::Blf(entry) => {
                let entry = entry.clone();
                if let Some(info) = bodies::parse_dialog_info(&body) {
                    self.store.update::<Presence, _>(|p| {
                        if let Some(e) = p.entries.iter_mut().find(|e| e.id == entry) {
                            e.state = info.state;
                            e.remote = if info.state == BlfState::Ringing { info.remote.clone() } else { Party::default() };
                            e.subscribed = !terminated;
                        }
                    });
                    self.blf_dialogs.insert(entry, info);
                    self.sync_keys();
                }
            }
            SubKind::Presence(entry) => {
                let entry = entry.clone();
                if let Some(doc) = bodies::parse_presence(&body) {
                    self.store.update::<Presence, _>(|p| {
                        if let Some(e) = p.entries.iter_mut().find(|e| e.id == entry) {
                            e.status = doc.status;
                            e.status_text = doc.note.clone();
                        }
                    });
                }
            }
            SubKind::Mwi => self.apply_mwi(acct, &body),
        }
        if terminated {
            let s = &mut self.subs[idx];
            s.established = false;
            s.generation += 1;
            let generation = s.generation;
            let delay = if state.contains("reason=rejected") || state.contains("reason=noresource") { 300 } else { 5 };
            self.after(Duration::from_secs(delay), Timer::Subscribe { sub: id, generation });
        }
        self.respond_code(tx, 200);
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
        let dest = match Core::addr(&to.host, port, transport) {
            Ok(d) => d,
            Err(e) => {
                let _ = reply.send(Err(e));
                return;
            }
        };
        let from_user = msg
            .from_user
            .clone()
            .or_else(|| acct.map(|a| a.cfg.user.clone()))
            .unwrap_or_else(|| "phone".into());
        // Like stock, a MESSAGE that is not tied to an account is from the
        // phone's own address, which is also where replies go.
        let from_uri = match (acct, &msg.from_user) {
            (Some(a), None) => format!("sip:{}@{}", a.cfg.user, a.cfg.host(a.alt).server),
            _ => format!("sip:{from_user}@{}", self.local_hostport()),
        };
        let mut headers = vec![("Contact".to_string(), self.contact(&from_user, transport))];
        headers.extend(msg.headers.iter().cloned());
        let spec = Spec {
            method: "MESSAGE".into(),
            ruri: msg.to.clone(),
            from: format!("<{from_uri}>;tag={}", util::token(10)),
            to: format!("<{}>", msg.to),
            call_id: format!("{}@dphone", util::token(20)),
            cseq: 1,
            routes: Vec::new(),
            headers,
            content_type: if msg.content_type.is_empty() { "text/plain".into() } else { msg.content_type.clone() },
            body: msg.body,
            transport,
        };
        let cred = acct.map(|a| a.cfg.credentials(a.alt));
        // The reply channel moves into the transaction context; a send
        // error is logged and the dropped channel tells the caller.
        if let Err(e) = self.send(spec, dest, Ctx::Message { reply: Some(reply) }, cred, Owner::None) {
            log::warn!("sip: MESSAGE: {e}");
        }
    }

    /// An incoming MESSAGE: DPMA traffic goes to the registered listeners,
    /// anything else becomes a store event.
    pub fn on_message(&mut self, tx: Transaction) {
        let req = &tx.original;
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
        self.respond_code(tx, 200);
    }
}
