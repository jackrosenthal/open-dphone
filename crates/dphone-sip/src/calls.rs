//! Call control policy on top of the stack's INVITE dialogs: what to
//! offer and answer, when to ring, hold, transfer, and conference, DND
//! and forward, and the commands of [`dphone_state::Domain::Calls`].
//!
//! The dialog layer owns the signaling mechanics (transactions, CANCEL
//! races, ACK, authentication, target refresh, glare back-off); this file
//! reacts to its [`DialogState`] events and calls its dialog methods.

use crate::bodies;
use crate::core::{CallRt, Core, Ev, ReferFrom, Timer};
use crate::media::{MediaEvent, RingRequest, StreamParams, StreamSrtp};
use crate::sdp::{self, Crypto, Direction as SdpDir, Sdp, SrtpPolicy, SrtpSuite};
use crate::util::{self, NameAddr, SipUri};
use dphone_state::{
    clock, AccountId, AudioPath, Call, CallId, CallLog, CallLogEntry, CallLogKind, CallState, Calls, Command,
    CommandError, CommandResult, Direction, EndCause, Envelope, Event, Features, ForwardKind, Keys, Party,
    Presence, Reply, Tone,
};
use rsipstack::dialog::dialog::{Dialog, DialogState, TerminatedReason, TransactionHandle};
use rsipstack::dialog::invitation::InviteOption;
use rsipstack::dialog::invite_dialog::InviteDialog;
use rsipstack::sip::{Header, Method, Request, Response, StatusCode};
use rsipstack::transaction::transaction::{Transaction, TransactionEvent};
use std::time::Duration;

fn invalid(s: impl Into<String>) -> CommandError {
    CommandError::Invalid(s.into())
}

fn sdp_type() -> Header {
    Header::ContentType("application/sdp".to_string().into())
}

fn flatten<T>(r: rsipstack::Result<T>) -> Result<T, String> {
    r.map_err(|e| e.to_string())
}

impl Core {
    // ---------------------------------------------------------- helpers

    fn set_call(&self, id: CallId, f: impl FnOnce(&mut Call)) {
        self.store.update::<Calls, _>(|c| {
            if let Some(call) = c.get_mut(id) {
                f(call);
            }
        });
    }

    fn local_media(&self, c: &CallRt, direction: SdpDir) -> sdp::Local {
        let available = self.media.codecs();
        let codecs: Vec<_> = self.cfg.codecs.iter().filter(|p| available.contains(&p.codec)).collect();
        let srtp = self.acct(c.acct).is_some_and(|a| a.cfg.host(c.alt).srtp);
        sdp::Local {
            addr: self.cfg.local_ip,
            port: c.rtp_port,
            codecs: codecs.iter().map(|p| p.codec).collect(),
            ptime: codecs.first().map(|p| p.ptime).unwrap_or(20),
            srtp: if srtp { SrtpPolicy::Sdes } else { SrtpPolicy::Off },
            direction,
            session_id: c.session_id,
            version: c.sdp_version,
        }
    }

    /// Pushes the negotiated stream (and hold state) to the media engine
    /// and the store.
    fn apply_media(&mut self, id: CallId) {
        let Some(c) = self.call(id) else { return };
        let Some(n) = c.neg.clone() else { return };
        let params = StreamParams {
            codec: n.codec,
            tx_pt: n.tx_pt,
            rx_pt: n.rx_pt,
            remote: n.remote,
            ptime_ms: n.ptime,
            dtmf_tx_pt: n.dtmf_tx_pt,
            dtmf_rx_pt: n.dtmf_rx_pt,
            srtp: n.srtp.as_ref().map(|k| StreamSrtp { suite: k.suite, tx_key: k.tx, rx_key: k.rx }),
            send: n.send && !c.local_hold,
            recv: n.recv && !c.local_hold,
        };
        if let Err(e) = self.media.start(id, &params) {
            log::warn!("sip: media start failed for call {}: {e}", id.0);
        }
        self.set_call(id, |call| {
            call.codec = Some(n.codec);
            call.srtp = n.srtp.is_some();
        });
        self.refresh_connect();
    }

    /// Tells the media engine which streams feed the audio path.
    fn refresh_connect(&self) {
        let calls = self.store.get::<Calls>();
        let live = |id: &CallId| self.call(*id).is_some_and(|c| !c.ended && c.neg.is_some() && !c.local_hold);
        let list: Vec<CallId> = if self.conference.len() >= 2 {
            self.conference.iter().copied().filter(live).collect()
        } else {
            calls.focused.filter(live).into_iter().collect()
        };
        self.media.connect(&list);
    }

    fn held_state(c: &CallRt) -> CallState {
        if c.local_hold || c.remote_hold {
            CallState::Held { local: c.local_hold, remote: c.remote_hold }
        } else {
            CallState::Active
        }
    }

    fn focus(&self, id: Option<CallId>) {
        self.store.update::<Calls, _>(|c| c.focused = id);
    }

    /// Our SRTP key for a call, made on first use.
    fn ensure_key(&mut self, id: CallId) {
        let srtp = self.call(id).is_some_and(|c| self.acct(c.acct).is_some_and(|a| a.cfg.host(c.alt).srtp));
        if let Some(c) = self.call_mut(id) {
            if srtp && c.key_material.is_none() {
                c.key_material = Crypto::generate(1, SrtpSuite::AesCm128HmacSha1_80).ok();
            }
        }
    }

    /// Hangs the dialog up in the background: CANCEL or BYE, as the layer
    /// decides from its state.
    fn hangup_dialog(&self, id: CallId) {
        if let Some(dialog) = self.call(id).map(|c| c.dialog.clone()) {
            tokio::spawn(async move {
                if let Err(e) = dialog.hangup().await {
                    log::debug!("sip: hangup: {e}");
                }
            });
        }
    }

    /// Ends a call locally: media, state, call log, focus, events.
    pub(crate) fn end_call(&mut self, id: CallId, cause: EndCause) {
        let Some(c) = self.call_mut(id) else { return };
        if c.ended {
            return;
        }
        c.ended = true;
        c.server = None;
        let (answered, outgoing, refer_from) = (c.answered, c.outgoing, c.refer_from.clone());
        self.media.close(id);
        self.media.ring(None);
        match cause {
            EndCause::Busy => self.media.tone(Some(Tone::Busy)),
            EndCause::Failed { .. } if outgoing && !answered => self.media.tone(Some(Tone::Reorder)),
            _ => self.media.tone(None),
        }
        if self.conference.contains(&id) {
            let rest: Vec<CallId> = self.conference.drain(..).filter(|c| *c != id).collect();
            for r in rest {
                self.set_call(r, |c| c.in_conference = false);
            }
        }
        if let (Some(r), false) = (refer_from, answered) {
            let code = match &cause {
                EndCause::Failed { code, .. } => *code,
                EndCause::Busy => 486,
                _ => 487,
            };
            self.notify_refer(r.call, code, true);
        }
        let mut snapshot = None;
        self.store.update::<Calls, _>(|calls| {
            if let Some(call) = calls.get_mut(id) {
                call.state = CallState::Ended(cause.clone());
                call.in_conference = false;
                snapshot = Some(call.clone());
            }
            if calls.focused == Some(id) {
                // Prefer a ringing or active call, else any live one.
                let next = calls
                    .live()
                    .find(|c| matches!(c.state, CallState::Active | CallState::Ringing))
                    .map(|c| c.id)
                    .or_else(|| calls.live().next().map(|c| c.id));
                calls.focused = next;
            }
        });
        if let Some(call) = snapshot {
            let kind = match (outgoing, answered) {
                (true, _) => CallLogKind::Placed,
                (false, true) => CallLogKind::Received,
                (false, false) => CallLogKind::Missed,
            };
            let duration =
                call.connected_mono_ms.map(|c| (clock::mono_ms().saturating_sub(c) / 1000) as u32).unwrap_or(0);
            self.store.update::<CallLog, _>(|log| {
                log.push(CallLogEntry {
                    id: 0,
                    kind,
                    party: call.remote.clone(),
                    account: call.account,
                    start_unix: call.started_unix,
                    duration_secs: duration,
                })
            });
            self.store.emit(Event::CallEnded { call });
        }
        self.after(Duration::from_secs(self.cfg.ended_linger_secs as u64), Timer::Linger { call: id });
        // If the user was left with other calls ringing, keep them ringing.
        let ringing = self.store.get::<Calls>().ringing().next().map(|c| c.id);
        if let Some(r) = ringing {
            let alert_info = self.call(r).map(|c| c.alert_info.clone()).unwrap_or_default();
            let busy = self.store.get::<Calls>().live().any(|c| c.id != r && c.state.is_established());
            self.media.ring(Some(RingRequest { call: r, alert_info, call_waiting: busy }));
        }
        self.refresh_connect();
        self.sync_keys();
    }

    pub fn on_linger(&mut self, id: CallId) {
        if self.call(id).is_some_and(|c| c.ended) {
            if let Some(c) = self.call(id) {
                self.layer.remove_dialog(&c.dialog.id());
            }
            self.calls.retain(|c| c.id != id);
            let mut idle = false;
            self.store.update::<Calls, _>(|c| {
                c.calls.retain(|x| x.id != id);
                idle = c.is_idle();
            });
            if idle {
                self.media.tone(None);
            }
            self.sync_keys();
        }
    }

    // --------------------------------------------------------- outgoing

    fn pick_account(&self, wanted: Option<AccountId>) -> Result<AccountId, CommandError> {
        match wanted {
            Some(id) => self.acct(id).map(|a| a.cfg.id).ok_or_else(|| invalid("no such account")),
            None => self
                .accts
                .iter()
                .find(|a| a.registered)
                .or(self.accts.first())
                .map(|a| a.cfg.id)
                .ok_or_else(|| invalid("no account is configured")),
        }
    }

    /// The URI a dial string means on an account.
    fn dial_uri(&self, acct: AccountId, target: &str) -> Result<String, CommandError> {
        let target = target.trim();
        if target.is_empty() {
            return Err(invalid("nothing to dial"));
        }
        if target.starts_with("sip:") || target.starts_with("sips:") {
            return SipUri::parse(target).map(|_| target.to_string()).ok_or_else(|| invalid("bad SIP URI"));
        }
        let a = self.acct(acct).ok_or_else(|| invalid("no such account"))?;
        let digits: String = target.chars().filter(|c| !c.is_whitespace()).collect();
        let user = util::percent_encode(&digits).replace("%2A", "*").replace("%2B", "+");
        Ok(format!("sip:{user}@{}", a.cfg.host(a.alt).server))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dial(
        &mut self,
        target: &str,
        account: Option<AccountId>,
        path: Option<AudioPath>,
        replaces: Option<String>,
        referred_by: Option<String>,
        refer_from: Option<ReferFrom>,
    ) -> Result<CallId, CommandError> {
        let acct = self.pick_account(account)?;
        let uri = self.dial_uri(acct, target)?;
        if self.calls.iter().filter(|c| !c.ended).count() >= self.cfg.max_calls {
            return Err(invalid("too many calls"));
        }
        let id = self.store.next_call_id();
        let rtp_port = self.media.open(id).map_err(CommandError::Failed)?;
        self.hold_others(None);
        let a = self.acct(acct).expect("picked account exists");
        let (alt, srtp) = (a.alt, a.cfg.host(a.alt).srtp);
        let key_material = if srtp { Crypto::generate(1, SrtpSuite::AesCm128HmacSha1_80).ok() } else { None };
        // The layer creates the dialog from the INVITE, and the INVITE
        // carries the offer, so the offer is built before the call record.
        let session_id = clock::unix_secs();
        let probe = sdp::Local {
            addr: self.cfg.local_ip,
            port: rtp_port,
            codecs: {
                let available = self.media.codecs();
                self.cfg.codecs.iter().filter(|p| available.contains(&p.codec)).map(|p| p.codec).collect()
            },
            ptime: self.cfg.codecs.first().map(|p| p.ptime).unwrap_or(20),
            srtp: if srtp { SrtpPolicy::Sdes } else { SrtpPolicy::Off },
            direction: SdpDir::SendRecv,
            session_id,
            version: 1,
        };
        let offer = sdp::offer(&probe, key_material.as_ref());
        let number = SipUri::parse(&uri).map(|u| util::percent_decode(&u.user)).unwrap_or_default();
        let pending = PendingCall { acct, replaces: replaces.clone(), referred_by: referred_by.clone() };
        let dialog = match self.invite_for(id, &pending, &uri, &offer) {
            Ok(d) => d,
            Err(e) => {
                self.media.close(id);
                return Err(CommandError::Failed(e));
            }
        };
        let did = dialog.id();
        let mut call = Call::new(id, acct, Direction::Outgoing, Party { name: String::new(), number, uri: uri.clone() });
        call.sip_call_id = did.call_id.clone();
        self.calls.push(CallRt {
            id,
            acct,
            alt,
            key: (did.call_id, did.local_tag),
            dialog,
            outgoing: true,
            server: None,
            rtp_port,
            session_id,
            sdp_version: 1,
            offer: Some(offer),
            remote_offer: None,
            key_material,
            neg: None,
            local_hold: false,
            remote_hold: false,
            reinvite_busy: false,
            hold_wanted: None,
            answered: false,
            ended: false,
            alert_info: String::new(),
            refer_from,
            replaces,
            referred_by,
            redirected: false,
            peer: uri.clone(),
        });
        self.store.update::<Calls, _>(|c| {
            c.calls.push(call);
            c.focused = Some(id);
        });
        if let Some(p) = path {
            self.media.request_path(p);
        }
        self.media.tone(None);
        self.sync_keys();
        Ok(id)
    }

    /// Starts an INVITE through the dialog layer; its end is reported as
    /// [`Ev::InviteDone`].
    fn invite_for(&mut self, id: CallId, p: &PendingCall, uri: &str, offer: &Sdp) -> Result<InviteDialog, String> {
        let a = self.acct(p.acct).ok_or("no such account")?;
        let (dest, transport) = self.request_dest(a, uri)?;
        let mut headers = vec![Core::allow(), Header::Supported("replaces".to_string().into())];
        if let Some(r) = &p.replaces {
            headers.push(Header::Replaces(r.clone().into()));
        }
        if let Some(r) = &p.referred_by {
            headers.push(Header::ReferredBy(r.clone().into()));
        }
        let opt = InviteOption {
            caller_display_name: (!a.cfg.display_name.is_empty()).then(|| a.cfg.display_name.clone()),
            caller: crate::core::uri(&format!("sip:{}@{}", a.cfg.user, a.cfg.host(a.alt).server))?,
            callee: crate::core::uri(uri)?,
            destination: Some(dest),
            offer: Some(offer.render().into_bytes()),
            contact: self.contact_uri(&a.cfg.user, a.public.clone(), transport),
            credential: Some(Core::credential(a)),
            headers: Some(headers),
            ..Default::default()
        };
        let (dialog, done) = flatten(self.layer.do_invite_async(opt, self.dialog_tx.clone()))?;
        let post = self.tx.clone();
        tokio::spawn(async move {
            let result = match done.await {
                Ok(Ok((_, resp))) => Ok(resp),
                Ok(Err(e)) => Err(e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            let _ = post.send(Ev::InviteDone { call: id, result });
        });
        Ok(dialog)
    }

    fn on_early(&mut self, id: CallId, resp: &Response) {
        let Some(c) = self.call_mut(id) else { return };
        if c.ended || c.answered || !c.outgoing {
            return;
        }
        let mut early = false;
        if !resp.body.is_empty() {
            if let (Some(offer), Ok(remote)) = (c.offer.clone(), Sdp::parse(&String::from_utf8_lossy(&resp.body))) {
                if let Ok(n) = sdp::accept(&offer, &remote) {
                    c.neg = Some(n);
                    early = true;
                }
            }
        }
        let name = NameAddr::parse(util::header(&resp.headers, "To").unwrap_or(""));
        if early {
            self.media.tone(None);
            self.apply_media(id);
        } else if self.call(id).is_some_and(|c| c.neg.is_none()) {
            self.media.tone(Some(Tone::Ringback));
        }
        let has_media = self.call(id).is_some_and(|c| c.neg.is_some());
        self.set_call(id, |call| {
            call.state = CallState::Ringback { early_media: has_media };
            if call.remote.name.is_empty() {
                call.remote.name = name.display.clone();
            }
        });
        if let Some(r) = self.call(id).and_then(|c| c.refer_from.clone()) {
            self.notify_refer(r.call, 180, false);
        }
        self.sync_keys();
    }

    /// The 2xx to our INVITE: the answer to our offer.
    fn on_answered(&mut self, id: CallId, resp: &Response) {
        let Some(c) = self.call_mut(id) else { return };
        if c.ended || c.answered || !c.outgoing {
            return;
        }
        let negotiated = match (&c.offer, Sdp::parse(&String::from_utf8_lossy(&resp.body))) {
            (Some(offer), Ok(remote)) => sdp::accept(offer, &remote),
            // No SDP in the 200 after an early answer: keep that one.
            (_, Err(_)) if c.neg.is_some() => Ok(c.neg.clone().expect("checked")),
            (_, Err(e)) => Err(e),
            (None, _) => Err("no offer outstanding".into()),
        };
        match negotiated {
            Ok(n) => {
                c.neg = Some(n);
                c.offer = None;
                c.answered = true;
            }
            Err(e) => {
                log::warn!("sip: call {}: unusable answer: {e}", id.0);
                self.hangup_dialog(id);
                self.end_call(id, EndCause::Error(e));
                return;
            }
        }
        let refer = c.refer_from.clone();
        self.media.tone(None);
        self.apply_media(id);
        let name = NameAddr::parse(util::header(&resp.headers, "To").unwrap_or(""));
        self.set_call(id, |call| {
            call.state = CallState::Active;
            call.connected_mono_ms = Some(clock::mono_ms());
            if call.remote.name.is_empty() {
                call.remote.name = name.display.clone();
            }
        });
        if let Some(r) = refer {
            self.notify_refer(r.call, 200, true);
        }
        self.sync_keys();
    }

    /// The INVITE transaction finished: failures and redirects are ours to
    /// interpret (success also arrives as `DialogState::Confirmed`).
    pub fn on_invite_done(&mut self, id: CallId, result: Result<Option<Response>, String>) {
        let Some(c) = self.call_mut(id) else { return };
        if c.ended {
            return;
        }
        let resp = match result {
            Ok(Some(r)) => r,
            Ok(None) => return self.end_call(id, EndCause::NoAnswer),
            Err(e) => return self.end_call(id, EndCause::Error(e)),
        };
        let code = resp.status_code.code();
        if (200..300).contains(&code) {
            return self.on_answered(id, &resp);
        }
        // Redirect: follow the first Contact once.
        if (300..400).contains(&code) && !c.redirected {
            let contact = util::header(&resp.headers, "Contact").map(|c| NameAddr::parse(c).uri);
            if let (Some(contact), Some(offer)) = (contact.filter(|c| SipUri::parse(c).is_some()), c.offer.clone()) {
                c.redirected = true;
                let old = c.dialog.id();
                let p = PendingCall { acct: c.acct, replaces: c.replaces.clone(), referred_by: c.referred_by.clone() };
                if let Ok(dialog) = self.invite_for(id, &p, &contact, &offer) {
                    self.layer.remove_dialog(&old);
                    let did = dialog.id();
                    if let Some(c) = self.call_mut(id) {
                        c.key = (did.call_id.clone(), did.local_tag);
                        c.dialog = dialog;
                    }
                    self.set_call(id, |call| call.sip_call_id = did.call_id);
                    return;
                }
            }
        }
        let reason = resp.reason_phrase().map(str::to_string).unwrap_or_else(|| resp.status_code.to_string());
        let cause = match code {
            486 | 600 => EndCause::Busy,
            487 => EndCause::LocalHangup,
            408 | 480 => EndCause::NoAnswer,
            _ => EndCause::Failed { code, reason },
        };
        self.end_call(id, cause);
    }

    // --------------------------------------------------------- incoming

    /// An INVITE that matches no dialog: a new call (or a replacement).
    /// Policy that refuses the call outright answers on the transaction;
    /// otherwise the call becomes a server dialog of the layer.
    pub fn on_invite(&mut self, tx: Transaction) {
        let req = tx.original.clone();
        let from = NameAddr::parse(util::header(&req.headers, "From").unwrap_or(""));
        if NameAddr::parse(util::header(&req.headers, "To").unwrap_or("")).tag().is_some() {
            return self.reply(tx, 481, Vec::new());
        }
        let Some(acct) = self.account_for(&req) else { return self.reply(tx, 404, Vec::new()) };
        let features = self.store.get::<Features>();
        let host = self.acct(acct).map(|a| a.cfg.host(a.alt).server.clone()).unwrap_or_default();
        let redirect = |target: &str| {
            let uri = if target.starts_with("sip:") { target.to_string() } else { format!("sip:{target}@{host}") };
            vec![Header::Contact(format!("<{uri}>").into())]
        };
        // A replacement (attended transfer, pickup) bypasses DND and busy.
        let replaced = util::header(&req.headers, "Replaces").and_then(|r| {
            let mut parts = r.split(';');
            let cid = parts.next()?.trim().to_string();
            let params: Vec<(String, String)> = parts
                .filter_map(|p| p.split_once('=').map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())))
                .collect();
            let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
            let (to_tag, from_tag) = (get("to-tag")?, get("from-tag")?);
            self.calls
                .iter()
                .find(|c| {
                    let d = c.dialog.id();
                    !c.ended && d.call_id == cid && d.local_tag == to_tag && d.remote_tag == from_tag
                })
                .map(|c| c.id)
        });
        if replaced.is_none() {
            if !features.forward.always.is_empty() {
                return self.reply(tx, 302, redirect(&features.forward.always));
            }
            if features.dnd {
                return self.reply(tx, 486, Vec::new());
            }
            let live = self.calls.iter().filter(|c| !c.ended).count();
            if live > 0 && (!self.cfg.call_waiting || !features.call_waiting || live >= self.cfg.max_calls) {
                return if features.forward.busy.is_empty() {
                    self.reply(tx, 486, Vec::new())
                } else {
                    self.reply(tx, 302, redirect(&features.forward.busy))
                };
            }
        }
        let id = self.store.next_call_id();
        let remote_offer = if req.body.is_empty() {
            None
        } else {
            match Sdp::parse(&String::from_utf8_lossy(&req.body)) {
                Ok(s) => Some(s),
                Err(e) => {
                    log::warn!("sip: bad offer: {e}");
                    return self.reply(tx, 488, Vec::new());
                }
            }
        };
        let rtp_port = match self.media.open(id) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("sip: no media for an incoming call: {e}");
                return self.reply(tx, 480, Vec::new());
            }
        };
        let a = self.acct(acct).expect("account_for returned it");
        let (alt, srtp) = (a.alt, a.cfg.host(a.alt).srtp);
        let key_material = if srtp { Crypto::generate(1, SrtpSuite::AesCm128HmacSha1_80).ok() } else { None };
        // Refuse now what cannot be answered later.
        if let Some(offer) = &remote_offer {
            let available = self.media.codecs();
            let probe = sdp::Local {
                addr: self.cfg.local_ip,
                port: rtp_port,
                codecs: self.cfg.codecs.iter().filter(|p| available.contains(&p.codec)).map(|p| p.codec).collect(),
                ptime: 20,
                srtp: if srtp { SrtpPolicy::Sdes } else { SrtpPolicy::Off },
                direction: SdpDir::SendRecv,
                session_id: 0,
                version: 0,
            };
            if let Err(e) = sdp::answer(offer, &probe, key_material.as_ref()) {
                log::info!("sip: refusing a call: {e}");
                self.media.close(id);
                return self.reply(tx, 488, Vec::new());
            }
        }
        let contact = self.account_contact(a);
        let dialog = match self.layer.get_or_create_server_invite(
            &tx,
            self.dialog_tx.clone(),
            Some(Core::credential(a)),
            Some(contact),
        ) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("sip: cannot create a dialog: {e}");
                self.media.close(id);
                return self.reply(tx, 400, Vec::new());
            }
        };
        let server = Some((tx.tu_sender.clone(), req.clone()));
        self.dispatch(Dialog::Invite(dialog.clone()), tx);
        if let Err(e) = dialog.ringing(None, None) {
            log::debug!("sip: ringing: {e}");
        }
        let alert_info = util::header(&req.headers, "Alert-Info").unwrap_or("").to_string();
        let call_info = util::header(&req.headers, "Call-Info").unwrap_or("").to_ascii_lowercase();
        let lower_alert = alert_info.to_ascii_lowercase();
        let wants_auto = call_info.contains("answer-after=0")
            || lower_alert.contains("auto answer")
            || lower_alert.contains("autoanswer")
            || lower_alert.contains("ring answer")
            || lower_alert.contains("intercom");
        let did = dialog.id();
        let pai = util::header(&req.headers, "P-Asserted-Identity").map(NameAddr::parse);
        let who = pai.as_ref().unwrap_or(&from);
        let party = Party { name: who.display.clone(), number: util::percent_decode(&who.user()), uri: who.uri.clone() };
        let mut call = Call::new(id, acct, Direction::Incoming, party);
        call.sip_call_id = did.call_id.clone();
        call.auto_answer = wants_auto && self.cfg.allow_auto_answer;
        call.paging = lower_alert.contains("paging") || call_info.contains("paging");
        let auto = call.auto_answer;
        self.calls.push(CallRt {
            id,
            acct,
            alt,
            key: (did.call_id, did.local_tag),
            dialog,
            outgoing: false,
            server,
            rtp_port,
            session_id: clock::unix_secs(),
            sdp_version: 1,
            offer: None,
            remote_offer,
            key_material,
            neg: None,
            local_hold: false,
            remote_hold: false,
            reinvite_busy: false,
            hold_wanted: None,
            answered: false,
            ended: false,
            alert_info: alert_info.clone(),
            refer_from: None,
            replaces: None,
            referred_by: None,
            redirected: false,
            peer: from.uri.clone(),
        });
        let others = self.store.get::<Calls>().live().next().is_some();
        self.store.update::<Calls, _>(|c| {
            c.calls.push(call);
            if c.focused.is_none() || c.focused().is_none_or(|f| f.state.is_ended()) {
                c.focused = Some(id);
            }
        });
        self.store.emit(Event::IncomingCall { call: id });
        self.sync_keys();
        if let Some(old) = replaced {
            // The replacement takes over the old call's place at once.
            if self.answer(id, None).is_ok() {
                self.hangup_dialog(old);
                self.end_call(old, EndCause::Transferred);
            }
            return;
        }
        if auto && !others {
            let _ = self.answer(id, Some(AudioPath::Speaker));
            return;
        }
        self.media.ring(Some(RingRequest { call: id, alert_info, call_waiting: others }));
        if !features.forward.no_answer.is_empty() {
            let secs =
                if features.forward.no_answer_secs > 0 { features.forward.no_answer_secs } else { self.cfg.no_answer_secs };
            self.after(Duration::from_secs(secs as u64), Timer::NoAnswer { call: id });
        }
    }

    pub fn on_no_answer(&mut self, id: CallId) {
        let target = self.store.get::<Features>().forward.no_answer.clone();
        if target.is_empty() || self.call(id).is_none_or(|c| c.answered || c.ended) {
            return;
        }
        self.redirect_call(id, &target, EndCause::NoAnswer);
    }

    /// Answers a ringing call with a 302.  The layer's `reject` cannot
    /// carry a Contact, so the response is built here and sent on the
    /// INVITE transaction; `reject` then only retires the dialog (its own
    /// response finds the transaction already completed).
    fn redirect_call(&mut self, id: CallId, target: &str, cause: EndCause) {
        let Some(c) = self.call_mut(id) else { return };
        let Some((tu, req)) = c.server.take() else { return };
        let dialog = c.dialog.clone();
        let acct = c.acct;
        let host = self.acct(acct).map(|a| a.cfg.host(a.alt).server.clone()).unwrap_or_default();
        let uri = if target.starts_with("sip:") { target.to_string() } else { format!("sip:{target}@{host}") };
        let mut resp = self.inner.make_response(&req, StatusCode::MovedTemporarily, None);
        let tag = dialog.id().local_tag;
        for h in resp.headers.iter_mut() {
            if let Header::To(to) = h {
                let v = to.value().to_string();
                if NameAddr::parse(&v).tag().is_none() {
                    *h = Header::To(format!("{v};tag={tag}").into());
                }
                break;
            }
        }
        resp.headers.push(Header::Contact(format!("<{uri}>").into()));
        let _ = tu.send(TransactionEvent::Respond(resp));
        let _ = dialog.reject(None, None);
        self.end_call(id, cause);
    }

    fn reject(&mut self, id: CallId, code: u16) -> CommandResult {
        let c = self.call_mut(id).ok_or_else(|| invalid("no such call"))?;
        if c.answered || c.outgoing || c.ended {
            return Err(invalid("the call is not ringing"));
        }
        flatten(c.dialog.reject(Some(StatusCode::from(code)), None)).map_err(CommandError::Failed)?;
        self.end_call(id, EndCause::Rejected);
        Ok(Reply::Done)
    }

    pub(crate) fn answer(&mut self, id: CallId, path: Option<AudioPath>) -> CommandResult {
        let c = self.call(id).ok_or_else(|| invalid("no such call"))?;
        if c.answered || c.ended || c.outgoing {
            return Err(invalid("the call is not ringing"));
        }
        let local = self.local_media(c, SdpDir::SendRecv);
        let (body, neg, offer) = match &c.remote_offer {
            Some(remote) => {
                let (ans, neg) = sdp::answer(remote, &local, c.key_material.as_ref()).map_err(CommandError::Failed)?;
                (ans.render(), Some(neg), None)
            }
            // An INVITE without SDP: we offer, the ACK answers.
            None => {
                let offer = sdp::offer(&local, c.key_material.as_ref());
                (offer.render(), None, Some(offer))
            }
        };
        let headers = vec![Core::allow(), Header::Supported("replaces".to_string().into()), sdp_type()];
        flatten(c.dialog.accept(Some(headers), Some(body.into_bytes()))).map_err(CommandError::Failed)?;
        self.hold_others(Some(id));
        let c = self.call_mut(id).expect("checked above");
        c.answered = true;
        c.server = None;
        c.neg = neg;
        c.offer = offer;
        self.media.ring(None);
        self.media.tone(None);
        if let Some(p) = path {
            self.media.request_path(p);
        }
        self.set_call(id, |call| {
            call.state = CallState::Active;
            call.connected_mono_ms = Some(clock::mono_ms());
        });
        self.focus(Some(id));
        self.apply_media(id);
        self.sync_keys();
        Ok(Reply::Done)
    }

    /// The answer to an offer of ours that travelled in a 200.
    pub fn on_ack_body(&mut self, id: CallId, body: &[u8]) {
        let Some(c) = self.call_mut(id) else { return };
        let Some(offer) = c.offer.take() else { return };
        match Sdp::parse(&String::from_utf8_lossy(body)).and_then(|r| sdp::accept(&offer, &r)) {
            Ok(n) => {
                c.neg = Some(n);
                self.apply_media(id);
            }
            Err(e) => {
                log::warn!("sip: call {}: bad answer in ACK: {e}", id.0);
                self.hangup_dialog(id);
                self.end_call(id, EndCause::Error(e));
            }
        }
    }

    // ------------------------------------------------- dialog events

    /// Everything the dialog layer reports.
    pub fn on_dialog(&mut self, state: DialogState) {
        let did = state.id().clone();
        let Some(id) = self.calls.iter().find(|c| c.key.0 == did.call_id && c.key.1 == did.local_tag).map(|c| c.id)
        else {
            return self.on_sub_dialog(state);
        };
        match state {
            DialogState::Calling(_) | DialogState::WaitAck(..) => {}
            DialogState::Trying(_) => self.set_call(id, |call| {
                if call.state == CallState::Dialing {
                    call.state = CallState::Proceeding;
                }
            }),
            DialogState::Early(_, resp) => self.on_early(id, &resp),
            DialogState::Confirmed(_, resp) => {
                let Some(c) = self.call(id) else { return };
                if c.outgoing && !c.answered {
                    self.on_answered(id, &resp);
                } else if !c.outgoing && c.offer.is_some() && c.neg.is_none() {
                    // Our 200 carried the offer; the ACK has the answer.
                    if let Some(ack) = c.dialog.last_remote_ack().filter(|a| !a.body.is_empty()) {
                        self.on_ack_body(id, &ack.body);
                    }
                }
            }
            DialogState::Updated(_, req, handle) => self.on_reoffer(id, req, handle),
            DialogState::Info(_, req, handle) => {
                let body = String::from_utf8_lossy(&req.body);
                let ctype = util::header(&req.headers, "Content-Type").unwrap_or("").to_ascii_lowercase();
                if ctype.contains("dtmf") {
                    let digit = body
                        .lines()
                        .find_map(|l| l.trim().strip_prefix("Signal=").map(|s| s.trim().to_string()))
                        .or_else(|| Some(body.trim().to_string()))
                        .and_then(|s| s.chars().next());
                    if let Some(d) = digit.filter(|d| "0123456789*#ABCDabcd".contains(*d)) {
                        self.store.emit(Event::DtmfReceived { call: id, digit: d.to_ascii_uppercase() });
                    }
                }
                self.respond(handle, 200, Vec::new(), None);
            }
            DialogState::Options(_, _, handle) => self.respond(handle, 200, vec![Core::allow()], None),
            DialogState::Message(_, req, handle) => {
                self.deliver_message(&req);
                self.respond(handle, 200, Vec::new(), None);
            }
            DialogState::Publish(_, _, handle) => self.respond(handle, 405, Vec::new(), None),
            DialogState::Refer(_, req, handle) => self.on_refer(id, req, handle),
            DialogState::Notify(_, req, handle) => {
                self.respond(handle, 200, Vec::new(), None);
                self.on_call_notify(id, &req);
            }
            DialogState::Terminated(_, reason) => {
                let Some(c) = self.call(id) else { return };
                // A failed outgoing INVITE is reported, with its response,
                // by `on_invite_done`.
                if c.ended || (c.outgoing && !c.answered) {
                    return;
                }
                let cause = match reason {
                    TerminatedReason::UacBye | TerminatedReason::UasBye => EndCause::RemoteHangup,
                    TerminatedReason::UacCancel => EndCause::Cancelled { answered_elsewhere: false },
                    TerminatedReason::Timeout => EndCause::Error("the far end stopped answering".into()),
                    other => EndCause::Error(format!("{other:?}")),
                };
                self.end_call(id, cause);
            }
        }
    }

    /// Dialog events that are not about a call: our subscriptions.
    fn on_sub_dialog(&mut self, state: DialogState) {
        let did = state.id().clone();
        let Some(idx) = self.subs.iter().position(|s| s.call_id == did.call_id) else { return };
        match state {
            DialogState::Notify(_, req, handle) => self.on_sub_notify(idx, req, handle),
            DialogState::Message(_, _, handle) | DialogState::Refer(_, _, handle) => {
                self.respond(handle, 405, Vec::new(), None)
            }
            _ => {}
        }
    }

    /// Applies an offer of theirs inside the dialog; returns our answer.
    fn answer_reoffer(&mut self, id: CallId, body: &[u8]) -> Result<String, String> {
        self.ensure_key(id);
        let c = self.call_mut(id).ok_or("no such call")?;
        c.sdp_version += 1;
        let c = self.call(id).ok_or("no such call")?;
        let remote = Sdp::parse(&String::from_utf8_lossy(body))?;
        let dir = if c.local_hold { SdpDir::SendOnly } else { SdpDir::SendRecv };
        let (ans, neg) = sdp::answer(&remote, &self.local_media(c, dir), c.key_material.as_ref())?;
        let remote_hold = !remote.direction.receives();
        let c = self.call_mut(id).ok_or("no such call")?;
        c.neg = Some(neg);
        c.remote_hold = remote_hold;
        let state = Self::held_state(c);
        self.set_call(id, |call| {
            if call.state.is_established() && call.state != CallState::Transferring {
                call.state = state;
            }
        });
        self.apply_media(id);
        self.sync_keys();
        Ok(ans.render())
    }

    /// A re-INVITE or UPDATE of theirs.
    fn on_reoffer(&mut self, id: CallId, req: Request, handle: TransactionHandle) {
        if !req.body.is_empty() {
            return match self.answer_reoffer(id, &req.body) {
                Ok(ans) => self.respond(handle, 200, vec![sdp_type()], Some(ans.into_bytes())),
                Err(e) => {
                    log::warn!("sip: call {}: refused a new offer: {e}", id.0);
                    self.respond(handle, 488, Vec::new(), None)
                }
            };
        }
        if req.method != Method::Invite {
            return self.respond(handle, 200, Vec::new(), None);
        }
        // Offerless re-INVITE: offer what we have now; the ACK answers.
        self.ensure_key(id);
        let Some(c) = self.call_mut(id) else { return };
        c.sdp_version += 1;
        let c = self.call(id).expect("exists");
        let dir = if c.local_hold { SdpDir::SendOnly } else { SdpDir::SendRecv };
        let offer = sdp::offer(&self.local_media(c, dir), c.key_material.as_ref());
        let body = offer.render().into_bytes();
        if let Some(c) = self.call_mut(id) {
            c.offer = Some(offer);
        }
        let post = self.tx.clone();
        tokio::spawn(async move {
            if handle.respond(StatusCode::OK, Some(vec![sdp_type()]), Some(body)).await.is_ok() {
                if let Ok(ack) = handle.wait_ack().await {
                    let _ = post.send(Ev::AckBody { call: id, body: ack.body });
                }
            }
        });
    }

    /// We are asked to call someone else (we are the transferee).
    fn on_refer(&mut self, id: CallId, req: Request, handle: TransactionHandle) {
        let Some(refer_to) = util::header(&req.headers, "Refer-To").map(NameAddr::parse) else {
            return self.respond(handle, 400, Vec::new(), None);
        };
        let Some(uri) = SipUri::parse(&refer_to.uri) else { return self.respond(handle, 400, Vec::new(), None) };
        let replaces = uri
            .headers
            .split('&')
            .find_map(|h| h.strip_prefix("Replaces=").or_else(|| h.strip_prefix("replaces=")))
            .map(util::percent_decode);
        let target = refer_to.uri.split('?').next().unwrap_or(&refer_to.uri).to_string();
        let referred_by = util::header(&req.headers, "Referred-By").map(str::to_string);
        let acct = self.call(id).map(|c| c.acct);
        self.respond(handle, 202, Vec::new(), None);
        self.notify_refer(id, 100, false);
        if let Err(e) = self.dial(&target, acct, None, replaces, referred_by, Some(ReferFrom { call: id })) {
            log::warn!("sip: cannot follow REFER: {e}");
            self.notify_refer(id, 503, true);
        }
    }

    /// Reports the progress of a REFER we are following.
    fn notify_refer(&mut self, id: CallId, code: u16, last: bool) {
        let Some(c) = self.call(id).filter(|c| !c.ended) else { return };
        let dialog = c.dialog.clone();
        let state = if last { "terminated;reason=noresource" } else { "active;expires=60" };
        tokio::spawn(async move {
            if let Err(e) = dialog.notify_refer(StatusCode::from(code), state).await {
                log::debug!("sip: NOTIFY: {e}");
            }
        });
    }

    /// Progress of a transfer we asked for.
    fn on_call_notify(&mut self, id: CallId, req: &Request) {
        let event = util::header(&req.headers, "Event").unwrap_or("").to_ascii_lowercase();
        if !event.starts_with("refer") {
            return;
        }
        let Some(code) = bodies::parse_sipfrag(&String::from_utf8_lossy(&req.body)) else { return };
        let transferring = self.store.get::<Calls>().get(id).is_some_and(|c| c.state == CallState::Transferring);
        if !transferring || code < 200 {
            return;
        }
        if code < 300 {
            let consult = self.store.get::<Calls>().calls.iter().find(|c| c.transfer_of == Some(id)).map(|c| c.id);
            self.hangup_dialog(id);
            self.end_call(id, EndCause::Transferred);
            if let Some(c) = consult {
                self.hangup_dialog(c);
                self.end_call(c, EndCause::Transferred);
            }
        } else {
            log::info!("sip: transfer of call {} failed: {code}", id.0);
            self.transfer_failed(id);
        }
    }

    fn transfer_failed(&mut self, id: CallId) {
        if let Some(c) = self.call(id) {
            let state = Self::held_state(c);
            self.set_call(id, |call| {
                if call.state == CallState::Transferring {
                    call.state = state;
                }
            });
            self.store.update::<Calls, _>(|calls| {
                for c in calls.calls.iter_mut().filter(|c| c.transfer_of == Some(id)) {
                    c.transfer_of = None;
                }
            });
            self.sync_keys();
        }
    }

    // ----------------------------------------------------- hold, resume

    fn send_reinvite(&mut self, id: CallId, hold: bool) -> Result<(), CommandError> {
        self.ensure_key(id);
        let c = self.call_mut(id).ok_or_else(|| invalid("no such call"))?;
        if !c.answered || c.ended {
            return Err(invalid("the call is not connected"));
        }
        if c.reinvite_busy {
            c.hold_wanted = Some(hold);
            return Ok(());
        }
        c.sdp_version += 1;
        let c = self.call(id).expect("exists");
        let dir = if hold { SdpDir::SendOnly } else { SdpDir::SendRecv };
        let offer = sdp::offer(&self.local_media(c, dir), c.key_material.as_ref());
        let body = offer.render().into_bytes();
        let (dialog, post) = (c.dialog.clone(), self.tx.clone());
        tokio::spawn(async move {
            // The layer retries glare (491) with the RFC 3261 back-off.
            let result =
                dialog.reinvite_with_glare_retry(Some(vec![Core::allow(), sdp_type()]), Some(body), 3).await;
            let _ = post.send(Ev::ReinviteDone { call: id, hold, result: result.map_err(|e| e.to_string()) });
        });
        let c = self.call_mut(id).expect("exists");
        c.offer = Some(offer);
        c.reinvite_busy = true;
        c.hold_wanted = None;
        Ok(())
    }

    pub fn on_reinvite_done(&mut self, id: CallId, hold: bool, result: Result<Option<Response>, String>) {
        let Some(c) = self.call_mut(id) else { return };
        c.reinvite_busy = false;
        if c.ended {
            return;
        }
        let offer = c.offer.take();
        match result {
            Ok(Some(resp)) if (200..300).contains(&resp.status_code.code()) => {
                let neg = match (offer, Sdp::parse(&String::from_utf8_lossy(&resp.body))) {
                    (Some(o), Ok(remote)) => sdp::accept(&o, &remote).ok(),
                    _ => None,
                };
                if let Some(n) = neg {
                    c.neg = Some(n);
                }
                c.local_hold = hold;
                let state = Self::held_state(c);
                self.set_call(id, |call| {
                    if call.state.is_established() && call.state != CallState::Transferring {
                        call.state = state;
                    }
                });
                self.apply_media(id);
                self.sync_keys();
            }
            Ok(Some(resp)) if matches!(resp.status_code.code(), 408 | 481) => {
                self.hangup_dialog(id);
                return self.end_call(id, EndCause::Error("the far end stopped answering".into()));
            }
            Ok(Some(resp)) => log::warn!("sip: call {}: re-INVITE refused with {}", id.0, resp.status_code),
            // The dialog was not ready for a re-INVITE (our 200 not yet
            // acknowledged): try once more shortly.
            Ok(None) => {
                c.hold_wanted = Some(hold);
                return self.after(Duration::from_millis(500), Timer::HoldRetry { call: id });
            }
            Err(e) => log::warn!("sip: call {}: re-INVITE failed: {e}", id.0),
        }
        self.on_hold_retry(id);
    }

    /// Sends a hold change that was waiting for a re-INVITE to finish.
    pub fn on_hold_retry(&mut self, id: CallId) {
        let Some(c) = self.call_mut(id) else { return };
        if let Some(want) = c.hold_wanted.take() {
            if want != c.local_hold && !c.reinvite_busy {
                let _ = self.send_reinvite(id, want);
            }
        }
    }

    /// Holds every connected call except `keep` and the members of its
    /// conference.
    fn hold_others(&mut self, keep: Option<CallId>) {
        let conf = keep.is_some_and(|k| self.conference.contains(&k));
        let ids: Vec<CallId> = self
            .calls
            .iter()
            .filter(|c| c.answered && !c.ended && !c.local_hold && Some(c.id) != keep)
            .filter(|c| !(conf && self.conference.contains(&c.id)))
            .map(|c| c.id)
            .collect();
        for id in ids {
            let _ = self.send_reinvite(id, true);
        }
    }

    // ---------------------------------------------------------- transfer

    fn refer(&mut self, id: CallId, refer_to: String, consult: Option<CallId>) -> CommandResult {
        let c = self.call(id).ok_or_else(|| invalid("no such call"))?;
        if !c.answered || c.ended {
            return Err(invalid("the call is not connected"));
        }
        let a = self.acct(c.acct).ok_or_else(|| invalid("no such account"))?;
        let me = format!("<sip:{}@{}>", a.cfg.user, a.cfg.host(c.alt).server);
        let (dialog, post) = (c.dialog.clone(), self.tx.clone());
        tokio::spawn(async move {
            let headers = vec![Header::ReferredBy(me.into())];
            let result = dialog
                .refer(rsipstack::sip::ReferTo::from(refer_to), Some(headers), None)
                .await
                .map_err(|e| e.to_string());
            let _ = post.send(Ev::ReferDone { call: id, result });
        });
        self.set_call(id, |call| call.state = CallState::Transferring);
        if let Some(k) = consult {
            self.set_call(k, |call| call.transfer_of = Some(id));
        }
        self.sync_keys();
        Ok(Reply::Done)
    }

    pub fn on_refer_done(&mut self, id: CallId, result: Result<Option<Response>, String>) {
        let ok = matches!(&result, Ok(Some(r)) if (200..300).contains(&r.status_code.code()));
        if !ok {
            log::info!("sip: call {}: REFER was not accepted", id.0);
            self.transfer_failed(id);
        }
        // Accepted: the NOTIFYs carry the outcome.
    }

    // ---------------------------------------------------------- commands

    pub fn on_command(&mut self, env: Envelope) {
        let (command, responder) = env.split();
        let result = self.command(command);
        responder.reply(result);
    }

    fn established(&self, id: CallId) -> Result<&CallRt, CommandError> {
        match self.call(id) {
            Some(c) if c.answered && !c.ended => Ok(c),
            Some(_) => Err(invalid("the call is not connected")),
            None => Err(invalid("no such call")),
        }
    }

    fn command(&mut self, command: Command) -> CommandResult {
        match command {
            Command::Dial { target, account, path } => {
                self.last_dialed = Some((target.clone(), account));
                self.dial(&target, account, path, None, None, None).map(Reply::Call)
            }
            Command::Redial => {
                let (target, account) = self.last_dialed.clone().ok_or_else(|| invalid("nothing to redial"))?;
                self.dial(&target, account, None, None, None, None).map(Reply::Call)
            }
            Command::DialVoicemail { account } => {
                let acct = self.pick_account(account)?;
                let vm = self.acct(acct).map(|a| a.cfg.voicemail_number.clone()).unwrap_or_default();
                if vm.is_empty() {
                    return Err(invalid("no voicemail number is configured"));
                }
                self.dial(&vm, Some(acct), None, None, None, None).map(Reply::Call)
            }
            Command::Answer { call, path } => self.answer(call, path),
            Command::Hangup { call } => self.hangup(call),
            Command::Reject { call, busy } => self.reject(call, if busy { 486 } else { 603 }),
            Command::Silence { call } => {
                self.call(call).ok_or_else(|| invalid("no such call"))?;
                self.media.ring(None);
                Ok(Reply::Done)
            }
            Command::Hold { call } => {
                self.established(call)?;
                self.send_reinvite(call, true).map(|_| Reply::Done)
            }
            Command::Resume { call } => {
                self.established(call)?;
                self.hold_others(Some(call));
                self.focus(Some(call));
                self.send_reinvite(call, false).map(|_| Reply::Done)
            }
            Command::Focus { call } => {
                let c = self.call(call).ok_or_else(|| invalid("no such call"))?;
                let resume = c.answered && !c.ended && c.local_hold;
                self.focus(Some(call));
                if resume {
                    self.hold_others(Some(call));
                    self.send_reinvite(call, false)?;
                }
                self.refresh_connect();
                Ok(Reply::Done)
            }
            Command::TransferBlind { call, target } => {
                let acct = self.established(call)?.acct;
                let uri = self.dial_uri(acct, &target)?;
                self.refer(call, format!("<{uri}>"), None)
            }
            Command::TransferAttended { call, consult } => {
                // Attended transfer: the transferee calls the consulted
                // party, replacing our consultation dialog.
                let k = self.established(consult)?;
                let d = k.dialog.id();
                let replaces =
                    util::percent_encode(&format!("{};to-tag={};from-tag={}", d.call_id, d.remote_tag, d.local_tag));
                let base = k.peer.split('?').next().unwrap_or(&k.peer).to_string();
                self.established(call)?;
                self.refer(call, format!("<{base}?Replaces={replaces}>"), Some(consult))
            }
            Command::Park { call } => {
                let acct = self.established(call)?.acct;
                let uri = self.dial_uri(acct, &self.park_extension.clone())?;
                self.refer(call, format!("<{uri}>"), None)
            }
            Command::SendToVoicemail { call } => {
                let c = self.call(call).ok_or_else(|| invalid("no such call"))?;
                let vm = self.acct(c.acct).map(|a| a.cfg.voicemail_number.clone()).unwrap_or_default();
                if c.answered {
                    if vm.is_empty() {
                        return Err(invalid("no voicemail number is configured"));
                    }
                    let uri = self.dial_uri(c.acct, &vm)?;
                    self.refer(call, format!("<{uri}>"), None)
                } else if vm.is_empty() {
                    self.reject(call, 486)
                } else {
                    if c.server.is_none() {
                        return Err(invalid("the call is not ringing"));
                    }
                    self.redirect_call(call, &vm, EndCause::Rejected);
                    Ok(Reply::Done)
                }
            }
            Command::Conference { a, b } => {
                self.established(a)?;
                self.established(b)?;
                self.conference = vec![a, b];
                for id in [a, b] {
                    self.set_call(id, |c| c.in_conference = true);
                    if self.call(id).is_some_and(|c| c.local_hold || c.hold_wanted == Some(true)) {
                        self.send_reinvite(id, false)?;
                    }
                }
                self.focus(Some(a));
                self.refresh_connect();
                Ok(Reply::Done)
            }
            Command::SplitConference { keep } => {
                if !self.conference.contains(&keep) {
                    return Err(invalid("the call is not in the conference"));
                }
                let members = std::mem::take(&mut self.conference);
                for id in members {
                    self.set_call(id, |c| c.in_conference = false);
                    if id != keep {
                        let _ = self.send_reinvite(id, true);
                    }
                }
                self.focus(Some(keep));
                self.refresh_connect();
                Ok(Reply::Done)
            }
            Command::Dtmf { call, digit } => {
                let digit = digit.to_ascii_uppercase();
                if !"0123456789*#ABCD".contains(digit) {
                    return Err(invalid("not a DTMF digit"));
                }
                let c = self.call(call).ok_or_else(|| invalid("no such call"))?;
                let Some(n) = &c.neg else { return Err(invalid("the call has no media")) };
                if n.dtmf_tx_pt.is_some() {
                    self.media.send_dtmf(call, digit, 100);
                } else {
                    // No telephone-event agreed: SIP INFO.
                    let dialog = c.dialog.clone();
                    let body = format!("Signal={digit}\r\nDuration=100\r\n").into_bytes();
                    tokio::spawn(async move {
                        let headers = vec![Header::ContentType("application/dtmf-relay".to_string().into())];
                        let _ = dialog.info(Some(headers), Some(body)).await;
                    });
                }
                Ok(Reply::Done)
            }
            Command::Pickup { blf_entry } => {
                let b = self
                    .cfg
                    .blf
                    .iter()
                    .find(|b| b.id == blf_entry)
                    .cloned()
                    .ok_or_else(|| invalid("no such BLF entry"))?;
                if !b.pickup_prefix.is_empty() {
                    let target = format!("{}{}", b.pickup_prefix, b.number);
                    return self.dial(&target, Some(b.account), None, None, None, None).map(Reply::Call);
                }
                let info = self.blf_dialogs.get(&blf_entry).filter(|i| !i.call_id.is_empty()).cloned();
                let uri = self.store.get::<Presence>().get(&blf_entry).map(|e| e.uri.clone()).unwrap_or_default();
                match info {
                    Some(i) if !uri.is_empty() => {
                        let replaces = format!("{};to-tag={};from-tag={};early-only", i.call_id, i.remote_tag, i.local_tag);
                        self.dial(&uri, Some(b.account), None, Some(replaces), None, None).map(Reply::Call)
                    }
                    _ => Err(invalid("nothing to pick up")),
                }
            }
            Command::SetDnd { on } => {
                self.store.update::<Features, _>(|f| f.dnd = on);
                Ok(Reply::Done)
            }
            Command::SetForward { kind, target, no_answer_secs } => {
                self.store.update::<Features, _>(|f| {
                    match kind {
                        ForwardKind::Always => f.forward.always = target,
                        ForwardKind::Busy => f.forward.busy = target,
                        ForwardKind::NoAnswer => f.forward.no_answer = target,
                    }
                    if let Some(s) = no_answer_secs {
                        f.forward.no_answer_secs = s;
                    }
                });
                Ok(Reply::Done)
            }
            Command::SetPresence { status, text } => {
                self.store.update::<Presence, _>(|p| {
                    p.own_status = status;
                    p.own_status_text = text.clone();
                });
                self.publish_presence(status, &text);
                Ok(Reply::Done)
            }
            Command::Reregister { account } => {
                let ids: Vec<AccountId> = match account {
                    Some(id) => vec![self.pick_account(Some(id))?],
                    None => self.accts.iter().map(|a| a.cfg.id).collect(),
                };
                for id in ids {
                    self.start_registration(id);
                }
                Ok(Reply::Done)
            }
            Command::ClearCallLog => {
                self.store.update::<CallLog, _>(|l| {
                    l.entries.clear();
                    l.missed_unseen = 0;
                });
                Ok(Reply::Done)
            }
            Command::MarkMissedSeen => {
                self.store.update::<CallLog, _>(|l| l.missed_unseen = 0);
                Ok(Reply::Done)
            }
            Command::SetRapidDialPage { page } => {
                let mut ok = false;
                self.store.update::<Keys, _>(|k| {
                    if page < k.rapid_pages.max(1) {
                        k.rapid_page = page;
                        ok = true;
                    }
                });
                if ok { Ok(Reply::Done) } else { Err(invalid("no such page")) }
            }
            other => Err(invalid(format!("not a call command: {other:?}"))),
        }
    }

    pub(crate) fn hangup(&mut self, id: CallId) -> CommandResult {
        let c = self.call(id).ok_or_else(|| invalid("no such call"))?;
        if c.ended {
            return Ok(Reply::Done);
        }
        if !c.answered && !c.outgoing {
            return self.reject(id, 486);
        }
        self.hangup_dialog(id);
        self.end_call(id, EndCause::LocalHangup);
        Ok(Reply::Done)
    }

    // ------------------------------------------------------------- media

    pub fn on_media(&mut self, ev: MediaEvent) {
        match ev {
            MediaEvent::Dtmf { call, digit } => self.store.emit(Event::DtmfReceived { call, digit }),
            MediaEvent::Stats { call, stats } => self.set_call(call, |c| c.stats = stats),
            MediaEvent::RtpTimeout { call } => {
                let dead = self.call(call).is_some_and(|c| c.answered && !c.ended && !c.local_hold && !c.remote_hold);
                if dead {
                    log::warn!("sip: call {}: no media; hanging up", call.0);
                    self.hangup_dialog(call);
                    self.end_call(call, EndCause::Error("no audio from the far end".into()));
                }
            }
        }
    }
}

/// What `invite_for` needs before the call record exists.
struct PendingCall {
    acct: AccountId,
    replaces: Option<String>,
    referred_by: Option<String>,
}
