//! The Alarms app: the screens for the phone's alarm clock, under the real
//! runtime.  The alarm clock is the main role's (its tests are there, and
//! the ringing alarm's are in `dphone-ui`); here a stand-in answers the
//! `Alarms` commands and publishes the state, which is all the app sees
//! of it.
//!
//! The last test draws the app's screens with the phone UI on a simulated
//! D60 (color), D70, and D40 (gray), and writes them as PNG files under
//! `alarms/` in cargo's test scratch directory (`CARGO_TARGET_TMPDIR`), or
//! in the directory `ALARMS_PREVIEWS` names.

mod common;

use common::{Rig, capabilities, staged, temp_dir};
use dphone_apps::engine::Target;
use dphone_apps::testutil::{FakeConfig, FakePbx, services};
use dphone_apps::uishell::UiShell;
use dphone_apps::{AppManager, ManagerConfig};
use dphone_hal::{Hal, Screen};
use dphone_state::{Alarm, Alarms, Command, CommandError, Domain, Key, Model, Phase, Reply, Store, System};
use dphone_ui::{Ui, UiConfig};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const APP: &str = "alarms";
const WEEKDAYS: u8 = 0b0111110;

/// Stands in for the phone's alarm clock: keeps the alarms in the state
/// and answers the commands, as the main role's does.  Nothing rings.
fn alarm_clock(store: &Store) -> Arc<Mutex<Vec<Command>>> {
    store.update::<Alarms, _>(|a| {
        a.tones = ["Marimba", "Classic", "Chime"].map(String::from).to_vec();
        a.show_icon = true;
        a.snooze_secs = 600;
    });
    let log = Arc::new(Mutex::new(Vec::new()));
    let (rx, store, seen) = (store.handle(Domain::Alarms), store.clone(), log.clone());
    std::thread::spawn(move || {
        let mut last = 0;
        while let Some(env) = rx.recv() {
            let (cmd, responder) = env.split();
            let mut reply = Ok(Reply::Done);
            store.update::<Alarms, _>(|a| match &cmd {
                Command::SetAlarm { alarm } if alarm.id == 0 && a.alarms.len() >= Alarms::MAX => {
                    reply = Err(CommandError::Invalid(format!("the phone keeps at most {} alarms", Alarms::MAX)));
                }
                Command::SetAlarm { alarm } if alarm.id == 0 => {
                    last += 1;
                    a.alarms.push(Alarm { id: last, ..alarm.clone() });
                    reply = Ok(Reply::Id(last.into()));
                }
                Command::SetAlarm { alarm } => match a.alarms.iter_mut().find(|x| x.id == alarm.id) {
                    Some(x) => *x = alarm.clone(),
                    None => reply = Err(CommandError::Invalid("no such alarm".into())),
                },
                Command::RemoveAlarm { id } => a.alarms.retain(|x| x.id != *id),
                Command::SetAlarmIcon { on } => a.show_icon = *on,
                Command::DismissAlarm => a.snoozed = None,
                _ => {}
            });
            seen.lock().unwrap().push(cmd);
            responder.reply(reply);
        }
    });
    log
}

fn rig() -> (Rig, Arc<Mutex<Vec<Command>>>) {
    let r = Rig::new(FakeConfig::default());
    let log = alarm_clock(&r.store);
    (r, log)
}

fn alarm(hour: u8, minute: u8, days: u8, tone: &str) -> Alarm {
    Alarm { id: 0, hour, minute, days, tone: tone.into(), enabled: true }
}

fn rows(r: &Rig) -> Vec<(String, String)> {
    r.ui.items(APP)
}

fn row(text: &str, detail: &str) -> (String, String) {
    (text.to_string(), detail.to_string())
}

fn type_digits(r: &mut Rig, digits: &str) {
    for d in digits.bytes() {
        r.key(APP, Key::Digit(d - b'0'));
    }
}

#[test]
fn the_app_uses_the_alarm_clock_and_keeps_nothing_itself() {
    let caps = capabilities(APP);
    assert!(caps.contains(&"alarms"), "{caps:?}");
    for no in ["storage", "audio", "calls", "pbx", "http"] {
        assert!(!caps.contains(&no), "{caps:?}");
    }
}

#[test]
fn an_alarm_is_added_with_a_time_days_and_a_ringtone() {
    let (mut r, log) = rig();
    r.launch(APP);
    // No alarms: only the setting of the idle screen's icon.
    assert_eq!(rows(&r), [row("Idle screen icon", "On")]);
    assert_eq!(r.ui.softkeys(APP), ["New", "Change", "Exit"]);

    r.softkey(APP, "New");
    assert_eq!(r.ui.texts(APP), ["New Alarm"]);
    let fields = rows(&r);
    assert_eq!(fields.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), ["Time", "Repeat", "Ringtone", "Alarm"]);
    assert_eq!(fields[1..], [row("Repeat", "Once"), row("Ringtone", "Marimba"), row("Alarm", "On")]);
    assert_eq!(r.ui.softkeys(APP), ["Save", "Change", "Cancel"]);

    // The time: typed as hour and minutes, with AM or PM.
    r.key(APP, Key::Select);
    assert_eq!(r.ui.softkeys(APP), ["OK", "AM/PM", "Delete", "Cancel"]);
    type_digits(&mut r, "063");
    assert!(r.ui.texts(APP).iter().any(|t| t.starts_with("06:3_")), "{:?}", r.ui.texts(APP));
    // Not a time yet: OK does nothing.
    r.softkey(APP, "OK");
    assert_eq!(r.ui.softkeys(APP)[0], "OK");
    type_digits(&mut r, "5");
    r.softkey(APP, "Delete");
    type_digits(&mut r, "0");
    if !r.ui.shows(APP, "06:30 AM") {
        r.softkey(APP, "AM/PM");
    }
    assert!(r.ui.shows(APP, "06:30 AM"), "{:?}", r.ui.texts(APP));
    r.softkey(APP, "OK");
    assert_eq!(rows(&r)[0], row("Time", "6:30 AM"));

    // The days: Monday to Friday.
    r.key(APP, Key::Down);
    r.softkey(APP, "Change");
    assert_eq!(r.ui.texts(APP), ["Repeat"]);
    assert_eq!(rows(&r).len(), 7);
    for _ in 0..5 {
        r.key(APP, Key::Down);
        r.key(APP, Key::Select);
    }
    r.softkey(APP, "Done");
    assert_eq!(rows(&r)[1], row("Repeat", "Weekdays"));

    // The ringtone: Play sounds the highlighted one, Select takes it.
    r.key(APP, Key::Down);
    r.key(APP, Key::Select);
    assert_eq!(rows(&r), [row("Marimba", ""), row("Classic", ""), row("Chime", "")]);
    assert_eq!(r.ui.softkeys(APP), ["Select", "Play", "Back"]);
    r.key(APP, Key::Down);
    r.key(APP, Key::Down);
    r.softkey(APP, "Select");
    assert_eq!(rows(&r)[2], row("Ringtone", "Chime"));

    assert!(log.lock().unwrap().is_empty(), "nothing is set before Save");
    r.softkey(APP, "Save");
    assert_eq!(*log.lock().unwrap(), [Command::SetAlarm { alarm: alarm(6, 30, WEEKDAYS, "Chime") }]);
    assert_eq!(rows(&r), [row("6:30 AM", "Weekdays"), row("Idle screen icon", "On")]);
    assert_eq!(r.ui.softkeys(APP), ["New", "Edit", "Turn Off", "Delete", "Exit"]);
}

#[test]
fn the_ringtone_screen_plays_a_tone_and_stops_it() {
    let (mut r, _log) = rig();
    let audio = Arc::new(Mutex::new(Vec::new()));
    common::answer_commands(&r.store, Domain::Audio, audio.clone(), |_| Ok(Reply::Done));
    r.launch(APP);
    r.softkey(APP, "New");
    r.key(APP, Key::Down);
    r.key(APP, Key::Down);
    r.key(APP, Key::Select);
    r.key(APP, Key::Down);
    r.softkey(APP, "Play");
    r.softkey(APP, "Back");
    assert_eq!(*audio.lock().unwrap(), [Command::PlaySound { name: "Classic".into() }, Command::StopSound]);
    // Back did not take the tone.
    assert_eq!(rows(&r)[2], row("Ringtone", "Marimba"));
}

#[test]
fn alarms_are_changed_switched_and_deleted() {
    let (mut r, log) = rig();
    for a in [alarm(8, 0, 0b0000010, "Marimba"), alarm(13, 15, 0, "Classic")] {
        r.store.send(Command::SetAlarm { alarm: a }).wait().unwrap();
    }
    log.lock().unwrap().clear();
    r.launch(APP);
    assert_eq!(rows(&r), [row("8:00 AM", "Mon"), row("1:15 PM", "Once"), row("Idle screen icon", "On")]);

    // Off, and on again.
    r.softkey(APP, "Turn Off");
    assert_eq!(rows(&r)[0], row("8:00 AM", "Off"));
    assert_eq!(r.ui.softkeys(APP)[2], "Turn On");
    r.softkey(APP, "Turn On");
    assert_eq!(rows(&r)[0], row("8:00 AM", "Mon"));

    // Select edits the highlighted alarm; Cancel leaves it as it was.
    r.key(APP, Key::Down);
    assert_eq!(r.ui.softkeys(APP), ["New", "Edit", "Turn Off", "Delete", "Exit"]);
    r.key(APP, Key::Select);
    assert_eq!(r.ui.texts(APP), ["Edit Alarm"]);
    assert_eq!(rows(&r)[0], row("Time", "1:15 PM"));
    r.key(APP, Key::Select);
    assert!(r.ui.shows(APP, "01:15 PM"));
    type_digits(&mut r, "2145");
    r.key(APP, Key::Select);
    assert_eq!(rows(&r)[0], row("Time", "9:45 PM"));
    r.softkey(APP, "Cancel");
    assert_eq!(rows(&r)[1], row("1:15 PM", "Once"));
    // Changed and saved.
    r.softkey(APP, "Edit");
    for _ in 0..3 {
        r.key(APP, Key::Down);
    }
    r.softkey(APP, "Change");
    assert_eq!(rows(&r)[3], row("Alarm", "Off"));
    r.softkey(APP, "Save");
    assert_eq!(rows(&r)[1], row("1:15 PM", "Off"));

    // Deleted.
    r.softkey(APP, "Delete");
    assert_eq!(rows(&r), [row("8:00 AM", "Mon"), row("Idle screen icon", "On")]);
    // The icon's row: Select or Change switches it.
    assert_eq!(r.ui.softkeys(APP), ["New", "Change", "Exit"]);
    r.softkey(APP, "Change");
    assert_eq!(rows(&r)[1], row("Idle screen icon", "Off"));
    r.key(APP, Key::Select);
    assert_eq!(rows(&r)[1], row("Idle screen icon", "On"));

    let sent = log.lock().unwrap().clone();
    let on = Alarm { id: 1, ..alarm(8, 0, 0b0000010, "Marimba") };
    assert_eq!(
        sent,
        [
            Command::SetAlarm { alarm: Alarm { enabled: false, ..on.clone() } },
            Command::SetAlarm { alarm: on },
            Command::SetAlarm { alarm: Alarm { id: 2, enabled: false, ..alarm(13, 15, 0, "Classic") } },
            Command::RemoveAlarm { id: 2 },
            Command::SetAlarmIcon { on: false },
            Command::SetAlarmIcon { on: true },
        ]
    );
    // Cancel on the list leaves the app.
    r.key(APP, Key::Cancel);
    assert_eq!(r.ui.top(), None);
}

#[test]
fn the_list_follows_changes_made_elsewhere() {
    let (mut r, _log) = rig();
    r.launch(APP);
    assert_eq!(rows(&r).len(), 1);
    // From the web interface.
    r.store.send(Command::SetAlarm { alarm: alarm(7, 5, 0x7f, "Chime") }).wait().unwrap();
    r.settle();
    assert_eq!(rows(&r), [row("7:05 AM", "Every day"), row("Idle screen icon", "On")]);
    // It rang and was snoozed: the list says so, and can end the snooze.
    r.store.update::<Alarms, _>(|a| a.snoozed = Some((1, 1_800_000_000)));
    r.settle();
    assert_eq!(rows(&r)[0], row("7:05 AM", "Snoozed"));
    assert_eq!(r.ui.softkeys(APP), ["New", "Edit", "Turn Off", "Dismiss", "Delete", "Exit"]);
    r.softkey(APP, "Dismiss");
    assert_eq!(rows(&r)[0], row("7:05 AM", "Every day"));
    // An alarm that is being edited is not swept away by a change.
    r.softkey(APP, "Edit");
    r.store.send(Command::SetAlarm { alarm: alarm(9, 0, 0, "Chime") }).wait().unwrap();
    r.settle();
    assert_eq!(r.ui.texts(APP), ["Edit Alarm"]);
    r.softkey(APP, "Cancel");
    assert_eq!(rows(&r).len(), 3);
}

#[test]
fn a_refusal_of_the_alarm_clock_is_shown() {
    let (mut r, _log) = rig();
    for _ in 0..Alarms::MAX {
        r.store.send(Command::SetAlarm { alarm: alarm(8, 0, 0, "Marimba") }).wait().unwrap();
    }
    r.launch(APP);
    r.softkey(APP, "New");
    r.softkey(APP, "Save");
    assert!(r.ui.shows(APP, "The phone keeps at most 16 alarms."), "{:?}", r.ui.texts(APP));
    r.softkey(APP, "OK");
    assert_eq!(rows(&r).len(), Alarms::MAX + 1);
}

/// A simulated phone with the app on screen, drawn by the phone UI.
struct Phone {
    model: Model,
    ui: Ui,
    sim: dphone_hal::sim::SimHandle,
    mgr: AppManager,
    _dir: common::TempDir,
}

impl Phone {
    fn new(model: Model) -> Phone {
        let dir = temp_dir("alarms-phone");
        let mut hal = Hal::sim(model);
        let sim = hal.sim.clone().unwrap();
        let store = Store::new(model);
        let ui = Ui::from_hal(&mut hal, store.clone(), UiConfig::default());
        store.update::<System, _>(|s| {
            s.phase = Phase::Running;
            s.time_synced = true;
        });
        alarm_clock(&store);
        for a in [
            alarm(6, 30, WEEKDAYS, "Chime"),
            alarm(8, 0, 0b0000010, "Marimba"),
            Alarm { enabled: false, ..alarm(9, 30, 0b1000001, "Classic") },
            alarm(14, 45, 0, "Marimba"),
        ] {
            store.send(Command::SetAlarm { alarm: a }).wait().unwrap();
        }
        let mut cfg = ManagerConfig::rooted(staged(), &dir.0.join("nvdata"));
        cfg.target = Target::Pulley64;
        let mut mgr = AppManager::new(
            cfg,
            store,
            Box::new(UiShell::new(ui.handle(), model)),
            services(&FakePbx::default(), FakeConfig::default()),
        )
        .expect("engine");
        mgr.scan();
        let mut phone = Phone { model, ui, sim, mgr, _dir: dir };
        phone.settle();
        phone.mgr.launch(APP).unwrap();
        phone.settle();
        assert_eq!(phone.ui.screen_name(), "app:alarms");
        phone
    }

    fn settle(&mut self) {
        for _ in 0..4 {
            self.ui.pump();
            self.mgr.pump();
        }
        self.ui.pump();
    }

    fn tap(&mut self, keys: &[Key]) {
        for key in keys {
            self.sim.tap(*key);
            self.settle();
        }
    }

    /// Writes the frame as a PNG (the gray ones doubled in size), and
    /// returns its pixels.
    fn shot(&self, name: &str) -> Vec<u8> {
        let dir = match std::env::var_os("ALARMS_PREVIEWS") {
            Some(d) => PathBuf::from(d),
            None => PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("alarms"),
        };
        std::fs::create_dir_all(&dir).unwrap();
        let f = self.sim.frame(Screen::Main).expect("main frame");
        let (w, h) = (f.info.width as usize, f.info.height as usize);
        let bpp = f.info.format.bytes_per_pixel();
        let scale = if self.model.is_color() { 1 } else { 2 };
        let mut rgb = Vec::with_capacity(w * h * 3 * scale * scale);
        for y in 0..h * scale {
            for x in 0..w * scale {
                let i = (y / scale) * f.info.stride + (x / scale) * bpp;
                if bpp == 4 {
                    rgb.extend_from_slice(&[f.pixels[i + 2], f.pixels[i + 1], f.pixels[i]]);
                } else {
                    rgb.extend_from_slice(&[f.pixels[i]; 3]);
                }
            }
        }
        let path = dir.join(format!("{}-{name}.png", self.model.name()));
        let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
        let mut enc = png::Encoder::new(file, (w * scale) as u32, (h * scale) as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&rgb).unwrap();
        f.pixels.to_vec()
    }
}

#[test]
fn the_screens_on_every_kind_of_display() {
    for model in [Model::D60, Model::D70, Model::D40] {
        let mut p = Phone::new(model);
        let list = p.shot("1-list");
        // Edit the first alarm.
        p.tap(&[Key::Select]);
        let edit = p.shot("2-edit");
        assert!(edit != list, "{model:?}");
        p.tap(&[Key::Select]);
        let time = p.shot("3-time");
        assert!(time != edit, "{model:?}");
        p.tap(&[Key::Digit(0), Key::Digit(7), Key::Digit(1)]);
        let typed = p.shot("4-time-typed");
        assert!(typed != time, "{model:?}");
        p.tap(&[Key::Digit(5), Key::Select, Key::Down, Key::Select]);
        let days = p.shot("5-days");
        assert!(days != typed, "{model:?}");
        p.tap(&[Key::Cancel, Key::Down, Key::Down, Key::Select]);
        let tones = p.shot("6-ringtone");
        assert!(tones != days, "{model:?}");
        p.tap(&[Key::Cancel]);
        let edited = p.shot("7-edited");
        assert!(edited != edit, "{model:?}: the new time shows");
        assert_eq!(p.ui.screen_name(), "app:alarms");
    }
}
