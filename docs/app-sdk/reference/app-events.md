# Events

The phone reports everything that happens by calling `App::handle` with an `Event`: a key press, a softkey, a timer, the result of a request, a change in a call.  In `app.wit`, the type is the `event` variant in the `types` interface.

```rust
pub enum Event {
    Foreground,
    Background,
    Exit,
    IdleScreenShow,
    IdleScreenHide,
    Key(String),
    Hook(bool),
    Softkey(u32),
    Widget((Widget, WidgetEvent)),
    Timer(u32),
    Pbx((u32, PbxOutcome)),
    Http((u32, Result<HttpResponse, String>)),
    Call(CallInfo),
    Push(Push),
    Mwi(Mwi),
    Presence(Presence),
    NewConfig,
    ServerChange,
    PlaybackStopped(u32),
    Message((String, String)),
    PbxStorage((u32, PbxStorageOutcome)),
    PbxStorageChanged(PbxStorageChange),
    PbxStorageOnline,
    AlarmsChanged,
    HotdeskDone(Result<(), String>),
}
```

The payload types are described in [Shared types](types.md).

## Summary

| Event | Sent when | Delivered to |
| --- | --- | --- |
| [`Foreground`](#foreground) | The app's window became the top screen. | The app. |
| [`Background`](#background) | The app's window stopped being the top screen. | The app. |
| [`Exit`](#exit) | The phone is about to discard the instance. | The app. |
| [`IdleScreenShow`](#idlescreenshow-and-idlescreenhide) | The app's idle window appeared on screen. | Apps with `has_idle`. |
| [`IdleScreenHide`](#idlescreenshow-and-idlescreenhide) | The app's idle window left the screen. | Apps with `has_idle`. |
| [`Key`](#key) | A claimed key was pressed, or any key during `input::capture`. | The app on screen. |
| [`Hook`](#hook) | The handset was lifted or replaced during `input::capture`. | The app on screen. |
| [`Softkey`](#softkey) | One of the app's softkeys was pressed. | The app on screen. |
| [`Widget`](#widget) | The user moved a list selection, activated a row, edited an input, or submitted an input. | The app on screen. |
| [`Timer`](#timer) | One of the app's timers fired. | The app that set the timer. |
| [`Pbx`](#pbx) | A `pbx::request` finished. | The app that made the request. |
| [`Http`](#http) | An `http::request` finished. | The app that made the request. |
| [`Call`](#call) | A call changed. | Apps that called `calls::observe(true)`. |
| [`Push`](#push) | The PBX pushed an event to the phone. | Apps that called `events::observe` with the event's name. |
| [`Mwi`](#mwi) | An account's message-waiting counts changed. | Every running app that uses `calls`. |
| [`Presence`](#presence) | A watched contact's presence changed. | Apps that called `presence::observe` with a matching target. |
| [`NewConfig`](#newconfig) | The phone applied a new configuration. | Every running app that uses `config`. |
| [`ServerChange`](#serverchange) | An account registered with the alternate host, or returned to the primary. | Every running app that uses `events`. |
| [`PlaybackStopped`](#playbackstopped) | The app's sound stopped playing. | The app that called `audio::play`. |
| [`Message`](#message) | An app called `events::send`. | The app named in the call. |
| [`PbxStorage`](#pbxstorage) | A `pbx_storage` request finished. | The app that made the request. |
| [`PbxStorageChanged`](#pbxstoragechanged) | Another phone set or removed one of the app's shared keys. | Apps that called `pbx_storage::watch(true)`. |
| [`PbxStorageOnline`](#pbxstorageonline) | The PBX store became reachable. | Every running app that uses `pbx_storage`. |
| [`AlarmsChanged`](#alarmschanged) | The alarm clock changed. | Every running app that uses `alarms`. |
| [`HotdeskDone`](#hotdeskdone) | A login or logout finished. | The app that called `hotdesk::login` or `hotdesk::logout`. |

Only running apps receive events.  The phone doesn't queue events for an app that isn't running, and discards an event addressed to an instance that no longer exists, such as the result of a request made before a restart.

## `Foreground`

The app's window became the top screen: either the app was opened, or a screen that covered the window (a call, a menu, another app) went away.  `runtime::in_foreground` returns `true` from this event until `Background`.

On a color model, the phone also sends `Foreground` to the app on screen when the user switches themes, so that an app that draws with [`canvas::theme_color`](canvas.md#theme_color) can redraw with the new colors.  Treat `Foreground` as "draw the screen," not as "the app was just opened."

## `Background`

The app's window stopped being the top screen: another screen covers the window, the user pressed line key 1 or an unclaimed `cancel`, or the app called `runtime::background`.

After `handle` returns, the phone stops a `Foreground` app that has no idle window, unless the app called `runtime::set_exit_after_background(false)`.  The app then receives `Exit`.  An app that keeps running keeps all widgets, timers, and pending requests, and any changes made to the window while off screen appear when the window returns.

When the app on screen leaves the screen, the phone also ends every call that belongs to apps (see [`Push`](#push)).

## `Exit`

The last event that an instance receives.  The phone sends `Exit` before discarding the instance for any of these reasons: `runtime::exit`, `runtime::restart`, the stop that follows `Background`, removal or replacement of the app, a configuration that disables a built-in app, or shutdown.  The handler gets the usual 2 seconds and can call any function, but a result that would arrive later is never delivered.

An app stopped by a fault (a trap, the time limit, or the memory limit) receives no `Exit`.

## `IdleScreenShow` and `IdleScreenHide`

`IdleScreenShow` means that the app's [idle window](ui.md#idle-window) appeared on screen.  That happens when the idle screen is the top screen and the app has the current turn among the apps with idle windows.

`IdleScreenHide` means that the idle window left the screen, either because another screen (a call, a menu, an app) covers the idle screen or because the turn passed to another app.

## `Key`

```rust
Event::Key(name)
```

The user pressed a key that the window or one of the app's widgets claimed with [`ui::claim_keys`](ui.md#claim_keys), or pressed any key while [`input::capture`](input.md) is on.  The phone doesn't report key releases or repeats, and the event doesn't say which widget claimed the key.

| Names | Keys |
| --- | --- |
| `0` to `9`, `*`, `#` | The dial pad. |
| `up`, `down`, `left`, `right`, `select`, `cancel` | The navigation keys. |
| `sk1` to `sk4` | The four keys below the display, from left to right. |
| `line1` to `line6` | The line keys, from top to bottom. |
| `blf1` to `blf10` | The rapid-dial keys, from top to bottom. |
| `vup`, `vdn` | Volume up and down. |
| `mute`, `spkr`, `hdst` | Mute, speaker, and headset. |
| `hold`, `xfer`, `conf`, `redial`, `msgs` | Hold, transfer, conference, redial, and messages. |
| `info`, `status`, `dir`, `apps`, `menu` | Information, status, contacts, applications, and menu. |
| `blfleft`, `blfright` | The page keys for the rapid-dial keys. |
| `expM.K` | Key `K` on expansion module `M`. |
| `keyN` | A key that the phone has no name for, identified by key code `N`. |

No model has every key in the table.

## `Hook`

```rust
Event::Hook(off_hook)
```

The handset was lifted (`true`) or replaced (`false`) while the app's window was on screen with [`input::capture`](input.md) on.  Without capture, the phone handles the hookswitch and doesn't notify the app.

## `Softkey`

```rust
Event::Softkey(id)
```

The user pressed a softkey.  `id` is the `id` of the `Softkey` passed to [`ui::set_softkeys`](ui.md#set_softkeys).  The "More" key that pages through more than four softkeys and the "Delete" key of an input don't produce `Softkey` events.

## `Widget`

```rust
Event::Widget((widget, widget_event))
```

The user interacted with a list or an input.  `widget` is the handle that `ui::create` returned.

| `WidgetEvent` | Sent when |
| --- | --- |
| `Highlighted(row)` | The user moved a list's selection with `up` or `down`.  `ui::set_selected` and `ui::set_items` don't send this event. |
| `Activated(row)` | The user pressed `select` on a list with at least one row. |
| `Changed(text)` | The user changed an input's text by typing or deleting.  `ui::set_value` doesn't send this event. |
| `Submitted(text)` | The user pressed `select` in an input. |

## `Timer`

```rust
Event::Timer(id)
```

A timer set with [`timers::set`](timers.md#set) fired.  Timers run whether or not the app is on screen.

## `Pbx`

```rust
Event::Pbx((id, outcome))
```

The result of a [`pbx::request`](pbx.md#request).  `id` is the ID that the call returned.  Every request ends with exactly one `Pbx` event, unless the app canceled the request.

## `Http`

```rust
Event::Http((id, result))
```

The result of an [`http::request`](http.md#request).  `Ok(response)` can carry any HTTP status, including 404 or 500.  `Err(text)` explains why no response was received.

## `Call`

```rust
Event::Call(info)
```

A call appeared, or a call's state, remote name, or remote number changed.  ([`CallInfo`](types.md#calls) describes the fields.)  The seconds counting up in `duration` don't cause events.  Apps that called [`calls::observe(true)`](calls.md#observe) receive the event for every call on the phone, not only for calls that the app placed.  The end of a call arrives as a single event with the state `CallState::Ended`.

## `Push`

```rust
Event::Push(push)
```

The PBX sent the phone an event whose name the app observes with [`events::observe`](events.md#observe).

| Field | Contents |
| --- | --- |
| `name` | The event's name. |
| `kind` | For an event that arrived with a call, the value of the `X-Event-Type` header; otherwise `""`. |
| `data` | The event's data as text.  For an event sent through the DPMA session, the payload.  For an event that arrived in an `INVITE` or `INFO`, the `X-Event-Data` header, or the body if there is no such header and the body isn't SDP. |
| `method` | `"MESSAGE"` for an event sent through the DPMA session; `"INVITE"` or `"INFO"` for an event that arrived with a call. |
| `call` | For `INVITE` and `INFO`, the call that the event arrived with. |

For an event carried by an `INVITE` or `INFO`, the name is taken from the last of the `X-Digium-Event-Name`, `X-Event-Name`, and `X-Event-Type` headers in the request.

An incoming call whose event name the phone's configuration assigns to apps (by default, `digium.incomingCall.voicemail`) doesn't ring and has no call screen.  The app answers or rejects the call with [`calls::answer`](calls.md#answer) or [`calls::reject`](calls.md#reject), using `push.call`.  If such an event names no call, the phone fills in `call` with the newest ringing call and sets `method` to `"INVITE"`, for every name that starts with `digium.incomingCall.`.

A call like this belongs to the apps that received the push.  Because the call has no screen where the user could hang up, the phone ends the call when the last of those apps has left the screen or stopped.  An app that isn't on screen when the push arrives keeps the call until the app has been on screen and left, or until the app stops.  A call whose event no running app observes ends when the app on screen leaves the screen.

## `Mwi`

```rust
Event::Mwi(mwi)
```

An account's counts of new and old messages changed.  Every running app that uses the `calls` interface receives this event without subscribing.

## `Presence`

```rust
Event::Presence(presence)
```

A contact that the phone watches changed, and the app observes a matching target with [`presence::observe`](presence.md#observe).  `presence.target` is the target string that the app passed to `observe`.

## `NewConfig`

The phone applied a new merged configuration.  Values read earlier from [`config`](config.md) may have changed.

## `ServerChange`

An account registered with a different host: the phone either failed over to the alternate host or returned to the primary.  The event has no payload; [`config::accounts`](config.md#accounts) shows the current registrations.  The push named `digium.phone.server_change` never arrives as `Event::Push`.

## `PlaybackStopped`

```rust
Event::PlaybackStopped(position_secs)
```

The sound that the app started with [`audio::play`](audio.md#play) is no longer playing.  Either the sound reached the end, the app called `audio::stop`, another sound replaced the app's sound, or the ringer cut the sound off.  The value is the position in the sound, in seconds.

## `Message`

```rust
Event::Message((name, payload))
```

An app called [`events::send`](events.md#send) with this app's name.  An app can send a message to any app by name, including the sending app.  The event doesn't identify the sender, and the receiving app needs no capability.

## `PbxStorage`

```rust
Event::PbxStorage((id, outcome))
```

The result of a [`pbx_storage`](pbx-storage.md) request.

## `PbxStorageChanged`

```rust
Event::PbxStorageChanged(change)
```

Another phone set or removed the app's shared key `change.key`.  The app receives this event after calling [`pbx_storage::watch(true)`](pbx-storage.md#watch).

## `PbxStorageOnline`

The app's store on the PBX became reachable.  Requests that ended in `Unreachable` are worth retrying, and anything the app read earlier may have changed.

## `AlarmsChanged`

The phone's alarm clock changed: an alarm was added, changed, or removed (by any app or by the web interface), an alarm rang, or an alarm was snoozed or dismissed.  Read the new state with [`alarms`](alarms.md).

## `HotdeskDone`

```rust
Event::HotdeskDone(result)
```

A [`hotdesk::login`](hotdesk.md#login) or [`hotdesk::logout`](hotdesk.md#logout) that this app started has finished.

- `Ok(())`: the phone has the requested configuration.
- `Err(text)`: the PBX or the phone refused.  `text` gives the reason (for example, a wrong PIN, with the PBX's error code), and the phone's assignment is unchanged.

The event arrives once for each call that returned `Ok`, and only at the app that made the call.
