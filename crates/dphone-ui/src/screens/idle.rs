//! The idle (home) screen: clock, status, and line-key tabs.

use super::*;
use dphone_state::{AccountId, BlfState, LineKeyBinding, Registration};

pub(crate) struct Idle;

/// One line-key tab as drawn.
pub(crate) struct Tab {
    pub label: String,
    pub icon: &'static str,
    pub active: bool,
    pub dim: bool,
}

/// The line-key tabs, from the key state, or from the accounts while the
/// call core has not published keys yet.
pub(crate) fn line_tabs(cx: &Cx) -> Vec<Tab> {
    let st = cx.st;
    let account_tab = |id: AccountId, label: &str, active: bool| {
        let acct = st.accounts.get(id);
        let reg = acct.map(|a| &a.registration);
        let label = if !label.is_empty() {
            label.to_string()
        } else {
            acct.map(|a| if a.label.is_empty() { a.user.clone() } else { a.label.clone() }).unwrap_or_default()
        };
        Tab {
            label,
            icon: match reg {
                _ if active => "phone",
                Some(Registration::Registered { .. }) => "registered",
                Some(Registration::Failed { .. }) => "warning",
                _ => "unregistered",
            },
            active,
            dim: !matches!(reg, Some(Registration::Registered { .. })),
        }
    };
    if st.keys.line_keys.is_empty() {
        return st.accounts.accounts.iter().map(|a| account_tab(a.id, "", false)).collect();
    }
    st.keys
        .line_keys
        .iter()
        .take(cx.model.line_keys() as usize)
        .map(|k| match &k.binding {
            LineKeyBinding::Account(id) => account_tab(*id, &k.label, !k.calls.is_empty()),
            LineKeyBinding::Blf(entry) => {
                let e = st.presence.get(entry);
                let state = e.map(|e| e.state).unwrap_or_default();
                Tab {
                    label: if k.label.is_empty() { e.map(|e| e.label.clone()).unwrap_or_default() } else { k.label.clone() },
                    icon: match state {
                        BlfState::Ringing => "ring",
                        BlfState::InUse => "phone",
                        BlfState::OnHold => "hold",
                        BlfState::Idle => "registered",
                        _ => "unregistered",
                    },
                    active: matches!(state, BlfState::Ringing | BlfState::InUse | BlfState::OnHold),
                    dim: matches!(state, BlfState::Unknown | BlfState::Unavailable),
                }
            }
            LineKeyBinding::Unused => Tab { label: String::new(), icon: "", active: false, dim: true },
        })
        .collect()
}

/// The one-line status message under the clock.
pub(crate) fn status_line(cx: &Cx) -> (String, bool) {
    let st = cx.st;
    if st.accounts.accounts.is_empty() {
        return (tr::NO_ACCOUNTS.into(), true);
    }
    let default = st.accounts.default_account.and_then(|id| st.accounts.get(id)).or(st.accounts.accounts.first());
    if let Some(a) = default {
        match &a.registration {
            Registration::Registered { .. } => {}
            Registration::Registering => return (tr::REGISTERING.into(), false),
            Registration::Failed { code, reason, .. } => {
                return (format!("{}: {} {}", tr::NOT_REGISTERED, code, reason), true);
            }
            Registration::Disabled => return (tr::DISABLED.into(), true),
            Registration::Unregistered => return (tr::NOT_REGISTERED.into(), true),
        }
    }
    if st.features.dnd {
        return (tr::DND_ON.into(), true);
    }
    if !st.features.forward.always.is_empty() {
        return (format!("{} {}", tr::FORWARD_ON, st.features.forward.always), true);
    }
    if st.call_log.missed_unseen > 0 {
        return (format!("{} {}", st.call_log.missed_unseen, tr::MISSED_CALLS), false);
    }
    let name = default
        .map(|a| if a.display_name.is_empty() { a.label.clone() } else { a.display_name.clone() })
        .unwrap_or_default();
    (name, false)
}

/// Status icons, right to left: (icon, count).
pub(crate) fn status_icons(cx: &Cx) -> Vec<(&'static str, u32)> {
    let st = cx.st;
    let mut v = Vec::new();
    if !st.network.is_up() {
        v.push(("net-down", 0));
    }
    if st.features.dnd {
        v.push(("dnd", 0));
    }
    if st.features.forward.any() {
        v.push(("forward", 0));
    }
    if st.audio.headset_mode {
        v.push(("headset", 0));
    }
    if st.audio.muted {
        v.push(("mute", 0));
    }
    if st.mwi.any_waiting() {
        v.push(("voicemail", st.mwi.total_new()));
    }
    if st.call_log.missed_unseen > 0 {
        v.push(("phone-missed", st.call_log.missed_unseen));
    }
    if matches!(st.update.trial, dphone_state::TrialBoot::Pending { .. }) {
        v.push(("update", 0));
    }
    v
}

/// The status bar shared by the idle and call screens.
pub(crate) fn status_bar(cx: &Cx, c: &mut Canvas, r: IRect, show_clock: bool) {
    let th = cx.th;
    c.fill(r, th.title_bg);
    let fg = th.title_fg;
    let f = th.font(Font::Normal);
    let mut x = r.right() - th.pad;
    for (name, count) in status_icons(cx) {
        if count > 0 {
            let s = count.to_string();
            let w = f.width(&s);
            x -= w;
            c.text_in(f, IRect::new(x, r.y, w, r.h), &s, fg, HAlign::Left);
            x -= 2;
        }
        if let Some(icon) = th.icon(name) {
            x -= icon.width();
            c.icon(icon, x, r.y + (r.h - icon.height()) / 2, fg);
            x -= th.pad;
        }
    }
    let left = IRect::new(r.x + th.pad, r.y, (x - r.x - th.pad).max(0), r.h);
    if show_clock {
        let civil = cx.civil();
        let m = civil.meridiem(cx.cfg.time_24h);
        let text = if cx.st.system.time_synced || cx.unix > 1_600_000_000 {
            format!("{}{}{}  {}", civil.time(cx.cfg.time_24h), if m.is_empty() { "" } else { " " }, m, civil.date())
        } else {
            tr::PRODUCT.to_string()
        };
        c.text_in(f, left, &text, fg, HAlign::Left);
    } else if let Some(icon) = th.icon("wordmark") {
        let old = c.push_clip(left);
        c.icon(icon, left.x, r.y + (r.h - icon.height()) / 2, fg);
        c.restore_clip(old);
    }
}

/// Width of the line-key tab column.
pub(crate) fn tab_width(th: &Theme) -> i32 {
    match th.skin {
        dphone_state::Skin::Color => 156,
        dphone_state::Skin::LargeGray => 104,
        dphone_state::Skin::SmallGray => 80,
    }
}

fn draw_tabs(cx: &Cx, c: &mut Canvas, r: IRect) {
    let th = cx.th;
    let tabs = line_tabs(cx);
    let n = (cx.model.line_keys() as usize).max(tabs.len()).max(1) as i32;
    let max_h = if th.gray { 22 } else { 46 };
    let h = (r.h / n).min(max_h);
    let f = th.font(Font::Normal);
    for (i, tab) in tabs.iter().enumerate() {
        let tr_ = IRect::new(r.x, r.y + i as i32 * h, r.w, h);
        let cell = tr_.inset_xy(0, 1);
        let (bg, fg) = if tab.active {
            (th.sel_bg, th.sel_fg)
        } else if th.gray {
            (th.bg, th.fg)
        } else {
            (th.card, if tab.dim { th.secondary } else { th.fg })
        };
        if th.gray {
            c.fill(cell, bg);
            c.frame(cell, th.fg);
        } else {
            c.fill_round(cell, 6, bg);
        }
        let inner = cell.inset_xy(th.pad.min(5), 0);
        let (ic, text) = inner.split_left(th.icon + 3);
        if let Some(icon) = th.icon(tab.icon) {
            let tint = if tab.active || th.gray {
                fg
            } else if tab.icon == "registered" {
                th.good
            } else if tab.icon == "warning" {
                th.warn
            } else {
                th.secondary
            };
            c.icon_in(icon, IRect { w: th.icon, ..ic }, tint);
        }
        c.text_in(f, text, &tab.label, fg, HAlign::Left);
    }
}

impl Screen for Idle {
    fn name(&self) -> String {
        "idle".into()
    }
    fn softkeys(&self, cx: &Cx) -> Vec<Soft> {
        vec![
            soft(1, tr::CALL_LOG),
            soft(2, tr::CONTACTS),
            soft(3, tr::MENU),
            soft(4, tr::APPS),
            soft(5, if cx.st.features.dnd { "DND Off" } else { tr::DND }),
            soft(6, tr::FORWARD),
            soft(7, tr::REDIAL),
            soft(8, tr::STATUS),
        ]
    }
    fn softkey(&mut self, cx: &Cx, id: u32) -> Nav {
        match id {
            1 => Nav::Push(menus::call_log()),
            2 => {
                cx.post(Command::LaunchApp { name: "contacts".into() });
                Nav::Stay
            }
            3 => Nav::Push(menus::main_menu()),
            4 => Nav::Push(menus::launcher()),
            5 => {
                cx.post(Command::SetDnd { on: !cx.st.features.dnd });
                Nav::Stay
            }
            6 => Nav::Push(menus::forward_menu()),
            7 => {
                cx.post(Command::Redial);
                Nav::Stay
            }
            8 => Nav::Push(menus::about()),
            _ => Nav::Stay,
        }
    }
    fn key(&mut self, cx: &Cx, key: Key) -> Nav {
        match key {
            Key::Digit(_) | Key::Star | Key::Pound => {
                let mut d = calls::Dialer::new(calls::Purpose::Call, None);
                d.type_char(cx, key.dial_char().unwrap_or('0'));
                Nav::Push(Box::new(d))
            }
            Key::Select => Nav::Push(menus::main_menu()),
            Key::Up | Key::Down => Nav::Push(menus::call_log()),
            _ => Nav::Ignored,
        }
    }
    fn draw(&mut self, cx: &Cx, c: &mut Canvas, area: IRect) {
        let th = cx.th;
        c.fill(area, th.bg);
        let (bar, body) = area.split_top(th.status_h);
        status_bar(cx, c, bar, false);
        let (main, tabs) = body.split_right(tab_width(th));
        // An app's idle view takes the bottom of the main area.
        let main = match cx.idle_view {
            Some(view) => {
                let (rest, app) = main.split_bottom(2 * th.line_h);
                let mut tree = view.batch_quiet();
                crate::appview_impl::render(&mut tree, c, th, app, cx.now_ms);
                rest
            }
            None => main,
        };
        draw_tabs(cx, c, tabs.inset_xy(if th.gray { 1 } else { 4 }, if th.gray { 0 } else { 3 }));

        // Clock, date, and status line, centered in the main area.
        let civil = cx.civil();
        let big = th.font(Font::Large);
        let norm = th.font(Font::Normal);
        let clock_set = cx.st.system.time_synced || cx.unix > 1_600_000_000;
        let time = if clock_set { civil.time(cx.cfg.time_24h) } else { "--:--".to_string() };
        let mer = if clock_set { civil.meridiem(cx.cfg.time_24h) } else { "" };
        let compact = main.h < big.height() + 2 * th.line_h;
        let total_h = big.height() + if compact { th.line_h } else { 2 * th.line_h };
        let mut y = main.y + ((main.h - total_h) / 2).max(0);
        let tw = big.width(&time);
        let mw = if mer.is_empty() { 0 } else { norm.width(mer) + th.pad };
        let x = main.x + (main.w - tw - mw) / 2;
        let base = y + big.ascent as i32;
        c.text(big, x, base, &time, th.fg);
        if !mer.is_empty() {
            c.text(norm, x + tw + th.pad, base, mer, th.secondary);
        }
        y += big.height();
        if clock_set {
            c.text_in(norm, IRect::new(main.x, y, main.w, th.line_h), &civil.date(), th.secondary, HAlign::Center);
        }
        y += th.line_h;
        if compact {
            return;
        }
        let (text, alert) = status_line(cx);
        let color = if alert && !th.gray { th.warn } else { th.fg };
        c.text_in(
            th.font(if alert { Font::Bold } else { Font::Normal }),
            IRect::new(main.x + th.pad, y, main.w - 2 * th.pad, th.line_h),
            &text,
            color,
            HAlign::Center,
        );
    }
    fn deadline(&self, cx: &Cx) -> Option<u64> {
        // Repaint at the next minute boundary.
        let into = cx.unix % 60;
        Some(cx.now_ms + (60 - into) * 1000 + 50)
    }
}
