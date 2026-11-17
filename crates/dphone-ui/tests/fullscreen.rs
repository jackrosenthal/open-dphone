//! A full-screen app view: the whole main screen, no softkey bar, and the
//! softkey keys as ordinary keys.

use dphone_hal::Hal;
use dphone_state::{Key, Model, Phase, Store, System};
use dphone_ui::appview::{AppView, Color, KeyClaims, Rect, Softkey, WidgetKind};
use dphone_ui::{AppEvent, Ui, UiConfig};

#[test]
fn a_full_screen_view_has_the_display_and_no_softkeys() {
    for model in [Model::D60, Model::D40, Model::D70] {
        let mut hal = Hal::sim(model);
        let sim = hal.sim.clone().unwrap();
        let store = Store::new(model);
        let mut ui = Ui::from_hal(&mut hal, store.clone(), UiConfig::default());
        store.update::<System, _>(|s| s.phase = Phase::Running);
        let (view, events) = AppView::new("game", model);
        let (screen, area) = (model.main_screen(), model.app_content_area());
        // A marker that covers the whole screen, softkey bar included.
        let marker = Color::Rgb(255, 255, 255);
        let all = view.create(WidgetKind::Group, None, Rect::new(0, 0, screen.width, screen.height));
        view.set_colors(all, Color::Default, marker);
        view.set_softkeys(None, vec![Softkey::new(7, "Go")]);
        ui.handle().open_app(view.clone());
        ui.pump();
        assert_eq!(events.try_recv(), Some(AppEvent::Foreground));
        let last = screen.height as i32 - 1;
        let white = 0xFF_FFFF;

        // As every app: the content area, and the softkey bar below it.
        assert_eq!(view.metrics().height, area.height);
        let bar: Vec<u32> = (0..screen.width as i32).map(|x| ui.canvas().pixel(x, last - 2)).collect();
        assert!(bar.iter().any(|p| *p != white), "{model:?}: a softkey bar");
        sim.tap(Key::Soft(1));
        ui.pump();
        assert_eq!(events.try_recv(), Some(AppEvent::Softkey { id: 7 }));

        // Full-screen: the view's own pixels to the last row, and the
        // screen's height in the metrics.
        view.set_full_screen(true);
        ui.pump();
        assert_eq!((view.metrics().width, view.metrics().height), (screen.width, screen.height));
        for y in [0, area.height as i32, last] {
            assert!((0..screen.width as i32).all(|x| ui.canvas().pixel(x, y) == white), "{model:?}: row {y}");
        }
        // A softkey key is a key like any other now: it can be claimed.
        view.set_window_key_claims(KeyClaims::keys([Key::Soft(1)]));
        sim.tap(Key::Soft(1));
        ui.pump();
        assert_eq!(events.try_recv(), Some(AppEvent::Key { widget: None, key: Key::Soft(1) }));
        // Cancel, unclaimed, still asks the app to close.
        sim.tap(Key::Cancel);
        ui.pump();
        assert_eq!(events.try_recv(), Some(AppEvent::CloseRequested));
        assert_eq!(ui.screen_name(), "app:game");

        // And back.
        view.set_full_screen(false);
        view.set_window_key_claims(KeyClaims::none());
        ui.pump();
        assert_eq!(view.metrics().height, area.height);
        assert!((0..screen.width as i32).any(|x| ui.canvas().pixel(x, last - 2) != white));
        sim.tap(Key::Soft(1));
        ui.pump();
        assert_eq!(events.try_recv(), Some(AppEvent::Softkey { id: 7 }));
    }
}
