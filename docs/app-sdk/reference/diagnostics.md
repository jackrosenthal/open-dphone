# `diagnostics`

The state of the phone's subsystems, and the log.  Capability `diagnostics`.

## `health`

```rust
pub fn health() -> Vec<(String, String)>
```

Every subsystem of the phone with the subsystem's state, in a fixed order.  The names are `Core`, `Configuration`, `Network`, `SIP`, `Media`, `DPMA`, `UI`, `Apps`, `Web`, `SSH`, and `Update`.  The state is one of:

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

WIT name: `log-tail`.

The newest entries of the phone's log, newest first: at most `max` of them, and never more than 100.  Each entry is `<source> <message>`, where the source names the part of the phone that wrote the entry; lines that apps write with `runtime::log` have the source `app` and begin with the app's name.  The entries carry no time and no level.
