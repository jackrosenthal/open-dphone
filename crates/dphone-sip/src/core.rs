//! The user-agent core: one task that owns all signaling state.  Helper
//! tasks drive individual transactions and timers and post what happens
//! back as [`Ev`]s, so nothing here needs a lock.

use crate::config::{AccountConfig, SipConfig};
use crate::media::{MediaControl, MediaEvent};
use crate::sdp::{Crypto, Negotiated, Sdp};
use crate::util::{self, NameAddr, SipUri};
use crate::{digest, IncomingMessage, OutMessage};
use dphone_state::{
    AccountId, Blink, CallId, Envelope, Keys, LampColor, LampState, LineKey, LineKeyBinding, RapidDialKey,
    Store, Subsystem, Transport,
};
use rsipstack::sip::{Header, Request, Response, StatusCode};
use rsipstack::transaction::endpoint::EndpointInnerRef;
use rsipstack::transaction::key::{TransactionKey, TransactionRole};
use rsipstack::transaction::transaction::{Transaction, TransactionEvent, TransactionEventSender};
use rsipstack::transport::SipAddr;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

/// Everything that can happen to the core.
pub(crate) enum Ev {
    Cmd(Envelope),
    Incoming(Transaction),
    Resp { id: u64, resp: Box<Response> },
    TxDone { id: u64 },
    /// An ACK or CANCEL on one of our server INVITE transactions.
    ServerReq { tx: u64, req: Box<Request> },
    ServerDone { tx: u64 },
    Timer(Timer),
    Media(MediaEvent),
    Reconfigure(Box<SipConfig>),
    SendMessage { msg: OutMessage, reply: oneshot::Sender<Result<u16, String>> },
    Subscribe(mpsc::UnboundedSender<IncomingMessage>),
    Shutdown(oneshot::Sender<()>),
}

#[derive(Clone, Debug)]
pub(crate) enum Timer {
    Register { acct: AccountId, generation: u64 },
    Keepalive { acct: AccountId, generation: u64 },
    Subscribe { sub: u32, generation: u64 },
    NoAnswer { call: CallId },
    Linger { call: CallId },
    Glare { call: CallId },
    Heartbeat,
}

/// What a client transaction was for.
pub(crate) enum Ctx {
    Register { acct: AccountId, generation: u64, unregister: bool },
    Invite { call: CallId },
    Reinvite { call: CallId, hold: bool },
    Bye,
    Cancel,
    Refer { call: CallId, consult: Option<CallId> },
    Subscribe { sub: u32, generation: u64 },
    Message { reply: Option<oneshot::Sender<Result<u16, String>>> },
    /// Requests whose outcome only matters for logging.
    Other(&'static str),
}

/// The parts of a request that survive an authentication retry.
#[derive(Clone, Debug)]
pub(crate) struct Spec {
    pub method: String,
    pub ruri: String,
    /// Full From value including the tag.
    pub from: String,
    /// Full To value (with a tag when in a dialog).
    pub to: String,
    pub call_id: String,
    pub cseq: u32,
    pub routes: Vec<String>,
    /// Further headers, in order.
    pub headers: Vec<(String, String)>,
    pub content_type: String,
    pub body: Vec<u8>,
    pub transport: Transport,
}

pub(crate) struct Pending {
    pub spec: Spec,
    pub dest: SipAddr,
    pub ctx: Ctx,
    /// Digest user and password, when we may answer a challenge.
    pub cred: Option<(String, String)>,
    pub tries: u8,
    pub branch: String,
    pub done: bool,
    /// Which account's (or call's) CSeq space the request uses.
    pub owner: Owner,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Owner {
    None,
    Registration(AccountId),
    Call(CallId),
    Sub(u32),
}

/// Dialog state (RFC 3261 section 12).
#[derive(Clone, Debug, Default)]
pub(crate) struct Dialog {
    pub call_id: String,
    pub local_tag: String,
    pub remote_tag: String,
    /// name-addr of ourselves and the peer, without tags.
    pub local: String,
    pub remote: String,
    /// The peer's Contact URI.
    pub target: String,
    /// Route set, in the order to use for requests.
    pub routes: Vec<String>,
    pub cseq: u32,
}

impl Dialog {
    pub fn local_header(&self) -> String {
        format!("{};tag={}", self.local, self.local_tag)
    }
    pub fn remote_header(&self) -> String {
        if self.remote_tag.is_empty() {
            self.remote.clone()
        } else {
            format!("{};tag={}", self.remote, self.remote_tag)
        }
    }
    pub fn next_cseq(&mut self) -> u32 {
        self.cseq += 1;
        self.cseq
    }
}

/// Registration state of one account.
pub(crate) struct Acct {
    pub cfg: AccountConfig,
    /// Using the alternate host.
    pub alt: bool,
    pub call_id: String,
    pub tag: String,
    pub cseq: u32,
    pub generation: u64,
    pub fails: u32,
    pub registered: bool,
    /// Wall-clock-free time of the last successful registration.
    pub last_ok_mono_ms: u64,
}

/// An unanswered (or just answered) incoming INVITE transaction.
pub(crate) struct ServerInvite {
    pub tx: u64,
    pub tu: TransactionEventSender,
    pub req: Request,
}

/// A REFER we received and are acting on: the NOTIFYs go back on the
/// referring call's dialog.
#[derive(Clone, Debug)]
pub(crate) struct ReferFrom {
    pub call: CallId,
}

pub(crate) struct CallRt {
    pub id: CallId,
    pub acct: AccountId,
    pub alt: bool,
    pub dialog: Dialog,
    pub outgoing: bool,
    /// Pending client INVITE (for CANCEL).
    pub invite: Option<u64>,
    pub provisional_seen: bool,
    pub cancel_wanted: bool,
    pub server: Option<ServerInvite>,
    /// A re-INVITE of theirs that we have answered and whose ACK may carry
    /// the answer.
    pub reinvite_server: Option<ServerInvite>,
    pub rtp_port: u16,
    pub session_id: u64,
    pub sdp_version: u64,
    /// Our outstanding offer.
    pub offer: Option<Sdp>,
    /// Their offer, kept until we answer.
    pub remote_offer: Option<Sdp>,
    pub key: Option<Crypto>,
    pub neg: Option<Negotiated>,
    pub local_hold: bool,
    pub remote_hold: bool,
    /// A re-INVITE of ours is in flight.
    pub reinvite_busy: bool,
    /// The hold state to move to once the current re-INVITE finishes.
    pub hold_wanted: Option<bool>,
    pub answered: bool,
    pub ended: bool,
    pub alert_info: String,
    /// We are acting on a REFER from this call.
    pub refer_from: Option<ReferFrom>,
    /// `Replaces` value for the INVITE (attended transfer target, pickup).
    pub replaces: Option<String>,
    pub referred_by: Option<String>,
    pub target_text: String,
}

pub(crate) enum SubKind {
    /// `dialog` event for a BLF entry.
    Blf(String),
    /// `presence` event for a BLF entry.
    Presence(String),
    Mwi,
}

pub(crate) struct Sub {
    pub id: u32,
    pub kind: SubKind,
    pub acct: AccountId,
    pub uri: String,
    pub dialog: Dialog,
    pub generation: u64,
    pub established: bool,
}

pub(crate) struct Core {
    pub store: Store,
    pub cfg: SipConfig,
    pub media: Arc<dyn MediaControl>,
    pub inner: EndpointInnerRef,
    pub tx: mpsc::UnboundedSender<Ev>,
    pub local_port: u16,
    pub next_id: u64,
    pub pending: HashMap<u64, Pending>,
    pub accts: Vec<Acct>,
    pub calls: Vec<CallRt>,
    pub subs: Vec<Sub>,
    pub next_sub: u32,
    pub conference: Vec<CallId>,
    pub last_dialed: Option<(String, Option<AccountId>)>,
    pub listeners: Vec<mpsc::UnboundedSender<IncomingMessage>>,
    pub nonce_count: u32,
    pub park_extension: String,
    /// The latest dialog-info per BLF entry, for directed pickup.
    pub blf_dialogs: HashMap<String, crate::bodies::DialogInfo>,
}

impl Core {
    pub fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Posts `t` after `d`.
    pub fn after(&self, d: Duration, t: Timer) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(d).await;
            let _ = tx.send(Ev::Timer(t));
        });
    }

    pub fn acct(&self, id: AccountId) -> Option<&Acct> {
        self.accts.iter().find(|a| a.cfg.id == id)
    }
    pub fn acct_mut(&mut self, id: AccountId) -> Option<&mut Acct> {
        self.accts.iter_mut().find(|a| a.cfg.id == id)
    }
    pub fn call(&self, id: CallId) -> Option<&CallRt> {
        self.calls.iter().find(|c| c.id == id)
    }
    pub fn call_mut(&mut self, id: CallId) -> Option<&mut CallRt> {
        self.calls.iter_mut().find(|c| c.id == id)
    }

    pub fn transport_param(t: Transport) -> &'static str {
        match t {
            Transport::Udp => "",
            Transport::Tcp => ";transport=tcp",
            Transport::Tls => ";transport=tls",
        }
    }

    /// Our Contact for `user` over `t`.
    pub fn contact(&self, user: &str, t: Transport) -> String {
        format!("<sip:{user}@{}{}>", self.local_hostport(), Self::transport_param(t))
    }

    pub fn local_hostport(&self) -> String {
        match self.cfg.local_ip {
            std::net::IpAddr::V4(a) => format!("{a}:{}", self.local_port),
            std::net::IpAddr::V6(a) => format!("[{a}]:{}", self.local_port),
        }
    }

    /// The transport address for `host:port` over `t`.
    pub fn addr(host: &str, port: u16, t: Transport) -> Result<SipAddr, String> {
        use rsipstack::sip::{Host, HostWithPort};
        let host = Host::try_from(host.trim_matches(|c| c == '[' || c == ']'))
            .map_err(|e| format!("bad host {host}: {e}"))?;
        Ok(SipAddr {
            r#type: Some(match t {
                Transport::Udp => rsipstack::sip::Transport::Udp,
                Transport::Tcp => rsipstack::sip::Transport::Tcp,
                Transport::Tls => rsipstack::sip::Transport::Tls,
            }),
            addr: HostWithPort { host, port: Some(port.into()) },
        })
    }

    /// Where out-of-dialog requests of an account go, and how.
    pub fn account_dest(&self, a: &Acct) -> Result<(SipAddr, Transport), String> {
        let h = a.cfg.host(a.alt);
        if !a.cfg.outbound_proxy.is_empty() {
            let u = SipUri::parse(&format!("sip:{}", a.cfg.outbound_proxy)).ok_or("bad outbound proxy")?;
            let port = u.port.unwrap_or(if h.transport == Transport::Tls { 5061 } else { 5060 });
            return Ok((Self::addr(&u.host, port, h.transport)?, h.transport));
        }
        Ok((Self::addr(&h.server, h.effective_port(), h.transport)?, h.transport))
    }

    /// Where a new request for `uri` goes: the outbound proxy when one is
    /// configured, else wherever the URI itself points (RFC 3261 8.1.2).
    /// A URI at the account's own server without a port uses the
    /// configured port.
    pub fn request_dest(&self, a: &Acct, uri: &str) -> Result<(SipAddr, Transport), String> {
        if !a.cfg.outbound_proxy.is_empty() {
            return self.account_dest(a);
        }
        let h = a.cfg.host(a.alt);
        let u = SipUri::parse(uri).ok_or_else(|| format!("bad URI {uri}"))?;
        let t = match u.param("transport").map(str::to_ascii_lowercase).as_deref() {
            Some("tcp") => Transport::Tcp,
            Some("tls") => Transport::Tls,
            Some("udp") => Transport::Udp,
            _ if u.secure => Transport::Tls,
            _ => h.transport,
        };
        let default = if t == Transport::Tls { 5061 } else { 5060 };
        let port = match u.port {
            Some(p) => p,
            None if u.host.eq_ignore_ascii_case(&h.server) && t == h.transport => h.effective_port(),
            None => default,
        };
        Ok((Self::addr(&u.host, port, t)?, t))
    }

    /// Where an in-dialog request goes: the first route, else the remote
    /// target; the transport is the URI's, else `fallback`.
    pub fn dialog_dest(d: &Dialog, fallback: Transport) -> Result<(SipAddr, Transport), String> {
        let hop = match d.routes.first() {
            Some(r) => NameAddr::parse(r).uri,
            None => d.target.clone(),
        };
        let u = SipUri::parse(&hop).ok_or_else(|| format!("bad next hop {hop}"))?;
        let t = match u.param("transport").map(str::to_ascii_lowercase).as_deref() {
            Some("tcp") => Transport::Tcp,
            Some("tls") => Transport::Tls,
            Some("udp") => Transport::Udp,
            _ if u.secure => Transport::Tls,
            _ => fallback,
        };
        let port = u.port.unwrap_or(if t == Transport::Tls { 5061 } else { 5060 });
        Ok((Self::addr(&u.host, port, t)?, t))
    }

    /// Serializes `spec` with a new Via and parses it into the stack's
    /// request type.
    pub fn build(&self, spec: &Spec, branch: &str, auth: Option<&(String, String)>) -> Result<Request, String> {
        let via_t = match spec.transport {
            Transport::Udp => "UDP",
            Transport::Tcp => "TCP",
            Transport::Tls => "TLS",
        };
        let mut s = String::with_capacity(512 + spec.body.len());
        s.push_str(&format!("{} {} SIP/2.0\r\n", spec.method, spec.ruri));
        s.push_str(&format!("Via: SIP/2.0/{via_t} {};branch={branch};rport\r\n", self.local_hostport()));
        for r in &spec.routes {
            s.push_str(&format!("Route: {r}\r\n"));
        }
        s.push_str("Max-Forwards: 70\r\n");
        s.push_str(&format!("From: {}\r\nTo: {}\r\nCall-ID: {}\r\n", spec.from, spec.to, spec.call_id));
        s.push_str(&format!("CSeq: {} {}\r\n", spec.cseq, spec.method));
        s.push_str(&format!("User-Agent: {}\r\n", self.cfg.user_agent()));
        for (k, v) in &spec.headers {
            s.push_str(&format!("{k}: {v}\r\n"));
        }
        if let Some((k, v)) = auth {
            s.push_str(&format!("{k}: {v}\r\n"));
        }
        if !spec.body.is_empty() {
            s.push_str(&format!("Content-Type: {}\r\n", spec.content_type));
        }
        s.push_str("Content-Length: 0\r\n\r\n");
        let mut req = Request::try_from(s).map_err(|e| format!("cannot build {}: {e}", spec.method))?;
        req.body = spec.body.clone();
        Ok(req)
    }

    /// Sends a request in a new client transaction; responses come back as
    /// [`Ev::Resp`] with the returned ID.
    pub fn send(
        &mut self,
        spec: Spec,
        dest: SipAddr,
        ctx: Ctx,
        cred: Option<(String, String)>,
        owner: Owner,
    ) -> Result<u64, String> {
        self.send_with(spec, dest, ctx, cred, owner, 0, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn send_with(
        &mut self,
        spec: Spec,
        dest: SipAddr,
        ctx: Ctx,
        cred: Option<(String, String)>,
        owner: Owner,
        tries: u8,
        auth: Option<(String, String)>,
    ) -> Result<u64, String> {
        let branch = util::branch();
        let req = self.build(&spec, &branch, auth.as_ref())?;
        let id = self.id();
        self.drive(id, req, dest.clone())?;
        self.pending.insert(id, Pending { spec, dest, ctx, cred, tries, branch, done: false, owner });
        Ok(id)
    }

    /// Spawns the task that runs one client transaction.
    fn drive(&self, id: u64, req: Request, dest: SipAddr) -> Result<(), String> {
        let key = TransactionKey::from_request(&req, TransactionRole::Client).map_err(|e| e.to_string())?;
        let invite = req.method == rsipstack::sip::Method::Invite;
        let mut tx = Transaction::new_client(key, req, self.inner.clone(), None);
        tx.destination = Some(dest);
        let post = self.tx.clone();
        tokio::spawn(async move {
            if let Err(e) = tx.send().await {
                log::warn!("sip: send failed: {e}");
                let _ = post.send(Ev::TxDone { id });
                return;
            }
            while let Some(msg) = tx.receive().await {
                if let rsipstack::sip::SipMessage::Response(r) = msg {
                    let last = r.status_code.code() >= 200;
                    let _ = post.send(Ev::Resp { id, resp: Box::new(r) });
                    // An INVITE transaction lives on to ACK retransmitted
                    // finals; the others are finished.
                    if last && !invite {
                        break;
                    }
                }
            }
            let _ = post.send(Ev::TxDone { id });
        });
        Ok(())
    }

    /// CANCEL for a pending INVITE: same branch, CSeq number, and target.
    pub fn send_cancel(&mut self, invite: u64) {
        let Some(p) = self.pending.get(&invite) else { return };
        let mut spec = p.spec.clone();
        spec.method = "CANCEL".into();
        spec.body.clear();
        spec.headers.clear();
        let (branch, dest) = (p.branch.clone(), p.dest.clone());
        let req = match self.build(&spec, &branch, None) {
            Ok(r) => r,
            Err(e) => return log::warn!("sip: {e}"),
        };
        let id = self.id();
        if self.drive(id, req, dest.clone()).is_ok() {
            self.pending.insert(
                id,
                Pending { spec, dest, ctx: Ctx::Cancel, cred: None, tries: 0, branch, done: false, owner: Owner::None },
            );
        }
    }

    /// Takes the next CSeq from whatever owns the request.
    fn bump_cseq(&mut self, owner: Owner, old: u32) -> u32 {
        match owner {
            Owner::Registration(a) => self.acct_mut(a).map(|a| {
                a.cseq += 1;
                a.cseq
            }),
            Owner::Call(c) => self.call_mut(c).map(|c| c.dialog.next_cseq()),
            Owner::Sub(s) => self.subs.iter_mut().find(|x| x.id == s).map(|x| x.dialog.next_cseq()),
            Owner::None => None,
        }
        .unwrap_or(old + 1)
    }

    pub fn on_resp(&mut self, id: u64, resp: Response) {
        let code = resp.status_code.code();
        let Some(p) = self.pending.get_mut(&id) else { return };
        if p.done {
            return;
        }
        if code < 200 {
            if let Ctx::Invite { call } = p.ctx {
                self.on_invite_provisional(call, &resp);
            }
            return;
        }
        p.done = true;
        let Some(mut p) = self.pending.remove(&id) else { return };
        if (code == 401 || code == 407) && p.tries < 2 {
            let (hname, aname) = if code == 401 {
                ("WWW-Authenticate", "Authorization")
            } else {
                ("Proxy-Authenticate", "Proxy-Authorization")
            };
            let challenge = util::headers_all(&resp.headers, hname).into_iter().find_map(digest::Challenge::parse);
            if let (Some(c), Some((user, pass))) = (challenge, p.cred.clone()) {
                self.nonce_count += 1;
                let value =
                    digest::authorize(&c, &user, &pass, &p.spec.method, &p.spec.ruri, &util::token(8), self.nonce_count);
                p.spec.cseq = self.bump_cseq(p.owner, p.spec.cseq);
                let (owner, tries) = (p.owner, p.tries + 1);
                let is_invite = matches!(p.ctx, Ctx::Invite { .. });
                let call = match p.ctx {
                    Ctx::Invite { call } => Some(call),
                    _ => None,
                };
                match self.send_with(p.spec, p.dest, p.ctx, p.cred, owner, tries, Some((aname.into(), value))) {
                    Ok(new) => {
                        if let (true, Some(c)) = (is_invite, call.and_then(|c| self.call_mut(c))) {
                            c.invite = Some(new);
                            c.provisional_seen = false;
                        }
                    }
                    Err(e) => log::warn!("sip: authentication retry failed: {e}"),
                }
                return;
            }
        }
        let reason = resp.reason_phrase().map(str::to_string).unwrap_or_else(|| resp.status_code.to_string());
        self.on_final(p, code, reason, Some(&resp));
    }

    pub fn on_tx_done(&mut self, id: u64) {
        // A transaction that ended without a final response timed out or
        // could not be sent.
        if let Some(p) = self.pending.remove(&id) {
            if !p.done {
                self.on_final(p, 408, "Request Timeout".into(), None);
            }
        }
    }

    fn on_final(&mut self, p: Pending, code: u16, reason: String, resp: Option<&Response>) {
        match p.ctx {
            Ctx::Register { acct, generation, unregister } => {
                self.on_register_final(acct, generation, unregister, code, &reason, resp)
            }
            Ctx::Invite { call } => self.on_invite_final(call, code, &reason, resp),
            Ctx::Reinvite { call, hold } => self.on_reinvite_final(call, hold, code, resp),
            Ctx::Refer { call, consult } => self.on_refer_final(call, consult, code, &reason),
            Ctx::Subscribe { sub, generation } => self.on_subscribe_final(sub, generation, code, resp),
            Ctx::Message { reply } => {
                if let Some(r) = reply {
                    let _ = r.send(if code == 408 && resp.is_none() {
                        Err("no answer to the MESSAGE".into())
                    } else {
                        Ok(code)
                    });
                }
            }
            Ctx::Bye | Ctx::Cancel => {}
            Ctx::Other(what) => {
                if code >= 300 {
                    log::debug!("sip: {what} answered {code} {reason}");
                }
            }
        }
    }

    // ------------------------------------------------------- server side

    /// A response to `req` with our User-Agent, `extra` headers, and an
    /// optional body.  `to_tag` is added to the To header when it has none.
    pub fn response(
        &self,
        req: &Request,
        code: u16,
        to_tag: Option<&str>,
        extra: &[(&str, String)],
        body: Option<(&str, Vec<u8>)>,
    ) -> Response {
        let (ctype, body) = match body {
            Some((c, b)) => (Some(c.to_string()), Some(b)),
            None => (None, None),
        };
        let mut resp = self.inner.make_response(req, StatusCode::from(code), body);
        if let Some(tag) = to_tag {
            for h in resp.headers.iter_mut() {
                if let Header::To(to) = h {
                    let v = to.value().to_string();
                    if NameAddr::parse(&v).tag().is_none() {
                        *h = Header::To(format!("{v};tag={tag}").into());
                    }
                    break;
                }
            }
        }
        for (k, v) in extra {
            if let Some(h) = make_header(k, v) {
                resp.headers.push(h);
            }
        }
        if let Some(c) = ctype {
            resp.headers.push(Header::ContentType(c.into()));
        }
        resp
    }

    /// Answers a non-INVITE server transaction and lets it go.
    pub fn respond(&self, mut tx: Transaction, resp: Response) {
        tokio::spawn(async move {
            if let Err(e) = tx.respond(resp).await {
                log::debug!("sip: could not answer: {e}");
            }
        });
    }

    /// Answers a non-INVITE transaction with just a status.
    pub fn respond_code(&self, tx: Transaction, code: u16) {
        let tag = util::token(8);
        let resp = self.response(&tx.original, code, Some(&tag), &[], None);
        self.respond(tx, resp);
    }

    /// Hands a server INVITE transaction to a task that keeps receiving on
    /// it (ACK, CANCEL); responses are sent through the returned sender.
    pub fn adopt_invite(&mut self, mut tx: Transaction) -> ServerInvite {
        let id = self.id();
        let tu = tx.tu_sender.clone();
        let req = tx.original.clone();
        let post = self.tx.clone();
        tokio::spawn(async move {
            while let Some(msg) = tx.receive().await {
                if let rsipstack::sip::SipMessage::Request(r) = msg {
                    let _ = post.send(Ev::ServerReq { tx: id, req: Box::new(r) });
                }
            }
            let _ = post.send(Ev::ServerDone { tx: id });
        });
        ServerInvite { tx: id, tu, req }
    }

    pub fn server_respond(&self, s: &ServerInvite, resp: Response) {
        let _ = s.tu.send(TransactionEvent::Respond(resp));
    }

    // ------------------------------------------------------------- state

    /// Recomputes line keys, rapid-dial keys, and the message lamp.
    pub fn sync_keys(&self) {
        let model = self.cfg.model;
        let calls = self.store.get::<dphone_state::Calls>();
        let presence = self.store.get::<dphone_state::Presence>();
        let mwi = self.store.get::<dphone_state::Mwi>();
        let mut line_keys = Vec::new();
        for i in 0..model.line_keys() {
            let id = AccountId(i);
            let mut key = LineKey { index: i, ..Default::default() };
            if let Some(a) = self.acct(id) {
                key.binding = LineKeyBinding::Account(id);
                key.label = if a.cfg.label.is_empty() { a.cfg.user.clone() } else { a.cfg.label.clone() };
                let mine: Vec<_> = calls.live().filter(|c| c.account == id).collect();
                key.calls = mine.iter().map(|c| c.id).collect();
                use dphone_state::CallState as S;
                key.lamp = if mine.iter().any(|c| c.state == S::Ringing) {
                    LampState::blinking(LampColor::Green, Blink::Fast)
                } else if mine.iter().any(|c| matches!(c.state, S::Active | S::Dialing | S::Proceeding | S::Ringback { .. } | S::Transferring)) {
                    LampState::solid(LampColor::Green)
                } else if mine.iter().any(|c| c.state.is_held()) {
                    LampState::blinking(LampColor::Red, Blink::Slow)
                } else {
                    LampState::OFF
                };
            }
            line_keys.push(key);
        }
        let per_page = model.blf_keys().max(1) as usize;
        let rapid: Vec<RapidDialKey> = self
            .cfg
            .blf
            .iter()
            .enumerate()
            .map(|(i, b)| {
                use dphone_state::BlfState as B;
                let lamp = match presence.get(&b.id).map(|e| e.state) {
                    Some(B::Idle) => LampState::solid(LampColor::Green),
                    Some(B::Ringing) => LampState::blinking(LampColor::Red, Blink::Fast),
                    Some(B::InUse) => LampState::solid(LampColor::Red),
                    Some(B::OnHold) => LampState::blinking(LampColor::Red, Blink::Slow),
                    _ => LampState::OFF,
                };
                RapidDialKey {
                    index: (i % per_page) as u8,
                    page: (i / per_page) as u8,
                    entry: b.id.clone(),
                    label: b.label.clone(),
                    number: b.number.clone(),
                    lamp,
                }
            })
            .collect();
        let pages = rapid.len().div_ceil(per_page) as u8;
        let message_lamp = if mwi.any_waiting() { LampState::solid(LampColor::Red) } else { LampState::OFF };
        self.store.update::<Keys, _>(|k| {
            k.line_keys = line_keys;
            k.rapid_dial = rapid;
            k.rapid_pages = pages;
            if k.rapid_page >= pages.max(1) {
                k.rapid_page = 0;
            }
            k.message_lamp = message_lamp;
        });
    }

    pub fn heartbeat(&self) {
        self.store.heartbeat(Subsystem::Sip);
    }

    /// The account an incoming out-of-dialog request is for: the one whose
    /// user matches the request URI or To, else the first.
    pub fn account_for(&self, req: &Request) -> Option<AccountId> {
        let ruri_user = SipUri::parse(&req.uri.to_string()).map(|u| u.user).unwrap_or_default();
        let to_user = util::header(&req.headers, "To").map(|t| NameAddr::parse(t).user()).unwrap_or_default();
        self.accts
            .iter()
            .find(|a| a.cfg.user == ruri_user)
            .or_else(|| self.accts.iter().find(|a| a.cfg.user == to_user))
            .or(self.accts.first())
            .map(|a| a.cfg.id)
    }
}

/// Builds a typed header from a name and value by running it through the
/// stack's parser.
pub(crate) fn make_header(name: &str, value: &str) -> Option<Header> {
    let text = format!("OPTIONS sip:x@invalid SIP/2.0\r\n{name}: {value}\r\n\r\n");
    Request::try_from(text).ok()?.headers.iter().next().cloned()
}
