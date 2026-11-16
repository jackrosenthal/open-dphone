# Shared types

These types belong to the interface `types` of `app.wit` and are used by more than one interface or by [events](app-events.md).  The crate re-exports each at the root (`dphone_app_sdk::Rect`), and the prelude (`dphone_app_sdk::prelude::*`) holds the most common ones.  Types that one interface alone uses (`canvas::Op`, `lamps::Color`, `alarms::Alarm`) are on that interface's page.

Enum variants in Rust are the `app.wit` names in camel case (`small-gray` is `Skin::SmallGray`), and record fields are in snake case (`line-height` is `line_height`).

## Handles and identifiers

```rust
pub type Widget = u32;
pub const ROOT: Widget = 0;
```

A `Widget` names a widget within one app.  `ROOT` (0) is the app's window: the parent of top-level widgets, and the holder of the window's softkeys and key claims.  Handles of widgets in the [idle window](ui.md#the-idle-window) have the highest bit set, and the idle window's root is `0x8000_0000`.  The phone never reuses a handle within one instance of an app.  A function given a handle of a widget that no longer exists does nothing, and a getter returns `0` or `""`.

Timer identifiers, request identifiers, and call identifiers are `u32` (`timer-id`, `request-id`, and `call-id` in `app.wit`).  Timer and request identifiers are never 0.  The identifiers of `pbx::request`, `http::request`, and the `pbx_storage` requests come from one counter, so no two pending requests of an app share an identifier.

## Geometry

```rust
pub struct Rect { pub x: i16, pub y: i16, pub width: u16, pub height: u16 }
pub struct Size { pub width: u16, pub height: u16 }

impl Rect {
    pub const fn new(x: i16, y: i16, width: u16, height: u16) -> Rect;
}
```

Coordinates are pixels.  A widget's `Rect` is relative to the top-left corner of the parent widget, or of the window for a top-level widget.

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

[`ui::get_metrics`](ui.md#get_metrics) returns the sizes an app lays out with.  The values by skin:

| Field | `SmallGray` (D40, D45, D50) | `LargeGray` (D70) | `Color` (D60, D62, D65) |
| --- | --- | --- | --- |
| `width` | 240 | 320 | 480 |
| `height` | 102 | 140 | 243 |
| `height` while full-screen | 120 | 160 | 272 |
| `line_height` | 15 | 17 | 25 |
| `title_height` | 15 | 17 | 40 |
| `title_offset` | 1 | 1 | 4 |
| `row_height` | 15 | 17 | 32 |
| `scroll_bar_width` | 4 | 4 | 8 |
| `color` | `false` | `false` | `true` |

`width` and `height` are the size of the app's window.  `row_height` is the height of one row of a list.  The gray skins show four levels: black, dark gray, light gray, and white.

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

The page [`ui`](ui.md) describes the widget kinds, the fonts, the input modes, list items, and softkeys.  `WidgetKind::TextList` is `text-list` in `app.wit`.  `ListItem::new` leaves `detail` and `icon` empty, and `ListItem::with_detail` leaves `icon` empty.

## `Error`

```rust
pub enum Error {
    Denied,
    NotFound,
    Invalid(String),
    NoSpace,
    Failed(String),
}
```

| Variant | Meaning |
| --- | --- |
| `Denied` | The phone will not do this for the app.  `storage` returns `Denied` for a key whose file is a symbolic link. |
| `NotFound` | The thing named does not exist. |
| `Invalid(text)` | An argument is unusable; the text says which and why. |
| `NoSpace` | The flash has no room; the phone always keeps 2 MiB free. |
| `Failed(text)` | The phone tried and failed; the text is the reason. |

The functions of [`calls`](calls.md), [`presence`](presence.md), [`hotdesk`](hotdesk.md), and [`alarms`](alarms.md) pass a request to another part of the phone and wait for the answer (0.5 seconds; 1.5 seconds for `alarms`).  A request that the other part refuses as not possible (a bad argument, no such call, a call in the wrong state) returns `Invalid(text)`, a request the other part will not carry out for the app returns `Denied`, and a request the other part tried and could not carry out returns `Failed(text)`.  No answer in time is `Failed("the phone did not answer in time; what was asked may still happen")`: the request was passed on, and the other part may still carry the request out.

## HTTP

```rust
pub struct Header { pub name: String, pub value: String }
pub struct HttpResponse { pub status: u16, pub headers: Vec<Header>, pub body: Vec<u8> }
```

`Header` is also the type of the extra SIP headers of [`calls::dial`](calls.md#dial).

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

| Field | Content |
| --- | --- |
| `CallInfo::account`, `Mwi::account`, `AccountInfo::slot` | The account's slot, counted from 0. |
| `CallInfo::incoming` | `true` for a call the phone received. |
| `CallInfo::state` | `Dialing`, `Proceeding`, and `Ringback` are an outgoing call before the answer; `Ringing` is an incoming call before the answer; `Held` is a call on hold from either side; `Ended` is a call that is over and still listed for a few seconds. |
| `CallInfo::duration` | Seconds since the call was answered, computed when the `CallInfo` is made; 0 before the answer. |
| `AccountInfo::id` | The `id` attribute of the account in the phone's configuration. |
| `AccountInfo::registered` | Whether the account is registered now. |

## Presence

```rust
pub struct Presence {
    pub target: String,
    pub status: String,
    pub text: String,
    pub line_state: String,
}
```

| Field | Content |
| --- | --- |
| `target` | The string the app asked with. |
| `status` | `available`, `chat`, `away`, `dnd`, `xa` (extended away), `unavailable`, or `unknown`. |
| `text` | The contact's status message; may be empty. |
| `line_state` | The contact's line: `idle`, `ringing`, `busy`, `hold`, `hold-ringing` (one call held and another ringing), `unavailable` (no device registered), or `unknown`. |

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
