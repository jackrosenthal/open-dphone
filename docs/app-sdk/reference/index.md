# Reference

This reference covers the interface between an app and a phone running Open DPhone: the functions an app can call, the events an app receives, the manifest, and the limits that the phone enforces.

The interface is defined in a single file, `wit/app.wit` in the `dphone-app-sdk` crate (the WIT package `dphone:app@3.0.0`), and the firmware generates the phone's side from the same file.  These pages use the Rust names that `dphone-app-sdk` exports, which are the `app.wit` names with `_` in place of `-`.  A page notes any name that differs in some other way.

## Execution model

An app is a WebAssembly component that exports two functions, which the SDK maps to the `App` trait:

```rust
pub trait App: Default + 'static {
    fn start(&mut self);
    fn handle(&mut self, event: Event);
}
```

The phone creates one instance of the type with `Default::default()`, calls `start` once, and then calls `handle` once for every [event](app-events.md).  Calls into an app are strictly sequential, on a single thread.

No function in the interface blocks.  A function that needs the network, such as a PBX request or an HTTP request, returns an ID immediately, and the result arrives later as an event with the same ID.

Requests to the app manager (`runtime::exit`, `runtime::background`, `runtime::restart`, `runtime::show_app`, and `events::send`) take effect after the current `start` or `handle` returns, in the order the app made them.

## Faults and restarts

`start` may run for up to 5 seconds, and each call to `handle` for up to 2 seconds.  The phone stops an app that runs longer, that requests more memory than the app's limit allows, or that traps.  (A Rust panic is a trap.)  This kind of stop is called a fault: the app receives no `Event::Exit`, and the app's windows close.  Before a panic stops the app, `export_app!` logs the panic message with the file and line.  See [Limits](limits.md) for every limit.

What happens after a fault depends on how the app starts:

- An app that starts only when opened stays stopped until the user or another app opens the app again.
- An app that starts with the phone (`AppKind::Background`, or `has_idle`) is restarted automatically as a new instance.  The first restart comes 10 seconds after the fault.  Each consecutive fault doubles the delay (20 seconds, 40 seconds, and so on), up to one hour.  The count of consecutive faults resets when the user or another app opens the app, and when an instance runs for an hour without a fault.  An app that stops for any other reason (`runtime::exit`, removal, or the configuration) is not restarted this way.

## Capabilities

Every interface except `runtime` is a capability.  Apps don't declare capabilities.  An app's capabilities are the interfaces that the component imports, and a component imports exactly the interfaces whose functions the app calls.

At install time, the phone reads the component's imports and records them.  A component that imports anything other than an interface of the firmware is rejected with the message `the app imports "<name>", which is not an interface of this firmware`.  When the app starts, the phone links the recorded interfaces and nothing else.

Some events are delivered only to apps that have a particular capability, whether or not the app asked for the event.  [Events](app-events.md) names the capability for each.

## App lifecycle

The `kind` in the [manifest](manifest.md) determines when an app runs:

| Kind | Started | Stopped |
| --- | --- | --- |
| `AppKind::Foreground` | When the user or another app opens the app. | When the app's window leaves the screen, unless the app called `runtime::set_exit_after_background(false)` or has an idle window (`has_idle`). |
| `AppKind::Background` | When the phone starts, and when the app is opened. | Only by `runtime::exit`, a fault, removal, or shutdown. |

An app with `has_idle: true` also starts with the phone and, whatever the kind, stops only for the reasons that stop a `Background` app.  A built-in app that the phone's configuration disables is stopped when the configuration arrives, and can't be opened.

Before discarding an instance in an orderly way (exit, restart, removal, replacement by a newly installed app with the same name, or shutdown), the phone sends the app `Event::Exit`.  Any sound that the app was playing stops, timers and pending requests are dropped, and both windows close.  Everything in [`storage`](storage.md) and [`pbx-storage`](pbx-storage.md) is kept.
