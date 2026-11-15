//! The idle (home) screen: clock, status, and line-key tabs.

use super::*;
use crate::gfx::Rgb;
use dphone_state::{AccountId, BlfState, LineKeyBinding, Registration};

pub(crate) struct Idle;

/// One line-key tab as drawn.
pub(crate) struct Tab {
    pub label: String,
    pub icon: &'static str,
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
                // Stock: a person while idle, a handset while in a call.
                Some(Registration::Registered { .. }) => "contacts",
                Some(Registration::Failed { .. }) => "warning",
                _ => "unregistered",
            },
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
                        BlfState::Ringing | BlfState::OnHoldRinging => "ring",
                        BlfState::InUse => "phone",
                        BlfState::OnHold => "hold",
                        BlfState::Idle => "registered",
                        _ => "unregistered",
                    },
                }
            }
            LineKeyBinding::Unused => Tab { label: String::new(), icon: "" },
        })
        .collect()
}

/// Status icons, left to right: (icon, count).  Stock shows the missed
/// call count at the top left of the idle screen; the others follow it.
pub(crate) fn status_icons(cx: &Cx) -> Vec<(&'static str, u32)> {
    let st = cx.st;
    let mut v = Vec::new();
    if st.call_log.missed_unseen > 0 {
        v.push(("phone-missed", st.call_log.missed_unseen));
    }
    if st.mwi.any_waiting() {
        v.push(("voicemail", st.mwi.total_new()));
    }
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
    if matches!(st.update.trial, dphone_state::TrialBoot::Pending { .. }) {
        v.push(("update", 0));
    }
    v
}

/// Draws the status icons in the header band, from its left edge.
pub(crate) fn draw_status_icons(cx: &Cx, c: &mut Canvas, fg: Rgb) {
    let th = cx.th;
    let r = th.header_rect();
    let f = th.font(Font::Normal);
    // Stock: the first icon starts at x 8 (color); the count follows 8
    // pixels after it.
    let (start, gap, after) = if th.gray { (3, 2, 5) } else { (8, 8, 12) };
    let mut x = r.x + start;
    let old = c.push_clip(r);
    for (name, count) in status_icons(cx) {
        let Some(icon) = th.icon(name) else { continue };
        c.icon(icon, x, r.y + (r.h - icon.height()) / 2, fg);
        x += icon.width();
        if count > 0 {
            let s = count.to_string();
            x += gap;
            x += c.text(f, x, r.y + (r.h - f.height()) / 2 + f.ascent as i32, &s, fg);
        }
        x += after;
    }
    c.restore_clip(old);
}

/// Draws the line-key tabs down the right edge.  Tab `i` is at
/// [`Theme::tab_rect`]`(i)`: a fixed pitch from the top of the screen, so
/// that each label sits beside its physical key.
pub(crate) fn draw_tabs(cx: &Cx, c: &mut Canvas) {
    let th = cx.th;
    let f = th.font(Font::Normal);
    let edge = if th.gray { th.fg } else { 0x000001 };
    for (i, tab) in line_tabs(cx).iter().enumerate() {
        let r = th.tab_rect(i);
        if r.bottom() > th.soft_y {
            break;
        }
        if tab.label.is_empty() && tab.icon.is_empty() {
            continue;
        }
        // The tab is open to the right edge of the screen and to the tab
        // above; its left and bottom edges are drawn, with the lower-left
        // corner cut (color) or rounded (gray) as on stock.
        let bottom = r.h - 1;
        for j in 0..r.h {
            // Border offset from the tab's left edge on this row, and how
            // many border pixels it has there.
            let (off, run) = if th.gray {
                match bottom - j {
                    0 => (7, r.w),
                    1 => (5, 2),
                    2 => (3, 2),
                    3 => (2, 1),
                    4 | 5 => (1, 1),
                    _ => (0, 1),
                }
            } else {
                match bottom - j {
                    0 => (4, r.w),
                    k @ 1..=3 => (3 - k, 2),
                    _ => (0, 1),
                }
            };
            let y = r.y + j;
            c.span(y, r.x + off, (r.x + off + run).min(r.right()), edge);
            if j != bottom {
                c.span(y, r.x + off + run, r.right(), th.tab_bg);
            }
        }
        let (left, right) = if th.gray { (6, 5) } else { (14, 9) };
        let mut text = IRect::new(r.x + left, r.y, r.w - left - right, r.h - 1);
        if let Some(icon) = th.icon(tab.icon) {
            let ir = IRect::new(r.right() - right - icon.width(), r.y, icon.width(), r.h - 1);
            c.icon_in(icon, ir, th.tab_fg);
            text.w -= icon.width() + 4;
        }
        c.text_in(f, text, &tab.label, th.tab_fg, HAlign::Left);
    }
}

/// The header band on gray skins: black, left of the tab column, with
/// `text` (a call state) or the status icons.
pub(crate) fn gray_header(cx: &Cx, c: &mut Canvas, text: Option<&str>) {
    let th = cx.th;
    let r = th.header_rect();
    c.fill(r, th.title_bg);
    match text {
        Some(t) => c.text_in(
            th.font(Font::Normal),
            IRect { x: r.x + th.title_pad, w: r.w - 2 * th.title_pad, ..r },
            t,
            th.title_fg,
            HAlign::Left,
        ),
        None => draw_status_icons(cx, c, th.title_fg),
    }
}

/// Where the idle clock goes: the time's right edge and baseline, the
/// meridiem's left edge, and the date's center and baseline.  Color values
/// are the stock positions (digits at x 68..222, y 89..152; meridiem from
/// x 242; date centered on x 148 with its baseline at y 196).
pub(crate) struct ClockLayout {
    pub time_right: i32,
    pub time_base: i32,
    pub mer_x: i32,
    pub date_cx: i32,
    pub date_base: i32,
}

pub(crate) fn clock_layout(th: &Theme, area: IRect) -> ClockLayout {
    if !th.gray {
        return ClockLayout { time_right: 228, time_base: 152, mer_x: 240, date_cx: 148, date_base: 196 };
    }
    // Gray: no stock clock to match (stock shows a logo with a small time
    // line under it), so the block is centered in the area left of the
    // tabs.
    let big = th.clock_font();
    let digits = big.glyph('0').map(|g| g.top as i32).unwrap_or(big.ascent as i32);
    let block = digits + 4 + th.line_h;
    let top = area.y + ((area.h - block) / 2).max(0);
    let cx = area.x + area.w / 2;
    ClockLayout {
        time_right: cx, // centered: see `Idle::draw`
        time_base: top + digits,
        mer_x: 0,
        date_cx: cx,
        date_base: top + digits + 4 + (th.line_h + th.font(Font::Normal).ascent as i32) / 2,
    }
}

/// Height of the idle-screen area offered to an app's idle view.
pub(crate) fn idle_app_h(th: &Theme) -> i32 {
    if th.gray { 2 * th.line_h } else { 36 }
}

impl Screen for Idle {
    fn name(&self) -> String {
        "idle".into()
    }
    fn wallpaper(&self) -> bool {
        true
    }
    fn softkeys(&self, cx: &Cx) -> Vec<Soft> {
        // Stock: the first softkey reads "Missed Calls" while there are
        // unseen missed calls.
        let log = if cx.st.call_log.missed_unseen > 0 { tr::MISSED_CALLS_KEY } else { tr::CALL_LOG };
        vec![
            soft(1, log),
            soft(2, tr::CONTACTS),
            soft(8, tr::STATUS),
            soft(3, tr::MENU),
            soft(4, tr::APPS),
            soft(5, if cx.st.features.dnd { tr::DND_OFF } else { tr::DND }),
            soft(6, tr::FORWARD),
            soft(7, tr::REDIAL),
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
        // The background (wallpaper) is already painted.
        if th.gray {
            gray_header(cx, c, None);
        } else {
            draw_status_icons(cx, c, th.fg);
        }
        draw_tabs(cx, c);

        // The clock area: left of the tabs, below the header band.
        let mut main = IRect::new(area.x, th.header_h, th.tab_x - area.x, area.bottom() - th.header_h);
        if let Some(view) = cx.idle_view {
            let (rest, app) = main.split_bottom(idle_app_h(th));
            let mut tree = view.batch_quiet();
            if th.gray {
                crate::appview_impl::render(&mut tree, c, th, app, cx.now_ms);
            } else {
                // Over the wallpaper: the app's widgets without a fill.
                crate::appview_impl::render_over(&mut tree, c, th, app, cx.now_ms);
            }
            main = rest;
        }
        let civil = cx.civil();
        let big = th.clock_font();
        let sub = th.date_font();
        let clock_set = cx.st.system.time_synced || cx.unix > 1_600_000_000;
        let time = if clock_set { civil.time(cx.cfg.time_24h) } else { "--:--".to_string() };
        let mer = if clock_set { civil.meridiem(cx.cfg.time_24h) } else { "" };
        let lay = clock_layout(th, main);
        let tw = big.width(&time);
        if th.gray {
            let mw = if mer.is_empty() { 0 } else { sub.width(mer) + 3 };
            let x = lay.date_cx - (tw + mw) / 2;
            c.text(big, x, lay.time_base, &time, th.fg);
            if !mer.is_empty() {
                c.text(sub, x + tw + 3, lay.time_base, mer, th.fg);
            }
        } else {
            // With a meridiem the time ends where stock's does; without
            // one it is centered over the date.
            let x = if mer.is_empty() { lay.date_cx - tw / 2 } else { lay.time_right - tw };
            c.text(big, x, lay.time_base, &time, th.fg);
            if !mer.is_empty() {
                c.text(th.meridiem_font(), lay.mer_x, lay.time_base, mer, th.fg);
            }
        }
        if clock_set {
            let date = civil.date();
            c.text(sub, lay.date_cx - sub.width(&date) / 2, lay.date_base, &date, th.fg);
        }
    }
    fn deadline(&self, cx: &Cx) -> Option<u64> {
        // Repaint at the next minute boundary.
        let into = cx.unix % 60;
        Some(cx.now_ms + (60 - into) * 1000 + 50)
    }
}
