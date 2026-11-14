//! A thin Connect RPC server layer over HTTP/1.1 (plan spike S5).
//!
//! Covers what the firmware needs and no more: unary and server-streaming
//! calls by POST, the protobuf and JSON codecs, the Connect error model,
//! and the streaming envelope with its end-of-stream frame.  Compression
//! of messages is not offered (`identity` only), which the protocol allows.
//!
//! Reference: <https://connectrpc.com/docs/protocol>.

use bytes::{BufMut, Bytes, BytesMut};
use http::{header, HeaderName, HeaderValue, Response, StatusCode};
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tokio::sync::mpsc;

/// Connect error codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    Canceled,
    Unknown,
    InvalidArgument,
    DeadlineExceeded,
    NotFound,
    AlreadyExists,
    PermissionDenied,
    ResourceExhausted,
    FailedPrecondition,
    Aborted,
    OutOfRange,
    Unimplemented,
    Internal,
    Unavailable,
    DataLoss,
    Unauthenticated,
}

impl Code {
    pub const fn as_str(self) -> &'static str {
        match self {
            Code::Canceled => "canceled",
            Code::Unknown => "unknown",
            Code::InvalidArgument => "invalid_argument",
            Code::DeadlineExceeded => "deadline_exceeded",
            Code::NotFound => "not_found",
            Code::AlreadyExists => "already_exists",
            Code::PermissionDenied => "permission_denied",
            Code::ResourceExhausted => "resource_exhausted",
            Code::FailedPrecondition => "failed_precondition",
            Code::Aborted => "aborted",
            Code::OutOfRange => "out_of_range",
            Code::Unimplemented => "unimplemented",
            Code::Internal => "internal",
            Code::Unavailable => "unavailable",
            Code::DataLoss => "data_loss",
            Code::Unauthenticated => "unauthenticated",
        }
    }

    /// The HTTP status a unary error response carries.
    pub const fn http_status(self) -> u16 {
        match self {
            Code::Canceled => 499,
            Code::Unknown | Code::Internal | Code::DataLoss => 500,
            Code::InvalidArgument | Code::FailedPrecondition | Code::OutOfRange => 400,
            Code::DeadlineExceeded => 504,
            Code::NotFound => 404,
            Code::AlreadyExists | Code::Aborted => 409,
            Code::PermissionDenied => 403,
            Code::ResourceExhausted => 429,
            Code::Unimplemented => 501,
            Code::Unavailable => 503,
            Code::Unauthenticated => 401,
        }
    }
}

/// An RPC failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectError {
    pub code: Code,
    pub message: String,
}

impl ConnectError {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        ConnectError { code, message: message.into() }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(Code::InvalidArgument, message)
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(Code::NotFound, message)
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(Code::Internal, message)
    }
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(Code::Unavailable, message)
    }
    pub fn precondition(message: impl Into<String>) -> Self {
        Self::new(Code::FailedPrecondition, message)
    }

    fn json(&self) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({ "code": self.code.as_str(), "message": self.message }))
            .unwrap_or_default()
    }
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}
impl std::error::Error for ConnectError {}

/// Message encoding of a call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Proto,
    Json,
}

/// What every request and response message implements (prost for the
/// binary codec, pbjson's serde implementations for proto3 JSON).
pub trait Message:
    prost::Message + Default + serde::Serialize + serde::de::DeserializeOwned + Send + 'static
{
}
impl<T> Message for T where
    T: prost::Message + Default + serde::Serialize + serde::de::DeserializeOwned + Send + 'static
{
}

pub fn decode<M: Message>(codec: Codec, data: &[u8]) -> Result<M, ConnectError> {
    match codec {
        Codec::Proto => M::decode(data).map_err(|e| ConnectError::invalid(format!("bad protobuf message: {e}"))),
        Codec::Json => {
            // An empty body is not valid JSON, but a message with no fields
            // set is commonly sent that way by hand-written clients.
            let data = if data.iter().all(u8::is_ascii_whitespace) { b"{}" } else { data };
            serde_json::from_slice(data).map_err(|e| ConnectError::invalid(format!("bad JSON message: {e}")))
        }
    }
}

pub fn encode<M: Message>(codec: Codec, msg: &M) -> Bytes {
    match codec {
        Codec::Proto => Bytes::from(msg.encode_to_vec()),
        Codec::Json => Bytes::from(serde_json::to_vec(msg).unwrap_or_else(|_| b"{}".to_vec())),
    }
}

/// Parses a request `Content-Type`: the codec, and whether the call uses
/// the streaming envelope.
pub fn parse_content_type(ct: &str) -> Option<(Codec, bool)> {
    let base = ct.split(';').next()?.trim().to_ascii_lowercase();
    Some(match base.as_str() {
        "application/proto" => (Codec::Proto, false),
        "application/json" => (Codec::Json, false),
        "application/connect+proto" => (Codec::Proto, true),
        "application/connect+json" => (Codec::Json, true),
        _ => return None,
    })
}

const fn content_type(codec: Codec, streaming: bool) -> &'static str {
    match (codec, streaming) {
        (Codec::Proto, false) => "application/proto",
        (Codec::Json, false) => "application/json",
        (Codec::Proto, true) => "application/connect+proto",
        (Codec::Json, true) => "application/connect+json",
    }
}

/// Response body: nothing, one buffer, or frames from a channel.
pub enum Body {
    Empty,
    Full(Bytes),
    Stream(mpsc::Receiver<Bytes>),
}

impl From<Bytes> for Body {
    fn from(b: Bytes) -> Self {
        if b.is_empty() { Body::Empty } else { Body::Full(b) }
    }
}
impl From<Vec<u8>> for Body {
    fn from(b: Vec<u8>) -> Self {
        Bytes::from(b).into()
    }
}
impl From<&'static str> for Body {
    fn from(b: &'static str) -> Self {
        Bytes::from_static(b.as_bytes()).into()
    }
}
impl From<String> for Body {
    fn from(b: String) -> Self {
        Bytes::from(b).into()
    }
}

impl http_body::Body for Body {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, Infallible>>> {
        let this = self.get_mut();
        match this {
            Body::Empty => Poll::Ready(None),
            Body::Full(b) => {
                let b = std::mem::take(b);
                *this = Body::Empty;
                Poll::Ready(Some(Ok(http_body::Frame::data(b))))
            }
            Body::Stream(rx) => rx.poll_recv(cx).map(|o| o.map(|b| Ok(http_body::Frame::data(b)))),
        }
    }

    fn is_end_stream(&self) -> bool {
        matches!(self, Body::Empty)
    }

    fn size_hint(&self) -> http_body::SizeHint {
        match self {
            Body::Empty => http_body::SizeHint::with_exact(0),
            Body::Full(b) => http_body::SizeHint::with_exact(b.len() as u64),
            Body::Stream(_) => http_body::SizeHint::default(),
        }
    }
}

/// Extra response headers a handler wants (the login cookie).
#[derive(Clone, Default)]
pub struct ResponseHeaders(Arc<Mutex<Vec<(HeaderName, HeaderValue)>>>);

impl ResponseHeaders {
    pub fn add(&self, name: HeaderName, value: HeaderValue) {
        self.0.lock().unwrap().push((name, value));
    }
    fn apply(&self, resp: &mut Response<Body>) {
        for (n, v) in self.0.lock().unwrap().drain(..) {
            resp.headers_mut().append(n, v);
        }
    }
}

/// A decoded-enough RPC request: the codec, the framing, and the body.
pub struct RpcRequest {
    pub codec: Codec,
    pub streaming: bool,
    pub body: Bytes,
    pub headers: ResponseHeaders,
}

/// A unary error response.
pub fn error_response(err: &ConnectError) -> Response<Body> {
    let mut resp = Response::new(Body::from(err.json()));
    *resp.status_mut() = StatusCode::from_u16(err.code.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    resp
}

/// One enveloped frame: flags, big-endian length, payload.
fn envelope(flags: u8, payload: &[u8]) -> Bytes {
    let mut b = BytesMut::with_capacity(5 + payload.len());
    b.put_u8(flags);
    b.put_u32(payload.len() as u32);
    b.put_slice(payload);
    b.freeze()
}

const FLAG_COMPRESSED: u8 = 0x01;
const FLAG_END_STREAM: u8 = 0x02;

/// The end-of-stream frame, carrying the error if the stream failed.
fn end_stream(err: Option<&ConnectError>) -> Bytes {
    let payload = match err {
        None => b"{}".to_vec(),
        Some(e) => serde_json::to_vec(&serde_json::json!({
            "error": { "code": e.code.as_str(), "message": e.message }
        }))
        .unwrap_or_else(|_| b"{}".to_vec()),
    };
    envelope(FLAG_END_STREAM, &payload)
}

/// A streaming error response (HTTP 200 with only the end-of-stream frame).
pub fn stream_error_response(codec: Codec, err: &ConnectError) -> Response<Body> {
    let mut resp = Response::new(Body::from(end_stream(Some(err))));
    resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type(codec, true)));
    resp
}

/// The error response in whichever shape the request's framing calls for.
pub fn error_for(req: &RpcRequest, err: &ConnectError) -> Response<Body> {
    if req.streaming { stream_error_response(req.codec, err) } else { error_response(err) }
}

/// Takes the single request message out of a streaming request body.
fn unwrap_envelope(body: &Bytes) -> Result<Bytes, ConnectError> {
    if body.len() < 5 {
        return Err(ConnectError::invalid("streaming request without an enveloped message"));
    }
    let flags = body[0];
    let len = u32::from_be_bytes([body[1], body[2], body[3], body[4]]) as usize;
    if flags & FLAG_COMPRESSED != 0 {
        return Err(ConnectError::new(Code::Unimplemented, "compressed messages are not supported"));
    }
    if body.len() < 5 + len {
        return Err(ConnectError::invalid("truncated enveloped message"));
    }
    Ok(body.slice(5..5 + len))
}

/// Runs a unary handler.
pub async fn unary<C, Req, Res, F, Fut>(ctx: C, req: RpcRequest, f: F) -> Response<Body>
where
    Req: Message,
    Res: Message,
    F: FnOnce(C, Req) -> Fut,
    Fut: Future<Output = Result<Res, ConnectError>>,
{
    if req.streaming {
        return stream_error_response(
            req.codec,
            &ConnectError::new(Code::Unimplemented, "this method is unary; use application/proto or application/json"),
        );
    }
    let msg: Req = match decode(req.codec, &req.body) {
        Ok(m) => m,
        Err(e) => return error_response(&e),
    };
    let mut resp = match f(ctx, msg).await {
        Ok(res) => {
            let mut r = Response::new(Body::from(encode(req.codec, &res)));
            r.headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type(req.codec, false)));
            r
        }
        Err(e) => error_response(&e),
    };
    req.headers.apply(&mut resp);
    resp
}

/// The sending side of a server stream, handed to streaming handlers.
pub struct Sink<Res> {
    tx: mpsc::Sender<Bytes>,
    codec: Codec,
    _marker: std::marker::PhantomData<fn(Res)>,
}

/// The client went away.
#[derive(Debug)]
pub struct Closed;

impl From<Closed> for ConnectError {
    fn from(_: Closed) -> Self {
        ConnectError::new(Code::Canceled, "client closed the stream")
    }
}

impl<Res: Message> Sink<Res> {
    /// Sends one message.  Waits while the client is slow (the channel
    /// holds a few frames), so a stalled browser throttles the producer
    /// rather than growing memory.
    pub async fn send(&self, msg: &Res) -> Result<(), Closed> {
        let payload = encode(self.codec, msg);
        self.tx.send(envelope(0, &payload)).await.map_err(|_| Closed)
    }

    /// Resolves when the client has gone away.
    pub async fn closed(&self) {
        self.tx.closed().await
    }

    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }
}

/// Runs a server-streaming handler on its own task.  The handler returns
/// when the stream is finished, or when sending fails because the client
/// went away.
pub async fn stream<C, Req, Res, F, Fut>(ctx: C, req: RpcRequest, f: F) -> Response<Body>
where
    Req: Message,
    Res: Message,
    F: FnOnce(C, Req, Sink<Res>) -> Fut,
    Fut: Future<Output = Result<(), ConnectError>> + Send + 'static,
{
    if !req.streaming {
        return error_response(&ConnectError::new(
            Code::Unimplemented,
            "this method is server-streaming; use application/connect+proto or application/connect+json",
        ));
    }
    let msg: Req = match unwrap_envelope(&req.body).and_then(|b| decode(req.codec, &b)) {
        Ok(m) => m,
        Err(e) => return stream_error_response(req.codec, &e),
    };
    let (tx, rx) = mpsc::channel::<Bytes>(4);
    let sink = Sink { tx: tx.clone(), codec: req.codec, _marker: std::marker::PhantomData };
    let fut = f(ctx, msg, sink);
    tokio::spawn(async move {
        let result = fut.await;
        let err = match &result {
            Ok(()) => None,
            Err(e) if e.code == Code::Canceled => return,
            Err(e) => Some(e),
        };
        let _ = tx.send(end_stream(err)).await;
    });
    let mut resp = Response::new(Body::Stream(rx));
    resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type(req.codec, true)));
    // Proxies and browsers must not buffer or transform the stream.
    resp.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store, no-transform"));
    resp.headers_mut().insert("x-accel-buffering", HeaderValue::from_static("no"));
    req.headers.apply(&mut resp);
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_types() {
        assert_eq!(parse_content_type("application/json; charset=utf-8"), Some((Codec::Json, false)));
        assert_eq!(parse_content_type("application/connect+proto"), Some((Codec::Proto, true)));
        assert_eq!(parse_content_type("text/plain"), None);
    }

    #[test]
    fn envelope_layout() {
        let e = envelope(0, b"abc");
        assert_eq!(&e[..], &[0, 0, 0, 0, 3, b'a', b'b', b'c']);
        let end = end_stream(None);
        assert_eq!(end[0], 2);
        assert_eq!(&end[5..], b"{}");
        let err = end_stream(Some(&ConnectError::not_found("x")));
        let v: serde_json::Value = serde_json::from_slice(&err[5..]).unwrap();
        assert_eq!(v["error"]["code"], "not_found");
    }

    #[test]
    fn unwrap() {
        let b = envelope(0, b"hello");
        assert_eq!(&unwrap_envelope(&b).unwrap()[..], b"hello");
        assert!(unwrap_envelope(&Bytes::from_static(&[0, 0, 0, 0, 9, 1])).is_err());
        assert!(unwrap_envelope(&Bytes::from_static(&[1, 0, 0, 0, 0])).is_err());
    }

    #[test]
    fn status_mapping() {
        assert_eq!(Code::Unauthenticated.http_status(), 401);
        assert_eq!(Code::ResourceExhausted.http_status(), 429);
        assert_eq!(Code::Unimplemented.http_status(), 501);
    }
}
