# Reference

## Execution model

An app implements the `App` trait:

```rust
pub trait App: Default + 'static {
    fn start(&mut self);
    fn handle(&mut self, event: Event);
}
```

The phone creates one instance of the type with `Default::default()`, calls `start` once, and then calls `handle` once for every [event](app-events.md).  Calls into an app are strictly sequential, on a single thread.

No function in the interface blocks.  A function that needs the network, such as a PBX request or an HTTP request, returns an ID immediately, and the result arrives later as an event with the same ID.

## Faults and restarts

`start` may run for up to 5 seconds, and each call to `handle` for up to 2 seconds.  The phone stops an app that runs longer, that requests more memory than the app's limit allows, or that traps.  (A Rust panic is a trap.)  This kind of stop is called a fault: the app receives no `Event::Exit`, and the app's windows close.

What happens after a fault depends on how the app starts:

- An app that starts only when opened stays stopped until the user or another app opens the app again.
- An app that starts with the phone (`AppKind::Background`, or `has_idle`) is restarted automatically as a new instance.  The first restart comes 10 seconds after the fault.  Each consecutive fault doubles the delay (20 seconds, 40 seconds, and so on), up to one hour.  The count of consecutive faults resets when the user or another app opens the app, and when an instance runs for an hour without a fault.  An app that stops for any other reason (`runtime::exit`, removal, or the configuration) is not restarted this way.

An app also fails to start if the component has more than 1 memory, 4 tables, 50,000 table elements, or 8 instances.

## Capabilities

Every interface except `runtime` is a capability.  An app's capabilities are the interfaces that the component imports, and a component imports exactly the interfaces whose functions the app calls; apps don't declare capabilities.

Some events are delivered only to apps that have a particular capability, whether or not the app asked for the event.  [Events](app-events.md) names the capability for each.

## App lifecycle

The `kind` in the [manifest](manifest.md) and `has_idle` determine when an app runs:

| Apps | Started | Stopped |
| --- | --- | --- |
| `AppKind::Foreground` without `has_idle` | When the user or another app opens the app. | When the app's window leaves the screen, unless the app called `runtime::set_exit_after_background(false)`. |
| `AppKind::Background`, and every app with `has_idle` | When the phone starts, and when the app is opened. | Only by `runtime::exit`, a fault, removal, shutdown, or, for a built-in app, a configuration that disables the app. |

When the phone discards an instance, any sound that the app was playing stops, timers and pending requests are dropped, and both windows close.  Before an orderly discard, the app receives [`Event::Exit`](app-events.md#exit).
