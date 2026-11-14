//! Open DPhone DPMA client: the provisioning state machine and the
//! application-request transport, on `dpma-proto`.
//!
//! The stock firmware ran this as the `bootconfig` JavaScript app plus
//! native code in `core`; here it is one async task.
//!
//! ```ignore
//! let (transport, incoming) = dphone_dpma::sip::transport(&sip_handle, &mac);
//! let dpma = dphone_dpma::start(store.clone(), cfg, transport, incoming, storage, sink);
//! dpma.network(true, option_66, mdns_servers);        // from dphone-net
//! let reply = dpma.app_request("dasp://pbx/json", Some("100:1234"), doc).await?;
//! ```
//!
//! It owns the `Provisioning` state, handles
//! [`dphone_state::Domain::Provisioning`] commands, emits `Event::Push` for
//! server events, and heartbeats as [`dphone_state::Subsystem::Dpma`].

mod link;
mod machine;
#[cfg(feature = "sip")]
pub mod sip;
pub mod xml;

pub use dpma_proto::message::Message;
pub use link::{AppResponse, DpmaError};

use dphone_state::{DiscoveredServer, Store};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// Carries DPMA MESSAGEs.  `dphone-sip` implements this through
/// [`sip::transport`]; tests use an in-memory pair.
#[async_trait::async_trait]
pub trait Transport: Send + Sync + 'static {
    /// Sends one MESSAGE to the server (a SIP URI); `Ok` once the server
    /// accepted it at the SIP level.
    async fn send(&self, server_uri: &str, message: &Message) -> Result<(), String>;
}

/// Small durable files of ours (`/nvdata/open/` on the phone).
pub trait Storage: Send + Sync + 'static {
    fn read(&self, name: &str) -> Option<Vec<u8>>;
    fn write(&self, name: &str, data: &[u8]) -> Result<(), String>;
    fn remove(&self, name: &str);
}

/// The stock-format files this client reads and produces.  The integrator
/// implements it over `dphone-config`.
pub trait ConfigSink: Send + Sync + 'static {
    /// Current `/nvdata/configurator.xml`, if any.
    fn configurator(&self) -> Option<Vec<u8>>;
    /// Replaces `/nvdata/configurator.xml` (server, user, token).
    fn write_configurator(&self, xml: &[u8]) -> Result<(), String>;
    /// The cached digest in `/nvdata/md5s/config`.
    fn cached_md5(&self) -> Option<String>;
    /// A configuration was downloaded: write `fetched-config.xml` and the
    /// digest, merge, and apply.
    fn apply(&self, fetched_config: &[u8], md5: &str) -> Result<(), String>;
    /// Provisioning was forgotten: remove `fetched-config.xml`, the
    /// digest, and the configurator state, and merge again.
    fn forget(&self) -> Result<(), String>;
}

/// Names used with [`Storage`].
pub mod files {
    /// The phone identity minted on first use (PEM certificate).
    pub const IDENTITY_CERT: &str = "dpma-identity-cert.pem";
    /// Its RSA private key (PKCS#1 PEM).
    pub const IDENTITY_KEY: &str = "dpma-identity-key.pem";
}

#[derive(Clone, Debug)]
pub struct DpmaConfig {
    /// The phone's MAC address: twelve lowercase hex digits.
    pub mac: String,
    /// `Digium <model> <version>`; the server reads the model from it.
    pub user_agent: String,
    /// Act on firmware the server provisions.  Off by default: stock
    /// flashes on any version difference, which would put stock back.
    pub accept_firmware: bool,
    /// How long to wait for an application-level reply.
    pub timeout: Duration,
    /// Body bytes per MESSAGE.  UDP needs the whole MESSAGE under the MTU.
    pub chunk_len: usize,
    /// Seconds between liveness pings while attached (0: none).
    pub ping_secs: u64,
    /// Seconds before retrying after a failure.
    pub retry_secs: u64,
}

impl DpmaConfig {
    pub fn new(mac: impl Into<String>, user_agent: impl Into<String>) -> Self {
        DpmaConfig {
            mac: mac.into().to_ascii_lowercase().replace([':', '-'], ""),
            user_agent: user_agent.into(),
            accept_firmware: false,
            timeout: Duration::from_secs(15),
            chunk_len: 600,
            ping_secs: 300,
            retry_secs: 30,
        }
    }
}

pub(crate) enum Input {
    Network { up: bool, option66: Option<String>, mdns: Vec<DiscoveredServer> },
}

/// The running client.  Cheap to clone.
#[derive(Clone)]
pub struct DpmaHandle {
    link: Arc<link::Link>,
    input: mpsc::UnboundedSender<Input>,
}

impl DpmaHandle {
    /// Network state and discovery results from `dphone-net`: DHCP option
    /// 66 (used when it has the `sip:` form) and the `_digiumproxy._udp`
    /// servers found by mDNS.  Provisioning starts when the network first
    /// comes up.
    pub fn network(&self, up: bool, option66: Option<String>, mdns: Vec<DiscoveredServer>) {
        let _ = self.input.send(Input::Network { up, option66, mdns });
    }

    /// A request on behalf of an app, by URL: `file://NAME` is a
    /// FileRequest, `dasp://...` an AsteriskRequest, anything else an
    /// HTTPRequest through the server.  `credentials` is `user:secret`
    /// for the app account.
    pub async fn app_request(
        &self,
        url: &str,
        credentials: Option<&str>,
        document: &[u8],
    ) -> Result<AppResponse, DpmaError> {
        if let Some(name) = url.strip_prefix("file://") {
            return self.file(name).await.map(|body| AppResponse { status: 200, headers: Vec::new(), body });
        }
        let kind = if url.starts_with("dasp://") {
            dpma_proto::types::ASTERISK_REQUEST
        } else {
            dpma_proto::types::HTTP_REQUEST
        };
        self.link.app(kind, credentials, document).await
    }

    /// A resource by name (`FileRequest` for `file://NAME`).
    pub async fn file(&self, name: &str) -> Result<Vec<u8>, DpmaError> {
        self.link.file(name).await
    }

    /// Whether a session is up.
    pub fn attached(&self) -> bool {
        self.link.has_session()
    }
}

/// Starts the client on the current tokio runtime.  `incoming` carries
/// every DPMA MESSAGE the transport receives.
pub fn start(
    store: Store,
    cfg: DpmaConfig,
    transport: Arc<dyn Transport>,
    incoming: mpsc::UnboundedReceiver<Message>,
    storage: Arc<dyn Storage>,
    sink: Arc<dyn ConfigSink>,
) -> DpmaHandle {
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let link = Arc::new(link::Link::new(cfg.clone(), transport, events_tx));
    tokio::spawn(link::pump(link.clone(), incoming));
    let (input, input_rx) = mpsc::unbounded_channel();
    // Registered here, not in the task, so commands sent right after
    // `start` returns find their handler.
    let commands = store.handle(dphone_state::Domain::Provisioning);
    let m = machine::Machine::new(store, cfg, link.clone(), storage, sink);
    tokio::spawn(m.run(input_rx, events_rx, commands));
    DpmaHandle { link, input }
}
