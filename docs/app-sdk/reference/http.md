# `http`

## `request`

```rust
pub fn request(method: &str, url: &str, headers: &[Header], body: &[u8]) -> u32
```

The result arrives as [`Event::Http((id, result))`](app-events.md#http).  `Ok(HttpResponse)` holds the status, headers, and body of the final response, whatever the status.  `Err(text)` means that no response was received.  `request` never fails directly: a request that can't be sent still returns an ID, and ends in `Err`.

Arguments:

- `method` must consist of letters only.  `""` means `GET`.
- `url` must start with `http://` or `https://`.  A `user:password@` part supplies credentials, which the phone sends only in response to a Basic or Digest challenge from the server.
- `headers` are added to the request.  A header name must not be empty or contain spaces, colons, or line breaks, and a value must not contain line breaks.  The phone writes `Host`, `Connection: close`, and `Content-Length`, and drops any headers with those names, as well as `Transfer-Encoding`, from `headers`.  If `headers` has no `User-Agent`, the phone sends `User-Agent: Open DPhone`.

Behavior:

- Each request uses a new HTTP/1.1 connection.
- For HTTPS, the phone verifies the server's certificate against the phone's trust bundle, unless the phone's configuration sets `allow_insecure_ssl`.
- The phone follows up to five redirects (301, 302, 303, 307, or 308).  After a 301, 302, or 303, any request other than `HEAD` continues as a `GET` with no body.  A redirect to a different host, port, or scheme drops the credentials and the `Authorization` and `Cookie` headers.

Limits:

| Limit | Value | Result when exceeded |
| --- | --- | --- |
| Connection time | 10 seconds | `Err("connecting to <address> timed out")` |
| Time to send the request and read the response | 30 seconds | `Err("the request timed out")` |
| Response size, headers and body combined | 1 MiB | `Err("reply too large")` |
| Redirects | 5 | `Err("too many redirects")` |

Other `Err` texts describe the cause: `only http and https URLs are allowed`, `bad request method`, `a header is malformed`, a host name that doesn't resolve, a refused connection, or a failed TLS handshake.

## `cancel`

```rust
pub fn cancel(id: u32)
```

Discards a pending request, so that no `Event::Http` arrives for that ID.  The HTTP exchange still runs to completion.
