---
title: "Reference"
---

## Execution model

An app implements `App`:

```rust
pub trait App: Default + 'static {
    fn start(&mut self);
    fn handle(&mut self, event: Event);
}
```

The phone creates one instance with `Default::default()`, calls `start` once, and then calls `handle` once for each [event](app-events.md), sequentially, on one thread.

A function that needs the network, such as a PBX or HTTP request, returns an ID at once, and the result arrives later as an event with that ID.  These functions wait for the phone's answer:

| Function | Longest wait |
| --- | --- |
| [`calls`](calls.md) functions that return `Result` | 0.5 seconds; in `send_dtmf`, 0.5 seconds for each digit |
| [`presence::set_status`](presence.md#set_status) | 0.5 seconds |
| [`hotdesk::login`](hotdesk.md#login), [`hotdesk::logout`](hotdesk.md#logout) | 0.5 seconds |
| [`alarms`](alarms.md) functions that return `Result` | 1.5 seconds |
| [`audio::play`](audio.md#play) | 5 seconds |

## Faults and restarts

The phone stops an app that:

- runs longer than 5 seconds in `start`, or 2 seconds in one `handle` call, not counting [waits for the phone's answer](#execution-model),
- doesn't return from `start` or `handle` within 10 seconds, waits included,
- requests more memory than the app's limit, or
- traps.  A Rust panic is a trap.

Such a stop is a fault: the app receives no `Event::Exit`, and the app's windows close.

- An app that starts only when opened stays stopped until the user or another app opens the app.
- An app that starts with the phone (`AppKind::Background`, or `has_idle`) restarts as a new instance 10 seconds after the fault.  Each consecutive fault doubles the delay, up to one hour.  The fault count resets when the app is opened, and when an instance runs one hour without a fault.  An app stopped for another reason (`runtime::exit`, removal, or the configuration) doesn't restart this way.

An app doesn't start if the component has more than 1 memory, 4 tables, 50,000 table elements, or 8 instances.

## Capabilities

Every interface except `runtime` is a capability.  An app's capabilities are the interfaces whose functions the app calls; apps don't declare capabilities.  Some [events](app-events.md) go to every running app that has a given capability, without a subscription.

## App lifecycle

| App | Starts | Stops |
| --- | --- | --- |
| `AppKind::Foreground` without `has_idle` | When the user or another app opens the app. | When the window leaves the screen, unless the app called `runtime::set_exit_after_background(false)`. |
| `AppKind::Background`, or `has_idle` | When the phone starts, and when the app is opened. | Only on `runtime::exit`, a fault, removal, shutdown, or, for a built-in app, a configuration that disables the app. |

When the phone discards an instance, the app's sound stops, timers and pending requests are dropped, and both windows close.  Before an orderly discard, the app receives [`Event::Exit`](app-events.md#exit).
