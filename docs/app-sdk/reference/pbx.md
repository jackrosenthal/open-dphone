# `pbx`

Requests to the PBX's application server: one function for each method of Open DPMA (and the original DPMA).

The phone adds the envelope, the credentials, and the `account_id` of the app's provisioned `<account>` ([`config`](config.md)), and sends the request by the account's transport:

| The account's `appserver` | Transport |
| --- | --- |
| `dasp://...` | The phone's DPMA session with the PBX. |
| `http://...` or `https://...` | An HTTP `POST` of `application/json` by the [`http`](http.md) client.  The phone answers a Basic or Digest challenge with the account's `username` and `password`. |

Every function returns a request ID at once.  The result arrives as [`Event::Pbx((id, result))`](app-events.md#pbx), a `Result<PbxResult, PbxError>`:

| `PbxResult` | Answers |
| --- | --- |
| `Done` | The functions that change something. |
| `Presence(PbxPresence)` | `presence_info` |
| `PresenceOptions(Vec<PbxPresenceOption>)` | `presence_options` |
| `ParkingLots(Vec<ParkingLot>)` | `parking_lots` |
| `Queues(Vec<CallQueue>)` | `queues` |
| `QueueStatus(QueueStatus)` | `queue_status` |
| `Voicemail(VoicemailPage)` | `voicemail_messages` |
| `VoicemailFolders(Vec<VoicemailFolder>)` | `voicemail_folders` |

| `PbxError` | Meaning |
| --- | --- |
| `Server(PbxServerError)` | The server refused: `code` is the DPMA error code (e.g., 27168: the account isn't permitted to do this), `message` the server's text. |
| `Transport(text)` | No usable answer; `text` says why. |

The `Transport` texts:

| Text | Cause |
| --- | --- |
| `this app has no application account configured` | The configuration has no `<application>` with an `<account>` for the app. |
| `the application server address is not usable` | `appserver` doesn't start with `dasp://`, `http://`, or `https://`. |
| `the phone has no such call` | `start_recording` or `stop_recording` named a call that the phone doesn't have. |
| `not provisioned: no DPMA session` | The account uses `dasp://`, and the phone has no session with the PBX. |
| `server answered N` | Over DPMA, the server answered with a status `N` of 400 or higher. |
| `HTTP status N` | Over HTTP, the status wasn't 2xx. |
| `the server's reply was not JSON`, `the server's reply had no response`, `the server's reply had no result`, `the server's reply was not the method's` | The reply isn't the method's document. |
| Anything else | A transport error: the network, TLS, or a timeout. |

`PbxError` implements `Display`: the server's message, or `Could not reach the server.` and the transport's text on a second line, for an error screen.

The result types are listed in [Types](types.md#pbx).

## Presence

### `presence_info`

```rust
pub fn presence_info() -> u32
```

The user's presence on the PBX.  Ends in `Presence`.

### `presence_options`

```rust
pub fn presence_options() -> u32
```

The presences the user can choose.  Ends in `PresenceOptions`.

### `set_presence`

```rust
pub fn set_presence(option_id: u32, message: &str) -> u32
```

Sets the user's presence to the option with the `id` `option_id`, with `message` (may be empty).  Ends in `Done`.  The phone's own presence ([`presence::set_status`](presence.md#set_status)) doesn't change.

## Parking

### `parking_lots`

```rust
pub fn parking_lots() -> u32
```

The parking lots, and the calls parked in the lots that the phone may see: the lots of the phone's parking applications, or every lot if the phone has none.  Ends in `ParkingLots`.  Dialing a call's `space` takes the call.

## Recording

### `start_recording`

```rust
pub fn start_recording(call: u32) -> u32
```

Starts recording a call of the phone into the voicemail box of the call's account, folder `Recordings`.  Ends in `Done`.

### `stop_recording`

```rust
pub fn stop_recording(call: u32) -> u32
```

Stops recording a call.  Ends in `Done`.

## Call queues

A queue function's `queue` is a `CallQueue::id`; `None` means every queue of the phone's queue applications.

### `queues`

```rust
pub fn queues() -> u32
```

The queues of the phone's queue applications.  Ends in `Queues`.

### `queue_status`

```rust
pub fn queue_status(queue: &str) -> u32
```

Today's figures of a queue.  Ends in `QueueStatus`.

### `queue_login`

```rust
pub fn queue_login(queue: Option<&str>) -> u32
```

Logs the user in.  Ends in `Done`.

### `queue_logout`

```rust
pub fn queue_logout(queue: Option<&str>) -> u32
```

Logs the user out.  Ends in `Done`.

### `queue_pause`

```rust
pub fn queue_pause(queue: Option<&str>, reason: &str) -> u32
```

Pauses the user, with `reason` (may be empty).  Ends in `Done`.

### `queue_resume`

```rust
pub fn queue_resume(queue: Option<&str>) -> u32
```

Ends a pause.  Ends in `Done`.

## Contacts

### `add_contact`

```rust
pub fn add_contact(phonebook: &str, extension: &str, name: &str) -> u32
```

Adds a contact to the phone's contact list whose `id` is `phonebook`, and provisions the phone again.  Ends in `Done`.

## Voicemail

A voicemail function's `folder` is a `VoicemailFolder::name`, and a message is a `VoicemailMessage::id`.

### `voicemail_messages`

```rust
pub fn voicemail_messages(folder: &str, sort: VoicemailSort, descending: bool, per_page: u32, page: u32) -> u32
```

One page of a folder's messages, ordered by `sort` (`Date` or `Id`), `per_page` messages to a page (at least 1), the first page 1.  Ends in `Voicemail`.

### `voicemail_folders`

```rust
pub fn voicemail_folders() -> u32
```

The folders and their message counts.  Ends in `VoicemailFolders`.

### `voicemail_move`

```rust
pub fn voicemail_move(folder: &str, target: &str, messages: &[String]) -> u32
```

Moves messages from `folder` to `target`.  Ends in `Done`.

### `voicemail_remove`

```rust
pub fn voicemail_remove(folder: &str, messages: &[String]) -> u32
```

Deletes messages.  Ends in `Done`.

### `voicemail_forward`

```rust
pub fn voicemail_forward(folder: &str, account: &str, messages: &[String]) -> u32
```

Copies messages to the inbox of the voicemail box of the account with the `id` `account`.  Ends in `Done`.

### `voicemail_mark_read`

```rust
pub fn voicemail_mark_read(folder: &str, messages: &[String]) -> u32
```

Marks messages heard.  Ends in `Done`.

### `voicemail_mark_unread`

```rust
pub fn voicemail_mark_unread(folder: &str, messages: &[String]) -> u32
```

Marks messages new.  Ends in `Done`.

### `voicemail_play`

```rust
pub fn voicemail_play(folder: &str, message: &str) -> u32
```

Has the PBX call the phone and play a message.  Ends in `Done`.  The call arrives as [`Event::Push`](app-events.md#push) `digium.incomingCall.voicemail`; observe the event with [`events::observe`](events.md#observe), and answer the call with [`calls::answer`](calls.md#answer).

## `cancel`

```rust
pub fn cancel(id: u32)
```

Discards the result of a pending request; no `Event::Pbx` arrives.  The server still receives the request.
