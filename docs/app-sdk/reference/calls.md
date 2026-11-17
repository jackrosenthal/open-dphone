# `calls`

A call that an app places or answers is an ordinary call: the phone shows the call screen, selects the audio path, and writes the call log.

The functions that return `Result` wait up to 0.5 seconds for the phone's call control, and `send_dtmf` up to 0.5 seconds for each digit:

- `Error::Invalid(text)`: call control rejected the request as impossible: no such call, a call in the wrong state, or a bad argument.
- `Error::Failed(text)`: call control tried and failed.
- `Failed("the phone did not answer in time; what was asked may still happen")`: call control didn't answer within that time, and may still carry out the request.  [`list`](#list) or [`Event::Call`](app-events.md#call) shows what happened.

## `dial`

```rust
pub fn dial(number: &str, account: Option<u8>, headers: &[Header]) -> Result<u32, Error>
```

The phone puts other calls on hold.

- `number`: a dial string or a SIP URI, 1 to 256 bytes.
- `account`: the account's slot, from 0.  `None` selects the first registered account, or the first account if none is registered.
- `headers`: added to the `INVITE`; at most 30.  Each name is a SIP token of 2 to 64 characters, and each value one line of at most 512 bytes.  These headers can't be set: `Via`, `From`, `To`, `Call-ID`, `CSeq`, `Contact`, `Max-Forwards`, `Content-Type`, `Content-Length`, `Route`, `Record-Route`, `Authorization`, `Proxy-Authorization`, `Session-Expires`, `Min-SE`, `Supported`, `Require`, `Allow`, `Replaces`, `Referred-By`, `User-Agent`, `Expires`, `WWW-Authenticate`, `Proxy-Authenticate`, and every one-letter name.

| Error | Cause |
| --- | --- |
| `Invalid("bad number")` | `number` is empty or longer than 256 bytes. |
| `Invalid("no such account")` | `account` isn't an account's slot. |
| `Invalid("no account is configured")` | The phone has no accounts. |
| `Invalid("too many calls")` | The phone has the maximum number of calls: four unless configured otherwise. |
| `Invalid("too many headers")`, `Invalid("the header ... cannot be set")` | `headers` breaks a rule above. |
| `Failed(text)` | The phone couldn't open the audio or start the call. |

## `answer`

```rust
pub fn answer(call: u32) -> Result<(), Error>
```

Answers a ringing incoming call, with the audio path selected as for a call answered at the phone.  Returns `Invalid("no such call")` for an unknown ID, and `Invalid("the call is not ringing")` for an outgoing, answered, or ended call.

## `reject`

```rust
pub fn reject(call: u32) -> Result<(), Error>
```

Declines a ringing incoming call with SIP status 603.  Errors as for `answer`.

## `hangup`

```rust
pub fn hangup(call: u32) -> Result<(), Error>
```

Ends a call in any state.  A ringing incoming call is rejected with SIP status 486.  Returns `Ok` for an ended call, and `Invalid("no such call")` for an unknown ID.

## `send_dtmf`

```rust
pub fn send_dtmf(call: u32, digits: &str) -> Result<(), Error>
```

Sends at most 64 DTMF digits, each `0` to `9`, `*`, or `#`, by the call's negotiated method: RTP telephone events, or else SIP `INFO`.

Any other character returns `Invalid("digits must be 0-9, *, or #")`, and nothing is sent.  The digits go one at a time, and the first failure stops the rest with `Invalid("no such call")` or `Invalid("the call has no media")`.

## `list`

```rust
pub fn list() -> Vec<CallInfo>
```

Every call on the phone ([`CallInfo`](types.md#calls)).  An ended call stays in the list for a few seconds, as `Ended`.

## `observe`

```rust
pub fn observe(on: bool)
```

Turns call events on or off; off in each new instance.  While on, [`Event::Call`](app-events.md#call) arrives, on or off screen, whenever any call changes in a way that [`CallInfo`](types.md#calls) shows.

## `get_mwi`

```rust
pub fn get_mwi() -> Vec<Mwi>
```

The counts of new and old messages for each account that has counts.  Changes arrive as [`Event::Mwi`](app-events.md#mwi), without `observe`.

## `dial_voicemail`

```rust
pub fn dial_voicemail(account: Option<u8>) -> Result<(), Error>
```

Calls an account's voicemail number.  `None` selects the account as for `dial`.  The call ID isn't returned; `list` or `Event::Call` shows the call.  Returns `Invalid("no voicemail number is configured")` if the account has no voicemail number, and otherwise the errors of `dial`.
