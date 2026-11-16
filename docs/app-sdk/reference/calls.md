# `calls`

Placing and controlling calls, call events, and message-waiting counts.  Capability `calls`.

A call that an app places or answers is an ordinary call of the phone: the phone shows the call screen, chooses the audio path, and writes the call log.  The app sees every call of the phone, not only calls the app started.

The functions that return `Result` pass the request to the phone's call control and wait up to 0.5 seconds.  A request that call control refuses as not possible (no such call, a call in the wrong state, a bad argument) returns `Error::Invalid(text)`; the tables below quote the texts.  A request that call control tried and could not carry out returns `Error::Failed(text)`.  When call control does not answer within the 0.5 seconds, the function returns `Failed("the phone did not answer in time; what was asked may still happen")`, and the request may still be carried out afterward; [`list`](#list) and [`Event::Call`](app-events.md#call) show what happened.

## `dial`

```rust
pub fn dial(number: &str, account: Option<u8>, headers: &[Header]) -> Result<u32, Error>
```

Places a call and returns the call's identifier.

- `number` is a dial string or a SIP URI, 1 to 256 bytes.
- `account` is the account's slot, counted from 0.  With `None`, the phone uses the first registered account, or the first account when none is registered.
- `headers` are added to the `INVITE`.  At most 30; each name is a SIP token of 2 to 64 characters and each value a single line of at most 512 bytes.  The headers that SIP and call control write cannot be set: `Via`, `From`, `To`, `Call-ID`, `CSeq`, `Contact`, `Max-Forwards`, `Content-Type`, `Content-Length`, `Route`, `Record-Route`, `Authorization`, `Proxy-Authorization`, `Session-Expires`, `Min-SE`, `Supported`, `Require`, `Allow`, `Replaces`, `Referred-By`, `User-Agent`, `Expires`, `WWW-Authenticate`, `Proxy-Authenticate`, and every one-letter name.

The phone puts the other calls on hold.

| Error | Cause |
| --- | --- |
| `Invalid("bad number")` | `number` is empty or longer than 256 bytes. |
| `Invalid("no such account")` | `account` names no account. |
| `Invalid("no account is configured")` | The phone has no account. |
| `Invalid("too many calls")` | The phone already has the most calls allowed (four unless configured otherwise). |
| `Invalid("too many headers")`, `Invalid("the header ... cannot be set")` | `headers` breaks a rule above. |
| `Failed(text)` | A failure to open the audio or to start the call, or no answer within 0.5 seconds (the call may still start). |

```rust
let call = calls::dial("*97", None, &[])?;
```

## `answer`

```rust
pub fn answer(call: u32) -> Result<(), Error>
```

Answers an incoming call that is ringing.  The phone chooses the audio path as for a call answered at the phone.  `Invalid("no such call")` for an unknown identifier, and `Invalid("the call is not ringing")` for an outgoing, answered, or ended call.

## `reject`

```rust
pub fn reject(call: u32) -> Result<(), Error>
```

Declines an incoming call that is ringing, with the SIP status 603.  The errors are those of `answer`.

## `hangup`

```rust
pub fn hangup(call: u32) -> Result<(), Error>
```

Ends a call in any state.  A ringing incoming call is rejected with the SIP status 486.  A call that has already ended returns `Ok`; an unknown identifier returns `Invalid("no such call")`.

## `send_dtmf`

```rust
pub fn send_dtmf(call: u32, digits: &str) -> Result<(), Error>
```

WIT name: `send-dtmf`.

Sends digits on a call, by the method the call negotiated (RTP telephone events, else SIP `INFO`).  `digits` is at most 64 characters, each `0` to `9`, `*`, or `#`; anything else returns `Invalid("digits must be 0-9, *, or #")` and sends nothing.  The digits are sent one by one, and the first failure stops the rest: `Invalid("no such call")` or `Invalid("the call has no media")`.

## `list`

```rust
pub fn list() -> Vec<CallInfo>
```

Every call of the phone, as [`CallInfo`](types.md#calls).  A call that ended stays in the list for a few seconds with the state `Ended`.

## `observe`

```rust
pub fn observe(on: bool)
```

With `true`, the app receives [`Event::Call`](app-events.md#call) for every change of every call that a [`CallInfo`](types.md#calls) shows, on screen or not.  The setting starts as `false` in every new instance.

## `get_mwi`

```rust
pub fn get_mwi() -> Vec<Mwi>
```

WIT name: `get-mwi`.

The counts of new and old messages, for each account the phone has counts for.  Changes arrive as [`Event::Mwi`](app-events.md#mwi) without a call to `observe`.

## `dial_voicemail`

```rust
pub fn dial_voicemail(account: Option<u8>) -> Result<(), Error>
```

WIT name: `dial-voicemail`.

Calls the voicemail number of an account (`None`: chosen as in `dial`).  The call's identifier is not returned; `list` or `Event::Call` shows the call.  `Invalid("no voicemail number is configured")` when the account has none, and otherwise the errors of `dial`.
