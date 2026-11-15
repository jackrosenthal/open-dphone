//! The built-in wallpapers and their palettes: each wallpaper draws the
//! same picture every time; every palette's text can be read (measured
//! contrast of every pairing of roles that carries text or state, and of
//! the idle screen's text against the wallpaper behind it); the colors
//! with a meaning keep it; an unknown name is the default; a new
//! wallpaper re-themes every screen at once, apps included; and the
//! default wallpaper leaves every screen as it was.
//!
//! `contact_sheets` writes one picture per family, and one of them all,
//! of the idle screen and a menu over every wallpaper: into the directory
//! named by `WALLPAPER_SHEETS`, else into `wallpapers/` in cargo's test
//! scratch directory.

use dphone_hal::{Hal, Screen};
use dphone_state::*;
use dphone_state::{WallpaperName, WALLPAPERS};
use dphone_ui::appview::{AppEvent, AppView, Font, ThemeColor};
use dphone_ui::gfx::{Canvas, HAlign, IRect, Rgb};
use dphone_ui::skin::Theme;
use dphone_ui::wallpaper::{self, Family, Picture, Scheme, Wallpaper, HEIGHT, WIDTH};
use dphone_ui::{Ui, UiConfig};
use std::path::PathBuf;

/// 2026-11-12 18:15:00 UTC: "11:15 AM", "Thu, Nov 12" seven hours west.
const UNIX: u64 = 1_794_507_300;

/// The least contrast ratio text may have against what is behind it (the
/// WCAG figure for body text; `skin::TEXT_CONTRAST`).  The pairings of
/// roles and their limits are `Palette::pairings`.
const TEXT: f64 = 4.5;

/// Relative luminance of an sRGB color (WCAG 2).
fn luminance(c: Rgb) -> f64 {
    let lin = |v: u32| {
        let v = f64::from(v & 255) / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(c >> 16) + 0.7152 * lin(c >> 8) + 0.0722 * lin(c)
}

/// Contrast ratio of two colors (WCAG 2): 1 to 21.
fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

struct Rig {
    store: Store,
    ui: Ui,
    sim: dphone_hal::sim::SimHandle,
    config: CommandReceiver,
}

impl Rig {
    /// A D65 at its idle screen with status icons, a count, and two
    /// line-key tabs.
    fn new(cfg: UiConfig) -> Rig {
        Rig::with_model(Model::D65, cfg)
    }

    fn with_model(model: Model, cfg: UiConfig) -> Rig {
        let mut hal = Hal::sim(model);
        let sim = hal.sim.clone().unwrap();
        let store = Store::new(model);
        store.update::<System, _>(|s| {
            s.phase = Phase::Running;
            s.time_synced = true;
        });
        store.update::<Network, _>(|n| {
            n.phase = NetPhase::Up;
            n.lan.up = true;
        });
        store.update::<Accounts, _>(|a| {
            for (i, label) in ["Jordan 101", "Front Desk"].iter().enumerate() {
                let mut acct = Account::new(AccountId(i as u8));
                acct.label = label.to_string();
                acct.registration = Registration::Registered { host: ActiveHost::Primary, expires_mono_ms: u64::MAX };
                a.accounts.push(acct);
            }
        });
        store.update::<CallLog, _>(|l| l.missed_unseen = 2);
        store.update::<Features, _>(|f| f.dnd = true);
        let config = store.handle(Domain::Config);
        let mut ui = Ui::from_hal(&mut hal, store.clone(), UiConfig { utc_offset_secs: -7 * 3600, backlight_timeout_secs: 0, ..cfg });
        ui.set_fixed_time(Some((5_000_000, UNIX)));
        ui.pump();
        Rig { store, ui, sim, config }
    }

    fn named(wallpaper: &str) -> Rig {
        Rig::new(UiConfig { wallpaper: wallpaper.into(), ..UiConfig::default() })
    }

    fn tap(&mut self, key: Key) {
        self.sim.tap(key);
        self.ui.pump();
    }

    fn show(&mut self, screen: &str) {
        let p = self.store.send(Command::ShowScreen { name: screen.into() });
        self.ui.pump();
        assert_eq!(p.wait(), Ok(Reply::Done), "{screen}");
    }

    /// The main screen as `0x00RRGGBB`.
    fn frame(&mut self) -> Vec<u32> {
        self.ui.pump();
        let frame = self.sim.frame(Screen::Main).expect("main frame");
        if frame.info.format.bytes_per_pixel() == 4 {
            frame.pixels.as_chunks::<4>().0.iter().map(|p| u32::from_le_bytes(*p) & 0xFF_FFFF).collect()
        } else {
            frame.pixels.iter().map(|p| u32::from(*p) * 0x010101).collect()
        }
    }

    fn commands(&self) -> Vec<Command> {
        let mut v = Vec::new();
        while let Some(env) = self.config.try_recv() {
            v.push(env.command.clone());
            env.done();
        }
        v
    }
}

fn idle_frame(wallpaper: &str) -> Vec<u32> {
    Rig::named(wallpaper).frame()
}

/// The settings menu (a title bar, a list with its first row selected,
/// softkeys) with `wallpaper` selected.
fn menu_frame(wallpaper: &str) -> Vec<u32> {
    let mut r = Rig::named(wallpaper);
    r.show("settings");
    r.frame()
}

fn at(pixels: &[u32], x: i32, y: i32) -> Rgb {
    pixels[(y * WIDTH + x) as usize]
}

#[test]
fn the_list_is_well_formed() {
    let all = wallpaper::all();
    assert!((30..=45).contains(&all.len()), "{} wallpapers", all.len());
    assert_eq!(wallpaper::default().id, all[0].id);
    for (i, w) in all.iter().enumerate() {
        assert!(!w.id.is_empty() && w.id.bytes().all(|c| c.is_ascii_lowercase() || c == b'-'), "{}", w.id);
        assert!(!w.name.is_empty() && w.name.len() <= 14, "{}", w.name);
        assert!(all[..i].iter().all(|o| o.id != w.id && o.name != w.name), "{} is listed twice", w.id);
        assert_eq!(wallpaper::find(w.id).map(|f| f.id), Some(w.id));
    }
    // The list by name that the configuration and the web UI use is this
    // list.
    let names: Vec<WallpaperName> = all.iter().map(|w| WallpaperName { id: w.id, name: w.name, family: w.family.name() }).collect();
    assert_eq!(names, WALLPAPERS);
    // Grouped by family, in the families' order, none empty.
    let order: Vec<usize> = all.iter().map(|w| Family::ALL.iter().position(|f| *f == w.family).unwrap()).collect();
    assert!(order.is_sorted(), "{order:?}");
    for (i, f) in Family::ALL.iter().enumerate() {
        assert!(order.contains(&i), "no wallpaper in {}", f.name());
    }
}

/// The default wallpaper's palette is the color skin's own, role for
/// role, and its theme is the skin's theme itself: every screen is drawn
/// as it was before there were palettes.
#[test]
fn the_default_wallpaper_keeps_the_skins_palette() {
    let skin = Theme::for_model(Model::D65);
    let default = wallpaper::default();
    assert_eq!(default.scheme, Scheme::Skin);
    assert_eq!(default.palette(), skin.palette());
    assert!(std::ptr::eq(Theme::for_wallpaper(default), skin));
    // The stock values, by measurement (see `skin.rs`).
    let p = default.palette();
    assert_eq!((p.bg, p.fg, p.title_bg, p.sel_bg), (0x212121, 0xEEEEEE, 0x014174, 0x01579B));
    assert_eq!((p.sk_bg, p.sk_edge, p.tab_bg, p.card), (0x2E2E2E, 0x757575, 0x424242, 0x01579B));
    // A palette file's roles are these fields, by these names.
    let mut q = p;
    for (name, color) in p.roles() {
        assert!(q.set(name, color ^ 0xFFFFFF), "{name}");
    }
    assert!(!q.set("no_such_role", 0));
    assert!(p.roles().iter().zip(q.roles()).all(|(a, b)| a.0 == b.0 && a.1 == b.1 ^ 0xFFFFFF));
}

#[test]
fn an_unknown_name_is_the_default() {
    let default = wallpaper::default();
    for name in ["", "  ", "no-such-wallpaper", "harbor.png", "DUSK!"] {
        assert!(wallpaper::find(name).is_none(), "{name:?}");
        assert_eq!(wallpaper::find_or_default(name).id, default.id, "{name:?}");
    }
    // Case and surrounding blanks do not matter.
    assert_eq!(wallpaper::find(" Dusk ").map(|w| w.id), Some("dusk"));
    assert_eq!(wallpaper::find_or_default("DEEP-SEA").id, "deep-sea");
    // The screens, too: an unknown name shows the default wallpaper in
    // the default palette.
    assert_eq!(idle_frame("no-such-wallpaper"), idle_frame(default.id));
    assert_eq!(idle_frame(""), idle_frame(default.id));
    assert_eq!(menu_frame("no-such-wallpaper"), menu_frame(""));
}

fn fnv(pixels: &[u32]) -> u64 {
    pixels.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, p| (h ^ u64::from(*p)).wrapping_mul(0x0100_0000_01b3))
}

#[test]
fn every_wallpaper_draws_the_same_picture_every_time() {
    for w in wallpaper::all() {
        let first = w.render();
        assert_eq!(first.len(), (WIDTH * HEIGHT) as usize, "{}", w.id);
        assert!(first.iter().all(|p| *p <= 0xFF_FFFF), "{}", w.id);
        assert_eq!(first, w.render(), "{} differs between two renders", w.id);
        // The kept copy is the same picture, whichever was kept before.
        assert!(w.with_pixels(|kept| kept == first.as_slice()), "{}", w.id);
    }
}

/// The pictures themselves, for a few: a change to the drawing code that
/// alters them shows here.  (Integer arithmetic only, so the phone draws
/// the same pixels as this test.)
#[test]
fn pinned_pictures() {
    let pinned: [(&str, u64); 6] = [
        ("harbor", 0x9c561acb0ab4173c),
        ("dusk", 0x2ec2620abe6207e7),
        ("bokeh", 0x1ce6b3c47e9e552c),
        ("prism", 0xac26a85dfbc93341),
        ("summit", 0x27b556330a07588d),
        ("houndstooth", 0xa7b848ca8b7cf2a5),
    ];
    let mut wrong = Vec::new();
    for (id, want) in pinned {
        let got = fnv(&wallpaper::find(id).expect(id).render());
        if got != want {
            wrong.push(format!("(\"{id}\", {got:#018x})"));
        }
    }
    assert!(wrong.is_empty(), "pictures changed: {}", wrong.join(", "));
}

/// The first wallpaper is the picture the firmware always had.  The
/// pinned hash above and these pixels are of that picture as the code
/// before this one drew it (worked out from that code, separately).
#[test]
fn the_default_is_the_original_picture() {
    let p = wallpaper::default().render();
    for (x, y, want) in
        [(0, 0, 0x152031), (479, 0, 0x151E2C), (60, 0, 0x172846), (100, 100, 0x10274C), (300, 200, 0x0D2144), (0, 271, 0x202122)]
    {
        assert_eq!(at(&p, x, y), want, "at {x},{y}: {:06x}", at(&p, x, y));
    }
}

/// The least contrast of `fg` against the pixels of `r`.
fn least_contrast(pixels: &[u32], r: IRect, fg: Rgb) -> f64 {
    let mut least = f64::MAX;
    for y in r.y..r.bottom() {
        for x in r.x..r.right() {
            least = least.min(contrast(fg, at(pixels, x, y)));
        }
    }
    least
}

/// Where the idle screen draws text on the wallpaper itself: (element,
/// area).  The areas are the whole of where each can be, not the ink of
/// one particular time.
fn text_areas(th: &Theme) -> [(&'static str, IRect); 4] {
    [
        ("status icons", th.header_rect_to(th.idle_tab_x)),
        // Stock positions: digits x 68..222 (to 290 with the meridiem),
        // y 89..152.
        ("clock", IRect::new(30, 84, 266, 72)),
        ("date", IRect::new(30, 176, 236, 28)),
        ("app idle view", IRect::new(0, th.soft_y - 1 - 36, th.idle_tab_x, 36)),
    ]
}

fn hue(c: Rgb) -> &'static str {
    let (r, g, b) = ((c >> 16) as i32, ((c >> 8) & 255) as i32, (c & 255) as i32);
    if r > g + 40 && r > b + 40 && g < b + 60 {
        "red"
    } else if g > r + 20 && g > b + 20 {
        "green"
    } else if r > b + 60 && g > b + 30 {
        "amber"
    } else {
        "other"
    }
}

#[test]
fn every_palette_can_be_read() {
    let mut report = String::new();
    let mut failures = Vec::new();
    for w in wallpaper::all() {
        let th = Theme::for_wallpaper(w);
        assert_eq!(th.palette(), w.palette(), "{}", w.id);
        let pixels = w.render();
        let (mut worst, mut worst_what) = (f64::MAX, "");
        let mut check = |what: &'static str, ratio: f64, least: f64| {
            if ratio / least < worst {
                (worst, worst_what) = (ratio / least, what);
            }
            if ratio < least {
                failures.push(format!("{}: {what}: {ratio:.2}, under {least}", w.id));
            }
        };
        // The idle screen's text, on the wallpaper itself: against every
        // pixel it can be on.
        let mut idle = f64::MAX;
        for (what, area) in text_areas(th) {
            let ratio = least_contrast(&pixels, area, th.fg);
            idle = idle.min(ratio);
            check(what, ratio, TEXT);
        }
        // Every pairing of roles that carries text or a state.
        let pairings = th.palette().pairings();
        assert!(pairings.len() >= 22);
        for (what, fg, bg, least) in pairings {
            check(what, contrast(fg, bg), f64::from(least));
        }
        assert!(th.palette().low_contrast().is_empty(), "{}: {:?}", w.id, th.palette().low_contrast());
        // The colors with a meaning keep it.
        for (what, color, want) in [
            ("stop softkey", th.sk_stop, "red"),
            ("error", th.error, "red"),
            ("go softkey", th.sk_go, "green"),
            ("good", th.good, "green"),
            ("ringing call card", th.card_ringing, "green"),
            ("warning", th.warn, "amber"),
        ] {
            if hue(color) != want {
                failures.push(format!("{}: {what} is {color:06X}, not {want}", w.id));
            }
        }
        report.push_str(&format!("{:12} idle text {idle:5.2}; closest to its limit: {worst_what} ({:.2} x)\n", w.id, worst));
    }
    println!("{report}");
    assert!(failures.is_empty(), "palettes that cannot be read:\n{}", failures.join("\n"));
}

/// A palette file's palette: derived as the built-in ones are, role by
/// role where it says, the skin's own where it says nothing; and a
/// palette that cannot be read is reported, not refused.
#[test]
fn palettes_from_palette_files() {
    let skin = Theme::for_model(Model::D65).palette();
    // Nothing given: the skin's own palette.
    assert_eq!(wallpaper::palette_from(false, None, None, &[]), (skin, vec![]));
    // The two colors: the built-in rule.
    let evergreen = wallpaper::find("evergreen").unwrap();
    let Scheme::Dark { primary, surface } = evergreen.scheme else { panic!("evergreen is dark") };
    assert_eq!(wallpaper::palette_from(false, Some(primary), Some(surface), &[]).0, evergreen.palette());
    let stone = wallpaper::find("stone").unwrap();
    let Scheme::Light { primary, surface } = stone.scheme else { panic!("stone is light") };
    assert_eq!(wallpaper::palette_from(true, Some(primary), Some(surface), &[]).0, stone.palette());
    // One of the two: the other is the skin's.
    let p = wallpaper::palette_from(false, Some(0x1F6E4E), None, &[]).0;
    assert_eq!((p.sel_bg, p.bg, p.sk_bg), (0x1F6E4E, skin.bg, skin.sk_bg));
    let p = wallpaper::palette_from(false, None, Some(0x0F1E18), &[]).0;
    assert_eq!((p.sel_bg, p.bg), (skin.sel_bg, 0x0F1E18));
    let p = wallpaper::palette_from(true, None, None, &[]).0;
    assert!(p.is_light() && p.low_contrast().is_empty(), "{:?}", p.low_contrast());
    // Roles by name go over the derived colors; unknown names come back.
    let (p, unknown) = wallpaper::palette_from(false, Some(0x1F6E4E), Some(0x0F1E18), &[("title_bg", 0x14503A), ("sparkle", 1), ("accent", 0x8FD6B4)]);
    assert_eq!((p.title_bg, p.accent, p.sel_bg), (0x14503A, 0x8FD6B4, 0x1F6E4E));
    assert_eq!(unknown, ["sparkle"]);
    // Text that cannot be read is said, with the roles to look at.
    let (p, _) = wallpaper::palette_from(false, None, None, &[("fg", 0x303030), ("sel_bg", 0xF0F0F0)]);
    let low = p.low_contrast();
    assert!(low.iter().any(|l| l.starts_with("text (fg) on the background (bg): contrast 1.")), "{low:?}");
    assert!(low.iter().any(|l| l.starts_with("selected row (sel_fg on sel_bg)")), "{low:?}");
}

/// The same on the screens as drawn, for every wallpaper: the idle screen
/// and a menu are in the wallpaper's palette.
#[test]
fn every_screen_is_drawn_in_the_wallpapers_palette() {
    for w in wallpaper::all() {
        let th = Theme::for_wallpaper(w);
        let (frame, bare) = (idle_frame(w.id), w.render());
        // Left of the tabs and above the softkeys the idle screen is the
        // wallpaper with text on it: the pixels at full ink are the text
        // color, on wallpaper pixels it contrasts with.
        let (mut ink, mut least) = (0, f64::MAX);
        for y in 0..th.soft_y {
            for x in 0..th.idle_tab_x {
                if at(&frame, x, y) == th.fg && at(&bare, x, y) != th.fg {
                    ink += 1;
                    least = least.min(contrast(th.fg, at(&bare, x, y)));
                }
            }
        }
        assert!(ink > 400, "{}: {ink} pixels of text in its text color", w.id);
        assert!(least >= TEXT, "{}: text on a pixel with contrast {least:.2}", w.id);
        // A line-key tab: its fill and its label.
        let tab = th.tab_rect_from(th.idle_tab_x, 0);
        assert_eq!(at(&frame, tab.x + 5, tab.y + 3), th.tab_bg, "{}: tab fill", w.id);
        let label = (tab.y..tab.bottom()).flat_map(|y| (tab.x + 14..tab.right() - 40).map(move |x| (x, y)));
        assert!(label.filter(|(x, y)| at(&frame, *x, *y) == th.tab_fg).count() > 40, "{}: tab label", w.id);
        // A softkey: its fill, its outline, and its label.
        let key = th.softkey_rect(0);
        assert_eq!(at(&frame, key.x + 1, key.y + 3), th.sk_bg, "{}: softkey fill", w.id);
        assert_eq!(at(&frame, key.x, key.y + 6), th.sk_edge, "{}: softkey outline", w.id);
        let label = (key.y..key.bottom()).flat_map(|y| (key.x..key.right()).map(move |x| (x, y)));
        assert!(label.filter(|(x, y)| at(&frame, *x, *y) == th.sk_fg).count() > 40, "{}: softkey label", w.id);
        // Everything else is the wallpaper: under the tabs' rows and
        // between the softkeys.
        assert_eq!(at(&frame, 400, 200), at(&bare, 400, 200), "{}", w.id);
        assert_eq!(at(&frame, 4, 250), at(&bare, 4, 250), "{}", w.id);

        // A menu: the title bar, the selected row, the background, and
        // the softkeys.
        let menu = menu_frame(w.id);
        assert_eq!(at(&menu, 470, 5), th.title_bg, "{}: title bar", w.id);
        assert_eq!(at(&menu, 440, th.title_h + 8), th.sel_bg, "{}: selected row", w.id);
        assert_eq!(at(&menu, 440, 200), th.bg, "{}: menu background", w.id);
        assert_eq!(at(&menu, 4, 250), th.bg, "{}: beside the softkeys", w.id);
        assert_eq!(at(&menu, key.x + 1, key.y + 3), th.sk_bg, "{}: menu softkey", w.id);
        let count = |c: Rgb| menu.iter().filter(|p| **p == c).count();
        assert!(count(th.title_fg) > 40 && count(th.fg) > 100, "{}: menu text", w.id);
    }
}

/// A new wallpaper is a new palette at once, without a restart, on the
/// screen that is up and for apps: `theme_color` answers in the new
/// palette, and an app in the foreground is told to draw again.
#[test]
fn a_new_wallpaper_rethemes_at_once() {
    let mut r = Rig::named("");
    let handle = r.ui.handle();
    r.show("settings");
    let skin = Theme::for_model(Model::D65);
    assert_eq!(at(&r.frame(), 470, 5), skin.title_bg);
    let dusk = Theme::for_wallpaper(wallpaper::find("dusk").unwrap());
    assert_ne!(dusk.title_bg, skin.title_bg);
    handle.set_config(UiConfig { wallpaper: "dusk".into(), utc_offset_secs: -7 * 3600, backlight_timeout_secs: 0, ..UiConfig::default() });
    let frame = r.frame();
    assert_eq!(at(&frame, 470, 5), dusk.title_bg);
    assert_eq!(at(&frame, 440, dusk.title_h + 8), dusk.sel_bg);
    assert_eq!(frame, menu_frame("dusk"), "the same screen as after a start with that wallpaper");

    // An app: opened under the new palette, it gets that palette's colors.
    let rgb = |c: Rgb| ((c >> 16) as u8, (c >> 8) as u8, c as u8);
    let (view, events) = AppView::new("probe", Model::D65);
    assert_eq!(view.theme_color(ThemeColor::TitleBackground), rgb(skin.title_bg), "before it is on a screen");
    handle.open_app(view.clone());
    r.ui.pump();
    assert_eq!(r.ui.screen_name(), "app:probe");
    assert_eq!(view.theme_color(ThemeColor::TitleBackground), rgb(dusk.title_bg));
    assert_eq!(view.theme_color(ThemeColor::SelectBackground), rgb(dusk.sel_bg));
    while events.try_recv().is_some() {}
    // The palette changes under it: the colors follow, and the app is
    // told it is in the foreground (again), on which apps draw.
    let tides = Theme::for_wallpaper(wallpaper::find("tides").unwrap());
    handle.set_config(UiConfig { wallpaper: "tides".into(), utc_offset_secs: -7 * 3600, backlight_timeout_secs: 0, ..UiConfig::default() });
    r.ui.pump();
    assert_eq!(view.theme_color(ThemeColor::TitleBackground), rgb(tides.title_bg));
    assert_eq!(view.theme_color(ThemeColor::Background), rgb(tides.bg));
    assert_eq!(events.try_recv(), Some(AppEvent::Foreground));
    // The same configuration again changes nothing and tells no one.
    handle.set_config(UiConfig { wallpaper: "tides".into(), utc_offset_secs: -7 * 3600, backlight_timeout_secs: 0, ..UiConfig::default() });
    r.ui.pump();
    assert_eq!(events.try_recv(), None);
}

/// The wallpaper selector, in the Display menu on the color models: it
/// is the idle screen over the wallpaper the selection is on, in that
/// wallpaper's palette, with a bubble at the bottom that names it between
/// two arrows; Left and Right step through the wallpapers; Select saves.
#[test]
fn the_wallpaper_selector_is_the_idle_screen_with_a_bubble() {
    let mut r = Rig::named("");
    r.show("settings");
    r.tap(Key::Digit(3)); // Display
    assert_eq!(r.ui.screen_name(), "display");
    r.tap(Key::Digit(4)); // Wallpaper
    assert_eq!(r.ui.screen_name(), "wallpaper");
    let all = wallpaper::all();
    let skin = Theme::for_model(Model::D65);
    // The bubble: centered under the date, left of the tabs, above the
    // softkeys.
    let bubble = IRect::new(38, 210, 220, 28);
    // The frame is the idle screen over wallpaper `w`, pixel for pixel,
    // but for the bubble and the softkeys' labels (Select and Back here).
    let is_idle_with_bubble = |frame: &[u32], w: &'static Wallpaper| {
        let th = Theme::for_wallpaper(w);
        let idle = idle_frame(w.id);
        for y in 0..th.soft_y {
            for x in 0..WIDTH {
                let inside = x >= bubble.x && x < bubble.right() && y >= bubble.y && y < bubble.bottom();
                if !inside {
                    assert_eq!(at(frame, x, y), at(&idle, x, y), "{}: at {x},{y}", w.id);
                }
            }
        }
        // The softkeys are the idle screen's shapes in the same colors.
        for i in 0..4 {
            let key = th.softkey_rect(i);
            assert_eq!(at(frame, key.x + 1, key.y + 3), th.sk_bg, "{}: softkey {i}", w.id);
            assert_eq!(at(frame, key.x, key.y + 6), th.sk_edge, "{}: softkey {i}", w.id);
        }
        assert_eq!(at(frame, 4, 250), at(&idle, 4, 250), "{}: beside the softkeys", w.id);
        // The bubble: the softkeys' fill with their outline around it, the
        // name and the two arrows in their label color, and the wallpaper
        // at its corners, which are round.
        let mid = bubble.y + bubble.h / 2;
        assert_eq!(at(frame, bubble.x + 40, bubble.y + 3), th.sk_bg, "{}: bubble fill", w.id);
        assert_eq!(at(frame, bubble.x + 60, bubble.y), th.sk_edge, "{}: bubble outline", w.id);
        assert_eq!(at(frame, bubble.x, bubble.y), at(&idle, bubble.x, bubble.y), "{}: round corner", w.id);
        assert_eq!(at(frame, bubble.x + 17, mid), th.sk_fg, "{}: left arrow", w.id);
        assert_eq!(at(frame, bubble.right() - 18, mid), th.sk_fg, "{}: right arrow", w.id);
        // The arrows point outward: each is one pixel at its tip.
        assert_eq!(at(frame, bubble.x + 14, mid - 4), th.sk_bg, "{}: left arrow's tip", w.id);
        assert_eq!(at(frame, bubble.right() - 15, mid - 4), th.sk_bg, "{}: right arrow's tip", w.id);
        let name = (bubble.y..bubble.bottom()).flat_map(|y| (bubble.x + 30..bubble.right() - 30).map(move |x| (x, y)));
        assert!(name.filter(|(x, y)| at(frame, *x, *y) == th.sk_fg).count() > 60, "{}: the name", w.id);
    };
    assert_eq!(at(&idle_frame(""), 400, 200), at(&all[0].render(), 400, 200));
    is_idle_with_bubble(&r.frame(), &all[0]);
    // Right: the next wallpaper fills the screen, in its palette, at once.
    r.tap(Key::Right);
    is_idle_with_bubble(&r.frame(), &all[1]);
    assert_ne!(Theme::for_wallpaper(&all[1]).sk_bg, skin.sk_bg);
    // Left twice: back, and around the end to the last one.
    r.tap(Key::Left);
    is_idle_with_bubble(&r.frame(), &all[0]);
    r.tap(Key::Left);
    is_idle_with_bubble(&r.frame(), all.last().unwrap());
    r.tap(Key::Right);
    // Up and Down step too.
    r.tap(Key::Down);
    r.tap(Key::Down);
    is_idle_with_bubble(&r.frame(), &all[2]);
    r.tap(Key::Up);
    is_idle_with_bubble(&r.frame(), &all[1]);
    // Nothing is saved by looking; Back leaves the wallpaper as it was.
    assert!(r.commands().is_empty());
    r.tap(Key::Soft(4));
    assert_eq!(r.ui.screen_name(), "display");
    assert_eq!(at(&r.frame(), 470, 5), skin.title_bg);
    assert!(r.commands().is_empty());
    // Select saves the one that is showing.
    r.tap(Key::Digit(4));
    r.tap(Key::Right);
    r.tap(Key::Right);
    r.tap(Key::Soft(1));
    assert_eq!(r.commands(), vec![Command::SetWallpaper { name: all[2].id.into() }]);
    assert_eq!(r.ui.screen_name(), "display");
    // No family is named anywhere on the screen: nothing but the idle
    // screen and the bubble is drawn (checked above, pixel for pixel).
}

/// A wallpaper the provisioning server fixed can be looked at, not set.
#[test]
fn a_fixed_wallpaper_cannot_be_changed_in_the_menu() {
    let mut r = Rig::new(UiConfig { wallpaper: "slate".into(), wallpaper_locked: true, ..UiConfig::default() });
    r.show("settings");
    r.tap(Key::Digit(3));
    r.tap(Key::Digit(4));
    assert_eq!(r.ui.screen_name(), "wallpaper");
    r.tap(Key::Right);
    r.tap(Key::Soft(1));
    r.tap(Key::Select);
    assert_eq!(r.ui.screen_name(), "wallpaper");
    assert!(r.commands().is_empty());
}

/// A wallpaper picture that is not built in is shown instead of the
/// built-in one: in the skin's own palette when it has none of its own,
/// in its palette when it has.
#[test]
fn a_wallpaper_picture_and_its_palette() {
    // A picture of another size and shape: 96 x 96, the left half red,
    // the right half blue.  It is scaled to cover the screen.
    let pixels: Vec<u8> =
        (0..96 * 96).flat_map(|i| if i % 96 < 48 { [0x40, 0x10, 0x10, 255] } else { [0x10, 0x10, 0x40, 255] }).collect();
    let image = dphone_ui::image::Image { width: 96, height: 96, pixels: pixels.into() };
    let skin = Theme::for_model(Model::D65);
    let plain = std::sync::Arc::new(Picture::new(&image, None));
    assert_eq!(plain.pixels().len(), (WIDTH * HEIGHT) as usize);
    assert_eq!((at(plain.pixels(), 10, 10), at(plain.pixels(), 470, 260)), (0x401010, 0x101040));
    // It wins over a built-in wallpaper's name, palette and all.
    let mut r = Rig::new(UiConfig { wallpaper: "dusk".into(), wallpaper_picture: Some(plain), ..UiConfig::default() });
    let frame = r.frame();
    assert_eq!((at(&frame, 4, 250), at(&frame, 400, 200)), (0x401010, 0x101040));
    assert_eq!(at(&frame, skin.softkey_rect(0).x + 1, skin.soft_y + 3), skin.sk_bg);
    r.show("settings");
    assert_eq!(r.frame(), menu_frame(""), "the skin's own palette");
    // With a palette of its own.
    let mut palette = wallpaper::find("merlot").unwrap().palette();
    palette.title_bg = 0x123456;
    let themed = std::sync::Arc::new(Picture::new(&image, Some(palette)));
    let mut r = Rig::new(UiConfig { wallpaper_picture: Some(themed), ..UiConfig::default() });
    let frame = r.frame();
    assert_eq!(at(&frame, 4, 250), 0x401010);
    assert_eq!(at(&frame, skin.softkey_rect(0).x + 1, skin.soft_y + 3), palette.sk_bg);
    r.show("settings");
    let frame = r.frame();
    assert_eq!((at(&frame, 470, 5), at(&frame, 440, 200)), (0x123456, palette.bg));
}

/// Gray screens have no wallpaper and one palette: every screen is the
/// same whatever is selected, and the Display menu has no Wallpaper item.
#[test]
fn gray_models_are_unaffected() {
    for model in [Model::D40, Model::D45, Model::D50, Model::D70] {
        let image = dphone_ui::image::Image { width: 1, height: 1, pixels: vec![255, 0, 0, 255].into() };
        let picture = std::sync::Arc::new(Picture::new(&image, Some(wallpaper::find("sunrise").unwrap().palette())));
        let mut plain = Rig::with_model(model, UiConfig::default());
        let mut named = Rig::with_model(model, UiConfig { wallpaper: "sunrise".into(), ..UiConfig::default() });
        let mut pictured = Rig::with_model(model, UiConfig { wallpaper_picture: Some(picture), ..UiConfig::default() });
        assert_eq!(named.frame(), plain.frame(), "{model}: idle");
        assert_eq!(pictured.frame(), plain.frame(), "{model}: idle");
        for r in [&mut plain, &mut named, &mut pictured] {
            r.show("settings");
            r.tap(Key::Digit(3));
            assert_eq!(r.ui.screen_name(), "display");
        }
        assert_eq!(named.frame(), plain.frame(), "{model}: display menu");
        assert_eq!(pictured.frame(), plain.frame(), "{model}: display menu");
        // Brightness, Contrast, Dim when idle, Backlight timeout: a fifth
        // item would open on 5.
        named.tap(Key::Digit(5));
        assert_eq!(named.ui.screen_name(), "display", "{model}");
    }
}

// ---------------------------------------------------------- contact sheets

fn write_png(path: &std::path::Path, c: &Canvas) {
    let rgb: Vec<u8> = c.pixels().iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]).collect();
    let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut enc = png::Encoder::new(file, c.width() as u32, c.height() as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&rgb).unwrap();
}

/// A screen with a call on it, in `wallpaper`'s palette.
fn call_frame(wallpaper: &str) -> Vec<u32> {
    let mut r = Rig::named(wallpaper);
    let id = r.store.next_call_id();
    r.store.update::<Calls, _>(|c| {
        let mut call = Call::new(id, AccountId(0), Direction::Incoming, Party { number: "1002".into(), name: "Alex Example".into(), ..Default::default() });
        call.state = CallState::Ringing;
        c.calls.push(call);
        c.focused = Some(id);
    });
    r.frame()
}

/// A sheet of screens over `list`: for each wallpaper a row of the idle
/// screen, a menu, and (`with_call`) an incoming call, each at 1/`shrink`
/// size, `columns` wallpapers across, with the name under each.
fn sheet(title: &str, list: &[&'static Wallpaper], columns: usize, shrink: i32, with_call: bool) -> Canvas {
    let th = Theme::for_model(Model::D65);
    let (cw, ch) = (WIDTH / shrink, HEIGHT / shrink);
    let shots = if with_call { 3 } else { 2 };
    let (gap, label, head) = (12, 22, 34);
    let cell = shots * cw + (shots - 1) * 2;
    let rows = list.len().div_ceil(columns);
    let mut c = Canvas::new(
        (gap + columns as i32 * (cell + gap)) as u16,
        (head + rows as i32 * (ch + label + gap)) as u16,
        false,
    );
    c.clear(0x2B2B2B);
    c.text_in(th.font(Font::Title), IRect::new(gap, 0, 900, head), title, 0xFFFFFF, HAlign::Left);
    for (i, w) in list.iter().enumerate() {
        let (x0, y0) = (gap + (i % columns) as i32 * (cell + gap), head + (i / columns) as i32 * (ch + label + gap));
        let mut frames = vec![idle_frame(w.id), menu_frame(w.id)];
        if with_call {
            frames.push(call_frame(w.id));
        }
        for (k, frame) in frames.iter().enumerate() {
            let mut rgba = Vec::with_capacity((cw * ch * 4) as usize);
            for y in 0..ch {
                for x in 0..cw {
                    // The mean of each block.
                    let mut sum = [0u32; 3];
                    for sy in 0..shrink {
                        for sx in 0..shrink {
                            let p = at(frame, x * shrink + sx, y * shrink + sy);
                            sum[0] += p >> 16;
                            sum[1] += (p >> 8) & 255;
                            sum[2] += p & 255;
                        }
                    }
                    let n = (shrink * shrink) as u32;
                    rgba.extend_from_slice(&[(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8, 255]);
                }
            }
            c.rgba(x0 + k as i32 * (cw + 2), y0, cw, ch, &rgba);
        }
        let name = format!("{} ({})", w.name, w.id);
        c.text_in(th.font(Font::Normal), IRect::new(x0, y0 + ch, cell, label), &name, 0xDDDDDD, HAlign::Left);
    }
    c
}

#[test]
fn contact_sheets() {
    let dir = std::env::var_os("WALLPAPER_SHEETS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("wallpapers"));
    std::fs::create_dir_all(&dir).unwrap();
    let all: Vec<&'static Wallpaper> = wallpaper::all().iter().collect();
    for family in Family::ALL {
        let list: Vec<&'static Wallpaper> = all.iter().copied().filter(|w| w.family == family).collect();
        let c = sheet(&format!("{} ({}): idle screen, menu, incoming call", family.name(), list.len()), &list, 1, 1, true);
        write_png(&dir.join(format!("{}.png", family.name().to_lowercase())), &c);
    }
    let c = sheet(&format!("Open DPhone wallpapers ({}): idle screen and menu", all.len()), &all, 3, 2, false);
    write_png(&dir.join("overview.png"), &c);
    // The wallpaper selector on a few wallpapers, dark and light.
    let shown = ["harbor", "dusk", "sunrise", "houndstooth", "regatta", "hot-dog-stand"];
    let mut c = Canvas::new((12 + 2 * (WIDTH + 12)) as u16, (12 + 3 * (HEIGHT + 12)) as u16, false);
    c.clear(0x2B2B2B);
    for (i, id) in shown.iter().enumerate() {
        let mut r = Rig::named("");
        r.show("settings");
        r.tap(Key::Digit(3));
        r.tap(Key::Digit(4));
        for _ in 0..all.iter().position(|w| w.id == *id).unwrap() {
            r.tap(Key::Right);
        }
        let rgba: Vec<u8> = r.frame().iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, 255]).collect();
        c.rgba(12 + (i % 2) as i32 * (WIDTH + 12), 12 + (i / 2) as i32 * (HEIGHT + 12), WIDTH, HEIGHT, &rgba);
    }
    write_png(&dir.join("wallpaper-menu.png"), &c);
    assert!(dir.join("overview.png").metadata().unwrap().len() > 10_000);
}

/// What the wallpapers cost: prints the time to draw each (on this
/// machine) and what the same pictures would take as PNG files.  Run with
/// `--release -- --ignored --nocapture costs`; `WALLPAPER_ONLY` draws one
/// wallpaper a hundred times and nothing else (for counting instructions).
#[test]
#[ignore = "a measurement, not a check"]
fn costs() {
    if let Ok(only) = std::env::var("WALLPAPER_ONLY") {
        let w = wallpaper::find(&only).expect("WALLPAPER_ONLY names a wallpaper");
        let mut sum = 0u64;
        for _ in 0..100 {
            sum = sum.wrapping_add(fnv(&w.render()));
        }
        println!("{only}: {sum:x}");
        return;
    }
    let mut total_png = 0;
    for w in wallpaper::all() {
        let start = std::time::Instant::now();
        let mut pixels = Vec::new();
        for _ in 0..20 {
            pixels = w.render();
        }
        let ms = start.elapsed().as_secs_f64() * 1000.0 / 20.0;
        let rgb: Vec<u8> = pixels.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]).collect();
        let mut png_file = Vec::new();
        let mut enc = png::Encoder::new(&mut png_file, WIDTH as u32, HEIGHT as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::High);
        enc.write_header().unwrap().write_image_data(&rgb).unwrap();
        total_png += png_file.len();
        println!("{:12} {ms:6.2} ms here, {:7} bytes as PNG", w.id, png_file.len());
    }
    println!("{} wallpapers as PNG files: {} bytes", wallpaper::all().len(), total_png);
}
