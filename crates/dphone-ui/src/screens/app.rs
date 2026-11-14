//! An app's view as a screen on the navigation stack.

use super::*;
use crate::appview::{AppEvent, AppView};
use crate::appview_impl::{self, Dispatch};

pub(crate) struct AppScreen {
    pub view: AppView,
    name: String,
    foreground: bool,
}

impl AppScreen {
    pub fn new(view: AppView) -> AppScreen {
        let name = view.name().to_string();
        AppScreen { view, name, foreground: false }
    }

    /// Tells the app whether it is the top screen (once per change).
    pub fn set_foreground(&mut self, on: bool) {
        if self.foreground != on {
            self.foreground = on;
            self.view.push_event(if on { AppEvent::Foreground } else { AppEvent::Background });
        }
    }
}

impl Screen for AppScreen {
    fn name(&self) -> String {
        format!("app:{}", self.name)
    }
    fn tag(&self) -> Tag {
        Tag::App
    }
    fn app_name(&self) -> Option<&str> {
        Some(&self.name)
    }
    fn raw_softkeys(&self, _cx: &Cx) -> Option<[String; 4]> {
        Some(appview_impl::softkey_labels(&self.view.batch_quiet()))
    }
    fn key(&mut self, cx: &Cx, key: Key) -> Nav {
        match appview_impl::dispatch(&self.view, key, cx.now_ms) {
            Dispatch::Consumed => Nav::Stay,
            Dispatch::Ignored => Nav::Ignored,
            // Stock: Line 1 backgrounds a nonprivileged app.
            Dispatch::Background => Nav::Home,
        }
    }
    fn draw(&mut self, cx: &Cx, c: &mut Canvas, area: IRect) {
        self.view.take_dirty();
        let mut tree = self.view.batch_quiet();
        appview_impl::render(&mut tree, c, cx.th, area, cx.now_ms);
    }
    fn deadline(&self, cx: &Cx) -> Option<u64> {
        self.view.batch_quiet().multitap.deadline_ms().filter(|d| *d > cx.now_ms)
    }
    fn dirty(&self) -> bool {
        self.view.is_dirty()
    }
    fn leave(&mut self, _cx: &Cx) {
        self.set_foreground(false);
    }
    fn enter(&mut self, _cx: &Cx) {
        self.set_foreground(true);
    }
}
