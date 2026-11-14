//! Call control: INVITE dialogs in both directions, hold, transfer,
//! conference, DTMF, forward and DND, and the commands of
//! [`dphone_state::Domain::Calls`].

use crate::bodies;
use crate::core::{CallRt, Core, Ctx, Dialog, Owner, ReferFrom, ServerInvite, Spec, Timer};
use crate::media::{MediaEvent, RingRequest, StreamParams, StreamSrtp};
use crate::sdp::{self, Crypto, Direction as SdpDir, Sdp, SrtpPolicy, SrtpSuite};
use crate::util::{self, NameAddr, SipUri};
use dphone_state::{
    clock, AccountId, AudioPath, Call, CallId, CallLog, CallLogEntry, CallLogKind, CallState, Calls, Command,
    CommandError, CommandResult, Direction, EndCause, Envelope, Event, Features, ForwardKind, Keys, Party,
    Presence, Reply, Tone, Transport,
};
use rsipstack::sip::{Request, Response};
use rsipstack::transaction::transaction::Transaction;
use std::time::Duration;

fn invalid(s: impl Into<String>) -> CommandError {
    CommandError::Invalid(s.into())
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

    fn call_transport(&self, c: &CallRt) -> Transport {
        self.acct(c.acct).map(|a| a.cfg.host(c.alt).transport).unwrap_or(Transport::Udp)
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
        let live = |id: &CallId| {
            self.call(*id).is_some_and(|c| !c.ended && c.neg.is_some() && !c.local_hold)
        };
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

    /// Our SDP key for a call, made on first use.
    fn ensure_key(&mut self, id: CallId) {
        let srtp = self.call(id).is_some_and(|c| self.acct(c.acct).is_some_and(|a| a.cfg.host(c.alt).srtp));
        if let Some(c) = self.call_mut(id) {
            if srtp && c.key.is_none() {
                c.key = Crypto::generate(1, SrtpSuite::AesCm128HmacSha1_80).ok();
            }
        }
    }

    fn in_dialog_spec(
        &mut self,
        id: CallId,
        method: &str,
        headers: Vec<(String, String)>,
        body: Option<(&str, Vec<u8>)>,
    ) -> Option<(Spec, rsipstack::transport::SipAddr, Option<(String, String)>)> {
        let transport = self.call_transport(self.call(id)?);
        let user = self.acct(self.call(id)?.acct).map(|a| a.cfg.user.clone()).unwrap_or_default();
        let contact = self.contact(&user, transport);
        let cred = self.call(id).and_then(|c| self.acct(c.acct).map(|a| a.cfg.credentials(c.alt)));
        let c = self.call_mut(id)?;
        let cseq = c.dialog.next_cseq();
        let d = c.dialog.clone();
        let (dest, t) = Core::dialog_dest(&d, transport).ok()?;
        let mut h = vec![("Contact".to_string(), contact)];
        h.extend(headers);
        let (content_type, body) = match body {
            Some((c, b)) => (c.to_string(), b),
            None => (String::new(), Vec::new()),
        };
        Some((
            Spec {
                method: method.into(),
                ruri: d.target.clone(),
                from: d.local_header(),
                to: d.remote_header(),
                call_id: d.call_id.clone(),
                cseq,
                routes: d.routes.clone(),
                headers: h,
                content_type,
                body,
                transport: t,
            },
            dest,
            cred,
        ))
    }

    fn send_in_dialog(
        &mut self,
        id: CallId,
        method: &str,
        headers: Vec<(String, String)>,
        body: Option<(&str, Vec<u8>)>,
        ctx: Ctx,
    ) -> Option<u64> {
        let (spec, dest, cred) = self.in_dialog_spec(id, method, headers, body)?;
        self.send(spec, dest, ctx, cred, Owner::Call(id)).ok()
    }

    fn send_bye(&mut self, id: CallId) {
        if self.call(id).is_some_and(|c| !c.dialog.remote_tag.is_empty()) {
            self.send_in_dialog(id, "BYE", Vec::new(), None, Ctx::Bye);
        }
    }

    /// Ends a call locally: media, state, call log, focus, events.
    pub(crate) fn end_call(&mut self, id: CallId, cause: EndCause) {
        let Some(c) = self.call_mut(id) else { return };
        if c.ended {
            return;
        }
        c.ended = true;
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
            let (code, text) = match &cause {
                EndCause::Failed { code, reason } => (*code, reason.clone()),
                EndCause::Busy => (486, "Busy Here".into()),
                _ => (487, "Request Terminated".into()),
            };
            self.notify_refer(r.call, code, &text, true);
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
            let duration = call.connected_mono_ms.map(|c| (clock::mono_ms().saturating_sub(c) / 1000) as u32).unwrap_or(0);
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
        let alt = a.alt;
        let dialog = Dialog {
            call_id: format!("{}@dphone", util::token(20)),
            local_tag: util::token(10),
            remote_tag: String::new(),
            local: self.aor(a),
            remote: format!("<{uri}>"),
            target: uri.clone(),
            routes: Vec::new(),
            cseq: 0,
        };
        let number = SipUri::parse(&uri).map(|u| util::percent_decode(&u.user)).unwrap_or_default();
        let mut call = Call::new(id, acct, Direction::Outgoing, Party { name: String::new(), number, uri: uri.clone() });
        call.sip_call_id = dialog.call_id.clone();
        self.calls.push(CallRt {
            id,
            acct,
            alt,
            dialog,
            outgoing: true,
            invite: None,
            provisional_seen: false,
            cancel_wanted: false,
            server: None,
            reinvite_server: None,
            rtp_port,
            session_id: clock::unix_secs(),
            sdp_version: 1,
            offer: None,
            remote_offer: None,
            key: None,
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
            target_text: target.to_string(),
        });
        self.store.update::<Calls, _>(|c| {
            c.calls.push(call);
            c.focused = Some(id);
        });
        if let Some(p) = path {
            self.media.request_path(p);
        }
        self.media.tone(None);
        if let Err(e) = self.send_invite(id) {
            self.end_call(id, EndCause::Error(e.clone()));
            return Err(CommandError::Failed(e));
        }
        self.sync_keys();
        Ok(id)
    }

    fn send_invite(&mut self, id: CallId) -> Result<(), String> {
        self.ensure_key(id);
        let c = self.call(id).ok_or("no such call")?;
        let a = self.acct(c.acct).ok_or("no such account")?;
        let transport = a.cfg.host(c.alt).transport;
        let offer = sdp::offer(&self.local_media(c, SdpDir::SendRecv), c.key.as_ref());
        let mut headers = vec![
            ("Contact".to_string(), self.contact(&a.cfg.user, transport)),
            ("Allow".to_string(), Core::allow().to_string()),
            ("Supported".to_string(), "replaces".to_string()),
        ];
        if let Some(r) = &c.replaces {
            headers.push(("Replaces".into(), r.clone()));
        }
        if let Some(r) = &c.referred_by {
            headers.push(("Referred-By".into(), r.clone()));
        }
        let (dest, transport) = self.request_dest(a, &c.dialog.target)?;
        headers[0].1 = self.contact(&a.cfg.user, transport);
        let cred = Some(a.cfg.credentials(c.alt));
        let d = &c.dialog;
        let spec = Spec {
            method: "INVITE".into(),
            ruri: d.target.clone(),
            from: d.local_header(),
            to: d.remote.clone(),
            call_id: d.call_id.clone(),
            cseq: d.cseq + 1,
            routes: Vec::new(),
            headers,
            content_type: "application/sdp".into(),
            body: offer.render().into_bytes(),
            transport,
        };
        let pending = self.send(spec, dest, Ctx::Invite { call: id }, cred, Owner::Call(id))?;
        let c = self.call_mut(id).ok_or("no such call")?;
        c.dialog.cseq += 1;
        c.offer = Some(offer);
        c.invite = Some(pending);
        c.provisional_seen = false;
        Ok(())
    }

    /// Records dialog identifiers from a response to our INVITE.
    fn learn_dialog(c: &mut CallRt, resp: &Response, final_response: bool) {
        if let Some(tag) = util::header(&resp.headers, "To").and_then(|t| NameAddr::parse(t).tag().map(str::to_string)) {
            c.dialog.remote_tag = tag;
        }
        if let Some(contact) = util::header(&resp.headers, "Contact") {
            c.dialog.target = NameAddr::parse(contact).uri;
        }
        if final_response || c.dialog.routes.is_empty() {
            let mut routes: Vec<String> = util::headers_all(&resp.headers, "Record-Route")
                .into_iter()
                .flat_map(|v| v.split(',').map(|x| x.trim().to_string()).collect::<Vec<_>>())
                .collect();
            routes.reverse();
            c.dialog.routes = routes;
        }
    }

    pub(crate) fn on_invite_provisional(&mut self, id: CallId, resp: &Response) {
        let code = resp.status_code.code();
        let Some(c) = self.call_mut(id) else { return };
        c.provisional_seen = true;
        if c.cancel_wanted || c.ended {
            if let Some(inv) = c.invite {
                c.cancel_wanted = false;
                self.send_cancel(inv);
            }
            return;
        }
        if code == 100 {
            self.set_call(id, |call| {
                if call.state == CallState::Dialing {
                    call.state = CallState::Proceeding;
                }
            });
            return;
        }
        Self::learn_dialog(c, resp, false);
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
            self.notify_refer(r.call, 180, "Ringing", false);
        }
        self.sync_keys();
    }

    pub(crate) fn on_invite_final(&mut self, id: CallId, code: u16, reason: &str, resp: Option<&Response>) {
        let Some(c) = self.call_mut(id) else { return };
        c.invite = None;
        if (200..300).contains(&code) {
            let Some(resp) = resp else { return };
            Self::learn_dialog(c, resp, true);
            if c.ended || c.cancel_wanted {
                // Answered while we were hanging up: the dialog exists, so
                // it needs a BYE.
                self.send_bye(id);
                return;
            }
            let negotiated = match (&c.offer, Sdp::parse(&String::from_utf8_lossy(&resp.body))) {
                (Some(offer), Ok(remote)) => sdp::accept(offer, &remote),
                (_, Err(e)) if c.neg.is_some() => {
                    // No SDP in the 200 after an early answer: keep it.
                    let _ = e;
                    Ok(c.neg.clone().expect("checked"))
                }
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
                    self.send_bye(id);
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
                self.notify_refer(r.call, 200, "OK", true);
            }
            self.sync_keys();
            return;
        }
        if c.ended {
            return;
        }
        // Redirect: follow the first Contact once.
        if (300..400).contains(&code) && !c.target_text.starts_with('\u{1}') {
            if let Some(contact) = resp.and_then(|r| util::header(&r.headers, "Contact")).map(|c| NameAddr::parse(c).uri) {
                if SipUri::parse(&contact).is_some() {
                    c.target_text.insert(0, '\u{1}');
                    c.dialog.target = contact.clone();
                    c.dialog.remote = format!("<{contact}>");
                    c.dialog.remote_tag.clear();
                    if self.send_invite(id).is_ok() {
                        return;
                    }
                }
            }
        }
        let cause = match code {
            486 | 600 => EndCause::Busy,
            487 => EndCause::LocalHangup,
            408 | 480 => EndCause::NoAnswer,
            _ => EndCause::Failed { code, reason: reason.to_string() },
        };
        self.end_call(id, cause);
    }

    // --------------------------------------------------------- incoming

    /// A new INVITE transaction: a call, a re-INVITE, or a replacement.
    pub fn on_invite(&mut self, tx: Transaction) {
        let req = tx.original.clone();
        let call_id = util::header(&req.headers, "Call-ID").unwrap_or("").to_string();
        let to = NameAddr::parse(util::header(&req.headers, "To").unwrap_or(""));
        let from = NameAddr::parse(util::header(&req.headers, "From").unwrap_or(""));
        let server = self.adopt_invite(tx);
        if let Some(tag) = to.tag() {
            let found = self.calls.iter().find(|c| !c.ended && c.dialog.call_id == call_id && c.dialog.local_tag == tag).map(|c| c.id);
            return match found {
                Some(id) => self.on_reinvite(id, server),
                None => self.server_respond(&server, self.response(&req, 481, None, &[], None)),
            };
        }
        let tag = util::token(10);
        let Some(acct) = self.account_for(&req) else {
            return self.server_respond(&server, self.response(&req, 404, Some(&tag), &[], None));
        };
        let features = self.store.get::<Features>();
        let host = self.acct(acct).map(|a| a.cfg.host(a.alt).server.clone()).unwrap_or_default();
        let redirect = |target: &str| {
            let uri = if target.starts_with("sip:") { target.to_string() } else { format!("sip:{target}@{host}") };
            self.response(&req, 302, Some(&tag), &[("Contact", format!("<{uri}>"))], None)
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
                .find(|c| !c.ended && c.dialog.call_id == cid && c.dialog.local_tag == to_tag && c.dialog.remote_tag == from_tag)
                .map(|c| c.id)
        });
        if replaced.is_none() {
            if !features.forward.always.is_empty() {
                return self.server_respond(&server, redirect(&features.forward.always));
            }
            if features.dnd {
                return self.server_respond(&server, self.response(&req, 486, Some(&tag), &[], None));
            }
            let live = self.calls.iter().filter(|c| !c.ended).count();
            if live > 0 && (!self.cfg.call_waiting || !features.call_waiting || live >= self.cfg.max_calls) {
                let resp = if features.forward.busy.is_empty() {
                    self.response(&req, 486, Some(&tag), &[], None)
                } else {
                    redirect(&features.forward.busy)
                };
                return self.server_respond(&server, resp);
            }
        }
        let id = self.store.next_call_id();
        let rtp_port = match self.media.open(id) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("sip: no media for an incoming call: {e}");
                return self.server_respond(&server, self.response(&req, 480, Some(&tag), &[], None));
            }
        };
        let remote_offer = if req.body.is_empty() {
            None
        } else {
            match Sdp::parse(&String::from_utf8_lossy(&req.body)) {
                Ok(s) => Some(s),
                Err(e) => {
                    log::warn!("sip: bad offer: {e}");
                    self.media.close(id);
                    return self.server_respond(&server, self.response(&req, 488, Some(&tag), &[], None));
                }
            }
        };
        let alt = self.acct(acct).is_some_and(|a| a.alt);
        let routes: Vec<String> = util::headers_all(&req.headers, "Record-Route")
            .into_iter()
            .flat_map(|v| v.split(',').map(|x| x.trim().to_string()).collect::<Vec<_>>())
            .collect();
        let strip = |n: &NameAddr| {
            if n.display.is_empty() { format!("<{}>", n.uri) } else { format!("\"{}\" <{}>", n.display, n.uri) }
        };
        let dialog = Dialog {
            call_id: call_id.clone(),
            local_tag: tag.clone(),
            remote_tag: from.tag().unwrap_or("").to_string(),
            local: strip(&to),
            remote: strip(&from),
            target: util::header(&req.headers, "Contact").map(|c| NameAddr::parse(c).uri).unwrap_or_else(|| from.uri.clone()),
            routes,
            cseq: 0,
        };
        let alert_info = util::header(&req.headers, "Alert-Info").unwrap_or("").to_string();
        let call_info = util::header(&req.headers, "Call-Info").unwrap_or("").to_ascii_lowercase();
        let lower_alert = alert_info.to_ascii_lowercase();
        let wants_auto = call_info.contains("answer-after=0")
            || lower_alert.contains("auto answer")
            || lower_alert.contains("autoanswer")
            || lower_alert.contains("ring answer")
            || lower_alert.contains("intercom");
        let mut rt = CallRt {
            id,
            acct,
            alt,
            dialog,
            outgoing: false,
            invite: None,
            provisional_seen: false,
            cancel_wanted: false,
            server: None,
            reinvite_server: None,
            rtp_port,
            session_id: clock::unix_secs(),
            sdp_version: 1,
            offer: None,
            remote_offer,
            key: None,
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
            target_text: String::new(),
        };
        // Refuse now what cannot be answered later.
        if let Some(offer) = &rt.remote_offer {
            let srtp = self.acct(acct).is_some_and(|a| a.cfg.host(alt).srtp);
            let key = if srtp { Crypto::generate(1, SrtpSuite::AesCm128HmacSha1_80).ok() } else { None };
            rt.key = key;
            if let Err(e) = sdp::answer(offer, &self.local_media(&rt, SdpDir::SendRecv), rt.key.as_ref()) {
                log::info!("sip: refusing a call: {e}");
                self.media.close(id);
                return self.server_respond(&server, self.response(&req, 488, Some(&tag), &[], None));
            }
        }
        let pai = util::header(&req.headers, "P-Asserted-Identity").map(NameAddr::parse);
        let who = pai.as_ref().unwrap_or(&from);
        let party = Party { name: who.display.clone(), number: util::percent_decode(&who.user()), uri: who.uri.clone() };
        let mut call = Call::new(id, acct, Direction::Incoming, party);
        call.sip_call_id = call_id;
        call.auto_answer = wants_auto && self.cfg.allow_auto_answer;
        call.paging = lower_alert.contains("paging") || call_info.contains("paging");
        let auto = call.auto_answer;
        let user = self.acct(acct).map(|a| a.cfg.user.clone()).unwrap_or_default();
        let contact = self.contact(&user, self.acct(acct).map(|a| a.cfg.host(alt).transport).unwrap_or(Transport::Udp));
        self.server_respond(&server, self.response(&req, 180, Some(&tag), &[("Contact", contact)], None));
        rt.server = Some(server);
        self.calls.push(rt);
        let others = self.store.get::<Calls>().live().any(|c| c.state.is_established() || !c.state.is_ended());
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
                self.send_bye(old);
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
            let secs = if features.forward.no_answer_secs > 0 { features.forward.no_answer_secs } else { self.cfg.no_answer_secs };
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

    /// Answers an unanswered incoming call with a 302.
    fn redirect_call(&mut self, id: CallId, target: &str, cause: EndCause) {
        let Some(c) = self.call_mut(id) else { return };
        let Some(server) = c.server.take() else { return };
        let tag = c.dialog.local_tag.clone();
        let acct = c.acct;
        let host = self.acct(acct).map(|a| a.cfg.host(a.alt).server.clone()).unwrap_or_default();
        let uri = if target.starts_with("sip:") { target.to_string() } else { format!("sip:{target}@{host}") };
        let resp = self.response(&server.req, 302, Some(&tag), &[("Contact", format!("<{uri}>"))], None);
        self.server_respond(&server, resp);
        self.end_call(id, cause);
    }

    fn reject(&mut self, id: CallId, code: u16) -> CommandResult {
        let c = self.call_mut(id).ok_or_else(|| invalid("no such call"))?;
        let server = c.server.take().ok_or_else(|| invalid("the call is not ringing"))?;
        if c.answered {
            return Err(invalid("the call is already answered"));
        }
        let tag = c.dialog.local_tag.clone();
        let resp = self.response(&server.req, code, Some(&tag), &[], None);
        self.server_respond(&server, resp);
        self.end_call(id, EndCause::Rejected);
        Ok(Reply::Done)
    }

    pub(crate) fn answer(&mut self, id: CallId, path: Option<AudioPath>) -> CommandResult {
        self.ensure_key(id);
        let c = self.call(id).ok_or_else(|| invalid("no such call"))?;
        if c.answered || c.ended || c.outgoing {
            return Err(invalid("the call is not ringing"));
        }
        let Some(server) = c.server.as_ref() else { return Err(invalid("the call is not ringing")) };
        let local = self.local_media(c, SdpDir::SendRecv);
        let (body, neg, offer) = match &c.remote_offer {
            Some(remote) => {
                let (ans, neg) = sdp::answer(remote, &local, c.key.as_ref()).map_err(CommandError::Failed)?;
                (ans.render(), Some(neg), None)
            }
            // An INVITE without SDP: we offer, the ACK answers.
            None => {
                let offer = sdp::offer(&local, c.key.as_ref());
                (offer.render(), None, Some(offer))
            }
        };
        let user = self.acct(c.acct).map(|a| a.cfg.user.clone()).unwrap_or_default();
        let contact = self.contact(&user, self.call_transport(c));
        let resp = self.response(
            &server.req,
            200,
            Some(&c.dialog.local_tag),
            &[("Contact", contact), ("Allow", Core::allow().to_string()), ("Supported", "replaces".into())],
            Some(("application/sdp", body.into_bytes())),
        );
        self.server_respond(server, resp);
        self.hold_others(Some(id));
        let c = self.call_mut(id).expect("checked above");
        c.answered = true;
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

    /// ACK or CANCEL on a server INVITE transaction.
    pub fn on_server_req(&mut self, tx: u64, req: Request) {
        let Some(c) = self
            .calls
            .iter_mut()
            .find(|c| c.server.as_ref().is_some_and(|s| s.tx == tx) || c.reinvite_server.as_ref().is_some_and(|s| s.tx == tx))
        else {
            return;
        };
        let id = c.id;
        match req.method {
            rsipstack::sip::Method::Cancel => {
                if c.answered || c.ended {
                    return;
                }
                let Some(server) = c.server.take() else { return };
                let tag = c.dialog.local_tag.clone();
                let elsewhere = util::header(&req.headers, "Reason").is_some_and(|r| r.to_ascii_lowercase().contains("elsewhere"));
                let resp = self.response(&server.req, 487, Some(&tag), &[], None);
                self.server_respond(&server, resp);
                self.end_call(id, EndCause::Cancelled { answered_elsewhere: elsewhere });
            }
            rsipstack::sip::Method::Ack => {
                // The answer to an offer we made in a 200.
                if let (Some(offer), false) = (c.offer.clone(), req.body.is_empty()) {
                    match Sdp::parse(&String::from_utf8_lossy(&req.body)).and_then(|r| sdp::accept(&offer, &r)) {
                        Ok(n) => {
                            c.neg = Some(n);
                            c.offer = None;
                            self.apply_media(id);
                        }
                        Err(e) => {
                            log::warn!("sip: call {}: bad answer in ACK: {e}", id.0);
                            self.send_bye(id);
                            self.end_call(id, EndCause::Error(e));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    pub fn on_server_done(&mut self, tx: u64) {
        for c in self.calls.iter_mut() {
            if c.server.as_ref().is_some_and(|s| s.tx == tx) {
                c.server = None;
            }
            if c.reinvite_server.as_ref().is_some_and(|s| s.tx == tx) {
                c.reinvite_server = None;
            }
        }
    }

    // -------------------------------------------- in-dialog, from remote

    /// Finds the call an in-dialog request belongs to.
    pub fn call_for(&self, req: &Request) -> Option<CallId> {
        let call_id = util::header(&req.headers, "Call-ID")?;
        let to_tag = NameAddr::parse(util::header(&req.headers, "To")?).tag()?.to_string();
        self.calls.iter().find(|c| c.dialog.call_id == call_id && c.dialog.local_tag == to_tag).map(|c| c.id)
    }

    /// Applies an offer of theirs inside the dialog; returns our answer.
    fn answer_reoffer(&mut self, id: CallId, body: &[u8]) -> Result<String, String> {
        self.ensure_key(id);
        let c = self.call_mut(id).ok_or("no such call")?;
        c.sdp_version += 1;
        let c = self.call(id).ok_or("no such call")?;
        let remote = Sdp::parse(&String::from_utf8_lossy(body))?;
        let dir = if c.local_hold { SdpDir::SendOnly } else { SdpDir::SendRecv };
        let (ans, neg) = sdp::answer(&remote, &self.local_media(c, dir), c.key.as_ref())?;
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

    fn on_reinvite(&mut self, id: CallId, server: ServerInvite) {
        let req = server.req.clone();
        let Some(c) = self.call(id) else { return };
        let tag = c.dialog.local_tag.clone();
        let user = self.acct(c.acct).map(|a| a.cfg.user.clone()).unwrap_or_default();
        let contact = self.contact(&user, self.call_transport(c));
        if c.reinvite_busy {
            return self.server_respond(&server, self.response(&req, 491, Some(&tag), &[], None));
        }
        if let Some(target) = util::header(&req.headers, "Contact").map(|c| NameAddr::parse(c).uri) {
            if let Some(c) = self.call_mut(id) {
                c.dialog.target = target;
            }
        }
        let resp = if req.body.is_empty() {
            // Offerless re-INVITE: offer what we have now.
            self.ensure_key(id);
            let Some(c) = self.call_mut(id) else { return };
            c.sdp_version += 1;
            let c = self.call(id).expect("exists");
            let dir = if c.local_hold { SdpDir::SendOnly } else { SdpDir::SendRecv };
            let offer = sdp::offer(&self.local_media(c, dir), c.key.as_ref());
            let body = offer.render().into_bytes();
            if let Some(c) = self.call_mut(id) {
                c.offer = Some(offer);
            }
            self.response(&req, 200, Some(&tag), &[("Contact", contact)], Some(("application/sdp", body)))
        } else {
            match self.answer_reoffer(id, &req.body) {
                Ok(ans) => self.response(
                    &req,
                    200,
                    Some(&tag),
                    &[("Contact", contact)],
                    Some(("application/sdp", ans.into_bytes())),
                ),
                Err(e) => {
                    log::warn!("sip: call {}: refused re-INVITE: {e}", id.0);
                    self.response(&req, 488, Some(&tag), &[], None)
                }
            }
        };
        self.server_respond(&server, resp);
        if let Some(c) = self.call_mut(id) {
            c.reinvite_server = Some(server);
        }
    }

    /// BYE, UPDATE, INFO, REFER, NOTIFY, OPTIONS, MESSAGE inside a call.
    pub fn on_call_request(&mut self, id: CallId, tx: Transaction) {
        use rsipstack::sip::Method as M;
        let req = tx.original.clone();
        match req.method {
            M::Bye => {
                self.respond_code(tx, 200);
                self.end_call(id, EndCause::RemoteHangup);
            }
            M::Update => {
                if req.body.is_empty() {
                    return self.respond_code(tx, 200);
                }
                let resp = match self.answer_reoffer(id, &req.body) {
                    Ok(ans) => self.response(&req, 200, None, &[], Some(("application/sdp", ans.into_bytes()))),
                    Err(_) => self.response(&req, 488, None, &[], None),
                };
                self.respond(tx, resp);
            }
            M::Info => {
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
                self.respond_code(tx, 200);
            }
            M::Refer => self.on_refer(id, tx),
            M::Notify => self.on_call_notify(id, tx),
            M::Message => self.on_message(tx),
            M::Options => {
                let resp = self.response(&req, 200, None, &[("Allow", Core::allow().to_string())], None);
                self.respond(tx, resp);
            }
            _ => self.respond_code(tx, 405),
        }
    }

    /// We are asked to call someone else (we are the transferee).
    fn on_refer(&mut self, id: CallId, tx: Transaction) {
        let req = tx.original.clone();
        let Some(refer_to) = util::header(&req.headers, "Refer-To").map(NameAddr::parse) else {
            return self.respond_code(tx, 400);
        };
        let Some(uri) = SipUri::parse(&refer_to.uri) else { return self.respond_code(tx, 400) };
        let replaces = uri
            .headers
            .split('&')
            .find_map(|h| h.strip_prefix("Replaces=").or_else(|| h.strip_prefix("replaces=")))
            .map(util::percent_decode);
        let target = refer_to.uri.split('?').next().unwrap_or(&refer_to.uri).to_string();
        let referred_by = util::header(&req.headers, "Referred-By").map(str::to_string);
        let acct = self.call(id).map(|c| c.acct);
        self.respond_code(tx, 202);
        self.notify_refer(id, 100, "Trying", false);
        if let Err(e) = self.dial(&target, acct, None, replaces, referred_by, Some(ReferFrom { call: id })) {
            log::warn!("sip: cannot follow REFER: {e}");
            self.notify_refer(id, 503, "Service Unavailable", true);
        }
    }

    /// Reports the progress of a REFER we are following.
    fn notify_refer(&mut self, id: CallId, code: u16, text: &str, last: bool) {
        if self.call(id).is_none_or(|c| c.ended) {
            return;
        }
        let state = if last { "terminated;reason=noresource".to_string() } else { "active;expires=60".to_string() };
        self.send_in_dialog(
            id,
            "NOTIFY",
            vec![("Event".into(), "refer".into()), ("Subscription-State".into(), state)],
            Some(("message/sipfrag;version=2.0", format!("SIP/2.0 {code} {text}\r\n").into_bytes())),
            Ctx::Other("NOTIFY"),
        );
    }

    /// Progress of a transfer we asked for.
    fn on_call_notify(&mut self, id: CallId, tx: Transaction) {
        let req = tx.original.clone();
        let event = util::header(&req.headers, "Event").unwrap_or("").to_ascii_lowercase();
        self.respond_code(tx, 200);
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
            self.send_bye(id);
            self.end_call(id, EndCause::Transferred);
            if let Some(c) = consult {
                self.send_bye(c);
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
        let offer = sdp::offer(&self.local_media(c, dir), c.key.as_ref());
        let body = offer.render().into_bytes();
        let sent = self.send_in_dialog(
            id,
            "INVITE",
            vec![("Allow".into(), Core::allow().into())],
            Some(("application/sdp", body)),
            Ctx::Reinvite { call: id, hold },
        );
        if sent.is_none() {
            return Err(CommandError::Failed("cannot send the re-INVITE".into()));
        }
        let c = self.call_mut(id).expect("exists");
        c.offer = Some(offer);
        c.reinvite_busy = true;
        c.hold_wanted = None;
        Ok(())
    }

    pub(crate) fn on_reinvite_final(&mut self, id: CallId, hold: bool, code: u16, resp: Option<&Response>) {
        let Some(c) = self.call_mut(id) else { return };
        c.reinvite_busy = false;
        if c.ended {
            return;
        }
        let offer = c.offer.take();
        match code {
            200..=299 => {
                if let Some(target) = resp.and_then(|r| util::header(&r.headers, "Contact")).map(|c| NameAddr::parse(c).uri) {
                    c.dialog.target = target;
                }
                let neg = match (offer, resp.map(|r| Sdp::parse(&String::from_utf8_lossy(&r.body)))) {
                    (Some(o), Some(Ok(remote))) => sdp::accept(&o, &remote).ok(),
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
            491 => {
                // Glare: try again after a random pause (RFC 3261 14.1).
                c.hold_wanted = Some(hold);
                let ms = 1000 + (clock::mono_ms() % 2000);
                self.after(Duration::from_millis(ms), Timer::Glare { call: id });
                return;
            }
            408 | 481 => {
                self.send_bye(id);
                self.end_call(id, EndCause::Error("the far end stopped answering".into()));
                return;
            }
            _ => log::warn!("sip: call {}: re-INVITE refused with {code}", id.0),
        }
        self.on_glare(id);
    }

    /// Sends a hold change that was waiting for a re-INVITE to finish.
    pub fn on_glare(&mut self, id: CallId) {
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
        let sent = self.send_in_dialog(
            id,
            "REFER",
            vec![("Refer-To".into(), refer_to), ("Referred-By".into(), me)],
            None,
            Ctx::Refer { call: id, consult },
        );
        if sent.is_none() {
            return Err(CommandError::Failed("cannot send the REFER".into()));
        }
        self.set_call(id, |call| call.state = CallState::Transferring);
        if let Some(k) = consult {
            self.set_call(k, |call| call.transfer_of = Some(id));
        }
        self.sync_keys();
        Ok(Reply::Done)
    }

    pub(crate) fn on_refer_final(&mut self, id: CallId, _consult: Option<CallId>, code: u16, reason: &str) {
        if code >= 300 {
            log::info!("sip: call {}: REFER refused: {code} {reason}", id.0);
            self.transfer_failed(id);
        }
        // 2xx: the NOTIFYs carry the outcome.
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
                let k = self.established(consult)?;
                let d = &k.dialog;
                let replaces = util::percent_encode(&format!(
                    "{};to-tag={};from-tag={}",
                    d.call_id, d.remote_tag, d.local_tag
                ));
                let base = NameAddr::parse(&d.remote).uri;
                let base = base.split('?').next().unwrap_or(&base).to_string();
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
                    let body = format!("Signal={digit}\r\nDuration=100\r\n").into_bytes();
                    self.send_in_dialog(call, "INFO", Vec::new(), Some(("application/dtmf-relay", body)), Ctx::Other("INFO"));
                }
                Ok(Reply::Done)
            }
            Command::Pickup { blf_entry } => {
                let b = self.cfg.blf.iter().find(|b| b.id == blf_entry).cloned().ok_or_else(|| invalid("no such BLF entry"))?;
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
        let c = self.call_mut(id).ok_or_else(|| invalid("no such call"))?;
        if c.ended {
            return Ok(Reply::Done);
        }
        if c.answered {
            self.send_bye(id);
            self.end_call(id, EndCause::LocalHangup);
        } else if c.outgoing {
            match (c.invite, c.provisional_seen) {
                (Some(inv), true) => self.send_cancel(inv),
                _ => c.cancel_wanted = true,
            }
            self.end_call(id, EndCause::LocalHangup);
        } else {
            return self.reject(id, 486);
        }
        Ok(Reply::Done)
    }

    /// PUBLISH of our own presence on every registered account.
    fn publish_presence(&mut self, status: dphone_state::PresenceStatus, text: &str) {
        for i in 0..self.accts.len() {
            let a = &self.accts[i];
            if !a.registered {
                continue;
            }
            let h = a.cfg.host(a.alt);
            let uri = format!("sip:{}@{}", a.cfg.user, h.server);
            let spec = Spec {
                method: "PUBLISH".into(),
                ruri: uri.clone(),
                from: format!("<{uri}>;tag={}", util::token(10)),
                to: format!("<{uri}>"),
                call_id: format!("{}@dphone", util::token(20)),
                cseq: 1,
                routes: Vec::new(),
                headers: vec![("Event".into(), "presence".into()), ("Expires".into(), "3600".into())],
                content_type: "application/pidf+xml".into(),
                body: bodies::build_pidf(&uri, status, text).into_bytes(),
                transport: h.transport,
            };
            let cred = Some(a.cfg.credentials(a.alt));
            if let Ok((dest, _)) = self.account_dest(a) {
                let _ = self.send(spec, dest, Ctx::Other("PUBLISH"), cred, Owner::None);
            }
        }
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
                    self.send_bye(call);
                    self.end_call(call, EndCause::Error("no audio from the far end".into()));
                }
            }
        }
    }
}
