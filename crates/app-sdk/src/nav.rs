//! Small screen helpers shared by the built-in apps: a message screen and
//! a labelled-input form.

use crate::{InputMode, ROOT, Rect, Softkey, Widget, WidgetKind, row_height, screen, ui};

/// A full-screen wrapped message (loading, error, confirmation text).
pub struct Message {
    pub text: Widget,
}

impl Message {
    /// Clears the window and shows `text`, with the given softkeys.
    pub fn show(title: &str, text: &str, keys: &[Softkey]) -> Message {
        let r = screen(title);
        let w = ui::create(
            WidgetKind::Text,
            ROOT,
            Rect::new(
                4,
                r.y + 2,
                r.width.saturating_sub(8),
                r.height.saturating_sub(4),
            ),
        );
        ui::set_wrap(w, true);
        ui::set_text(w, text);
        ui::set_softkeys(ROOT, keys);
        ui::set_focus(w);
        Message { text: w }
    }
}

/// A vertical form of labelled inputs.
pub struct Form {
    pub inputs: Vec<Widget>,
}

impl Form {
    /// Clears the window and builds the form; focus goes to the first input.
    pub fn show(title: &str, fields: &[(&str, InputMode, &str)], keys: &[Softkey]) -> Form {
        let r = screen(title);
        let h = row_height();
        let label_w = r.width / 3;
        let mut inputs = Vec::new();
        for (i, (label, mode, value)) in fields.iter().enumerate() {
            let y = r.y + (i as u16 * (h + 2)) as i16 + 2;
            let l = ui::create(WidgetKind::Text, ROOT, Rect::new(4, y, label_w, h));
            ui::set_text(l, label);
            let inp = ui::create(
                WidgetKind::Input,
                ROOT,
                Rect::new(label_w as i16 + 6, y, r.width - label_w - 10, h),
            );
            ui::set_input_mode(inp, *mode);
            ui::set_value(inp, value);
            inputs.push(inp);
        }
        ui::set_softkeys(ROOT, keys);
        if let Some(first) = inputs.first() {
            ui::set_focus(*first);
        }
        Form { inputs }
    }

    pub fn value(&self, index: usize) -> String {
        self.inputs
            .get(index)
            .map(|w| ui::value(*w))
            .unwrap_or_default()
    }

    /// Moves focus to the next input after `current`; returns false when
    /// `current` was the last.
    pub fn focus_next(&self, current: Widget) -> bool {
        match self.inputs.iter().position(|w| *w == current) {
            Some(i) if i + 1 < self.inputs.len() => {
                ui::set_focus(self.inputs[i + 1]);
                true
            }
            _ => false,
        }
    }
}
