---
title: "storage"
---

The app's key-value store, in three locations.  No other app can access the app's keys.

| `StorageLocation` | Keys | Answer |
| --- | --- | --- |
| `Local` | On the phone.  The keys survive restarts of the app and the phone, firmware updates, and newer versions of the app.  Removing the app deletes the keys. | At once: `StorageReply::Done(outcome)`. |
| `PbxPrivate` | On the PBX, one set per user, the person that the phone is provisioned for: the same on all of the user's phones, and kept across a phone reset. | Later: `StorageReply::Pending(id)`, then [`Event::Storage((id, outcome))`](app-events.md#storage). |
| `PbxShared` | On the PBX, one set for every phone on the PBX that runs the app. | Later, as for `PbxPrivate`. |

The PBX locations require a PBX running [Open DPMA](../../open-dpma/index.md), and no configuration.  A multi-tenant PBX keeps one store per tenant, and a phone reaches only the store of the phone's tenant.

## Limits

| | `Local` | `PbxPrivate`, `PbxShared` |
| --- | --- | --- |
| Key | 1 to 64 bytes of UTF-8, not starting with `.`, without `/`, `\`, or NUL | 1 to 64 characters, each a letter, a digit, `.`, `_`, `-`, `/`, or `:` |
| Value | 1 MiB | 4096 bytes |
| Keys per app | As many as fit in free flash, of which the phone reserves 2 MiB | 64 per user in `PbxPrivate`, 1024 in `PbxShared` |

A request beyond a limit ends in `Refused`.

## Outcomes

```rust
pub enum StorageReply {
    Done(StorageOutcome),
    Pending(u32),
}
```

A request to `Local` returns `Done`; a request to a PBX location returns `Pending` with a request ID, and `Event::Storage((id, outcome))` arrives exactly once for each request that wasn't canceled.

| `StorageOutcome` | Answers | Meaning |
| --- | --- | --- |
| `Entry(Option<StorageEntry>)` | `get` | The entry, or `None` if the key isn't set. |
| `Entries((Vec<StorageEntry>, bool))` | `list` | The entries in key order, and whether more follow. |
| `Stored(StorageEntry)` | `set` | The entry as stored. |
| `Removed` | `remove` | The key isn't set: removed, or never set. |
| `Conflict(Option<StorageEntry>)` | `set`, `remove` | The key didn't match `expect`; nothing changed.  The payload is the key's current entry, or `None`. |
| `Unreachable(String)` | Any | The store couldn't be reached.  For a PBX location, the request didn't reach the PBX, or the PBX didn't answer; see [Offline behavior](#offline-behavior).  For `Local`, the flash couldn't be read or written. |
| `Refused(String)` | Any | The store refused the request and would refuse the request again: an invalid key, a value too large, or a full store.  The text says which. |

| `StorageEntry` field | Contents |
| --- | --- |
| `key`, `value` | The key and the value. |
| `version` | Changes with every write to the key; never 0, and never repeated for a key.  Pass to `set` or `remove` as `expect`. |
| `modified` | The time of the write, in seconds since the Unix epoch, by the PBX's clock; for `Local`, by the phone's clock. |
| `author_id`, `author_name` | The user who wrote the key (`PbxUser::id`), and that user's name at the time.  The PBX fills in both.  Empty for `Local`. |

## `get`

```rust
pub fn get(location: StorageLocation, key: &str) -> StorageReply
```

Requests a key's entry.  Ends in `Entry`.  For UTF-8 text in `Local`, the helper [`read_string`](helpers.md#logging-and-storage) returns a `String`.

## `list`

```rust
pub fn list(location: StorageLocation, prefix: &str, after: &str) -> StorageReply
```

Requests the entries whose keys start with `prefix` and sort after `after`, in byte order.  An empty `prefix` matches every key; an empty `after` starts at the first key.

Ends in `Entries((entries, more))`, with at most 64 entries, and at most about 16 KiB of values after the first entry.  If `more` is `true`, call `list` again with the last key as `after`.

## `set`

```rust
pub fn set(location: StorageLocation, key: &str, value: &[u8], expect: Option<u64>) -> StorageReply
```

Requests a write, conditional on `expect`:

| `expect` | The store writes |
| --- | --- |
| `None` | Always, replacing any value. |
| `Some(0)` | Only if the key isn't set. |
| `Some(version)` | Only if the key's entry has that version. |

Ends in `Stored`, or in `Conflict` if the condition fails.  A `Local` write is atomic: after a power loss, the key has the old value or the new one, never part of a value.  A `Local` write that needs more space than the flash has free beyond the 2 MiB reserve ends in `Refused`; replacing a value counts only the growth.

## `remove`

```rust
pub fn remove(location: StorageLocation, key: &str, expect: Option<u64>) -> StorageReply
```

Requests removal of a key, with `expect` as for `set`.  Ends in `Removed` or `Conflict`.

## `cancel`

```rust
pub fn cancel(id: u32)
```

Discards the result of a pending request.  The request isn't withdrawn: a canceled `set` or `remove` may still take effect.

## `watch`

```rust
pub fn watch(on: bool)
```

Turns change notifications on or off.  While on, [`Event::StorageChanged`](app-events.md#storagechanged) arrives whenever another phone sets or removes one of the app's `PbxShared` keys, for the life of the instance.  The event carries the location and the key, not the value.  Writes from the same phone and changes to `PbxPrivate` keys aren't notified.

## `online`

```rust
pub fn online() -> bool
```

Whether the PBX locations are reachable: the phone has a session with the PBX, and the PBX has answered a storage request in that session.  `Local` is always reachable.

## `user`

```rust
pub fn user() -> Option<PbxUser>
```

The phone's user as known to the PBX: `id` (stable), `name` (for display), and `extension` (may be empty).  `None` until the PBX locations are first reached after the phone starts.

## Concurrent writes

Without `expect`, the later write wins.  For a value that several phones modify, such as a list in one key, read the entry, modify the value, and write with `Some(entry.version)`.  On `Conflict(current)`, apply the change to `current`, and write with the version from `current`.

```rust
const TABLE: &str = "best/7x7-easy";

fn add_mine(&mut self, current: Option<StorageEntry>) {
    let (mut rows, version) = match &current {
        Some(entry) => (parse(&entry.value), entry.version),
        None => (Vec::new(), 0),
    };
    rows.push(self.mine);
    let reply = storage::set(StorageLocation::PbxShared, TABLE, &format(&rows), Some(version));
    if let StorageReply::Pending(id) = reply {
        self.asked = id;
    }
}

fn handle(&mut self, event: Event) {
    if let Event::Storage((id, outcome)) = event {
        if id != self.asked { return; }
        match outcome {
            // The table as the PBX has the key now; after a conflict,
            // the table another phone wrote in the meantime.
            StorageOutcome::Entry(current) | StorageOutcome::Conflict(current) => {
                self.add_mine(current)
            }
            StorageOutcome::Stored(entry) => self.show(parse(&entry.value)),
            _ => {}
        }
    }
}
```

## Offline behavior

A request to a PBX location ends in `Unreachable(text)` when the phone has no session with the PBX (the phone isn't provisioned, or the network or the PBX is down), or when the PBX doesn't answer.  A PBX without Open DPMA never answers.  After an unanswered storage request, the phone answers further requests with `Unreachable` for 60 seconds, without contacting the PBX.

The phone doesn't queue requests.  To keep a write, keep the value (in `Local`, to outlive the instance), and write again on [`Event::StorageOnline`](app-events.md#storageonline).

A `set` or `remove` that ended in `Unreachable` for lack of an answer may have reached the PBX.  With `expect`, a retry then ends in `Conflict`, with the entry that the first attempt wrote.
