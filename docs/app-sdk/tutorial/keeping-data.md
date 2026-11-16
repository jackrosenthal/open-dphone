# Keeping data

At the end of [the previous step](softkeys-and-state.md), the app forgot the pot whenever the app left the screen.  This step explains why, and stores the time on the phone so that the time outlasts the app.

## Why the time was lost

The phone ends an app such as Fresh Pot when the app's window leaves the screen: after `runtime::exit`, and also when the user leaves with line key 1.  The next opening starts a new instance, with a new `FreshPot::default()`, in which `brewed` is `None`.

That lifetime belongs to an app of the kind `Foreground`, which is the kind of an app whose manifest names no other.  The reference describes [the other kinds](../reference/index.md#lifetime-of-an-app), which keep running off screen.  A board for a coffee pot has no work to do off screen, so Fresh Pot stays a `Foreground` app and stores what must last.

## The app's store

The phone keeps a store of keys and values for each app, in the interface [`storage`](../reference/storage.md).  A key is a short name, and a value is any bytes up to 1 MiB.  The phone keeps the store when the app ends, when the phone restarts, when the firmware is updated, and when a newer build of the app is installed; no other app can read the store; and removing the app removes the store.

Fresh Pot needs one key.  Name the key at the top of the file:

```rust
/// The key of the phone's store that holds when the pot was brewed.
const KEY: &str = "brewed";
```

Add two functions to `impl FreshPot`, one for each direction:

```rust
/// Reads when the pot was brewed from the phone's store.
fn load(&mut self) {
    self.brewed = read_string(KEY).and_then(|text| text.parse().ok());
}

/// Writes when the pot was brewed to the phone's store.
fn save(&self) {
    let result = match self.brewed {
        Some(at) => storage::set(KEY, at.to_string().as_bytes()),
        None => storage::remove(KEY),
    };
    if let Err(e) = result {
        log_warn(&format!("the time was not stored: {e:?}"));
    }
}
```

- `load` uses the SDK's helper `read_string`, which returns the value of a key as text, or `None` when the key is not set.  A value that does not parse as a number also leaves `brewed` as `None`, which is the right answer for a store that an older build wrote differently.
- `save` writes the time as text with [`storage::set`](../reference/storage.md#set), or removes the key with [`storage::remove`](../reference/storage.md#remove) when there is no pot.  Both return a `Result`: a write fails when the phone's flash is full.  The app has no better answer to that failure than a line in the log, which `log_warn` writes.

The imports grow by the two helpers:

```rust
use dphone_app_sdk::prelude::*;
use dphone_app_sdk::{Align, Font, log_info, log_warn, read_string};
```

## Load at the start, save at each change

At the end of `start`, load the time before showing the state:

```rust
let _ = timers::set(REFRESH_MS, true);
self.load();
self.show();
```

In `handle`, save after each change of the state:

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

The app saves at the moment of the change, and not when the app ends.  The phone does send `Event::Exit` before ending an app in an orderly way, but an app that the phone stops after a fault receives no event, and neither does an app on a phone that loses power.

## No permission to declare

The app now uses a second interface of the phone, and nothing in `Cargo.toml` or in the manifest says so.  The component that `wasm-tools` writes imports exactly the interfaces whose functions the app calls, and the phone gives an installed app those interfaces and no others.

## Build, install, and try

Build and install as before:

```sh
cargo build --release --target wasm32-unknown-unknown
wasm-tools component new \
    target/wasm32-unknown-unknown/release/freshpot.wasm -o freshpot.wasm
```

Open the app, press **Brewed**, press **Exit**, and open the app again: the time is still there, and the age has moved on.  The time also survives a reboot of the phone (the button **Reboot** of the web interface).

Next, [Drawing on a canvas](drawing.md) turns the age into a picture.

??? example "The whole `src/lib.rs` after this step"

    ```rust
    use dphone_app_sdk::prelude::*;
    use dphone_app_sdk::{Align, Font, log_info, log_warn, read_string};

    const SK_BREWED: u32 = 1;
    const SK_EMPTY: u32 = 2;
    const SK_EXIT: u32 = 3;

    /// How often the screen is brought up to date, in milliseconds.
    const REFRESH_MS: u32 = 30_000;

    /// The key of the phone's store that holds when the pot was brewed.
    const KEY: &str = "brewed";

    #[derive(Default)]
    struct FreshPot {
        /// When the pot was brewed, in seconds since the Unix epoch.
        brewed: Option<u64>,
        /// The line that says when the pot was brewed.
        status: Widget,
        /// The line that says how long ago.
        age: Widget,
    }

    impl FreshPot {
        /// Reads when the pot was brewed from the phone's store.
        fn load(&mut self) {
            self.brewed = read_string(KEY).and_then(|text| text.parse().ok());
        }

        /// Writes when the pot was brewed to the phone's store.
        fn save(&self) {
            let result = match self.brewed {
                Some(at) => storage::set(KEY, at.to_string().as_bytes()),
                None => storage::remove(KEY),
            };
            if let Err(e) = result {
                log_warn(&format!("the time was not stored: {e:?}"));
            }
        }

        /// Shows the state on the screen.
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
