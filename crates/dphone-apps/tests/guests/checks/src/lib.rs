//! A test guest that does what a test asks of it and shows what the phone
//! answered.  A test asks with a message (`Event::Message`): the name is
//! what to do, the payload its argument, and the answer is the text of
//! the app's first widget.  The calls and pushes the app is told of are
//! the text of its second widget, in the order they came.

use dphone_app_sdk::canvas::{Color, Op};
use dphone_app_sdk::prelude::*;
use dphone_app_sdk::{CallState, alarms, canvas, lamps};

/// The app's data is in its code.
const TONE: &[u8] = include_bytes!("../../probe/data/tone.wav");
const PIC: &[u8] = include_bytes!("../../probe/data/pic.png");

#[derive(Default)]
struct Checks {
    out: Widget,
    seen: Widget,
    log: Vec<String>,
}

fn state(s: CallState) -> &'static str {
    match s {
        CallState::Dialing => "dialing",
        CallState::Proceeding => "proceeding",
        CallState::Ringback => "ringback",
        CallState::Ringing => "ringing",
        CallState::Active => "active",
        CallState::Held => "held",
        CallState::Transferring => "transferring",
        CallState::Ended => "ended",
    }
}

/// How many times `f` succeeds before it fails, and the failure.
fn until_refused<E: core::fmt::Debug>(most: u32, mut f: impl FnMut(u32) -> Result<(), E>) -> String {
    for n in 0..most {
        if let Err(e) = f(n) {
            return format!("{n} then {e:?}");
        }
    }
    format!("{most} and no refusal")
}

fn fill() -> Vec<Op> {
    vec![Op::Fill((Rect::new(0, 0, 4, 4), Color { r: 255, g: 0, b: 0 }))]
}

impl Checks {
    fn ask(&mut self, what: &str, arg: &str) -> String {
        match what {
            "timers" => until_refused(40, |_| timers::set(60_000, false).map(|_| ())),
            "observe" => until_refused(70, |n| events::observe(&format!("name.{n}"))),
            // A name that is already observed is not one more.
            "observe-again" => format!("{:?}", events::observe("name.0")),
            "subscribe" => until_refused(260, |n| presence::subscribe(&format!("{}", 1000 + n))),
            "watch" => format!("{:?}", events::observe(arg)),
            "calls" => {
                calls::observe(true);
                "observing".to_string()
            }
            "draw" => {
                let text = ui::create(WidgetKind::Text, ROOT, Rect::new(0, 40, 10, 10));
                let c = canvas::create(ROOT, Rect::new(0, 50, 10, 10));
                let on_canvas = canvas::draw(c, &fill());
                let on_text = canvas::draw(text, &fill());
                ui::destroy(c);
                let on_dead = canvas::draw(c, &fill());
                let on_window = canvas::draw(ROOT, &fill());
                format!("canvas={on_canvas:?} text={on_text:?} dead={on_dead:?} window={on_window:?}")
            }
            "pictures" => {
                let good = ui::define_image("good", PIC);
                let bad = ui::define_image("bad", b"GIF89a, not a PNG");
                // The refused content does not take the place of a picture.
                let again = ui::define_image("good", b"nor is this");
                let image = ui::create(WidgetKind::Image, ROOT, Rect::new(0, 60, 10, 10));
                ui::set_image(image, "good");
                ui::set_image(image, "bad");
                format!("good={good:?} bad={bad:?} again={again:?}")
            }
            "show" => format!("{:?}", runtime::show_app(arg)),
            "send" => format!("{:?}", events::send(arg, "hello", "there")),
            "lamps" => lamps::names().join(","),
            "long-key" => {
                let key = "k".repeat(arg.parse().unwrap_or(0));
                format!("{:?}", storage::set(&key, b"v"))
            }
            "play" => format!("{:?}", audio::play(TONE, 0)),
            "stop" => {
                audio::stop();
                "stopped".to_string()
            }
            "preview" => {
                alarms::preview((!arg.is_empty()).then_some(arg));
                "previewed".to_string()
            }
            "hangup" => format!("{:?}", calls::hangup(arg.parse().unwrap_or(0))),
            "answer" => format!("{:?}", calls::answer(arg.parse().unwrap_or(0))),
            "leave" => {
                runtime::background();
                "left".to_string()
            }
            other => format!("unknown: {other}"),
        }
    }
}

impl App for Checks {
    fn start(&mut self) {
        self.out = ui::create(WidgetKind::Text, ROOT, Rect::new(0, 0, 100, 20));
        self.seen = ui::create(WidgetKind::Text, ROOT, Rect::new(0, 20, 100, 20));
        // Stay loaded when off the screen.
        runtime::set_exit_after_background(false);
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Message((what, arg)) => {
                let answer = self.ask(&what, &arg);
                ui::set_text(self.out, &answer);
                return;
            }
            Event::Call(c) => self.log.push(format!("call {} {}", c.id, state(c.state))),
            Event::Push(p) => self.log.push(format!(
                "push {} kind={:?} data={:?} method={} call={:?}",
                p.name, p.kind, p.data, p.method, p.call
            )),
            Event::PlaybackStopped(at) => self.log.push(format!("stopped {at}")),
            _ => return,
        }
        ui::set_text(self.seen, &self.log.join("; "));
    }
}

dphone_app_sdk::export_app! {
    app: Checks,
    name: "checks",
    id: "test.checks",
    display_name: "Checks",
}
