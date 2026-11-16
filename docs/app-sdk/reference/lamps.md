# `lamps`

Control of the phone's lamps.

A color that an app sets shows only while the app's window is the top screen.  During a call, under another screen, and after the app has left the screen, exited, or faulted, every lamp shows the phone's normal indication.  The app's settings are remembered while the window is covered, and take effect again when the window returns.

```rust
pub enum Color { Off, Red, Green, Amber }
```

`lamps::Color` is a different type from `canvas::Color`.

## `names`

```rust
pub fn names() -> Vec<String>
```

Returns the names of the lamps on this model, in this order:

| Names | Lamps |
| --- | --- |
| `line1`, `line2`, ... | The line key lamps, from top to bottom: 2 on the D40, D45, D60, and D62; 4 on the D50; and 6 on the D65 and D70. |
| `blf1` to `blf10` | The rapid-dial key lamps, from top to bottom, on the D50 and D70. |
| `message` | The message-waiting lamp. |
| `mute`, `hdst`, `spkr` | The lamps on the mute, headset, and speaker keys. |

## `set`

```rust
pub fn set(lamp: &str, color: Color)
```

Shows a steady color on a lamp, overriding the phone's normal indication.  `Color::Off` keeps the lamp dark.  A name that `names` doesn't list is ignored, as is a color that the lamp can't show.

## `release`

```rust
pub fn release()
```

Returns every lamp to the phone's control.  Lamps can't be released individually.
