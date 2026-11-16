# `pbx`

Requests to the PBX's application server.  Capability `pbx`.

The app names a method and gives parameters; the phone adds the envelope and the credentials and sends the request over the transport that the app's provisioned `<account>` names (see [`config`](config.md)):

| `appserver` of the account | Transport |
| --- | --- |
| `dasp://...` | The phone's DPMA session with the PBX. |
| `http://...` or `https://...` | An HTTP `POST` with the content type `application/json`, by the client behind [`http`](http.md).  The phone answers a Basic or Digest challenge with the account's `username` and `password`. |

## `request`

```rust
pub fn request(method: &str, parameters: &str) -> u32
```

Sends a request and returns the request's identifier.  `parameters` is the text of a JSON object.  The phone sends the document `{"request": {"method": <method>, "parameters": <parameters>}}`.  The result arrives as [`Event::Pbx((id, outcome))`](app-events.md#pbx):

| `PbxOutcome` | Meaning |
| --- | --- |
| `Ok(text)` | The server answered with a result.  `text` is the JSON text of `response.result` of the reply. |
| `Errors(text)` | The server answered with errors.  `text` is the JSON text of `response.errors`. |
| `Transport(text)` | No usable answer; `text` says why. |

`request` never fails directly: a request that cannot be sent still returns an identifier and ends in `Transport`.  The causes of `Transport`:

| Text | Cause |
| --- | --- |
| `this app has no application account configured` | The configuration has no `<application>` with an `<account>` for the app. |
| `parameters must be a JSON object` | `parameters` is not JSON or not an object. |
| `the application server address is not usable` | `appserver` starts with none of `dasp://`, `http://`, and `https://`. |
| `not provisioned: no DPMA session` | The account uses `dasp://` and the phone has no session with the PBX. |
| `server answered N` | Over DPMA, the server answered with the status `N` of 400 or more. |
| `HTTP status N` | Over HTTP, the status was not 2xx. |
| `the server's reply was not JSON`, `the server's reply had no response`, `the server's reply had no result` | The reply is not the expected document. |
| Other texts | The transport's error (the network, TLS, a timeout). |

The SDK's [`pbx_request`](helpers.md#pbx_request) builds `parameters` from a `Json` value and adds the account's `account_id`, and [`pbx_error_text`](helpers.md#pbx_error_text) turns a failed outcome into a sentence for the screen.

```rust
self.asked = pbx::request("switchvox.users.presence.getInfo", r#"{"account_id": "101"}"#);
// in handle:
// Event::Pbx((id, PbxOutcome::Ok(json))) if id == self.asked => { ... }
```

## `cancel`

```rust
pub fn cancel(id: u32)
```

Forgets a request: no `Event::Pbx` arrives for the identifier.  The request is not withdrawn from the server.
