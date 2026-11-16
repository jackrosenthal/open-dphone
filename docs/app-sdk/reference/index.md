# Reference

The reference describes the interface between an app and a phone that runs Open DPhone: every function an app can call, every event an app receives, the manifest, and the limits the phone enforces.  The interface is defined in one file, `wit/app.wit` in the crate `dphone-app-sdk` (the package `dphone:app@3.0.0`), and the firmware generates the phone's side from the same file.  The Rust names on these pages are the names the crate `dphone-app-sdk` exposes; where the name in `app.wit` differs by more than `-` against `_`, the page gives both.

## How an app and the phone talk

An app is a WebAssembly component that exports two functions, which the SDK maps to the trait `App`:

```rust
pub trait App: Default + 'static {
    fn start(&mut self);
    fn handle(&mut self, event: Event);
}
```

The phone creates one instance of the type with `Default::default()`, calls `start` once, and then calls `handle` once for every [event](app-events.md).  All calls to one app come one after the other on one thread.  No function of the interface blocks: a function that needs the network (a PBX request, an HTTP request) returns an identifier at once, and the result arrives later as an event that carries the same identifier.

A request that an app makes of the app manager (`runtime::exit`, `runtime::background`, `runtime::restart`, `runtime::show_app`, `events::send`) takes effect after the current `start` or `handle` returns, in the order the app made the requests.

`start` may run for 5 seconds and each `handle` for 2 seconds.  The phone stops an app that runs longer, that asks for more memory than the app's limit, or that traps (a Rust panic is a trap).  Such a stop is a fault: the app receives no `Event::Exit`, and the app's windows close.  Before a panic stops the app, `export_app!` writes the panic's message, file, and line to the phone's log.  The page [Limits](limits.md) has every limit in one place.

What the phone does after a fault depends on how the app starts:

- An app that starts only when opened stays stopped until the user or another app opens the app.
- An app that starts with the phone (`AppKind::Background`, or `has_idle`) is started again by the phone, as a new instance: 10 seconds after the first fault, and after twice the previous wait for each further fault in a row (20 seconds, 40 seconds, and so on), up to one hour.  The count of faults in a row starts over when the user or another app opens the app, and when an instance has run for one hour without a fault.  An app that the phone stops for another reason (`runtime::exit`, removal, the configuration) is not started again this way.

## Capabilities

Each interface except `runtime` is a capability.  An app does not declare capabilities: the capabilities of an app are the interfaces that the app's component imports, and a component imports exactly the interfaces whose functions the app calls.  When an app is installed, the phone reads the imports and records them; a component that imports anything that is not an interface of the firmware is refused with the message `the app imports "<name>", which is not an interface of this firmware`.  When the app starts, the phone links the recorded interfaces and no others.

Some events arrive only at apps that have a capability, whether or not the app asked for the event: the page [Events](app-events.md) names the capability for each.

| Interface | Purpose |
| --- | --- |
| [`runtime`](runtime.md) | Leaving the screen, exiting, logging, and random numbers.  Every app has `runtime`. |
| [`ui`](ui.md) | Widgets, softkeys, keys, pictures, and the window. |
| [`canvas`](canvas.md) | A widget that the app draws from rectangles, bitmaps, and text. |
| [`timers`](timers.md) | Timers and clocks. |
| [`storage`](storage.md) | Keys and values kept on the phone. |
| [`pbx-storage`](pbx-storage.md) | Keys and values kept on the PBX and shared between phones.  The Rust module is `pbx_storage`. |
| [`config`](config.md) | The app's provisioned configuration, the accounts, and some settings of the phone. |
| [`pbx`](pbx.md) | Requests to the PBX's application server. |
| [`http`](http.md) | HTTP and HTTPS requests. |
| [`calls`](calls.md) | Placing and controlling calls, call events, and message-waiting counts. |
| [`presence`](presence.md) | The presence of contacts and of the phone's user. |
| [`events`](events.md) | Events that the PBX pushes to the phone, and messages between apps. |
| [`audio`](audio.md) | Playing a sound. |
| [`l10n`](l10n.md) | The locale, common words, and the text of dates and times. |
| [`hotdesk`](hotdesk.md) | Changing which user the phone is provisioned for. |
| [`diagnostics`](diagnostics.md) | The state of the phone's subsystems, and the log. |
| [`lamps`](lamps.md) | The phone's lamps, while the app is on screen. |
| [`input`](input.md) | Every key and the hookswitch, while the app is on screen. |
| [`alarms`](alarms.md) | The phone's alarm clock. |

## The other pages

| Page | Contents |
| --- | --- |
| [Manifest](manifest.md) | `export_app!` and every field of the manifest, and what the phone checks when an app is installed. |
| [Events](app-events.md) | Every variant of `Event`, when the phone sends the event, and to which apps. |
| [Shared types](types.md) | The types that more than one interface uses: `Rect`, `Metrics`, `Error`, and the rest. |
| [Limits](limits.md) | Every limit the phone enforces, and what happens at each. |
| [SDK helpers](helpers.md) | The functions and types that `dphone-app-sdk` adds on top of the interface: `screen`, `pbx_request`, `Json`, and others. |

## Lifetime of an app

When an app runs depends on the `kind` in the [manifest](manifest.md):

| Kind | Started | Stopped |
| --- | --- | --- |
| `AppKind::Foreground` | When the user or another app opens the app. | When the app's window leaves the screen, unless the app called `runtime::set_exit_after_background(false)` or has an idle window (`has_idle`). |
| `AppKind::Background` | When the phone starts, and when the app is opened. | Only by `runtime::exit`, a fault, removal, or the phone shutting down. |

An app with `has_idle: true` also starts when the phone starts and is stopped only as a `Background` app is, whatever the kind.  A built-in app that the phone's configuration disables is stopped when the configuration arrives and cannot be opened.

Before the phone drops an instance in an orderly way (exit, restart, removal, replacement by a newly installed app of the same name, shutdown), the app receives `Event::Exit`.  A sound the app was playing stops, timers and pending requests are forgotten, and both windows close.  Everything in [`storage`](storage.md) and [`pbx-storage`](pbx-storage.md) stays.
