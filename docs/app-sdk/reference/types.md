# Shared types

The crate exports each of these types at the root, e.g., `dphone_app_sdk::Rect`.

## Handles and identifiers

```rust
pub type Widget = u32;
pub const ROOT: Widget = 0;
```

`ROOT` is the app's [window](ui.md#window).  Handles of widgets in the [idle window](ui.md#idle-window) have the high bit set.  The phone never reuses a handle within one instance of an app.

Timer and request IDs are never 0.  `pbx::request`, `http::request`, and the `pbx_storage` requests draw IDs from a single counter, so no two pending requests in an app share an ID.

## Geometry

```rust
pub struct Rect { pub x: i16, pub y: i16, pub width: u16, pub height: u16 }
pub struct Size { pub width: u16, pub height: u16 }

impl Rect {
    pub const fn new(x: i16, y: i16, width: u16, height: u16) -> Rect;
}
```

Coordinates are in pixels.

## `Metrics` and `Skin`

```rust
pub enum Skin { SmallGray, LargeGray, Color }

pub struct Metrics {
    pub skin: Skin,
    pub width: u16,
    pub height: u16,
    pub line_height: u16,
    pub title_height: u16,
    pub title_offset: u16,
    pub row_height: u16,
    pub scroll_bar_width: u16,
    pub color: bool,
}
```

The values that [`ui::get_metrics`](ui.md#get_metrics) returns depend on the skin:

| Field | `SmallGray` (D40, D45, D50) | `LargeGray` (D70) | `Color` (D60, D62, D65) |
| --- | --- | --- | --- |
| `width` | 240 | 320 | 480 |
| `height` | 102 | 140 | 243 |
| `height` in full-screen mode | 120 | 160 | 272 |
| `line_height` | 15 | 17 | 25 |
| `title_height` | 15 | 17 | 40 |
| `title_offset` | 1 | 1 | 4 |
| `row_height` | 15 | 17 | 32 |
| `scroll_bar_width` | 4 | 4 | 8 |
| `color` | `false` | `false` | `true` |

`width` and `height` are the dimensions of the app's window, and `row_height` is the height of one row in a list.

## Widgets

```rust
pub enum WidgetKind { Group, Text, TextList, Scroll, Input, Image }
pub enum Align { Left, Center, Right }
pub enum Font { Small, Normal, Large, Bold, Title }
pub enum InputMode { Text, Numeric, Dial, Password, Pin }

pub struct ListItem { pub text: String, pub detail: String, pub icon: String }
pub struct Softkey { pub id: u32, pub label: String }

pub enum WidgetEvent {
    Highlighted(u32),
    Activated(u32),
    Changed(String),
    Submitted(String),
}

impl ListItem {
    pub fn new(text: impl Into<String>) -> ListItem;
    pub fn with_detail(text: impl Into<String>, detail: impl Into<String>) -> ListItem;
}
impl Softkey {
    pub fn new(id: u32, label: impl Into<String>) -> Softkey;
}
```

See [`ui`](ui.md) for the widget kinds, fonts, input modes, list items, and softkeys.

## `Error`

```rust
pub enum Error {
    NotFound,
    Invalid(String),
    NoSpace,
    Failed(String),
}
```

| Variant | Meaning |
| --- | --- |
| `NotFound` | The named item doesn't exist. |
| `Invalid(text)` | An argument is invalid.  The text says which one and why. |
| `NoSpace` | The flash is full. |
| `Failed(text)` | The phone tried and failed, or refused.  The text gives the reason. |

## HTTP

```rust
pub struct Header { pub name: String, pub value: String }
pub struct HttpResponse { pub status: u16, pub headers: Vec<Header>, pub body: Vec<u8> }
```

## PBX requests

```rust
pub enum PbxOutcome { Ok(String), Errors(String), Transport(String) }
```

See [`pbx::request`](pbx.md#request).

## PBX storage

```rust
pub enum PbxScope { Shared, User }

pub struct PbxUser { pub id: String, pub name: String, pub extension: String }

pub struct PbxEntry {
    pub key: String,
    pub value: Vec<u8>,
    pub version: u64,
    pub modified: u64,
    pub author_id: String,
    pub author_name: String,
}

pub enum PbxStorageOutcome {
    Entry(Option<PbxEntry>),
    Entries((Vec<PbxEntry>, bool)),
    Stored(PbxEntry),
    Removed,
    Conflict(Option<PbxEntry>),
    Unreachable(String),
    Refused(String),
}

pub struct PbxStorageChange { pub scope: PbxScope, pub key: String }
```

See [`pbx_storage`](pbx-storage.md).

## Calls

```rust
pub enum CallState {
    Dialing, Proceeding, Ringback, Ringing, Active, Held, Transferring, Ended,
}

pub struct CallInfo {
    pub id: u32,
    pub account: u8,
    pub incoming: bool,
    pub state: CallState,
    pub remote_name: String,
    pub remote_number: String,
    pub duration: u32,
}

pub struct AccountInfo {
    pub slot: u8,
    pub id: String,
    pub label: String,
    pub user: String,
    pub server: String,
    pub registered: bool,
}

pub struct Mwi { pub account: u8, pub new_messages: u32, pub old_messages: u32 }
```

| Field | Contents |
| --- | --- |
| `CallInfo::account`, `Mwi::account`, `AccountInfo::slot` | The account's slot, starting at 0. |
| `CallInfo::state` | `Dialing`, `Proceeding`, and `Ringback` describe an outgoing call that hasn't been answered.  `Ringing` is an incoming call that hasn't been answered.  `Held` is a call on hold, by either side.  `Ended` is a call that is over but still listed for a few seconds. |
| `CallInfo::duration` | Seconds since the call was answered, computed when the `CallInfo` is created.  0 before the call is answered. |
| `AccountInfo::id` | The account's `id` attribute in the phone's configuration. |

## Presence

```rust
pub struct Presence {
    pub target: String,
    pub status: String,
    pub text: String,
    pub line_state: String,
}
```

| Field | Contents |
| --- | --- |
| `target` | The target string that the app passed in. |
| `status` | `available`, `chat`, `away`, `dnd`, `xa` (extended away), `unavailable`, or `unknown`. |
| `text` | The contact's status message, which may be empty. |
| `line_state` | The state of the contact's line: `idle`, `ringing`, `busy`, `hold`, `hold-ringing` (one call held and another ringing), `unavailable` (no device registered), or `unknown`. |

## `Push`

```rust
pub struct Push {
    pub name: String,
    pub kind: String,
    pub data: String,
    pub method: String,
    pub call: Option<u32>,
}
```

See [`Event::Push`](app-events.md#push).
