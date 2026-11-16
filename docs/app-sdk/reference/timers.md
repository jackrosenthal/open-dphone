# `timers`

Timers and clocks.  Capability `timers`.

## `set`

```rust
pub fn set(delay_ms: u32, repeat: bool) -> Result<u32, Error>
```

Starts a timer and returns the timer's identifier.  The app receives [`Event::Timer(id)`](app-events.md#timer) when `delay_ms` milliseconds have passed; with `repeat`, the event arrives again every `delay_ms` milliseconds until the timer is cancelled.  A timer that does not repeat is gone after the event.

- An app has at most 32 timers at a time.  Past that, `set` starts no timer and returns `Err(Error::Failed(_))`.  An identifier is never 0.
- A repeating timer with a delay under 50 milliseconds runs at 50.
- Timers run whether or not the app is on screen, and end with the instance.
- Timers do not queue up.  When the phone is busy past several periods of a repeating timer, one event arrives, and the next is due one full period later.

```rust
self.tick = timers::set(1000, true).ok();   // an Option<u32>
// in handle:
// Event::Timer(id) if Some(id) == self.tick => self.redraw_clock(),
```

## `cancel`

```rust
pub fn cancel(id: u32)
```

Stops a timer.  No event for the timer arrives afterward.  An identifier that names no timer is ignored.

## `now_ms`

```rust
pub fn now_ms() -> u64
```

WIT name: `now-ms`.

Milliseconds on a clock that only moves forward and that changes of the wall clock do not affect; for measuring intervals.  The zero point is the start of the phone's main process, not of the app.

## `wall_time`

```rust
pub fn wall_time() -> u64
```

WIT name: `wall-time`.

The phone's wall clock: seconds since the Unix epoch (UTC).  [`l10n::format_time`](l10n.md#format_time) turns the value into local time.
