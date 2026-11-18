---
title: "lamps"
---

An app's lamp colors show only while the app's window is the top screen.  During a call, under another screen, and after the app leaves the screen, exits, or faults, the lamps show the phone's normal indication.  The settings persist while the window is covered and return with the window.

```rust
pub enum Color { Off, Red, Green, Amber }
pub enum Lamp { Line(u8), Blf(u8), Message, Mute, Headset, Speaker }
```

## `list`

```rust
pub fn list() -> Vec<Lamp>
```

The lamps of this model, in this order:

| `Lamp` | Lamps |
| --- | --- |
| `Line(1)`, `Line(2)`, ... | The line key lamps, top to bottom: 2 on the D40, D45, D60, and D62; 4 on the D50; and 6 on the D65 and D70. |
| `Blf(1)` to `Blf(10)` | The rapid-dial key lamps, top to bottom, on the D50 and D70. |
| `Message` | The message-waiting lamp. |
| `Mute`, `Headset`, `Speaker` | The lamps on the mute, headset, and speaker keys. |

## `set`

```rust
pub fn set(lamp: Lamp, color: Color)
```

Shows a steady color on a lamp, overriding the phone's normal indication.  `Color::Off` keeps the lamp dark.  A lamp that `list` doesn't list, and a color that the lamp can't show, are ignored.

## `release`

```rust
pub fn release()
```

Returns every lamp to the phone.  Lamps can't be released individually.
