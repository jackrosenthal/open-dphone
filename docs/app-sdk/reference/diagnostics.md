# `diagnostics`

Subsystem health and the phone's log.

## `health`

```rust
pub fn health() -> Vec<(String, String)>
```

Returns every subsystem of the phone with that subsystem's state, in a fixed order.  The subsystems are `Core`, `Configuration`, `Network`, `SIP`, `Media`, `DPMA`, `UI`, `Apps`, `Web`, `SSH`, and `Update`.  The state is one of:

| State | Meaning |
| --- | --- |
| `ok` | Working. |
| `starting` | Starting. |
| `stopped` | Not started, or not part of this phone. |
| `degraded: <why>` | Working, with a fault. |
| `failed: <why>` | Not working. |

## `log_tail`

```rust
pub fn log_tail(max: u32) -> Vec<String>
```

Returns the newest entries in the phone's log, newest first.  The result has at most `max` entries, and never more than 100.

Each entry has the form `<source> <message>`, where the source is the part of the phone that wrote the entry.  Lines that apps write with `runtime::log` have the source `app` and start with the app's name.  Entries don't include a time or a level.
