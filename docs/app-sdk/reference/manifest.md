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

`app` must come first, and names the type that implements `App`.  The manifest fields follow in any order.  The macro registers the type with the phone and embeds the manifest in the app file.

`export_app!` also installs a panic hook, which writes `panicked at <file>:<line>: <message>` to the log at the error level before the phone stops the app.

## Fields

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `name` | `&str` | Required | Identifies the app on the phone, and also the app's [storage](storage.md) and configuration (`<application id="name">`).  1 to 64 characters: the first must be an ASCII letter or digit, and the rest may be ASCII letters, digits, `_`, `.`, or `,`.  At install time, the phone rejects the name of a built-in app and the name `dphone`. |
| `id` | `&str` | `""` | An identifier for the app, such as a reverse domain name (`"org.example.weather"`).  The phone stores and reports the identifier, and otherwise ignores the value. |
| `display_name` | `&str` | Required | The name that the phone shows for the app, exactly as written.  The manifest has no translations. |
| `description` | `&str` | `""` | A one-line description, shown next to the name in the web interface. |
| `kind` | `AppKind` | `AppKind::Foreground` | When the app runs.  See [App lifecycle](index.md#app-lifecycle). |
| `has_idle` | `bool` | `false` | Gives the app a second window on the idle screen.  The app then starts when the phone starts and keeps running when the main window leaves the screen, whatever the `kind`.  See [Idle window](ui.md#idle-window). |
| `hidden` | `bool` | `false` | Hides the app from the Applications menu.  Another app can still open the app with `runtime::show_app`, and `kind` and `has_idle` still start the app. |
| `icon` | `&[u8]` | `&[]` | The app's icon, as the contents of a PNG file: `include_bytes!("weather.png")`.  At most 256 KiB, and the data must begin with the PNG signature. |
| `icon_for` | `&[(&str, &[u8])]` | `&[]` | Icons for specific models, each subject to the same rules as `icon`: `&[("D60", include_bytes!("color.png"))]`.  The models are `D40`, `D45`, `D50`, `D60`, `D62`, `D65`, and `D70`, and each may appear only once. |
| `phone_icon` | `&str` | `""` | The name of one of the phone's [app icons](ui.md#built-in-icons), without the `app-` prefix, used when the app has no PNG icon for the model. |
| `memory_kib` | `u32` | `0` | The app's linear memory limit, in KiB.  `0` selects the default of 4096 (4 MiB).  Values above 16384 (16 MiB) do not compile.  Data that the app embeds with `include_bytes!` or `include_str!` counts toward the limit. |

## Icon selection

The Applications menu shows the first of these that exists:

1. The `icon_for` entry for the phone's model.
2. `icon`.
3. The phone's app icon named by `phone_icon`.
4. The phone's app icon with the same name as the app's `name`.
5. The phone's generic app icon.

A PNG icon is scaled down to the size of the phone's app icons (48 by 48 pixels on the color models) and is never scaled up.

## Installation checks

The phone runs these checks in order when installing an app, and reports the first that fails:

| Check | Message on failure |
| --- | --- |
| The file is at most 8 MiB. | The file is larger than any app may be. |
| The file is a WebAssembly component or module. | `not an app: an app is a WebAssembly component (a .wasm file made by wasm-tools component new)`.  A ZIP package of a JavaScript app for stock firmware is identified as such. |
| The file has a `dphone-app` section. | `the WebAssembly file has no app manifest`. |
| The manifest's kind is one that the phone knows. | `bad app kind N`. |
| The manifest's format number is 1. | `the app's manifest has format N, and this firmware knows format 1`. |
| `name` follows the rules under [Fields](#fields) and is not `dphone`. | `bad app name: ...`. |
| Each icon is a PNG of at most 256 KiB, and each model in `icon_for` is a real model. | `bad app icon: ...`. |
| The component's code sections total at most 1 MiB.  Data doesn't count. | The component holds more code than the phone compiles. |
| No built-in app has the same `name`. | `"name" is a built-in app`. |
| The flash has room for the file and the compiled code, with 2 MiB to spare. | Not enough space. |
| The component compiles, imports only interfaces of the firmware, and links against them. | `the app cannot be loaded: ...`, or `the app imports "<name>", which is not an interface of this firmware`. |

Installing an app with the `name` of an installed app replaces that app and keeps the app's storage.  If the replaced app was running, the phone sends `Event::Exit`, discards the instance, and starts the new version.  Removing an app deletes the app's storage on the phone, but the app's keys on the PBX remain.

After a firmware update that changes the WebAssembly engine, the phone recompiles every installed app at startup.  Until an app's code is ready, opening the app fails with `the app is being compiled for this firmware; try again in a moment`.
