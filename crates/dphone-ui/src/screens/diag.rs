//! Diagnostics: display, key, lamp, and tone tests; subsystem health; log.

use super::*;
use dphone_state::{HealthStatus, Subsystem, Tone};

pub(crate) fn menu() -> Box<dyn Screen> {
    Menu::boxed("diagnostics", tr::DIAGNOSTICS, |_| {
        vec![
            Item::new(tr::TEST_DISPLAY, |_| Nav::Push(Box::new(DisplayTest { step: 0 }))),
            Item::new(tr::TEST_KEYS, |_| Nav::Push(Box::new(KeyTest { keys: Vec::new(), cancels: 0 }))),
            Item::new(tr::TEST_LAMPS, |_| Nav::Push(Box::new(LampTest))),
            Item::new(tr::TEST_TONE, |cx| {
                cx.post(Command::PlayTone { tone: Tone::Dial, duration_ms: 2000 });
                Nav::Stay
            }),
            Item::new(tr::HEALTH, |_| Nav::Push(health())),
            Item::new(tr::LOGS, |_| Nav::Push(log())),
        ]
    })
}

pub(crate) fn health() -> Box<dyn Screen> {
    Info::boxed("health", tr::HEALTH, |cx| {
        Subsystem::ALL
            .iter()
            .map(|s| {
                let status = match cx.st.health.status(*s) {
                    HealthStatus::Stopped => "stopped".to_string(),
                    HealthStatus::Starting => "starting".to_string(),
                    HealthStatus::Ok => "ok".to_string(),
                    HealthStatus::Degraded(e) => format!("degraded: {e}"),
                    HealthStatus::Failed(e) => format!("failed: {e}"),
                };
                (s.label().to_string(), status)
            })
            .collect()
    })
}

fn log() -> Box<dyn Screen> {
    Info::boxed("log", tr::LOGS, |cx| {
        cx.store.log().tail(40).into_iter().rev().map(|e| (String::new(), format!("{} {}", e.target, e.message))).collect()
    })
}

/// Full-screen patterns; any key advances, Cancel exits.
struct DisplayTest {
    step: u8,
}

impl Screen for DisplayTest {
    fn name(&self) -> String {
        "display_test".into()
    }
    fn full_screen(&self) -> bool {
        true
    }
    fn key(&mut self, _cx: &Cx, key: Key) -> Nav {
        if key == Key::Cancel || self.step >= 5 {
            return Nav::Pop;
        }
        self.step += 1;
        Nav::Stay
    }
    fn draw(&mut self, cx: &Cx, c: &mut Canvas, area: IRect) {
        use crate::gfx::{rgb, GRAY};
        let th = cx.th;
        match self.step {
            0 => c.fill(area, GRAY[3]),
            1 => c.fill(area, GRAY[0]),
            2 => {
                // Gray ramp, or color bars.
                let bars: &[u32] = if th.gray {
                    &GRAY
                } else {
                    &[rgb(255, 0, 0), rgb(0, 255, 0), rgb(0, 0, 255), rgb(255, 255, 0), rgb(0, 255, 255), rgb(255, 0, 255), GRAY[3], GRAY[0]]
                };
                let w = area.w / bars.len() as i32;
                for (i, b) in bars.iter().enumerate() {
                    c.fill(IRect::new(area.x + i as i32 * w, area.y, w + 1, area.h), *b);
                }
            }
            3 => {
                // Checkerboard of single pixels: shows stuck columns.
                c.fill(area, GRAY[3]);
                for y in (area.y..area.bottom()).step_by(2) {
                    for x in (area.x..area.right()).step_by(2) {
                        c.fill(IRect::new(x, y, 1, 1), GRAY[0]);
                    }
                }
            }
            4 => {
                c.fill(area, GRAY[3]);
                c.frame(area, GRAY[0]);
                c.frame(area.inset(4), GRAY[1]);
                c.hline(area.x, area.y + area.h / 2, area.w, GRAY[0]);
                c.vline(area.x + area.w / 2, area.y, area.h, GRAY[0]);
            }
            _ => {
                c.fill(area, th.bg);
                let mut y = area.y + 2;
                for f in [Font::Small, Font::Normal, Font::Bold, Font::Title, Font::Large] {
                    let font = th.font(f);
                    c.text(font, area.x + 4, y + font.ascent as i32, "Open DPhone 0123456789 AaBbGgQq", th.fg);
                    y += font.height() + 2;
                }
            }
        }
    }
}

/// Shows key names as they are pressed; two Cancels in a row exit.
struct KeyTest {
    keys: Vec<String>,
    cancels: u8,
}

impl Screen for KeyTest {
    fn name(&self) -> String {
        "key_test".into()
    }
    fn captures_all_keys(&self) -> bool {
        true
    }
    fn key(&mut self, _cx: &Cx, key: Key) -> Nav {
        self.cancels = if key == Key::Cancel { self.cancels + 1 } else { 0 };
        if self.cancels >= 2 {
            return Nav::Pop;
        }
        self.keys.push(key.name());
        if self.keys.len() > 24 {
            self.keys.remove(0);
        }
        Nav::Stay
    }
    fn hook(&mut self, _cx: &Cx, off_hook: bool) -> Nav {
        self.keys.push(if off_hook { "hook-up".into() } else { "hook-down".into() });
        Nav::Stay
    }
    fn draw(&mut self, cx: &Cx, c: &mut Canvas, area: IRect) {
        let th = cx.th;
        c.fill(area, th.bg);
        let (title, body) = area.split_top(th.title_h);
        widgets::title_bar(c, th, title, tr::TEST_KEYS);
        let f = th.font(Font::Normal);
        let (hint, rest) = body.split_top(th.line_h);
        c.text_in(th.font(Font::Small), hint.inset_xy(th.pad, 0), "Cancel twice to exit", th.secondary, HAlign::Left);
        c.text_wrapped(f, rest.inset_xy(th.pad, 0), &self.keys.join(" "), th.fg, HAlign::Left, th.line_h);
    }
}

/// Cycles every lamp through its colors while shown.
struct LampTest;

impl Screen for LampTest {
    fn name(&self) -> String {
        "lamp_test".into()
    }
    fn softkeys(&self, _cx: &Cx) -> Vec<Soft> {
        vec![soft(0, ""), soft(0, ""), soft(0, ""), soft(4, tr::BACK)]
    }
    fn softkey(&mut self, _cx: &Cx, _id: u32) -> Nav {
        Nav::Pop
    }
    fn key(&mut self, _cx: &Cx, key: Key) -> Nav {
        if matches!(key, Key::Cancel | Key::Select) { Nav::Pop } else { Nav::Ignored }
    }
    fn lamp_test(&self) -> bool {
        true
    }
    fn draw(&mut self, cx: &Cx, c: &mut Canvas, area: IRect) {
        splash(c, cx.th, area, false, tr::TEST_LAMPS, tr::LAMPS_CYCLING, None);
    }
    fn deadline(&self, cx: &Cx) -> Option<u64> {
        Some(cx.now_ms + 500)
    }
}
