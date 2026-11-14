//! Side-by-side comparison against stock firmware screenshots.
//!
//! The stock screenshots are not in this repository.  Point
//! `DPHONE_STOCK_SCREENSHOTS` at the reverse-engineering notes directory
//! (the one holding `screenshots/` and `docs/assets/`) to run this; without
//! it the test is skipped.  For each screen with a stock reference it
//! renders the same state through the HAL simulator and writes
//! `<skin>-<screen>-compare.png` (stock, ours, amplified difference) under
//! `DPHONE_UI_COMPARE_DIR` (default `target/ui-compare`).

use dphone_hal::{Hal, Screen};
use dphone_state::*;
use dphone_ui::{Ui, UiConfig};
use std::path::{Path, PathBuf};

struct Image {
    w: usize,
    h: usize,
    rgb: Vec<u8>,
}

fn load(path: &Path) -> Image {
    let mut dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    let n = info.color_type.samples();
    let (w, h) = (info.width as usize, info.height as usize);
    let mut rgb = Vec::with_capacity(w * h * 3);
    for p in buf[..w * h * n].chunks_exact(n) {
        // Alpha is ignored: some captures carry a zeroed alpha channel.
        match n {
            1 | 2 => rgb.extend_from_slice(&[p[0]; 3]),
            _ => rgb.extend_from_slice(&p[..3]),
        }
    }
    Image { w, h, rgb }
}

fn ours(frame: &dphone_hal::Frame) -> Image {
    let (w, h) = (frame.info.width as usize, frame.info.height as usize);
    let bpp = frame.info.format.bytes_per_pixel();
    let mut rgb = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let i = y * frame.info.stride + x * bpp;
            if bpp == 4 {
                rgb.extend_from_slice(&[frame.pixels[i + 2], frame.pixels[i + 1], frame.pixels[i]]);
            } else {
                rgb.extend_from_slice(&[frame.pixels[i]; 3]);
            }
        }
    }
    Image { w, h, rgb }
}

/// Stock | ours | difference (amplified), scaled by `scale`.
fn write_compare(path: &Path, stock: &Image, ours: &Image, scale: usize) {
    assert_eq!((stock.w, stock.h), (ours.w, ours.h), "{}", path.display());
    let (w, h) = (stock.w, stock.h);
    let gap = 4;
    let out_w = (3 * w + 2 * gap) * scale;
    let mut out = vec![0x80u8; out_w * h * scale * 3];
    for y in 0..h * scale {
        for x in 0..w * scale {
            let i = ((y / scale) * w + x / scale) * 3;
            for (panel, px) in [
                [stock.rgb[i], stock.rgb[i + 1], stock.rgb[i + 2]],
                [ours.rgb[i], ours.rgb[i + 1], ours.rgb[i + 2]],
                [0, 1, 2].map(|k| (stock.rgb[i + k].abs_diff(ours.rgb[i + k]) as u32 * 3).min(255) as u8),
            ]
            .into_iter()
            .enumerate()
            {
                let o = (y * out_w + panel * (w + gap) * scale + x) * 3;
                out[o..o + 3].copy_from_slice(&px);
            }
        }
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut enc = png::Encoder::new(file, out_w as u32, (h * scale) as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&out).unwrap();
}

struct Rig {
    model: Model,
    store: Store,
    ui: Ui,
    sim: dphone_hal::sim::SimHandle,
    stock: PathBuf,
    out: PathBuf,
}

/// 2026-11-12 18:15:00 UTC.
const UNIX: u64 = 1_794_507_300;
const MONO: u64 = 5_000_000;

impl Rig {
    fn new(model: Model, stock: &Path, out: &Path, unix: u64) -> Rig {
        let mut hal = Hal::sim(model);
        let sim = hal.sim.clone().unwrap();
        let store = Store::new(model);
        store.update::<System, _>(|s| {
            s.phase = Phase::Running;
            s.time_synced = true;
        });
        store.update::<Network, _>(|n| n.phase = NetPhase::Up);
        let mut ui = Ui::from_hal(&mut hal, store.clone(), UiConfig::default());
        ui.set_fixed_time(Some((MONO, unix)));
        Rig { model, store, ui, sim, stock: stock.to_path_buf(), out: out.to_path_buf() }
    }

    fn account(&mut self, label: &str) {
        self.store.update::<Accounts, _>(|a| {
            let mut acct = Account::new(AccountId(0));
            acct.label = label.into();
            acct.user = "020".into();
            acct.registration = Registration::Registered { host: ActiveHost::Primary, expires_mono_ms: MONO + 60_000 };
            a.accounts.push(acct);
            a.default_account = Some(AccountId(0));
        });
        self.store.update::<Keys, _>(|k| {
            k.line_keys.push(LineKey {
                index: 0,
                binding: LineKeyBinding::Account(AccountId(0)),
                label: String::new(),
                lamp: LampState::OFF,
                calls: vec![],
            })
        });
    }

    fn call(&mut self, direction: Direction, state: CallState, name: &str, number: &str, secs: Option<u64>) {
        let id = self.store.next_call_id();
        self.store.update::<Calls, _>(|c| {
            c.calls.clear();
            let mut call = Call::new(id, AccountId(0), direction, Party { name: name.into(), number: number.into(), uri: String::new() });
            call.state = state;
            call.connected_mono_ms = secs.map(|s| MONO - s * 1000);
            c.calls.push(call);
            c.focused = Some(id);
        });
        self.store.update::<Keys, _>(|k| {
            if let Some(key) = k.line_keys.first_mut() {
                key.calls = vec![id];
            }
        });
    }

    fn compare(&mut self, stock_file: &str, name: &str) {
        self.ui.pump();
        let frame = self.sim.frame(Screen::Main).unwrap();
        let stock = load(&self.stock.join(stock_file));
        let skin = match self.model.skin() {
            Skin::Color => "color",
            Skin::LargeGray => "large-gray",
            Skin::SmallGray => "small-gray",
        };
        let scale = if self.model.is_color() { 1 } else { 2 };
        write_compare(&self.out.join(format!("{skin}-{name}-compare.png")), &stock, &ours(&frame), scale);
    }

    fn show(&mut self, screen: &str) {
        let p = self.store.send(Command::ShowScreen { name: screen.into() });
        self.ui.pump();
        p.wait().unwrap();
    }
}

#[test]
fn compare_with_stock() {
    let Some(stock) = std::env::var_os("DPHONE_STOCK_SCREENSHOTS").map(PathBuf::from) else {
        eprintln!("skipped: set DPHONE_STOCK_SCREENSHOTS to the directory holding the stock screenshots");
        return;
    };
    let out = std::env::var_os("DPHONE_UI_COMPARE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-compare"));
    std::fs::create_dir_all(&out).unwrap();

    // Color: the owner's D60 captures.
    let mut r = Rig::new(Model::D60, &stock, &out, UNIX);
    r.account("Jack's Desk (020)");
    r.compare("screenshots/d60-stock-idle-home.png", "idle");
    r.store.update::<CallLog, _>(|l| l.missed_unseen = 1);
    r.ui.set_fixed_time(Some((MONO, UNIX + 19 * 60)));
    r.compare("screenshots/d60-stock-idle-missed-call.png", "idle-missed");
    r.store.update::<CallLog, _>(|l| l.missed_unseen = 0);
    r.call(Direction::Incoming, CallState::Ringing, "Bo's Desk", "021", None);
    r.compare("screenshots/d60-stock-incoming-call.png", "call-incoming");
    r.call(Direction::Incoming, CallState::Active, "Bo's Desk", "021", Some(13));
    r.compare("screenshots/d60-stock-call-connected.png", "call-connected");
    r.call(Direction::Outgoing, CallState::Dialing, "", "021", None);
    r.compare("screenshots/d60-stock-dialing-021.png", "call-dialing");
    r.store.update::<Calls, _>(|c| c.calls.clear());
    r.store.update::<Keys, _>(|k| k.line_keys[0].calls.clear());
    r.ui.pump();
    r.sim.set_hook(true);
    r.compare("screenshots/d60-stock-handset-offhook-dialtone.png", "dialer");
    r.sim.set_hook(false);
    r.ui.pump();
    r.show("about");
    r.compare("docs/assets/ui-D60-display-settings.png", "about");

    // Gray: the per-model renders of the stock binaries.
    for (model, tag) in [(Model::D40, "D40"), (Model::D70, "D70")] {
        // The stock fixture's clock: 2015-11-09 09:41 (only the layout is
        // compared; stock shows a logo where our clock is).
        let mut r = Rig::new(model, &stock, &out, UNIX);
        r.account("");
        r.compare(&format!("docs/assets/ui-{tag}-home.png"), "idle");
        r.call(Direction::Incoming, CallState::Ringing, "Alice Example", "1002", None);
        r.compare(&format!("docs/assets/ui-{tag}-incoming-call.png"), "call-incoming");
        r.call(Direction::Incoming, CallState::Active, "Alice Example", "1002", Some(1));
        r.compare(&format!("docs/assets/ui-{tag}-connected-call.png"), "call-connected");
        r.store.update::<Calls, _>(|c| c.calls.clear());
        r.ui.pump();
        r.show("about");
        r.compare(&format!("docs/assets/ui-{tag}-display-settings.png"), "about");
        if model == Model::D40 {
            r.show("menu");
            r.compare("docs/assets/ui-D40-preferences.png", "menu");
        }
    }
}
