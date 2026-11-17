# `hotdesk`

Hot desking: changing the user that the phone is provisioned for.

## `login`

```rust
pub fn login(extension: &str, pin: &str) -> Result<(), Error>
```

Starts provisioning the phone for the user with `extension` and `pin`.  `Ok` means that the phone has asked the PBX for the user's configuration, not that the PBX accepted.  The outcome arrives at this app as [`Event::HotdeskDone`](app-events.md#hotdeskdone):

- `HotdeskDone(Ok(()))`: the PBX accepted the user, and the phone has the user's configuration.  Apps that use `config` receive `Event::NewConfig`, and `current_user` returns the new user.
- `HotdeskDone(Err(text))`: the login failed.  `text` is the PBX's reason, followed by the PBX's error code in parentheses if supplied (for a wrong PIN, or an extension that the PBX doesn't know), or the phone's reason if the PBX couldn't be reached.  The phone returns to the previous user and configuration; `current_user` is unchanged.

The phone asks the PBX about every login, even when the phone already has a user.  If the app stops before the outcome arrives, the login continues, and no event arrives.

| Error | Cause |
| --- | --- |
| `Invalid("bad extension or PIN")` | `extension` is empty or longer than 64 bytes, or `pin` is longer than 64 bytes. |
| `Invalid(text)` or `Failed(text)` | The phone's provisioning rejected the request; `text` says why. |
| `Failed(text)` | The phone's provisioning didn't accept the request within 0.5 seconds, and may still act on the request. |

## `logout`

```rust
pub fn logout() -> Result<(), Error>
```

Starts provisioning the phone with no user, which returns the phone to the configuration for the phone's MAC address.  Results and [`Event::HotdeskDone`](app-events.md#hotdeskdone) as for `login`.

A PBX that provisions phones by user has no configuration for a phone without a user.  There, the logout ends with `HotdeskDone(Ok(()))`, `current_user` returns `""`, and the phone keeps the last configuration until the next `login`.

## `current_user`

```rust
pub fn current_user() -> String
```

The phone's user, or `""` if the phone has none.
