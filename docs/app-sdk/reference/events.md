# `events`

Events pushed by the PBX, and messages between apps.  Every running app that uses `events` receives [`Event::ServerChange`](app-events.md#serverchange).

## `observe`

```rust
pub fn observe(name: &str) -> Result<(), Error>
```

Subscribes the app to [`Event::Push`](app-events.md#push) for PBX events with exactly this name, such as `digium.parkingLot.parked`.  The subscription lasts for the life of the instance; an app that must not miss a push should be a `Background` app.

An app observes at most 64 names.  Beyond that, a new name returns `Err(Error::Failed(_))`; observing a name again doesn't count.

## `stop_observing`

```rust
pub fn stop_observing(name: &str)
```

Ends the subscription to `name`.

## `send`

```rust
pub fn send(app: &str, name: &str, payload: &str) -> Result<(), String>
```

Sends [`Event::Message((name, payload))`](app-events.md#message) to the running app whose manifest `name` is `app`, after the current handler returns.

- `Ok`: the app was running when `send` was called.  An app that stops before the sender's handler returns doesn't get the message.
- `Err`, with the reason: `app` isn't a well-formed app name, or no app with that name is running.  Nothing is sent, and the phone never starts an app to deliver a message.
