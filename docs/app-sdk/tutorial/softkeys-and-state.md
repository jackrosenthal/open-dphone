# Softkeys and state

In this chapter, the app becomes useful on a single phone.  The **Brewed** softkey records the current time, the screen shows that time and the age of the pot, **Empty** clears both, and a timer keeps the age up to date.  All of the changes are in `src/lib.rs`.

<figure markdown="span">
  ![Fresh Pot with no pot recorded, and 25 minutes after the softkey Brewed](../../assets/app-sdk/softkeys.png){ width="656" }
  <figcaption>The app at the end of this chapter: before anyone has pressed Brewed, and 25 minutes after.</figcaption>
</figure>

## App state

Add fields to the app type.  `brewed` is everything the app knows about the pot.  The two `Widget` fields are handles to the two lines of text, which later events need in order to change the text:

```rust
#[derive(Default)]
struct FreshPot {
    /// When the pot was brewed, in seconds since the Unix epoch.
    brewed: Option<u64>,
    /// The line that shows when the pot was brewed.
    status: Widget,
    /// The line that shows how long ago that was.
    age: Widget,
}
```

`Default` still creates the value, so `brewed` starts out as `None`.  A `Widget` is a number, and the default of 0 refers to the window.  `start` replaces both handles before anything uses them.

## Screen layout

The same app file runs on a 480 by 272 color display and on 320 by 160 and 240 by 120 grayscale displays, so never hard-code a size.  Ask the phone instead: `screen` returns the rectangle below the title, and [`ui::get_metrics`](../reference/ui.md#get_metrics) returns, among other dimensions, the height of a line of text in the normal font.

Replace `start` with:

```rust
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

    ui::set_softkeys(
        ROOT,
        &[
            Softkey::new(SK_BREWED, "Brewed"),
            Softkey::new(SK_EMPTY, "Empty"),
            Softkey::new(SK_EXIT, "Exit"),
        ],
    );
    let _ = timers::set(REFRESH_MS, true);
    self.show();
}
```

The first widget is two lines tall and uses `Font::Large`.  The second sits below the first and is one line tall.  `Align::Center` centers the text within each widget's rectangle.

`Font` and `Align` aren't in the SDK's prelude, so change the imports at the top of the file to:

```rust
use dphone_app_sdk::prelude::*;
use dphone_app_sdk::{Align, Font, log_info};
```

## Softkeys

Near the end of `start`, [`ui::set_softkeys`](../reference/ui.md#set_softkeys) labels the keys below the display, from left to right.  Each softkey has an ID that you choose, and a press arrives as `Event::Softkey` with that ID.  Define the IDs, along with the refresh interval for the timer, at the top of the file:

```rust
const SK_BREWED: u32 = 1;
const SK_EMPTY: u32 = 2;
const SK_EXIT: u32 = 3;

/// How often the screen refreshes, in milliseconds.
const REFRESH_MS: u32 = 30_000;
```

The softkey bar has room for four softkeys.  If an app sets more, the phone shows three plus a "More" key that pages through the rest.

## Rendering

A single function renders the state to the screen, and every event that changes the state ends by calling that function.  [`timers::wall_time`](../reference/timers.md#wall_time) returns the current time in seconds since the Unix epoch.  [`l10n::format_time`](../reference/l10n.md#format_time) formats a timestamp in the phone's time zone, in 12-hour or 24-hour form depending on the phone's settings.  `TimeStyle::Time` asks for the time without the date.

```rust
impl FreshPot {
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
```

## Event handling

Replace `handle` with:

```rust
fn handle(&mut self, event: Event) {
    match event {
        Event::Softkey(SK_BREWED) => {
            self.brewed = Some(timers::wall_time());
            log_info("a pot was brewed");
        }
        Event::Softkey(SK_EMPTY) => self.brewed = None,
        Event::Softkey(SK_EXIT) => return runtime::exit(),
        Event::Key(key) if key == "cancel" => return runtime::exit(),
        Event::Timer(_) => {}
        _ => return,
    }
    self.show();
}
```

Each arm handles one kind of event:

- **Softkeys.**  **Brewed** stores the current time, **Empty** clears the time, and **Exit** exits the app.
- **Keys.**  The cancel key arrives as `Event::Key` because `screen` claimed that key for the window, and the app treats a press like **Exit**.  An app receives only the keys claimed with [`ui::claim_keys`](../reference/ui.md#claim_keys).  For the key names, see [`Key`](../reference/app-events.md#key) in the reference.
- **The timer.**  In `start`, [`timers::set(REFRESH_MS, true)`](../reference/timers.md#set) asked for an `Event::Timer` every 30 seconds.  The call returns a `Result`: `Ok` contains the timer's ID, which matters to an app that has several timers or cancels one, and `Err` means that no timer was started.  Fresh Pot has one timer and never cancels that timer, so the code discards the result with `let _ =`.  The arm is empty and falls through to `show`, which recomputes the age.
- **Everything else** returns without rendering.

The phone gives `handle` two seconds per event and stops an app that takes longer.  If an app has a lot of work to do, split the work across timer events.

## Build and run

Build and install the app the same way as in [the first chapter](first-app.md#build):

```sh
cargo build --release --target wasm32-unknown-unknown
wasm-tools component new \
    target/wasm32-unknown-unknown/release/freshpot.wasm -o freshpot.wasm
```

Open the app and press **Brewed**.  The screen shows the time and "Just now", and after a minute the second line starts counting.  Now press **Exit** and open the app again.  The screen says "No pot": the time is gone.  The [next chapter](persistent-storage.md) explains why and fixes the problem.

??? example "Complete `src/lib.rs` for this chapter"

    ```rust
    use dphone_app_sdk::prelude::*;
    use dphone_app_sdk::{Align, Font, log_info};

    const SK_BREWED: u32 = 1;
    const SK_EMPTY: u32 = 2;
    const SK_EXIT: u32 = 3;

    /// How often the screen refreshes, in milliseconds.
    const REFRESH_MS: u32 = 30_000;

    #[derive(Default)]
    struct FreshPot {
        /// When the pot was brewed, in seconds since the Unix epoch.
        brewed: Option<u64>,
        /// The line that shows when the pot was brewed.
        status: Widget,
        /// The line that shows how long ago that was.
        age: Widget,
    }

    impl FreshPot {
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

            ui::set_softkeys(
                ROOT,
                &[
                    Softkey::new(SK_BREWED, "Brewed"),
                    Softkey::new(SK_EMPTY, "Empty"),
                    Softkey::new(SK_EXIT, "Exit"),
                ],
            );
            let _ = timers::set(REFRESH_MS, true);
            self.show();
        }

        fn handle(&mut self, event: Event) {
            match event {
                Event::Softkey(SK_BREWED) => {
                    self.brewed = Some(timers::wall_time());
                    log_info("a pot was brewed");
                }
                Event::Softkey(SK_EMPTY) => self.brewed = None,
                Event::Softkey(SK_EXIT) => return runtime::exit(),
                Event::Key(key) if key == "cancel" => return runtime::exit(),
                Event::Timer(_) => {}
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
