//! A test guest that misbehaves on request: softkey 1 never returns,
//! softkey 2 allocates without bound, softkey 3 exercises storage and
//! reports what happened, softkey 4 sets a timer.

use app_sdk::prelude::*;

#[derive(Default)]
struct Faulty {
    out: Widget,
    ticks: u32,
}

fn storage_report() -> String {
    let mut r = Vec::new();
    r.push(format!(
        "write={}",
        storage::write(Location::Nv, "a.txt", b"hello").is_ok()
    ));
    r.push(format!(
        "read={}",
        storage::read(Location::Nv, "a.txt")
            .map(|d| d == b"hello")
            .unwrap_or(false)
    ));
    r.push(format!(
        "list={}",
        storage::list(Location::Nv)
            .map(|l| l.join(","))
            .unwrap_or_default()
    ));
    r.push(format!(
        "escape={}",
        storage::write(Location::Nv, "../b.txt", b"x").is_ok()
    ));
    r.push(format!(
        "slash={}",
        storage::read(Location::Tmp, "a/b").is_ok()
    ));
    r.push(format!(
        "rofs={}",
        storage::write(Location::App, "c.txt", b"x").is_ok()
    ));
    r.push(format!(
        "pkg={}",
        storage::exists(Location::App, "app.json")
    ));
    r.push(format!(
        "shared={}",
        storage::write(Location::Tmpshared, "s.txt", b"1").is_ok()
    ));
    r.push(format!(
        "remove={}",
        storage::remove(Location::Nv, "a.txt").is_ok()
    ));
    r.push(format!("gone={}", !storage::exists(Location::Nv, "a.txt")));
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
                timers::set(20, false);
            }
            Event::Timer(_) => {
                self.ticks += 1;
                ui::set_text(self.out, &format!("ticks={}", self.ticks));
            }
            _ => {}
        }
    }
}

app_sdk::export_app!(Faulty);
