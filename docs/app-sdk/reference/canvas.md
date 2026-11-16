# `canvas`

Free drawing, for what the widgets cannot show (a game board, a chart).  Capability `canvas`.

A canvas is a widget that shows a list of drawing operations.  [`draw`](#draw) replaces the whole list in one call, so the phone never paints half a picture, and the phone repaints from the list whenever the screen needs painting.  The functions of [`ui`](ui.md) place, hide, and destroy a canvas as any widget.  An app animates by calling `draw` from a repeating [timer](timers.md), which fires at most 20 times a second.

```rust
use dphone_app_sdk::canvas::{self, Color, Op, Role, Text};
use dphone_app_sdk::{Font, Rect, ROOT};

let board = canvas::create(ROOT, Rect::new(0, 0, 120, 120));
let ink = canvas::theme_color(Role::Foreground);
canvas::draw(board, &[
    Op::Fill((Rect::new(10, 10, 100, 100), Color { r: 0, g: 96, b: 160 })),
    Op::Text(Text {
        box_: Rect::new(10, 10, 100, 100),
        font: Font::Large,
        color: ink,
        text: "7".into(),
    }),
])?;
```

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

`Color` is red, green, and blue.  The gray skins show the nearest of four levels (black, dark gray, light gray, white) by brightness.

The operations are painted in the order of the list, later ones over earlier ones.  Coordinates are relative to the top-left corner of the canvas, and nothing is drawn outside the canvas.

| Operation | Draws |
| --- | --- |
| `Op::Fill((rect, color))` | A filled rectangle. |
| `Op::Bitmap(bitmap)` | A picture of one color.  `bits` holds one bit per pixel, rows from the top, each row padded to whole bytes, the leftmost pixel in the highest bit; `bits.len()` must equal `width.div_ceil(8) * height`.  Each set bit is painted as a square with the side `scale` (1 to 8) in `color`; clear bits leave what is underneath.  `x` and `y` place the top-left corner, and `width` and `height` (1 to 256 each) count the picture's pixels before scaling. |
| `Op::Text(text)` | One line of text in a font of the skin (the fonts of [`ui::set_font`](ui.md#set_font)), in one color, at most 64 bytes.  The text is placed so that the pixels the glyphs paint are centered in `box_`, horizontally and vertically (a digit sits in the middle of a square), and nothing is drawn outside `box_`.  The field is `box` in `app.wit`. |

## `theme_color`

```rust
pub fn theme_color(role: Role) -> Color
```

WIT name: `theme-color`.

A color of the skin in use, so that a drawing matches the widgets around the canvas.

| `Role` | Color |
| --- | --- |
| `Background`, `Foreground` | The window's background and text. |
| `Highlight`, `HighlightText` | The background and text of a list's selected row. |
| `TitleBackground`, `TitleText` | The background and text of a title. |
| `Secondary` | Secondary text. |
| `Border` | Separators and borders. |

On a color model the colors follow the wallpaper the user chose and can change while the app runs.  The app on screen then receives [`Event::Foreground`](app-events.md#foreground) again; an app that reads the colors and draws at every `Foreground` stays in step.

## `text_size`

```rust
pub fn text_size(font: Font, text: &str) -> Size
```

WIT name: `text-size`.

The width and height of the pixels that the glyphs of `text` paint in a font of the skin, without the space around a line; the same measure that `Op::Text` centers.  An app uses the result to choose a font that fits a box.  Only the first 64 bytes of `text` are measured.

## `create`

```rust
pub fn create(parent: Widget, bounds: Rect) -> Widget
```

Creates a canvas widget that shows nothing until the first `draw`.  `parent` and `bounds` work as in [`ui::create`](ui.md#create), including the result 0 for a parent in an idle window the app does not have.

## `draw`

```rust
pub fn draw(w: Widget, ops: &[Op]) -> Result<(), String>
```

Replaces what the canvas shows.  A list over a limit is refused whole, and the canvas keeps what was shown:

- The list is longer than 512 operations.
- A `Text` is longer than 64 bytes.
- A `Bitmap` has a `width` or `height` of 0 or above 256, or a `scale` outside 1 to 8.
- The `bits.len()` of a `Bitmap` is not `width.div_ceil(8) * height`.
- The `bits` of all bitmaps of the list together exceed 64 KiB.

The `Err` value is a sentence that names the limit.

The list is checked before the widget.  `draw` on a handle that does not exist, on the root, or on a widget of another kind returns `Err("not a canvas")` and changes nothing.  An empty list clears the canvas.
