# SDK helpers

The crate `dphone-app-sdk` adds a few functions and types on top of the interface.  All of them run inside the app and only call the functions the other pages describe, so an app that uses a helper gains the capability of the interface the helper calls.

`use dphone_app_sdk::prelude::*;` brings in `App`, `Event`, `ROOT`, `Widget`, `Rect`, `ListItem`, `Softkey`, `WidgetKind`, `WidgetEvent`, `Skin`, `PbxOutcome`, `PbxScope`, `PbxEntry`, `PbxStorageOutcome`, `AppKind`, `Json`, `Form`, `Message`, the functions `screen`, `content_rect`, and `row_height`, and the modules `audio`, `calls`, `config`, `events`, `http`, `l10n`, `pbx`, `pbx_storage`, `presence`, `runtime`, `storage`, `timers`, and `ui`.  Everything else is imported by name, e.g., `dphone_app_sdk::canvas` or `dphone_app_sdk::InputMode`.

## Screens

### `screen`

```rust
pub fn screen(title: &str) -> Rect
```

Starts a new screen: clears the window (`ui::clear(ROOT)`, which also removes the window's softkeys and key claims), claims `cancel` for the window, and creates a `Text` across the top, `title_height` high, in `Font::Title`.  Returns the rectangle below the title.

Because the window claims `cancel`, the key no longer takes the app off the screen: the app receives `Event::Key("cancel")` and calls `runtime::background` or goes back a screen.

### `content_rect`

```rust
pub fn content_rect() -> Rect
```

The whole window as a rectangle at the origin: `Rect::new(0, 0, metrics.width, metrics.height)`.

### `row_height`

```rust
pub fn row_height() -> u16
```

`ui::get_metrics().row_height`.

### `Message`

```rust
pub struct Message { pub text: Widget }

impl Message {
    pub fn show(title: &str, text: &str, keys: &[Softkey]) -> Message;
}
```

A screen with a title and one wrapped text (a loading notice, an error, a confirmation).  `show` calls `screen`, creates the text, sets `keys` as the window's softkeys, and focuses the text.  The field `text` is the text widget, for `ui::set_text` later.

### `Form`

```rust
pub struct Form { pub inputs: Vec<Widget> }

impl Form {
    pub fn show(title: &str, fields: &[(&str, InputMode, &str)], keys: &[Softkey]) -> Form;
    pub fn value(&self, index: usize) -> String;
    pub fn focus_next(&self, current: Widget) -> bool;
}
```

A screen with a title and one labeled input per entry of `fields`: the label, the input mode, and the initial value.  `show` sets `keys` as the window's softkeys and focuses the first input.  `value(index)` is the text of an input, or `""` for an index out of range.  `focus_next(current)` moves the focus to the input after `current` and returns `false` when `current` is the last input or no input of the form, which suits `WidgetEvent::Submitted`: move on, or submit the form.

## Logging and storage

```rust
pub fn log_error(msg: &str)
pub fn log_warn(msg: &str)
pub fn log_info(msg: &str)
pub fn log_debug(msg: &str)
pub fn read_string(key: &str) -> Option<String>
```

The four log functions call [`runtime::log`](runtime.md#log) with the levels 0 to 3.  `read_string` is [`storage::get`](storage.md#get) for a value that is UTF-8 text; a value that is not UTF-8 gives `None`.

## PBX requests

### `pbx_request`

```rust
pub fn pbx_request(method: &str, parameters: Json) -> u32
```

Calls [`pbx::request`](pbx.md#request) with `parameters` written as JSON text.  When the parameters have no member `account_id` and [`config::app_setting("account_id")`](config.md#app_setting) is nonempty, the helper adds that member.  A `parameters` value that is not an object is sent as an empty object.  The helper uses the capabilities `pbx` and `config`.

```rust
let id = pbx_request(
    "switchvox.users.callQueues.getTodaysStatus",
    Json::obj([("call_queue_account_id", Json::str("201"))]),
);
```

### `pbx_error_text`

```rust
pub fn pbx_error_text(outcome: &PbxOutcome) -> String
```

A sentence for an error screen: for `Errors`, the `message` of the first error of the server's reply, or `The server reported an error.` when there is none; for `Transport`, `Could not reach the server.` and the transport's text on a second line; for `Ok`, `""`.

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

A small JSON value with a parser and a writer, in the module `json`.  Numbers are kept as their text.

| Function | Result |
| --- | --- |
| `Json::parse(text: &str) -> Option<Json>` | The value, or `None` when `text` is not one complete JSON value. |
| `get(&self, key: &str) -> &Json` | A member of an object; `Null` when missing or when the value is not an object. |
| `path(&self, keys: &[&str]) -> &Json` | `get` applied for each key in turn. |
| `text(&self) -> String` | The text of a string, a number, or a boolean; `""` otherwise. |
| `as_str(&self) -> Option<&str>` | The text of a string or a number. |
| `int(&self) -> Option<i64>` | The integer of a number or a numeric string (a fraction is cut off), or 0 or 1 for a boolean. |
| `truthy(&self) -> bool` | `true` for `true` and for the texts `1`, `true`, `yes`, and `on` in any letter case. |
| `is_null(&self) -> bool` | Whether the value is `Null`. |
| `items(&self) -> Vec<&Json>` | The elements of an array; no elements for `Null`; any other value counts as a list of one, because the PBX collapses lists of one element in some replies. |
| `Json::obj([(key, value), ...]) -> Json` | An object. |
| `Json::str(s) -> Json`, `Json::num(n: i64) -> Json` | A string and a number. |
| `write(&self, out: &mut String)`, `to_string()` | The JSON text. |

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

A minimal XML reader in the module `xml`, for the texts of [`config::app_config`](config.md#app_config) and [`config::contact_lists`](config.md#contact_lists).  The reader keeps elements, attributes, and nesting, and skips text content, comments, and declarations.

`parse` returns a root without a name whose children are the top-level elements of the text; for malformed text, the result holds what was read up to the fault.  `attr` returns `""` for a missing attribute, `children_named` goes through the direct children with a name, and `find_all` appends every descendant with a name to `out`, depth first.
