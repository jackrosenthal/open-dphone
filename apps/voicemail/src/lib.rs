//! Visual voicemail: folders, the message list, playback, and message
//! actions, over the PBX's `switchvox.users.voicemail.*` methods.
//!
//! Playback is a call: `voicemail.play` makes the PBX ring this phone with
//! the event `digium.incomingCall.voicemail`; the app answers it while it
//! is on screen (and rejects it otherwise), and sends `*`, `#`, and `0` as
//! DTMF for rewind, skip, and pause.

use app_sdk::prelude::*;
use app_sdk::{pbx_error_text, pbx_request};

const SK_PLAY: u32 = 1;
const SK_DELETE: u32 = 2;
const SK_FOLDERS: u32 = 3;
const SK_MARK: u32 = 4;
const SK_CALL: u32 = 5;
const SK_EXIT: u32 = 6;
const SK_BACK: u32 = 7;
const SK_YES: u32 = 8;
const SK_NO: u32 = 9;
const SK_STOP: u32 = 10;
const SK_OPEN: u32 = 11;
const SK_MOVE: u32 = 12;
const SK_REFRESH: u32 = 13;

const PLAYBACK_EVENT: &str = "digium.incomingCall.voicemail";
const PAGE: i64 = 30;

#[derive(Clone, Default)]
struct Msg {
    id: String,
    name: String,
    number: String,
    date: u64,
    duration: i64,
    read: bool,
}

impl Msg {
    fn who(&self) -> &str {
        if self.name.is_empty() {
            &self.number
        } else {
            &self.name
        }
    }
}

#[derive(Clone, Default)]
struct Folder {
    name: String,
    count: String,
}

#[derive(Default, PartialEq)]
enum Screen {
    #[default]
    Loading,
    List,
    Folders,
    MoveTo,
    ConfirmDelete,
    Playing,
    Error,
}

/// What a pending request is for.
#[derive(Clone, Copy, PartialEq)]
enum Want {
    List,
    Folders,
    Play,
    /// Delete, move, or mark: reload the list afterwards.
    Change,
}

struct Voicemail {
    folder: String,
    folders: Vec<Folder>,
    messages: Vec<Msg>,
    screen: Screen,
    list: Widget,
    pending: Vec<(u32, Want)>,
    /// Message being played or acted on.
    current: Option<Msg>,
    call: Option<u32>,
    shown: bool,
}

impl Default for Voicemail {
    fn default() -> Self {
        Voicemail {
            folder: "INBOX".into(),
            folders: Vec::new(),
            messages: Vec::new(),
            screen: Screen::Loading,
            list: 0,
            pending: Vec::new(),
            current: None,
            call: None,
            shown: false,
        }
    }
}

fn folder_title(name: &str) -> String {
    match name {
        "INBOX" => "Inbox".to_string(),
        other => other.to_string(),
    }
}

impl Voicemail {
    fn ask(&mut self, want: Want, method: &str, params: Json) {
        let id = pbx_request(method, params);
        self.pending.push((id, want));
    }

    fn loading(&mut self) {
        Message::show("Voicemail", "Loading...", &[Softkey::new(SK_EXIT, "Exit")]);
        self.screen = Screen::Loading;
    }

    fn load_list(&mut self) {
        self.ask(
            Want::List,
            "switchvox.users.voicemail.getList",
            Json::obj([
                ("folder", Json::str(self.folder.clone())),
                ("sort_field", Json::str("date")),
                ("sort_order", Json::str("DESC")),
                ("items_per_page", Json::num(PAGE)),
            ]),
        );
    }

    fn load_folders(&mut self) {
        self.ask(
            Want::Folders,
            "switchvox.users.voicemail.getFolderList",
            Json::obj([]),
        );
    }

    fn selected(&self) -> Option<Msg> {
        if self.screen != Screen::List || self.messages.is_empty() {
            return None;
        }
        self.messages.get(ui::selected(self.list) as usize).cloned()
    }

    fn list_keys(&self) -> Vec<Softkey> {
        let mark = match self.selected() {
            Some(m) if m.read => "Unread",
            _ => "Read",
        };
        vec![
            Softkey::new(SK_PLAY, "Play"),
            Softkey::new(SK_DELETE, "Delete"),
            Softkey::new(SK_FOLDERS, "Folders"),
            Softkey::new(SK_MARK, mark),
            Softkey::new(SK_MOVE, "Move"),
            Softkey::new(SK_CALL, "Call Back"),
            Softkey::new(SK_EXIT, "Exit"),
        ]
    }

    fn draw_list(&mut self) {
        let r = screen(&folder_title(&self.folder));
        if self.messages.is_empty() {
            let t = ui::create(
                WidgetKind::Text,
                ROOT,
                Rect::new(4, r.y + 2, r.width - 8, r.height - 4),
            );
            ui::set_wrap(t, true);
            ui::set_text(t, "No messages.");
            ui::set_softkeys(
                ROOT,
                &[
                    Softkey::new(SK_FOLDERS, "Folders"),
                    Softkey::new(SK_REFRESH, "Refresh"),
                    Softkey::new(SK_EXIT, "Exit"),
                ],
            );
            self.screen = Screen::List;
            self.list = 0;
            return;
        }
        let list = ui::create(WidgetKind::TextList, ROOT, r);
        let items: Vec<ListItem> = self
            .messages
            .iter()
            .map(|m| {
                let mark = if m.read { "" } else { "* " };
                ListItem::with_detail(format!("{mark}{}", m.who()), l10n::format_time(m.date, 2))
            })
            .collect();
        ui::set_items(list, &items);
        ui::set_focus(list);
        self.list = list;
        self.screen = Screen::List;
        ui::set_softkeys(ROOT, &self.list_keys());
    }

    fn draw_folders(&mut self, moving: bool) {
        let r = screen(if moving { "Move To" } else { "Folders" });
        let list = ui::create(WidgetKind::TextList, ROOT, r);
        let items: Vec<ListItem> = self
            .folders
            .iter()
            .map(|f| ListItem::with_detail(folder_title(&f.name), f.count.clone()))
            .collect();
        ui::set_items(list, &items);
        ui::set_softkeys(
            ROOT,
            &[
                Softkey::new(SK_OPEN, if moving { "Move" } else { "Open" }),
                Softkey::new(SK_BACK, "Back"),
            ],
        );
        ui::set_focus(list);
        self.list = list;
        self.screen = if moving {
            Screen::MoveTo
        } else {
            Screen::Folders
        };
    }

    fn ids(m: &Msg) -> Json {
        Json::Arr(vec![Json::str(m.id.clone())])
    }

    fn play(&mut self) {
        let Some(m) = self.selected() else { return };
        self.ask(
            Want::Play,
            "switchvox.users.voicemail.play",
            Json::obj([
                ("folder", Json::str(self.folder.clone())),
                ("message_id", Json::str(m.id.clone())),
            ]),
        );
        let text = format!(
            "{}\n{}\n{}:{:02}\n\nConnecting...",
            m.who(),
            l10n::format_time(m.date, 2),
            m.duration / 60,
            m.duration % 60
        );
        Message::show("Playing", &text, &[Softkey::new(SK_STOP, "Stop")]);
        ui::claim_keys(ROOT, &["cancel", "*", "#", "0"].map(String::from));
        self.current = Some(m);
        self.screen = Screen::Playing;
    }

    fn stop_playing(&mut self) {
        if let Some(call) = self.call.take() {
            let _ = calls::hangup(call);
        }
        self.after_playing();
    }

    fn after_playing(&mut self) {
        self.call = None;
        if self.screen != Screen::Playing {
            return;
        }
        // Listening marks the message read, as the stock app does.
        match self.current.take() {
            Some(m) if !m.read => {
                self.loading();
                self.ask(
                    Want::Change,
                    "switchvox.users.voicemail.markRead",
                    Json::obj([
                        ("folder", Json::str(self.folder.clone())),
                        ("message_ids", Self::ids(&m)),
                    ]),
                );
            }
            _ => self.draw_list(),
        }
    }

    fn change(&mut self, method: &str, extra: Vec<(&str, Json)>) {
        let Some(m) = self.current.take().or_else(|| self.selected()) else {
            return;
        };
        let mut params = vec![("message_ids", Self::ids(&m))];
        params.extend(extra);
        self.loading();
        self.ask(
            Want::Change,
            method,
            Json::Obj(
                params
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
            ),
        );
    }

    fn error(&mut self, outcome: &PbxOutcome) {
        Message::show(
            "Voicemail",
            &pbx_error_text(outcome),
            &[
                Softkey::new(SK_REFRESH, "Retry"),
                Softkey::new(SK_EXIT, "Exit"),
            ],
        );
        self.screen = Screen::Error;
    }

    fn on_pbx(&mut self, id: u32, outcome: PbxOutcome) {
        let Some(pos) = self.pending.iter().position(|(i, _)| *i == id) else {
            return;
        };
        let (_, want) = self.pending.remove(pos);
        let ok = match &outcome {
            PbxOutcome::Ok(text) => Json::parse(text).unwrap_or(Json::Null),
            other => {
                if want == Want::Play {
                    self.current = None;
                }
                if self.shown {
                    self.error(other);
                }
                return;
            }
        };
        match want {
            Want::List => {
                self.messages = ok
                    .path(&["messages", "message"])
                    .items()
                    .into_iter()
                    .map(|m| Msg {
                        id: m.get("id").text(),
                        name: m.get("caller_id_name").text(),
                        number: m.get("caller_id_number").text(),
                        date: m.get("date").int().unwrap_or(0).max(0) as u64,
                        duration: m.get("duration").int().unwrap_or(0),
                        read: m.get("read").truthy(),
                    })
                    .collect();
                if self.shown
                    && matches!(self.screen, Screen::Loading | Screen::List | Screen::Error)
                {
                    self.draw_list();
                }
            }
            Want::Folders => {
                self.folders = ok
                    .path(&["folders", "folder"])
                    .items()
                    .into_iter()
                    .map(|f| Folder {
                        name: f.get("name").text(),
                        count: f.get("count").text(),
                    })
                    .collect();
            }
            // The call arrives as a push; nothing to do with the reply.
            Want::Play => {}
            Want::Change => {
                self.load_list();
                self.load_folders();
            }
        }
    }
}

impl App for Voicemail {
    fn start(&mut self) {
        events::observe(PLAYBACK_EVENT);
        calls::observe(true);
        // Stay loaded so a playback call that arrives late is still handled.
        runtime::set_exit_after_background(false);
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Foreground => {
                self.shown = true;
                self.folder = "INBOX".into();
                self.loading();
                self.load_list();
                self.load_folders();
            }
            Event::Background => {
                self.shown = false;
                if self.screen == Screen::Playing {
                    if let Some(call) = self.call.take() {
                        let _ = calls::hangup(call);
                    }
                    self.current = None;
                    self.screen = Screen::Loading;
                }
            }
            Event::Pbx((id, outcome)) => self.on_pbx(id, outcome),
            Event::Push(p) if p.name == PLAYBACK_EVENT => {
                let Some(call) = p.call else { return };
                if self.shown && self.screen == Screen::Playing {
                    if calls::answer(call).is_ok() {
                        self.call = Some(call);
                    }
                } else {
                    let _ = calls::reject(call);
                }
            }
            Event::Call(c) => {
                if self.call == Some(c.id) && c.state == app_sdk::CallState::Ended {
                    self.after_playing();
                }
            }
            Event::Mwi(_) => {
                if self.shown && self.screen == Screen::List {
                    self.load_list();
                }
            }
            Event::Key(k) => match (k.as_str(), &self.screen) {
                ("*" | "#" | "0", Screen::Playing) => {
                    if let Some(call) = self.call {
                        let _ = calls::send_dtmf(call, &k);
                    }
                }
                ("cancel", Screen::Playing) => self.stop_playing(),
                ("cancel", Screen::Folders | Screen::MoveTo | Screen::ConfirmDelete) => {
                    self.current = None;
                    self.draw_list()
                }
                ("cancel", _) => runtime::background(),
                _ => {}
            },
            Event::Widget((_, WidgetEvent::Highlighted(_))) => {
                if self.screen == Screen::List {
                    ui::set_softkeys(ROOT, &self.list_keys());
                }
            }
            Event::Widget((_, WidgetEvent::Activated(i))) => match self.screen {
                Screen::List => self.play(),
                Screen::Folders => {
                    if let Some(f) = self.folders.get(i as usize) {
                        self.folder = f.name.clone();
                        self.loading();
                        self.load_list();
                    }
                }
                Screen::MoveTo => {
                    if let Some(f) = self.folders.get(i as usize).cloned() {
                        let from = self.folder.clone();
                        self.change(
                            "switchvox.users.voicemail.move",
                            vec![
                                ("folder_from", Json::str(from)),
                                ("folder_to", Json::str(f.name)),
                            ],
                        );
                    }
                }
                _ => {}
            },
            Event::Softkey(SK_PLAY) => self.play(),
            Event::Softkey(SK_STOP) => self.stop_playing(),
            Event::Softkey(SK_DELETE) => {
                if let Some(m) = self.selected() {
                    Message::show(
                        "Delete",
                        &format!("Delete the message from {}?", m.who()),
                        &[Softkey::new(SK_YES, "Yes"), Softkey::new(SK_NO, "No")],
                    );
                    self.current = Some(m);
                    self.screen = Screen::ConfirmDelete;
                }
            }
            Event::Softkey(SK_YES) => {
                let folder = self.folder.clone();
                self.change(
                    "switchvox.users.voicemail.remove",
                    vec![("folder", Json::str(folder))],
                );
            }
            Event::Softkey(SK_NO | SK_BACK) => {
                self.current = None;
                self.draw_list();
            }
            Event::Softkey(SK_MARK) => {
                if let Some(m) = self.selected() {
                    let method = if m.read {
                        "switchvox.users.voicemail.markUnread"
                    } else {
                        "switchvox.users.voicemail.markRead"
                    };
                    let folder = self.folder.clone();
                    self.change(method, vec![("folder", Json::str(folder))]);
                }
            }
            Event::Softkey(SK_MOVE) => {
                if let Some(m) = self.selected() {
                    self.current = Some(m);
                    self.draw_folders(true);
                }
            }
            Event::Softkey(SK_FOLDERS) => self.draw_folders(false),
            Event::Softkey(SK_OPEN) => {
                let i = ui::selected(self.list);
                self.handle(Event::Widget((self.list, WidgetEvent::Activated(i))));
            }
            Event::Softkey(SK_CALL) => {
                if let Some(m) = self.selected()
                    && !m.number.is_empty()
                    && calls::dial(&m.number, None, &[]).is_ok()
                {
                    runtime::background();
                }
            }
            Event::Softkey(SK_REFRESH) => {
                self.loading();
                self.load_list();
                self.load_folders();
            }
            Event::Softkey(SK_EXIT) => runtime::background(),
            _ => {}
        }
    }
}

app_sdk::export_app!(Voicemail);
