# `pbx-storage`

The app's store of keys and values on the PBX.  Capability `pbx-storage`; the Rust module is `pbx_storage`.

[`storage`](storage.md) keeps data on one phone.  `pbx_storage` keeps data on the PBX, so that the data is the same on every phone and outlasts a phone: a game's table of best times, or how far a user has come.  The store needs a PBX that runs Open DPMA; the app and the phone need no configuration for the store.

An app has two sets of keys, and no other app can reach either:

| `PbxScope` | Keys |
| --- | --- |
| `Shared` | One set for the app's copies on all phones of the PBX. |
| `User` | One set for each user.  The user is the person the phone is provisioned for, so a user's keys are the same on every phone of that user and survive a reset of a phone. |

A PBX that serves several tenants keeps a separate store for each, and a phone reaches only the store of the phone's tenant.

A key is 1 to 64 characters, each a letter, a digit, or one of `.`, `_`, `-`, `/`, and `:`.  A value is at most 4096 bytes.  An app has at most 1024 shared keys and 64 keys for each user.  The PBX enforces these limits and answers `Refused`.

## Requests and outcomes

`get`, `list`, `set`, and `remove` return a request identifier at once.  The result arrives as [`Event::PbxStorage((id, outcome))`](app-events.md#pbxstorage), exactly once for each request that was not cancelled.

| `PbxStorageOutcome` | Ends | Meaning |
| --- | --- | --- |
| `Entry(Option<PbxEntry>)` | `get` | The entry, or `None` when the key is not set. |
| `Entries((Vec<PbxEntry>, bool))` | `list` | Entries in the order of the keys, and whether more follow the last one. |
| `Stored(PbxEntry)` | `set` | The entry as the PBX now has the key. |
| `Removed` | `remove` | The key is not set: the key was removed, or was never set. |
| `Conflict(Option<PbxEntry>)` | `set`, `remove` | The key was not as `expect` said, and nothing changed.  The payload is the entry the key has now, or `None`. |
| `Unreachable(String)` | any | The PBX could not be asked or did not answer.  See [When the PBX is out of reach](#when-the-pbx-is-out-of-reach). |
| `Refused(String)` | any | The PBX refused the request, and would refuse the same request again: a key that cannot be a key, a value over the limit, or a store that is full.  The text says which. |

A `PbxEntry`:

| Field | Content |
| --- | --- |
| `key`, `value` | The key and the value. |
| `version` | A number that changes with every write of the key, is never 0, and is never a version the key had before.  `set` and `remove` take the number as `expect`. |
| `modified` | When the key was set: seconds since the Unix epoch, by the PBX's clock. |
| `author_id`, `author_name` | The user who set the key (`PbxUser::id`), and that user's name at the time.  The PBX fills these in. |

## `get`

```rust
pub fn get(scope: PbxScope, key: &str) -> u32
```

Asks for the entry of a key.  Ends `Entry`.

## `list`

```rust
pub fn list(scope: PbxScope, prefix: &str, after: &str) -> u32
```

Asks for the entries whose keys start with `prefix` and come after the key `after` in the order of the keys.  `""` as `prefix` matches every key, and `""` as `after` starts at the first key.  Ends `Entries((entries, more))` with at most 64 entries and about 16 KiB of values.  When `more` is `true`, the app asks again with the last entry's key as `after`.

## `set`

```rust
pub fn set(scope: PbxScope, key: &str, value: &[u8], expect: Option<u64>) -> u32
```

Asks to set a key.  `expect` says what the write may replace:

| `expect` | The PBX writes |
| --- | --- |
| `None` | Always, replacing whatever value the key had. |
| `Some(0)` | Only if the key is not set. |
| `Some(version)` | Only if the key's entry still has that version. |

Ends `Stored`, or `Conflict` when the key is not as expected.

## `remove`

```rust
pub fn remove(scope: PbxScope, key: &str, expect: Option<u64>) -> u32
```

Asks to remove a key, under the same conditions as `set`.  Ends `Removed` or `Conflict`.

## `cancel`

```rust
pub fn cancel(id: u32)
```

Forgets a request: the result is not delivered.  The PBX request is not withdrawn, so a cancelled `set` or `remove` may still take effect on the PBX.

## `watch`

```rust
pub fn watch(on: bool)
```

With `true`, the app receives [`Event::PbxStorageChanged`](app-events.md#pbxstoragechanged) whenever another phone sets or removes a shared key of the app, for as long as the instance runs.  The event carries the scope and the key, not the value; the app reads the key again.  A phone is not told of the writes the phone made, nor of changes to a user's keys.  An app that is not running is told nothing and reads what the app shows at the next start.

## `online`

```rust
pub fn online() -> bool
```

Whether the store is in reach now: the phone has a session with the PBX, and the PBX has answered for the store in that session.

## `user`

```rust
pub fn user() -> Option<PbxUser>
```

The phone's user as the PBX knows the user: `id` (which identifies the user and does not change), `name` (to show), and `extension` (may be empty).  `None` until the store was first reached after the phone started.

## Two phones that write at the same time

Without `expect`, the later of two writes replaces the earlier.  An app that changes a value that other phones also change (a list kept in one key) reads the entry, changes the value, and writes with `Some(entry.version)`.  On `Conflict(current)` the app applies the change to `current` and writes again with the version of `current`.

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
            // The table as the PBX has the table now; after a conflict,
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

## When the PBX is out of reach

A request ends `Unreachable(text)` when the phone has no session with the PBX (the phone is not provisioned, or the network or the PBX is down) or when the PBX does not answer, which is also what a PBX without Open DPMA does.  After the PBX failed to answer for the store, the phone answers further requests `Unreachable` at once for the next 60 seconds without asking again.

The phone keeps nothing to send later.  An app that must not lose a write keeps the value, in [`storage`](storage.md) if the value must outlast the instance, and writes again at [`Event::PbxStorageOnline`](app-events.md#pbxstorageonline), which every running app that uses `pbx_storage` receives each time the store comes into reach.  The event also means that entries read earlier may have changed.

A `set` or `remove` that ends `Unreachable` because no answer came may still have reached the PBX.  Repeating a write that carried `expect` then ends in `Conflict` with the entry the first attempt wrote.
