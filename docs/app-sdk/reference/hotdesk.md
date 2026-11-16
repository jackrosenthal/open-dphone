# `hotdesk`

Changing which user the phone is provisioned for.  Capability `hotdesk`.

## `login`

```rust
pub fn login(extension: &str, pin: &str) -> Result<(), Error>
```

Starts provisioning the phone for a user, with the user's extension and PIN.

`Ok` means that the phone has begun to ask the PBX for the user's configuration, not that the PBX accepted the extension and the PIN.  The outcome arrives later as [`Event::HotdeskDone`](app-events.md#hotdeskdone), at the app that called `login`:

- `HotdeskDone(Ok(()))`: the PBX accepted the user, and the phone has the user's configuration.  Apps that use `config` receive `Event::NewConfig`, and `current_user` returns the user.
- `HotdeskDone(Err(text))`: the login failed.  `text` is the PBX's reason, with the PBX's error code in parentheses when the PBX gave one (a wrong PIN, an extension the PBX does not know), or the phone's reason when the PBX could not be asked.  The phone then returns to the user and the configuration from before the call, and `current_user` is unchanged.

The phone asks the PBX about every login, also when the phone already has a user.  An app that leaves the screen and is stopped before the outcome arrives receives no event; the login goes on.

`Invalid("bad extension or PIN")` when `extension` is empty or longer than 64 bytes, or `pin` is longer than 64 bytes.  `Invalid(text)` or `Failed(text)`, with the reason, when the phone's provisioning refuses the request.  `Failed(text)` also when the phone's provisioning does not take the request within 0.5 seconds; the phone may still act on the request afterward.

## `logout`

```rust
pub fn logout() -> Result<(), Error>
```

Starts provisioning the phone without a user, which returns the phone to the configuration for the phone's MAC address.  `Ok`, `Invalid`, and `Failed` mean what they mean for `login`, and the outcome arrives as [`Event::HotdeskDone`](app-events.md#hotdeskdone) in the same way.

A PBX that provisions phones by user has no configuration for a phone without a user.  On such a PBX the logout ends with `HotdeskDone(Ok(()))`, `current_user` returns `""`, and the phone keeps the last configuration in effect and waits for the next `login`.

## `current_user`

```rust
pub fn current_user() -> String
```

WIT name: `current-user`.

The user the phone is assigned to, or `""` when the phone has no user.
