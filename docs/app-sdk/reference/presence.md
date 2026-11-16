# `presence`

The presence of contacts and of the phone's user.

## Watched contacts

The phone watches contacts for the rapid-dial (BLF) keys, and the functions on this page read what the phone learns from doing so.  No function makes the phone watch an additional contact.  The phone watches a contact when both of these are true:

- The contact is on a rapid-dial key.  Either the phone's configuration names the contact in a `blf_item`, or the configuration has no `blf_items`, the contact is in the contact group named by the `blf_contact_group` setting, and the phone has placed the contact on an unused key.
- The contact's entry in the contact list has `subscribe_to`.

The `contacts_max_subscriptions` setting (40 by default) caps the number of different contacts that the phone watches.  A contact that is in a contact list but not on a key is not watched: `get` returns `None` for that contact, and no `Event::Presence` arrives.

A target identifies a watched contact by the number that the contact's key dials, by the contact's identifier in the contact list, or by the SIP URI that the phone subscribes to.  Targets are compared as exact text.

## `observe`

```rust
pub fn observe(target: &str) -> Result<(), Error>
```

Subscribes the app to [`Event::Presence`](app-events.md#presence) for the watched contact that matches `target`.  The event arrives whenever the contact's presence changes, and the `target` field of each event is the string passed here.

`observe` doesn't send an event with the current state; call `get` for that.  A target that matches no watched contact is accepted, but no events arrive for that target.  The subscription ends when the instance ends.

An app can observe at most 256 targets.  Beyond that, `observe` returns `Err(Error::Failed(_))` for a new target.  Observing a target again doesn't count against the limit.

## `stop_observing`

```rust
pub fn stop_observing(target: &str)
```

Ends the subscription that was started with the same string.

## `get`

```rust
pub fn get(target: &str) -> Option<Presence>
```

Returns the current presence of a contact, or `None` if the phone watches no contact that matches `target`.  See [`Presence`](types.md#presence) for the fields.  `get` works without `observe`.

## `set_status`

```rust
pub fn set_status(status: &str, text: &str) -> Result<(), Error>
```

Sets the presence of the phone's user.  The phone displays the status and, if the configuration says so, publishes the status to the PBX.  `status` is one of `available`, `chat`, `away`, `dnd`, `xa`, and `unavailable`, in any letter case.  `text` is the status message, and may be empty.

Returns `Invalid("unknown presence status")` for any other status.  Returns `Failed(text)` if the phone's SIP stack doesn't answer within 0.5 seconds, in which case the status may still change afterward.

## `get_status`

```rust
pub fn get_status() -> Presence
```

Returns the presence of the phone's user.  `status` is `unknown` if no status has been set, `text` is the status message, `target` is `""`, and `line_state` is always `unknown`.
