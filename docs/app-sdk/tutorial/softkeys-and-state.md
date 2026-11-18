# Softkeys and state

**Brewed** records the current time, the screen shows that time and the pot's age, **Empty** clears both, and a timer updates the age.

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

Display sizes differ by model, so take every size from the phone: `screen` returns the rectangle below the title, and [`ui::get_metrics`](../reference/ui.md#get_metrics) returns, among other values, the line height of the normal font.

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

`Font` and `Align` aren't in the prelude.  Change the imports to:

```rust
use opendeskphone_app_sdk::prelude::*;
use opendeskphone_app_sdk::{Align, Font, log_info};
```

## Softkeys

[`ui::set_softkeys`](../reference/ui.md#set_softkeys), near the end of `start`, labels the keys below the display, left to right.  A press arrives as `Event::Softkey` with the softkey's ID.  At the top of the file, define the IDs and the timer interval:

```rust
const SK_BREWED: u32 = 1;
const SK_EMPTY: u32 = 2;
const SK_EXIT: u32 = 3;

/// How often the screen refreshes, in milliseconds.
const REFRESH_MS: u32 = 30_000;
```

## Rendering

Below the app type, add a function that renders the state and a function that formats the age:

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

[`timers::wall_time`](../reference/timers.md#wall_time) returns seconds since the Unix epoch.  [`l10n::format_time`](../reference/l10n.md#format_time) formats a time in the phone's time zone and 12-hour or 24-hour setting.

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
        Event::Key(Key::Cancel) => return runtime::exit(),
        Event::Timer(_) => {}
        _ => return,
    }
    self.show();
}
```

[`timers::set(REFRESH_MS, true)`](../reference/timers.md#set), in `start`, requests an `Event::Timer` every 30 seconds.  The empty arm falls through to `show`, which recomputes the age.

## Build and run

[Build and install](first-app.md#build) the app, open the app, and press **Brewed**.  The screen shows the time and "Just now"; after a minute, the age starts counting.

??? example "Complete `src/lib.rs` for this chapter"

    ```rust
    use opendeskphone_app_sdk::prelude::*;
    use opendeskphone_app_sdk::{Align, Font, log_info};

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
                Event::Key(Key::Cancel) => return runtime::exit(),
                Event::Timer(_) => {}
                _ => return,
            }
            self.show();
        }
    }

    opendeskphone_app_sdk::export_app! {
        app: FreshPot,
        name: "freshpot",
        display_name: "Fresh Pot",
        description: "When the office coffee was brewed",
    }
    ```
