//! Pins the geometry measured from the stock firmware screenshots: the
//! line-key tab rows (each label beside its physical key), the softkey
//! extents (each label above its key, with the pointed shape on color),
//! the header band, and the idle clock box.  If one of these moves, the
//! labels no longer line up with the keys on the real phone.

use dphone_hal::Hal;
use dphone_state::*;
use dphone_ui::gfx::IRect;
use dphone_ui::skin::Theme;
use dphone_ui::{Ui, UiConfig};

/// 2026-11-12 18:15:00 UTC: "6:15 PM", "Thu, Nov 12".
const UNIX: u64 = 1_794_507_300;

fn idle(model: Model, accounts: u8) -> Ui {
    let mut hal = Hal::sim(model);
    let store = Store::new(model);
    store.update::<System, _>(|s| {
        s.phase = Phase::Running;
        s.time_synced = true;
    });
    store.update::<Network, _>(|n| n.phase = NetPhase::Up);
    store.update::<Accounts, _>(|a| {
        for i in 0..accounts {
            let mut acct = Account::new(AccountId(i));
            acct.label = format!("10{i}");
            acct.registration = Registration::Registered { host: ActiveHost::Primary, expires_mono_ms: u64::MAX };
            a.accounts.push(acct);
        }
    });
    let mut ui = Ui::from_hal(&mut hal, store, UiConfig::default());
    ui.set_fixed_time(Some((5_000_000, UNIX)));
    ui.pump();
    ui
}

#[test]
fn measured_geometry_per_skin() {
    // (model, tab x and width, tab pitch, header height, softkey top and
    // height, first softkey x, pitch, width)
    let table = [
        (Model::D40, (155, 85), 23, 15, (103, 17), (0, 60, 59)),
        (Model::D45, (155, 85), 23, 15, (103, 17), (0, 60, 59)),
        (Model::D50, (155, 85), 23, 15, (103, 17), (0, 60, 59)),
        (Model::D70, (206, 114), 23, 17, (141, 19), (0, 80, 79)),
        (Model::D60, (296, 184), 40, 40, (244, 25), (14, 116, 104)),
        (Model::D62, (296, 184), 40, 40, (244, 25), (14, 116, 104)),
        (Model::D65, (296, 184), 40, 40, (244, 25), (14, 116, 104)),
    ];
    for (model, (tab_x, tab_w), pitch, header, (soft_y, soft_h), (x0, soft_pitch, soft_w)) in table {
        let th = Theme::for_model(model);
        for i in 0..model.line_keys() as usize {
            // Tab i starts at i * pitch from the very top of the screen.
            assert_eq!(th.tab_rect(i), IRect::new(tab_x, i as i32 * pitch, tab_w, pitch), "{model} tab {i}");
            // Every line key's tab fits above the softkeys.
            assert!(th.tab_rect(i).bottom() <= soft_y, "{model} tab {i}");
        }
        assert_eq!(tab_x + tab_w, th.width, "{model}: tabs reach the right edge");
        assert_eq!(th.header_rect().h, header, "{model}");
        for i in 0..4 {
            assert_eq!(th.softkey_rect(i), IRect::new(x0 + i as i32 * soft_pitch, soft_y, soft_w, soft_h), "{model} softkey {i}");
        }
        assert!(th.softkey_rect(3).right() <= th.width && soft_y + soft_h <= th.height, "{model}");
        // The app content area is the stock one.
        assert_eq!(th.content_h() as u16, model.app_content_area().height, "{model}");
    }
}

#[test]
fn color_idle_matches_stock_rows() {
    let ui = idle(Model::D60, 2);
    let c = ui.canvas();
    let th = Theme::for_model(Model::D60);
    // Line-key tabs: gray fill at x 297..479, rows 0..38 and 40..78; black
    // left edge at x 296 and bottom edges at y 39 and 79.
    for top in [0, 40] {
        for y in [top, top + 20, top + 35] {
            assert_eq!(c.pixel(296, y), 0x000001, "left edge at y {y}");
            assert_eq!(c.pixel(297, y), th.tab_bg, "fill at y {y}");
            assert_eq!(c.pixel(479, y), th.tab_bg, "fill reaches the right edge at y {y}");
            assert_ne!(c.pixel(295, y), th.tab_bg, "nothing left of the tab at y {y}");
        }
        assert_eq!(c.pixel(400, top + 39), 0x000001, "bottom edge");
        // The lower-left corner is cut.
        assert_ne!(c.pixel(296, top + 38), th.tab_bg);
        assert_ne!(c.pixel(296, top + 39), 0x000001);
    }
    // No third tab on a two-key phone, and no status bar: wallpaper.
    assert_ne!(c.pixel(400, 90), th.tab_bg);
    assert_ne!(c.pixel(100, 5), th.title_bg);

    // Softkeys: outline at x 14 and 117 (+116 per key), top at y 244, the
    // point at the center reaching y 268.
    for i in 0..4 {
        let x = 14 + 116 * i;
        assert_eq!(c.pixel(x, 250), th.sk_edge, "key {i} left edge");
        assert_eq!(c.pixel(x + 103, 250), th.sk_edge, "key {i} right edge");
        assert_eq!(c.pixel(x + 1, 247), th.sk_bg, "key {i} fill");
        assert_ne!(c.pixel(x - 1, 250), th.sk_edge, "key {i}: nothing left of it");
        assert_ne!(c.pixel(x + 104, 250), th.sk_edge, "key {i}: nothing right of it");
        assert_eq!(c.pixel(x + 50, 244), th.sk_edge, "key {i} top edge");
        assert_ne!(c.pixel(x + 50, 243), th.sk_edge, "key {i}: nothing above it");
        // The point: filled at the center two rows above the tip, empty
        // at the same row near the corner, and nothing below y 268.
        assert_eq!(c.pixel(x + 52, 266), th.sk_bg, "key {i} point");
        assert_ne!(c.pixel(x + 52, 268), c.pixel(x + 52, 270), "key {i} tip at y 268");
        assert_ne!(c.pixel(x + 5, 266), th.sk_bg, "key {i}: the corner is cut away");
        assert_eq!(c.pixel(x + 52, 269), c.pixel(x - 2, 269), "key {i}: nothing below the tip");
    }

    // The clock: stock digits occupy x 68..222, y 89..152; the date
    // x 92..203, y 180..199.  Allow three pixels for the rasterizer.
    let bbox = |x0: i32, y0: i32, x1: i32, y1: i32| {
        let (mut l, mut t, mut r, mut b) = (i32::MAX, i32::MAX, 0, 0);
        for y in y0..y1 {
            for x in x0..x1 {
                if (c.pixel(x, y) >> 8) & 255 > 150 {
                    (l, t, r, b) = (l.min(x), t.min(y), r.max(x), b.max(y));
                }
            }
        }
        (l, t, r, b)
    };
    let close = |got: (i32, i32, i32, i32), want: (i32, i32, i32, i32), what: &str| {
        for (g, w) in [(got.0, want.0), (got.1, want.1), (got.2, want.2), (got.3, want.3)] {
            assert!((g - w).abs() <= 3, "{what}: got {got:?}, stock {want:?}");
        }
    };
    close(bbox(50, 60, 235, 165), (68, 89, 222, 152), "clock digits");
    close(bbox(236, 125, 290, 165), (242, 136, 270, 151), "meridiem");
    close(bbox(60, 170, 290, 205), (92, 180, 203, 199), "date");
    // Nothing is printed between the date and the softkeys (stock shows
    // nothing there; no identity line, no branding).
    let (l, ..) = bbox(0, 206, 296, 243);
    assert_eq!(l, i32::MAX, "text under the date");
}

/// The rows are stock's.  Two widths are not: the idle screen's tabs start
/// as far left as the clock allows (stock: x 155 and 206), and a softkey
/// fills its cell but for the last column (stock: a white column at each
/// side).
#[test]
fn gray_idle_rows() {
    for (model, tab_x, soft_y, pitch) in [(Model::D40, 114, 103, 60), (Model::D50, 114, 103, 60), (Model::D70, 160, 141, 80)] {
        let ui = idle(model, 2);
        let c = ui.canvas();
        let (w, h) = (c.width(), c.height());
        const BLACK: u32 = 0x000000;
        const WHITE: u32 = 0xFFFFFF;
        let header = Theme::for_model(model).header_h;
        // Header bar: black up to x tab_x - 2, a white gap, then the tab's
        // border at tab_x; white below the bar.
        assert_eq!(c.pixel(tab_x - 2, header - 1), BLACK, "{model}");
        assert_eq!(c.pixel(tab_x - 1, 2), WHITE, "{model}");
        assert_eq!(c.pixel(tab_x - 20, header), WHITE, "{model}");
        // Tabs at rows 0..22 and 23..45: left border, white inside, bottom
        // border on the last row.
        for top in [0, 23] {
            assert_eq!(c.pixel(tab_x, top + 3), BLACK, "{model} tab at {top}");
            assert_eq!(c.pixel(tab_x + 1, top + 3), WHITE, "{model} tab at {top}");
            assert_eq!(c.pixel(w - 30, top + 22), BLACK, "{model} tab at {top}");
            assert_eq!(c.pixel(w - 30, top + 21), WHITE, "{model} tab at {top}");
        }
        assert_eq!(Theme::for_model(model).idle_tab_x, tab_x, "{model}");
        // The clock, at its widest, stays clear of the tabs.
        let th = Theme::for_model(model);
        let clock = th.clock_font().width("12:58") + 3 + th.date_font().width("PM");
        assert!(clock + 8 <= tab_x, "{model}: clock {clock}");
        // Softkeys: black cells from soft_y to the bottom with the top
        // corners cut, one white column between two cells, white above.
        for i in 0..4 {
            let x = i * pitch;
            assert_eq!(c.pixel(x, soft_y), WHITE, "{model} key {i}");
            assert_eq!(c.pixel(x, soft_y + 3), BLACK, "{model} key {i}");
            assert_eq!(c.pixel(x + pitch - 2, soft_y + 3), BLACK, "{model} key {i}");
            assert_eq!(c.pixel(x + pitch - 1, soft_y + 8), WHITE, "{model} key {i}");
            assert_eq!(c.pixel(x + pitch / 2, soft_y), BLACK, "{model} key {i}");
            assert_eq!(c.pixel(x + pitch / 2, soft_y - 1), WHITE, "{model} key {i}");
            assert_eq!(c.pixel(x + 3, h - 1), BLACK, "{model} key {i}");
        }
    }
}

#[test]
fn branding_only_on_the_boot_screen() {
    // The wordmark and splash artwork are not in the icon tables under any
    // name a non-boot screen uses, and the idle screens were checked above
    // for stray text; here: the icon table has exactly one piece of art.
    let names = dphone_ui::icons::names();
    assert!(names.contains(&"splash"));
    assert!(!names.contains(&"wordmark"));
}

#[test]
fn boot_screen_has_a_loading_bar_and_no_softkeys() {
    // (model, the loading bar's rectangle)
    for (model, bar) in [
        (Model::D40, IRect::new(60, 69, 120, 7)),
        (Model::D50, IRect::new(60, 69, 120, 7)),
        (Model::D70, IRect::new(80, 94, 160, 9)),
        (Model::D60, IRect::new(120, 180, 240, 8)),
        (Model::D65, IRect::new(120, 180, 240, 8)),
    ] {
        let th = Theme::for_model(model);
        let mut hal = Hal::sim(model);
        let store = Store::new(model);
        let mut ui = Ui::from_hal(&mut hal, store.clone(), UiConfig::default());
        ui.set_fixed_time(Some((5_000_000, UNIX)));
        ui.pump();
        assert_eq!(ui.screen_name(), "boot", "{model}");

        // No softkey shapes: where the idle screen has them, the boot
        // screen has only its background.
        let mid = bar.y + bar.h / 2;
        let background = if th.gray { 0xFFFFFF } else { ui.canvas().pixel(bar.x - 6, mid) };
        for i in 0..4 {
            let r = th.softkey_rect(i);
            for (x, y) in [(r.x + r.w / 2, r.y + 3), (r.x + 2, r.y + 3), (r.x + r.w / 2, r.y)] {
                let p = ui.canvas().pixel(x, y);
                assert!(p != th.sk_bg && p != th.sk_edge, "{model} softkey {i} at {x},{y}: {p:06x}");
                if th.gray {
                    assert_eq!(p, background, "{model} softkey {i} at {x},{y}");
                }
            }
        }

        // The bar: empty before any stage has completed, and (on gray)
        // outlined.  `filled(permille)` looks just inside that part of it.
        let filled = |ui: &Ui, permille: i32| {
            let x = bar.x + 2 + (bar.w - 4) * permille / 1000 - 2;
            ui.canvas().pixel(x, mid) == th.load_fill
        };
        assert!(!filled(&ui, 50), "{model}");
        if th.gray {
            assert_eq!(ui.canvas().pixel(bar.x + bar.w / 2, bar.y), th.fg, "{model}");
            assert_eq!(ui.canvas().pixel(bar.x, bar.y), background, "{model}: the corner pixel is left out");
            assert_eq!(ui.canvas().pixel(bar.x + bar.w / 2, bar.y + 1), background, "{model}");
        } else {
            assert_eq!(ui.canvas().pixel(bar.x + bar.w / 2, mid), th.load_track, "{model}");
            assert_ne!(ui.canvas().pixel(bar.x, bar.y), th.load_track, "{model}: the end is round");
        }

        // It advances as the stages complete, in the main role's order.
        store.set_health(Subsystem::Config, HealthStatus::Ok);
        store.set_health(Subsystem::Media, HealthStatus::Starting);
        ui.pump();
        assert!(filled(&ui, 100) && !filled(&ui, 150), "{model}");
        store.set_health(Subsystem::Media, HealthStatus::Failed("no DSP".into()));
        store.set_health(Subsystem::Sip, HealthStatus::Ok);
        ui.pump();
        assert!(filled(&ui, 750) && !filled(&ui, 800), "{model}");
        // It never moves backward.
        store.set_health(Subsystem::Media, HealthStatus::Starting);
        ui.pump();
        assert!(filled(&ui, 750), "{model}");
        store.set_health(Subsystem::Media, HealthStatus::Ok);
        store.set_health(Subsystem::Apps, HealthStatus::Ok);
        store.set_health(Subsystem::Web, HealthStatus::Ok);
        ui.pump();
        assert!(filled(&ui, 950) && !filled(&ui, 1000), "{model}");

        // Shutting down shows no bar.
        store.update::<System, _>(|s| s.phase = Phase::ShuttingDown);
        ui.pump();
        assert_eq!(ui.screen_name(), "boot", "{model}");
        assert!(!filled(&ui, 100), "{model}");

        // The end of start-up leaves the screen.
        store.update::<System, _>(|s| s.phase = Phase::Running);
        ui.pump();
        assert_eq!(ui.screen_name(), "idle", "{model}");
    }
}

#[test]
fn gray_text_is_bitmap_with_a_fallback_font() {
    use dphone_ui::appview::Font;
    for model in [Model::D40, Model::D70] {
        let th = Theme::for_model(model);
        for role in [Font::Normal, Font::Bold, Font::Small, Font::Title, Font::Large] {
            let f = th.font(role);
            assert_eq!(f.bits, 1, "{model} {role:?}: a bitmap font");
            // Latin text with accents comes from the face itself.
            for ch in "Zoë Çelik Łukasz Ðặng…".chars() {
                let (from, _) = f.glyph_or_fallback(ch).unwrap();
                assert!(std::ptr::eq(from, f), "{model} {role:?}: {ch:?}");
            }
            // Greek and Cyrillic come from the fallback font, and a
            // character neither has is the replacement character.
            let fallback = f.fallback.expect("a fallback font");
            for ch in "Жанна Ωέγα".chars().filter(|c| *c != ' ') {
                let (from, g) = f.glyph_or_fallback(ch).unwrap();
                assert!(std::ptr::eq(from, fallback) && g.ch as u32 == ch as u32, "{model} {role:?}: {ch:?}");
            }
            let (from, g) = f.glyph_or_fallback('漢').unwrap();
            assert!(std::ptr::eq(from, fallback) && g.ch == 0xFFFD, "{model} {role:?}");
            // The fallback's line fits in the face's.
            assert!(fallback.ascent <= f.ascent && fallback.descent <= f.descent, "{model} {role:?}");
            // Digits have one width.
            let w = f.width("0");
            assert!("123456789".chars().all(|d| f.width(&d.to_string()) == w), "{model} {role:?}");
        }
        assert_eq!(th.clock_font().bits, 1, "{model}");
    }
    // The color skin keeps its anti-aliased outline text.
    let th = Theme::for_model(Model::D60);
    assert_eq!(th.font(Font::Normal).bits, 4);
    assert!(th.font(Font::Normal).fallback.is_none());
}
