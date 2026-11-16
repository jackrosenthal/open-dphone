# dphone-app-sdk

The SDK for writing apps for [Open DPhone](https://github.com/jackrosenthal/open-dphone), the open firmware for Digium D-series desk phones.

An app is a WebAssembly component.  The phone calls it once when it starts and then once for every event (a key, a softkey, a timer, a reply from the PBX), and the app answers by calling the phone: it builds its screen from widgets, sets softkeys, starts timers, reads and writes its files, and sends requests to the PBX.  Everything that takes time returns an id at once and delivers its result later as an event, so an app never blocks the phone.

The version of this crate is the version of the firmware whose app interface it is.  The interface itself is `wit/app.wit` in this crate; the firmware generates its side from the same file.

## A minimal app

`Cargo.toml`:

```toml
[package]
name = "hello"
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

`src/lib.rs`:

```rust,ignore
use dphone_app_sdk::prelude::*;

const SK_EXIT: u32 = 1;

#[derive(Default)]
struct Hello;

impl App for Hello {
    fn start(&mut self) {
        let area = screen("Hello");
        let text = ui::create(WidgetKind::Text, ROOT, area);
        ui::set_text(text, "Hello from an app.");
        ui::set_softkeys(ROOT, &[Softkey::new(SK_EXIT, "Exit")]);
    }

    fn handle(&mut self, event: Event) {
        if let Event::Softkey(SK_EXIT) = event {
            runtime::exit();
        }
    }
}

dphone_app_sdk::export_app! {
    app: Hello,
    name: "hello",
    display_name: "Hello",
    description: "Says hello",
}
```

That is the whole app: there is no other file, and no file beside the app on the phone.  An app implements `App` (`start` and `handle`) for a type that has a `Default`, and registers the type, with the app's manifest, in `export_app!`.  The phone makes one instance of it.

The release profile builds for size, as the built-in apps are built: the phone has little memory and little flash, and it compiles the app when it is installed.

## The manifest

The manifest is what the phone has to know about an app before it runs it.  It is written in `export_app!`, after `app` (the app's type), as fields in any order; `name` and `display_name` must be there.  A manifest the phone would refuse does not compile, and the error says why.

| Field | Meaning |
| --- | --- |
| `name` | The app's name on the phone: 1 to 64 characters, the first a letter or digit, the rest letters, digits, `_`, `.`, or `,`.  It cannot be the name of a built-in app.  Installing an app replaces the installed app of the same name. |
| `id` | An identifier of the app, e.g., in reverse domain form (`"org.example.hello"`). |
| `display_name` | What the phone calls the app on screen. |
| `display_name_in` | The same in other languages, by locale: `&[("de_de", "Hallo"), ("es_mx", "Hola")]`. |
| `description`, `description_in` | A line about the app, and the same in other languages. |
| `kind` | `AppKind::Foreground` (the default): the app runs while it is on screen.  `AppKind::Background`: it starts with the phone and runs without being on screen.  `AppKind::Idle`: it provides the idle screen. |
| `has_idle` | `true` if the app draws on the idle screen. |
| `hidden` | `true` marks the app as hidden. |
| `icon` | The app's icon: the content of a PNG file, `include_bytes!("hello.png")`.  At most 256 KiB. |
| `icon_for` | Icons for particular models of phone, each as `icon` is: `&[("D60", include_bytes!("color.png"))]`.  A phone shows its model's icon if there is one, else `icon`.  The models are D40, D45, D50, D60, D62, D65, and D70. |
| `phone_icon` | The name of one of the phone's own icons, for an app without an icon of its own. |
| `memory_kib` | The most memory the app may have, in KiB.  The default is 4 MiB, and the phone gives no app more than 16 MiB. |

A manifest with everything:

```rust,ignore
dphone_app_sdk::export_app! {
    app: Weather,
    name: "weather",
    id: "org.example.weather",
    display_name: "Weather",
    display_name_in: &[("de_de", "Wetter")],
    description: "The weather where the phone is",
    description_in: &[("de_de", "Das Wetter am Ort des Telefons")],
    kind: AppKind::Background,
    has_idle: true,
    icon: include_bytes!("weather.png"),
    icon_for: &[
        ("D60", include_bytes!("weather-color.png")),
        ("D65", include_bytes!("weather-color.png")),
    ],
    memory_kib: 8192,
}
```

The manifest travels in the app's file: `export_app!` writes it, when the app is compiled, into the custom section `dphone-app` of the WebAssembly module, as the protobuf message `dphone.app.v1.Manifest` (`proto/manifest.proto` in this crate).  The phone reads it from there before it installs the app.

## Data

An app's data (pictures, sounds, tables) is in its code: `include_bytes!` and `include_str!`, as in any Rust program.  The phone unpacks nothing.

- A picture: `ui::define_image("logo", include_bytes!("logo.png"))` gives a PNG a name, once; `ui::set_image` and the `icon` of a list item then take that name.  A name the app did not define is one of the phone's own icons.  At most 512 KiB a picture, and 32 pictures.  An app that wants another picture on another model of phone chooses by `ui::metrics()`.
- A sound: `audio::play(include_bytes!("tone.wav"), 0)`.  (`audio::play_file` plays a file from storage, e.g., one the app downloaded.)
- The data counts toward the app's memory (`memory_kib`), since it is part of the program.

## Capabilities

Each interface of the phone except `runtime` is a capability, and each is a module of this crate with the same name.  An app does not declare its capabilities: they are the interfaces it uses (the imports of its WebAssembly component, which hold exactly the interfaces the app calls), and the phone gives it those and no others.

| Capability | What it is for |
| --- | --- |
| `ui` | Widgets, softkeys, keys, and the window. |
| `canvas` | A widget the app draws itself, from filled rectangles, one-color bitmaps, and texts in the skin's fonts. |
| `timers` | One-shot and repeating timers, and the time. |
| `storage` | Files, kept or temporary, of the app's own or shared between apps. |
| `config` | The app's provisioned configuration, the accounts, and settings of the phone that apps may read. |
| `pbx` | Requests to the PBX. |
| `http` | HTTP requests. |
| `calls` | Placing and controlling calls, and call events. |
| `presence` | The presence of other extensions, and the user's own status. |
| `events` | Events pushed to the phone, and messages between apps. |
| `audio` | Playing a sound file. |
| `l10n` | The locale, common words, and the text of dates and times. |
| `hotdesk` | Changing which user the phone is provisioned for. |
| `diagnostics` | The phone's own state: its subsystems and its log. |
| `lamps` | The phone's lamps, while the app is on screen. |
| `input` | Every key and the hookswitch, while the app is on screen and asks for them. |

`runtime` (logging, leaving the screen, exiting, random numbers) is not a capability: every app has it.

The phone stops an app that takes more than two seconds to handle one event, or that asks for more memory than its limit.

## Building

Building an app takes the WebAssembly target of Rust and `wasm-tools`.  Install them once:

```text
rustup target add wasm32-unknown-unknown
cargo install wasm-tools
```

Then, in the app's directory:

```text
cargo build --release --target wasm32-unknown-unknown
wasm-tools component new target/wasm32-unknown-unknown/release/hello.wasm -o hello.wasm
```

The first command builds the crate into a WebAssembly module, which cargo names after the crate (with `_` for every `-` in the crate's name).  The second makes a WebAssembly component of the module, and that is the app: one file, here `hello.wasm`, with the manifest and all of the app's data in it.  The phone goes by the manifest's `name`, not by the name of the file.

The phone compiles the app's code when the app is installed, which takes it about 2 seconds and another 0.12 seconds for every KiB of code.  It takes a file of up to 8 MiB with up to 1 MiB of code in it.

## Installing

In the phone's web interface, the page "Apps" has the button "Install App", which uploads the file.  The phone reads the manifest, compiles the component (calls are not disturbed, and the phone can be used meanwhile), and lists the app with the others; installing an app whose `name` is already installed replaces that app.  "Remove" on the same page removes an installed app with what it stored.

After a firmware update that changes the phone's WebAssembly engine, the phone compiles the installed apps again by itself, one after the other, when it starts.

## License

GPL-2.0-or-later, as the firmware.
