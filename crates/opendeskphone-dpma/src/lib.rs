//! Open Desk Phone DPMA client: the provisioning state machine and the
//! application-request transport, on `dpma-proto`.
//!
//! The stock firmware ran this as the `bootconfig` JavaScript app plus
//! native code in `core`; here it is one async task.
//!
//! ```ignore
//! let (transport, incoming) = opendeskphone_dpma::sip::transport(&sip_handle,
//! &mac); let dpma = opendeskphone_dpma::start(store.clone(), cfg, transport,
//! incoming, files, sink); dpma.network(true, option_66, mdns_servers);
//! // from opendeskphone-net let reply = dpma.app_request("dasp://pbx/json",
//! Some("100:1234"), doc).await?;
//! ```
//!
//! Firmware the server provisions is installed when it differs from the
//! running image (see [`firmware`]).  The files a configuration names
//! (contact lists, ringtones, the BLF items file) are fetched after it is
//! applied (see [`resources`]).
//!
//! It owns the `Provisioning` state, handles
//! [`opendeskphone_state::Domain::Provisioning`] commands, emits `Event::Push`
//! for server events, and heartbeats as
//! [`opendeskphone_state::Subsystem::Dpma`].

pub mod fetch;
pub mod firmware;
pub mod resources;
#[cfg(feature = "update")]
pub mod install;
mod link;
mod machine;
#[cfg(feature = "sip")]
pub mod sip;
pub mod xml;

pub use dpma_proto::appdata;
pub use dpma_proto::message::Message;
pub use link::{AppResponse, DpmaError};

use opendeskphone_state::{DiscoveredServer, Store};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// Carries DPMA MESSAGEs.  `opendeskphone-sip` implements this through
/// [`sip::transport`]; tests use an in-memory pair.
#[async_trait::async_trait]
pub trait Transport: Send + Sync + 'static {
    /// Sends one MESSAGE to the server (a SIP URI); `Ok` once the server
    /// accepted it at the SIP level.
    async fn send(&self, server_uri: &str, message: &Message) -> Result<(), String>;
}


/// The stock-format files this client reads and produces.  The integrator
/// implements it over `opendeskphone-config`.
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
    /// The saved `fetched-config.xml`, for the firmware check on boots
    /// where the configuration is not downloaded again.
    fn fetched_config(&self) -> Option<Vec<u8>> {
        None
    }
    /// Whether provisioned firmware is to be ignored once this fetched
    /// configuration is in effect: the setting can come with the very
    /// configuration that offers the firmware.  `None` leaves it to
    /// [`DpmaHandle::set_ignore_provisioned_firmware`].
    fn ignore_provisioned_firmware(&self, _fetched_config: &[u8]) -> Option<bool> {
        None
    }
    /// The files the configuration in effect names, and the stock app
    /// packages it pushes.
    fn wanted_resources(&self) -> resources::Wanted {
        resources::Wanted::default()
    }
    /// Where those files are kept.  `None`: resources are not fetched.
    fn files(&self) -> Option<Arc<dyn resources::Files>> {
        None
    }
    /// A resource pass changed files on disk (contact lists, BLF items,
    /// ringtones): whoever reads them should read them again.
    fn resources_changed(&self) {}
    /// Provisioning was forgotten: remove `fetched-config.xml`, the
    /// digest, and the configurator state, and merge again.
    fn forget(&self) -> Result<(), String>;
}

/// The files the client keeps (in `/nvdata/opendeskphone/` on the phone).
#[derive(Clone, Debug)]
pub struct ClientFiles {
    /// The phone identity minted on first use (PEM certificate).
    pub identity_cert: PathBuf,
    /// Its RSA private key (PKCS#1 PEM), readable by its owner alone.
    pub identity_key: PathBuf,
    /// The last firmware install the server asked for
    /// ([`firmware::Attempt`]).
    pub firmware_attempt: PathBuf,
}

impl ClientFiles {
    /// The files in `dir`, under their usual names.
    pub fn in_dir(dir: &Path) -> ClientFiles {
        ClientFiles {
            identity_cert: dir.join("dpma-identity-cert.pem"),
            identity_key: dir.join("dpma-identity-key.pem"),
            firmware_attempt: dir.join("dpma-firmware-attempt"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DpmaConfig {
    /// The phone's MAC address: twelve lowercase hex digits.
    pub mac: String,
    /// `Digium <model> <version>`; the server reads the model from it.
    pub user_agent: String,
    /// Leave the firmware the server provisions alone: its entries are
    /// dropped from the fetched configuration and nothing is installed.
    /// For development.  Off by default: a provisioned phone runs the
    /// firmware its server names.  This is the value at start; afterward
    /// the client follows [`DpmaHandle::set_ignore_provisioned_firmware`]
    /// and [`ConfigSink::ignore_provisioned_firmware`].
    pub ignore_provisioned_firmware: bool,
    /// Send the MAC in the firmware download's User-Agent (stock
    /// `send_mac_with_useragent`).
    pub mac_in_user_agent: bool,
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
            ignore_provisioned_firmware: false,
            mac_in_user_agent: false,
            timeout: Duration::from_secs(15),
            chunk_len: 600,
            ping_secs: 300,
            retry_secs: 30,
        }
    }
}

pub(crate) type FirmwareParts = (Arc<dyn firmware::FirmwareSource>, Arc<dyn firmware::FirmwareInstaller>);

/// What the firmware check needs that is set after [`start`].
pub(crate) struct FirmwareCtl {
    pub ignore: std::sync::atomic::AtomicBool,
    pub parts: std::sync::Mutex<Option<FirmwareParts>>,
    pub running: Arc<std::sync::atomic::AtomicBool>,
    /// Where the files a configuration names are downloaded from when
    /// their URL is `http://` or `https://`.
    pub http: std::sync::Mutex<Option<Arc<dyn firmware::FirmwareSource>>>,
}

pub(crate) enum Input {
    Network { up: bool, option66: Option<String>, mdns: Vec<DiscoveredServer> },
}

/// The running client.  Cheap to clone.
#[derive(Clone)]
pub struct DpmaHandle {
    link: Arc<link::Link>,
    input: mpsc::UnboundedSender<Input>,
    firmware: Arc<FirmwareCtl>,
}

impl DpmaHandle {
    /// Network state and discovery results from `opendeskphone-net`: DHCP
    /// option 66 (used when it has the `sip:` form) and the
    /// `_digiumproxy._udp` servers found by mDNS.  Provisioning starts when
    /// the network first comes up.
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

    /// One operation on the server's app data: the store of keys and
    /// values that the phones' apps share through the server
    /// (`dpma_proto::appdata`).  The server takes the user, and the tenant,
    /// from the phone's configuration token.  A server without app data
    /// (the official DPMA) does not answer: [`DpmaError::Timeout`].
    pub async fn app_data(&self, op: &appdata::Op) -> Result<appdata::Response, DpmaError> {
        self.link.app_data(op).await
    }

    /// A resource by name (`FileRequest` for `file://NAME`).
    pub async fn file(&self, name: &str) -> Result<Vec<u8>, DpmaError> {
        self.link.file(name).await
    }

    /// Gives the client the means to act on provisioned firmware: where
    /// images come from and what writes them.  Until this is called (the
    /// simulator never does), provisioned firmware is only reported.
    pub fn set_firmware_updater(
        &self,
        source: Arc<dyn firmware::FirmwareSource>,
        installer: Arc<dyn firmware::FirmwareInstaller>,
    ) {
        *self.firmware.parts.lock().unwrap() = Some((source, installer));
    }

    /// Gives the client the HTTP client for the files a configuration
    /// names by `http://` or `https://` URL (contact lists, ringtones, the
    /// BLF items file).  Without it, [`fetch::HttpSource`] with the
    /// system's resolver is used.
    pub fn set_http_source(&self, source: Arc<dyn firmware::FirmwareSource>) {
        *self.firmware.http.lock().unwrap() = Some(source);
    }

    /// Turns "ignore firmware provisioned by the server" on or off: the
    /// integrator calls this whenever the setting in effect changes.  A
    /// firmware check that has not started yet sees the new value; the
    /// entries of a configuration already saved are dropped or restored at
    /// the next fetch.
    pub fn set_ignore_provisioned_firmware(&self, ignore: bool) {
        self.firmware.ignore.store(ignore, std::sync::atomic::Ordering::SeqCst);
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
    files: ClientFiles,
    sink: Arc<dyn ConfigSink>,
) -> DpmaHandle {
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let link = Arc::new(link::Link::new(cfg.clone(), transport, events_tx));
    tokio::spawn(link::pump(link.clone(), incoming));
    let (input, input_rx) = mpsc::unbounded_channel();
    // Registered here, not in the task, so commands sent right after
    // `start` returns find their handler.
    let commands = store.handle(opendeskphone_state::Domain::Provisioning);
    let firmware = Arc::new(FirmwareCtl {
        ignore: std::sync::atomic::AtomicBool::new(cfg.ignore_provisioned_firmware),
        parts: std::sync::Mutex::new(None),
        running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        http: std::sync::Mutex::new(None),
    });
    let m = machine::Machine::new(store, cfg, link.clone(), files, sink, firmware.clone());
    tokio::spawn(m.run(input_rx, events_rx, commands));
    DpmaHandle { link, input, firmware }
}
