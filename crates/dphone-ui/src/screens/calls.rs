//! The dialer and the call screen.

use super::*;
use dphone_state::{AccountId, AudioPath, Call, CallId, CallState, EndCause, Tone};

/// Why the dialer is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Purpose {
    Call,
    /// Transfer this call to the number entered.
    Transfer(CallId),
    /// Call a third party to join with this call.
    Conference(CallId),
}

pub(crate) struct Dialer {
    purpose: Purpose,
    pub text: String,
    account: Option<AccountId>,
    /// Opened by going off-hook (handset, speaker, or headset key): dial
    /// tone plays and digits time out into a call.
    pub live: Option<AudioPath>,
    last_key_ms: u64,
    tone: bool,
}

impl Dialer {
    pub fn new(purpose: Purpose, account: Option<AccountId>) -> Dialer {
        Dialer { purpose, text: String::new(), account, live: None, last_key_ms: 0, tone: false }
    }

    /// A dialer opened by lifting the handset or pressing speaker/headset.
    pub fn off_hook(cx: &Cx, path: AudioPath, account: Option<AccountId>) -> Dialer {
        cx.post(Command::SetAudioPath { path });
        cx.post(Command::PlayTone { tone: Tone::Dial, duration_ms: 0 });
        Dialer { purpose: Purpose::Call, text: String::new(), account, live: Some(path), last_key_ms: cx.now_ms, tone: true }
    }

    fn stop_tone(&mut self, cx: &Cx) {
        if self.tone {
            self.tone = false;
            cx.post(Command::StopTone);
        }
    }

    pub fn type_char(&mut self, cx: &Cx, ch: char) {
        self.stop_tone(cx);
        if self.text.len() < 64 {
            self.text.push(ch);
        }
        self.last_key_ms = cx.now_ms;
        if self.live.is_some() {
            cx.post(Command::PlayTone { tone: Tone::Dtmf(ch), duration_ms: 100 });
        }
    }

    fn close_live(&mut self, cx: &Cx) {
        self.stop_tone(cx);
        if self.live.take().is_some() && cx.st.calls.is_idle() {
            cx.post(Command::SetAudioPath { path: AudioPath::None });
        }
    }

    fn dial(&mut self, cx: &Cx, path: Option<AudioPath>) -> Nav {
        let target = self.text.trim().to_string();
        if target.is_empty() {
            if self.purpose == Purpose::Call {
                self.stop_tone(cx);
                self.live = None;
                cx.post(Command::Redial);
                return Nav::Pop;
            }
            return Nav::Stay;
        }
        self.stop_tone(cx);
        let path = path.or(self.live);
        self.live = None;
        match self.purpose {
            Purpose::Transfer(call) => cx.post(Command::TransferBlind { call, target }),
            _ => cx.post(Command::Dial { target, account: self.account, path }),
        }
        Nav::Pop
    }
}

impl Screen for Dialer {
    fn name(&self) -> String {
        "dialer".into()
    }
    fn tag(&self) -> Tag {
        Tag::Dialer
    }
    fn softkeys(&self, _cx: &Cx) -> Vec<Soft> {
        match self.purpose {
            Purpose::Transfer(_) => vec![soft(1, tr::TRANSFER), soft(2, tr::CONSULT), soft(3, tr::DELETE), soft(4, tr::CANCEL)],
            _ => vec![
                soft(1, if self.text.is_empty() { tr::REDIAL } else { tr::DIAL }),
                soft(0, ""),
                soft(3, tr::DELETE),
                soft(4, tr::CANCEL),
            ],
        }
    }
    fn softkey(&mut self, cx: &Cx, id: u32) -> Nav {
        match id {
            1 => self.dial(cx, None),
            2 => {
                // Consultation call: place a normal call; the call screen
                // then offers to complete the transfer.
                let target = self.text.trim().to_string();
                if target.is_empty() {
                    return Nav::Stay;
                }
                cx.post(Command::Dial { target, account: self.account, path: None });
                Nav::Pop
            }
            3 => {
                self.text.pop();
                self.last_key_ms = cx.now_ms;
                Nav::Stay
            }
            4 => {
                self.close_live(cx);
                Nav::Pop
            }
            _ => Nav::Stay,
        }
    }
    fn key(&mut self, cx: &Cx, key: Key) -> Nav {
        match key {
            Key::Digit(_) | Key::Star | Key::Pound => {
                self.type_char(cx, key.dial_char().unwrap_or('0'));
                Nav::Stay
            }
            Key::Select => self.dial(cx, None),
            Key::Left => {
                self.text.pop();
                self.last_key_ms = cx.now_ms;
                Nav::Stay
            }
            Key::Cancel => {
                self.close_live(cx);
                Nav::Pop
            }
            Key::Speaker if self.live != Some(AudioPath::Speaker) => self.dial_or_switch(cx, AudioPath::Speaker),
            Key::Headset if self.live != Some(AudioPath::Headset) => self.dial_or_switch(cx, AudioPath::Headset),
            Key::Speaker | Key::Headset => {
                // The key that opened the dialer closes it again.
                self.close_live(cx);
                Nav::Pop
            }
            Key::Line(n) => {
                let acct = cx.st.keys.line_keys.get(n as usize - 1).and_then(|k| match k.binding {
                    dphone_state::LineKeyBinding::Account(a) => Some(a),
                    _ => None,
                });
                if acct.is_some() {
                    self.account = acct;
                    if !self.text.is_empty() {
                        return self.dial(cx, None);
                    }
                    return Nav::Stay;
                }
                Nav::Ignored
            }
            Key::Redial if self.text.is_empty() => self.dial(cx, None),
            _ => Nav::Ignored,
        }
    }
    fn hook(&mut self, cx: &Cx, off_hook: bool) -> Nav {
        if off_hook {
            if self.text.is_empty() {
                // Lifted with nothing typed: become a live dialer.
                if self.live.is_none() {
                    cx.post(Command::PlayTone { tone: Tone::Dial, duration_ms: 0 });
                    self.tone = true;
                }
                cx.post(Command::SetAudioPath { path: AudioPath::Handset });
                self.live = Some(AudioPath::Handset);
                self.last_key_ms = cx.now_ms;
                Nav::Stay
            } else {
                self.dial(cx, Some(AudioPath::Handset))
            }
        } else if self.live == Some(AudioPath::Handset) {
            self.close_live(cx);
            Nav::Pop
        } else {
            Nav::Stay
        }
    }
    fn draw(&mut self, cx: &Cx, c: &mut Canvas, area: IRect) {
        let th = cx.th;
        c.fill(area, th.bg);
        let (title, body) = area.split_top(th.title_h);
        let label = match self.purpose {
            Purpose::Transfer(_) => tr::TRANSFER_TO,
            Purpose::Conference(_) => tr::CONFERENCE_WITH,
            _ => tr::ENTER_NUMBER,
        };
        widgets::title_bar(c, th, title, label);
        let big = th.font(Font::Large);
        let r = IRect::new(body.x + th.pad, body.y + (body.h - big.height()) / 2 - th.line_h / 2, body.w - 2 * th.pad, big.height());
        // Keep the tail of a long number visible.
        let mut shown: &str = &self.text;
        while big.width(shown) > r.w - 4 && !shown.is_empty() {
            shown = &shown[shown.chars().next().map(|c| c.len_utf8()).unwrap_or(1)..];
        }
        let w = big.width(shown);
        let x = r.x + (r.w - w) / 2;
        c.text(big, x, r.y + big.ascent as i32, shown, th.fg);
        c.vline(x + w + 1, r.y + 2, r.h - 4, if th.gray { th.fg } else { th.accent });
        c.hline(r.x, r.bottom() + 2, r.w, th.border);
        // The account the call will use.
        let acct = self
            .account
            .or(cx.st.accounts.default_account)
            .and_then(|id| cx.st.accounts.get(id))
            .or(cx.st.accounts.accounts.first());
        if let Some(a) = acct {
            let name = if a.label.is_empty() { a.user.clone() } else { a.label.clone() };
            c.text_in(th.font(Font::Normal), IRect::new(r.x, r.bottom() + 4, r.w, th.line_h), &name, th.secondary, HAlign::Center);
        }
    }
    fn deadline(&self, cx: &Cx) -> Option<u64> {
        if self.live.is_some() && !self.text.is_empty() && self.purpose == Purpose::Call {
            Some(self.last_key_ms + cx.cfg.dial_timeout_ms as u64)
        } else {
            None
        }
    }
    fn tick(&mut self, cx: &Cx) -> Nav {
        if self.live.is_some()
            && !self.text.is_empty()
            && cx.now_ms >= self.last_key_ms + cx.cfg.dial_timeout_ms as u64
        {
            return self.dial(cx, None);
        }
        Nav::Stay
    }
    fn leave(&mut self, cx: &Cx) {
        self.stop_tone(cx);
    }
    fn dial_buffer(&self) -> Option<&str> {
        Some(&self.text)
    }
}

impl Dialer {
    fn dial_or_switch(&mut self, cx: &Cx, path: AudioPath) -> Nav {
        if self.text.is_empty() {
            if self.live.is_none() {
                cx.post(Command::PlayTone { tone: Tone::Dial, duration_ms: 0 });
                self.tone = true;
            }
            cx.post(Command::SetAudioPath { path });
            self.live = Some(path);
            self.last_key_ms = cx.now_ms;
            Nav::Stay
        } else {
            self.dial(cx, Some(path))
        }
    }
}

// ------------------------------------------------------------ call screen

/// State text and icon of a call.
pub(crate) fn call_status(call: &Call, now_ms: u64) -> (String, &'static str) {
    let timer = |label: &str| match call.duration_secs(now_ms) {
        Some(s) => format!("{label}  {}", crate::time::duration(s)),
        None => label.to_string(),
    };
    match &call.state {
        CallState::Dialing | CallState::Proceeding => (tr::CALLING.into(), "phone-out"),
        CallState::Ringback { .. } => (tr::RINGING.into(), "phone-out"),
        CallState::Ringing => (if call.paging { tr::PAGING.into() } else { tr::INCOMING_CALL.into() }, "phone-in"),
        CallState::Active if call.in_conference => (timer(tr::IN_CONFERENCE), "conference"),
        CallState::Active if call.paging => (timer(tr::PAGING), "page"),
        CallState::Active => (timer(tr::CONNECTED), "phone"),
        CallState::Held { local: false, remote: true } => (timer(tr::HELD_BY_REMOTE), "hold"),
        CallState::Held { .. } => (timer(tr::ON_HOLD), "hold"),
        CallState::Transferring => (tr::TRANSFERRING.into(), "transfer"),
        CallState::Ended(cause) => (
            match cause {
                EndCause::Busy => tr::BUSY.into(),
                EndCause::NoAnswer => tr::NO_ANSWER.into(),
                EndCause::Rejected => tr::REJECTED.into(),
                EndCause::Failed { code, reason } => format!("{}: {} {}", tr::CALL_FAILED, code, reason),
                EndCause::Error(e) => format!("{}: {}", tr::CALL_FAILED, e),
                _ => tr::CALL_ENDED.into(),
            },
            "phone",
        ),
    }
}

fn party_name(call: &Call) -> String {
    let d = call.remote.display();
    if d.is_empty() { tr::UNKNOWN_CALLER.to_string() } else { d.to_string() }
}

#[derive(Default)]
pub(crate) struct CallScreen {
    /// The call the softkeys act on (follows the focused call until the
    /// user moves it).
    sel: Option<CallId>,
}

impl CallScreen {
    fn selected<'a>(&self, cx: &'a Cx) -> Option<&'a Call> {
        let calls = &cx.st.calls;
        self.sel
            .and_then(|id| calls.get(id))
            .filter(|c| !c.state.is_ended())
            .or_else(|| calls.ringing().next())
            .or_else(|| calls.focused().filter(|c| !c.state.is_ended()))
            .or_else(|| calls.live().next())
    }

    /// Another established call, for join and attended transfer.
    fn other<'a>(&self, cx: &'a Cx, this: CallId) -> Option<&'a Call> {
        cx.st.calls.live().find(|c| c.id != this && c.state.is_established())
    }

    fn step(&mut self, cx: &Cx, d: i32) {
        let live: Vec<CallId> = cx.st.calls.live().map(|c| c.id).collect();
        if live.is_empty() {
            return;
        }
        let cur = self.selected(cx).and_then(|c| live.iter().position(|i| *i == c.id)).unwrap_or(0) as i32;
        self.sel = Some(live[(cur + d).rem_euclid(live.len() as i32) as usize]);
    }

    fn answer_path(cx: &Cx) -> Option<AudioPath> {
        if cx.off_hook { Some(AudioPath::Handset) } else { None }
    }
}

impl Screen for CallScreen {
    fn name(&self) -> String {
        "call".into()
    }
    fn tag(&self) -> Tag {
        Tag::Call
    }
    fn softkeys(&self, cx: &Cx) -> Vec<Soft> {
        let Some(call) = self.selected(cx) else { return Vec::new() };
        let other = self.other(cx, call.id).is_some();
        match &call.state {
            CallState::Ringing => vec![
                soft(1, tr::ANSWER),
                soft(2, tr::IGNORE),
                soft(3, tr::REJECT),
                soft(4, tr::VOICEMAIL),
                soft(5, tr::TRANSFER),
            ],
            CallState::Dialing | CallState::Proceeding | CallState::Ringback { .. } | CallState::Transferring => {
                vec![soft(0, ""), soft(0, ""), soft(0, ""), soft(10, tr::END_CALL)]
            }
            CallState::Active if call.in_conference => {
                vec![soft(6, tr::HOLD), soft(13, tr::SPLIT), soft(0, ""), soft(10, tr::END_CALL)]
            }
            CallState::Active | CallState::Held { .. } => {
                let mut v = vec![
                    soft(if call.state.is_held() { 7 } else { 6 }, if call.state.is_held() { tr::RESUME } else { tr::HOLD }),
                    soft(8, tr::TRANSFER),
                ];
                if other {
                    v.push(soft(12, tr::JOIN));
                    v.push(soft(10, tr::END_CALL));
                    v.push(soft(14, tr::SWAP));
                } else {
                    v.push(soft(9, tr::CONFERENCE));
                    v.push(soft(10, tr::END_CALL));
                }
                v.push(soft(11, tr::PARK));
                v.push(soft(15, tr::NEW_CALL));
                v
            }
            CallState::Ended(_) => Vec::new(),
        }
    }
    fn softkey(&mut self, cx: &Cx, id: u32) -> Nav {
        let Some(call) = self.selected(cx) else { return Nav::Stay };
        let cid = call.id;
        let account = Some(call.account);
        match id {
            1 => cx.post(Command::Answer { call: cid, path: Self::answer_path(cx) }),
            2 => cx.post(Command::Silence { call: cid }),
            3 => cx.post(Command::Reject { call: cid, busy: true }),
            4 => cx.post(Command::SendToVoicemail { call: cid }),
            5 | 8 => {
                // With a second call up, Transfer connects the two.
                if id == 8
                    && let Some(o) = self.other(cx, cid) {
                        cx.post(Command::TransferAttended { call: o.id, consult: cid });
                        return Nav::Stay;
                    }
                return Nav::Push(Box::new(Dialer::new(Purpose::Transfer(cid), account)));
            }
            6 => cx.post(Command::Hold { call: cid }),
            7 => cx.post(Command::Resume { call: cid }),
            9 | 15 => return Nav::Push(Box::new(Dialer::new(Purpose::Conference(cid), account))),
            10 => cx.post(Command::Hangup { call: cid }),
            11 => cx.post(Command::Park { call: cid }),
            12 => {
                if let Some(o) = self.other(cx, cid) {
                    cx.post(Command::Conference { a: cid, b: o.id });
                }
            }
            13 => cx.post(Command::SplitConference { keep: cid }),
            14 => {
                if let Some(o) = self.other(cx, cid) {
                    cx.post(Command::Focus { call: o.id });
                    self.sel = Some(o.id);
                }
            }
            _ => {}
        }
        Nav::Stay
    }
    fn key(&mut self, cx: &Cx, key: Key) -> Nav {
        let Some(call) = self.selected(cx) else { return Nav::Ignored };
        let cid = call.id;
        match key {
            Key::Up => self.step(cx, -1),
            Key::Down => self.step(cx, 1),
            // Stock: on the call screen Select accepts and Cancel rejects.
            Key::Select => match call.state {
                CallState::Ringing => cx.post(Command::Answer { call: cid, path: Self::answer_path(cx) }),
                CallState::Held { local: true, .. } => cx.post(Command::Resume { call: cid }),
                _ => {}
            },
            Key::Cancel => match call.state {
                CallState::Ringing => cx.post(Command::Reject { call: cid, busy: true }),
                CallState::Dialing | CallState::Proceeding | CallState::Ringback { .. } => {
                    cx.post(Command::Hangup { call: cid })
                }
                _ => {}
            },
            Key::Digit(_) | Key::Star | Key::Pound if call.state.has_media() => {
                cx.post(Command::Dtmf { call: cid, digit: key.dial_char().unwrap_or('0') });
            }
            Key::Digit(_) | Key::Star | Key::Pound if call.state.is_held() => {
                let mut d = Dialer::new(Purpose::Conference(cid), Some(call.account));
                d.type_char(cx, key.dial_char().unwrap_or('0'));
                return Nav::Push(Box::new(d));
            }
            Key::Hold => match call.state {
                CallState::Active => cx.post(Command::Hold { call: cid }),
                CallState::Held { .. } => cx.post(Command::Resume { call: cid }),
                _ => {}
            },
            Key::Transfer if call.state.is_established() || call.state == CallState::Ringing => {
                return self.softkey(cx, 8);
            }
            Key::Conference if call.state.is_established() => {
                return self.softkey(cx, if self.other(cx, cid).is_some() { 12 } else { 9 });
            }
            _ => return Nav::Ignored,
        }
        Nav::Stay
    }
    fn draw(&mut self, cx: &Cx, c: &mut Canvas, area: IRect) {
        let th = cx.th;
        c.fill(area, th.bg);
        let (bar, body) = area.split_top(th.status_h);
        idle::status_bar(cx, c, bar, true);
        let Some(sel) = self.selected(cx) else { return };
        let others: Vec<&Call> = cx.st.calls.live().filter(|c| c.id != sel.id).collect();
        let row_h = th.row_h;
        let max_rows = ((body.h - 3 * th.line_h) / row_h).max(0) as usize;
        let shown = others.len().min(max_rows);
        let (card, rows) = body.split_bottom(shown as i32 * row_h);

        // The selected call's card.
        let card = card.inset_xy(if th.gray { 0 } else { 6 }, if th.gray { 0 } else { 4 });
        let (status, icon) = call_status(sel, cx.now_ms);
        let name = party_name(sel);
        if th.gray {
            // Stock layout: an inverted state header, then name and number.
            let (hdr, rest) = card.split_top(th.line_h);
            c.fill(hdr, th.fg);
            c.text_in(th.font(Font::Bold), hdr.inset_xy(th.pad, 0), &status, th.bg, HAlign::Left);
            let (ic, text) = rest.split_left(th.icon + 2 * th.pad + 6);
            if let Some(i) = th.icon(icon) {
                c.icon(i, ic.x + th.pad + 3, ic.y + 3, th.fg);
            }
            let big = th.font(Font::Bold);
            c.text_in(big, IRect::new(text.x, text.y + 1, text.w - th.pad, th.line_h), &name, th.fg, HAlign::Left);
            if sel.remote.number != name {
                c.text_in(
                    th.font(Font::Normal),
                    IRect::new(text.x, text.y + 1 + th.line_h, text.w - th.pad, th.line_h),
                    &sel.remote.number,
                    th.fg,
                    HAlign::Left,
                );
            }
        } else {
            let ringing = sel.state == CallState::Ringing;
            c.fill_round(card, 8, th.card);
            c.fill_round(IRect::new(card.x, card.y, 6, card.h), 3, if ringing { th.good } else if sel.state.is_held() { th.warn } else { th.sel_bg });
            let inner = IRect::new(card.x + 18, card.y + 4, card.w - 28, card.h - 8);
            let (top, rest) = inner.split_top(th.line_h);
            if let Some(i) = th.icon(icon) {
                c.icon(i, top.x, top.y + (top.h - i.height()) / 2, th.accent);
            }
            c.text_in(th.font(Font::Bold), IRect::new(top.x + th.icon + 8, top.y, top.w - th.icon - 8, top.h), &status, th.accent, HAlign::Left);
            let big = th.font(Font::Large);
            let name_font = if big.width(&name) <= rest.w { big } else { th.font(Font::Title) };
            let nh = name_font.height().min(rest.h);
            c.text_in(name_font, IRect::new(rest.x, rest.y, rest.w, nh), &name, th.fg, HAlign::Left);
            if sel.remote.number != name && rest.h >= nh + th.line_h {
                c.text_in(th.font(Font::Normal), IRect::new(rest.x, rest.y + nh, rest.w, th.line_h), &sel.remote.number, th.secondary, HAlign::Left);
            }
        }
        // Flags at the card's top right: muted, secure, codec.
        let mut x = card.right() - th.pad;
        let fy = if th.gray { card.y } else { card.y + 4 };
        let flag_fg = if th.gray { th.bg } else { th.secondary };
        for (on, name) in [(cx.st.audio.muted || sel.muted, "mute"), (sel.srtp, "lock")] {
            if on
                && let Some(i) = th.icon(name) {
                    x -= i.width();
                    c.icon(i, x, fy + (th.line_h - i.height()) / 2, if name == "mute" && !th.gray { th.error } else { flag_fg });
                    x -= th.pad;
                }
        }
        if let (Some(codec), false) = (sel.codec, th.gray) {
            let f = th.font(Font::Small);
            let w = f.width(codec.name());
            c.text_in(f, IRect::new(x - w, fy, w, th.line_h), codec.name(), th.secondary, HAlign::Left);
        }

        // The other calls, one row each.
        for (i, call) in others.iter().take(shown).enumerate() {
            let r = IRect::new(rows.x, rows.y + i as i32 * row_h, rows.w, row_h);
            c.hline(r.x, r.y, r.w, th.border);
            let (status, icon) = call_status(call, cx.now_ms);
            let (ic, rest) = r.inset_xy(th.pad, 0).split_left(th.icon + th.pad);
            if let Some(ico) = th.icon(icon) {
                c.icon_in(ico, IRect { w: th.icon, ..ic }, th.fg);
            }
            widgets::key_value(c, th, rest, &party_name(call), &status, th.fg, th.secondary);
        }
        if others.len() > shown {
            let r = IRect::new(body.right() - 40, body.y, 36, th.line_h);
            c.text_in(th.font(Font::Small), r, &format!("+{}", others.len() - shown), th.secondary, HAlign::Right);
        }
    }
    fn deadline(&self, cx: &Cx) -> Option<u64> {
        cx.st.calls.live().any(|c| c.connected_mono_ms.is_some()).then(|| cx.now_ms + 1000 - cx.now_ms % 1000)
    }
}
