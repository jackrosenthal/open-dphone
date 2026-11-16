//! Status: choose the user's presence from the options the PBX offers and
//! set an optional message.

use dphone_app_sdk::prelude::*;
use dphone_app_sdk::{InputMode, pbx_error_text, pbx_request};

const SK_SELECT: u32 = 1;
const SK_MESSAGE: u32 = 2;
const SK_EXIT: u32 = 3;
const SK_OK: u32 = 4;
const SK_BACK: u32 = 5;
const SK_RETRY: u32 = 6;

#[derive(Clone)]
struct Choice {
    id: String,
    presence: String,
    sub: String,
    active: bool,
}

impl Choice {
    fn label(&self) -> String {
        let base = presence_label(&self.presence);
        if self.sub.is_empty() {
            base.to_string()
        } else {
            format!("{base} - {}", self.sub)
        }
    }
}

fn presence_label(token: &str) -> &str {
    match token {
        "available" => "Available",
        "chat" => "Chat",
        "away" => "Away",
        "dnd" => "Do Not Disturb",
        "xa" => "Extended Away",
        "unavailable" => "Unavailable",
        other => other,
    }
}

enum Screen {
    Loading,
    List { list: Widget },
    Message { form: Form },
    Error,
}

struct Status {
    screen: Screen,
    options: Vec<Choice>,
    message: String,
    list_req: Option<u32>,
    info_req: Option<u32>,
    update_req: Option<u32>,
    /// Option being applied, for the phone's own presence on success.
    applying: Option<Choice>,
}

impl Default for Status {
    fn default() -> Self {
        Status {
            screen: Screen::Loading,
            options: Vec::new(),
            message: String::new(),
            list_req: None,
            info_req: None,
            update_req: None,
            applying: None,
        }
    }
}

impl Status {
    fn load(&mut self) {
        Message::show("Status", "Loading...", &[Softkey::new(SK_EXIT, "Exit")]);
        self.screen = Screen::Loading;
        self.list_req = Some(pbx_request(
            "switchvox.users.presence.options.getList",
            Json::obj([]),
        ));
        self.info_req = Some(pbx_request(
            "switchvox.users.presence.getInfo",
            Json::obj([]),
        ));
    }

    fn show_list(&mut self) {
        let r = screen("Status");
        let list = ui::create(WidgetKind::TextList, ROOT, r);
        let items: Vec<ListItem> = self
            .options
            .iter()
            .map(|o| ListItem::with_detail(o.label(), if o.active { "*" } else { "" }))
            .collect();
        ui::set_items(list, &items);
        if let Some(i) = self.options.iter().position(|o| o.active) {
            ui::set_selected(list, i as u32);
        }
        ui::set_softkeys(
            ROOT,
            &[
                Softkey::new(SK_SELECT, "Select"),
                Softkey::new(SK_MESSAGE, "Message"),
                Softkey::new(SK_EXIT, "Exit"),
            ],
        );
        ui::set_focus(list);
        self.screen = Screen::List { list };
    }

    fn show_error(&mut self, text: &str) {
        Message::show(
            "Status",
            text,
            &[
                Softkey::new(SK_RETRY, "Retry"),
                Softkey::new(SK_EXIT, "Exit"),
            ],
        );
        self.screen = Screen::Error;
    }

    fn apply(&mut self, index: usize) {
        let Some(choice) = self.options.get(index).cloned() else {
            return;
        };
        self.update_req = Some(pbx_request(
            "switchvox.users.presence.update",
            Json::obj([
                ("presence_option_id", Json::str(choice.id.clone())),
                ("message", Json::str(self.message.clone())),
            ]),
        ));
        self.applying = Some(choice);
    }

    fn active_index(&self) -> Option<usize> {
        self.options.iter().position(|o| o.active)
    }

    fn on_pbx(&mut self, id: u32, outcome: PbxOutcome) {
        if self.list_req == Some(id) {
            self.list_req = None;
            match &outcome {
                PbxOutcome::Ok(text) => {
                    let v = Json::parse(text).unwrap_or(Json::Null);
                    self.options = v
                        .path(&["presence_options", "presence_option"])
                        .items()
                        .into_iter()
                        .map(|o| Choice {
                            id: o.get("id").text(),
                            presence: o.get("presence").text(),
                            sub: o.get("sub_presence").text(),
                            active: o.get("active").truthy(),
                        })
                        .collect();
                    if self.options.is_empty() {
                        self.show_error("No status options are available.");
                    } else {
                        self.show_list();
                    }
                }
                other => self.show_error(&pbx_error_text(other)),
            }
        } else if self.info_req == Some(id) {
            self.info_req = None;
            if let PbxOutcome::Ok(text) = &outcome {
                let v = Json::parse(text).unwrap_or(Json::Null);
                self.message = v.path(&["presence", "message"]).text();
            }
        } else if self.update_req == Some(id) {
            self.update_req = None;
            match &outcome {
                PbxOutcome::Ok(_) => {
                    if let Some(c) = self.applying.take() {
                        let _ = presence::set_status(&c.presence, &self.message);
                        for o in &mut self.options {
                            o.active = o.id == c.id;
                        }
                    }
                    runtime::background();
                }
                other => {
                    self.applying = None;
                    self.show_error(&pbx_error_text(other));
                }
            }
        }
    }
}

impl App for Status {
    fn start(&mut self) {}

    fn handle(&mut self, event: Event) {
        match event {
            Event::Foreground => self.load(),
            Event::Pbx((id, outcome)) => self.on_pbx(id, outcome),
            Event::Widget((_, WidgetEvent::Activated(i))) => {
                if matches!(self.screen, Screen::List { .. }) {
                    self.apply(i as usize);
                }
            }
            Event::Widget((_, WidgetEvent::Submitted(text))) => {
                if matches!(self.screen, Screen::Message { .. }) {
                    self.message = text;
                    match self.active_index() {
                        Some(i) => self.apply(i),
                        None => self.show_list(),
                    }
                }
            }
            Event::Softkey(SK_SELECT) => {
                if let Screen::List { list } = self.screen {
                    self.apply(ui::selected(list) as usize);
                }
            }
            Event::Softkey(SK_MESSAGE) => {
                let form = Form::show(
                    "Status Message",
                    &[("Message", InputMode::Text, &self.message)],
                    &[Softkey::new(SK_OK, "OK"), Softkey::new(SK_BACK, "Back")],
                );
                self.screen = Screen::Message { form };
            }
            Event::Softkey(SK_OK) => {
                if let Screen::Message { form } = &self.screen {
                    self.message = form.value(0);
                    match self.active_index() {
                        Some(i) => self.apply(i),
                        None => self.show_list(),
                    }
                }
            }
            Event::Softkey(SK_BACK) => self.show_list(),
            Event::Softkey(SK_RETRY) => self.load(),
            Event::Softkey(SK_EXIT) => runtime::background(),
            Event::Key(k) if k == "cancel" => match self.screen {
                Screen::Message { .. } => self.show_list(),
                _ => runtime::background(),
            },
            _ => {}
        }
    }
}

dphone_app_sdk::export_app! {
    app: Status,
    name: "status",
    id: "org.opendphone.status",
    display_name: "Status",
    description: "Set your presence status",
    phone_icon: "status",
}
