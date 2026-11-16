# `ui`

Widgets, softkeys, key handling, images, and the app's window.

An app builds each screen as a tree of widgets inside the app's window.  The phone draws the widgets and handles the keys that widgets understand, such as moving a list's selection or typing into an input.  The phone reports what the user did through [`Event::Widget`](app-events.md#widget), [`Event::Softkey`](app-events.md#softkey), and [`Event::Key`](app-events.md#key).

```rust
use dphone_app_sdk::prelude::*;

fn show_menu() -> Widget {
    let area = screen("Menu");                       // clears the window, draws a title
    let list = ui::create(WidgetKind::TextList, ROOT, area);
    ui::set_items(list, &[ListItem::new("First"), ListItem::with_detail("Second", "2")]);
    ui::set_focus(list);
    ui::set_softkeys(ROOT, &[Softkey::new(1, "Select"), Softkey::new(2, "Exit")]);
    list
}
```

## Window

Handle 0 (`ROOT`) is the app's window, which covers the display above the softkey bar.  ([`Metrics`](types.md#metrics-and-skin) lists the dimensions.)  The phone fills the window with the skin's background color and then paints the widgets: parents before children, and siblings in creation order.  Each widget is clipped to the widget's bounds and to the bounds of every ancestor.

While `start` or `handle` runs, the phone holds back the app's changes to a window and shows them when the function returns.  The changes from one handler therefore normally appear together, in a single repaint.  The window isn't locked in the meantime, though.  If the phone repaints the window for another reason during a handler (the window comes on screen, or the user switches themes), that repaint shows the widgets in whatever state the handler has left them so far.  Changes made while the window is off screen are kept, and appear when the window returns.

Functions that take a widget do nothing when given a handle that doesn't exist, or when given `ROOT` where an actual widget is required.  Calling a function on a kind of widget that doesn't use the value stores the value with no visible effect.  Each function below says which kinds apply.

## Widget kinds

| `WidgetKind` | Shows | Keys handled when focused |
| --- | --- | --- |
| `Group` | Nothing.  A group holds child widgets. | None. |
| `Text` | One line of text, or wrapped lines after `set_wrap(w, true)`. | None. |
| `TextList` | Rows, one of which is selected.  The list scrolls to keep the selection visible and shows a scroll bar when the rows don't all fit. | `up` and `down` move the selection, wrapping around at the ends.  `select` activates the selected row. |
| `Scroll` | Child widgets, scrolled vertically, with a scroll bar.  The height of the content is the lowest bottom edge among the visible children. | `up` and `down` scroll by one `line_height`.  When a descendant has the focus, the widget scrolls to keep that descendant visible. |
| `Input` | A one-line text field with a cursor. | Digits, `*`, and `#` type.  `left` and `right` move the cursor.  `select` submits.  `up` and `down` move the focus to the previous or next visible `Input` or `TextList` in the window. |
| `Image` | An image, centered in the bounds at the image's natural size. | None. |

The [`canvas`](canvas.md) interface adds a seventh kind, created with `canvas::create`.

## Focus and keys

At most one widget in the window has the focus, which [`set_focus`](#set_focus) assigns.  For each key press while the window is on screen, the phone goes through the following steps and stops at the first one that applies:

1. If the phone is showing a notice, the notice takes `select`, `cancel`, and the keys below the display.
2. Line key 1 takes the app off the screen.  The volume keys, `mute`, `spkr`, and `hdst` go to the phone.
3. One of the four keys below the display triggers the softkey shown above that key.  This step is skipped in a [full-screen](#set_full_screen) window.
4. The focused widget, then each ancestor, then the window is checked for a claim on the key ([`claim_keys`](#claim_keys)).  The first claim found produces [`Event::Key`](app-events.md#key).
5. The focused widget handles the key as described under [Widget kinds](#widget-kinds).  If no widget has the focus and the window has exactly one visible `TextList` or `Input`, that widget handles the key.
6. An unclaimed `cancel` takes the app off the screen.
7. Any other key goes to the phone, which handles the key the same way as on any other screen (`redial`, for example) or ignores the key.

[`input::capture`](input.md) replaces steps 2 through 7: every key goes to the app.

## Softkeys

Softkeys belong to a widget or to the window, and are set with [`set_softkeys`](#set_softkeys).  The softkey bar shows the softkeys of the focused widget.  If that widget has none, the bar shows those of the nearest ancestor that has any, and otherwise those of the window.

Up to four softkeys fill the four positions in order.  With more than four, the bar shows three at a time, plus a "More" label in the fourth position that pages through the softkeys and wraps around.

While an `Input` has the focus and the third position is empty, the phone shows "Delete" there.  That key deletes the character before the cursor and sends `WidgetEvent::Changed`.

## Idle window

An app whose manifest sets `has_idle` has a second window, on the idle screen.  [`idle_root`](#idle_root) returns the root handle of that window, and any widget created with that handle (or a descendant) as the parent belongs to the idle window.  The `ui` and `canvas` functions work on these widgets the same way as on any others.

The idle window differs from the app's main window in several ways:

- The window is small, and the size is fixed ([`idle_size`](#idle_size)).
- The phone draws no background.  Widgets appear over the wallpaper of the user's theme, and text takes the theme's color for text on the wallpaper.
- The window never receives keys.  Softkeys, key claims, and focus have no effect, and no `Event::Widget` arrives.
- The idle screen shows one app's idle window at a time.  Windows that have at least one widget take turns, and the `idle_cycle_seconds` setting in the phone's configuration sets the length of a turn (10 seconds by default; 0 hides all idle windows).  [`Event::IdleScreenShow` and `Event::IdleScreenHide`](app-events.md#idlescreenshow-and-idlescreenhide) report the turns.
- `set_full_screen`, `get_metrics`, `input::capture`, and `lamps` apply only to the main window.

## Functions

### `get_metrics`

```rust
pub fn get_metrics() -> Metrics
```

Returns the skin, the dimensions of the app's window, and the skin's text sizes.  See [`Metrics`](types.md#metrics-and-skin) for the fields.  `height` reflects [`set_full_screen`](#set_full_screen).

### `idle_size`

```rust
pub fn idle_size() -> Size
```

Returns the size of the idle screen area that idle windows use: 296 by 36 on the color models, 160 by 34 on the D70, and 114 by 30 on the D40, D45, and D50.  The function works for every app, with or without `has_idle`.

### `idle_root`

```rust
pub fn idle_root() -> Option<Widget>
```

Returns the root handle of the app's idle window, `Some(0x8000_0000)`, or `None` if the manifest doesn't set `has_idle`.

### `create`

```rust
pub fn create(kind: WidgetKind, parent: Widget, bounds: Rect) -> Widget
```

Creates a widget and returns the handle.  `parent` is `ROOT`, the idle window's root, or another widget, and `bounds` is relative to the parent.  A new widget is visible, has no text, and uses `Font::Normal` and `Align::Left`.

Any kind of widget can be a parent, but only `Group` and `Scroll` draw child widgets.  If `parent` doesn't exist, the new widget becomes a top-level widget of the window that the handle would belong to.  If `parent` is in the idle window and the app has no idle window, `create` creates nothing and returns 0.

### `destroy`

```rust
pub fn destroy(w: Widget)
```

Destroys the widget and all descendants.  If one of them had the focus, no widget has the focus afterward.  `destroy(ROOT)` does nothing; use `clear` instead.

### `clear`

```rust
pub fn clear(w: Widget)
```

Destroys all descendants of the widget but keeps the widget.  `clear(ROOT)` empties the window and also removes the window's softkeys, key claims, and focus.  Calling `clear` on the idle window's root does the same for the idle window.  Images defined with `define_image` remain defined.

### `set_bounds`

```rust
pub fn set_bounds(w: Widget, bounds: Rect)
```

Moves and resizes the widget.

### `set_visible`

```rust
pub fn set_visible(w: Widget, visible: bool)
```

Shows or hides the widget and all descendants.  A hidden `TextList` or `Input` is skipped when `up` and `down` move the focus.  Hiding the focused widget, or an ancestor of the focused widget, leaves no widget focused, the same as `set_focus(ROOT)`.  Showing the widget again doesn't restore the focus.

### `set_focus`

```rust
pub fn set_focus(w: Widget)
```

Gives the widget the focus.  Any kind of widget can have the focus.  Even a kind that handles no keys determines which softkeys are shown and where the search for key claims starts.  Passing `ROOT`, or a handle that doesn't exist, leaves no widget focused.  Changing the focus resets the softkey bar to the first page.

### `set_text`

```rust
pub fn set_text(w: Widget, text: &str)
```

Sets the text of a `Text`.  On an `Input`, `set_text` is the same as [`set_value`](#set_value).

Without wrapping, a `Text` draws a single line, centered vertically in the bounds.  With wrapping, lines are `line_height` apart, and the block of lines is centered vertically.

### `set_align`

```rust
pub fn set_align(w: Widget, align: Align)
```

Sets the horizontal alignment of a `Text`: `Left` (the default), `Center`, or `Right`.

### `set_font`

```rust
pub fn set_font(w: Widget, font: Font)
```

Sets the font of a `Text` or an `Input`.  `Font::Normal` is the default.  Use `Small` for secondary text, `Bold` for emphasis, `Title` for a screen title, and `Large` for large digits and names.  The skin determines the typefaces and sizes.  Lists always draw rows in `Font::Normal`.

### `set_wrap`

```rust
pub fn set_wrap(w: Widget, wrap: bool)
```

Turns word wrapping on or off for a `Text`.  With wrapping on, the text breaks between words onto as many lines as the width requires, and lines that don't fit the height are clipped.

### `set_highlight`

```rust
pub fn set_highlight(w: Widget, on: bool)
```

Draws the widget in the skin's selection colors, the way a list draws the selected row: the bounds are filled with the selection background, and text takes the selection text color.  Pass `false` to restore the default colors.  `set_highlight` applies to `Text`, `Group`, `Scroll`, and `Image` (to highlight one cell in a grid of texts, for example).  An `Input` ignores the colors.

### `set_items`

```rust
pub fn set_items(w: Widget, items: &[ListItem])
```

Replaces the rows of a `TextList`.  Each `ListItem` has the row's `text`, a `detail` shown right-aligned at the right edge, and an `icon` shown at the left.  `detail` and `icon` may be empty.

- If any item has an icon, the list has an icon column one `row_height` wide.
- If any item has a detail, the list has a detail column that takes 40% of the width.
- `icon` names an image defined with [`define_image`](#define_image), which is scaled down to fit a square of `row_height`, or else one of the [built-in icons](#built-in-icons).  An unknown name draws nothing.
- The selection stays on the same row number, or moves to the last row if the list got shorter.  No `WidgetEvent::Highlighted` is sent.

### `set_selected`

```rust
pub fn set_selected(w: Widget, index: u32)
```

Selects a row of a `TextList`.  Rows are numbered from 0, and an index past the end selects the last row.  No event is sent.

### `selected`

```rust
pub fn selected(w: Widget) -> Option<u32>
```

Returns the selected row of a `TextList`.  Returns `None` for a list with no rows, for a widget of another kind, and for a handle that doesn't exist.

### `set_value`

```rust
pub fn set_value(w: Widget, value: &str)
```

Sets the text of an `Input` and moves the cursor to the end.  The value is stored as given: neither the input mode nor the maximum length applies.  No `WidgetEvent::Changed` is sent.  On a `Text`, `set_value` is the same as `set_text`.

### `value`

```rust
pub fn value(w: Widget) -> String
```

Returns the text of an `Input` or a `Text`, or `""` for a handle that doesn't exist.  In the `Password` and `Pin` modes, the result is the actual text, not the dots shown on screen.

### `set_input_mode`

```rust
pub fn set_input_mode(w: Widget, mode: InputMode)
```

Sets how the dial pad types into an `Input`:

| `InputMode` | Typing | Display |
| --- | --- | --- |
| `Text` (default) | Multi-tap: pressing a key again within one second cycles through that key's characters.  `2` cycles through `a b c 2 A B C`, `1` through punctuation, and `0` through a space and `0`. | As typed. |
| `Numeric` | Digits.  `*` types `.`, and `#` types nothing. | As typed. |
| `Dial` | Digits, `*`, and `#`. | As typed. |
| `Password` | Same as `Text`. | Dots, except for the character being cycled. |
| `Pin` | Digits only. | Dots. |

Changing the mode doesn't change existing text.

### `set_max_length`

```rust
pub fn set_max_length(w: Widget, max: u32)
```

Sets the maximum number of characters that the user can type into an `Input`.  The default is 255, and values above 65,535 are treated as 65,535.  At the limit, further key presses are ignored.  Existing text that is already longer is kept.

### `define_image`

```rust
pub fn define_image(name: &str, png: &[u8]) -> Result<(), Error>
```

Defines a named image for use with `set_image` and as the `icon` of list items.  `png` is the contents of a PNG file, which an app usually embeds with `include_bytes!`.

```rust
ui::define_image("logo", include_bytes!("logo.png"))?;
let w = ui::create(WidgetKind::Image, ROOT, Rect::new(0, 0, 64, 64));
ui::set_image(w, "logo");
```

Defining a name again replaces the image for later calls.  A widget or list that already shows the old image keeps showing the old image until the next call to `set_image` or `set_items`.  Definitions last for the life of the instance.

The phone decodes the image during the call, so `Ok` means that the image can be displayed.  An image larger than the window is scaled down to fit, preserving the aspect ratio.  Images are never scaled up.  The grayscale skins render an image in four shades, by brightness.

| Error | Cause |
| --- | --- |
| `Invalid("a picture needs a plain name")` | `name` is empty, is `.` or `..`, or contains `/`, `\`, or a NUL character. |
| `Invalid("a picture may be 512 KiB")` | `png` is larger than 512 KiB. |
| `Invalid("an app may define 32 pictures")` | The app already has 32 names, and `name` is a new one. |
| `Invalid("not a PNG picture the phone can show (at most 1,048,576 pixels)")` | `png` is not the contents of a PNG file, the file is corrupt, or the image has more than 1,048,576 pixels. |

A failed call changes nothing.  An image defined earlier under the same name remains, and a name with no image still refers to the built-in icon of that name, if there is one.

### `set_image`

```rust
pub fn set_image(w: Widget, name: &str)
```

Sets what an `Image` shows: the image that the app defined as `name`, or else the built-in icon with that name.  An unknown name shows nothing, and a name that `define_image` would reject is ignored.  One-color built-in icons take the widget's text color.

### `claim_keys`

```rust
pub fn claim_keys(w: Widget, keys: &[String])
```

Sets the keys that the widget, or the window for `ROOT`, receives as [`Event::Key`](app-events.md#key) instead of the built-in behavior.  The list replaces the widget's earlier claims, and an empty list removes them.  The names are the [key names](app-events.md#key) used by `Event::Key`.  Names that the phone doesn't recognize are dropped from the list.

A widget's claim applies while that widget, or a descendant, has the focus.  The window's claims always apply.  Claiming `cancel` keeps that key from taking the app off the screen, so the app can decide between going back and leaving:

```rust
ui::claim_keys(ROOT, &["cancel".to_string()]);
// later, in handle:
// Event::Key(k) if k == "cancel" => runtime::background(),
```

Line key 1, the volume keys, `mute`, `spkr`, and `hdst` can't be claimed (see [`input::capture`](input.md)), and the four keys below the display can be claimed only in a full-screen window.

### `set_full_screen`

```rust
pub fn set_full_screen(on: bool)
```

Expands the app's window to the whole display (`true`), or restores the usual area above the softkey bar (`false`).

- A full-screen window has no softkey bar.  Softkeys aren't shown, and the four keys below the display do nothing unless claimed as `sk1` through `sk4`.
- `get_metrics` reports the new height (272, 160, or 120).  Widgets keep their bounds, so the app needs to lay out the screen again.
- An unclaimed `cancel` and line key 1 still take the app off the screen, and a call or a notice from the phone covers a full-screen window like any other.

### `set_softkeys`

```rust
pub fn set_softkeys(w: Widget, keys: &[Softkey])
```

Sets the softkeys of a widget, or of the window for `ROOT`.  Each `Softkey` has a `label` and an `id` that the app chooses, and [`Event::Softkey`](app-events.md#softkey) reports the `id`.  An empty list removes a widget's softkeys, so that the bar shows those of an ancestor or of the window.  Every call resets the bar to the first page.  [Softkeys](#softkeys) explains which set the bar shows.

## Built-in icons

`set_image` and list items can refer to the phone's built-in icons by name.  The one-color icons take the text color:

`alarm`, `apps`, `backspace`, `brightness`, `calllog`, `clock`, `conference`, `contacts`, `desk`, `dnd`, `down`, `error`, `forward`, `handset`, `headset`, `hold`, `info`, `left`, `lock`, `more`, `mute`, `net-down`, `network`, `ok`, `page`, `park`, `phone`, `phone-in`, `phone-missed`, `phone-out`, `phone-unregistered`, `presence`, `registered`, `right`, `ring`, `settings`, `speaker`, `transfer`, `unregistered`, `up`, `update`, `voicemail`, and `warning`.

The app icons are in color on the color models, and have names that start with `app-`: `app-alarms`, `app-app`, `app-boxpusher`, `app-calendar`, `app-calllog`, `app-contacts`, `app-desksparrow`, `app-diagnostics`, `app-forward`, `app-hotdesking`, `app-info`, `app-lightup`, `app-network`, `app-paging`, `app-parking`, `app-queues`, `app-settings`, `app-status`, `app-update`, and `app-voicemail`.
