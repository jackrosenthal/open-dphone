# `ui`

Widgets, softkeys, keys, pictures, and the window.  Capability `ui`.

An app builds a screen as a tree of widgets inside the app's window.  The phone draws the widgets, handles the keys that widgets understand (moving a list's selection, typing into an input), and tells the app what the user did with [`Event::Widget`](app-events.md#widget), [`Event::Softkey`](app-events.md#softkey), and [`Event::Key`](app-events.md#key).

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

## The window

Handle 0 (`ROOT`) is the app's window: the area of the display above the softkey bar (see [`Metrics`](types.md#metrics-and-skin) for the sizes).  The phone paints the window in the skin's background color and then the widgets, parents before children and siblings in the order of creation.  Each widget is clipped to the widget's bounds and to the bounds of every ancestor.

The phone holds back what an app changes in a window while `start` or `handle` runs and shows the changes when the function returns, so the changes of one handler normally appear together, in one repaint.  The window is not locked meanwhile: when the phone repaints the window for another reason during a handler (the window comes on screen, the skin's palette changes), that repaint shows the widgets as the handler has left them so far.  Changes made while the window is off screen are kept and show when the window returns.

A function that takes a widget and is given a handle that does not exist, or `ROOT` where a real widget is needed, does nothing.  A function given a widget of a kind that has no use for the value stores the value without visible effect; the page notes the kinds each function serves.

## Widget kinds

| `WidgetKind` | Shows | Keys handled when focused |
| --- | --- | --- |
| `Group` | Nothing; holds child widgets. | None. |
| `Text` | One line of text, or wrapped lines after `set_wrap(w, true)`. | None. |
| `TextList` | Rows, one of them selected; scrolls to keep the selection in view and shows a scroll bar when the rows do not fit. | `up` and `down` move the selection and wrap around at the ends; `select` activates the selected row. |
| `Scroll` | Child widgets, scrolled vertically, with a scroll bar.  The content's height is the lowest bottom edge of the visible children. | `up` and `down` scroll by one `line_height`.  When a descendant has the focus, the widget scrolls to keep that descendant in view. |
| `Input` | A one-line text field with a cursor. | Digits, `*`, and `#` type; `left` and `right` move the cursor; `select` submits; `up` and `down` move the focus to the previous or next visible `Input` or `TextList` of the window. |
| `Image` | A picture, centered in the bounds at the picture's size. | None. |

The [`canvas`](canvas.md) interface adds a seventh kind, created with `canvas::create`.

## Focus and keys

One widget of the window, or none, has the focus ([`set_focus`](#set_focus)).  For each key press while the window is on screen, the phone goes through these steps and stops at the first that applies:

1. A notice of the phone on screen takes `select`, `cancel`, and the keys under the display.
2. Line key 1 takes the app off the screen.  The volume keys, `mute`, `spkr`, and `hdst` go to the phone.
3. One of the four keys under the display triggers the softkey shown above the key (not in a [full-screen](#set_full_screen) window).
4. The focused widget, then each ancestor, then the window: the first that claims the key ([`claim_keys`](#claim_keys)) gets [`Event::Key`](app-events.md#key).
5. The focused widget handles the key as the table above says.  When no widget has the focus and the window has exactly one visible `TextList` or `Input`, that widget handles the key.
6. An unclaimed `cancel` takes the app off the screen.
7. Any other key goes to the phone, which handles the key as on any other screen (e.g., `redial`) or ignores the key.

[`input::capture`](input.md) replaces steps 2 to 7: every key goes to the app.

## Softkeys

Softkeys belong to widgets or to the window ([`set_softkeys`](#set_softkeys)).  The bar shows the softkeys of the focused widget, or else of the nearest ancestor that has any, or else of the window.  Up to four softkeys take the four positions in order.  With more than four, the bar shows three at a time and the label "More" in the fourth position, which pages through the softkeys and wraps around.

While an `Input` has the focus and the third position is empty, the phone shows "Delete" there; the key deletes the character before the cursor and sends `WidgetEvent::Changed`.

## The idle window

An app whose manifest sets `has_idle` has a second window, on the idle screen.  [`idle_root`](#idle_root) returns the window's root handle, and widgets created with that handle (or a descendant) as the parent belong to the idle window.  The functions of `ui` and `canvas` work on these widgets as on others.

The idle window differs from the app's window in these ways:

- The size is small and fixed ([`idle_size`](#idle_size)).
- The phone draws no background; the widgets appear over the wallpaper, and text takes the wallpaper's text color.
- The window never receives keys: softkeys, key claims, and focus have no effect, and no `Event::Widget` arrives.
- The idle screen shows one app's idle window at a time.  Windows that have at least one widget take turns; the setting `idle_cycle_seconds` of the phone's configuration gives the length of a turn (10 seconds by default; with 0, no idle window shows).  [`Event::IdleScreenShow` and `Event::IdleScreenHide`](app-events.md#idlescreenshow-and-idlescreenhide) report the turns.
- `set_full_screen`, `get_metrics`, `input::capture`, and `lamps` concern the app's window only.

## Functions

### `get_metrics`

```rust
pub fn get_metrics() -> Metrics
```

WIT name: `get-metrics`.

The skin, the size of the app's window, and the skin's text sizes; see [`Metrics`](types.md#metrics-and-skin).  `height` follows [`set_full_screen`](#set_full_screen).

### `idle_size`

```rust
pub fn idle_size() -> Size
```

The size of the area of the idle screen given to idle windows: 296 by 36 on the color models, 160 by 34 on the D70, and 114 by 30 on the D40, D45, and D50.  The function answers for every app, with or without `has_idle`.

### `idle_root`

```rust
pub fn idle_root() -> Option<Widget>
```

The root handle of the app's idle window, `Some(0x8000_0000)`, or `None` when the manifest does not set `has_idle`.

### `create`

```rust
pub fn create(kind: WidgetKind, parent: Widget, bounds: Rect) -> Widget
```

Creates a widget and returns the handle.  `parent` is `ROOT`, the idle window's root, or a widget; `bounds` is relative to the parent.  A new widget is visible, has no text, and uses `Font::Normal` and `Align::Left`.

Any kind of widget can be a parent, but only `Group` and `Scroll` draw child widgets.  When `parent` names a widget that does not exist, the new widget becomes a top-level widget of that window.  When `parent` is in the idle window and the app has none, nothing is created and the result is 0.

### `destroy`

```rust
pub fn destroy(w: Widget)
```

Destroys the widget and every descendant.  If the focus was on one of them, no widget has the focus afterward.  `destroy(ROOT)` does nothing; use `clear`.

### `clear`

```rust
pub fn clear(w: Widget)
```

Destroys every descendant of the widget and keeps the widget.  `clear(ROOT)` empties the window and also removes the window's softkeys and key claims and the focus; `clear` on the idle window's root does the same there.  Pictures defined with `define_image` stay defined.

### `set_bounds`

```rust
pub fn set_bounds(w: Widget, bounds: Rect)
```

WIT name: `set-bounds`.

Moves and resizes the widget.

### `set_visible`

```rust
pub fn set_visible(w: Widget, visible: bool)
```

Hides or shows the widget with every descendant.  A hidden `TextList` or `Input` is skipped when `up` and `down` move the focus.  Hiding the widget that has the focus, or an ancestor of that widget, leaves no widget focused, as `set_focus(ROOT)` does; showing the widget again does not return the focus.

### `set_focus`

```rust
pub fn set_focus(w: Widget)
```

Gives the widget the focus.  Any kind of widget can have the focus; a kind that handles no keys still decides which softkeys show and where the search for key claims starts.  `set_focus(ROOT)`, or a handle that does not exist, leaves no widget focused.  Changing the focus returns the softkey bar to the first page.

### `set_text`

```rust
pub fn set_text(w: Widget, text: &str)
```

Sets the text of a `Text`.  On an `Input` the call is the same as [`set_value`](#set_value).

A `Text` without wrapping draws one line, centered vertically in the bounds.  With wrapping, the lines are `line_height` apart and the block of lines is centered vertically.

### `set_align`

```rust
pub fn set_align(w: Widget, align: Align)
```

Horizontal alignment of a `Text`: `Left` (the default), `Center`, or `Right`.

### `set_font`

```rust
pub fn set_font(w: Widget, font: Font)
```

The font of a `Text` or an `Input`.  `Font::Normal` is the default; `Small` is for secondary text, `Bold` for emphasis, `Title` for a screen's title, and `Large` for large digits and names.  The faces and sizes are the skin's.  A list draws rows in `Font::Normal` always.

### `set_wrap`

```rust
pub fn set_wrap(w: Widget, wrap: bool)
```

With `true`, a `Text` breaks the text at words onto as many lines as the width requires.  Lines that do not fit the height are clipped.

### `set_highlight`

```rust
pub fn set_highlight(w: Widget, on: bool)
```

WIT name: `set-highlight`.

Draws the widget in the skin's selection colors, as a list draws the selected row: the bounds are filled with the selection background, and text takes the selection text color.  `false` returns the widget to the defaults.  The call serves `Text`, `Group`, `Scroll`, and `Image` (e.g., one cell of a grid of texts); an `Input` ignores the colors.

### `set_items`

```rust
pub fn set_items(w: Widget, items: &[ListItem])
```

Replaces the rows of a `TextList`.  Each `ListItem` gives a row's `text`, a `detail` shown right-aligned at the right edge, and an `icon` shown at the left; `detail` and `icon` may be empty.

- The list has an icon column, one `row_height` wide, when any item has an icon.
- The list has a detail column, 40% of the width, when any item has a detail.
- `icon` names a picture defined with [`define_image`](#define_image), which is reduced to fit a square of `row_height`, or else one of [the phone's icons](#the-phones-icons).  An unknown name draws nothing.
- The selection stays on the same row number, or moves to the last row when the list became shorter.  No `WidgetEvent::Highlighted` is sent.

### `set_selected`

```rust
pub fn set_selected(w: Widget, index: u32)
```

WIT name: `set-selected`.

Selects a row of a `TextList`, counted from 0.  An index past the end selects the last row.  No event is sent.

### `selected`

```rust
pub fn selected(w: Widget) -> Option<u32>
```

The selected row of a `TextList`.  The result is `None` for a list without rows, for a widget of another kind, and for a handle that does not exist.

### `set_value`

```rust
pub fn set_value(w: Widget, value: &str)
```

WIT name: `set-value`.

Sets the text of an `Input` and puts the cursor at the end.  The value is stored as given: neither the input mode nor the maximum length applies.  No `WidgetEvent::Changed` is sent.  On a `Text` the call is the same as `set_text`.

### `value`

```rust
pub fn value(w: Widget) -> String
```

The text of an `Input` (or of a `Text`), or `""` for a handle that does not exist.  In the modes `Password` and `Pin` the result is the real text, not the dots on screen.

### `set_input_mode`

```rust
pub fn set_input_mode(w: Widget, mode: InputMode)
```

WIT name: `set-input-mode`.

How the dial pad types into an `Input`:

| `InputMode` | Typing | Display |
| --- | --- | --- |
| `Text` (default) | Multi-tap: pressing a key again within one second cycles through the key's characters (`2` gives `a b c 2 A B C`; `1` gives punctuation; `0` gives a space and `0`). | As typed. |
| `Numeric` | Digits; `*` types `.`; `#` types nothing. | As typed. |
| `Dial` | Digits, `*`, and `#`. | As typed. |
| `Password` | As `Text`. | Dots, except the character being cycled. |
| `Pin` | Digits only. | Dots. |

Changing the mode does not change text that is already there.

### `set_max_length`

```rust
pub fn set_max_length(w: Widget, max: u32)
```

WIT name: `set-max-length`.

The most characters the user can type into an `Input`.  The default is 255, and values above 65,535 count as 65,535.  At the limit, further keys are ignored.  Text that is already longer stays.

### `define_image`

```rust
pub fn define_image(name: &str, png: &[u8]) -> Result<(), Error>
```

WIT name: `define-image`.

Gives a picture a name for `set_image` and for the `icon` of list items.  `png` is the content of a PNG file, which an app usually holds with `include_bytes!`.  Defining a name again replaces the picture for later calls; a widget or a list that already shows the old picture keeps the old picture until `set_image` or `set_items` is called again.  The definitions last for the life of the instance.

```rust
ui::define_image("logo", include_bytes!("logo.png"))?;
let w = ui::create(WidgetKind::Image, ROOT, Rect::new(0, 0, 64, 64));
ui::set_image(w, "logo");
```

Errors:

| Error | Cause |
| --- | --- |
| `Invalid("a picture needs a plain name")` | `name` is empty, `.`, or `..`, or contains `/`, `\`, or a NUL character. |
| `Invalid("a picture may be 512 KiB")` | `png` is larger than 512 KiB. |
| `Invalid("an app may define 32 pictures")` | The app already has 32 names and `name` is a new one. |
| `Invalid("not a PNG picture the phone can show (at most 1,048,576 pixels)")` | `png` is not the content of a PNG file, the file is damaged, or the picture has more than 1,048,576 pixels. |

The phone decodes the picture in `define_image`, so `Ok` means that the picture can be shown.  A refused definition changes nothing: a picture defined earlier under the same name stays, and a name without a picture still means the phone's icon of that name, or nothing.  A picture larger than the window is reduced to fit the window, keeping the shape; pictures are never enlarged.  The gray skins show a picture in four levels by brightness.

### `set_image`

```rust
pub fn set_image(w: Widget, name: &str)
```

WIT name: `set-image`.

Sets what an `Image` shows: the picture the app defined as `name`, or else the phone's icon of that name.  An unknown name shows nothing.  A name that `define_image` would refuse is ignored.  One-color icons of the phone take the widget's text color.

### `claim_keys`

```rust
pub fn claim_keys(w: Widget, keys: &[String])
```

WIT name: `claim-keys`.

Sets the keys that the widget, or the window for `ROOT`, takes as [`Event::Key`](app-events.md#key) instead of the built-in behavior.  The list replaces the widget's earlier claims; an empty list removes them.  The names are the key names of [`Event::Key`](app-events.md#key), and a name the phone does not know is dropped from the list.

A claim applies when the claiming widget is the focused widget or an ancestor of the focused widget; claims of the window always apply.  Claiming `cancel` keeps the key from taking the app off the screen, and the app then decides between "back" and leaving:

```rust
ui::claim_keys(ROOT, &["cancel".to_string()]);
// later, in handle:
// Event::Key(k) if k == "cancel" => runtime::background(),
```

Line key 1, the volume keys, `mute`, `spkr`, and `hdst` cannot be claimed (see [`input::capture`](input.md)), and the four keys under the display can be claimed only in a full-screen window.

### `set_full_screen`

```rust
pub fn set_full_screen(on: bool)
```

WIT name: `set-full-screen`.

Gives the app's window the whole display (`true`), or the usual area above the softkey bar again (`false`).

- A full-screen window has no softkey bar.  Softkeys are not shown, and the four keys under the display do nothing unless claimed as `sk1` to `sk4`.
- `get_metrics` reports the new height (272, 160, or 120).  Widgets keep their bounds; the app lays the screen out again.
- An unclaimed `cancel` and line key 1 still take the app off the screen, and a call or a notice of the phone covers a full-screen window as any other.

### `set_softkeys`

```rust
pub fn set_softkeys(w: Widget, keys: &[Softkey])
```

WIT name: `set-softkeys`.

Sets the softkeys of a widget, or of the window for `ROOT`.  Each `Softkey` has a `label` and an `id` that the app chooses and that [`Event::Softkey`](app-events.md#softkey) reports.  An empty list removes a widget's softkeys, so that the bar shows an ancestor's or the window's.  Every call returns the bar to the first page.  See [Softkeys](#softkeys) for which set shows.

## The phone's icons

`set_image` and list items can name the phone's icons.  The one-color icons take the text color:

`alarm`, `apps`, `backspace`, `brightness`, `calllog`, `clock`, `conference`, `contacts`, `desk`, `dnd`, `down`, `error`, `forward`, `handset`, `headset`, `hold`, `info`, `left`, `lock`, `more`, `mute`, `net-down`, `network`, `ok`, `page`, `park`, `phone`, `phone-in`, `phone-missed`, `phone-out`, `phone-unregistered`, `presence`, `registered`, `right`, `ring`, `settings`, `speaker`, `transfer`, `unregistered`, `up`, `update`, `voicemail`, and `warning`.

The app icons are in color on the color models and are named with the prefix `app-`: `app-alarms`, `app-app`, `app-boxpusher`, `app-calendar`, `app-calllog`, `app-contacts`, `app-desksparrow`, `app-diagnostics`, `app-forward`, `app-hotdesking`, `app-info`, `app-lightup`, `app-network`, `app-paging`, `app-parking`, `app-queues`, `app-settings`, `app-status`, `app-update`, and `app-voicemail`.
