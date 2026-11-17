# SDK helpers

Helpers run inside the app and call the functions on the other pages, so using a helper gives the app the capability of each interface that the helper calls.

## Prelude

`use dphone_app_sdk::prelude::*;` imports:

- The trait `App`, the constant `ROOT`, and the types `Event`, `Widget`, `Rect`, `ListItem`, `Softkey`, `WidgetKind`, `WidgetEvent`, `Skin`, `PbxOutcome`, `PbxScope`, `PbxEntry`, `PbxStorageOutcome`, `TimeStyle`, `AppKind`, `Json`, `Form`, and `Message`.
- The functions `screen`, `content_rect`, and `row_height`.
- The modules `audio`, `calls`, `config`, `events`, `http`, `l10n`, `pbx`, `pbx_storage`, `presence`, `runtime`, `storage`, `timers`, and `ui`.

## Screens

### `screen`

```rust
pub fn screen(title: &str) -> Rect
```

Starts a new screen and returns the rectangle below the title.  `screen` clears the window with `ui::clear(ROOT)`, which also removes the window's softkeys and key claims.  The function then claims `cancel` for the window and creates a `Text` across the top of the window, `title_height` tall, in `Font::Title`.

Because the window claims `cancel`, that key no longer takes the app off the screen.  The app receives `Event::Key("cancel")` instead, and should call `runtime::background` or go back one screen.

### `content_rect`

```rust
pub fn content_rect() -> Rect
```

Returns the whole window as a rectangle at the origin: `Rect::new(0, 0, metrics.width, metrics.height)`.

### `row_height`

```rust
pub fn row_height() -> u16
```

Returns `ui::get_metrics().row_height`.

### `Message`

```rust
pub struct Message { pub text: Widget }

impl Message {
    pub fn show(title: &str, text: &str, keys: &[Softkey]) -> Message;
}
```

A screen with a title and one block of wrapped text.  `show` calls `screen`, creates the text widget, sets `keys` as the window's softkeys, and focuses the text.

### `Form`

```rust
pub struct Form { pub inputs: Vec<Widget> }

impl Form {
    pub fn show(title: &str, fields: &[(&str, InputMode, &str)], keys: &[Softkey]) -> Form;
    pub fn value(&self, index: usize) -> String;
    pub fn focus_next(&self, current: Widget) -> bool;
}
```

A screen with a title and one labeled input for each entry in `fields`.  Each entry gives the label, the input mode, and the initial value.

- `show` sets `keys` as the window's softkeys and focuses the first input.
- `value(index)` returns the text of an input, or `""` if `index` is out of range.
- `focus_next(current)` moves the focus to the input after `current`.  The function returns `false` when `current` is the last input or isn't part of the form.  Call `focus_next` on `WidgetEvent::Submitted` to either advance to the next field or, on `false`, submit the form.

## Logging and storage

```rust
pub fn log_error(msg: &str)
pub fn log_warn(msg: &str)
pub fn log_info(msg: &str)
pub fn log_debug(msg: &str)
pub fn read_string(key: &str) -> Option<String>
```

The four log functions call [`runtime::log`](runtime.md#log) with levels 0 through 3.

`read_string` calls [`storage::get`](storage.md#get) and returns the value as a `String`.  A value that isn't valid UTF-8 returns `None`.

## PBX requests

### `pbx_request`

```rust
pub fn pbx_request(method: &str, parameters: Json) -> u32
```

Calls [`pbx::request`](pbx.md#request) with `parameters` serialized as JSON.  If the parameters have no `account_id` member and [`config::app_setting("account_id")`](config.md#app_setting) is not empty, the helper adds that member.  If `parameters` is not an object, the helper sends an empty object.

### `pbx_error_text`

```rust
pub fn pbx_error_text(outcome: &PbxOutcome) -> String
```

Returns a message suitable for an error screen:

- For `Errors`, the `message` of the first error in the server's reply, or `The server reported an error.` if the reply has none.
- For `Transport`, `Could not reach the server.`, with the transport's error text on a second line.
- For `Ok`, `""`.

## `Json`

```rust
pub enum Json {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}
```

A JSON value type with a parser and a serializer, in the `json` module.  Numbers are stored as text.

| Function | Result |
| --- | --- |
| `Json::parse(text: &str) -> Option<Json>` | The parsed value, or `None` if `text` is not exactly one complete JSON value. |
| `get(&self, key: &str) -> &Json` | A member of an object.  `Null` if the member is missing or the value is not an object. |
| `path(&self, keys: &[&str]) -> &Json` | The result of applying `get` for each key in turn. |
| `text(&self) -> String` | The text of a string, number, or boolean.  `""` for anything else. |
| `as_str(&self) -> Option<&str>` | The text of a string or number. |
| `int(&self) -> Option<i64>` | The integer value of a number or numeric string, with any fraction truncated, or 0 or 1 for a boolean. |
| `truthy(&self) -> bool` | `true` for `true` and for the strings `1`, `true`, `yes`, and `on`, ignoring case. |
| `is_null(&self) -> bool` | Whether the value is `Null`. |
| `items(&self) -> Vec<&Json>` | The elements of an array.  `Null` has no elements.  Any other value is treated as a list of one, because the PBX collapses single-element lists in some replies. |
| `Json::obj([(key, value), ...]) -> Json` | An object. |
| `Json::str(s) -> Json`, `Json::num(n: i64) -> Json` | A string and a number. |
| `write(&self, out: &mut String)`, `to_string()` | The value serialized as JSON. |

## `xml`

```rust
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Element>,
}

pub fn parse(text: &str) -> Element;

impl Element {
    pub fn attr(&self, name: &str) -> &str;
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element>;
    pub fn find_all<'a>(&'a self, name: &str, out: &mut Vec<&'a Element>);
}
```

A minimal XML reader in the `xml` module, meant for the output of [`config::app_config`](config.md#app_config) and [`config::contact_lists`](config.md#contact_lists).  The reader keeps elements, attributes, and nesting, and skips text content, comments, and declarations.

- `parse` returns an unnamed root element whose children are the top-level elements of the text.  For malformed text, the result contains everything read up to the error.
- `attr` returns the value of an attribute, or `""` if the attribute is missing.
- `children_named` iterates over the direct children with the given name.
- `find_all` appends every descendant with the given name to `out`, depth first.
