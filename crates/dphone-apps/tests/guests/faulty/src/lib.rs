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
    use StorageLocation::Local;
    let done = |reply| match reply {
        StorageReply::Done(outcome) => outcome,
        StorageReply::Pending(_) => panic!("a local request is answered at once"),
    };
    let set = |key: &str, value: &[u8]| {
        matches!(done(storage::set(Local, key, value, None)), StorageOutcome::Stored(_))
    };
    let get = |key: &str| match done(storage::get(Local, key)) {
        StorageOutcome::Entry(e) => e.map(|e| e.value),
        _ => None,
    };
    let remove = |key: &str| matches!(done(storage::remove(Local, key, None)), StorageOutcome::Removed);
    let keys = || match done(storage::list(Local, "", "")) {
        StorageOutcome::Entries((entries, _)) => entries.into_iter().map(|e| e.key).collect(),
        _ => Vec::new(),
    };
    let r = [
        format!("set={}", set("a.txt", b"hello") && set("b", b"2")),
        format!("get={}", get("a.txt").is_some_and(|d| d == b"hello")),
        format!("keys={}", keys().join(",")),
        format!("escape={}", set("../b.txt", b"x")),
        format!("slash={}", get("a/b").is_some()),
        format!("dot={}", set(".hidden", b"x")),
        format!("remove={}", remove("a.txt")),
        format!("gone={}", get("a.txt").is_none()),
        format!("again={}", remove("a.txt")),
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
