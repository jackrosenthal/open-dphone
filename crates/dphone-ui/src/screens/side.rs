//! The D70's side display (113x357): ten rapid-dial/BLF keys with labels
//! and state, plus the page indicator.

use super::*;
use dphone_state::{BlfState, LampColor};

/// Paints the whole side display.
pub(crate) fn draw(cx: &Cx, c: &mut Canvas) {
    let th = cx.th;
    let area = c.bounds();
    c.fill(area, th.bg);
    let keys = &cx.st.keys;
    let n = cx.model.blf_keys().max(1) as i32;
    let footer = th.line_h;
    let row_h = (area.h - footer) / n;
    let f = th.font(Font::Normal);
    let small = th.font(Font::Small);
    for i in 0..n {
        let r = IRect::new(area.x, area.y + i * row_h, area.w, row_h);
        c.hline(r.x, r.bottom() - 1, r.w, th.border);
        let key = keys.rapid_dial.iter().find(|k| k.index as i32 == i && k.page == keys.rapid_page);
        let Some(key) = key else { continue };
        if key.entry.is_empty() && key.label.is_empty() {
            continue;
        }
        let entry = cx.st.presence.get(&key.entry);
        let state = entry.map(|e| e.state).unwrap_or_default();
        let busy = matches!(state, BlfState::InUse | BlfState::Ringing | BlfState::OnHold) || key.lamp.color == LampColor::Red;
        let (bg, fg) = if busy { (th.fg, th.bg) } else { (th.bg, th.fg) };
        let cell = IRect { h: r.h - 1, ..r };
        c.fill(cell, bg);
        let label = if key.label.is_empty() { entry.map(|e| e.label.as_str()).unwrap_or("") } else { &key.label };
        let (top, bottom) = cell.inset_xy(3, 2).split_top((cell.h - 4) / 2);
        c.text_in(f, top, label, fg, HAlign::Left);
        let sub = match state {
            BlfState::Ringing => tr::RINGING.to_string(),
            BlfState::InUse => tr::BUSY.to_string(),
            BlfState::OnHold => tr::ON_HOLD.to_string(),
            _ => {
                let text = entry.map(|e| e.status_text.as_str()).unwrap_or("");
                if text.is_empty() { key.number.clone() } else { text.to_string() }
            }
        };
        c.text_in(small, bottom, &sub, fg, HAlign::Left);
    }
    // Page indicator.
    let r = IRect::new(area.x, area.bottom() - footer, area.w, footer);
    c.fill(r, th.title_bg);
    let pages = keys.rapid_pages.max(1);
    c.text_in(f, r, &format!("{} / {}", keys.rapid_page as u32 + 1, pages), th.title_fg, HAlign::Center);
}
