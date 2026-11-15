//! The app view's canvas widget: drawing operations painted in order,
//! clipped to the widget, in the skin's colors or the gray panel's levels.

use dphone_hal::Hal;
use dphone_state::{Model, Phase, Store, System};
use dphone_ui::appview::{AppView, Color, DrawOp, Font, Rect, ThemeColor, WidgetId, WidgetKind};
use dphone_ui::{Ui, UiConfig};

fn rig(model: Model) -> (Ui, AppView, WidgetId) {
    let mut hal = Hal::sim(model);
    let store = Store::new(model);
    let mut ui = Ui::from_hal(&mut hal, store.clone(), UiConfig::default());
    store.update::<System, _>(|s| s.phase = Phase::Running);
    let (view, events) = AppView::new("game", model);
    // The view outlives its event queue in this test.
    std::mem::forget(events);
    let canvas = view.create(WidgetKind::Canvas, None, Rect::new(10, 20, 60, 40));
    ui.handle().open_app(view.clone());
    ui.pump();
    assert_eq!(ui.screen_name(), "app:game");
    (ui, view, canvas)
}

fn fill(x: i16, y: i16, w: u16, h: u16, color: Color) -> DrawOp {
    DrawOp::Fill { rect: Rect::new(x, y, w, h), color }
}

const RED: Color = Color::Rgb(200, 0, 0);
const GREEN: Color = Color::Rgb(0, 200, 0);

#[test]
fn operations_are_painted_in_order_inside_the_canvas() {
    let (mut ui, view, canvas) = rig(Model::D60);
    let bg = ui.canvas().pixel(10, 20);
    // Nine pixels a row: two bytes a row.  The first row has its first
    // and last pixel set, the second its second.
    let bits: std::sync::Arc<[u8]> = vec![0b1000_0000, 0b1000_0000, 0b0100_0000, 0].into();
    view.canvas_set_ops(
        canvas,
        vec![
            fill(0, 0, 8, 8, RED),
            // Over the first: the later operation wins.
            fill(4, 4, 8, 8, GREEN),
            // Half outside the canvas on the right and below.
            fill(50, 30, 20, 20, RED),
            // Wholly outside.
            fill(-30, 0, 20, 5, RED),
            DrawOp::Bitmap { x: 20, y: 10, width: 9, height: 2, scale: 3, color: GREEN, bits: bits.clone() },
            // A picture cut by the left edge: only its last column shows.
            DrawOp::Bitmap { x: -24, y: 30, width: 9, height: 2, scale: 3, color: RED, bits },
            // Too few bytes for its size: nothing.
            DrawOp::Bitmap { x: 0, y: 20, width: 16, height: 2, scale: 1, color: RED, bits: vec![0xFF; 3].into() },
        ],
    );
    ui.pump();
    let c = ui.canvas();
    let px = |x: i32, y: i32| c.pixel(10 + x, 20 + y);
    let (red, green) = (0xC8_0000, 0x00_C800);
    assert_eq!([px(0, 0), px(3, 7), px(4, 4), px(11, 11), px(12, 12)], [red, red, green, green, bg]);
    // Clipped at the canvas's right and bottom edges.
    assert_eq!([px(50, 30), px(59, 39), px(60, 39), px(59, 40)], [red, red, bg, bg]);
    assert_eq!(px(-1, 0), bg);
    // The bitmap: three-pixel squares for the set bits only.
    assert_eq!([px(20, 10), px(22, 12), px(23, 10), px(44, 10), px(46, 12), px(47, 10)], [green, green, bg, green, green, bg]);
    assert_eq!([px(20, 13), px(23, 13), px(25, 15), px(26, 13)], [bg, green, green, bg]);
    assert_eq!([px(0, 30), px(2, 32), px(3, 30), px(0, 33)], [red, red, bg, bg]);
    assert_eq!([px(0, 20), px(15, 21)], [bg, bg]);

    // A new list replaces the old one.
    view.canvas_set_ops(canvas, vec![fill(0, 0, 2, 2, Color::Theme(ThemeColor::SelectBackground))]);
    ui.pump();
    let (r, g, b) = view.theme_color(ThemeColor::SelectBackground);
    let select = u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b);
    let c = ui.canvas();
    assert_eq!([c.pixel(10, 20), c.pixel(14, 24), c.pixel(30, 30)], [select, bg, bg]);
}

#[test]
fn a_gray_screen_shows_the_nearest_level() {
    let (mut ui, view, canvas) = rig(Model::D40);
    view.canvas_set_ops(
        canvas,
        vec![
            fill(0, 0, 1, 1, Color::Rgb(0, 0, 0)),
            fill(1, 0, 1, 1, Color::Rgb(0x60, 0x60, 0x60)),
            fill(2, 0, 1, 1, Color::Rgb(0xA0, 0xA0, 0xA0)),
            fill(3, 0, 1, 1, Color::Rgb(255, 255, 255)),
            // A dark blue is nearly black; yellow is nearly white.
            fill(4, 0, 1, 1, Color::Rgb(0, 0, 160)),
            fill(5, 0, 1, 1, Color::Rgb(255, 255, 0)),
        ],
    );
    ui.pump();
    let c = ui.canvas();
    let row: Vec<u32> = (0..6).map(|x| c.pixel(10 + x, 20)).collect();
    assert_eq!(row, [0x00_0000, 0x55_5555, 0xAA_AAAA, 0xFF_FFFF, 0x00_0000, 0xFF_FFFF]);
    assert_eq!(view.theme_color(ThemeColor::Background), (255, 255, 255));
}

/// The box around the pixels in `rect` (of the canvas) that are not
/// `ground`: left, top, right, and bottom, the last two exclusive.
fn painted(ui: &Ui, rect: (i32, i32, i32, i32), ground: u32) -> Option<(i32, i32, i32, i32)> {
    let c = ui.canvas();
    let mut found: Option<(i32, i32, i32, i32)> = None;
    for y in rect.1..rect.1 + rect.3 {
        for x in rect.0..rect.0 + rect.2 {
            if c.pixel(10 + x, 20 + y) != ground {
                let (l, t, r, b) = found.unwrap_or((x, y, x + 1, y + 1));
                found = Some((l.min(x), t.min(y), r.max(x + 1), b.max(y + 1)));
            }
        }
    }
    found
}

#[test]
fn text_is_in_the_skins_font_with_its_glyphs_in_the_middle_of_the_box() {
    for model in [Model::D60, Model::D40, Model::D70] {
        let (mut ui, view, canvas) = rig(model);
        let ground = ui.canvas().pixel(10, 20);
        for font in [Font::Small, Font::Normal, Font::Bold, Font::Title, Font::Large] {
            for text in ["0", "1", "4", "8", "12"] {
                // What the text paints, as the view tells an app.
                let (w, h) = view.text_ink(font, text);
                assert!(w > 0 && h > 0, "{model:?} {font:?} {text}");
                let rect = Rect::new(5, 3, 40, 31);
                view.canvas_set_ops(canvas, vec![DrawOp::Text { rect, font, color: RED, text: text.into() }]);
                ui.pump();
                let (l, t, r, b) = painted(&ui, (0, 0, 60, 40), ground).expect("some text");
                // That size (a smoothed glyph's outermost row or column
                // may be too faint to show), in the middle of the box:
                // the space left and right differs by a pixel or two at
                // most, and above and below.
                let (pw, ph) = (r - l, b - t);
                assert!(pw <= w as i32 && pw >= w as i32 - 2, "{model:?} {font:?} {text}: {pw} of {w}");
                assert!(ph <= h as i32 && ph >= h as i32 - 2, "{model:?} {font:?} {text}: {ph} of {h}");
                assert!(((l - 5) - (45 - r)).abs() <= 2, "{model:?} {font:?} {text}: {l}..{r}");
                assert!(((t - 3) - (34 - b)).abs() <= 2, "{model:?} {font:?} {text}: {t}..{b}");
            }
        }
        // The digits of a font are equally tall, so "0123456789" tells
        // how tall any of them is.
        let all = view.text_ink(Font::Bold, "0123456789").1;
        assert!(["0", "5", "8"].iter().all(|d| view.text_ink(Font::Bold, d).1 == all));
        // Nothing is painted outside the box, however small it is.
        view.canvas_set_ops(canvas, vec![DrawOp::Text { rect: Rect::new(20, 10, 4, 3), font: Font::Large, color: RED, text: "8".into() }]);
        ui.pump();
        let (l, t, r, b) = painted(&ui, (0, 0, 60, 40), ground).expect("a part of the 8");
        assert!(l >= 20 && t >= 10 && r <= 24 && b <= 13, "{model:?}: {l} {t} {r} {b}");
        // Text of no glyphs paints nothing and has no size.
        assert_eq!(view.text_ink(Font::Normal, ""), (0, 0));
        view.canvas_set_ops(canvas, vec![DrawOp::Text { rect: Rect::new(0, 0, 60, 40), font: Font::Normal, color: RED, text: String::new() }]);
        ui.pump();
        assert_eq!(painted(&ui, (0, 0, 60, 40), ground), None);
    }
}
