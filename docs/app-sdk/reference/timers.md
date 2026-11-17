# `timers`

## `set`

```rust
pub fn set(delay_ms: u32, repeat: bool) -> Result<u32, Error>
```

Starts a timer and returns the timer's ID, which is never 0.  The app receives [`Event::Timer(id)`](app-events.md#timer) after `delay_ms` milliseconds.  With `repeat`, the event arrives again every `delay_ms` milliseconds until the timer is canceled.  A one-shot timer is gone once the event has been sent.

- An app can have at most 32 timers at a time.  Beyond that, `set` starts no timer and returns `Err(Error::Failed(_))`.
- A repeating timer with a delay under 50 milliseconds runs at 50 milliseconds.
- Timers run whether or not the app is on screen, and end with the instance.
- Timer events don't queue up.  If the phone is busy for several periods of a repeating timer, a single event arrives, and the next is due one full period later.

## `cancel`

```rust
pub fn cancel(id: u32)
```

Stops a timer.  No further events arrive for that timer.  An ID that doesn't belong to a timer is ignored.

## `now_ms`

```rust
pub fn now_ms() -> u64
```

Returns the time in milliseconds on a monotonic clock, for measuring intervals.  The clock starts at zero when the phone's main process starts, not when the app starts.

## `wall_time`

```rust
pub fn wall_time() -> u64
```

Returns the phone's wall-clock time, in seconds since the Unix epoch (UTC).  [`l10n::format_time`](l10n.md#format_time) converts the value to local time.
