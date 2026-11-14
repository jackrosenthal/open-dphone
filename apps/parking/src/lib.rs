//! Parking: lists the calls parked in the user's parking lots and
//! retrieves one by dialing its space.

use app_sdk::prelude::*;
use app_sdk::{pbx_error_text, pbx_request};

const SK_RETRIEVE: u32 = 1;
const SK_REFRESH: u32 = 2;
const SK_EXIT: u32 = 3;
const REFRESH_MS: u32 = 10_000;
/// Push the PBX sends to the parker's phone when a call is parked.
const PARKED_EVENT: &str = "digium.parkingLot.parked";

struct Parked {
    space: String,
    name: String,
    number: String,
    seconds: i64,
}

#[derive(Default)]
struct Parking {
    calls: Vec<Parked>,
    list: Option<Widget>,
    request: Option<u32>,
    timer: Option<u32>,
    shown: bool,
}

fn duration(secs: i64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

impl Parking {
    fn refresh(&mut self) {
        self.request = Some(pbx_request(
            "switchvox.users.parkingLots.getList",
            Json::obj([]),
        ));
    }

    fn draw(&mut self) {
        let r = screen("Parked Calls");
        if self.calls.is_empty() {
            let t = ui::create(
                WidgetKind::Text,
                ROOT,
                Rect::new(4, r.y + 2, r.width - 8, r.height - 4),
            );
            ui::set_wrap(t, true);
            ui::set_text(t, "No calls are parked.");
            ui::set_softkeys(
                ROOT,
                &[
                    Softkey::new(SK_REFRESH, "Refresh"),
                    Softkey::new(SK_EXIT, "Exit"),
                ],
            );
            self.list = None;
            return;
        }
        let keep = self.list.map(ui::selected).unwrap_or(0);
        let list = ui::create(WidgetKind::TextList, ROOT, r);
        let items: Vec<ListItem> = self
            .calls
            .iter()
            .map(|c| {
                let who = if c.name.is_empty() {
                    c.number.clone()
                } else {
                    c.name.clone()
                };
                ListItem::with_detail(format!("{}  {who}", c.space), duration(c.seconds))
            })
            .collect();
        ui::set_items(list, &items);
        ui::set_selected(list, keep.min(items.len() as u32 - 1));
        ui::set_softkeys(
            ROOT,
            &[
                Softkey::new(SK_RETRIEVE, "Retrieve"),
                Softkey::new(SK_REFRESH, "Refresh"),
                Softkey::new(SK_EXIT, "Exit"),
            ],
        );
        ui::set_focus(list);
        self.list = Some(list);
    }

    fn retrieve(&mut self, index: usize) {
        if let Some(c) = self.calls.get(index)
            && calls::dial(&c.space, None, &[]).is_ok()
        {
            runtime::background();
        }
    }

    fn on_reply(&mut self, outcome: PbxOutcome) {
        match outcome {
            PbxOutcome::Ok(text) => {
                let v = Json::parse(&text).unwrap_or(Json::Null);
                self.calls.clear();
                for lot in v.path(&["parking_lots", "parking_lot"]).items() {
                    for c in lot.path(&["parked_calls", "parked_call"]).items() {
                        self.calls.push(Parked {
                            space: c.get("lotnum").text(),
                            name: c.get("from_caller_id_name").text(),
                            number: c.get("from_caller_id_number").text(),
                            seconds: c.get("duration").int().unwrap_or(0),
                        });
                    }
                }
                self.calls.sort_by(|a, b| a.space.cmp(&b.space));
                self.draw();
            }
            other => {
                self.list = None;
                Message::show(
                    "Parked Calls",
                    &pbx_error_text(&other),
                    &[
                        Softkey::new(SK_REFRESH, "Retry"),
                        Softkey::new(SK_EXIT, "Exit"),
                    ],
                );
            }
        }
    }
}

impl App for Parking {
    fn start(&mut self) {
        events::observe(PARKED_EVENT);
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Foreground => {
                self.shown = true;
                self.list = None;
                Message::show(
                    "Parked Calls",
                    "Loading...",
                    &[Softkey::new(SK_EXIT, "Exit")],
                );
                self.refresh();
                self.timer = Some(timers::set(REFRESH_MS, true));
            }
            Event::Background | Event::Exit => {
                self.shown = false;
                if let Some(t) = self.timer.take() {
                    timers::cancel(t);
                }
            }
            Event::Timer(id) if self.timer == Some(id) => self.refresh(),
            Event::Push(p) if p.name == PARKED_EVENT && self.shown => self.refresh(),
            Event::Pbx((id, outcome)) if self.request == Some(id) => {
                self.request = None;
                if self.shown {
                    self.on_reply(outcome);
                }
            }
            Event::Widget((_, WidgetEvent::Activated(i))) => self.retrieve(i as usize),
            Event::Softkey(SK_RETRIEVE) => {
                if let Some(l) = self.list {
                    self.retrieve(ui::selected(l) as usize);
                }
            }
            Event::Softkey(SK_REFRESH) => self.refresh(),
            Event::Softkey(SK_EXIT) => runtime::background(),
            Event::Key(k) if k == "cancel" => runtime::background(),
            _ => {}
        }
    }
}

app_sdk::export_app!(Parking);
