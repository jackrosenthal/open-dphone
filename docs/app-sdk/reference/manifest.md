# Manifest

The manifest is what the phone knows about an app before running the app: the name, the texts shown on screen, the icon, when the app runs, and how much memory the app may use.  An app states the manifest in `export_app!`, which also registers the type that implements `App`.

## `export_app!`

```rust
dphone_app_sdk::export_app! {
    app: Weather,
    name: "weather",
    display_name: "Weather",
    description: "The weather where the phone is",
    kind: AppKind::Background,
    has_idle: true,
    icon: include_bytes!("weather.png"),
    memory_kib: 8192,
}
```

`app` comes first and names a type that implements `App` and `Default`.  The fields of the manifest follow in any order; `name` and `display_name` are required, and every other field has a default.  A crate calls `export_app!` once.

The macro does two things:

- Exports the component's `guest` interface: `start` creates the app with `Default::default()` and calls `App::start`, and `handle` calls `App::handle` on that instance.
- Encodes the manifest at compile time as the protobuf message `dphone.app.v1.Manifest` (`proto/manifest.proto` in the crate) and places the bytes in the custom section `dphone-app` of the WebAssembly module, where the phone reads them without running the app.

The manifest is checked while the crate compiles.  Each check that fails stops the compilation with a message that begins with `export_app!:` and says what is wrong.  The form `export_app!(Type)` without fields does not compile.

## Fields

| Field | Type | Default | Meaning |
| --- | --- | --- | --- |
| `name` | `&str` | required | The app's name on the phone, which also names the app's [storage](storage.md) and the app's configuration (`<application id="name">`).  1 to 64 characters; the first is an ASCII letter or digit, and the rest are ASCII letters, digits, `_`, `.`, or `,`. |
| `id` | `&str` | `""` | An identifier for the app, e.g., in reverse domain form (`"org.example.weather"`).  The phone stores and reports the identifier and does nothing else with the value. |
| `display_name` | `&str` | required | The name the phone shows for the app. |
| `display_name_in` | `&[(&str, &str)]` | `&[]` | Names by locale, e.g., `&[("de_de", "Wetter")]`.  Each locale and each text must be nonempty.  See [Which text the phone shows](#which-text-the-phone-shows). |
| `description` | `&str` | `""` | One line about the app. |
| `description_in` | `&[(&str, &str)]` | `&[]` | Descriptions by locale, with the same rules as `display_name_in`. |
| `kind` | `AppKind` | `AppKind::Foreground` | When the app runs.  See [Kinds](#kinds). |
| `has_idle` | `bool` | `false` | The app gets a second window, on the idle screen, and starts when the phone starts.  See [the idle window](ui.md#the-idle-window). |
| `hidden` | `bool` | `false` | The phone's Apps menu does not list the app.  Another app can still open the app with `runtime::show_app`, and `kind` and `has_idle` still start the app. |
| `icon` | `&[u8]` | `&[]` | The app's icon: the content of a PNG file, e.g., `include_bytes!("weather.png")`.  At most 256 KiB, and the content must begin with the PNG signature. |
| `icon_for` | `&[(&str, &[u8])]` | `&[]` | Icons for particular models, each under the rules of `icon`: `&[("D60", include_bytes!("color.png"))]`.  The models are `D40`, `D45`, `D50`, `D60`, `D62`, `D65`, and `D70`, and a model may appear once. |
| `phone_icon` | `&str` | `""` | The name of one of the phone's app icons, used when the app has no PNG icon for the model. |
| `memory_kib` | `u32` | `0` | The most linear memory the app may have, in KiB.  `0` means the default of 4096 (4 MiB).  The largest value that compiles is 16384 (16 MiB). |

## Kinds

```rust
pub enum AppKind { Foreground, Background, Idle }
```

| Kind | Behavior |
| --- | --- |
| `Foreground` | The app starts when opened and is stopped when the app's window leaves the screen.  `runtime::set_exit_after_background(false)` keeps the app running after that. |
| `Background` | The app starts when the phone starts, without a window on screen, and keeps running when the window leaves the screen. |
| `Idle` | The app starts when opened and keeps running when the window leaves the screen.  The phone does not start an `Idle` app when the phone starts and gives the kind no part of the idle screen; an app that draws on the idle screen sets `has_idle`. |

The page [Reference](index.md#lifetime-of-an-app) describes starting and stopping in full.

## Which icon the phone shows

The Apps menu shows, in this order of preference:

1. The entry of `icon_for` for the phone's model.
2. `icon`.
3. The phone's app icon named by `phone_icon`.
4. The phone's app icon that has the app's `name`.
5. The phone's generic app icon.

A PNG icon is reduced to the size of the phone's app icons (48 by 48 pixels on the color models) and never enlarged.  The phone's app icons are named `alarms`, `app`, `boxpusher`, `calendar`, `calllog`, `contacts`, `desksparrow`, `diagnostics`, `forward`, `hotdesking`, `info`, `lightup`, `network`, `paging`, `parking`, `queues`, `settings`, `status`, `update`, and `voicemail`.

## Which text the phone shows

The phone shows the entry of `display_name_in` whose locale is `en_us` (in any letter case) when there is one, and `display_name` otherwise; `description` and `description_in` work the same way.  The phone keeps the entries for other locales with the app and does not show them.

## What the phone checks at installation

The phone installs an app from one file, the WebAssembly component.  The checks, in order, and the reason the phone gives for each refusal:

| Check | Refusal |
| --- | --- |
| The file is at most 8 MiB. | The file is larger than any app may be. |
| The file is a WebAssembly component or module. | `not an app: an app is a WebAssembly component (a .wasm file made by wasm-tools component new)`.  A ZIP package of a JavaScript app for stock firmware is named as such. |
| The file has a `dphone-app` section. | `the WebAssembly file has no app manifest`. |
| The manifest's format number is 1. | `the app's manifest has format N, and this firmware knows format 1`. |
| `name` follows the rule above. | `bad app name: ...`. |
| Each icon is a PNG of at most 256 KiB, and each model of `icon_for` is a model. | `bad app icon: ...`. |
| The code sections of the component total at most 1 MiB.  Data does not count. | The component holds more code than the phone compiles. |
| No built-in app has the same `name`. | `"name" is a built-in app`. |
| The flash has room for the file and the compiled code, with 2 MiB left free. | Not enough space. |
| The component compiles, imports only interfaces of the firmware, and links against them. | `the app cannot be loaded: ...`, or `the app imports "<name>", which is not an interface of this firmware`. |

Installing an app whose `name` is already installed replaces the installed app; the app's storage is kept.  If the replaced app was running, the phone sends `Event::Exit`, drops the instance, and starts the new version.  Removing an app deletes the app's storage on the phone; the app's keys on the PBX stay.

The phone compiles the component when the app is installed, on a thread of the lowest priority.  On a D60 the compilation takes about 2 seconds plus 0.12 seconds for every KiB of code.  After a firmware update that changes the WebAssembly engine, the phone compiles each installed app again when the phone starts; until an app's code is ready, opening the app fails with `the app is being compiled for this firmware; try again in a moment`.
