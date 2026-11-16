# Shared types

These types come from the `types` interface in `app.wit` and are used by more than one interface or by [events](app-events.md).  The crate re-exports each of them at the root (`dphone_app_sdk::Rect`), and the most common ones are also in the prelude (`dphone_app_sdk::prelude::*`).  Types used by a single interface (`canvas::Op`, `lamps::Color`, `alarms::Alarm`) are documented with that interface.

In Rust, enum variants are the `app.wit` names in camel case (`small-gray` becomes `Skin::SmallGray`), and record fields are in snake case (`line-height` becomes `line_height`).

## Handles and identifiers

```rust
pub type Widget = u32;
pub const ROOT: Widget = 0;
```

A `Widget` is a handle to a widget within one app.  `ROOT` (0) is the app's window, which is the parent of top-level widgets and holds the window's softkeys and key claims.  Handles for widgets in the [idle window](ui.md#idle-window) have the high bit set, and the idle window's root is `0x8000_0000`.

The phone never reuses a handle within one instance of an app.  Functions ignore a handle to a widget that no longer exists; given such a handle, `ui::selected` returns `None` and `ui::value` returns `""`.

Timer IDs, request IDs, and call IDs are `u32` values (`timer-id`, `request-id`, and `call-id` in `app.wit`).  Timer and request IDs are never 0.  `pbx::request`, `http::request`, and the `pbx_storage` requests draw IDs from a single counter, so no two pending requests in an app share an ID.

## Geometry

```rust
pub struct Rect { pub x: i16, pub y: i16, pub width: u16, pub height: u16 }
pub struct Size { pub width: u16, pub height: u16 }

impl Rect {
    pub const fn new(x: i16, y: i16, width: u16, height: u16) -> Rect;
}
```

Coordinates are in pixels.  A widget's `Rect` is relative to the top-left corner of the parent widget, or of the window for a top-level widget.

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

[`ui::get_metrics`](ui.md#get_metrics) returns the dimensions that an app needs for layout.  The values depend on the skin:

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

`width` and `height` are the dimensions of the app's window.  `row_height` is the height of one row in a list.  The grayscale skins have four shades: black, dark gray, light gray, and white.

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

See [`ui`](ui.md) for the widget kinds, fonts, input modes, list items, and softkeys.  `ListItem::new` leaves `detail` and `icon` empty, and `ListItem::with_detail` leaves `icon` empty.

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
| `NoSpace` | The flash is full.  The phone always keeps 2 MiB free. |
| `Failed(text)` | The phone tried and failed, or refused.  The text gives the reason. |

Functions in [`calls`](calls.md), [`presence`](presence.md), [`hotdesk`](hotdesk.md), and [`alarms`](alarms.md) hand a request to another part of the phone and wait for an answer, for up to 0.5 seconds (1.5 seconds for `alarms`):

- A request that can't be carried out as asked (a bad argument, no such call, a call in the wrong state) returns `Invalid(text)`.
- A request that was attempted and failed, or that the phone won't carry out for the app, returns `Failed(text)`.
- A request that gets no answer in time returns `Failed("the phone did not answer in time; what was asked may still happen")`.  The request was handed off, and may still be carried out.

## HTTP

```rust
pub struct Header { pub name: String, pub value: String }
pub struct HttpResponse { pub status: u16, pub headers: Vec<Header>, pub body: Vec<u8> }
```

[`calls::dial`](calls.md#dial) also uses `Header` for extra SIP headers.

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

See [`pbx-storage`](pbx-storage.md).

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
| `CallInfo::incoming` | `true` for a call that the phone received. |
| `CallInfo::state` | `Dialing`, `Proceeding`, and `Ringback` describe an outgoing call that hasn't been answered.  `Ringing` is an incoming call that hasn't been answered.  `Held` is a call on hold, by either side.  `Ended` is a call that is over but still listed for a few seconds. |
| `CallInfo::duration` | Seconds since the call was answered, computed when the `CallInfo` is created.  0 before the call is answered. |
| `AccountInfo::id` | The account's `id` attribute in the phone's configuration. |
| `AccountInfo::registered` | Whether the account is currently registered. |

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
