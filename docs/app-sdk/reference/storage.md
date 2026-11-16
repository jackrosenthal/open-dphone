# `storage`

The app's private key-value store on the phone.

The store survives restarts of the app and of the phone, firmware updates, and installation of a newer version of the app.  The store is tied to the app's `name`, and no other app can access the data.  Removing the app deletes the store.

A key is a name of 1 to 64 bytes that doesn't start with `.` and contains no `/`, `\`, or NUL character.  The limit counts UTF-8 bytes, not characters, and is the same number as the limit on a [`pbx_storage`](pbx-storage.md) key.  A value is any sequence of bytes, up to 1 MiB.  Each key is stored as a file on the phone's flash, and writes count against the flash's free space, of which the phone always keeps 2 MiB in reserve.

## `get`

```rust
pub fn get(key: &str) -> Option<Vec<u8>>
```

Returns the value of a key, or `None` if the key isn't set.  An invalid key also returns `None`.

For a value that is UTF-8 text, the SDK helper [`read_string`](helpers.md#logging-and-storage) returns a `String` instead.

## `set`

```rust
pub fn set(key: &str, value: &[u8]) -> Result<(), Error>
```

Sets a key, replacing any existing value.  The write is atomic: the phone writes the new value to the flash and then swaps the value into place in one step, so after a power loss the key has either the old value or the new one, never a partial value.

```rust
storage::set("level", level.to_string().as_bytes())?;
let level: u32 = read_string("level").and_then(|s| s.parse().ok()).unwrap_or(1);
```

| Error | Cause |
| --- | --- |
| `Invalid(text)` | The key is empty, is longer than 64 bytes, starts with `.`, or contains `/`, `\`, or NUL; or the value is larger than 1 MiB.  The phone checks the key before touching the flash. |
| `NoSpace` | The value needs more space than the flash has free beyond the 2 MiB reserve.  Replacing a value counts only the growth. |
| `Failed(text)` | The write failed, and `text` gives the system's reason.  This error also occurs when the key's file in the store is a symbolic link, which the phone refuses to follow; `text` says so. |

## `remove`

```rust
pub fn remove(key: &str) -> Result<(), Error>
```

Removes a key.  Removing a key that isn't set succeeds.

Returns `Invalid` under the same conditions as `set`, and `Failed(text)` when the file can't be deleted or is a symbolic link.

## `keys`

```rust
pub fn keys() -> Vec<String>
```

Returns the keys that are set, sorted in byte order.  The list is empty for an app that has never stored anything.
