# Manifest

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

`app` comes first and names the type that implements `App`.  The fields follow in any order.  The macro registers the type, embeds the manifest in the app file, and installs a panic hook that logs `panicked at <file>:<line>: <message>` at the error level before the phone stops the app.

## Fields

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `name` | `&str` | Required | Identifies the app, the app's [storage](storage.md), and the app's configuration (`<application id="name">`).  1 to 64 characters: an ASCII letter or digit, then ASCII letters, digits, `_`, `.`, or `,`.  The phone rejects the name of a built-in app, and `dphone`. |
| `id` | `&str` | `""` | An identifier such as `"org.example.weather"`.  The phone stores and reports the value, and otherwise ignores the value. |
| `display_name` | `&str` | Required | The name that the phone shows, exactly as written.  No translations. |
| `description` | `&str` | `""` | One line, shown next to the name in the web interface. |
| `kind` | `AppKind` | `AppKind::Foreground` | See [App lifecycle](index.md#app-lifecycle). |
| `has_idle` | `bool` | `false` | Adds a window on the idle screen ([Idle window](ui.md#idle-window)).  The app then starts with the phone and keeps running off screen, whatever the `kind`. |
| `hidden` | `bool` | `false` | Hides the app from the Applications menu.  `runtime::show_app`, `kind`, and `has_idle` still start the app. |
| `icon` | `&[u8]` | `&[]` | PNG data, starting with the PNG signature, at most 256 KiB: `include_bytes!("weather.png")`. |
| `icon_for` | `&[(&str, &[u8])]` | `&[]` | Icons by model, each under the rules for `icon`: `&[("D60", include_bytes!("color.png"))]`.  Models: `D40`, `D45`, `D50`, `D60`, `D62`, `D65`, and `D70`, each at most once. |
| `phone_icon` | `&str` | `""` | A [built-in app icon](ui.md#built-in-icons) name without the `app-` prefix, for a model with no PNG icon. |
| `memory_kib` | `u32` | `0` | The linear memory limit in KiB.  `0` means 4096 (4 MiB).  Above 16384 (16 MiB) doesn't compile.  Data embedded with `include_bytes!` or `include_str!` counts toward the limit. |

## Icon selection

The Applications menu shows the first of these that exists:

1. The `icon_for` entry for the phone's model.
2. `icon`.
3. The built-in app icon named by `phone_icon`.
4. The built-in app icon named by `name`.
5. The generic built-in app icon.

A PNG icon is scaled down, never up, to the size of the built-in app icons: 48 by 48 pixels on the color models.

## Installation checks

The phone runs these checks in order and reports the first failure:

| Check | Message on failure |
| --- | --- |
| The file is at most 8 MiB. | The file is larger than any app may be. |
| The file is a WebAssembly component or module. | `not an app: an app is a WebAssembly component (a .wasm file made by wasm-tools component new)`.  A ZIP package of a JavaScript app for stock firmware is identified as such. |
| The file has a `dphone-app` section. | `the WebAssembly file has no app manifest`. |
| The phone knows the manifest's kind. | `bad app kind N`. |
| The manifest's format number is 1. | `the app's manifest has format N, and this firmware knows format 1`. |
| `name` follows the rules under [Fields](#fields) and is not `dphone`. | `bad app name: ...`. |
| Each icon is a PNG of at most 256 KiB, and each model in `icon_for` is a real model. | `bad app icon: ...`. |
| The code sections total at most 1 MiB.  Data doesn't count. | The component holds more code than the phone compiles. |
| No built-in app has the same `name`. | `"name" is a built-in app`. |
| The flash has room for the file and the compiled code, with 2 MiB to spare. | Not enough space. |
| The component compiles, imports only interfaces of the firmware, and links against them. | `the app cannot be loaded: ...`, or `the app imports "<name>", which is not an interface of this firmware`. |

Installing an app with the `name` of an installed app replaces that app and keeps the app's storage.  A running instance of the replaced app receives `Event::Exit`, and the new version starts.  Removing an app deletes the app's storage on the phone, but not the app's keys on the PBX.

After a firmware update that changes the WebAssembly engine, the phone recompiles every installed app at startup.  Until an app is recompiled, opening the app fails with `the app is being compiled for this firmware; try again in a moment`.
