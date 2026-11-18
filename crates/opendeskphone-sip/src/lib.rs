//! Open Desk Phone SIP user agent.
//!
//! Built on `rsipstack`: its transports, transactions, and dialog layer
//! (`Registration`, INVITE dialogs, client subscription dialogs) do the
//! signaling.  This crate adds what a phone needs around them: failover
//! between registrars, SDP offer/answer, the NOTIFY bodies, transfer and
//! hold policy, out-of-dialog MESSAGE, and the state store.
//!
//! # Running it
//!
//! ```ignore
//! let handle = opendeskphone_sip::start(store.clone(), config, media,
//! None).await?; // ... later, when the merged configuration or the address
//! changes: handle.reconfigure(new_config);
//! ```
//!
//! [`start`] binds the signaling socket, registers as the handler of
//! [`opendeskphone_state::Domain::Calls`], and spawns its tasks on the current
//! tokio runtime (a `current_thread` runtime is enough).
//!
//! # State it owns
//!
//! `Accounts`, `Calls`, `Presence`, `Mwi`, `CallLog`, `Features`, and in
//! `Keys` the line keys, rapid-dial keys, and the message lamp.  It emits
//! `IncomingCall`, `CallEnded`, `DtmfReceived`, and `Message` events and
//! heartbeats as [`opendeskphone_state::Subsystem::Sip`].

// Nested `if let` chains read better than `&&`-joined let-chains in the
// long protocol handlers here.
#![allow(clippy::collapsible_if, clippy::large_enum_variant, clippy::type_complexity)]

pub mod bodies;
mod calls;
pub mod config;
pub mod keys;
mod core;
pub mod media;
mod reg;
pub mod sdp;
pub mod timer;
#[cfg(feature = "tls")]
pub mod tls;
pub mod util;

pub use config::{AccountConfig, ActivityLamps, BlfConfig, BlfLocation, CodecPref, EventAction, HostConfig, SipConfig, TlsConfig};
pub use media::{MediaControl, MediaEvent, NullMedia, RingRequest, StreamParams, StreamSrtp};

use crate::core::{Core, Ev, Timer};
use opendeskphone_state::{AccountId, Domain, Features, Store, Transport};
use rsipstack::transport::udp::UdpConnection;
use rsipstack::transport::SipAddr;
use rsipstack::EndpointBuilder;
use rsipstack::dialog::dialog_layer::DialogLayer;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

/// An out-of-dialog MESSAGE to send (the DPMA transport).
#[derive(Clone, Debug, Default)]
pub struct OutMessage {
    /// Send as this account (its address and credentials).  `None` sends
    /// from the phone's own address.
    pub account: Option<AccountId>,
    /// Destination URI, e.g. `sip:proxy@pbx.example:5060;transport=tcp`.
    pub to: String,
    /// User part of From and Contact at the phone's own address; DPMA uses
    /// the MAC address.  Overrides the account's address.
    pub from_user: Option<String>,
    pub headers: Vec<(String, String)>,
    /// Empty means `text/plain`.
    pub content_type: String,
    pub body: Vec<u8>,
}

/// A MESSAGE that arrived carrying application (DPMA) headers.
#[derive(Clone, Debug)]
pub struct IncomingMessage {
    pub account: Option<AccountId>,
    pub from_uri: String,
    pub contact_uri: String,
    pub headers: Vec<(String, String)>,
    pub content_type: String,
    pub body: Vec<u8>,
}

/// Applies the network's QoS marking (DSCP, socket priority) to a SIP
/// socket.  The integrator supplies it from the network subsystem.
pub type SocketMarker = Arc<dyn Fn(std::os::fd::BorrowedFd<'_>) + Send + Sync>;

static MARKER: std::sync::Mutex<Option<SocketMarker>> = std::sync::Mutex::new(None);

/// Sets how SIP sockets are marked.  It applies to sockets made from now
/// on, and to the UDP socket again at every [`SipHandle::reconfigure`].
///
/// Marked: the UDP socket; TCP connections this crate opens toward a
/// registrar; TLS connections in both directions.  Not marked: TCP
/// connections the stack accepts or opens by itself (it creates those
/// sockets internally and offers no hook), which in practice means
/// accepted TCP connections and TCP to targets other than a registrar.
pub fn set_socket_marker(marker: SocketMarker) {
    *MARKER.lock().unwrap() = Some(marker);
}

pub(crate) fn mark(socket: &impl std::os::fd::AsFd) {
    let marker = MARKER.lock().unwrap().clone();
    if let Some(m) = marker {
        m(socket.as_fd());
    }
}

/// Name resolution for signaling targets.  The integrator implements this over
/// `opendeskphone-net`'s resolver (which can follow NAPTR/SRV); without one
/// the system resolver is used.
#[async_trait::async_trait]
pub trait Resolver: Send + Sync + 'static {
    /// The address to use for `host`.  `port` is what the configuration or
    /// URI said (the transport default when it said nothing).
    async fn resolve(&self, host: &str, port: u16, transport: Transport) -> Result<SocketAddr, String>;
}

struct SystemResolver;

#[async_trait::async_trait]
impl Resolver for SystemResolver {
    async fn resolve(&self, host: &str, port: u16, _transport: Transport) -> Result<SocketAddr, String> {
        tokio::net::lookup_host((host, port))
            .await
            .map_err(|e| format!("cannot resolve {host}: {e}"))?
            .next()
            .ok_or_else(|| format!("{host} has no address"))
    }
}

struct StackResolver(Arc<dyn Resolver>);

#[async_trait::async_trait]
impl rsipstack::transport::transport_layer::DomainResolver for StackResolver {
    async fn resolve(&self, target: &SipAddr) -> rsipstack::Result<SipAddr> {
        let transport = match target.r#type {
            Some(rsipstack::sip::Transport::Tcp) => Transport::Tcp,
            Some(rsipstack::sip::Transport::Tls) => Transport::Tls,
            _ => Transport::Udp,
        };
        let default = if transport == Transport::Tls { 5061 } else { 5060 };
        let port = target.addr.port.as_ref().map(|p| u16::from(*p)).unwrap_or(default);
        let host = target.addr.host.to_string();
        let addr = self.0.resolve(&host, port, transport).await.map_err(rsipstack::Error::Error)?;
        Ok(SipAddr { r#type: target.r#type, addr: addr.into() })
    }
}

/// The running service.  Cheap to clone.
#[derive(Clone)]
pub struct SipHandle {
    tx: mpsc::UnboundedSender<Ev>,
    local_port: u16,
}

impl SipHandle {
    /// The signaling port in use.
    pub fn local_port(&self) -> u16 {
        self.local_port
    }

    /// Applies a new configuration (accounts, codecs, BLF list, address).
    /// Accounts that did not change keep their registrations.
    pub fn reconfigure(&self, cfg: SipConfig) {
        let _ = self.tx.send(Ev::Reconfigure(Box::new(cfg)));
    }

    /// Feeds a report from the media engine.
    pub fn media_event(&self, ev: MediaEvent) {
        let _ = self.tx.send(Ev::Media(ev));
    }

    /// Sends an out-of-dialog MESSAGE and waits for its final status code.
    pub async fn send_message(&self, msg: OutMessage) -> Result<u16, String> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(Ev::SendMessage { msg, reply }).map_err(|_| "the SIP service has stopped".to_string())?;
        rx.await.map_err(|_| "the MESSAGE could not be sent".to_string())?
    }

    /// Incoming application MESSAGEs (those with `X-Digium-AppServer-*` or
    /// `X-Event-Name` headers).  Every subscriber gets every message.
    pub fn messages(&self) -> mpsc::UnboundedReceiver<IncomingMessage> {
        let (tx, rx) = mpsc::unbounded_channel();
        let _ = self.tx.send(Ev::Subscribe(tx));
        rx
    }

    /// Ends calls, un-registers, and stops.
    pub async fn shutdown(&self) {
        let (tx, rx) = oneshot::channel();
        if self.tx.send(Ev::Shutdown(tx)).is_ok() {
            let _ = rx.await;
        }
    }
}

/// Starts the SIP service on the current tokio runtime.
pub async fn start(
    store: Store,
    cfg: SipConfig,
    media: Arc<dyn MediaControl>,
    resolver: Option<Arc<dyn Resolver>>,
) -> Result<SipHandle, String> {
    #[cfg(feature = "tls")]
    tls::install(cfg.tls.clone());

    let resolver = resolver.unwrap_or_else(|| Arc::new(SystemResolver));
    let elsewhere = Arc::new(std::sync::Mutex::new(Vec::new()));
    let streams = Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
    let endpoint = EndpointBuilder::new()
        .with_inspector(Box::new(crate::core::CancelWatch(elsewhere.clone())))
        .with_transport_inspector(Box::new(crate::core::StreamWatch(streams.clone())))
        .with_user_agent(&cfg.user_agent())
        .with_domain_resolver(Box::new(StackResolver(resolver.clone())))
        .build();
    let any: IpAddr = match cfg.local_ip {
        IpAddr::V4(_) => Ipv4Addr::UNSPECIFIED.into(),
        IpAddr::V6(_) => Ipv6Addr::UNSPECIFIED.into(),
    };
    // Loopback configurations (tests, the simulator) bind loopback so two
    // instances on one machine stay apart.
    let bind_ip = if cfg.local_ip.is_loopback() { cfg.local_ip } else { any };
    let external = Some(SocketAddr::new(cfg.local_ip, cfg.local_port));
    // The socket is made here, not by the stack, so it can be marked.
    let socket = tokio::net::UdpSocket::bind(SocketAddr::new(bind_ip, cfg.local_port))
        .await
        .map_err(|e| format!("cannot bind the SIP port {}: {e}", cfg.local_port))?;
    mark(&socket);
    let socket = Arc::new(socket);
    let udp = UdpConnection::from_socket(socket.clone(), if cfg.local_port != 0 { external } else { None }, None)
        .await
        .map_err(|e| format!("cannot use the SIP socket: {e}"))?;
    let local_port = socket.local_addr().map(|a| a.port()).unwrap_or(cfg.local_port);
    endpoint.inner.transport_layer.add_transport(udp.into());
    let mut incoming = endpoint.incoming_transactions().map_err(|e| e.to_string())?;
    let inner = endpoint.inner.clone();
    {
        let inner = inner.clone();
        tokio::spawn(async move {
            let _ = inner.serve().await;
        });
    }

    let (tx, mut rx) = mpsc::unbounded_channel::<Ev>();
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            while let Some(t) = incoming.recv().await {
                if tx.send(Ev::Incoming(t)).is_err() {
                    break;
                }
            }
        });
    }
    let commands = store.handle(Domain::Calls);
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            while let Some(env) = commands.next().await {
                if tx.send(Ev::Cmd(env)).is_err() {
                    break;
                }
            }
        });
    }
    store.update::<Features, _>(|f| f.call_waiting = cfg.call_waiting);

    let layer = Arc::new(DialogLayer::new(inner.clone()));
    let (dialog_tx, mut dialog_rx) = layer.new_dialog_state_channel();
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            while let Some(state) = dialog_rx.recv().await {
                if tx.send(Ev::Dialog(state)).is_err() {
                    break;
                }
            }
        });
    }
    let mut core = Core {
        store,
        cfg: SipConfig::new(cfg.model, cfg.local_ip),
        media,
        resolver,
        inner,
        layer,
        dialog_tx,
        tx: tx.clone(),
        local_port,
        accts: Vec::new(),
        calls: Vec::new(),
        subs: Vec::new(),
        next_sub: 0,
        conference: Vec::new(),
        last_dialed: None,
        listeners: Vec::new(),
        park_extension: cfg.park_extension.clone(),
        page_changes: 0,
        elsewhere,
        streams,
        udp: socket,
        tcp_listening: None,
        tls_listening: None,
    };
    core.configure(cfg);
    core.heartbeat();
    core.after(Duration::from_secs(5), Timer::Heartbeat);

    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            if let Ev::Shutdown(done) = ev {
                core.shutdown().await;
                endpoint.shutdown();
                let _ = done.send(());
                break;
            }
            core.handle(ev);
        }
    });
    Ok(SipHandle { tx, local_port })
}

impl Core {
    fn handle(&mut self, ev: Ev) {
        match ev {
            Ev::Cmd(env) => self.on_command(env),
            Ev::Incoming(tx) => self.on_incoming(tx),
            Ev::Dialog(state) => self.on_dialog(state),
            Ev::InviteDone { call, result } => self.on_invite_done(call, result),
            Ev::ReinviteDone { call, hold, result } => self.on_reinvite_done(call, hold, result),
            Ev::ReferDone { call, result } => self.on_refer_done(call, result),
            Ev::RefreshDone { call, generation, result } => self.on_refresh_done(call, generation, result),
            Ev::AckBody { call, body } => self.on_ack_body(call, &body),
            Ev::Registered { acct, generation, outcome } => self.on_registered(acct, generation, outcome),
            Ev::Subscribed { sub, generation, result } => self.on_subscribed(sub, generation, result),
            Ev::Timer(t) => match t {
                Timer::Register { acct, generation } => self.on_register_timer(acct, generation),
                Timer::Keepalive { acct, generation } => self.on_keepalive_timer(acct, generation),
                Timer::Subscribe { sub, generation } => self.on_subscribe_timer(sub, generation),
                Timer::NoAnswer { call } => self.on_no_answer(call),
                Timer::Linger { call } => self.on_linger(call),
                Timer::PageReturn { generation } => {
                    if generation == self.page_changes {
                        self.store.update::<opendeskphone_state::Keys, _>(|k| k.rapid_page = 0);
                        self.sync_keys();
                    }
                }
                Timer::HoldRetry { call } => self.on_hold_retry(call),
                Timer::SessionRefresh { call, generation } => self.on_session_refresh(call, generation),
                Timer::SessionExpire { call, generation } => self.on_session_expire(call, generation),
                Timer::Heartbeat => {
                    self.heartbeat();
                    self.after(Duration::from_secs(5), Timer::Heartbeat);
                }
            },
            Ev::Media(m) => self.on_media(m),
            Ev::Reconfigure(cfg) => {
                // The marks in force may have changed with the network.
                mark(&*self.udp);
                #[cfg(feature = "tls")]
                tls::install(cfg.tls.clone());
                self.park_extension = cfg.park_extension.clone();
                self.store.update::<Features, _>(|f| f.call_waiting = cfg.call_waiting);
                self.configure(*cfg);
            }
            Ev::SendMessage { msg, reply } => self.send_message(msg, reply),
            Ev::Subscribe(tx) => self.listeners.push(tx),
            Ev::Shutdown(_) => {}
        }
    }

    /// A new server transaction: the dialog layer takes what belongs to
    /// one of its dialogs; the rest is ours.
    fn on_incoming(&mut self, tx: rsipstack::transaction::transaction::Transaction) {
        use rsipstack::sip::Method as M;
        if let Some(dialog) = self.layer.match_dialog(&tx) {
            return self.dispatch(dialog, tx);
        }
        match tx.original.method.clone() {
            M::Invite => self.on_invite(tx),
            M::Ack | M::Cancel => {}
            M::Notify => self.on_stray_notify(tx),
            M::Message => {
                self.deliver_message(&tx.original.clone());
                self.reply(tx, 200, Vec::new());
            }
            M::Options => {
                let headers = vec![Core::allow(), rsipstack::sip::Header::Accept("application/sdp".to_string().into())];
                self.reply(tx, 200, headers);
            }
            M::Bye | M::Update | M::Info | M::Refer => self.reply(tx, 481, Vec::new()),
            M::Subscribe => self.reply(tx, 489, Vec::new()),
            _ => self.reply(tx, 405, Vec::new()),
        }
    }

    async fn shutdown(&mut self) {
        let live: Vec<_> = self.calls.iter().filter(|c| !c.ended).map(|c| c.id).collect();
        for id in live {
            let _ = self.hangup(id);
        }
        self.unregister_all();
        self.publish_accounts(true);
        // Give the BYEs and un-REGISTERs a moment to leave.
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}
