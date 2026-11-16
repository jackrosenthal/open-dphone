# `events`

Events pushed by the PBX, and messages between apps.

Every running app that uses this interface also receives [`Event::ServerChange`](app-events.md#serverchange).

## `observe`

```rust
pub fn observe(name: &str) -> Result<(), Error>
```

Subscribes the app to [`Event::Push`](app-events.md#push) for events that the PBX pushes with exactly this name, such as `digium.parkingLot.parked`.  The subscription lasts for the life of the instance, so an app that can't afford to miss a push should be a `Background` app.

An app can observe at most 64 names.  Beyond that, `observe` returns `Err(Error::Failed(_))` for a new name.  Observing a name again doesn't count against the limit.

## `stop_observing`

```rust
pub fn stop_observing(name: &str)
```

Ends the subscription to a name.

## `send`

```rust
pub fn send(app: &str, name: &str, payload: &str) -> Result<(), String>
```

Sends [`Event::Message((name, payload))`](app-events.md#message) to the running app whose manifest `name` is `app`.  The message is delivered after the current handler returns.

Returns `Ok` if the target app was running when `send` was called.  An app that stops before the sender's handler returns doesn't get the message.

Returns `Err`, with a message that explains why, when `app` is not a well-formed app name or when no app with that name is running.  Nothing is sent in that case, and the phone never starts an app in order to deliver a message.
