# `pbx_storage`

The app's key-value store on the PBX.  Unlike [`storage`](storage.md), the store is the same on every phone and outlives any one phone.  The store requires a PBX running Open DPMA, and no configuration.

| `PbxScope` | Keys |
| --- | --- |
| `Shared` | One set for every phone on the PBX that runs the app. |
| `User` | One set per user, the person that the phone is provisioned for: the same on all of the user's phones, and kept across a phone reset. |

No other app can access either set.  A multi-tenant PBX keeps one store per tenant, and a phone reaches only the store of the phone's tenant.

- Key: 1 to 64 characters, each a letter, a digit, `.`, `_`, `-`, `/`, or `:`.
- Value: at most 4096 bytes.
- Keys per app: 1024 shared, and 64 per user.

The PBX answers a request beyond a limit with `Refused`.

## Requests and outcomes

`get`, `list`, `set`, and `remove` return a request ID at once.  [`Event::PbxStorage((id, outcome))`](app-events.md#pbxstorage) arrives exactly once for each request that wasn't canceled.

| `PbxStorageOutcome` | Answers | Meaning |
| --- | --- | --- |
| `Entry(Option<PbxEntry>)` | `get` | The entry, or `None` if the key isn't set. |
| `Entries((Vec<PbxEntry>, bool))` | `list` | The entries in key order, and whether more follow. |
| `Stored(PbxEntry)` | `set` | The entry as stored. |
| `Removed` | `remove` | The key isn't set: removed, or never set. |
| `Conflict(Option<PbxEntry>)` | `set`, `remove` | The key didn't match `expect`; nothing changed.  The payload is the key's current entry, or `None`. |
| `Unreachable(String)` | Any | The request didn't reach the PBX, or the PBX didn't answer.  See [Offline behavior](#offline-behavior). |
| `Refused(String)` | Any | The PBX refused the request and would refuse the request again: an invalid key, a value too large, or a full store.  The text says which. |

| `PbxEntry` field | Contents |
| --- | --- |
| `key`, `value` | The key and the value. |
| `version` | Changes with every write to the key; never 0, and never repeated for a key.  Pass to `set` or `remove` as `expect`. |
| `modified` | The time of the write, in seconds since the Unix epoch, by the PBX's clock. |
| `author_id`, `author_name` | The user who wrote the key (`PbxUser::id`), and that user's name at the time.  The PBX fills in both. |

## `get`

```rust
pub fn get(scope: PbxScope, key: &str) -> u32
```

Requests a key's entry.  Ends in `Entry`.

## `list`

```rust
pub fn list(scope: PbxScope, prefix: &str, after: &str) -> u32
```

Requests the entries whose keys start with `prefix` and sort after `after`.  An empty `prefix` matches every key; an empty `after` starts at the first key.

Ends in `Entries((entries, more))`, with at most 64 entries and about 16 KiB of values.  If `more` is `true`, call `list` again with the last key as `after`.

## `set`

```rust
pub fn set(scope: PbxScope, key: &str, value: &[u8], expect: Option<u64>) -> u32
```

Requests a write, conditional on `expect`:

| `expect` | The PBX writes |
| --- | --- |
| `None` | Always, replacing any value. |
| `Some(0)` | Only if the key isn't set. |
| `Some(version)` | Only if the key's entry has that version. |

Ends in `Stored`, or in `Conflict` if the condition fails.

## `remove`

```rust
pub fn remove(scope: PbxScope, key: &str, expect: Option<u64>) -> u32
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

Turns change notifications on or off.  While on, [`Event::PbxStorageChanged`](app-events.md#pbxstoragechanged) arrives whenever another phone sets or removes one of the app's shared keys, for the life of the instance.  The event carries the scope and the key, not the value.  Writes from the same phone and changes to user keys aren't notified.

## `online`

```rust
pub fn online() -> bool
```

Whether the store is reachable: the phone has a session with the PBX, and the PBX has answered a storage request in that session.

## `user`

```rust
pub fn user() -> Option<PbxUser>
```

The phone's user as known to the PBX: `id` (stable), `name` (for display), and `extension` (may be empty).  `None` until the store is first reached after the phone starts.

## Concurrent writes

Without `expect`, the later write wins.  For a value that several phones modify, such as a list in one key, read the entry, modify the value, and write with `Some(entry.version)`.  On `Conflict(current)`, apply the change to `current`, and write with the version from `current`.

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

A request ends in `Unreachable(text)` when the phone has no session with the PBX (the phone isn't provisioned, or the network or the PBX is down), or when the PBX doesn't answer.  A PBX without Open DPMA never answers.  After an unanswered storage request, the phone answers further requests with `Unreachable` for 60 seconds, without contacting the PBX.

The phone doesn't queue requests.  To keep a write, keep the value (in [`storage`](storage.md), to outlive the instance), and write again on [`Event::PbxStorageOnline`](app-events.md#pbxstorageonline).

A `set` or `remove` that ended in `Unreachable` for lack of an answer may have reached the PBX.  With `expect`, a retry then ends in `Conflict`, with the entry that the first attempt wrote.
