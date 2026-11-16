//! `canvas.theme-color` through the phone UI: an app's window answers in
//! the palette of the theme in use, for every role, from the moment the
//! window exists (an app asks for its colors as it starts, before the
//! window is first on screen), and follows the theme when the user
//! chooses another.

use dphone_apps::uishell::UiShell;
use dphone_apps::view::{Shell, WindowKind};
use dphone_apps::{Color, ColorRole};
use dphone_hal::Hal;
use dphone_state::{Model, Phase, Store, System};
use dphone_ui::skin::Theme;
use dphone_ui::{Ui, UiConfig, wallpaper};
use std::sync::Arc;

fn ui(model: Model, cfg: UiConfig) -> Ui {
    let mut hal = Hal::sim(model);
    let store = Store::new(model);
    store.update::<System, _>(|s| s.phase = Phase::Running);
    let mut ui = Ui::from_hal(&mut hal, store, cfg);
    ui.set_fixed_time(Some((5_000_000, 1_794_507_300)));
    ui.pump();
    ui
}

/// Every role with the palette's color for it.
fn roles(th: &Theme) -> [(ColorRole, u32); 8] {
    [
        (ColorRole::Background, th.bg),
        (ColorRole::Foreground, th.fg),
        (ColorRole::Highlight, th.sel_bg),
        (ColorRole::HighlightText, th.sel_fg),
        (ColorRole::TitleBackground, th.title_bg),
        (ColorRole::TitleText, th.title_fg),
        (ColorRole::Secondary, th.secondary),
        (ColorRole::Border, th.border),
    ]
}

fn rgb(c: u32) -> (u8, u8, u8) {
    ((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

fn parts(c: Color) -> (u8, u8, u8) {
    (c.r, c.g, c.b)
}

#[test]
fn theme_colors_are_the_active_themes() {
    // Nothing configured: the default theme, Tides, whose palette is not
    // the color skin's own.
    let mut ui = ui(Model::D65, UiConfig::default());
    let tides = Theme::for_wallpaper(wallpaper::default());
    assert_ne!(tides.bg, Theme::for_model(Model::D65).bg);
    let mut shell = UiShell::new(ui.handle(), Model::D65);
    for kind in [WindowKind::App, WindowKind::Idle] {
        let window = shell.open("probe", kind, Arc::new(|| ()));
        // Before the window is shown.
        for (role, color) in roles(tides) {
            assert_eq!(parts(window.theme_color(role)), rgb(color), "{role:?} of a new window");
        }
        // The user chooses another theme while the app is open: on
        // screen or not, the window answers in the new palette.
        ui.handle().set_config(UiConfig { theme: "regatta".into(), ..UiConfig::default() });
        ui.pump();
        let regatta = Theme::for_wallpaper(wallpaper::find("regatta").unwrap());
        for (role, color) in roles(regatta) {
            assert_eq!(parts(window.theme_color(role)), rgb(color), "{role:?} after the theme changed");
        }
        ui.handle().set_config(UiConfig::default());
        ui.pump();
        assert_eq!(parts(window.theme_color(ColorRole::Background)), rgb(tides.bg));
    }

    // A gray model has no themes: its skin's colors.
    let gray = ui_of_gray();
    let mut shell = UiShell::new(gray.handle(), Model::D50);
    let window = shell.open("probe", WindowKind::App, Arc::new(|| ()));
    for (role, color) in roles(Theme::for_model(Model::D50)) {
        assert_eq!(parts(window.theme_color(role)), rgb(color), "{role:?} on a gray model");
    }
}

fn ui_of_gray() -> Ui {
    ui(Model::D50, UiConfig { theme: "regatta".into(), ..UiConfig::default() })
}
