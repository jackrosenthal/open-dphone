//! A test guest that calls host services on request and shows what came
//! back: softkey 1 dials with headers, 2 plays a sound it carries, 3 stops
//! it, 4 resets the idle timer, 5 shows a picture it carries.
//! Its idle window says whether the idle screen is showing.  The message
//! `panic` makes the app panic with the message's payload.
//!
//! Softkeys 6 to 11 use the app's store on the PBX: 6 watches it and
//! lists the shared keys under `best/`, 7 adds the user's name to the
//! list in the shared key `best/top` without losing a name another phone
//! adds meanwhile, 8 saves the user's progress, 9 loads it, 10 says who
//! the user is, and 11 asks for a key that cannot be one.

use dphone_app_sdk::prelude::*;
use dphone_app_sdk::{Header, ListItem};

/// The app's data is in its code.
const PIC: &[u8] = include_bytes!("../data/pic.png");
const TONE: &[u8] = include_bytes!("../data/tone.wav");

/// What a request to the PBX's store was for.
enum Asked {
    Board,
    /// Reading `best/top` to add a name to it.
    TopRead,
    /// Writing `best/top` with a name added.
    TopWrite,
    Save,
    Load,
    Bad,
}

#[derive(Default)]
struct Probe {
    out: Widget,
    idle: Widget,
    asked: Vec<(u32, Asked)>,
}

fn text(value: &[u8]) -> String {
    String::from_utf8_lossy(value).into_owned()
}

impl Probe {
    /// Writes `best/top` again with the user's name added to what
    /// `current` holds, over exactly that entry.
    fn add_to_top(&mut self, current: Option<PbxEntry>) {
        let me = pbx_storage::user().map(|u| u.name).unwrap_or_default();
        let (mut names, version) = match &current {
            Some(e) => (text(&e.value), e.version),
            None => (String::new(), 0),
        };
        if !names.is_empty() {
            names.push(',');
        }
        names.push_str(&me);
        let id = pbx_storage::set(PbxScope::Shared, "best/top", names.as_bytes(), Some(version));
        self.asked.push((id, Asked::TopWrite));
    }

    fn storage(&mut self, id: u32, outcome: PbxStorageOutcome) -> String {
        let Some(at) = self.asked.iter().position(|(i, _)| *i == id) else {
            return "unasked".to_string();
        };
        let (_, asked) = self.asked.remove(at);
        match (asked, outcome) {
            (_, PbxStorageOutcome::Unreachable(why)) => format!("unreachable={why}"),
            (_, PbxStorageOutcome::Refused(why)) => format!("refused={why}"),
            (Asked::Board, PbxStorageOutcome::Entries((entries, more))) => {
                let rows: Vec<String> = entries
                    .iter()
                    .map(|e| format!("{}:{}@{}", e.key, text(&e.value), e.author_name))
                    .collect();
                format!("board={} more={more}", rows.join(";"))
            }
            (Asked::TopRead, PbxStorageOutcome::Entry(e))
            | (Asked::TopWrite, PbxStorageOutcome::Conflict(e)) => {
                self.add_to_top(e);
                "adding".to_string()
            }
            (Asked::TopWrite, PbxStorageOutcome::Stored(e)) => format!("top={}", text(&e.value)),
            (Asked::Save, PbxStorageOutcome::Stored(e)) => {
                format!("saved={} by {} ({})", text(&e.value), e.author_name, e.author_id)
            }
            (Asked::Load, PbxStorageOutcome::Entry(e)) => {
                format!("loaded={:?}", e.map(|e| text(&e.value)))
            }
            (_, other) => format!("unexpected={other:?}"),
        }
    }
}

impl App for Probe {
    fn start(&mut self) {
        self.out = ui::create(WidgetKind::Text, ROOT, Rect::new(0, 0, 100, 20));
        ui::set_text(self.out, "ready");
        if let Some(root) = ui::idle_root() {
            self.idle = ui::create(WidgetKind::Text, root, Rect::new(0, 0, 100, 20));
            ui::set_text(self.idle, "idle=unknown");
        }
        ui::set_softkeys(
            ROOT,
            &[
                Softkey::new(1, "Dial"),
                Softkey::new(2, "Play"),
                Softkey::new(3, "Stop"),
                Softkey::new(4, "Awake"),
                Softkey::new(5, "Pics"),
                Softkey::new(6, "Board"),
                Softkey::new(7, "Top"),
                Softkey::new(8, "Save"),
                Softkey::new(9, "Load"),
                Softkey::new(10, "Who"),
                Softkey::new(11, "Bad"),
            ],
        );
    }

    fn handle(&mut self, event: Event) {
        let report = match event {
            Event::Softkey(1) => {
                let headers = [
                    Header { name: "X-Digium-Call-Feature".into(), value: "voicemail".into() },
                    Header { name: "X-Probe".into(), value: "1".into() },
                ];
                format!("dial={:?}", calls::dial("8500", Some(1), &headers))
            }
            Event::Softkey(2) => format!("play={:?}", audio::play(TONE, 3)),
            Event::Softkey(3) => {
                audio::stop();
                "stop".to_string()
            }
            Event::Softkey(4) => {
                runtime::reset_idle_timer();
                "awake".to_string()
            }
            Event::Softkey(5) => {
                let defined = ui::define_image("pic.png", PIC);
                let image = ui::create(WidgetKind::Image, ROOT, Rect::new(0, 20, 48, 48));
                ui::set_image(image, "pic.png");
                let list = ui::create(WidgetKind::TextList, ROOT, Rect::new(0, 70, 200, 60));
                let mut mine = ListItem::new("mine");
                mine.icon = "pic.png".into();
                let mut theirs = ListItem::new("theirs");
                theirs.icon = "app-contacts".into();
                ui::set_items(list, &[mine, theirs]);
                format!("pics={defined:?}")
            }
            Event::Softkey(6) => {
                pbx_storage::watch(true);
                let id = pbx_storage::list(PbxScope::Shared, "best/", "");
                self.asked.push((id, Asked::Board));
                "asked".to_string()
            }
            Event::Softkey(7) => {
                let id = pbx_storage::get(PbxScope::Shared, "best/top");
                self.asked.push((id, Asked::TopRead));
                "asked".to_string()
            }
            Event::Softkey(8) => {
                let id = pbx_storage::set(PbxScope::User, "progress", b"level 5", None);
                self.asked.push((id, Asked::Save));
                "asked".to_string()
            }
            Event::Softkey(9) => {
                let id = pbx_storage::get(PbxScope::User, "progress");
                self.asked.push((id, Asked::Load));
                "asked".to_string()
            }
            Event::Softkey(10) => {
                let who = pbx_storage::user().map(|u| format!("{}/{}/{}", u.id, u.name, u.extension));
                format!("who={who:?} online={}", pbx_storage::online())
            }
            Event::Softkey(11) => {
                let id = pbx_storage::get(PbxScope::Shared, "not a key");
                self.asked.push((id, Asked::Bad));
                "asked".to_string()
            }
            Event::PbxStorage((id, outcome)) => self.storage(id, outcome),
            Event::PbxStorageChanged(change) => {
                format!("changed={:?}:{}", change.scope, change.key)
            }
            Event::PbxStorageOnline => "online".to_string(),
            Event::Message((what, why)) if what == "panic" => panic!("{why}"),
            Event::PlaybackStopped(at) => format!("stopped={at}"),
            Event::IdleScreenShow => {
                ui::set_text(self.idle, "idle=shown");
                return;
            }
            Event::IdleScreenHide => {
                ui::set_text(self.idle, "idle=hidden");
                return;
            }
            _ => return,
        };
        ui::set_text(self.out, &report);
    }
}

dphone_app_sdk::export_app! {
    app: Probe,
    name: "probe",
    id: "test.probe",
    display_name: "Probe",
    has_idle: true,
    icon: PIC,
}
