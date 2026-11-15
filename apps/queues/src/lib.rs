//! Queues: the user's call queues with their login state; log in and out,
//! pause and resume, and view a queue's numbers for today.

use dphone_app_sdk::prelude::*;
use dphone_app_sdk::{InputMode, pbx_error_text, pbx_request};

const SK_LOGIN: u32 = 1;
const SK_LOGOUT: u32 = 2;
const SK_PAUSE: u32 = 3;
const SK_RESUME: u32 = 4;
const SK_DETAILS: u32 = 5;
const SK_EXIT: u32 = 6;
const SK_BACK: u32 = 7;
const SK_OK: u32 = 8;
const SK_REFRESH: u32 = 9;
const REFRESH_MS: u32 = 15_000;

#[derive(Clone, Default)]
struct Queue {
    id: String,
    name: String,
    /// `logged_in`, `logged_out`, `permanent`, `manager`.
    status: String,
    paused: bool,
}

impl Queue {
    fn state_text(&self) -> &str {
        match (self.status.as_str(), self.paused) {
            ("logged_in" | "permanent", true) => "Paused",
            ("logged_in", false) => "In",
            ("permanent", false) => "Always",
            ("manager", _) => "Manager",
            _ => "Out",
        }
    }
    fn member(&self) -> bool {
        matches!(self.status.as_str(), "logged_in" | "permanent")
    }
}

#[derive(Default)]
enum Screen {
    #[default]
    Loading,
    List,
    Details,
    Reason(Form),
    Error,
}

#[derive(Default)]
struct Queues {
    queues: Vec<Queue>,
    screen: Screen,
    selected: usize,
    list_req: Option<u32>,
    action_req: Option<u32>,
    stats_req: Option<u32>,
    timer: Option<u32>,
    shown: bool,
}

impl Queues {
    fn refresh(&mut self) {
        self.list_req = Some(pbx_request(
            "switchvox.users.callQueues.getList",
            Json::obj([]),
        ));
    }

    fn current(&self) -> Option<&Queue> {
        self.queues.get(self.selected)
    }

    fn softkeys(&self) -> Vec<Softkey> {
        let mut keys = Vec::new();
        if let Some(q) = self.current() {
            match q.status.as_str() {
                "logged_in" => keys.push(Softkey::new(SK_LOGOUT, "Log Out")),
                "logged_out" => keys.push(Softkey::new(SK_LOGIN, "Log In")),
                _ => {}
            }
            if q.member() {
                keys.push(if q.paused {
                    Softkey::new(SK_RESUME, "Resume")
                } else {
                    Softkey::new(SK_PAUSE, "Pause")
                });
            }
            keys.push(Softkey::new(SK_DETAILS, "Details"));
        }
        keys.push(Softkey::new(SK_EXIT, "Exit"));
        keys
    }

    fn draw_list(&mut self) {
        let r = screen("Queues");
        if self.queues.is_empty() {
            let t = ui::create(
                WidgetKind::Text,
                ROOT,
                Rect::new(4, r.y + 2, r.width - 8, r.height - 4),
            );
            ui::set_wrap(t, true);
            ui::set_text(t, "You are not a member of any queue.");
            ui::set_softkeys(ROOT, &[Softkey::new(SK_EXIT, "Exit")]);
            self.screen = Screen::Error;
            return;
        }
        self.selected = self.selected.min(self.queues.len() - 1);
        let list = ui::create(WidgetKind::TextList, ROOT, r);
        let items: Vec<ListItem> = self
            .queues
            .iter()
            .map(|q| ListItem::with_detail(q.name.clone(), q.state_text()))
            .collect();
        ui::set_items(list, &items);
        ui::set_selected(list, self.selected as u32);
        ui::set_softkeys(ROOT, &self.softkeys());
        ui::set_focus(list);
        self.screen = Screen::List;
    }

    fn action(&mut self, method: &str, reason: Option<String>) {
        let Some(q) = self.current() else { return };
        let mut params = vec![("call_queue_account_id", Json::str(q.id.clone()))];
        if let Some(r) = reason {
            params.push(("reason", Json::str(r)));
        }
        let params = Json::Obj(
            params
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        );
        self.action_req = Some(pbx_request(method, params));
    }

    fn show_details(&mut self) {
        let Some(q) = self.current().cloned() else {
            return;
        };
        Message::show(&q.name, "Loading...", &[Softkey::new(SK_BACK, "Back")]);
        self.screen = Screen::Details;
        self.stats_req = Some(pbx_request(
            "switchvox.users.callQueues.getTodaysStatus",
            Json::obj([("call_queue_account_id", Json::str(q.id))]),
        ));
    }

    fn draw_details(&mut self, v: &Json) {
        let Some(q) = self.current().cloned() else {
            return;
        };
        let o = v.path(&["callQueue", "overview"]);
        let field = |k: &str| {
            let t = o.get(k).text();
            if t.is_empty() { "-".to_string() } else { t }
        };
        let waiting = v
            .path(&["callQueue", "waiting_callers", "waiting_caller"])
            .items()
            .len();
        let text = format!(
            "Members: {} ({} logged in)\nOn calls: {}\nWaiting: {}\nCalls today: {}\nCompleted: {}\nAbandoned: {}",
            field("total_members"),
            field("logged_in_count"),
            field("members_on_call"),
            if o.get("callers_waiting").is_null() {
                waiting.to_string()
            } else {
                field("callers_waiting")
            },
            field("total_calls"),
            field("completed_calls_today"),
            field("abandoned_calls"),
        );
        Message::show(
            &q.name,
            &text,
            &[
                Softkey::new(SK_REFRESH, "Refresh"),
                Softkey::new(SK_BACK, "Back"),
            ],
        );
        self.screen = Screen::Details;
    }

    fn error(&mut self, outcome: &PbxOutcome) {
        Message::show(
            "Queues",
            &pbx_error_text(outcome),
            &[Softkey::new(SK_BACK, "Back")],
        );
        self.screen = Screen::Error;
    }

    fn on_pbx(&mut self, id: u32, outcome: PbxOutcome) {
        if self.list_req == Some(id) {
            self.list_req = None;
            match &outcome {
                PbxOutcome::Ok(text) => {
                    let v = Json::parse(text).unwrap_or(Json::Null);
                    self.queues = v
                        .path(&["callQueues", "callQueue"])
                        .items()
                        .into_iter()
                        .map(|q| Queue {
                            id: q.get("account_id").text(),
                            name: q.get("name").text(),
                            status: q.get("loggedin_status").text(),
                            paused: !q.get("paused_time").is_null(),
                        })
                        .collect();
                    // Do not pull the user out of a detail or input screen.
                    if matches!(self.screen, Screen::Loading | Screen::List | Screen::Error) {
                        self.draw_list();
                    }
                }
                other if matches!(self.screen, Screen::Loading) => self.error(other),
                _ => {}
            }
        } else if self.action_req == Some(id) {
            self.action_req = None;
            match &outcome {
                PbxOutcome::Ok(_) => {
                    self.screen = Screen::Loading;
                    self.refresh();
                }
                other => self.error(other),
            }
        } else if self.stats_req == Some(id) {
            self.stats_req = None;
            if matches!(self.screen, Screen::Details) {
                match &outcome {
                    PbxOutcome::Ok(text) => {
                        self.draw_details(&Json::parse(text).unwrap_or(Json::Null))
                    }
                    other => self.error(other),
                }
            }
        }
    }
}

impl App for Queues {
    fn start(&mut self) {
        // Keep the queue list across visits, as the stock app does.
        runtime::set_exit_after_background(false);
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Foreground => {
                self.shown = true;
                if self.queues.is_empty() {
                    Message::show("Queues", "Loading...", &[Softkey::new(SK_EXIT, "Exit")]);
                    self.screen = Screen::Loading;
                } else {
                    self.draw_list();
                }
                self.refresh();
                self.timer = Some(timers::set(REFRESH_MS, true));
            }
            Event::Background | Event::Exit => {
                self.shown = false;
                if let Some(t) = self.timer.take() {
                    timers::cancel(t);
                }
            }
            Event::Timer(id) if self.timer == Some(id) => {
                if matches!(self.screen, Screen::List) {
                    self.refresh();
                }
            }
            Event::Pbx((id, outcome)) => self.on_pbx(id, outcome),
            Event::Widget((_, WidgetEvent::Highlighted(i))) => {
                self.selected = i as usize;
                ui::set_softkeys(ROOT, &self.softkeys());
            }
            Event::Widget((_, WidgetEvent::Activated(i))) => {
                self.selected = i as usize;
                self.show_details();
            }
            Event::Softkey(SK_LOGIN) => self.action("switchvox.users.callQueues.login", None),
            Event::Softkey(SK_LOGOUT) => self.action("switchvox.users.callQueues.logout", None),
            Event::Softkey(SK_RESUME) => self.action("switchvox.users.callQueues.resume", None),
            Event::Softkey(SK_PAUSE) => {
                let form = Form::show(
                    "Pause",
                    &[("Reason", InputMode::Text, "")],
                    &[Softkey::new(SK_OK, "Pause"), Softkey::new(SK_BACK, "Back")],
                );
                self.screen = Screen::Reason(form);
            }
            Event::Softkey(SK_OK) | Event::Widget((_, WidgetEvent::Submitted(_))) => {
                if let Screen::Reason(form) = &self.screen {
                    let reason = form.value(0);
                    self.action("switchvox.users.callQueues.pause", Some(reason));
                }
            }
            Event::Softkey(SK_DETAILS | SK_REFRESH) => self.show_details(),
            Event::Softkey(SK_BACK) => self.draw_list(),
            Event::Softkey(SK_EXIT) => runtime::background(),
            Event::Key(k) if k == "cancel" => match self.screen {
                Screen::Details | Screen::Reason(_) => self.draw_list(),
                _ => runtime::background(),
            },
            _ => {}
        }
    }
}

dphone_app_sdk::export_app!(Queues);
