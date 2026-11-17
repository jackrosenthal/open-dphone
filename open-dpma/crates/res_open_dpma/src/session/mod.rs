//! Phone sessions and messaging.
//!
//! A *session* is created by a successful Handshake: it remembers the
//! common password that encrypts everything after the handshake.  A
//! *transaction* is one complete incoming request (possibly reassembled
//! from several SIP MESSAGEs); handlers answer it by filling in its
//! [`Outgoing`] and calling [`outgoing::send_and_destroy`].

pub mod ami;
pub mod cli;
pub mod handshake;
pub mod incoming;
pub mod outgoing;
pub mod persist;
pub mod store;

pub use incoming::next_transaction;
pub use outgoing::{
    send, send_and_destroy, send_event, send_handshake_required, send_maybe, send_timeout,
};
pub use store::{
    all, find, find_by_mac, global_auth_flags, invalidate, load, mark_authenticated, mark_inactive,
    mark_unconfigured_by_mac, reload, set_global_auth_flags, unload,
};

use dpma_ast::msg::Msg;
use dpma_proto::cert::{Certificate, PublicKey};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};

/// AstDB family holding persisted sessions.
pub const DB_FAMILY: &str = "DigiumPhoneSessions";

/// Mutable session state.  Empty strings stand for the official DPMA's
/// NULL pointers.
#[derive(Default)]
pub struct SessionState {
    /// Contact URI from the handshake, without brackets.
    pub contact: Vec<u8>,
    /// MAC address; empty when the handshake had none.
    pub mac: Vec<u8>,
    /// User agent.
    pub user_agent: Vec<u8>,
    /// The certificate PEM the phone sent in its Handshake body, stored in
    /// AstDB and verified again on restore.
    pub client_cert_pem: Vec<u8>,
    /// The verified phone certificate.
    pub certificate: Option<Certificate>,
    /// Its public key; None when it is an RSA key that does not decode.
    pub public_key: Option<PublicKey>,
    /// Intermediate certificate serial, 0 for legacy certificates.
    pub intermediate_serial: i64,
    /// Authentication flags (`config::auth`) in force for this session.
    pub auth_flags: u32,
    /// The phone authenticated.
    pub authenticated: bool,
    /// The session is inactive.
    pub inactive: bool,
    /// A ConfigRequest was answered.
    pub configured: bool,
    /// Creation time, seconds since the epoch.
    pub created: i64,
    /// Last activity, seconds since the epoch: set when the user
    /// agent is first learned and after every successful send.
    pub last_activity: i64,
    /// Configuration password (never set by 3.6.13).
    pub config_password: Vec<u8>,
    /// Where to send messages for this phone.
    pub response_uri: Vec<u8>,
    /// `X-Origination-Network`.
    pub origination_network: Vec<u8>,
    /// The 40-byte common password.
    pub common_password: Vec<u8>,
}

/// A phone session.  Looked up by `id` (`X-Digium-AppServer-Session`).
/// The state's lock is held for the whole of an outgoing send, as the
/// official DPMA holds the session's ao2 lock.
pub struct Session {
    /// Session id: `"%d%ld"` of a djb2 hash of time+contact and
    /// `ast_random()`, unless restored.
    pub id: Vec<u8>,
    pub state: Mutex<SessionState>,
}

impl Session {
    pub fn new(id: &[u8]) -> Session {
        Session {
            id: id.to_vec(),
            state: Mutex::new(SessionState::default()),
        }
    }

    /// Lock the state (a poisoned lock is still usable).
    pub fn lock(&self) -> MutexGuard<'_, SessionState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The authentication flags in force for this session.
    pub fn auth_flags(&self) -> u32 {
        self.lock().auth_flags
    }

    /// Where to send messages for this phone.
    pub fn response_uri(&self) -> Vec<u8> {
        self.lock().response_uri.clone()
    }

    /// The configuration password.
    pub fn config_password(&self) -> Vec<u8> {
        self.lock().config_password.clone()
    }

    /// The session's MAC address ("" when none).
    pub fn mac(&self) -> Vec<u8> {
        self.lock().mac.clone()
    }
}

pub use dpma_proto::message::Outgoing;

/// One complete incoming request.  Immutable once built, except for
/// `outgoing`, `configured`, and `unconfigured`.
#[derive(Default)]
pub struct Transaction {
    /// Container key: `"<X-Digium-AppServer-ID>-<MAC or unknown>"`.
    pub key: Vec<u8>,
    /// `X-Digium-AppServer-ID`.
    pub app_server_id: Vec<u8>,
    /// `X-Digium-AppServer-RequestType`.
    pub request_type: Vec<u8>,
    /// `X-Digium-AppServer-ResponseType`.
    pub response_type: Vec<u8>,
    /// `X-Digium-AppServer-Response-URI` plus `;transport=<PJSIP_TRANSPORT>`
    /// when that variable exists, cut to 78 bytes.
    pub response_uri: Vec<u8>,
    /// `X-Digium-AppServer-Response-FullContact` (Asterisk 24 never sets
    /// PJSIP_FULLCONTACT, so this is empty).
    pub full_contact: Vec<u8>,
    /// `X-Digium-AppServer-Username`.
    pub username: Vec<u8>,
    /// `X-Digium-AppServer-MACAddress`.
    pub mac: Vec<u8>,
    /// The user agent (`X-Digium-User-Agent`, else `User-Agent`).  Not a
    /// field of the official DPMA's transaction; handlers there read it
    /// from the message.
    pub user_agent: Vec<u8>,
    /// Decrypted `X-Digium-AppServer-Password`.
    pub password: Vec<u8>,
    /// Decrypted `X-Digium-AppServer-ConfigPassword`.
    pub config_password: Vec<u8>,
    /// Decrypted `X-Digium-AppServer-Config-Token`.
    pub config_token: Vec<u8>,
    /// Host of the `To` URI (see `dpma_proto::uri::to_host_port`).
    pub to_host: Vec<u8>,
    /// Port of the `To` URI.
    pub to_port: Vec<u8>,
    /// Phone model from the user agent.
    pub model: Vec<u8>,
    /// `X-Origination-Network` of the first message (not a field of the
    /// official DPMA's transaction).
    pub origination_network: Vec<u8>,
    /// Reassembled (and decrypted) body.
    pub body: Vec<u8>,
    /// The first message of the transaction, for any other variable.
    pub first_msg: Option<Msg>,
    /// The session the transaction is for.
    pub session: Option<Arc<Session>>,
    /// The reply being built; allocated by
    /// `request::handle_incoming`.
    pub outgoing: Mutex<Option<Outgoing>>,
    /// Set by the ConfigRequest (and token) handlers: the session
    /// becomes configured when the reply is sent.
    pub configured: AtomicBool,
    /// Set by the UserlistRequest handler: the session becomes
    /// unconfigured when the reply is sent.
    pub unconfigured: AtomicBool,
}

impl Transaction {
    /// The value of message variable `name` from the first message.
    pub fn var(&self, name: &str) -> Option<Vec<u8>> {
        self.first_msg.as_ref().and_then(|m| m.var(name))
    }
}

/// Seconds since the epoch (`time(NULL)`).
pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The user agent of a message: `X-Digium-User-Agent` when present and
/// not empty, else `User-Agent` (possibly absent).
pub(crate) fn user_agent_of(m: &Msg) -> Option<Vec<u8>> {
    match m.get_var(dpma_proto::headers::DIGIUM_USER_AGENT) {
        Some(ua) if !ua.is_empty() => Some(ua),
        _ => m.get_var(dpma_proto::headers::USER_AGENT),
    }
}

/// `%s` of a C string that may be NULL.
pub(crate) fn or_null(s: Option<&[u8]>) -> &[u8] {
    s.unwrap_or(b"(null)")
}

/// Test hooks (feature "devtest").
#[cfg(feature = "devtest")]
pub fn devtest_commands() -> Vec<dpma_ast::cli::CliCommand> {
    store::devtest_commands()
}
