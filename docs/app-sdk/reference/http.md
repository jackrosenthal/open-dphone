# `http`

## `request`

```rust
pub fn request(method: &str, url: &str, headers: &[Header], body: &[u8]) -> u32
```

The result arrives as [`Event::Http((id, result))`](app-events.md#http): `Ok(HttpResponse)`, the final response with any status, or `Err(text)`, no response.  `request` never fails directly: a request that can't be sent ends in `Err`.

| Argument | Rules |
| --- | --- |
| `method` | Letters only.  `""` means `GET`. |
| `url` | Starts with `http://` or `https://`.  A `user:password@` part supplies credentials, which the phone sends only on a Basic or Digest challenge. |
| `headers` | Added to the request.  A name is not empty and has no spaces, colons, or line breaks; a value has no line breaks.  The phone writes `Host`, `Connection: close`, and `Content-Length`, and drops headers with those names and `Transfer-Encoding`.  Without a `User-Agent`, the phone sends `User-Agent: Open Desk Phone`. |

- Each request uses a new HTTP/1.1 connection.
- For HTTPS, the phone verifies the server's certificate against the phone's trust bundle, unless the configuration sets `allow_insecure_ssl`.
- The phone follows up to five redirects (301, 302, 303, 307, or 308).  After a 301, 302, or 303, a request other than `HEAD` continues as a `GET` without a body.  A redirect to another host, port, or scheme drops the credentials and the `Authorization` and `Cookie` headers.

| Limit | Value | `Err` text |
| --- | --- | --- |
| Connection time | 10 seconds | `connecting to <address> timed out` |
| Time to send the request and read the response | 30 seconds | `the request timed out` |
| Response size, headers and body | 1 MiB | `reply too large` |
| Redirects | 5 | `too many redirects` |

Other `Err` texts: `only http and https URLs are allowed`, `bad request method`, `a header is malformed`, and the reason for a failed name lookup, connection, or TLS handshake.

## `cancel`

```rust
pub fn cancel(id: u32)
```

Discards the result of a pending request; no `Event::Http` arrives.  The HTTP exchange still completes.
