# Softkeys and state

The **Brewed** softkey records the current time, the screen shows that time and the age of the pot, **Empty** clears both, and a timer keeps the age up to date.

<figure markdown="span">
  ![Fresh Pot with no pot recorded, and 25 minutes after the softkey Brewed](../../assets/app-sdk/softkeys.png){ width="656" }
</figure>

## App state

Add fields to the app type:

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

## Screen layout

The same app file runs on color and grayscale displays of different sizes, so never hard-code a size.  Ask the phone instead: `screen` returns the rectangle below the title, and [`ui::get_metrics`](../reference/ui.md#get_metrics) returns, among other dimensions, the height of a line of text in the normal font.

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

## Rendering

Below the app type, add a function that renders the state to the screen, and a function that formats the age:

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

[`timers::wall_time`](../reference/timers.md#wall_time) returns the current time in seconds since the Unix epoch, and [`l10n::format_time`](../reference/l10n.md#format_time) formats a timestamp in the phone's time zone, in 12-hour or 24-hour form depending on the phone's settings.

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

In `start`, [`timers::set(REFRESH_MS, true)`](../reference/timers.md#set) asked for an `Event::Timer` every 30 seconds.  The arm for that event is empty and falls through to `show`, which recomputes the age.

## Build and run

[Build and install](first-app.md#build) the app as before.  Open the app and press **Brewed**.  The screen shows the time and "Just now", and after a minute the second line starts counting.

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
