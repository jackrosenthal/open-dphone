# `pbx_storage`

The app's key-value store on the PBX.  Unlike [`storage`](storage.md), the store is the same on every phone and outlives any one phone.  The store requires a PBX running Open DPMA, and needs no configuration in the app or on the phone.

An app has two sets of keys, and no other app can access either:

| `PbxScope` | Keys |
| --- | --- |
| `Shared` | One set, shared by every phone on the PBX that runs the app. |
| `User` | One set per user.  The user is the person that the phone is provisioned for, so a user's keys are the same on every one of that user's phones and survive a phone reset. |

A multi-tenant PBX keeps a separate store for each tenant, and a phone can reach only the store of the tenant that the phone belongs to.

A key is 1 to 64 characters, each a letter, a digit, or one of `.`, `_`, `-`, `/`, and `:`.  A value is at most 4096 bytes.  An app can have up to 1024 shared keys and 64 keys per user.  The PBX enforces these limits, and answers a request that exceeds one with `Refused`.

## Requests and outcomes

`get`, `list`, `set`, and `remove` return a request ID immediately.  The result arrives as [`Event::PbxStorage((id, outcome))`](app-events.md#pbxstorage), exactly once for each request that wasn't canceled.

| `PbxStorageOutcome` | Answers | Meaning |
| --- | --- | --- |
| `Entry(Option<PbxEntry>)` | `get` | The entry, or `None` if the key isn't set. |
| `Entries((Vec<PbxEntry>, bool))` | `list` | The entries in key order, and whether more entries follow the last one. |
| `Stored(PbxEntry)` | `set` | The entry as now stored on the PBX. |
| `Removed` | `remove` | The key isn't set: either the key was removed, or the key was never set. |
| `Conflict(Option<PbxEntry>)` | `set`, `remove` | The key didn't match `expect`, and nothing changed.  The payload is the key's current entry, or `None`. |
| `Unreachable(String)` | Any | The request never reached the PBX, or the PBX didn't answer.  See [Offline behavior](#offline-behavior). |
| `Refused(String)` | Any | The PBX refused the request and would refuse the same request again: the key is invalid, the value is too large, or the store is full.  The text says which. |

The fields of a `PbxEntry`:

| Field | Contents |
| --- | --- |
| `key`, `value` | The key and the value. |
| `version` | A number that changes with every write to the key.  A version is never 0 and never repeats for the same key.  Pass the version to `set` or `remove` as `expect`. |
| `modified` | When the key was set, in seconds since the Unix epoch, according to the PBX's clock. |
| `author_id`, `author_name` | The user who set the key (`PbxUser::id`), and that user's name at the time.  The PBX fills in both. |

## `get`

```rust
pub fn get(scope: PbxScope, key: &str) -> u32
```

Requests the entry for a key.  The request ends in `Entry`.

## `list`

```rust
pub fn list(scope: PbxScope, prefix: &str, after: &str) -> u32
```

Requests the entries whose keys start with `prefix` and sort after the key `after`.  An empty `prefix` matches every key, and an empty `after` starts from the first key.

The request ends in `Entries((entries, more))`, with at most 64 entries and about 16 KiB of values.  If `more` is `true`, call `list` again with the last entry's key as `after`.

## `set`

```rust
pub fn set(scope: PbxScope, key: &str, value: &[u8], expect: Option<u64>) -> u32
```

Requests a write to a key.  `expect` makes the write conditional:

| `expect` | The PBX writes |
| --- | --- |
| `None` | Always, replacing any existing value. |
| `Some(0)` | Only if the key isn't set. |
| `Some(version)` | Only if the key's entry still has that version. |

The request ends in `Stored`, or in `Conflict` if the condition wasn't met.

## `remove`

```rust
pub fn remove(scope: PbxScope, key: &str, expect: Option<u64>) -> u32
```

Requests removal of a key.  `expect` works the same way as for `set`.  The request ends in `Removed` or `Conflict`.

## `cancel`

```rust
pub fn cancel(id: u32)
```

Discards a pending request, so that the result is never delivered.  The request to the PBX isn't withdrawn, and a canceled `set` or `remove` may still take effect on the PBX.

## `watch`

```rust
pub fn watch(on: bool)
```

Turns change notifications on or off.  While notifications are on, the app receives [`Event::PbxStorageChanged`](app-events.md#pbxstoragechanged) whenever another phone sets or removes one of the app's shared keys, for as long as the instance runs.

The event carries the scope and the key but not the value, so the app needs to read the key again.  A phone isn't notified of writes made from that same phone, or of changes to a user's keys.

## `online`

```rust
pub fn online() -> bool
```

Returns whether the store is currently reachable.  The store is reachable when the phone has a session with the PBX and the PBX has answered a storage request during that session.

## `user`

```rust
pub fn user() -> Option<PbxUser>
```

Returns the phone's user as known to the PBX: `id` (a stable identifier for the user), `name` (for display), and `extension` (which may be empty).  Returns `None` until the store has been reached for the first time since the phone started.

## Concurrent writes

Without `expect`, the later of two writes replaces the earlier one.  When several phones modify the same value, such as a list kept in one key, read the entry, modify the value, and write with `Some(entry.version)`.  On `Conflict(current)`, apply the change to `current` and write again with the version from `current`.

```rust
const TABLE: &str = "best/7x7-easy";

fn add_mine(&mut self, current: Option<PbxEntry>) {
    let (mut rows, version) = match &current {
        Some(entry) => (parse(&entry.value), entry.version),
        None => (Vec::new(), 0),
    };
    rows.push(self.mine);
    self.asked = pbx_storage::set(PbxScope::Shared, TABLE, &format(&rows), Some(version));
}

fn handle(&mut self, event: Event) {
    if let Event::PbxStorage((id, outcome)) = event {
        if id != self.asked { return; }
        match outcome {
            // The table as the PBX has the key now; after a conflict,
            // the table another phone wrote in the meantime.
            PbxStorageOutcome::Entry(current) | PbxStorageOutcome::Conflict(current) => {
                self.add_mine(current)
            }
            PbxStorageOutcome::Stored(entry) => self.show(parse(&entry.value)),
            _ => {}
        }
    }
}
```

## Offline behavior

A request ends in `Unreachable(text)` when the phone has no session with the PBX (the phone isn't provisioned, or the network or the PBX is down) or when the PBX doesn't answer.  A PBX without Open DPMA never answers.  After the PBX fails to answer a storage request, the phone answers further requests with `Unreachable` immediately for the next 60 seconds, without contacting the PBX.

The phone doesn't queue requests to send later.  An app that can't afford to lose a write should keep the value (in [`storage`](storage.md), if the value must outlive the instance) and write again on [`Event::PbxStorageOnline`](app-events.md#pbxstorageonline).

A `set` or `remove` that ends in `Unreachable` because no answer came may still have reached the PBX.  If the write carried `expect`, retrying then ends in `Conflict`, with the entry that the first attempt wrote.
