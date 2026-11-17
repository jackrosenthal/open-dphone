# `storage`

The app's private key-value store on the phone.  The store survives restarts of the app and the phone, firmware updates, and newer versions of the app.  The store belongs to the app's `name`; no other app can access the store, and removing the app deletes the store.

- Key: 1 to 64 bytes of UTF-8, not starting with `.`, without `/`, `\`, or NUL.
- Value: any bytes, up to 1 MiB.

Each key is a file on the flash.  Writes use free flash, of which the phone reserves 2 MiB.

## `get`

```rust
pub fn get(key: &str) -> Option<Vec<u8>>
```

The key's value, or `None` if the key isn't set or is invalid.  For UTF-8 text, the helper [`read_string`](helpers.md#logging-and-storage) returns a `String`.

## `set`

```rust
pub fn set(key: &str, value: &[u8]) -> Result<(), Error>
```

The write is atomic: after a power loss, the key has the old value or the new one, never part of a value.

| Error | Cause |
| --- | --- |
| `Invalid(text)` | The key is empty, is longer than 64 bytes, starts with `.`, or contains `/`, `\`, or NUL; or the value is larger than 1 MiB. |
| `NoSpace` | The value needs more space than the flash has free beyond the 2 MiB reserve.  Replacing a value counts only the growth. |
| `Failed(text)` | The write failed; `text` gives the system's reason. |

## `remove`

```rust
pub fn remove(key: &str) -> Result<(), Error>
```

Removing a key that isn't set succeeds.  Returns `Invalid` under the same conditions as `set`, and `Failed(text)` if the file can't be deleted.

## `keys`

```rust
pub fn keys() -> Vec<String>
```

The keys that are set, in byte order.
