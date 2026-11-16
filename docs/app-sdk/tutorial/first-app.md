# A first app

This step goes the whole way once, with the smallest app that can be seen on a phone: install the tools, create the crate, write a screen with one line of text, build the app file, install the file on a phone, open the app, and read the app's log.  The later steps change the code and repeat the build and the installation.

## Install the tools

An app is built in two moves: `cargo` compiles the crate to a WebAssembly module, and `wasm-tools` makes a WebAssembly component of the module.  Install the Rust target for the first move and the command for the second:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-tools
```

`cargo install` compiles `wasm-tools` from source, which takes a few minutes.

## Create the crate

Create a library crate named `freshpot`:

```sh
cargo new --lib freshpot
cd freshpot
```

Replace the content of `Cargo.toml` with:

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

Three parts of the file matter to the phone:

- `crate-type = ["cdylib"]` makes `cargo` write a WebAssembly module, which a plain library crate does not produce.
- `dphone-app-sdk = "3"` is the SDK for firmware 3.  The SDK's version is the version of the firmware whose interface the SDK describes.
- The release profile builds for size.  The phone compiles an app when the app is installed, and a smaller app installs faster and leaves more of the phone's memory free.  With `panic = "abort"`, the app carries no code for unwinding; a panic stops an app on the phone either way.

!!! note "Building against the SDK in a clone of the repository"

    Each release of the firmware publishes `dphone-app-sdk` on crates.io under the release's version.  For a phone that runs firmware built from source, depend on the SDK of the same source instead.  Clone the repository beside the directory `freshpot`:

    ```sh
    git clone https://github.com/jackrosenthal/open-dphone
    ```

    Then name the crate by path in `Cargo.toml`, in place of the line `dphone-app-sdk = "3"`:

    ```toml
    dphone-app-sdk = { path = "../open-dphone/crates/dphone-app-sdk" }
    ```

## Write the app

Replace the content of `src/lib.rs` with:

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

The file has three parts.

**The app's type.**  `FreshPot` holds the app's state, of which this step has none.  The phone creates one value of the type with `Default::default()` when the app starts.

**The trait `App`.**  The phone calls `start` once, and then calls `handle` once for every event.

- `start` builds the screen.  [`screen`](../reference/helpers.md#screen) clears the app's window, writes a title, and returns the rectangle under the title.  [`ui::create`](../reference/ui.md#create) creates a widget: here a `Text`, as a child of the window (`ROOT`), filling that rectangle.  [`ui::set_text`](../reference/ui.md#set_text) gives the widget a line to show.
- `log_info` writes a line to the phone's log, which [the last part of this step](#read-the-log) reads.
- `handle` receives the events.  `screen` reserves the cancel key (the key marked ✕) for the app, so the key arrives as `Event::Key("cancel")`, and the app answers with [`runtime::exit`](../reference/runtime.md#exit), which ends the app and takes the app's window off the display.

**The manifest.**  `export_app!` registers the type with the phone and writes the manifest into the app file: what the phone must know about an app before running the app.

- `name` identifies the app on the phone.  Installing an app replaces the installed app of the same `name`, and a name of a built-in app is refused.
- `display_name` is the name that the phone shows in the Applications menu.
- `description` is one line that the web interface shows beside the name.

The [manifest reference](../reference/manifest.md) lists the other fields, e.g., `icon`.  A manifest with a malformed field, e.g., a `name` with a space, does not compile, and the compiler's error says why.

## Build the app file

In the crate's directory, run:

```sh
cargo build --release --target wasm32-unknown-unknown
wasm-tools component new \
    target/wasm32-unknown-unknown/release/freshpot.wasm -o freshpot.wasm
```

The first command writes the module to `target/wasm32-unknown-unknown/release/freshpot.wasm`; `cargo` names the module after the crate, with `_` for each `-`.  The second command writes the component to `freshpot.wasm` in the crate's directory.  That file, of about 30 KiB, is the whole app: the code, the manifest, and any data that the code includes.

## Install the app on the phone

1. In a browser, open the phone's web interface at the phone's IP address, and sign in.
2. Open the page **Apps**, press **Install App**, and choose `freshpot.wasm`.
3. Wait while the phone compiles the app, which takes a few seconds for an app of this size.  The phone can be used meanwhile.

The list on the page now has the entry "Fresh Pot", with the name `freshpot`, the description, and the buttons **Open** and **Remove**.

Installing a later build is the same three actions.  The phone replaces the installed app and ends the running copy, so open the app again to see the new build.

## Open the app

On the phone, open the Applications menu (**Applications** in the main menu, or the softkey **Apps** of the idle screen) and choose Fresh Pot.  **Open** on the web interface's page Apps does the same from the browser, and the page **Screen** shows the phone's display and presses the phone's keys.

<figure markdown="span">
  ![The first screen of Fresh Pot: a title and one line of text](../../assets/app-sdk/first-screen.png){ width="480" }
  <figcaption>The app on a D60.  The softkeys are empty, because the app has not set any.</figcaption>
</figure>

Press the cancel key to end the app.

## Read the log

Open the page **Logs** of the web interface, and type `freshpot` in the field **Filter**.  The page shows the lines that the app wrote, under the source `app` and prefixed with the app's name, and the lines that the phone wrote about the app, among them:

```text
INFO  dphone_apps::manager app freshpot: installed
INFO  app freshpot: the first screen is built
INFO  dphone_apps::manager app freshpot: started
INFO  dphone_apps::manager app freshpot: stopped
```

`log_info` has the siblings `log_error`, `log_warn`, and `log_debug`, and the page's level selector hides the levels below the chosen one.

When an app panics, the phone ends the app and writes two error lines.  The first comes from the app, through a hook that `export_app!` installs, and carries the panic's message with the file and the line:

```text
ERROR app freshpot: panicked at src/lib.rs:81: the pot is empty
```

The second line comes from the phone, begins `app freshpot: stopped after a fault`, and carries a WebAssembly backtrace.

## Next

The app shows a fixed line and knows nothing about coffee.  [Softkeys, keys, and state](softkeys-and-state.md) gives the app softkeys, a memory of when the pot was brewed, and a clock.
