# `diagnostics`

## `health`

```rust
pub fn health() -> Vec<(String, String)>
```

Every subsystem of the phone with the subsystem's state, in this order: `Core`, `Configuration`, `Network`, `SIP`, `Media`, `DPMA`, `UI`, `Apps`, `Web`, `SSH`, and `Update`.

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

The newest entries of the phone's log, newest first: at most `max`, and never more than 100.  Each entry is `<source> <message>`, where the source is the part of the phone that wrote the entry; lines from `runtime::log` have the source `app` and start with the app's name.  Entries have no time or level.
