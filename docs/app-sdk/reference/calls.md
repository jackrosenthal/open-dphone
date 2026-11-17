# `calls`

A call that an app places or answers is an ordinary call on the phone: the phone shows the call screen, selects the audio path, and writes the call log.

The functions that return `Result` hand the request to the phone's call control and wait up to 0.5 seconds for an answer:

- `Error::Invalid(text)` means that call control rejected the request as impossible: no such call, a call in the wrong state, or a bad argument.
- `Error::Failed(text)` means that call control tried to carry out the request and couldn't.
- `Failed("the phone did not answer in time; what was asked may still happen")` means that call control didn't answer within 0.5 seconds.  The request may still be carried out afterward.  Check [`list`](#list) or [`Event::Call`](app-events.md#call) to find out what happened.

## `dial`

```rust
pub fn dial(number: &str, account: Option<u8>, headers: &[Header]) -> Result<u32, Error>
```

The phone puts any other calls on hold.

- `number` is a dial string or a SIP URI, 1 to 256 bytes long.
- `account` is the account's slot, starting at 0.  With `None`, the phone uses the first registered account, or the first account if none is registered.
- `headers` are added to the `INVITE`.  There can be at most 30.  Each name must be a SIP token of 2 to 64 characters, and each value a single line of at most 512 bytes.  Headers that SIP and call control write can't be set: `Via`, `From`, `To`, `Call-ID`, `CSeq`, `Contact`, `Max-Forwards`, `Content-Type`, `Content-Length`, `Route`, `Record-Route`, `Authorization`, `Proxy-Authorization`, `Session-Expires`, `Min-SE`, `Supported`, `Require`, `Allow`, `Replaces`, `Referred-By`, `User-Agent`, `Expires`, `WWW-Authenticate`, `Proxy-Authenticate`, and every one-letter name.

| Error | Cause |
| --- | --- |
| `Invalid("bad number")` | `number` is empty or longer than 256 bytes. |
| `Invalid("no such account")` | `account` doesn't refer to an account. |
| `Invalid("no account is configured")` | The phone has no accounts. |
| `Invalid("too many calls")` | The phone already has the maximum number of calls (four unless configured otherwise). |
| `Invalid("too many headers")`, `Invalid("the header ... cannot be set")` | `headers` breaks one of the rules above. |
| `Failed(text)` | The phone couldn't open the audio or start the call. |

## `answer`

```rust
pub fn answer(call: u32) -> Result<(), Error>
```

Answers a ringing incoming call.  The phone selects the audio path the same way as for a call answered at the phone.

Returns `Invalid("no such call")` for an unknown ID, and `Invalid("the call is not ringing")` for a call that is outgoing, already answered, or over.

## `reject`

```rust
pub fn reject(call: u32) -> Result<(), Error>
```

Declines a ringing incoming call with SIP status 603.  The errors are the same as for `answer`.

## `hangup`

```rust
pub fn hangup(call: u32) -> Result<(), Error>
```

Ends a call in any state.  A ringing incoming call is rejected with SIP status 486.

Returns `Ok` for a call that has already ended, and `Invalid("no such call")` for an unknown ID.

## `send_dtmf`

```rust
pub fn send_dtmf(call: u32, digits: &str) -> Result<(), Error>
```

Sends DTMF digits on a call, using the method that the call negotiated: RTP telephone events, or else SIP `INFO`.  `digits` is at most 64 characters, each of which is `0` through `9`, `*`, or `#`.

Returns `Invalid("digits must be 0-9, *, or #")`, without sending anything, if `digits` contains any other character.  The digits are sent one at a time, and the first failure stops the rest with `Invalid("no such call")` or `Invalid("the call has no media")`.

## `list`

```rust
pub fn list() -> Vec<CallInfo>
```

Returns every call on the phone.  See [`CallInfo`](types.md#calls) for the fields.  A call that has ended stays in the list for a few seconds with the state `Ended`.

## `observe`

```rust
pub fn observe(on: bool)
```

Turns call events on or off.  While call events are on, the app receives an [`Event::Call`](app-events.md#call) every time any call changes in a way that shows in [`CallInfo`](types.md#calls), whether or not the app is on screen.  Call events are off in every new instance.

## `get_mwi`

```rust
pub fn get_mwi() -> Vec<Mwi>
```

Returns the counts of new and old messages for each account that the phone has counts for.  Changes arrive as [`Event::Mwi`](app-events.md#mwi), with no need to call `observe`.

## `dial_voicemail`

```rust
pub fn dial_voicemail(account: Option<u8>) -> Result<(), Error>
```

Calls an account's voicemail number.  With `None`, the phone chooses the account the same way as `dial`.  The call ID isn't returned; use `list` or `Event::Call` to find the call.

Returns `Invalid("no voicemail number is configured")` if the account has no voicemail number.  Otherwise, the errors are the same as for `dial`.
