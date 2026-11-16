# `lamps`

The phone's lamps.  Capability `lamps`.

A color that an app sets shows only while the app's window is the top screen.  Under a call or another screen, and after the app has left the screen, exited, or failed, every lamp shows what the phone shows there.  The app's settings are kept while the window is covered and show again when the window returns.

```rust
pub enum Color { Off, Red, Green, Amber }
```

`lamps::Color` is a different type from `canvas::Color`.

## `names`

```rust
pub fn names() -> Vec<String>
```

The lamps of this model, in this order:

| Names | Lamps |
| --- | --- |
| `line1`, `line2`, ... | The lamps of the line keys, from the top: 2 on the D40, D45, D60, and D62, 4 on the D50, and 6 on the D65 and D70. |
| `blf1` to `blf10` | The lamps of the rapid-dial keys, from the top, on the D50 and D70. |
| `message` | Message waiting. |
| `mute`, `hdst`, `spkr` | The lamps of the mute, headset, and speaker keys. |

## `set`

```rust
pub fn set(lamp: &str, color: Color)
```

Shows a steady color on a lamp in place of what the phone shows there.  `Color::Off` holds the lamp dark.  A name that `names` does not list is ignored, and so is a color that the lamp cannot show.

## `release`

```rust
pub fn release()
```

Gives every lamp back to the phone.  A single lamp cannot be released.
