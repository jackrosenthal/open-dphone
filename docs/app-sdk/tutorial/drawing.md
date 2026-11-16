# Drawing with the canvas

Text works for someone who stops to read the screen.  This chapter adds a picture for someone who only glances at the phone: a mug, and a bar that starts full and empties over an hour as the coffee goes stale.

<figure markdown="span">
  ![Fresh Pot with the mug and the bar, on a color display and on a grayscale display](../../assets/app-sdk/gauge.png){ width="656" }
  <figcaption>The app at the end of this chapter, 18 minutes after Brewed, on a D60 and on a D40.  The same app file drew both.</figcaption>
</figure>

## Canvas basics

The [`ui`](../reference/ui.md) widgets show text, lists, input fields, and PNG images.  For anything else, the [`canvas`](../reference/canvas.md) interface provides a widget that the app draws.  The app gives the phone a list of drawing operations, and the phone paints from that list whenever the display needs repainting.  There are three operations:

| Operation | Draws |
| --- | --- |
| `Op::Fill` | A rectangle in a solid color. |
| `Op::Bitmap` | A one-color image, one bit per pixel, scaled up by a whole-number factor. |
| `Op::Text` | A single line of text in one of the phone's fonts. |

Each call to [`canvas::draw`](../reference/canvas.md#draw) replaces the entire list, so the phone never shows a half-finished picture.  Fresh Pot uses `Fill` and `Bitmap`.

Import the module and the types:

```rust
use dphone_app_sdk::canvas::{self, Bitmap, Op, Role};
```

## Bitmap data

A bitmap is stored as rows of bits.  Each row is padded to a whole number of bytes, and the leftmost pixel is the high bit of the first byte.  The mug is 12 pixels wide, so each row takes two bytes, and in binary notation you can see the picture in the constant.  Add the mug, along with the number of minutes that a pot stays fresh, below the other constants:

```rust
/// A pot is stale after this many minutes.
const FRESH_MINUTES: u64 = 60;

/// A mug, 12 pixels wide and 10 pixels tall.  Each row is two bytes, and
/// the leftmost pixel is the high bit of the first byte.
#[rustfmt::skip]
const MUG: [u8; 20] = [
    0b1111_1111, 0b1000_0000,
    0b1111_1111, 0b1000_0000,
    0b1111_1111, 0b1110_0000,
    0b1111_1111, 0b1011_0000,
    0b1111_1111, 0b1011_0000,
    0b1111_1111, 0b1011_0000,
    0b1111_1111, 0b1110_0000,
    0b1111_1111, 0b1000_0000,
    0b1111_1111, 0b1000_0000,
    0b0111_1111, 0b0000_0000,
];
```

`#[rustfmt::skip]` keeps `cargo fmt` from joining the rows.

## Canvas placement

Like any widget, a canvas is created with bounds inside the window.  The app needs to keep the handle, and also the dimensions, which the drawing code uses.  Add three fields to `FreshPot`:

```rust
/// The canvas below the two lines of text, and the canvas dimensions.
gauge: Widget,
gauge_width: u16,
gauge_height: u16,
```

In `start`, after the two text widgets, create the canvas below the second line.  The layout is again in multiples of the phone's line height:

```rust
// The canvas is half the width of the window, centered, and two
// lines tall.
self.gauge_width = area.width / 2;
self.gauge_height = 2 * line;
let bounds = Rect::new(
    (area.width / 4) as i16,
    area.y + (3 * line + line / 2) as i16,
    self.gauge_width,
    self.gauge_height,
);
self.gauge = canvas::create(ROOT, bounds);
```

## Drawing operations

Add a function to `impl FreshPot` that builds the list of operations:

```rust
/// The drawing operations for the canvas: a mug, and a bar that empties
/// as the pot goes stale.  With no pot, the canvas is empty.
fn gauge_ops(&self) -> Vec<Op> {
    let Some(at) = self.brewed else {
        return Vec::new();
    };
    let minutes = timers::wall_time().saturating_sub(at) / 60;
    let fresh = FRESH_MINUTES.saturating_sub(minutes);

    // Scale the mug to the height of the canvas.
    let scale = (self.gauge_height / 10).clamp(1, 8);
    let mug_width = 12 * scale;
    let mug = Bitmap {
        x: 0,
        y: 0,
        width: 12,
        height: 10,
        scale: scale as u8,
        color: canvas::theme_color(Role::Foreground),
        bits: MUG.to_vec(),
    };

    // The bar takes up the remaining width: a one-pixel frame, filled
    // in proportion to the time left in the hour.
    let x = (mug_width + scale) as i16;
    let y = (self.gauge_height / 4) as i16;
    let width = self.gauge_width - mug_width - scale;
    let height = self.gauge_height / 2;
    let filled = ((width - 2) as u64 * fresh / FRESH_MINUTES) as u16;
    let right = x + width as i16 - 1;
    let bottom = y + height as i16 - 1;
    let border = canvas::theme_color(Role::Border);
    vec![
        Op::Bitmap(mug),
        Op::Fill((Rect::new(x, y, width, 1), border)),
        Op::Fill((Rect::new(x, bottom, width, 1), border)),
        Op::Fill((Rect::new(x, y, 1, height), border)),
        Op::Fill((Rect::new(right, y, 1, height), border)),
        Op::Fill((
            Rect::new(x + 1, y + 1, filled, height - 2),
            canvas::theme_color(Role::Highlight),
        )),
    ]
}
```

Coordinates are relative to the top-left corner of the canvas, and the phone clips everything to the canvas.  Three decisions make the same drawing work on every display:

- **Sizes are derived from the canvas.**  The mug's `scale` turns each bit into a square, so that the 10 rows fill the height of the canvas: 5 pixels per bit on a color display, 3 on the smallest grayscale display.  The bar takes up the remaining width.
- **Colors come from the phone.**  [`canvas::theme_color`](../reference/canvas.md#theme_color) returns the color that the phone uses for a given role: `Role::Foreground` for text, `Role::Border` for lines, and `Role::Highlight` for a selection.  On a color display, these are the colors of the theme that the user chose.  A grayscale display has four shades of gray.  Either way, the drawing matches the rest of the phone's screens.
- **Only the foreground is painted.**  The frame is four rectangles, each one pixel thick, and the only other paint inside the frame is the filled portion, so the window's background shows through everywhere else.  Where operations overlap, later ones paint over earlier ones.

Finally, draw whenever the state is rendered.  At the end of `show`, add:

```rust
if let Err(e) = canvas::draw(self.gauge, &self.gauge_ops()) {
    log_warn(&format!("couldn't draw the gauge: {e}"));
}
```

`show` already runs after every softkey press and on every tick of the 30-second timer, so the bar empties with no further code.  `canvas::draw` rejects a list that exceeds the [canvas limits](../reference/limits.md#ui-and-canvas), such as more than 512 operations, and the canvas then keeps the previous picture.

## Theme changes

The operations contain the colors that `theme_color` returned at the time of the call.  If the user picks another theme, the phone restyles text widgets automatically, but a canvas keeps the old colors until the app draws again.

The phone tells the app when to redraw: `Event::Foreground` arrives whenever the app's window comes to the front, and again when the theme changes while the app is on screen.  In `handle`, add `Event::Foreground` to the timer's arm so that both fall through to `show`:

```rust
// On a timer tick, or when the window returns to the screen
// (possibly with a new theme's colors), fall through and redraw.
Event::Timer(_) | Event::Foreground => {}
```

## Build and run

Build and install the app as before:

```sh
cargo build --release --target wasm32-unknown-unknown
wasm-tools component new \
    target/wasm32-unknown-unknown/release/freshpot.wasm -o freshpot.wasm
```

Open the app.  The pot from the previous chapter is still there, because installing a newer build keeps the app's store, and the bar shows how much of the pot's hour remains.

One limitation remains: only the phone where someone pressed **Brewed** knows about the pot.  The last chapter fixes that.

??? example "Complete `src/lib.rs` for this chapter"

    ```rust
    use dphone_app_sdk::canvas::{self, Bitmap, Op, Role};
    use dphone_app_sdk::prelude::*;
    use dphone_app_sdk::{Align, Font, log_info, log_warn, read_string};

    const SK_BREWED: u32 = 1;
    const SK_EMPTY: u32 = 2;
    const SK_EXIT: u32 = 3;

    /// How often the screen refreshes, in milliseconds.
    const REFRESH_MS: u32 = 30_000;

    /// The storage key for the time the pot was brewed.
    const KEY: &str = "brewed";

    /// A pot is stale after this many minutes.
    const FRESH_MINUTES: u64 = 60;

    /// A mug, 12 pixels wide and 10 pixels tall.  Each row is two bytes, and
    /// the leftmost pixel is the high bit of the first byte.
    #[rustfmt::skip]
    const MUG: [u8; 20] = [
        0b1111_1111, 0b1000_0000,
        0b1111_1111, 0b1000_0000,
        0b1111_1111, 0b1110_0000,
        0b1111_1111, 0b1011_0000,
        0b1111_1111, 0b1011_0000,
        0b1111_1111, 0b1011_0000,
        0b1111_1111, 0b1110_0000,
        0b1111_1111, 0b1000_0000,
        0b1111_1111, 0b1000_0000,
        0b0111_1111, 0b0000_0000,
    ];

    #[derive(Default)]
    struct FreshPot {
        /// When the pot was brewed, in seconds since the Unix epoch.
        brewed: Option<u64>,
        /// The line that shows when the pot was brewed.
        status: Widget,
        /// The line that shows how long ago that was.
        age: Widget,
        /// The canvas below the two lines of text, and the canvas dimensions.
        gauge: Widget,
        gauge_width: u16,
        gauge_height: u16,
    }

    impl FreshPot {
        /// Loads the brew time from the phone's store.
        fn load(&mut self) {
            self.brewed = read_string(KEY).and_then(|text| text.parse().ok());
        }

        /// Saves the brew time to the phone's store.
        fn save(&self) {
            let result = match self.brewed {
                Some(at) => storage::set(KEY, at.to_string().as_bytes()),
                None => storage::remove(KEY),
            };
            if let Err(e) = result {
                log_warn(&format!("couldn't store the brew time: {e:?}"));
            }
        }

        /// Renders the state to the screen.
        fn show(&self) {
            match self.brewed {
                Some(at) => {
                    let minutes = timers::wall_time().saturating_sub(at) / 60;
                    ui::set_text(
                        self.status,
                        &format!("Brewed at {}", l10n::format_time(at, TimeStyle::Time)),
                    );
                    ui::set_text(self.age, &age_text(minutes));
                }
                None => {
                    ui::set_text(self.status, "No pot");
                    ui::set_text(self.age, "Press Brewed after making coffee.");
                }
            }
            if let Err(e) = canvas::draw(self.gauge, &self.gauge_ops()) {
                log_warn(&format!("couldn't draw the gauge: {e}"));
            }
        }

        /// The drawing operations for the canvas: a mug, and a bar that empties
        /// as the pot goes stale.  With no pot, the canvas is empty.
        fn gauge_ops(&self) -> Vec<Op> {
            let Some(at) = self.brewed else {
                return Vec::new();
            };
            let minutes = timers::wall_time().saturating_sub(at) / 60;
            let fresh = FRESH_MINUTES.saturating_sub(minutes);

            // Scale the mug to the height of the canvas.
            let scale = (self.gauge_height / 10).clamp(1, 8);
            let mug_width = 12 * scale;
            let mug = Bitmap {
                x: 0,
                y: 0,
                width: 12,
                height: 10,
                scale: scale as u8,
                color: canvas::theme_color(Role::Foreground),
                bits: MUG.to_vec(),
            };

            // The bar takes up the remaining width: a one-pixel frame, filled
            // in proportion to the time left in the hour.
            let x = (mug_width + scale) as i16;
            let y = (self.gauge_height / 4) as i16;
            let width = self.gauge_width - mug_width - scale;
            let height = self.gauge_height / 2;
            let filled = ((width - 2) as u64 * fresh / FRESH_MINUTES) as u16;
            let right = x + width as i16 - 1;
            let bottom = y + height as i16 - 1;
            let border = canvas::theme_color(Role::Border);
            vec![
                Op::Bitmap(mug),
                Op::Fill((Rect::new(x, y, width, 1), border)),
                Op::Fill((Rect::new(x, bottom, width, 1), border)),
                Op::Fill((Rect::new(x, y, 1, height), border)),
                Op::Fill((Rect::new(right, y, 1, height), border)),
                Op::Fill((
                    Rect::new(x + 1, y + 1, filled, height - 2),
                    canvas::theme_color(Role::Highlight),
                )),
            ]
        }
    }

    fn age_text(minutes: u64) -> String {
        match minutes {
            0 => "Just now".to_string(),
            1 => "1 minute ago".to_string(),
            m if m < 60 => format!("{m} minutes ago"),
            m => format!("{} h {} min ago", m / 60, m % 60),
        }
    }

    impl App for FreshPot {
        fn start(&mut self) {
            let area = screen("Fresh Pot");
            let line = ui::get_metrics().line_height;

            self.status = ui::create(
                WidgetKind::Text,
                ROOT,
                Rect::new(0, area.y, area.width, 2 * line),
            );
            ui::set_font(self.status, Font::Large);
            ui::set_align(self.status, Align::Center);

            self.age = ui::create(
                WidgetKind::Text,
                ROOT,
                Rect::new(0, area.y + 2 * line as i16, area.width, line),
            );
            ui::set_align(self.age, Align::Center);

            // The canvas is half the width of the window, centered, and two
            // lines tall.
            self.gauge_width = area.width / 2;
            self.gauge_height = 2 * line;
            let bounds = Rect::new(
                (area.width / 4) as i16,
                area.y + (3 * line + line / 2) as i16,
                self.gauge_width,
                self.gauge_height,
            );
            self.gauge = canvas::create(ROOT, bounds);

            ui::set_softkeys(
                ROOT,
                &[
                    Softkey::new(SK_BREWED, "Brewed"),
                    Softkey::new(SK_EMPTY, "Empty"),
                    Softkey::new(SK_EXIT, "Exit"),
                ],
            );
            let _ = timers::set(REFRESH_MS, true);
            self.load();
            self.show();
        }

        fn handle(&mut self, event: Event) {
            match event {
                Event::Softkey(SK_BREWED) => {
                    self.brewed = Some(timers::wall_time());
                    self.save();
                    log_info("a pot was brewed");
                }
                Event::Softkey(SK_EMPTY) => {
                    self.brewed = None;
                    self.save();
                }
                Event::Softkey(SK_EXIT) => return runtime::exit(),
                Event::Key(key) if key == "cancel" => return runtime::exit(),
                // On a timer tick, or when the window returns to the screen
                // (possibly with a new theme's colors), fall through and redraw.
                Event::Timer(_) | Event::Foreground => {}
                _ => return,
            }
            self.show();
        }
    }

    dphone_app_sdk::export_app! {
        app: FreshPot,
        name: "freshpot",
        display_name: "Fresh Pot",
        description: "When the office coffee was brewed",
    }
    ```
