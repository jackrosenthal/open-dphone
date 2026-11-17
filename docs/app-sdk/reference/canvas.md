# `canvas`

A canvas is a widget that displays a list of drawing operations, which [`draw`](#draw) replaces in one call.  The phone repaints from the list whenever the screen needs repainting.  The [`ui`](ui.md) functions move, hide, and destroy a canvas like any other widget.  To animate, call `draw` from a repeating [timer](timers.md), which can fire up to 20 times per second.

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

The grayscale skins map each color to the nearest of four shades (black, dark gray, light gray, and white) by brightness.

Operations are painted in list order, with later operations on top of earlier ones.  Coordinates are relative to the top-left corner of the canvas, and drawing is clipped to the canvas.

| Operation | Draws |
| --- | --- |
| `Op::Fill((rect, color))` | A filled rectangle. |
| `Op::Bitmap(bitmap)` | A one-color image.  `bits` holds one bit per pixel, in rows from the top.  Each row is padded to a whole number of bytes, with the leftmost pixel in the high bit, so `bits.len()` must equal `width.div_ceil(8) * height`.  Each set bit is painted in `color` as a square `scale` pixels on a side (1 to 8), and clear bits are transparent.  `x` and `y` position the top-left corner, and `width` and `height` (1 to 256 each) are the image's dimensions before scaling. |
| `Op::Text(text)` | A single line of text, at most 64 bytes, in one color and in one of the skin's fonts (the same fonts as [`ui::set_font`](ui.md#set_font)).  The text is positioned so that the pixels that the glyphs actually paint are centered in `box_`, both horizontally and vertically, which puts a digit in the middle of a square.  Drawing is clipped to `box_`. |

## `theme_color`

```rust
pub fn theme_color(role: Role) -> Color
```

Returns one of the current skin's colors.

| `Role` | Color |
| --- | --- |
| `Background`, `Foreground` | The window's background and text colors. |
| `Highlight`, `HighlightText` | The background and text colors of a list's selected row. |
| `TitleBackground`, `TitleText` | The background and text colors of a title. |
| `Secondary` | Secondary text. |
| `Border` | Separators and borders. |

On a color model, these are the colors of the user's chosen theme, and they change while the app is running if the user switches themes.  The app on screen then receives [`Event::Foreground`](app-events.md#foreground) again.

## `text_size`

```rust
pub fn text_size(font: Font, text: &str) -> Size
```

Returns the width and height of the pixels that the glyphs of `text` paint in one of the skin's fonts, without any line spacing.  `Op::Text` centers text by the same measurement.  Only the first 64 bytes of `text` are measured.

## `create`

```rust
pub fn create(parent: Widget, bounds: Rect) -> Widget
```

The canvas is empty until the first `draw`.  `parent` and `bounds` work the same way as in [`ui::create`](ui.md#create), including the result of 0 for a parent in an idle window that the app doesn't have.

## `draw`

```rust
pub fn draw(w: Widget, ops: &[Op]) -> Result<(), String>
```

Replaces the contents of the canvas.  An empty list clears the canvas.

`draw` rejects the entire list, and leaves the canvas unchanged, in any of these cases:

- The list has more than 512 operations.
- A `Text` is longer than 64 bytes.
- A `Bitmap` has a `width` or `height` of 0 or more than 256, or a `scale` outside 1 to 8.
- A `Bitmap` has a `bits.len()` other than `width.div_ceil(8) * height`.
- The `bits` of all bitmaps in the list total more than 64 KiB.

The `Err` value is a message that names the limit.

`draw` checks the list before the widget.  Given a valid list, `draw` returns `Err("not a canvas")` and changes nothing when the handle doesn't exist, is the root, or refers to another kind of widget.
