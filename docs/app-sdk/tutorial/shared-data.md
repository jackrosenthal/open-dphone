# Data shared by every phone

A coffee pot serves an office, not one desk.  This last step moves the pot's time to the PBX, so that a press of Brewed on any phone shows on every phone, together with the name of the person who pressed.

<figure markdown="span">
  ![Fresh Pot showing a pot that Alice brewed twelve minutes ago](../../assets/app-sdk/shared.png){ width="480" }
  <figcaption>Bob's phone, twelve minutes after Alice pressed Brewed on another phone.</figcaption>
</figure>

## The app's store on the PBX

The interface [`pbx-storage`](../reference/pbx-storage.md) (the Rust module `pbx_storage`) is a second store of keys and values, kept by the PBX.  The store needs a PBX that runs [Open DPMA](../../open-dpma/index.md), and needs no configuration in the app or on the phone.  An app has two sets of keys there:

- `PbxScope::Shared`: one set for the copies of the app on all phones of the PBX.  Fresh Pot keeps the pot here.
- `PbxScope::User`: one set for each user, the same on every phone of that user.  A game keeps a player's progress here.

Two things differ from the store on the phone:

- **A request does not return the answer.**  The PBX is across the network, and no call of an app may block.  `get`, `set`, and `remove` return a request number at once, and the answer arrives later as `Event::PbxStorage` with the same number.
- **The PBX records who wrote each entry.**  An entry comes back with the value and with the name of the user who wrote the value, which the PBX fills in from the user that the writing phone is provisioned for.  Fresh Pot shows who brewed the pot without ever asking for a name.

## Plan

The store on the phone stays, as the app's memory between answers from the PBX:

1. At the start, the app shows what the phone's store holds, and asks the PBX.
2. Whatever the PBX answers replaces the app's state, and is saved to the phone's store.
3. Brewed and Empty change the state at once, save, and send the change to the PBX.
4. When another phone changes the pot, the PBX tells this phone, and the app asks again.

On a phone whose PBX has no store, every request ends `Unreachable`, and the app behaves as at the end of the previous step.

## State for the conversation

The same key name serves in both stores.  Change the comment on the constant to say so:

```rust
/// The key that holds when the pot was brewed, in the phone's store and
/// in the store on the PBX.
const KEY: &str = "brewed";
```

Add two fields to `FreshPot`, for the name and for the number of the request that the app is waiting on:

```rust
/// The name of the user who brewed the pot, once the PBX has told.
by: String,
/// The request to the PBX whose result is awaited.
asked: u32,
```

## Ask, and take the answer

Add two functions to `impl FreshPot`:

```rust
/// Asks the PBX when the pot was brewed.
fn ask(&mut self) {
    self.asked = pbx_storage::get(PbxScope::Shared, KEY);
}

/// Takes what the PBX holds: the entry of the key, or `None` when the
/// key is not set.
fn take(&mut self, entry: Option<PbxEntry>) {
    self.brewed = entry
        .as_ref()
        .and_then(|e| str::from_utf8(&e.value).ok()?.parse().ok());
    self.by = entry.map(|e| e.author_name).unwrap_or_default();
    self.save();
}
```

`ask` sends a `get` and remembers the request number.  `take` receives a [`PbxEntry`](../reference/pbx-storage.md#requests-and-outcomes), or `None` when the key is not set on the PBX: the entry's `value` is the bytes that a phone wrote (here the time as text), and `author_name` is the name that the PBX recorded for the writer.

In `show`, add the name to the second line.  Replace the line that sets the text of `self.age` with:

```rust
let mut age = age_text(minutes);
if !self.by.is_empty() {
    age = format!("{age}, by {}", self.by);
}
ui::set_text(self.age, &age);
```

## Start the conversation

At the end of `start`, after showing what the phone's store holds, turn on notices of changes and ask the PBX:

```rust
self.load();
self.show();

pbx_storage::watch(true);
self.ask();
```

After [`pbx_storage::watch(true)`](../reference/pbx-storage.md#watch), the app receives `Event::PbxStorageChanged` whenever another phone sets or removes one of the app's shared keys, for as long as the app runs.

## Handle the answers

Replace `handle` with:

```rust
fn handle(&mut self, event: Event) {
    match event {
        Event::Softkey(SK_BREWED) => {
            let now = timers::wall_time();
            self.brewed = Some(now);
            self.by = pbx_storage::user()
                .map(|user| user.name)
                .unwrap_or_default();
            self.save();
            self.asked =
                pbx_storage::set(PbxScope::Shared, KEY, now.to_string().as_bytes(), None);
            log_info("a pot was brewed");
        }
        Event::Softkey(SK_EMPTY) => {
            self.brewed = None;
            self.save();
            self.asked = pbx_storage::remove(PbxScope::Shared, KEY, None);
        }
        Event::PbxStorage((id, outcome)) if id == self.asked => match outcome {
            PbxStorageOutcome::Entry(entry) => self.take(entry),
            PbxStorageOutcome::Stored(entry) => self.take(Some(entry)),
            PbxStorageOutcome::Removed => self.take(None),
            // The phone shows what the phone's store holds.
            PbxStorageOutcome::Unreachable(why) => log_info(&format!("not shared: {why}")),
            other => log_warn(&format!("the PBX answered {other:?}")),
        },
        // Another phone set or removed the key, or the PBX is in reach
        // again.
        Event::PbxStorageChanged(_) | Event::PbxStorageOnline => return self.ask(),
        Event::Softkey(SK_EXIT) => return runtime::exit(),
        Event::Key(key) if key == "cancel" => return runtime::exit(),
        Event::Timer(_) => {}
        _ => return,
    }
    self.show();
}
```

The softkeys now also write to the PBX:

- **Brewed** sets the key to the time.  [`pbx_storage::user`](../reference/pbx-storage.md#user) is the phone's user as the PBX knows the user, so the name shows at once, before the PBX answers.  The last argument of `set`, `None`, means that the write replaces whatever the key holds; for a pot of coffee, the latest press is the truth.  An app in which two phones must not overwrite each other, e.g., a shared list, passes the version of the entry that the app read, as [Two phones that write at the same time](../reference/pbx-storage.md#two-phones-that-write-at-the-same-time) describes.
- **Empty** removes the key.

Three new arms handle what comes back:

- **`Event::PbxStorage`** is the answer to a request.  The guard `id == self.asked` drops the answer to any request but the latest.  `Entry` answers a `get`, `Stored` answers a `set` with the entry as the PBX now holds the key, and `Removed` answers a `remove`; all three end in `take`.  `Unreachable` means that the PBX could not be asked or did not answer; the app logs the reason and goes on showing what the phone's store holds.  Of the remaining outcomes, only `Refused` can follow these requests (the PBX refuses, e.g., a value of more than 4096 bytes), and the app logs the outcome.
- **`Event::PbxStorageChanged`** says that another phone wrote the key.  The event carries no value, so the app asks.
- **`Event::PbxStorageOnline`** says that the store is in reach, at first or again.  What the app read before may be out of date, so the app asks.

Both of the last two arms return without calling `show`: nothing has changed until the answer arrives.

!!! note "A press while the PBX is out of reach"

    The phone does not keep a failed write to send later.  In Fresh Pot, a press of Brewed while the PBX is out of reach therefore shows only on that phone, and gives way to what the PBX holds once the PBX is in reach again.  An app that must not lose a write keeps the write in `storage` and sends the write again at `Event::PbxStorageOnline`; see [When the PBX is out of reach](../reference/pbx-storage.md#when-the-pbx-is-out-of-reach).

## Build, install, and try

Build as before, and install the app file on two phones of the same PBX:

```sh
cargo build --release --target wasm32-unknown-unknown
wasm-tools component new \
    target/wasm32-unknown-unknown/release/freshpot.wasm -o freshpot.wasm
```

Open the app on both phones, and press **Brewed** on one.  The other phone shows the same time, with the name of the first phone's user, a moment later.  Press **Empty** on either phone, and both return to "No pot".  With one phone, the second line shows the name of that phone's user after Brewed, and the Logs page shows `not shared:` and a reason when the phone has no store to reach.

## Where to go from here

Fresh Pot is complete: about 250 lines of Rust in one file, running on every model.  Further directions, each with a page in the reference:

- **An icon.**  The manifest's fields `icon` and `icon_for` give the app a picture in the Applications menu, from PNG files that the crate includes; see [Manifest](../reference/manifest.md).
- **Lists and inputs.**  The widget kinds `TextList` and `Input` handle selection and typing; see [`ui`](../reference/ui.md#widget-kinds).
- **Calls and presence.**  An app can place calls, watch call states, and read the presence of the contacts that the phone watches; see [`calls`](../reference/calls.md) and [`presence`](../reference/presence.md).
- **The network.**  [`http`](../reference/http.md) sends HTTP requests, with the same pattern of a request number and a later event as `pbx_storage`.
- **Running off screen.**  A `Background` app starts with the phone and keeps running; see [Lifetime of an app](../reference/index.md#lifetime-of-an-app).

The source of the [built-in apps](https://github.com/jackrosenthal/open-dphone/tree/main/apps) shows many of these interfaces in use.

??? example "The whole `src/lib.rs` after this step"

    ```rust
    use dphone_app_sdk::canvas::{self, Bitmap, Op, Role};
    use dphone_app_sdk::prelude::*;
    use dphone_app_sdk::{Align, Font, log_info, log_warn, read_string};

    const SK_BREWED: u32 = 1;
    const SK_EMPTY: u32 = 2;
    const SK_EXIT: u32 = 3;

    /// How often the screen is brought up to date, in milliseconds.
    const REFRESH_MS: u32 = 30_000;

    /// The key that holds when the pot was brewed, in the phone's store and
    /// in the store on the PBX.
    const KEY: &str = "brewed";

    /// After this many minutes, a pot is stale.
    const FRESH_MINUTES: u64 = 60;

    /// A mug, 12 pixels wide and 10 pixels high.  Each row is two bytes, and
    /// the leftmost pixel of a row is the highest bit of the row's first byte.
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
        /// The name of the user who brewed the pot, once the PBX has told.
        by: String,
        /// The request to the PBX whose result is awaited.
        asked: u32,
        /// The line that says when the pot was brewed.
        status: Widget,
        /// The line that says how long ago.
        age: Widget,
        /// The canvas under the two lines, and the size of the canvas.
        gauge: Widget,
        gauge_width: u16,
        gauge_height: u16,
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

        /// Asks the PBX when the pot was brewed.
        fn ask(&mut self) {
            self.asked = pbx_storage::get(PbxScope::Shared, KEY);
        }

        /// Takes what the PBX holds: the entry of the key, or `None` when the
        /// key is not set.
        fn take(&mut self, entry: Option<PbxEntry>) {
            self.brewed = entry
                .as_ref()
                .and_then(|e| str::from_utf8(&e.value).ok()?.parse().ok());
            self.by = entry.map(|e| e.author_name).unwrap_or_default();
            self.save();
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
                    let mut age = age_text(minutes);
                    if !self.by.is_empty() {
                        age = format!("{age}, by {}", self.by);
                    }
                    ui::set_text(self.age, &age);
                }
                None => {
                    ui::set_text(self.status, "No pot");
                    ui::set_text(self.age, "Press Brewed after making coffee.");
                }
            }
            if let Err(e) = canvas::draw(self.gauge, &self.gauge_ops()) {
                log_warn(&format!("the gauge was not drawn: {e}"));
            }
        }

        /// The drawing on the canvas: a mug, and a bar that empties while the
        /// pot goes stale.  Without a pot, the canvas shows nothing.
        fn gauge_ops(&self) -> Vec<Op> {
            let Some(at) = self.brewed else {
                return Vec::new();
            };
            let minutes = timers::wall_time().saturating_sub(at) / 60;
            let fresh = FRESH_MINUTES.saturating_sub(minutes);

            // The mug fills the height of the canvas.
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

            // The bar takes the rest of the width: a frame one pixel wide, and
            // inside the frame, the part of the hour that is left.
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

            // The canvas: half as wide as the window, in the middle, and two
            // lines high.
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

            pbx_storage::watch(true);
            self.ask();
        }

        fn handle(&mut self, event: Event) {
            match event {
                Event::Softkey(SK_BREWED) => {
                    let now = timers::wall_time();
                    self.brewed = Some(now);
                    self.by = pbx_storage::user()
                        .map(|user| user.name)
                        .unwrap_or_default();
                    self.save();
                    self.asked =
                        pbx_storage::set(PbxScope::Shared, KEY, now.to_string().as_bytes(), None);
                    log_info("a pot was brewed");
                }
                Event::Softkey(SK_EMPTY) => {
                    self.brewed = None;
                    self.save();
                    self.asked = pbx_storage::remove(PbxScope::Shared, KEY, None);
                }
                Event::PbxStorage((id, outcome)) if id == self.asked => match outcome {
                    PbxStorageOutcome::Entry(entry) => self.take(entry),
                    PbxStorageOutcome::Stored(entry) => self.take(Some(entry)),
                    PbxStorageOutcome::Removed => self.take(None),
                    // The phone shows what the phone's store holds.
                    PbxStorageOutcome::Unreachable(why) => log_info(&format!("not shared: {why}")),
                    other => log_warn(&format!("the PBX answered {other:?}")),
                },
                // Another phone set or removed the key, or the PBX is in reach
                // again.
                Event::PbxStorageChanged(_) | Event::PbxStorageOnline => return self.ask(),
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
