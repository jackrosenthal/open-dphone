---
title: "diagnostics"
---

## `health`

```rust
pub fn health() -> Vec<SubsystemHealth>
```

```rust
pub enum Subsystem { Core, Configuration, Network, Sip, Media, Dpma, Ui, Apps, Web, Ssh, Update }
pub enum HealthState { Ok, Starting, Stopped, Degraded(String), Failed(String) }
pub struct SubsystemHealth { pub subsystem: Subsystem, pub health: HealthState }
```

Every subsystem of the phone with the subsystem's state, in the order of `Subsystem`.

| `HealthState` | Meaning |
| --- | --- |
| `Ok` | Working. |
| `Starting` | Starting. |
| `Stopped` | Not started, or not part of this phone. |
| `Degraded(why)` | Working, with a fault. |
| `Failed(why)` | Not working. |

## `log_tail`

```rust
pub fn log_tail(max: u32) -> Vec<String>
```

The newest entries of the phone's log, newest first: at most `max`, and never more than 100.  Each entry is `<source> <message>`, where the source is the part of the phone that wrote the entry; lines from `runtime::log` have the source `app` and start with the app's name.  Entries have no time or level.
