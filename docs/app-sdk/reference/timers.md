# `timers`

## `set`

```rust
pub fn set(delay_ms: u32, repeat: bool) -> Result<u32, Error>
```

Starts a timer and returns the timer's ID, which is never 0.  [`Event::Timer(id)`](app-events.md#timer) arrives after `delay_ms` milliseconds and, with `repeat`, every `delay_ms` milliseconds until the timer is canceled.  A one-shot timer ends after the event.

- An app has at most 32 timers.  Beyond that, `set` starts no timer and returns `Err(Error::Failed(_))`.
- A repeating timer under 50 milliseconds runs at 50 milliseconds.
- Timers run on and off screen, and end with the instance.
- Timer events don't queue.  After several missed periods of a repeating timer, one event arrives, and the next is due one full period later.

## `cancel`

```rust
pub fn cancel(id: u32)
```

Stops a timer; no further events arrive.  An unknown ID is ignored.

## `now_ms`

```rust
pub fn now_ms() -> u64
```

Milliseconds on a monotonic clock, for measuring intervals.  The clock starts at zero when the phone's main process starts, not when the app starts.

## `wall_time`

```rust
pub fn wall_time() -> u64
```

The phone's wall-clock time, in seconds since the Unix epoch (UTC).  [`l10n::format_time`](l10n.md#format_time) converts the value to local time.
