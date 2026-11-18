---
title: "presence"
---

## Watched contacts

The functions read what the phone learns by watching contacts for the rapid-dial (BLF) keys.  No function adds a watched contact.  The phone watches a contact that:

- is on a rapid-dial key: named in a `blf_item` of the configuration, or, without `blf_items`, in the contact group named by the `blf_contact_group` setting and placed by the phone on an unused key; and
- has `subscribe_to` in the contact list.

The `contacts_max_subscriptions` setting (40 by default) caps the number of watched contacts.  For a contact that isn't watched, `get` returns `None`, and no `Event::Presence` arrives.

A target identifies a watched contact by the number that the contact's key dials, the contact's identifier in the contact list, or the SIP URI that the phone subscribes to, compared as exact text.

## `observe`

```rust
pub fn observe(target: &str) -> Result<(), Error>
```

Subscribes the app to [`Event::Presence`](app-events.md#presence) for the watched contact that matches `target`.  The event arrives whenever the contact's presence changes, with `target` set to this string.

- No event reports the current state; call `get`.
- A target that matches no watched contact is accepted, and gets no events.
- The subscription ends with the instance.
- An app observes at most 256 targets.  Beyond that, a new target returns `Err(Error::Failed(_))`; observing a target again doesn't count.

## `stop_observing`

```rust
pub fn stop_observing(target: &str)
```

Ends the subscription for `target`.

## `get`

```rust
pub fn get(target: &str) -> Option<Presence>
```

The contact's presence ([`Presence`](types.md#presence)), or `None` if no watched contact matches `target`.  Works without `observe`.

## `set_status`

```rust
pub fn set_status(status: PresenceStatus, text: &str) -> Result<(), Error>
```

Sets the presence of the phone's user.  The phone displays the status and, if the configuration says so, publishes the status to the PBX.  `text` is the status message, and may be empty.

Returns `Invalid("unknown presence status")` for `PresenceStatus::Unknown`, and `Failed(text)` if the phone's SIP stack doesn't answer within 0.5 seconds; the status may still change afterward.

## `get_status`

```rust
pub fn get_status() -> Presence
```

The presence of the phone's user: `status` is `Unknown` if never set, `text` is the status message, `target` is `""`, and `line_state` is always `Unknown`.
