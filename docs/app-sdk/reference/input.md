# `input`

## `capture`

```rust
pub fn capture(on: bool)
```

Turns key capture on or off.  While capture is on and the app's window is the top screen:

- Every key press arrives as [`Event::Key`](app-events.md#key).  That includes the keys that a window otherwise never receives (line key 1, the volume keys, `mute`, `spkr`, and `hdst`), the four keys below the display, and the keys that a widget would normally handle.
- Every hookswitch change arrives as [`Event::Hook`](app-events.md#hook).
- The phone does nothing else with these keys.  The volume doesn't change, softkeys don't trigger, widgets don't react, key claims aren't consulted, and lifting the handset doesn't produce a dial tone.

With capture off, keys are routed normally again (see [Focus and keys](ui.md#focus-and-keys)).

The user can always get out: the third consecutive press of `cancel` takes the app off the screen, and is not delivered to the app.  The first two presses are delivered.  Any other key in between resets the count.

A call or a notice from the phone covers a capturing window like any other, and gets the keys while showing.  A notice takes `select`, `cancel`, and the four keys below the display even when the notice is over a capturing window.  Capture stays set while the window is off screen, and resumes when the window returns.
