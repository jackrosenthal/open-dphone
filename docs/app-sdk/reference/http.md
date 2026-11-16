# `http`

HTTP and HTTPS requests.  Capability `http`.

## `request`

```rust
pub fn request(method: &str, url: &str, headers: &[Header], body: &[u8]) -> u32
```

Sends a request and returns the request's identifier.  The result arrives as [`Event::Http((id, result))`](app-events.md#http): `Ok(HttpResponse)` with the status, the headers, and the body of the final response, whatever the status; or `Err(text)` when no response was received.  `request` never fails directly: a request that cannot be sent still returns an identifier and ends in `Err`.

How the phone sends the request:

- `method` is letters only; `""` means `GET`.
- `url` starts with `http://` or `https://`.  A `user:password@` part gives credentials, which the phone sends only in answer to a Basic or Digest challenge of the server.
- The phone writes `Host`, `Connection: close`, and `Content-Length`; headers of the app with those names, and `Transfer-Encoding`, are dropped.  Without a `User-Agent` of the app, the phone sends `User-Agent: Open DPhone`.
- A header name must be nonempty and free of spaces, colons, and line breaks, and a value free of line breaks.
- Each request uses a new connection (HTTP/1.1).
- For HTTPS, the server's certificate is checked against the phone's trust bundle, unless the phone's configuration sets `allow_insecure_ssl`.
- The phone follows up to five redirects (301, 302, 303, 307, 308).  After 301, 302, or 303, a request other than `HEAD` continues as a `GET` without a body.  A redirect to another host, port, or scheme drops the credentials and the headers `Authorization` and `Cookie`.

Limits:

| Limit | Value | Result past the limit |
| --- | --- | --- |
| Connecting | 10 seconds | `Err("connecting to <address> timed out")` |
| Sending the request and reading the response | 30 seconds | `Err("the request timed out")` |
| Response, head and body together | 1 MiB | `Err("reply too large")` |
| Redirects | 5 | `Err("too many redirects")` |

Other `Err` texts name the cause: `only http and https URLs are allowed`, `bad request method`, `a header is malformed`, a host that does not resolve, a refused connection, or a failed TLS handshake.

```rust
let headers = [Header { name: "Accept".into(), value: "application/json".into() }];
self.asked = http::request("GET", "https://example.org/weather.json", &headers, &[]);
// in handle:
// Event::Http((id, Ok(response))) if id == self.asked && response.status == 200 => { ... }
```

## `cancel`

```rust
pub fn cancel(id: u32)
```

Forgets a request: no `Event::Http` arrives for the identifier.  The HTTP exchange still runs to the end.
