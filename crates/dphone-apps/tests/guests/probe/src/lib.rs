//! A test guest that calls host services on request and shows what came
//! back: softkey 1 dials with headers, 2 plays a sound it carries, 3 stops
//! it, 4 resets the idle timer, 5 shows a picture it carries.
//! Its idle window says whether the idle screen is showing.

use dphone_app_sdk::prelude::*;
use dphone_app_sdk::{Header, ListItem};

/// The app's data is in its code.
const PIC: &[u8] = include_bytes!("../data/pic.png");
const TONE: &[u8] = include_bytes!("../data/tone.wav");

#[derive(Default)]
struct Probe {
    out: Widget,
    idle: Widget,
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
                Softkey::new(6, "Bad"),
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
            Event::Softkey(6) => format!("play={:?}", audio::play_file(Location::Tmp, "missing.wav", 0)),
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
