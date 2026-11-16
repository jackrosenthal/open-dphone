//! Alarms: the screens for setting the phone's alarm clock.  The alarm
//! clock itself is the phone's (the interface `alarms`): it keeps the
//! alarms, rings them, and shows the ringing alarm with Snooze and Dismiss,
//! whether or not this app runs.  Here the user adds, changes, switches,
//! and deletes alarms, chooses each one's days and ringtone, and says
//! whether the idle screen shows an icon while an alarm is upcoming.

use dphone_app_sdk::alarms::{self, Alarm};
use dphone_app_sdk::prelude::*;
use dphone_app_sdk::{Align, Error, Font};

mod text;

use text::{Typed, days_label, sentence};

const SK_NEW: u32 = 1;
const SK_EDIT: u32 = 2;
const SK_SWITCH: u32 = 3;
const SK_DELETE: u32 = 4;
const SK_DISMISS: u32 = 5;
const SK_EXIT: u32 = 6;
const SK_SAVE: u32 = 7;
const SK_CHANGE: u32 = 8;
const SK_CANCEL: u32 = 9;
const SK_OK: u32 = 10;
const SK_MERIDIEM: u32 = 11;
const SK_ERASE: u32 = 12;
const SK_SELECT: u32 = 13;
const SK_PLAY: u32 = 14;
const SK_BACK: u32 = 15;
const SK_DONE: u32 = 16;

const DAY_NAMES: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// The rows of the screen that edits one alarm.
const ROW_TIME: u32 = 0;
const ROW_REPEAT: u32 = 1;
const ROW_TONE: u32 = 2;
const ROW_ON: u32 = 3;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Screen {
    #[default]
    List,
    Edit,
    Time,
    Days,
    Tones,
    Error,
}

#[derive(Default)]
struct Alarms {
    screen: Screen,
    /// The widget of the screen that takes the keys: a list, or the
    /// time's text.
    widget: Widget,
    alarms: Vec<Alarm>,
    /// The row highlighted in the list of alarms, and in the alarm's rows.
    row: u32,
    edit_row: u32,
    /// The alarm being added or changed.
    draft: Option<Alarm>,
    typed: Typed,
    /// The phone shows times with AM and PM.
    meridiem: bool,
}

/// The text of a time of day as the phone shows times.
fn time_text(hour: u8, minute: u8) -> String {
    // Any day will do: the text has no date.
    let unix = 86_400 * 20_000 + i64::from(hour) * 3600 + i64::from(minute) * 60;
    l10n::format_time((unix - i64::from(l10n::utc_offset())) as u64, TimeStyle::Time)
}

fn on_off(on: bool) -> &'static str {
    if on { "On" } else { "Off" }
}

impl Alarms {
    fn list_widget(&mut self, title: &str, items: &[ListItem], selected: u32) {
        let area = screen(title);
        let list = ui::create(WidgetKind::TextList, ROOT, area);
        ui::set_items(list, items);
        ui::set_selected(list, selected.min(items.len().saturating_sub(1) as u32));
        ui::set_focus(list);
        self.widget = list;
    }

    // ------------------------------------------------------------ the list

    /// The alarm of the highlighted row; the row after the alarms is the
    /// idle screen's icon.
    fn selected(&self) -> Option<&Alarm> {
        self.alarms.get(self.row as usize)
    }

    fn list_softkeys(&self) {
        let mut keys = vec![Softkey::new(SK_NEW, "New")];
        match self.selected() {
            Some(a) => {
                keys.push(Softkey::new(SK_EDIT, "Edit"));
                keys.push(Softkey::new(SK_SWITCH, if a.enabled { "Turn Off" } else { "Turn On" }));
                if alarms::snoozed().is_some_and(|(id, _)| id == a.id) {
                    keys.push(Softkey::new(SK_DISMISS, "Dismiss"));
                }
                keys.push(Softkey::new(SK_DELETE, "Delete"));
            }
            None => keys.push(Softkey::new(SK_CHANGE, "Change")),
        }
        keys.push(Softkey::new(SK_EXIT, "Exit"));
        ui::set_softkeys(ROOT, &keys);
    }

    fn show_list(&mut self) {
        self.screen = Screen::List;
        self.draft = None;
        self.alarms = alarms::list();
        let snoozed = alarms::snoozed().map(|(id, _)| id);
        let mut items: Vec<ListItem> = self
            .alarms
            .iter()
            .map(|a| {
                let detail = if snoozed == Some(a.id) {
                    "Snoozed".to_string()
                } else if a.enabled {
                    days_label(a.days)
                } else {
                    "Off".to_string()
                };
                ListItem::with_detail(time_text(a.hour, a.minute), detail)
            })
            .collect();
        items.push(ListItem::with_detail("Idle screen icon", on_off(alarms::icon())));
        self.row = self.row.min(items.len() as u32 - 1);
        self.list_widget("Alarms", &items, self.row);
        self.list_softkeys();
    }

    fn switch(&mut self) {
        let Some(a) = self.selected() else { return };
        let result = alarms::set(&Alarm {
            enabled: !a.enabled,
            ..a.clone()
        });
        self.after(result.map(|_| ()));
    }

    /// Shows the list again after a change, or why the change was not made.
    fn after(&mut self, result: Result<(), Error>) {
        match result {
            Ok(()) => self.show_list(),
            Err(e) => self.show_error(&e),
        }
    }

    fn show_error(&mut self, e: &Error) {
        self.screen = Screen::Error;
        let why = match e {
            Error::Invalid(why) | Error::Failed(why) => sentence(why),
            _ => "The alarm clock did not answer.".to_string(),
        };
        Message::show("Alarms", &why, &[Softkey::new(SK_OK, "OK")]);
    }

    // ------------------------------------------------------------ one alarm

    fn new_alarm(&mut self) {
        // The next whole hour, once.
        let local = timers::wall_time() as i64 + i64::from(l10n::utc_offset());
        let hour = ((local.rem_euclid(86_400) / 3600 + 1) % 24) as u8;
        self.draft = Some(Alarm {
            id: 0,
            hour,
            minute: 0,
            days: 0,
            tone: alarms::tones().first().cloned().unwrap_or_default(),
            enabled: true,
        });
        self.edit_row = ROW_TIME;
        self.show_edit();
    }

    fn edit_alarm(&mut self) {
        if let Some(a) = self.selected() {
            self.draft = Some(a.clone());
            self.edit_row = ROW_TIME;
            self.show_edit();
        }
    }

    fn show_edit(&mut self) {
        let Some(d) = &self.draft else {
            return self.show_list();
        };
        self.screen = Screen::Edit;
        let items = [
            ListItem::with_detail("Time", time_text(d.hour, d.minute)),
            ListItem::with_detail("Repeat", days_label(d.days)),
            ListItem::with_detail("Ringtone", d.tone.clone()),
            ListItem::with_detail("Alarm", on_off(d.enabled)),
        ];
        let title = if d.id == 0 { "New Alarm" } else { "Edit Alarm" };
        self.list_widget(title, &items, self.edit_row);
        ui::set_softkeys(
            ROOT,
            &[
                Softkey::new(SK_SAVE, "Save"),
                Softkey::new(SK_CHANGE, "Change"),
                Softkey::new(SK_CANCEL, "Cancel"),
            ],
        );
    }

    fn change_row(&mut self) {
        match self.edit_row {
            ROW_TIME => self.show_time(),
            ROW_REPEAT => self.show_days(0),
            ROW_TONE => self.show_tones(),
            ROW_ON => {
                if let Some(d) = &mut self.draft {
                    d.enabled = !d.enabled;
                }
                self.show_edit();
            }
            _ => {}
        }
    }

    fn save(&mut self) {
        if let Some(d) = self.draft.take() {
            let result = alarms::set(&d);
            if result.is_ok() && d.id == 0 {
                // The new alarm is the last.
                self.row = self.alarms.len() as u32;
            }
            self.after(result.map(|_| ()));
        }
    }

    // -------------------------------------------------------------- the time

    fn show_time(&mut self) {
        let Some(d) = &self.draft else { return };
        self.screen = Screen::Time;
        self.meridiem = time_text(13, 0).contains('M');
        self.typed = Typed::of(d.hour, d.minute, self.meridiem);
        let area = screen("Time");
        let keys: Vec<String> = "0123456789".chars().map(String::from).collect();
        let mut claims = vec!["cancel".to_string(), "select".to_string(), "left".to_string()];
        claims.extend(keys);
        ui::claim_keys(ROOT, &claims);
        let m = ui::get_metrics();
        let hint_h = m.line_height;
        let time = ui::create(
            WidgetKind::Text,
            ROOT,
            Rect::new(0, area.y, area.width, area.height.saturating_sub(hint_h)),
        );
        ui::set_font(time, Font::Large);
        ui::set_align(time, Align::Center);
        let hint = ui::create(
            WidgetKind::Text,
            ROOT,
            Rect::new(0, area.y + (area.height.saturating_sub(hint_h)) as i16, area.width, hint_h),
        );
        ui::set_font(hint, Font::Small);
        ui::set_align(hint, Align::Center);
        ui::set_text(hint, "Type the hour and the minutes.");
        self.widget = time;
        let mut softkeys = vec![Softkey::new(SK_OK, "OK")];
        if self.meridiem {
            softkeys.push(Softkey::new(SK_MERIDIEM, "AM/PM"));
        }
        softkeys.push(Softkey::new(SK_ERASE, "Delete"));
        softkeys.push(Softkey::new(SK_CANCEL, "Cancel"));
        ui::set_softkeys(ROOT, &softkeys);
        self.draw_time();
    }

    fn draw_time(&self) {
        ui::set_text(self.widget, &self.typed.text(self.meridiem));
    }

    fn time_done(&mut self) {
        if let Some((hour, minute)) = self.typed.time(self.meridiem) {
            if let Some(d) = &mut self.draft {
                (d.hour, d.minute) = (hour, minute);
            }
            self.show_edit();
        }
    }

    // -------------------------------------------------------------- the days

    fn show_days(&mut self, row: u32) {
        let Some(d) = &self.draft else { return };
        self.screen = Screen::Days;
        let items: Vec<ListItem> = DAY_NAMES
            .iter()
            .enumerate()
            .map(|(i, name)| ListItem {
                text: name.to_string(),
                detail: String::new(),
                icon: if d.days & (1 << i) != 0 { "ok".into() } else { String::new() },
            })
            .collect();
        self.list_widget("Repeat", &items, row);
        ui::set_softkeys(
            ROOT,
            &[
                Softkey::new(SK_CHANGE, "Change"),
                Softkey::new(SK_DONE, "Done"),
            ],
        );
    }

    fn toggle_day(&mut self) {
        let Some(row) = ui::selected(self.widget) else { return };
        if let Some(d) = &mut self.draft {
            d.days ^= 1 << row.min(6);
        }
        self.show_days(row);
    }

    // ---------------------------------------------------------- the ringtone

    fn show_tones(&mut self) {
        let Some(d) = &self.draft else { return };
        self.screen = Screen::Tones;
        let tones = alarms::tones();
        let items: Vec<ListItem> = tones
            .iter()
            .map(|t| ListItem {
                text: t.clone(),
                detail: String::new(),
                icon: if *t == d.tone { "ok".into() } else { String::new() },
            })
            .collect();
        let at = tones.iter().position(|t| *t == d.tone).unwrap_or(0);
        self.list_widget("Ringtone", &items, at as u32);
        ui::set_softkeys(
            ROOT,
            &[
                Softkey::new(SK_SELECT, "Select"),
                Softkey::new(SK_PLAY, "Play"),
                Softkey::new(SK_BACK, "Back"),
            ],
        );
    }

    fn highlighted_tone(&self) -> Option<String> {
        alarms::tones().into_iter().nth(ui::selected(self.widget)? as usize)
    }

    fn tone_done(&mut self, choose: bool) {
        alarms::preview(None);
        if choose
            && let Some(tone) = self.highlighted_tone()
            && let Some(d) = &mut self.draft
        {
            d.tone = tone;
        }
        self.show_edit();
    }

    // ------------------------------------------------------------- the keys

    /// Select, or the first softkey, on the highlighted row.
    fn activate(&mut self) {
        match self.screen {
            Screen::List => match self.selected() {
                Some(_) => self.edit_alarm(),
                None => self.after(alarms::set_icon(!alarms::icon())),
            },
            Screen::Edit => self.change_row(),
            Screen::Days => self.toggle_day(),
            Screen::Tones => self.tone_done(true),
            Screen::Time => self.time_done(),
            Screen::Error => self.show_list(),
        }
    }

    /// Cancel: one screen back, and out of the app from the list.
    fn back(&mut self) {
        match self.screen {
            Screen::List => runtime::background(),
            Screen::Edit | Screen::Error => self.show_list(),
            Screen::Time | Screen::Days => self.show_edit(),
            Screen::Tones => self.tone_done(false),
        }
    }

    fn key(&mut self, key: &str) {
        match key {
            "cancel" => self.back(),
            "select" => self.activate(),
            "left" if self.screen == Screen::Time => {
                self.typed.erase();
                self.draw_time();
            }
            digit if self.screen == Screen::Time => {
                if let Some(d) = digit.chars().next().and_then(|c| c.to_digit(10)) {
                    self.typed.push(d as u8);
                    self.draw_time();
                }
            }
            _ => {}
        }
    }

    fn softkey(&mut self, id: u32) {
        match id {
            SK_NEW => self.new_alarm(),
            SK_EDIT => self.edit_alarm(),
            SK_SWITCH => self.switch(),
            SK_DELETE => {
                if let Some(id) = self.selected().map(|a| a.id) {
                    self.after(alarms::remove(id));
                }
            }
            SK_DISMISS => self.after(alarms::dismiss()),
            SK_EXIT => runtime::background(),
            SK_SAVE => self.save(),
            SK_CHANGE | SK_SELECT | SK_OK => self.activate(),
            SK_CANCEL | SK_BACK => self.back(),
            SK_DONE => self.show_edit(),
            SK_MERIDIEM => {
                self.typed.pm = !self.typed.pm;
                self.draw_time();
            }
            SK_ERASE => {
                self.typed.erase();
                self.draw_time();
            }
            SK_PLAY => alarms::preview(self.highlighted_tone().as_deref()),
            _ => {}
        }
    }
}

impl App for Alarms {
    fn start(&mut self) {}

    fn handle(&mut self, event: Event) {
        match event {
            Event::Foreground => {
                // The phone's theme may have changed: what was on screen
                // is drawn again.
                match self.screen {
                    Screen::Edit => self.show_edit(),
                    Screen::Days => self.show_days(0),
                    Screen::Tones => self.show_tones(),
                    Screen::Time => self.show_time(),
                    Screen::List | Screen::Error => self.show_list(),
                }
            }
            Event::Background | Event::Exit => {
                if self.screen == Screen::Tones {
                    alarms::preview(None);
                }
            }
            // The alarms changed elsewhere (the web interface, or an alarm
            // rang): the list follows; an alarm being edited is the user's.
            Event::AlarmsChanged if self.screen == Screen::List && runtime::in_foreground() => {
                self.show_list()
            }
            Event::Widget((w, WidgetEvent::Highlighted(row))) if w == self.widget => match self.screen {
                Screen::List => {
                    self.row = row;
                    self.list_softkeys();
                }
                Screen::Edit => self.edit_row = row,
                _ => {}
            },
            Event::Widget((w, WidgetEvent::Activated(row))) if w == self.widget => {
                match self.screen {
                    Screen::List => self.row = row,
                    Screen::Edit => self.edit_row = row,
                    _ => {}
                }
                self.activate();
            }
            Event::Softkey(id) => self.softkey(id),
            Event::Key(k) => self.key(&k),
            _ => {}
        }
    }
}

dphone_app_sdk::export_app! {
    app: Alarms,
    name: "alarms",
    id: "org.opendphone.alarms",
    display_name: "Alarms",
    description: "Set the alarm clock",
    phone_icon: "alarms",
}
