//! Drawing and editing helpers shared by the native screens and the app
//! view renderer.

use crate::appview::{Font, InputMode};
use crate::gfx::{Canvas, HAlign, IRect, Rgb};
use crate::multitap::{MultiTap, Tap};
use crate::skin::Theme;
use dphone_state::Key;

/// Selection and scroll position of a list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ListState {
    pub selected: usize,
    pub top: usize,
}

impl ListState {
    /// Moves the selection by `delta` among `len` rows, wrapping if asked.
    /// Returns whether it moved.
    pub fn step(&mut self, delta: i32, len: usize, wrap: bool) -> bool {
        if len == 0 {
            self.selected = 0;
            return false;
        }
        let old = self.selected.min(len - 1);
        let new = old as i32 + delta;
        let new = if wrap { new.rem_euclid(len as i32) } else { new.clamp(0, len as i32 - 1) } as usize;
        self.selected = new;
        new != old
    }

    /// Clamps to `len` rows and scrolls so the selection is within the
    /// `visible` rows.
    pub fn settle(&mut self, len: usize, visible: usize) {
        let visible = visible.max(1);
        self.selected = self.selected.min(len.saturating_sub(1));
        if self.selected < self.top {
            self.top = self.selected;
        }
        if self.selected >= self.top + visible {
            self.top = self.selected + 1 - visible;
        }
        self.top = self.top.min(len.saturating_sub(visible));
    }
}

/// A vertical scroll bar on the right edge of `r`.
pub fn scroll_bar(c: &mut Canvas, th: &Theme, r: IRect, total: i32, visible: i32, pos: i32) {
    if total <= visible || r.h <= 0 {
        return;
    }
    let track = IRect::new(r.right() - th.scroll_w, r.y, th.scroll_w, r.h);
    if !th.gray {
        c.fill(track, th.border);
    } else {
        c.vline(track.x, track.y, track.h, th.border);
    }
    let h = (r.h * visible / total).max(6);
    let y = r.y + (r.h - h) * pos / (total - visible).max(1);
    let thumb = IRect::new(track.x + 1, y, th.scroll_w - 1, h);
    c.fill(thumb, if th.gray { th.fg } else { th.secondary });
}

/// Draws rows of a list.  `draw_row(canvas, rect, index, selected, fg)`
/// paints the content of one row (its background is already painted).
/// Returns the number of visible rows.
pub fn list(
    c: &mut Canvas,
    th: &Theme,
    r: IRect,
    len: usize,
    state: &mut ListState,
    row_h: i32,
    focused: bool,
    mut draw_row: impl FnMut(&mut Canvas, IRect, usize, bool, Rgb),
) -> usize {
    let row_h = row_h.max(1);
    let visible = (r.h / row_h).max(1) as usize;
    state.settle(len, visible);
    let bar = len > visible;
    let w = if bar { r.w - th.scroll_w } else { r.w };
    let old = c.push_clip(r);
    for i in 0..visible {
        let idx = state.top + i;
        if idx >= len {
            break;
        }
        let rr = IRect::new(r.x, r.y + i as i32 * row_h, w, row_h);
        let sel = idx == state.selected && focused;
        if sel {
            if th.gray {
                c.fill(rr, th.sel_bg);
            } else {
                c.fill_round(rr.inset_xy(2, 1), 4, th.sel_bg);
            }
        }
        draw_row(c, rr, idx, sel, if sel { th.sel_fg } else { th.fg });
    }
    if bar {
        scroll_bar(c, th, r, len as i32, visible as i32, state.top as i32);
    }
    c.restore_clip(old);
    visible
}

/// A title bar across `r`.
pub fn title_bar(c: &mut Canvas, th: &Theme, r: IRect, title: &str) {
    c.fill(r, th.title_bg);
    let f = th.font(Font::Title);
    c.text_in(f, r.inset_xy(th.pad, 0), title, th.title_fg, if th.gray { HAlign::Center } else { HAlign::Left });
    if !th.gray {
        c.hline(r.x, r.bottom() - 1, r.w, th.border);
    }
}

/// The softkey bar: four cells; empty labels draw nothing.  A label
/// starting with `icon:` draws that icon instead.
pub fn softkey_bar(c: &mut Canvas, th: &Theme, r: IRect, labels: &[String; 4]) {
    c.fill(r, th.sk_bg);
    if !th.gray {
        c.hline(r.x, r.y, r.w, th.border);
    }
    let f = th.font(if th.gray { Font::Normal } else { Font::Bold });
    let cell_w = r.w / 4;
    for (i, label) in labels.iter().enumerate() {
        let cell = IRect::new(r.x + i as i32 * cell_w, r.y, cell_w, r.h);
        if i > 0 && th.gray {
            c.vline(cell.x, cell.y + 2, cell.h - 4, crate::gfx::GRAY[2]);
        }
        if label.is_empty() {
            continue;
        }
        if !th.gray {
            c.fill_round(cell.inset_xy(3, 3), 5, th.border);
        }
        if let Some(icon) = label.strip_prefix("icon:").and_then(|n| th.icon(n)) {
            c.icon_in(icon, cell, th.sk_fg);
        } else {
            c.text_in(f, cell.inset_xy(4, 0), label, th.sk_fg, HAlign::Center);
        }
    }
}

/// A horizontal progress bar.
pub fn progress(c: &mut Canvas, th: &Theme, r: IRect, percent: u32) {
    c.frame(r, if th.gray { th.fg } else { th.border });
    let inner = r.inset(2);
    let w = inner.w * percent.min(100) as i32 / 100;
    c.fill(IRect::new(inner.x, inner.y, w, inner.h), if th.gray { th.fg } else { th.sel_bg });
}

/// A level meter of `max` segments with `level` lit (volume popups).
pub fn meter(c: &mut Canvas, th: &Theme, r: IRect, level: u32, max: u32) {
    let max = max.max(1) as i32;
    let gap = if th.gray { 1 } else { 3 };
    let seg = (r.w - gap * (max - 1)) / max;
    for i in 0..max {
        let x = r.x + i * (seg + gap);
        let cell = IRect::new(x, r.y, seg, r.h);
        if (i as u32) < level {
            c.fill(cell, if th.gray { th.fg } else { th.sel_bg });
        } else {
            c.frame(cell, if th.gray { th.fg } else { th.border });
        }
    }
}

/// A centered modal box of the given size; returns its inner area.
pub fn modal(c: &mut Canvas, th: &Theme, area: IRect, w: i32, h: i32) -> IRect {
    let w = w.min(area.w - 4);
    let h = h.min(area.h - 2);
    let r = IRect::new(area.x + (area.w - w) / 2, area.y + (area.h - h) / 2, w, h);
    if th.gray {
        c.fill(r, th.bg);
        c.frame(r, th.fg);
        c.frame(r.inset(1), th.fg);
    } else {
        c.fill_round(r.inset(-2), 10, th.border);
        c.fill_round(r, 8, th.modal_bg);
    }
    r.inset(th.pad)
}

// ------------------------------------------------------------ text entry

/// Result of feeding a key to a text field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Not an editing key.
    Ignored,
    /// Consumed without changing the text (cursor moved, limit reached).
    Consumed,
    /// The text changed.
    Changed,
}

fn byte_at(text: &str, cursor: usize) -> usize {
    text.char_indices().nth(cursor).map(|(i, _)| i).unwrap_or(text.len())
}

/// Deletes the character before the cursor.
pub fn edit_backspace(text: &mut String, cursor: &mut u16, tap: &mut MultiTap) -> Edit {
    tap.reset();
    let cur = (*cursor as usize).min(text.chars().count());
    if cur == 0 {
        return Edit::Consumed;
    }
    let a = byte_at(text, cur - 1);
    let b = byte_at(text, cur);
    text.replace_range(a..b, "");
    *cursor = (cur - 1) as u16;
    Edit::Changed
}

/// Applies a key press to a one-line text field.
pub fn edit_key(
    text: &mut String,
    cursor: &mut u16,
    mode: InputMode,
    max_len: usize,
    tap: &mut MultiTap,
    key: Key,
    now_ms: u64,
) -> Edit {
    let len = text.chars().count();
    let cur = (*cursor as usize).min(len);
    match key {
        Key::Left => {
            tap.reset();
            *cursor = cur.saturating_sub(1) as u16;
            Edit::Consumed
        }
        Key::Right => {
            tap.reset();
            *cursor = (cur + 1).min(len) as u16;
            Edit::Consumed
        }
        Key::Digit(_) | Key::Star | Key::Pound => {
            let action = match mode {
                InputMode::Text | InputMode::Password => tap.tap(key, now_ms),
                InputMode::Dial => key.dial_char().map(Tap::Insert),
                InputMode::Numeric | InputMode::Pin => match key {
                    Key::Digit(d) => Some(Tap::Insert((b'0' + d) as char)),
                    Key::Star if mode == InputMode::Numeric => Some(Tap::Insert('.')),
                    _ => None,
                },
            };
            match action {
                None => Edit::Consumed,
                Some(Tap::Insert(ch)) => {
                    if len >= max_len {
                        tap.reset();
                        return Edit::Consumed;
                    }
                    text.insert(byte_at(text, cur), ch);
                    *cursor = (cur + 1) as u16;
                    Edit::Changed
                }
                Some(Tap::Replace(ch)) => {
                    if cur == 0 {
                        return Edit::Consumed;
                    }
                    let a = byte_at(text, cur - 1);
                    let b = byte_at(text, cur);
                    text.replace_range(a..b, ch.encode_utf8(&mut [0; 4]));
                    Edit::Changed
                }
            }
        }
        _ => Edit::Ignored,
    }
}

/// Draws a one-line text field with a cursor.  `pending` marks the
/// character before the cursor as still cycling (multi-tap).
#[allow(clippy::too_many_arguments)]
pub fn text_field(
    c: &mut Canvas,
    th: &Theme,
    r: IRect,
    text: &str,
    cursor: usize,
    mode: InputMode,
    focused: bool,
    pending: bool,
    placeholder: &str,
    font: Font,
) {
    let f = th.font(font);
    if th.gray {
        c.fill(r, th.bg);
        c.frame(r, th.fg);
    } else {
        c.fill_round(r, 5, if focused { th.sel_bg } else { th.border });
        c.fill_round(r.inset(if focused { 2 } else { 1 }), 4, th.title_bg);
    }
    let inner = r.inset_xy(th.pad.max(4), 2);
    if text.is_empty() && !placeholder.is_empty() {
        c.text_in(f, inner, placeholder, th.secondary, HAlign::Left);
    }
    let masked = matches!(mode, InputMode::Password | InputMode::Pin);
    let shown: String = if masked {
        let n = text.chars().count();
        text.chars()
            .enumerate()
            .map(|(i, ch)| if pending && i + 1 == cursor.min(n) { ch } else { '\u{2022}' })
            .collect()
    } else {
        text.to_string()
    };
    let cur_byte = byte_at(&shown, cursor);
    let cur_x = f.width(&shown[..cur_byte]);
    // Scroll so the cursor stays visible.
    let scroll = (cur_x - (inner.w - 2)).max(0);
    let old = c.push_clip(inner);
    let y = inner.y + (inner.h - f.height()) / 2 + f.ascent as i32;
    c.text(f, inner.x - scroll, y, &shown, th.fg);
    if focused {
        let x = inner.x - scroll + cur_x;
        if pending && cursor > 0 {
            // Underline the cycling character.
            let prev = byte_at(&shown, cursor - 1);
            let w = f.width(&shown[prev..cur_byte]);
            c.hline(x - w, y + 2, w, th.fg);
        } else {
            c.vline(x, inner.y + 1, inner.h - 2, if th.gray { th.fg } else { th.accent });
        }
    }
    c.restore_clip(old);
}

/// Draws `label: value` on one row, value right-aligned.
pub fn key_value(c: &mut Canvas, th: &Theme, r: IRect, label: &str, value: &str, fg: Rgb, dim: Rgb) {
    let f = th.font(Font::Normal);
    let vw = f.width(value).min(r.w * 2 / 3);
    let (l, v) = r.split_right(vw);
    c.text_in(f, IRect { w: (l.w - th.pad).max(0), ..l }, label, fg, HAlign::Left);
    c.text_in(f, v, value, dim, HAlign::Right);
}
