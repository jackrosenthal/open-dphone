# Your first app

## Toolchain

Install the Rust target for WebAssembly, and `wasm-tools`, which turns the module that `cargo` builds into a WebAssembly component:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-tools
```

## Project setup

Create a library crate named `freshpot`:

```sh
cargo new --lib freshpot
cd freshpot
```

Replace the contents of `Cargo.toml` with:

```toml
[package]
name = "freshpot"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
dphone-app-sdk = "3"

[profile.release]
opt-level = "s"
lto = "fat"
codegen-units = 1
panic = "abort"
strip = true
```

- `crate-type = ["cdylib"]` tells `cargo` to produce a WebAssembly module.
- `dphone-app-sdk = "3"` is the SDK for firmware 3.  The SDK's version always matches the firmware version that the SDK targets.

!!! note "Phones running firmware built from source"

    If your phone runs firmware built from source, build against the SDK from the same source.  Clone the repository next to the `freshpot` directory:

    ```sh
    git clone https://github.com/jackrosenthal/open-dphone
    ```

    Then, in `Cargo.toml`, replace the line `dphone-app-sdk = "3"` with a path dependency:

    ```toml
    dphone-app-sdk = { path = "../open-dphone/crates/dphone-app-sdk" }
    ```

## App code

Replace the contents of `src/lib.rs` with:

```rust
use dphone_app_sdk::log_info;
use dphone_app_sdk::prelude::*;

#[derive(Default)]
struct FreshPot;

impl App for FreshPot {
    fn start(&mut self) {
        let area = screen("Fresh Pot");
        let text = ui::create(WidgetKind::Text, ROOT, area);
        ui::set_text(text, "No pot has been brewed yet.");
        log_info("the first screen is built");
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Key(key) if key == "cancel" => runtime::exit(),
            _ => {}
        }
    }
}

dphone_app_sdk::export_app! {
    app: FreshPot,
    name: "freshpot",
    display_name: "Fresh Pot",
    description: "When the office coffee was brewed",
}
```

**The app type.**  `FreshPot` holds the app's state.  When the app starts, the phone creates one value of this type with `Default::default()`.

**The `App` trait.**  The phone calls `start` once, and then calls `handle` once for every event.

- In `start`, [`screen`](../reference/helpers.md#screen) clears the app's window, draws a title, and returns the rectangle below the title.  [`ui::create`](../reference/ui.md#create) then makes a `Text` widget that is a child of the window (`ROOT`) and fills that rectangle.
- `screen` also claims the cancel key (the key marked ✕) for the app, so a press arrives in `handle` as `Event::Key("cancel")`.  [`runtime::exit`](../reference/runtime.md#exit) stops the app and removes the window from the display.

**The manifest.**  `export_app!` registers the type with the phone and embeds the manifest in the app file.

- `name` identifies the app on the phone.  Installing an app replaces any installed app with the same `name`, and the phone rejects the name of a built-in app.
- `display_name` is the name shown in the Applications menu, and `description` is shown next to the name in the web interface.

The [manifest reference](../reference/manifest.md) covers the remaining fields, such as `icon`.

## Build

From the crate's directory, run:

```sh
cargo build --release --target wasm32-unknown-unknown
wasm-tools component new \
    target/wasm32-unknown-unknown/release/freshpot.wasm -o freshpot.wasm
```

The file `freshpot.wasm` is the entire app, including the manifest and any data that the code embeds.

## Installation

1. In a browser, go to the phone's IP address to open the web interface, and sign in.
2. Open the **Apps** page, click **Install App**, and choose `freshpot.wasm`.
3. Wait a few seconds while the phone compiles the app.

To install a newer build, repeat the same three steps.  The phone replaces the installed app and stops the running copy, so open the app again to see the new build.

## First run

On the phone, open the Applications menu (**Applications** in the main menu, or the **Apps** softkey on the idle screen) and choose Fresh Pot.  You can also click **Open** on the **Apps** page of the web interface.  The **Screen** page of the web interface mirrors the phone's display and lets you press the phone's keys from the browser.

<figure markdown="span">
  ![The first screen of Fresh Pot: a title and one line of text](../../assets/app-sdk/first-screen.png){ width="480" }
</figure>

Press the cancel key to exit the app.

## Logs

Open the **Logs** page of the web interface and type `freshpot` in the **Filter** field.  The page shows the lines that the app wrote with `log_info`, along with the lines that the phone wrote about the app:

```text
INFO  dphone_apps::manager app freshpot: installed
INFO  app freshpot: the first screen is built
INFO  dphone_apps::manager app freshpot: started
INFO  dphone_apps::manager app freshpot: stopped
```

When an app panics, the phone stops the app, and the log shows the panic message with the file and line:

```text
ERROR app freshpot: panicked at src/lib.rs:81: the pot is empty
```

A second error line, from the phone, starts with `app freshpot: stopped after a fault` and includes a WebAssembly backtrace.
