# `runtime`

App lifecycle control, logging, and random numbers.  Every app has `runtime`, which is not a capability.

`background`, `exit`, `restart`, and `show_app` only queue a request.  The phone carries out queued requests, in order, after the current `start` or `handle` returns.

## `background`

```rust
pub fn background()
```

Takes the app's window off the screen and reveals the screen underneath.  The app then receives [`Event::Background`](app-events.md#background), and the phone stops a `Foreground` app unless the app called `set_exit_after_background(false)`.  Does nothing if the window is not on screen.

## `exit`

```rust
pub fn exit()
```

Stops the app.  After the current handler returns, the app receives `Event::Exit`, and the phone then discards the instance and closes the app's windows.  Requests queued after `exit` in the same handler are dropped.

## `restart`

```rust
pub fn restart()
```

Stops the app the same way as `exit`, and then starts a fresh instance.  Memory, widgets, timers, pending requests, and defined images are all gone, and `start` runs again.  If the app was on screen, the phone shows the new instance.

## `set_exit_after_background`

```rust
pub fn set_exit_after_background(exit: bool)
```

Controls whether a `Foreground` app stops when the window leaves the screen.  Pass `false` to keep running.  The setting is `true` in every new instance.  `Background` apps and apps with an idle window keep running regardless.

## `show_app`

```rust
pub fn show_app(name: &str) -> Result<(), String>
```

Opens another app, identified by the `name` in that app's manifest.  The phone starts the other app if necessary and brings that app's window to the front.

Returns `Ok` once the request is queued.  If the other app then fails to start, the app isn't shown, the phone logs a warning, and the caller is not notified.

Returns `Err`, with a message that explains why, when `name` is not a well-formed app name, when no app has that name, or when the name belongs to a built-in app that the configuration disables.

## `reset_idle_timer`

```rust
pub fn reset_idle_timer()
```

Registers user activity: wakes the backlight and restarts the backlight timeout.  Has an effect only while the app's window is on screen.

## `log`

```rust
pub fn log(level: u8, message: &str)
```

Writes a line to the phone's log, prefixed with the app's name.  `level` is 0 for an error, 1 for a warning, 2 for information, and 3 or higher for debugging.  A message longer than 512 bytes is truncated at the last whole character that fits.

The SDK has a [helper for each level](helpers.md#logging-and-storage): `log_error`, `log_warn`, `log_info`, and `log_debug`.

Panics are logged automatically.  [`export_app!`](manifest.md#export_app) installs a panic hook that writes `panicked at <file>:<line>: <message>` at level 0 before the phone stops the app.

## `in_foreground`

```rust
pub fn in_foreground() -> bool
```

Returns whether the app's window is the top screen: `true` from `Event::Foreground` until `Event::Background`.  The idle window doesn't count.

## `random`

```rust
pub fn random() -> u64
```

Returns 64 random bits from the kernel's random source.  Use the result to seed a game's random number generator, for example.
