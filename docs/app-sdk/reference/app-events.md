# Events

The phone calls `App::handle` with one `Event` for everything that happens: a key, a softkey, a timer, the result of a request, a change of a call.  This page lists every variant, when the phone sends the variant, and which apps receive the variant.  In `app.wit` the type is the variant `event` of the interface `types`, and the names are the same in kebab-case (e.g., `pbx-storage-online` for `PbxStorageOnline`).

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
}
```

The payload types are on the page [Shared types](types.md).

## Summary

| Event | Sent when | Received by |
| --- | --- | --- |
| [`Foreground`](#foreground) | The app's window became the top screen. | The app. |
| [`Background`](#background) | The app's window stopped being the top screen. | The app. |
| [`Exit`](#exit) | The phone is about to drop the instance. | The app. |
| [`IdleScreenShow`](#idlescreenshow-and-idlescreenhide) | The app's idle window came on screen. | Apps with `has_idle`. |
| [`IdleScreenHide`](#idlescreenshow-and-idlescreenhide) | The app's idle window left the screen. | Apps with `has_idle`. |
| [`Key`](#key) | A claimed key was pressed, or any key under `input::capture`. | The app on screen. |
| [`Hook`](#hook) | The handset was lifted or put back, under `input::capture`. | The app on screen. |
| [`Softkey`](#softkey) | A softkey of the app was pressed. | The app on screen. |
| [`Widget`](#widget) | The user moved a list's selection, activated a row, edited an input, or submitted an input. | The app on screen. |
| [`Timer`](#timer) | A timer of the app is due. | The app that set the timer. |
| [`Pbx`](#pbx) | A `pbx::request` ended. | The app that made the request. |
| [`Http`](#http) | An `http::request` ended. | The app that made the request. |
| [`Call`](#call) | A call changed. | Apps that called `calls::observe(true)`. |
| [`Push`](#push) | The PBX pushed an event to the phone. | Apps that called `events::observe` with the event's name. |
| [`Mwi`](#mwi) | The message-waiting counts of an account changed. | Every running app that uses `calls`. |
| [`Presence`](#presence) | The presence of a watched contact changed. | Apps that called `presence::subscribe` with a matching target. |
| [`NewConfig`](#newconfig) | The phone applied a new configuration. | Every running app that uses `config`. |
| [`ServerChange`](#serverchange) | An account registered with the account's other host. | Every running app that uses `events`. |
| [`PlaybackStopped`](#playbackstopped) | The app's sound stopped playing. | The app that called `audio::play`. |
| [`Message`](#message) | Another app called `events::send`. | The app named in the call. |
| [`PbxStorage`](#pbxstorage) | A `pbx_storage` request ended. | The app that made the request. |
| [`PbxStorageChanged`](#pbxstoragechanged) | Another phone set or removed a shared key of the app. | Apps that called `pbx_storage::watch(true)`. |
| [`PbxStorageOnline`](#pbxstorageonline) | The store on the PBX came into reach. | Every running app that uses `pbx_storage`. |
| [`AlarmsChanged`](#alarmschanged) | The alarm clock changed. | Every running app that uses `alarms`. |

Only running apps receive events.  The phone keeps nothing for an app that is not running, and an event addressed to an instance that has since been dropped (e.g., the result of a request made before a restart) is discarded.

## `Foreground`

The app's window became the top screen: the app was opened, or a screen that covered the window (a call, a menu, another app) went away.  `runtime::in_foreground` returns `true` from this event until `Background`.

On a color model the phone sends `Foreground` again to the app on screen when the skin's palette changes (the user chose another wallpaper), so that an app that draws with [`canvas::theme_color`](canvas.md#theme_color) draws again.  An app therefore treats `Foreground` as "draw the screen", not as "the app was opened".

## `Background`

The app's window stopped being the top screen: another screen covers the window, the user pressed line key 1 or an unclaimed `cancel`, or the app called `runtime::background`.

After `handle` returns, the phone stops an app of kind `Foreground` that has no idle window, unless the app called `runtime::set_exit_after_background(false)`; the app then receives `Exit`.  An app that keeps running keeps the widgets, timers, and pending requests, and changes made to the window while off screen show when the window returns.

When the app on screen leaves the screen, the phone also ends every call that belongs to apps (see [`Push`](#push)).

## `Exit`

The last event of an instance.  The phone sends `Exit` before dropping the instance for `runtime::exit`, `runtime::restart`, the stop after `Background`, the removal or replacement of the app, the configuration disabling a built-in app, and the phone shutting down.  The handler has the usual 2 seconds, and functions called during the handler work; a result that would arrive later is never delivered.

An app stopped for a fault (a trap, the time limit, the memory limit) receives no `Exit`.

## `IdleScreenShow` and `IdleScreenHide`

`IdleScreenShow`: the app's [idle window](ui.md#the-idle-window) came on screen, which requires that the idle screen is the top screen and that the app has the turn among the apps with idle windows.  `IdleScreenHide`: the idle window left the screen, because another screen covers the idle screen (a call, a menu, an app) or the turn passed to another app.

## `Key`

```rust
Event::Key(name)
```

A key was pressed that a widget of the app or the window claimed with [`ui::claim_keys`](ui.md#claim_keys), or any key at all while [`input::capture`](input.md) is on.  Key releases and repeats are not reported.  The event does not say which widget claimed the key.

The names:

| Names | Keys |
| --- | --- |
| `0` to `9`, `*`, `#` | The dial pad. |
| `up`, `down`, `left`, `right`, `select`, `cancel` | The navigation keys. |
| `sk1` to `sk4` | The four keys under the display, from the left. |
| `line1` to `line6` | The line keys, from the top. |
| `blf1` to `blf10` | The rapid-dial keys, from the top. |
| `vup`, `vdn` | Volume up and down. |
| `mute`, `spkr`, `hdst` | Mute, speaker, and headset. |
| `hold`, `xfer`, `conf`, `redial`, `msgs` | Hold, transfer, conference, redial, and messages. |
| `info`, `status`, `dir`, `apps`, `menu` | Information, status, contacts, applications, and menu. |
| `blfleft`, `blfright` | The paging keys of the rapid-dial keys. |
| `expM.K` | Key `K` of expansion module `M`. |
| `keyN` | A key the phone has no name for, by the key's code `N`. |

Which of these a model has depends on the model.

## `Hook`

```rust
Event::Hook(off_hook)
```

The handset was lifted (`true`) or put back (`false`) while the app's window is on screen and [`input::capture`](input.md) is on.  Without capture the phone handles the hookswitch and the app is not told.

## `Softkey`

```rust
Event::Softkey(id)
```

The user pressed a softkey; `id` is the `id` of the `Softkey` given to [`ui::set_softkeys`](ui.md#set_softkeys).  The "More" key that pages through more than four softkeys and the "Delete" key of an input produce no `Softkey` event.

## `Widget`

```rust
Event::Widget((widget, widget_event))
```

| `WidgetEvent` | Sent when |
| --- | --- |
| `Highlighted(row)` | The user moved the selection of a list with `up` or `down`.  `ui::set_selected` and `ui::set_items` send nothing. |
| `Activated(row)` | The user pressed `select` on a list that has at least one row. |
| `Changed(text)` | The user changed the text of an input by typing or deleting.  `ui::set_value` sends nothing. |
| `Submitted(text)` | The user pressed `select` in an input. |

`widget` is the handle `ui::create` returned.

## `Timer`

```rust
Event::Timer(id)
```

A timer set with [`timers::set`](timers.md#set) is due.  Timers run whether or not the app is on screen.

## `Pbx`

```rust
Event::Pbx((id, outcome))
```

The result of [`pbx::request`](pbx.md#request): `id` is the identifier the call returned.  Each request ends with exactly one `Pbx` event unless the app cancelled the request.

## `Http`

```rust
Event::Http((id, result))
```

The result of [`http::request`](http.md#request).  `Ok(response)` carries any HTTP status, also 404 or 500; `Err(text)` says why no response was received.

## `Call`

```rust
Event::Call(info)
```

A call appeared, or the phone's record of a call changed (the state, the remote party, and also details that `CallInfo` does not show, so two events in a row can carry equal values).  The event arrives at apps that called [`calls::observe(true)`](calls.md#observe), for every call of the phone, not only calls the app placed.  The end of a call arrives as two events with the state `CallState::Ended`.

## `Push`

```rust
Event::Push(push)
```

The PBX sent the phone an event whose name the app observes ([`events::observe`](events.md#observe)).  The fields of `Push`:

| Field | Content |
| --- | --- |
| `name` | The event's name. |
| `kind` | The value of the header `X-Event-Type` for an event that arrived with a call, else `""`. |
| `data` | The event's data as text: the payload of an event sent through the DPMA session, or the header `X-Event-Data` (else the body, unless the body is SDP) of an event that arrived on an `INVITE` or `INFO`. |
| `method` | `"MESSAGE"` for an event sent through the DPMA session; `"INVITE"` or `"INFO"` for an event that arrived with a call. |
| `call` | The call the event arrived with, for `INVITE` and `INFO`. |

For an event carried by an `INVITE` or `INFO`, the name is the last of the headers `X-Digium-Event-Name`, `X-Event-Name`, and `X-Event-Type` in the request.

When the payload of an event from the DPMA session is a JSON object with a member `method` or `call`, the phone takes `kind`, `data`, `method`, and `call` from the object's members `type`, `data`, `method`, and `call` instead of passing the payload as `data`.

An incoming call whose event name the phone's configuration gives to apps (by default `digium.incomingCall.voicemail`) does not ring and has no call screen.  The app answers or rejects the call with [`calls::answer`](calls.md#answer) or [`calls::reject`](calls.md#reject), using `push.call`.  If such an event names no call, the phone fills `call` with the newest ringing call and sets `method` to `"INVITE"` for every name that starts with `digium.incomingCall.`.  The phone ends these calls when the app on screen leaves the screen.

## `Mwi`

```rust
Event::Mwi(mwi)
```

The counts of new and old messages of an account changed.  Every running app that uses the `calls` interface receives the event; there is nothing to turn on.

## `Presence`

```rust
Event::Presence(presence)
```

A contact that the phone watches changed, and the app subscribed to a target that matches the contact ([`presence::subscribe`](presence.md#subscribe)).  `presence.target` is the target string the app subscribed with.

## `NewConfig`

The phone applied a new merged configuration.  Values read earlier from [`config`](config.md) may have changed.

## `ServerChange`

An account registered with the account's other host: the phone failed over to the alternate host or returned to the primary.  The event has no payload; [`config::accounts`](config.md#accounts) shows the registrations.  The push named `digium.phone.server_change` never arrives as `Event::Push`.

## `PlaybackStopped`

```rust
Event::PlaybackStopped(position_secs)
```

The sound the app started with [`audio::play`](audio.md#play) is no longer playing: the sound reached the end, `audio::stop` was called, another sound replaced the sound, or the ringer cut the sound off.  The value is the position in the sound, in seconds.

## `Message`

```rust
Event::Message((name, payload))
```

An app (the receiving app included) called [`events::send`](events.md#send) with this app's name.  The event does not say which app sent the message.  The receiving app needs no capability.

## `PbxStorage`

```rust
Event::PbxStorage((id, outcome))
```

The result of a request of [`pbx_storage`](pbx-storage.md).

## `PbxStorageChanged`

```rust
Event::PbxStorageChanged(change)
```

Another phone set or removed the shared key `change.key` of this app, after [`pbx_storage::watch(true)`](pbx-storage.md#watch).

## `PbxStorageOnline`

The app's store on the PBX came into reach.  Requests that ended `Unreachable` are worth repeating, and what the app read earlier may have changed.

## `AlarmsChanged`

The phone's alarm clock changed: an alarm was added, changed, or removed (by any app or the web interface), an alarm rang, or an alarm was snoozed or dismissed.  The app reads the new state with [`alarms`](alarms.md).
