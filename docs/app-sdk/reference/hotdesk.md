# `hotdesk`

Hot desking: changing which user the phone is provisioned for.

## `login`

```rust
pub fn login(extension: &str, pin: &str) -> Result<(), Error>
```

Starts provisioning the phone for a user, given the user's extension and PIN.

`Ok` means that the phone has started asking the PBX for the user's configuration, not that the PBX accepted the extension and PIN.  The outcome arrives later as [`Event::HotdeskDone`](app-events.md#hotdeskdone), delivered to the app that called `login`:

- `HotdeskDone(Ok(()))`: the PBX accepted the user, and the phone has the user's configuration.  Apps that use `config` receive `Event::NewConfig`, and `current_user` returns the new user.
- `HotdeskDone(Err(text))`: the login failed.  `text` is the PBX's reason, followed by the PBX's error code in parentheses if the PBX supplied one (for a wrong PIN, or an extension that the PBX doesn't know).  If the PBX couldn't be reached, `text` is the phone's reason.  The phone goes back to the previous user and configuration, and `current_user` is unchanged.

The phone asks the PBX about every login, even when the phone already has a user.  If the app leaves the screen and is stopped before the outcome arrives, the app receives no event, but the login continues.

Errors:

- `Invalid("bad extension or PIN")` if `extension` is empty or longer than 64 bytes, or `pin` is longer than 64 bytes.
- `Invalid(text)` or `Failed(text)`, with the reason, if the phone's provisioning rejects the request.
- `Failed(text)` if the phone's provisioning doesn't accept the request within 0.5 seconds.  The phone may still act on the request afterward.

## `logout`

```rust
pub fn logout() -> Result<(), Error>
```

Starts provisioning the phone with no user, which returns the phone to the configuration for the phone's MAC address.  `Ok`, `Invalid`, and `Failed` have the same meanings as for `login`, and the outcome arrives as [`Event::HotdeskDone`](app-events.md#hotdeskdone) in the same way.

A PBX that provisions phones by user has no configuration for a phone without a user.  On such a PBX, the logout ends with `HotdeskDone(Ok(()))`, `current_user` returns `""`, and the phone keeps the last configuration in effect until the next `login`.

## `current_user`

```rust
pub fn current_user() -> String
```

Returns the user that the phone is assigned to, or `""` if the phone has no user.
