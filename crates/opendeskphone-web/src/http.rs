//! The HTTP side: listeners, routing, sessions and CSRF checks, static
//! files, and the streamed upload endpoints.

use crate::assets;
use crate::auth::{self, Session};
use crate::rpc::{self, Body, Code, ConnectError, ResponseHeaders};
use crate::{pb, App, FirmwareUpload, PendingUpload};
use bytes::Bytes;
use http::{header, HeaderValue, Method, Request, Response, StatusCode};
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper_util::rt::{TokioIo, TokioTimer};
use std::convert::Infallible;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;

/// Largest buffered RPC request (backup restore and contact import are the
/// big ones).
pub(crate) const MAX_RPC_BODY: usize = 8 << 20;
const HEADER_TIMEOUT: Duration = Duration::from_secs(20);
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// Per-request context handed to the RPC handlers.
#[derive(Clone)]
pub(crate) struct Ctx {
    pub app: Arc<App>,
    pub session: Option<Session>,
    pub peer: IpAddr,
    /// The request came over HTTPS.
    pub secure: bool,
    pub headers: ResponseHeaders,
}

/// Binds a listener.  An IPv6 wildcard address also accepts IPv4; on a
/// kernel without IPv6 the IPv4 wildcard is used instead.
pub(crate) fn listen(addr: SocketAddr) -> io::Result<TcpListener> {
    let std = match std::net::TcpListener::bind(addr) {
        Ok(l) => l,
        Err(e) if addr.is_ipv6() && addr.ip().is_unspecified() => {
            let v4 = SocketAddr::from(([0, 0, 0, 0], addr.port()));
            log::debug!("binding {addr} failed ({e}); using {v4}");
            std::net::TcpListener::bind(v4)?
        }
        Err(e) => return Err(e),
    };
    std.set_nonblocking(true)?;
    TcpListener::from_std(std)
}

pub(crate) async fn accept_loop(
    app: Arc<App>,
    listener: TcpListener,
    tls: Option<Arc<rustls::ServerConfig>>,
    limit: Arc<Semaphore>,
) -> io::Result<()> {
    let acceptor = tls.map(tokio_rustls::TlsAcceptor::from);
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(x) => x,
            Err(e) => {
                // EMFILE and friends: back off instead of spinning.
                log::warn!("accept: {e}");
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
        };
        let Ok(permit) = limit.clone().try_acquire_owned() else {
            log::debug!("connection limit reached; refusing {peer}");
            continue;
        };
        let _ = stream.set_nodelay(true);
        let app = app.clone();
        let acceptor = acceptor.clone();
        let peer_ip = match peer.ip() {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6)),
            v4 => v4,
        };
        tokio::spawn(async move {
            let _permit = permit;
            match acceptor {
                Some(acc) => {
                    match tokio::time::timeout(TLS_HANDSHAKE_TIMEOUT, acc.accept(stream)).await {
                        Ok(Ok(tls)) => serve(app, tls, peer_ip, true).await,
                        Ok(Err(e)) => log::debug!("TLS handshake with {peer_ip}: {e}"),
                        Err(_) => log::debug!("TLS handshake with {peer_ip} timed out"),
                    }
                }
                None => serve(app, stream, peer_ip, false).await,
            }
        });
    }
}

async fn serve<S>(app: Arc<App>, stream: S, peer: IpAddr, secure: bool)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let switch = app.switch.clone();
    let service = hyper::service::service_fn(move |req| {
        let app = app.clone();
        async move { Ok::<_, Infallible>(handle(app, peer, secure, req).await) }
    });
    let conn = hyper::server::conn::http1::Builder::new()
        .timer(TokioTimer::new())
        .header_read_timeout(HEADER_TIMEOUT)
        .keep_alive(true)
        .serve_connection(TokioIo::new(stream), service);
    // A connection does not outlive web configuration being switched off:
    // a stream (the shell, the screen mirror, a status watch) would
    // otherwise go on.
    tokio::select! {
        r = conn => {
            if let Err(e) = r {
                log::trace!("connection from {peer}: {e}");
            }
        }
        _ = switch.turned_off() => log::debug!("connection from {peer} closed: web configuration disabled"),
    }
}

fn plain(status: StatusCode, text: &'static str) -> Response<Body> {
    let mut r = Response::new(Body::from(text));
    *r.status_mut() = status;
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    r
}

fn common_headers(resp: &mut Response<Body>) {
    let h = resp.headers_mut();
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert(header::SERVER, HeaderValue::from_static("Open Desk Phone"));
}

async fn handle(app: Arc<App>, peer: IpAddr, secure: bool, req: Request<Incoming>) -> Response<Body> {
    let mut resp = route(app, peer, secure, req).await;
    common_headers(&mut resp);
    resp
}

async fn route(app: Arc<App>, peer: IpAddr, secure: bool, req: Request<Incoming>) -> Response<Body> {
    let path = req.uri().path().to_owned();
    match *req.method() {
        // Open to any scraper on the network by design (see `metrics`),
        // unless switched off; independent of web configuration.
        Method::GET | Method::HEAD if path == "/metrics" => {
            if app.switch.metrics_enabled() {
                crate::metrics::respond(&app, &req)
            } else {
                plain(StatusCode::NOT_FOUND, "not found\n")
            }
        }
        Method::GET | Method::HEAD => static_file(&req, &path),
        // Web configuration disabled: the frontend may ask for the state
        // (and is told only that); nothing else is answered.
        Method::POST if !app.switch.enabled() && path != "/opendeskphone.v1.AuthService/GetSession" => {
            rpc::error_response(ConnectError::permission_denied("The web interface is disabled."), req.headers())
        }
        Method::POST => {
            if !same_origin(&req) {
                return rpc::error_response(
                    ConnectError::permission_denied("cross-origin request refused"),
                    req.headers(),
                );
            }
            let session = req
                .headers()
                .get(header::COOKIE)
                .and_then(|v| v.to_str().ok())
                .and_then(auth::cookie_token)
                .filter(|_| app.switch.enabled())
                .and_then(|t| app.sessions.touch(t));
            let ctx = Ctx { app, session, peer, secure, headers: ResponseHeaders::default() };
            if path.starts_with("/opendeskphone.v1.") {
                rpc(ctx, &path, req).await
            } else if path == "/upload/firmware" || path == "/upload/app" {
                match authorize(&ctx, &req) {
                    Ok(()) => upload(ctx, path == "/upload/firmware", req).await,
                    Err(e) => rpc::error_response(e, req.headers()),
                }
            } else {
                plain(StatusCode::NOT_FOUND, "not found\n")
            }
        }
        _ => {
            let mut r = plain(StatusCode::METHOD_NOT_ALLOWED, "method not allowed\n");
            r.headers_mut().insert(header::ALLOW, HeaderValue::from_static("GET, HEAD, POST"));
            r
        }
    }
}

/// A browser sends `Origin` on every cross-origin POST; when present it
/// must name the host the request was sent to.  (Requests without it come
/// from non-browser clients, which hold no ambient cookie.)
fn same_origin(req: &Request<Incoming>) -> bool {
    let Some(origin) = req.headers().get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    let Some(host) = req.headers().get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let origin_host = origin.split_once("://").map(|(_, h)| h).unwrap_or(origin);
    origin_host.eq_ignore_ascii_case(host)
}

/// Requires a session and its CSRF token.
fn authorize(ctx: &Ctx, req: &Request<Incoming>) -> Result<(), ConnectError> {
    let Some(session) = &ctx.session else {
        return Err(ConnectError::new(Code::Unauthenticated, "log in first"));
    };
    let token = req.headers().get(auth::CSRF_HEADER).map(|v| v.as_bytes()).unwrap_or_default();
    if !auth::constant_time_eq(token, session.csrf.as_bytes()) {
        return Err(ConnectError::new(Code::PermissionDenied, "missing or wrong X-Csrf-Token"));
    }
    Ok(())
}

/// RPCs that work without a session.
fn is_public(path: &str) -> bool {
    matches!(
        path,
        "/opendeskphone.v1.AuthService/Login" | "/opendeskphone.v1.AuthService/Logout" | "/opendeskphone.v1.AuthService/GetSession"
    )
}

async fn rpc(ctx: Ctx, path: &str, mut req: Request<Incoming>) -> Response<Body> {
    // Refuse before the body is read: an unauthenticated caller costs no
    // more than its request head.
    if !is_public(path)
        && let Err(e) = authorize(&ctx, &req) {
            return rpc::error_response(e, req.headers());
        }
    let mut service = ctx.app.rpc.clone();
    req.extensions_mut().insert(ctx);
    let Ok(resp) = tower_service::Service::call(&mut service, req).await;
    let streaming = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .is_some_and(|v| v.as_bytes().starts_with(b"application/connect+"));
    let mut resp = resp.map(Body::Rpc);
    if streaming {
        // Proxies and browsers must not buffer or transform the stream.
        resp.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store, no-transform"));
        resp.headers_mut().insert("x-accel-buffering", HeaderValue::from_static("no"));
    }
    resp
}

// ---------------------------------------------------------------- static

fn accepts(req: &Request<Incoming>, encoding: &str) -> bool {
    req.headers()
        .get(header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',').any(|part| {
                let mut it = part.trim().split(';');
                let name = it.next().unwrap_or("").trim();
                let refused = it.any(|p| matches!(p.trim(), "q=0" | "q=0.0" | "q=0.00" | "q=0.000"));
                name.eq_ignore_ascii_case(encoding) && !refused
            })
        })
}

const CSP: &str = "default-src 'self'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; \
                   connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'";

fn static_file(req: &Request<Incoming>, path: &str) -> Response<Body> {
    let asset = match assets::find(path) {
        Some(a) => a,
        // Client-side routes (no file extension) get the application shell.
        None if !path.rsplit('/').next().unwrap_or("").contains('.') && !path.starts_with("/opendeskphone.v1.") => {
            match assets::find("/index.html") {
                Some(a) => a,
                None => return plain(StatusCode::NOT_FOUND, "not found\n"),
            }
        }
        None => return plain(StatusCode::NOT_FOUND, "not found\n"),
    };
    let mut resp = Response::new(Body::Empty);
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(asset.mime));
    h.insert(header::ETAG, HeaderValue::from_static(asset.etag));
    h.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if asset.immutable { "public, max-age=31536000, immutable" } else { "no-cache" }),
    );
    if asset.mime.starts_with("text/html") {
        h.insert("content-security-policy", HeaderValue::from_static(CSP));
    }
    let not_modified = req
        .headers()
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim().trim_start_matches("W/") == asset.etag));
    if not_modified {
        *resp.status_mut() = StatusCode::NOT_MODIFIED;
        return resp;
    }
    let (data, encoding): (Bytes, Option<&'static str>) = if let (Some(br), true) = (asset.brotli, accepts(req, "br")) {
        (Bytes::from_static(br), Some("br"))
    } else if let (Some(gz), true) = (asset.gzip, accepts(req, "gzip")) {
        (Bytes::from_static(gz), Some("gzip"))
    } else if let Some(raw) = asset.raw {
        (Bytes::from_static(raw), None)
    } else {
        // A client without gzip: inflate the stored copy.
        match asset.gzip.and_then(assets::gunzip) {
            Some(raw) => (Bytes::from(raw), None),
            None => return plain(StatusCode::INTERNAL_SERVER_ERROR, "asset unavailable\n"),
        }
    };
    let h = resp.headers_mut();
    if let Some(enc) = encoding {
        h.insert(header::CONTENT_ENCODING, HeaderValue::from_static(enc));
    }
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(data.len()));
    if req.method() != Method::HEAD {
        *resp.body_mut() = Body::from(data);
    }
    resp
}

// --------------------------------------------------------------- uploads

fn json_response<T: serde::Serialize>(value: &T) -> Response<Body> {
    let mut r = Response::new(Body::from(serde_json::to_vec(value).unwrap_or_default()));
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    r
}

/// What an upload writes into.
enum Target {
    Firmware(Box<dyn FirmwareUpload>),
    App { path: std::path::PathBuf, file: std::fs::File },
}

async fn upload(ctx: Ctx, firmware: bool, req: Request<Incoming>) -> Response<Body> {
    let app = ctx.app.clone();
    let max = if firmware { app.options.max_firmware_bytes } else { app.options.max_app_bytes };
    let declared = req
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|n| n > max) {
        return rpc::plain_error(ConnectError::new(Code::ResourceExhausted, "File too large."));
    }
    let io_err = |e: io::Error| rpc::plain_error(ConnectError::internal(format!("storing upload: {e}")));

    let mut target = if firmware {
        match app.firmware.begin(declared) {
            Ok(u) => Target::Firmware(u),
            Err(e) => return io_err(e),
        }
    } else {
        let path = app.options.upload_dir.join(format!("app-{}.pkg", auth::random_token()));
        match std::fs::create_dir_all(&app.options.upload_dir).and_then(|()| std::fs::File::create(&path)) {
            Ok(file) => Target::App { path, file },
            Err(e) => return io_err(e),
        }
    };
    let abandon = |t: Target| {
        if let Target::App { path, .. } = t {
            let _ = std::fs::remove_file(path);
        }
    };

    let mut body = req.into_body();
    let mut size = 0u64;
    loop {
        let chunk = match body.frame().await {
            None => break,
            Some(Ok(frame)) => match frame.into_data() {
                Ok(d) => d,
                Err(_) => continue,
            },
            Some(Err(e)) => {
                log::info!("upload from {} interrupted: {e}", ctx.peer);
                abandon(target);
                return rpc::plain_error(ConnectError::new(Code::Aborted, "upload interrupted"));
            }
        };
        size += chunk.len() as u64;
        if size > max {
            abandon(target);
            return rpc::plain_error(ConnectError::new(Code::ResourceExhausted, "File too large."));
        }
        // Off the reactor thread: a sink may write to flash.
        let written = tokio::task::spawn_blocking(move || {
            use std::io::Write;
            let r = match &mut target {
                Target::Firmware(u) => u.write(&chunk),
                Target::App { file, .. } => file.write_all(&chunk),
            };
            (target, r)
        })
        .await;
        match written {
            Ok((t, Ok(()))) => target = t,
            Ok((t, Err(e))) => {
                abandon(t);
                return io_err(e);
            }
            Err(e) => return io_err(io::Error::other(e)),
        }
    }
    if size == 0 {
        abandon(target);
        return rpc::plain_error(ConnectError::invalid_argument("Empty file."));
    }

    let id = auth::random_token();
    match target {
        Target::Firmware(u) => {
            let finished = tokio::task::spawn_blocking(move || u.finish()).await;
            let (path, info) = match finished {
                Ok(Ok(x)) => x,
                Ok(Err(e)) => {
                    return rpc::plain_error(ConnectError::invalid_argument(format!("Invalid firmware file: {e}")));
                }
                Err(e) => return io_err(io::Error::other(e)),
            };
            let mut uploads = app.uploads.lock().unwrap();
            // Only one firmware image is kept at a time.
            let stale: Vec<String> =
                uploads.iter().filter(|(_, u)| u.firmware.is_some()).map(|(k, _)| k.clone()).collect();
            for k in stale {
                if let Some(old) = uploads.remove(&k) {
                    app.firmware.discard(&old.path);
                }
            }
            let msg = crate::services::uploaded_firmware(&app, &id, size, &info);
            uploads.insert(id, PendingUpload { path, size, firmware: Some(info) });
            log::info!("firmware upload from {}: {size} bytes", ctx.peer);
            json_response(&msg)
        }
        Target::App { path, file } => {
            drop(file);
            app.uploads.lock().unwrap().insert(id.clone(), PendingUpload { path, size, firmware: None });
            log::info!("app upload from {}: {size} bytes", ctx.peer);
            json_response(&pb::UploadedFirmware { upload_id: id, size, ..Default::default() })
        }
    }
}
