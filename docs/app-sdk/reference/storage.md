# `storage`

The app's store of keys and values on the phone.  Capability `storage`.

The phone keeps the store across restarts of the app and of the phone, across firmware updates, and when a newer version of the app is installed.  No other app can reach the store; the store belongs to the app's `name`.  Removing the app deletes the store.

A key is a name of 1 to 64 bytes that does not start with `.` and contains no `/`, `\`, or NUL character.  The limit counts bytes of UTF-8, not characters, and is the same number as the limit of a [`pbx_storage`](pbx-storage.md) key.  A value is any bytes, at most 1 MiB.  Each key is a file on the phone's flash, and writes count against the flash's free space, of which the phone always keeps 2 MiB unused.

## `get`

```rust
pub fn get(key: &str) -> Option<Vec<u8>>
```

The value of a key, or `None` when the key is not set.  A key that could not be a key also returns `None`.

The SDK adds `read_string(key: &str) -> Option<String>`, which returns the value when the value is UTF-8 text.

## `set`

```rust
pub fn set(key: &str, value: &[u8]) -> Result<(), Error>
```

Sets a key, replacing the value the key had.  The phone writes the new value to the flash and then puts the value in place in one step, so that after a power loss the key has either the old value or the new one, never part of one.

| Error | Cause |
| --- | --- |
| `Invalid(text)` | The key is empty, longer than 64 bytes, starts with `.`, or contains `/`, `\`, or NUL; or the value is larger than 1 MiB.  The phone checks the key before touching the flash. |
| `NoSpace` | The value needs more room than the flash has free beyond the 2 MiB reserve.  Replacing a value counts only the growth. |
| `Failed(text)` | The write failed; the text is the system's reason.  Also when the key's file in the store is a symbolic link, which the phone does not follow; the text says so. |

```rust
storage::set("level", level.to_string().as_bytes())?;
let level: u32 = read_string("level").and_then(|s| s.parse().ok()).unwrap_or(1);
```

## `remove`

```rust
pub fn remove(key: &str) -> Result<(), Error>
```

Removes a key.  Removing a key that is not set succeeds.  The errors are `Invalid` as for `set`, and `Failed(text)` when the file cannot be deleted or is a symbolic link.

## `keys`

```rust
pub fn keys() -> Vec<String>
```

The keys that are set, sorted by byte order.  An app with no store yet gets an empty list.
