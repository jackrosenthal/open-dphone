# Persistent storage

By default, the phone stops an app when the app's window leaves the screen.  That happens after `runtime::exit`, and also when the user leaves the app by pressing line key 1.  Opening the app again starts a new instance with a new `FreshPot::default()`, where `brewed` is `None`.

## Key-value storage

The [`storage`](../reference/storage.md) interface gives each app a private key-value store on the phone, which survives app exits, phone restarts, and installing a newer build of the app.

Define the key at the top of the file:

```rust
/// The storage key for the time the pot was brewed.
const KEY: &str = "brewed";
```

Add two functions to `impl FreshPot`:

```rust
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
```

- `load` uses the SDK helper `read_string`, which returns a key's value as text, or `None` if the key isn't set.
- In `save`, [`storage::set`](../reference/storage.md#set) and [`storage::remove`](../reference/storage.md#remove) return a `Result`, because a write fails when the phone's flash is full.

Add the two new helpers to the imports:

```rust
use dphone_app_sdk::prelude::*;
use dphone_app_sdk::{Align, Font, log_info, log_warn, read_string};
```

## Loading and saving

At the end of `start`, load the time before rendering:

```rust
let _ = timers::set(REFRESH_MS, true);
self.load();
self.show();
```

In `handle`, save after every state change:

```rust
Event::Softkey(SK_BREWED) => {
    self.brewed = Some(timers::wall_time());
    self.save();
    log_info("a pot was brewed");
}
Event::Softkey(SK_EMPTY) => {
    self.brewed = None;
    self.save();
}
```

Save when the state changes, not when the app exits.  The phone does send `Event::Exit` before an orderly stop, but an app that is stopped after a fault gets no event, and neither does an app on a phone that loses power.

## Build and run

[Build and install](first-app.md#build) the app as before.  Open the app, press **Brewed**, press **Exit**, and open the app again.  The time is still there, and the age has advanced.  The time also survives a reboot, which you can trigger with the **Reboot** button in the web interface.

??? example "Complete `src/lib.rs` for this chapter"

    ```rust
    use dphone_app_sdk::prelude::*;
    use dphone_app_sdk::{Align, Font, log_info, log_warn, read_string};

    const SK_BREWED: u32 = 1;
    const SK_EMPTY: u32 = 2;
    const SK_EXIT: u32 = 3;

    /// How often the screen refreshes, in milliseconds.
    const REFRESH_MS: u32 = 30_000;

    /// The storage key for the time the pot was brewed.
    const KEY: &str = "brewed";

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
