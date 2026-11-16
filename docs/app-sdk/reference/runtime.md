# `runtime`

Leaving the screen, exiting, logging, and random numbers.  Every app has `runtime`; the interface is not a capability.

`background`, `exit`, `restart`, and `show_app` only record a request.  The phone carries the requests out, in order, after the current `start` or `handle` returns.

## `background`

```rust
pub fn background()
```

Takes the app's window off the screen; the screen underneath shows again.  The app then receives [`Event::Background`](app-events.md#background), and an app of kind `Foreground` is stopped unless `set_exit_after_background(false)` was called.  When the window is not on screen, nothing happens.

## `exit`

```rust
pub fn exit()
```

Stops the app.  After the current handler returns, the app receives `Event::Exit`, and the phone drops the instance and closes the app's windows.  Requests recorded after `exit` in the same handler are discarded.

## `restart`

```rust
pub fn restart()
```

Stops the app as `exit` does and starts a fresh instance: memory, widgets, timers, pending requests, and defined pictures are gone, and `start` runs again.  If the app was on screen, the new instance is shown.

## `set_exit_after_background`

```rust
pub fn set_exit_after_background(exit: bool)
```

WIT name: `set-exit-after-background`.

With `false`, an app of kind `Foreground` keeps running when the window leaves the screen.  The setting starts as `true` in every new instance.  Apps of kind `Background` or `Idle` and apps with an idle window keep running in either case.

## `show_app`

```rust
pub fn show_app(name: &str) -> Result<(), String>
```

Opens another app by the `name` of the app's manifest, starting the app if needed, and puts the app's window on top.

The result only says whether `name` is formed like an app name: `Err("bad app name")` otherwise.  When no app has the name, the app is a disabled built-in app, or the app fails to start, the phone writes a warning to the log and the calling app is not told.

## `reset_idle_timer`

```rust
pub fn reset_idle_timer()
```

Counts as user activity: wakes the backlight and restarts the backlight's timeout.  The call has an effect only while the app's window is on screen.

## `log`

```rust
pub fn log(level: u8, message: &str)
```

Writes a line to the phone's log, prefixed with the app's name.  `level` is 0 for an error, 1 for a warning, 2 for information, and 3 or more for debugging.  A message longer than 512 bytes is cut at the last whole character within 512 bytes.

The SDK has a helper for each level: `log_error`, `log_warn`, `log_info`, and `log_debug`, each taking `&str`.

## `in_foreground`

```rust
pub fn in_foreground() -> bool
```

Whether the app's window is the top screen: `true` from `Event::Foreground` until `Event::Background`.  The idle window does not count.

## `random`

```rust
pub fn random() -> u64
```

64 random bits from the kernel's random source, e.g., to seed a game's generator.
