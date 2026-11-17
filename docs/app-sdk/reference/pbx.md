# `pbx`

Requests to the PBX's application server.

The app supplies a method and parameters.  The phone adds the envelope and the credentials, and sends the request by the transport of the app's provisioned `<account>` ([`config`](config.md)):

| The account's `appserver` | Transport |
| --- | --- |
| `dasp://...` | The phone's DPMA session with the PBX. |
| `http://...` or `https://...` | An HTTP `POST` of `application/json` by the [`http`](http.md) client.  The phone answers a Basic or Digest challenge with the account's `username` and `password`. |

## `request`

```rust
pub fn request(method: &str, parameters: &str) -> u32
```

`parameters` is a JSON object as text.  The phone sends `{"request": {"method": <method>, "parameters": <parameters>}}`.

```rust
self.asked = pbx::request("switchvox.users.presence.getInfo", r#"{"account_id": "101"}"#);
```

The result arrives as [`Event::Pbx((id, outcome))`](app-events.md#pbx):

| `PbxOutcome` | Meaning |
| --- | --- |
| `Ok(text)` | The JSON text of `response.result` in the reply. |
| `Errors(text)` | The JSON text of `response.errors` in the reply. |
| `Transport(text)` | No usable answer; `text` says why. |

`request` never fails directly: a request that can't be sent ends in `Transport`.  The `Transport` texts:

| Text | Cause |
| --- | --- |
| `this app has no application account configured` | The configuration has no `<application>` with an `<account>` for the app. |
| `parameters must be a JSON object` | `parameters` isn't valid JSON, or isn't an object. |
| `the application server address is not usable` | `appserver` doesn't start with `dasp://`, `http://`, or `https://`. |
| `not provisioned: no DPMA session` | The account uses `dasp://`, and the phone has no session with the PBX. |
| `server answered N` | Over DPMA, the server answered with a status `N` of 400 or higher. |
| `HTTP status N` | Over HTTP, the status wasn't 2xx. |
| `the server's reply was not JSON`, `the server's reply had no response`, `the server's reply had no result` | The reply isn't the expected document. |
| Anything else | A transport error: the network, TLS, or a timeout. |

The helpers [`pbx_request`](helpers.md#pbx_request) and [`pbx_error_text`](helpers.md#pbx_error_text) build on `request`.

## `cancel`

```rust
pub fn cancel(id: u32)
```

Discards the result of a pending request; no `Event::Pbx` arrives.  The server still receives the request.
