//! The first thing a display shows is the boot screen: with its first
//! frame drawn, the UI sets the contrast and the configured backlight
//! level and turns the panels on with that frame.  Through the HAL
//! simulator, which records the order.

use dphone_hal::sim::SimDisplayEvent as Ev;
use dphone_hal::{Hal, Screen, backlight_level};
use dphone_state::*;
use dphone_ui::{Ui, UiConfig};

fn blank(frame: &dphone_hal::Frame) -> bool {
    frame.pixels.iter().all(|p| *p == frame.pixels[0])
}

#[test]
fn the_panels_come_on_with_the_boot_screen_at_the_configured_brightness() {
    for model in [Model::D60, Model::D40, Model::D70] {
        let mut hal = Hal::sim(model);
        let sim = hal.sim.clone().unwrap();
        let store = Store::new(model);
        assert_eq!(store.get::<System>().phase, Phase::Starting);
        let cfg = UiConfig { brightness: 3, contrast: 4, ..Default::default() };
        let mut ui = Ui::from_hal(&mut hal, store.clone(), cfg);

        ui.pump();
        let events = sim.display_events();
        let screens: Vec<Screen> =
            [Screen::Main, Screen::Side].into_iter().filter(|s| sim.screen_power(*s).is_some()).collect();
        assert_eq!(screens.len(), if model == Model::D70 { 2 } else { 1 });
        let first_on = events.iter().position(|e| matches!(e, Ev::Power(_, true))).expect("a panel was turned on");
        for &screen in &screens {
            // On, once, and with the boot screen already in the frame.
            assert_eq!(events.iter().filter(|e| **e == Ev::Power(screen, true)).count(), 1, "{model} {screen:?}");
            let first = sim.first_lit(screen).expect("the panel is on");
            assert!(!blank(&first), "{model} {screen:?}: the panel came on with an empty frame");
            assert!(first == sim.frame(screen).unwrap(), "{model} {screen:?}: the panel came on with another frame");

            // The backlight: the configured level, set before any panel
            // is on, and never anything else (no full brightness).
            let level = backlight_level(model, screen, 3);
            let set: Vec<(usize, u8)> = events
                .iter()
                .enumerate()
                .filter_map(|(i, e)| match e {
                    Ev::Brightness(s, l) if *s == screen => Some((i, *l)),
                    _ => None,
                })
                .collect();
            assert!(!set.is_empty(), "{model} {screen:?}");
            assert!(set.iter().all(|(_, l)| *l == level), "{model} {screen:?}: {set:?}");
            assert!(set[0].0 < first_on, "{model} {screen:?}: {events:?}");
            assert_eq!(sim.brightness(screen), Some(level));
        }

        // The gray panels' contrast is set before a panel is on.
        let contrast = events.iter().position(|e| matches!(e, Ev::Contrast(..)));
        if !model.is_color() {
            assert_eq!(events[contrast.expect("contrast set")], Ev::Contrast(Screen::Main, 4));
            assert!(contrast.unwrap() < first_on, "{model}: {events:?}");
        }

        // The boot screen it is: the same picture as a UI that has been
        // running in the starting phase draws.
        assert!(store.get::<dphone_state::Ui>().backlight_on);
        assert_eq!(store.get::<dphone_state::Ui>().screen, "boot", "{model}");

        // Later frames reach a panel that is on.
        store.update::<System, _>(|s| s.phase = Phase::Running);
        ui.pump();
        let later = &sim.display_events()[events.len()..];
        assert!(later.iter().any(|e| matches!(e, Ev::Flush(Screen::Main))), "{model}: {later:?}");
    }
}
