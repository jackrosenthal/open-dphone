---
title: "canvas"
---

A canvas is a widget that shows a list of drawing operations.  [`draw`](#draw) replaces the list in one call, and the phone repaints from the list as needed.  The [`ui`](ui.md) functions move, hide, and destroy a canvas.  To animate, call `draw` from a repeating [timer](timers.md), at most 20 times per second.

## Types

```rust
pub enum Role {
    Background, Foreground, Highlight, HighlightText,
    TitleBackground, TitleText, Secondary, Border,
}

pub struct Color { pub r: u8, pub g: u8, pub b: u8 }

pub struct Bitmap {
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
    pub scale: u8,
    pub color: Color,
    pub bits: Vec<u8>,
}

pub struct Text {
    pub box_: Rect,
    pub font: Font,
    pub color: Color,
    pub text: String,
}

pub enum Op {
    Fill((Rect, Color)),
    Bitmap(Bitmap),
    Text(Text),
}
```

The grayscale skins map each `Color` to the nearest of four shades (black, dark gray, light gray, and white) by brightness.

Operations paint in list order, later on top.  Coordinates are relative to the canvas's top-left corner, and drawing is clipped to the canvas.

| Operation | Draws |
| --- | --- |
| `Op::Fill((rect, color))` | A filled rectangle. |
| `Op::Bitmap(bitmap)` | A one-color image.  `bits` holds one bit per pixel, rows from the top, each row padded to whole bytes, the leftmost pixel in the high bit: `bits.len()` must be `width.div_ceil(8) * height`.  A set bit paints a `color` square `scale` pixels on a side (1 to 8); a clear bit is transparent.  `x` and `y` place the top-left corner; `width` and `height` (1 to 256) are the unscaled size. |
| `Op::Text(text)` | One line of text, at most 64 bytes, in one color and a skin font (as for [`ui::set_font`](ui.md#set_font)).  The painted pixels of the glyphs are centered in `box_`, horizontally and vertically, so a digit sits in the middle of a square.  Drawing is clipped to `box_`. |

## `theme_color`

```rust
pub fn theme_color(role: Role) -> Color
```

The skin's current color for `role`:

| `Role` | Color |
| --- | --- |
| `Background`, `Foreground` | The window's background and text colors. |
| `Highlight`, `HighlightText` | The background and text colors of a list's selected row. |
| `TitleBackground`, `TitleText` | The background and text colors of a title. |
| `Secondary` | Secondary text. |
| `Border` | Separators and borders. |

On a color model, the colors come from the user's theme and change when the user switches themes; the app on screen then receives [`Event::Foreground`](app-events.md#foreground).

## `text_size`

```rust
pub fn text_size(font: Font, text: &str) -> Size
```

The size of the pixels that the glyphs of `text` paint in `font`, without line spacing.  `Op::Text` centers by this measure.  Only the first 64 bytes are measured.

## `create`

```rust
pub fn create(parent: Widget, bounds: Rect) -> Widget
```

Creates an empty canvas.  `parent` and `bounds` work as in [`ui::create`](ui.md#create), including the result 0 for a parent in an idle window that the app doesn't have.

## `draw`

```rust
pub fn draw(w: Widget, ops: &[Op]) -> Result<(), String>
```

Replaces the canvas's list.  An empty list clears the canvas.

`draw` rejects the whole list, leaving the canvas unchanged, if:

- the list has more than 512 operations,
- a `Text` is longer than 64 bytes,
- a `Bitmap` has a `width` or `height` of 0 or over 256, or a `scale` outside 1 to 8,
- a `Bitmap`'s `bits.len()` isn't `width.div_ceil(8) * height`, or
- the `bits` of all bitmaps total more than 64 KiB.

The `Err` text names the limit.

`draw` checks the list before the handle.  For a valid list and a handle that doesn't exist, is `ROOT`, or isn't a canvas, `draw` returns `Err("not a canvas")` and changes nothing.
