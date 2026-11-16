# `events`

Events that the PBX pushes to the phone, and messages between apps.  Capability `events`.

Every running app that uses this interface also receives [`Event::ServerChange`](app-events.md#serverchange).

## `observe`

```rust
pub fn observe(name: &str)
```

Asks for [`Event::Push`](app-events.md#push) whenever the PBX pushes an event with exactly this name, e.g., `digium.parkingLot.parked`.  The observation lasts for the life of the instance, so an app that must not miss pushes is of kind `Background`.

An app can observe at most 64 names; further calls are ignored without an error.

## `stop_observing`

```rust
pub fn stop_observing(name: &str)
```

WIT name: `stop-observing`.

Ends the observation of a name.

## `send`

```rust
pub fn send(app: &str, name: &str, payload: &str) -> Result<(), String>
```

Sends [`Event::Message((name, payload))`](app-events.md#message) to the running app whose manifest `name` is `app`.  The message is delivered after the current handler returns.

The result only says whether `app` is formed like an app name: `Err("bad app name")` otherwise.  When the app is not running or does not exist, the message is dropped and the sender is not told.  The phone does not start an app to deliver a message.
