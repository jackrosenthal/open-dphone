//! Diagnostics: the display, key, lamp, and tone tests, the state of the
//! phone's subsystems, and its log.  The three hardware tests are the
//! phone's own screens, which have the whole display and every key; this
//! app is the menu that leads to them, and the two status pages.

use dphone_app_sdk::Align;
use dphone_app_sdk::diagnostics::{self, HardwareTest};
use dphone_app_sdk::prelude::*;

const SK_SELECT: u32 = 1;
const SK_BACK: u32 = 4;
/// A softkey position left empty.
const SK_NONE: u32 = 0;

const ITEMS: [&str; 6] = [
    "Display test",
    "Key test",
    "Lamp test",
    "Tone test",
    "Subsystems",
    "Log",
];
const LOG_ENTRIES: u32 = 40;
/// How often a status page is brought up to date.
const REFRESH_MS: u32 = 1000;

#[derive(Clone, Copy, PartialEq)]
enum Info {
    Health,
    Log,
}

impl Info {
    fn title(self) -> &'static str {
        match self {
            Info::Health => "Subsystems",
            Info::Log => "Log",
        }
    }
    /// The page's rows: a label (may be empty) and a value.
    fn rows(self) -> Vec<(String, String)> {
        match self {
            Info::Health => diagnostics::health(),
            Info::Log => diagnostics::log_tail(LOG_ENTRIES)
                .into_iter()
                .map(|line| (String::new(), line))
                .collect(),
        }
    }
}

#[derive(Default)]
enum Page {
    #[default]
    None,
    Menu {
        list: Widget,
    },
    Info {
        which: Info,
        /// The value widget of each row.
        values: Vec<Widget>,
        rows: Vec<(String, String)>,
    },
}

#[derive(Default)]
struct Diagnostics {
    page: Page,
    /// The menu row to come back to.
    selected: u32,
    timer: Option<u32>,
}

fn blank() -> Softkey {
    Softkey::new(SK_NONE, "")
}

impl Diagnostics {
    fn stop_timer(&mut self) {
        if let Some(t) = self.timer.take() {
            timers::cancel(t);
        }
    }

    fn show_menu(&mut self) {
        self.stop_timer();
        let r = screen("Diagnostics");
        // As in the phone's own menus: Right selects, and a digit selects
        // the row of that number.
        ui::claim_keys(
            ROOT,
            &["cancel", "left", "right", "1", "2", "3", "4", "5", "6"].map(String::from),
        );
        let list = ui::create(WidgetKind::TextList, ROOT, r);
        let items: Vec<ListItem> = ITEMS.iter().map(|i| ListItem::new(*i)).collect();
        ui::set_items(list, &items);
        ui::set_selected(list, self.selected);
        ui::set_softkeys(
            ROOT,
            &[
                Softkey::new(SK_SELECT, "Select"),
                blank(),
                blank(),
                Softkey::new(SK_BACK, "Back"),
            ],
        );
        ui::set_focus(list);
        self.page = Page::Menu { list };
    }

    fn show_info(&mut self, which: Info) {
        let rows = which.rows();
        let r = screen(which.title());
        ui::claim_keys(ROOT, &["cancel", "left", "select"].map(String::from));
        let scroll = ui::create(WidgetKind::Scroll, ROOT, r);
        let m = ui::get_metrics();
        let inset: u16 = if m.color { 15 } else { 4 };
        let width = r.width.saturating_sub(2 * inset + m.scroll_bar_width);
        let mut values = Vec::with_capacity(rows.len());
        for (i, (label, value)) in rows.iter().enumerate() {
            let y = (i as u16 * m.row_height) as i16;
            let label_w = if label.is_empty() { 0 } else { width * 2 / 5 };
            if label_w > 0 {
                let l = ui::create(
                    WidgetKind::Text,
                    scroll,
                    Rect::new(inset as i16, y, label_w, m.row_height),
                );
                ui::set_text(l, label);
            }
            let v = ui::create(
                WidgetKind::Text,
                scroll,
                Rect::new((inset + label_w) as i16, y, width - label_w, m.row_height),
            );
            if label_w > 0 {
                ui::set_align(v, Align::Right);
            }
            ui::set_text(v, value);
            values.push(v);
        }
        ui::set_softkeys(
            ROOT,
            &[blank(), blank(), blank(), Softkey::new(SK_BACK, "Back")],
        );
        ui::set_focus(scroll);
        self.page = Page::Info {
            which,
            values,
            rows,
        };
        if self.timer.is_none() {
            self.timer = Some(timers::set(REFRESH_MS, true));
        }
    }

    /// Brings the status page on screen up to date.
    fn refresh(&mut self) {
        let Page::Info {
            which,
            values,
            rows,
        } = &mut self.page
        else {
            return;
        };
        let now = which.rows();
        if now == *rows {
            return;
        }
        if now.len() != rows.len() {
            let which = *which;
            self.show_info(which);
            return;
        }
        for ((old, new), w) in rows.iter().zip(&now).zip(values.iter()) {
            if old.1 != new.1 {
                ui::set_text(*w, &new.1);
            }
        }
        *rows = now;
    }

    fn activate(&mut self, row: u32) {
        self.selected = row;
        let test = match row {
            0 => HardwareTest::Display,
            1 => HardwareTest::Keys,
            2 => HardwareTest::Lamps,
            3 => return diagnostics::play_test_tone(),
            4 => return self.show_info(Info::Health),
            5 => return self.show_info(Info::Log),
            _ => return,
        };
        // The test covers this app; it is still here when the test ends.
        runtime::set_exit_after_background(false);
        diagnostics::show_test(test);
    }

    fn back(&mut self) {
        match self.page {
            Page::Info { .. } => self.show_menu(),
            _ => runtime::background(),
        }
    }
}

impl App for Diagnostics {
    fn start(&mut self) {}

    fn handle(&mut self, event: Event) {
        match event {
            Event::Foreground => {
                runtime::set_exit_after_background(true);
                match self.page {
                    Page::Info { which, .. } => self.show_info(which),
                    _ => self.show_menu(),
                }
            }
            Event::Background => self.stop_timer(),
            Event::Timer(_) => self.refresh(),
            Event::Widget((_, WidgetEvent::Highlighted(row))) => self.selected = row,
            Event::Widget((_, WidgetEvent::Activated(row))) => self.activate(row),
            Event::Softkey(SK_SELECT) => {
                if let Page::Menu { list } = self.page {
                    self.activate(ui::selected(list));
                }
            }
            Event::Softkey(SK_BACK) => self.back(),
            Event::Key(k) => match (&self.page, k.as_str()) {
                (Page::Menu { list }, "right") => {
                    let row = ui::selected(*list);
                    self.activate(row);
                }
                (Page::Menu { list }, digit) if digit.len() == 1 && digit != "0" => {
                    let row = u32::from(digit.as_bytes()[0] - b'1');
                    ui::set_selected(*list, row);
                    self.activate(row);
                }
                _ => self.back(),
            },
            _ => {}
        }
    }
}

dphone_app_sdk::export_app!(Diagnostics);
