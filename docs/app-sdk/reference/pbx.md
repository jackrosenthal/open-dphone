# `pbx`

Requests to the PBX's application server.

The app supplies a method name and parameters.  The phone adds the envelope and the credentials, and sends the request over the transport named by the app's provisioned `<account>` (see [`config`](config.md)):

| The account's `appserver` | Transport |
| --- | --- |
| `dasp://...` | The phone's DPMA session with the PBX. |
| `http://...` or `https://...` | An HTTP `POST` with the content type `application/json`, sent by the same client that implements [`http`](http.md).  The phone answers a Basic or Digest challenge with the account's `username` and `password`. |

## `request`

```rust
pub fn request(method: &str, parameters: &str) -> u32
```

Sends a request and returns the request ID.  `parameters` is a JSON object as text.  The phone sends the document `{"request": {"method": <method>, "parameters": <parameters>}}`.

```rust
self.asked = pbx::request("switchvox.users.presence.getInfo", r#"{"account_id": "101"}"#);
// in handle:
// Event::Pbx((id, PbxOutcome::Ok(json))) if id == self.asked => { ... }
```

The result arrives as [`Event::Pbx((id, outcome))`](app-events.md#pbx):

| `PbxOutcome` | Meaning |
| --- | --- |
| `Ok(text)` | The server returned a result.  `text` is the JSON text of `response.result` in the reply. |
| `Errors(text)` | The server returned errors.  `text` is the JSON text of `response.errors`. |
| `Transport(text)` | The phone got no usable answer.  `text` explains why. |

`request` never fails directly.  A request that can't be sent still returns an ID, and ends in `Transport`.  The possible `Transport` texts:

| Text | Cause |
| --- | --- |
| `this app has no application account configured` | The configuration has no `<application>` with an `<account>` for the app. |
| `parameters must be a JSON object` | `parameters` is not valid JSON, or is not an object. |
| `the application server address is not usable` | `appserver` doesn't start with `dasp://`, `http://`, or `https://`. |
| `not provisioned: no DPMA session` | The account uses `dasp://` and the phone has no session with the PBX. |
| `server answered N` | Over DPMA, the server answered with a status `N` of 400 or higher. |
| `HTTP status N` | Over HTTP, the status was not 2xx. |
| `the server's reply was not JSON`, `the server's reply had no response`, `the server's reply had no result` | The reply isn't the expected document. |
| Anything else | An error from the transport: the network, TLS, or a timeout. |

Two SDK helpers build on `request`.  [`pbx_request`](helpers.md#pbx_request) builds `parameters` from a `Json` value and adds the account's `account_id`, and [`pbx_error_text`](helpers.md#pbx_error_text) turns a failed outcome into a message to display.

## `cancel`

```rust
pub fn cancel(id: u32)
```

Discards a pending request, so that no `Event::Pbx` arrives for that ID.  The request isn't withdrawn from the server.
