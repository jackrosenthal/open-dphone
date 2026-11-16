# Manifest

The manifest describes an app to the phone before the app ever runs: the name, the text shown on screen, the icon, when the app runs, and how much memory the app may use.  An app declares the manifest in `export_app!`, which also registers the type that implements `App`.

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

`app` must come first, and names a type that implements `App` and `Default`.  The manifest fields follow in any order.  `name` and `display_name` are required, and every other field has a default.  Call `export_app!` exactly once per crate.

The macro does two things:

- Exports the component's `guest` interface.  The exported `start` creates the app with `Default::default()` and calls `App::start`, and the exported `handle` calls `App::handle` on that instance.  Before creating the app, `start` installs a panic hook that logs the panic message, file, and line at the error level (through [`runtime::log`](runtime.md#log)).  The phone then stops the app.
- Encodes the manifest at compile time as the protobuf message `dphone.app.v1.Manifest` (`proto/manifest.proto` in the crate) and stores the bytes in the `dphone-app` custom section of the WebAssembly module, where the phone can read them without running the app.

The manifest is validated at compile time.  A failed check is a compile error that starts with `export_app!:` and describes the problem.  The form `export_app!(Type)`, with no fields, does not compile.

## Fields

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `name` | `&str` | Required | The app's name on the phone.  The same name identifies the app's [storage](storage.md) and the app's configuration (`<application id="name">`).  1 to 64 characters: the first must be an ASCII letter or digit, and the rest may be ASCII letters, digits, `_`, `.`, or `,`.  At install time, the phone rejects the name of a built-in app and the name `dphone`. |
| `id` | `&str` | `""` | An identifier for the app, such as a reverse domain name (`"org.example.weather"`).  The phone stores and reports the identifier, and otherwise ignores the value. |
| `display_name` | `&str` | Required | The name that the phone shows for the app, exactly as written.  The manifest has no translations. |
| `description` | `&str` | `""` | A one-line description of the app. |
| `kind` | `AppKind` | `AppKind::Foreground` | When the app runs.  See [App kinds](#app-kinds). |
| `has_idle` | `bool` | `false` | Gives the app a second window on the idle screen.  The app then starts when the phone starts and keeps running when the main window leaves the screen, whatever the `kind`.  See [Idle window](ui.md#idle-window). |
| `hidden` | `bool` | `false` | Hides the app from the Applications menu.  Another app can still open the app with `runtime::show_app`, and `kind` and `has_idle` still start the app. |
| `icon` | `&[u8]` | `&[]` | The app's icon, as the contents of a PNG file: `include_bytes!("weather.png")`.  At most 256 KiB, and the data must begin with the PNG signature. |
| `icon_for` | `&[(&str, &[u8])]` | `&[]` | Icons for specific models, each subject to the same rules as `icon`: `&[("D60", include_bytes!("color.png"))]`.  The models are `D40`, `D45`, `D50`, `D60`, `D62`, `D65`, and `D70`, and each may appear only once. |
| `phone_icon` | `&str` | `""` | The name of one of the phone's app icons, used when the app has no PNG icon for the model. |
| `memory_kib` | `u32` | `0` | The app's linear memory limit, in KiB.  `0` selects the default of 4096 (4 MiB).  Values above 16384 (16 MiB) do not compile. |

## App kinds

```rust
pub enum AppKind { Foreground, Background }
```

| Kind | Behavior |
| --- | --- |
| `Foreground` | The app starts when opened and stops when the window leaves the screen.  Calling `runtime::set_exit_after_background(false)` keeps the app running after that. |
| `Background` | The app starts when the phone starts, with no window on screen, and keeps running when the window leaves the screen. |

An app that draws on the idle screen sets `has_idle`, with either kind.

See [App lifecycle](index.md#app-lifecycle) for the full rules on starting and stopping.

## Icon selection

The Applications menu shows the first of these that exists:

1. The `icon_for` entry for the phone's model.
2. `icon`.
3. The phone's app icon named by `phone_icon`.
4. The phone's app icon with the same name as the app's `name`.
5. The phone's generic app icon.

A PNG icon is scaled down to the size of the phone's app icons (48 by 48 pixels on the color models) and is never scaled up.  The phone's app icons are named `alarms`, `app`, `boxpusher`, `calendar`, `calllog`, `contacts`, `desksparrow`, `diagnostics`, `forward`, `hotdesking`, `info`, `lightup`, `network`, `paging`, `parking`, `queues`, `settings`, `status`, `update`, and `voicemail`.

## Installation checks

The phone installs an app from a single file, the WebAssembly component.  The phone runs these checks in order and gives the listed reason when one fails:

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

The phone compiles the component at install time, on a thread with the lowest priority.  On a D60, compilation takes about 2 seconds plus 0.12 seconds per KiB of code.  After a firmware update that changes the WebAssembly engine, the phone recompiles every installed app at startup.  Until an app's code is ready, opening the app fails with `the app is being compiled for this firmware; try again in a moment`.
