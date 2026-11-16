# `presence`

The presence of contacts and of the phone's user.  Capability `presence`.

The phone watches the contacts that the phone's configuration subscribes to.  The functions here read that state; none of them makes the phone watch a contact the phone does not already watch.  A target names a contact by extension number, by the contact's identifier in the contact list, or by SIP URI, compared as exact text.

## `subscribe`

```rust
pub fn subscribe(target: &str) -> Result<(), Error>
```

Asks for [`Event::Presence`](app-events.md#presence) whenever the watched contact that matches `target` changes.  The `target` field of each event is the string given here.  For a target that matches no watched contact, no event ever arrives.

An app can subscribe to at most 256 targets.  One more target is refused with `Err(Error::Failed(_))`; a target that the app already subscribes to is not one more.  Subscribing sends no event with the current state; `get` reads the state.  The subscriptions end with the instance.

## `unsubscribe`

```rust
pub fn unsubscribe(target: &str)
```

Ends the subscription made with the same string.

## `get`

```rust
pub fn get(target: &str) -> Option<Presence>
```

The presence of a contact now, as [`Presence`](types.md#presence), or `None` when the phone watches no contact that matches `target`.  `get` works without `subscribe`.

## `set_status`

```rust
pub fn set_status(status: &str, text: &str) -> Result<(), Error>
```

WIT name: `set-status`.

Sets the presence of the phone's user: the phone shows the status and publishes the status to the PBX where the configuration says so.  `status` is one of `available`, `chat`, `away`, `dnd`, `xa`, and `unavailable`, in any letter case; `text` is the status message and may be empty.

`Invalid("unknown presence status")` for another status.  `Failed(text)` when the phone's SIP side does not answer within 0.5 seconds; the status may still change afterward.

## `get_status`

```rust
pub fn get_status() -> Presence
```

WIT name: `get-status`.

The presence of the phone's user: `status` (or `unknown` when none was set) and `text`.  `target` is `""` and `line_state` is always `unknown`.
