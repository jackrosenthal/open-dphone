# `presence`

The presence of contacts and of the phone's user.  Capability `presence`.

## Which contacts the phone watches

The phone watches contacts for the rapid-dial (BLF) keys, and the functions here read what the phone learns that way.  No function makes the phone watch a contact that the phone does not already watch.  The phone watches a contact when both of these hold:

- The contact is on a rapid-dial key: the phone's configuration names the contact in a `blf_item`, or, when the configuration has no `blf_items`, the contact is in the contact group that the setting `blf_contact_group` names and the phone has placed the contact on an unused key.
- The contact's entry in the contact list has `subscribe_to`.

The phone watches at most as many different contacts as the setting `contacts_max_subscriptions` allows (40 by default).  A contact that is only in a contact list, and on no key, is not watched: `get` returns `None` for such a contact, and no `Event::Presence` arrives.

A target names a watched contact by the number the contact's key dials, by the contact's identifier in the contact list, or by the SIP URI the phone subscribes to, compared as exact text.

## `observe`

```rust
pub fn observe(target: &str) -> Result<(), Error>
```

Asks for [`Event::Presence`](app-events.md#presence) whenever the watched contact that matches `target` changes.  The `target` field of each event is the string given here.  `observe` accepts a target that matches no watched contact, and for such a target no event arrives.

An app can observe at most 256 targets.  One more target is refused with `Err(Error::Failed(_))`; a target that the app already observes is not one more.  `observe` sends no event with the current state; `get` reads the state.  The app stops observing when the instance ends.

## `stop_observing`

```rust
pub fn stop_observing(target: &str)
```

WIT name: `stop-observing`.

Ends the observing begun with the same string.

## `get`

```rust
pub fn get(target: &str) -> Option<Presence>
```

The presence of a contact now, as [`Presence`](types.md#presence), or `None` when the phone watches no contact that matches `target`.  `get` works without `observe`.

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
