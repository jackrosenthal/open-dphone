# `hotdesk`

Changing which user the phone is provisioned for.  Capability `hotdesk`.

## `login`

```rust
pub fn login(extension: &str, pin: &str) -> Result<(), Error>
```

Starts provisioning the phone for a user, with the user's extension and PIN.

`Ok` means that the phone has begun to ask the PBX for the user's configuration, not that the PBX accepted the extension and the PIN.  The outcome shows later: on success the phone applies the user's configuration (apps that use `config` receive `Event::NewConfig`, and `current_user` changes).

`Invalid("bad extension or PIN")` when `extension` is empty or longer than 64 bytes, or `pin` is longer than 64 bytes.  `Failed(text)` when the phone's provisioning does not take the request within 0.5 seconds.

## `logout`

```rust
pub fn logout() -> Result<(), Error>
```

Starts provisioning the phone without a user, which returns the phone to the configuration for the phone's MAC address.  `Ok` and `Failed` mean what they mean for `login`.

## `current_user`

```rust
pub fn current_user() -> String
```

WIT name: `current-user`.

The user the phone is assigned to, or `""` when the phone has no user.
