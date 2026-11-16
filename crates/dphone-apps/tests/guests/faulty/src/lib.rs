//! A test guest that misbehaves on request: softkey 1 never returns,
//! softkey 2 allocates without bound, softkey 3 exercises storage and
//! reports what happened, softkey 4 sets a timer.

use dphone_app_sdk::prelude::*;

#[derive(Default)]
struct Faulty {
    out: Widget,
    ticks: u32,
}

fn storage_report() -> String {
    let r = [
        format!("set={}", storage::set("a.txt", b"hello").is_ok() && storage::set("b", b"2").is_ok()),
        format!("get={}", storage::get("a.txt").is_some_and(|d| d == b"hello")),
        format!("keys={}", storage::keys().join(",")),
        format!("escape={}", storage::set("../b.txt", b"x").is_ok()),
        format!("slash={}", storage::get("a/b").is_some()),
        format!("dot={}", storage::set(".hidden", b"x").is_ok()),
        format!("remove={}", storage::remove("a.txt").is_ok()),
        format!("gone={}", storage::get("a.txt").is_none()),
        format!("again={}", storage::remove("a.txt").is_ok()),
    ];
    r.join(" ")
}

impl App for Faulty {
    fn start(&mut self) {
        self.out = ui::create(WidgetKind::Text, ROOT, Rect::new(0, 0, 100, 20));
        ui::set_text(self.out, "ready");
        ui::set_softkeys(
            ROOT,
            &[
                Softkey::new(1, "Spin"),
                Softkey::new(2, "Hog"),
                Softkey::new(3, "Store"),
                Softkey::new(4, "Timer"),
            ],
        );
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Softkey(1) => {
                let mut x = 0u32;
                loop {
                    x = x.wrapping_add(1);
                    core::hint::black_box(x);
                }
            }
            Event::Softkey(2) => {
                let mut hoard: Vec<Vec<u8>> = Vec::new();
                loop {
                    hoard.push(vec![1u8; 1 << 20]);
                }
            }
            Event::Softkey(3) => ui::set_text(self.out, &storage_report()),
            Event::Softkey(4) => {
                let _ = timers::set(20, false);
            }
            Event::Timer(_) => {
                self.ticks += 1;
                ui::set_text(self.out, &format!("ticks={}", self.ticks));
            }
            _ => {}
        }
    }
}

dphone_app_sdk::export_app! {
    app: Faulty,
    name: "faulty",
    id: "test.faulty",
    display_name: "Faulty",
}
