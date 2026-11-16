# `events`

Events that the PBX pushes to the phone, and messages between apps.  Capability `events`.

Every running app that uses this interface also receives [`Event::ServerChange`](app-events.md#serverchange).

## `observe`

```rust
pub fn observe(name: &str) -> Result<(), Error>
```

Asks for [`Event::Push`](app-events.md#push) whenever the PBX pushes an event with exactly this name, e.g., `digium.parkingLot.parked`.  The observation lasts for the life of the instance, so an app that must not miss pushes is of kind `Background`.

An app can observe at most 64 names.  One more name is refused with `Err(Error::Failed(_))`; a name that the app already observes is not one more.

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

The call fails, with a sentence that says why, when `app` is not formed like an app name or no app of that name is running; no message is sent then.  The phone does not start an app to deliver a message.  `Ok` says that the app was running when the message was sent: an app that stops before the sender's handler returns does not get the message.
