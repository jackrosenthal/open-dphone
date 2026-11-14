//! The session with the server: request/response correlation, chunk
//! reassembly, and the application-request API.

use crate::{DpmaConfig, Message, Transport};
use dphone_state::clock;
use dpma_proto::client::Session;
use dpma_proto::message::{Incoming, Request, Sender, Transactions};
use dpma_proto::{documents, errors, framing, headers, types};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};

/// Why a DPMA request failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DpmaError {
    /// No session (not provisioned, or the server went away).
    NotAttached,
    /// The MESSAGE could not be delivered.
    Transport(String),
    /// No application-level reply in time.
    Timeout,
    /// The server answered with an envelope error (`-398` and friends).
    Server { code: i32, text: String },
    Protocol(String),
}

impl fmt::Display for DpmaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DpmaError::NotAttached => f.write_str("not attached to a DPMA server"),
            DpmaError::Transport(e) => write!(f, "transport: {e}"),
            DpmaError::Timeout => f.write_str("the server did not answer"),
            DpmaError::Server { code, text } => write!(f, "server error {code}: {text}"),
            DpmaError::Protocol(e) => write!(f, "protocol: {e}"),
        }
    }
}
impl std::error::Error for DpmaError {}

/// The reply to an app request, with the HTTP-style preamble removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

struct State {
    session: Session,
    server: String,
    waiting: HashMap<Vec<u8>, oneshot::Sender<Incoming>>,
    next: u64,
}

pub(crate) struct Link {
    cfg: DpmaConfig,
    transport: Arc<dyn Transport>,
    state: Mutex<State>,
    events: mpsc::UnboundedSender<Incoming>,
}

impl Link {
    pub fn new(cfg: DpmaConfig, transport: Arc<dyn Transport>, events: mpsc::UnboundedSender<Incoming>) -> Link {
        let session = Session::new(cfg.mac.clone().into_bytes());
        Link {
            cfg,
            transport,
            state: Mutex::new(State { session, server: String::new(), waiting: HashMap::new(), next: 0 }),
            events,
        }
    }

    pub fn set_server(&self, uri: &str) {
        self.state.lock().unwrap().server = uri.to_string();
    }

    pub fn has_session(&self) -> bool {
        self.state.lock().unwrap().session.session_id().is_some()
    }

    pub fn forget(&self) {
        self.state.lock().unwrap().session.forget();
    }

    pub fn set_token(&self, token: Option<&[u8]>) {
        let mut st = self.state.lock().unwrap();
        st.session.set_config_token(token.unwrap_or_default().to_vec());
    }

    pub fn token(&self) -> Option<Vec<u8>> {
        self.state.lock().unwrap().session.config_token().map(<[u8]>::to_vec)
    }

    pub fn accept_handshake(&self, resp: &Incoming, key: &rsa::RsaPrivateKey) -> Result<(), DpmaError> {
        self.state
            .lock()
            .unwrap()
            .session
            .accept_handshake_response(resp, key)
            .map_err(|e| DpmaError::Protocol(e.message().to_string()))
    }

    /// Sends the Handshake with `certificates` and waits for its response.
    pub async fn handshake(&self, certificates: Vec<u8>) -> Result<Incoming, DpmaError> {
        self.exchange(|s| s.handshake(certificates), |_| {}).await
    }

    /// Sends a request of `request_type` and waits for its response.
    /// `customize` can set the user name and secrets.
    pub async fn request(
        &self,
        request_type: &str,
        body: Vec<u8>,
        customize: impl FnOnce(&mut Request),
    ) -> Result<Incoming, DpmaError> {
        if types::needs_session(request_type) && !self.has_session() {
            return Err(DpmaError::NotAttached);
        }
        let resp = self.exchange(|s| s.request(request_type, body), customize).await?;
        if resp.error == errors::msg::SESSION_TIMEOUT {
            // The server no longer knows the session: tell the state
            // machine so it handshakes again.
            let _ = self.events.send(resp.clone());
        }
        Ok(resp)
    }

    async fn exchange(
        &self,
        build: impl FnOnce(&Session) -> Request,
        customize: impl FnOnce(&mut Request),
    ) -> Result<Incoming, DpmaError> {
        let (id, server, messages, rx) = {
            let mut st = self.state.lock().unwrap();
            if st.server.is_empty() {
                return Err(DpmaError::NotAttached);
            }
            st.next += 1;
            let id = format!("dp{}", st.next).into_bytes();
            let mut req = build(&st.session);
            req.id = id.clone();
            req.user_agent = self.cfg.user_agent.clone().into_bytes();
            customize(&mut req);
            let messages = st
                .session
                .encode(&req, self.cfg.chunk_len)
                .map_err(|e| DpmaError::Protocol(e.message().to_string()))?;
            let (tx, rx) = oneshot::channel();
            st.waiting.insert(id.clone(), tx);
            (id, st.server.clone(), messages, rx)
        };
        let result = async {
            for m in &messages {
                self.transport.send(&server, m).await.map_err(DpmaError::Transport)?;
            }
            match tokio::time::timeout(self.cfg.timeout, rx).await {
                Ok(Ok(resp)) => Ok(resp),
                Ok(Err(_)) => Err(DpmaError::Protocol("the reply was dropped".into())),
                Err(_) => Err(DpmaError::Timeout),
            }
        }
        .await;
        if result.is_err() {
            self.state.lock().unwrap().waiting.remove(&id);
        }
        result
    }

    /// An AsteriskRequest or HTTPRequest for an app.
    pub async fn app(&self, kind: &str, credentials: Option<&str>, document: &[u8]) -> Result<AppResponse, DpmaError> {
        let body = match credentials {
            Some(c) => documents::with_digest_auth(c, document),
            None => document.to_vec(),
        };
        let resp = self.request(kind, body, |_| {}).await?;
        if resp.error != 0 {
            return Err(DpmaError::Server { code: resp.error, text: errors::msg_error_text(resp.error).to_string() });
        }
        let header_status = resp
            .first
            .header(headers::HTTP_STATUS)
            .and_then(|s| String::from_utf8_lossy(s).split_whitespace().next().and_then(|c| c.parse::<u16>().ok()));
        Ok(split_http(&resp.body, header_status))
    }

    pub async fn file(&self, name: &str) -> Result<Vec<u8>, DpmaError> {
        let resp = self.request(types::FILE_REQUEST, documents::file_request(name), |_| {}).await?;
        if resp.error != 0 {
            return Err(DpmaError::Server { code: resp.error, text: errors::msg_error_text(resp.error).to_string() });
        }
        Ok(body_of(&resp))
    }
}

/// The body, with the server's "nothing to send" marker as empty.
pub(crate) fn body_of(resp: &Incoming) -> Vec<u8> {
    if resp.body == framing::EMPTY_BODY.as_bytes() {
        Vec::new()
    } else {
        resp.body.clone()
    }
}

/// Splits an optional `HTTP/1.x <status>` preamble and its headers from
/// the document that follows.
pub(crate) fn split_http(body: &[u8], header_status: Option<u16>) -> AppResponse {
    if body == framing::EMPTY_BODY.as_bytes() {
        return AppResponse { status: header_status.unwrap_or(200), headers: Vec::new(), body: Vec::new() };
    }
    if !body.starts_with(b"HTTP/") {
        return AppResponse { status: header_status.unwrap_or(200), headers: Vec::new(), body: body.to_vec() };
    }
    let (head_end, sep) = match (find(body, b"\r\n\r\n"), find(body, b"\n\n")) {
        (Some(a), Some(b)) if b < a => (b, 2),
        (Some(a), _) => (a, 4),
        (None, Some(b)) => (b, 2),
        (None, None) => (body.len(), 0),
    };
    let head = String::from_utf8_lossy(&body[..head_end]);
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .or(header_status)
        .unwrap_or(200);
    let headers = lines
        .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
        .collect();
    AppResponse { status, headers, body: body[(head_end + sep).min(body.len())..].to_vec() }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Reassembles incoming MESSAGEs into transactions and hands each whole
/// one to whoever waits for its ID, else to the state machine as an event.
pub(crate) async fn pump(link: Arc<Link>, mut incoming: mpsc::UnboundedReceiver<Message>) {
    let mut transactions: Transactions<()> = Transactions::new(Sender::Server);
    while let Some(message) = incoming.recv().await {
        let now = (clock::mono_ms() / 1000) as i64;
        // The server gives up on partial transactions after 40 seconds.
        transactions.prune(now, 40);
        let last = message.clone();
        let Some(((), parts)) = transactions.add(message, (), now, ()) else { continue };
        let decoded = {
            let st = link.state.lock().unwrap();
            parts.decode(Sender::Server, st.session.common_password())
        };
        let msg = match decoded {
            Ok(m) => m,
            // A server that no longer knows our session answers -398
            // encrypted under a session we do not share.  The envelope
            // headers are in the clear, and they are all that matters.
            Err(e) => match envelope_error(&last) {
                Some(m) => m,
                None => {
                    log::warn!("dpma: dropped an undecodable message: {}", e.message());
                    continue;
                }
            },
        };
        let waiter = link.state.lock().unwrap().waiting.remove(&msg.id);
        match waiter {
            Some(tx) => {
                let _ = tx.send(msg);
            }
            None => {
                let _ = link.events.send(msg);
            }
        }
    }
}

/// An [`Incoming`] made from the plaintext headers of an error reply
/// whose body cannot be decrypted.
fn envelope_error(m: &Message) -> Option<Incoming> {
    let error = m.header(headers::ERROR_ID).map(framing::parse_sequence).filter(|e| *e != 0)?;
    let get = |name: &str| m.header(name).map(<[u8]>::to_vec).unwrap_or_default();
    let mut first = m.clone();
    first.body.clear();
    Some(Incoming {
        id: get(headers::APPSERVER_ID),
        request_type: get(headers::REQUEST_TYPE),
        response_type: get(headers::RESPONSE_TYPE),
        session_id: get(headers::SESSION),
        mac: get(headers::MAC_ADDRESS),
        username: Vec::new(),
        response_uri: get(headers::RESPONSE_URI),
        full_contact: get(headers::RESPONSE_FULLCONTACT),
        origination_network: get(headers::ORIGINATION_NETWORK),
        handshake_version: Vec::new(),
        error,
        password: Vec::new(),
        config_password: Vec::new(),
        config_token: Vec::new(),
        encrypted: true,
        body: Vec::new(),
        payload: Vec::new(),
        first,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_preamble() {
        let r = split_http(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"response\":{}}", None);
        assert_eq!(r.status, 200);
        assert_eq!(r.headers, vec![("Content-Type".to_string(), "application/json".to_string())]);
        assert_eq!(r.body, b"{\"response\":{}}");
        let r = split_http(b"HTTP/1.0 404 Not Found\n\n", None);
        assert_eq!((r.status, r.body.len()), (404, 0));
        let r = split_http(b"{\"x\":1}", Some(500));
        assert_eq!((r.status, r.body.as_slice()), (500, &b"{\"x\":1}"[..]));
        assert!(split_http(b"empty body", None).body.is_empty());
    }
}
