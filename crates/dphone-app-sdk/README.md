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

dphone_app_sdk::export_app!(Hello);
```

An app implements `App` (`start` and `handle`) for a type that has a `Default`, and registers the type with `export_app!`.  The phone makes one instance of it.

`app.json`, the manifest:

```json
{
  "name": "hello",
  "id": "org.example.hello",
  "displayName": { "fallback": "Hello" },
  "description": { "fallback": "Says hello" },
  "type": "foreground",
  "wasm": "hello.wasm",
  "capabilities": ["ui"]
}
```

## The manifest

| Field | Meaning |
| --- | --- |
| `name` | The app's name on the phone: 1 to 64 characters, the first a letter or digit, the rest letters, digits, `_`, `.`, or `,`.  It cannot be the name of a built-in app. |
| `id` | An identifier of the app, e.g., in reverse domain form. |
| `displayName`, `description` | Text shown to the user: a string, or an object of texts by locale of which `en_us` or `fallback` is used. |
| `type` | `foreground` (the default), `background`, or `idle`. |
| `hasIdle` | `true` if the app draws on the idle screen. |
| `hidden` | `true` marks the app as hidden. |
| `iconFile` | The name of the app's icon. |
| `wasm` | The file name of the component in the package. |
| `capabilities` | The interfaces the app may use (below). |
| `memoryKiB` | The most memory the app may have.  The default is 4 MiB, and the phone gives no app more than 16 MiB. |

## Capabilities

Each interface of the phone except `runtime` is a capability, and each is a module of this crate with the same name.  An app is given only the capabilities its manifest lists; an app that uses another one does not load.

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

`runtime` (logging, leaving the screen, exiting, random numbers) needs no capability.  `hotdesk` and `diagnostics` are for built-in apps only: a package that asks for one is refused.

The phone stops an app that takes more than two seconds to handle one event, or that asks for more memory than its limit.

## Building

The target is `wasm32-unknown-unknown`, and the crate is a `cdylib`:

```text
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown
```

This makes a core WebAssembly module, `target/wasm32-unknown-unknown/release/hello.wasm`.  The phone takes a component, in a package: a ZIP archive that holds `app.json`, the component, and the app's data files (e.g., PNG pictures), at the top level or in one directory.  A package may hold 64 files and 8 MiB unpacked.  The tool `dphone-apps-build` in the firmware's repository makes both; run in a checkout of the repository:

```text
mkdir pkg
cp <app>/app.json pkg/
cargo run -p dphone-apps --features build-tool --bin dphone-apps-build -- \
    componentize <app>/target/wasm32-unknown-unknown/release/hello.wasm -o pkg/hello.wasm
cargo run -p dphone-apps --features build-tool --bin dphone-apps-build -- \
    pack pkg -o hello.zip
```

Build the app in release mode and for size (`opt-level = "s"`, `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, `strip = true` in `[profile.release]`), as the built-in apps are: the phone has little memory and little flash.

## Installing

In the phone's web interface, the page "Apps" has the button "Install App", which uploads a package.  The phone checks the package, compiles the component, and lists the app with the others; installing a package whose `name` is already installed replaces that app.

The phone can compile a component only if its firmware was built with the on-device compiler (the feature `compile` of the crate `dphone-apps`).  Firmware built without it runs its built-in apps and refuses to install a package.

## License

GPL-2.0-or-later, as the firmware.
