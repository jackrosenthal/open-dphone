# `runtime`

`background`, `exit`, `restart`, and `show_app` queue a request.  The phone carries out queued requests in order after `start` or `handle` returns.

## `background`

```rust
pub fn background()
```

Takes the app's window off the screen, revealing the screen underneath.  The app then receives [`Event::Background`](app-events.md#background), and the phone stops a `Foreground` app unless the app called `set_exit_after_background(false)`.  Does nothing if the window isn't on screen.

## `exit`

```rust
pub fn exit()
```

Stops the app.  After the handler returns, the app receives `Event::Exit`, and the phone discards the instance and closes the app's windows.  Requests queued after `exit` in the same handler are dropped.

## `restart`

```rust
pub fn restart()
```

Stops the app as `exit` does, then starts a new instance, shown if the app was on screen.

## `set_exit_after_background`

```rust
pub fn set_exit_after_background(exit: bool)
```

Whether a `Foreground` app stops when the window leaves the screen; `true` in each new instance.  `Background` apps and apps with an idle window keep running regardless.

## `show_app`

```rust
pub fn show_app(name: &str) -> Result<(), String>
```

Opens the app whose manifest `name` is `name`: starts the app if needed, and brings the app's window to the front.

- `Ok`: the request is queued.  If the other app then fails to start, the phone logs a warning; the caller isn't notified.
- `Err`, with the reason: `name` isn't a well-formed app name, no app has that name, or the name belongs to a built-in app that the configuration disables.

## `reset_idle_timer`

```rust
pub fn reset_idle_timer()
```

Registers user activity: wakes the backlight and restarts the backlight timeout.  Works only while the app's window is on screen.

## `log`

```rust
pub fn log(level: u8, message: &str)
```

Writes a line to the phone's log, prefixed with the app's name.  `level`: 0 error, 1 warning, 2 information, 3 or higher debugging.  A message longer than 512 bytes is truncated at the last whole character that fits.  The SDK has a [helper for each level](helpers.md#logging-and-storage).

## `in_foreground`

```rust
pub fn in_foreground() -> bool
```

Whether the app's window is the top screen: `true` from `Event::Foreground` until `Event::Background`.  The idle window doesn't count.

## `random`

```rust
pub fn random() -> u64
```

64 random bits from the kernel's random source.
