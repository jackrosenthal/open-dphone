# `input`

Every key and the hookswitch.  Capability `input`.

## `capture`

```rust
pub fn capture(on: bool)
```

With `true`, while the app's window is the top screen:

- Every key press arrives as [`Event::Key`](app-events.md#key): the keys a window otherwise never gets (line key 1, the volume keys, `mute`, `spkr`, and `hdst`), the four keys under the display, and the keys that a widget would handle.
- Every change of the hookswitch arrives as [`Event::Hook`](app-events.md#hook).
- The phone does nothing with these keys: the volume does not change, softkeys do not trigger, widgets do not react, key claims are not consulted, and lifting the handset gives no dial tone.

With `false`, keys go the usual way again ([Focus and keys](ui.md#focus-and-keys)).

The user can always leave: the third press of `cancel` in a row takes the app off the screen and is not delivered; the first two are.  Any other key in between starts the count again.

A call or a notice of the phone covers the window as any other and has the keys while showing; a notice takes `select`, `cancel`, and the four keys under the display even over a capturing window.  Capture stays set while the window is off screen and applies again when the window returns.
