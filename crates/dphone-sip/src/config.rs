//! Input configuration.  Plain structs: the integrator maps
//! `dphone-config`'s merged view onto these, so this crate does not depend
//! on the configuration crate.

use dphone_state::{AccountId, Codec, FirmwareVersion, Model, Transport};
use std::net::{IpAddr, Ipv4Addr};

/// One `host_primary` / `host_alternate` record.  Stock keeps these
/// host-specific; credentials fall back host, then account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostConfig {
    pub server: String,
    /// 0: the transport's default (or SRV, when the resolver does it).
    pub port: u16,
    pub transport: Transport,
    /// Registration lifetime requested (`reregister`); stock default 300.
    pub expires_secs: u32,
    /// Wait before retrying a failed registration; stock default 20.
    pub retry_secs: u32,
    /// Failed attempts on this host before trying the other one.
    pub num_retries: u32,
    /// `media_encryption`: SDES SRTP is required with this host.
    pub srtp: bool,
    /// Host-level overrides of the account's credentials.
    pub auth_user: Option<String>,
    pub password: Option<String>,
}

impl HostConfig {
    pub fn new(server: impl Into<String>) -> Self {
        HostConfig {
            server: server.into(),
            port: 0,
            transport: Transport::Udp,
            expires_secs: 300,
            retry_secs: 20,
            num_retries: 3,
            srtp: false,
            auth_user: None,
            password: None,
        }
    }

    pub fn effective_port(&self) -> u16 {
        match (self.port, self.transport) {
            (0, Transport::Tls) => 5061,
            (0, _) => 5060,
            (p, _) => p,
        }
    }
}

/// One SIP account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountConfig {
    pub id: AccountId,
    /// Stock `account id` attribute.
    pub config_id: String,
    pub label: String,
    pub display_name: String,
    /// SIP user part (`username`).
    pub user: String,
    /// `authname`; empty falls back to `user`.
    pub auth_user: String,
    pub password: String,
    pub primary: HostConfig,
    pub alternate: Option<HostConfig>,
    /// `host:port` of an outbound proxy, or empty.
    pub outbound_proxy: String,
    pub voicemail_number: String,
    /// Register at all (`register="1"`).
    pub register: bool,
    /// Subscribe to `message-summary` (unsolicited MWI is always accepted).
    pub mwi_subscribe: bool,
    /// `mwi_subscription_uri`; empty means the account's own address.
    pub mwi_uri: String,
    /// This account's registrar is the DPMA server.
    pub dpma: bool,
}

impl AccountConfig {
    pub fn new(id: AccountId, user: impl Into<String>, password: impl Into<String>, server: impl Into<String>) -> Self {
        let user = user.into();
        AccountConfig {
            id,
            config_id: user.clone(),
            label: user.clone(),
            display_name: String::new(),
            user,
            auth_user: String::new(),
            password: password.into(),
            primary: HostConfig::new(server),
            alternate: None,
            outbound_proxy: String::new(),
            voicemail_number: String::new(),
            register: true,
            mwi_subscribe: true,
            mwi_uri: String::new(),
            dpma: false,
        }
    }

    pub(crate) fn host(&self, alt: bool) -> &HostConfig {
        match (&self.alternate, alt) {
            (Some(a), true) => a,
            _ => &self.primary,
        }
    }

    pub(crate) fn credentials(&self, alt: bool) -> (String, String) {
        let h = self.host(alt);
        let user = h
            .auth_user
            .clone()
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| if self.auth_user.is_empty() { self.user.clone() } else { self.auth_user.clone() });
        let pass = h.password.clone().filter(|p| !p.is_empty()).unwrap_or_else(|| self.password.clone());
        (user, pass)
    }
}

/// A watched extension (BLF / rapid dial with a subscription).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlfConfig {
    /// Stable ID (the contact ID); becomes `BlfEntry::id`.
    pub id: String,
    pub label: String,
    /// What pressing the key dials.
    pub number: String,
    /// The user part or full URI to SUBSCRIBE to (stock
    /// `subscription_extension`, e.g. `auto_hint_101`); empty: no
    /// subscription, plain rapid dial.
    pub subscribe: String,
    pub account: AccountId,
    /// Prefix dialed with the extension for directed pickup when the
    /// ringing dialog's identifiers are not known (e.g. `**`); empty
    /// disables that fallback.
    pub pickup_prefix: String,
    /// Also subscribe to the `presence` event for the user's status.
    pub presence: bool,
    /// The item's `location`, `index`, and `paging`, and whether it is a
    /// blank that only reserves its key (see [`crate::keys`]).
    pub location: BlfLocation,
    pub index: u8,
    pub paging: bool,
    pub blank: bool,
}

/// What to do with an incoming call that carries a server event name
/// (stock `<events><event id=".." action=".."/>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventAction {
    /// Answer without ringing (`auto_answer`, `auto_answer_now`).
    AutoAnswer,
    /// The call is an app's: hidden, and left for the app to answer.
    App,
    Reject,
}

/// A `<blf_item>`'s `location`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BlfLocation {
    /// A line key.
    #[default]
    Main,
    /// The rapid-dial strip (D50, D70); on phones without one, the free
    /// line keys.
    Side,
    /// A key of an expansion module; watched here, shown there.
    Expansion,
}

/// TLS client policy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TlsConfig {
    /// PEM bundle of trusted roots (stock `/nvdata/certs`).  With
    /// `verify` set and no roots, every TLS connection fails.
    pub roots_pem: Vec<u8>,
    /// Check the server certificate against `roots_pem` and its name.
    /// Stock's default is not to; see `docs/device-identity-trust.md`.
    pub verify: bool,
    /// With `verify`: refuse a server whose certificate matches its name
    /// only through a wildcard (stock `tls_allow_wildcard_certs` off; RFC
    /// 5922 section 7.2 does not allow such certificates for SIP).
    pub refuse_wildcard_certs: bool,
    /// Certificate (DER) and PKCS#8 private key (DER) presented to peers
    /// that connect to our TLS port.  Empty: no TLS listener.
    pub server_cert_der: Vec<u8>,
    pub server_key_der: Vec<u8>,
}

/// One enabled codec, in preference order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodecPref {
    pub codec: Codec,
    /// Packetization in milliseconds.
    pub ptime: u16,
}

/// Everything the service needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SipConfig {
    pub model: Model,
    pub version: FirmwareVersion,
    /// The phone's address, for Via, Contact, and SDP.
    pub local_ip: IpAddr,
    /// Local UDP (and TCP source-independent) signaling port.  0 picks a
    /// free port (tests).
    pub local_port: u16,
    pub accounts: Vec<AccountConfig>,
    /// Enabled codecs in preference order.  Codecs the media engine
    /// reports unavailable are dropped at negotiation time.
    pub codecs: Vec<CodecPref>,
    /// Incoming calls treated specially by the event name they carry.
    /// Stock's base configuration has the one entry this starts with.
    pub events: Vec<(String, EventAction)>,
    /// The configured BLF and rapid-dial items, in configuration order.
    pub blf: Vec<BlfConfig>,
    /// Line keys page with the rapid-dial page (stock `page_linekeys`: on
    /// for the D65).
    pub page_line_keys: bool,
    /// Seconds after a page change until the first page shows again
    /// (stock `blf_page_return_timeout`); 0: stay.
    pub blf_page_return_secs: u32,
    pub tls: TlsConfig,
    /// Seconds between UDP keepalives to each registrar (`udp_ka_interval`);
    /// 0 turns them off.
    pub udp_keepalive_secs: u32,
    /// Offer a second call while one is up (486 otherwise).
    pub call_waiting: bool,
    /// Calls that may exist at once.
    pub max_calls: usize,
    /// Answer `Call-Info: answer-after=0` / `Alert-Info: ...;info=alert-autoanswer`
    /// calls (intercom, paging) without ringing.
    pub allow_auto_answer: bool,
    /// Seconds a ringing call rings before the no-answer forward fires,
    /// when that forward is set.
    pub no_answer_secs: u16,
    /// Seconds an ended call lingers in the state before removal.
    pub ended_linger_secs: u16,
    /// Subscription lifetime requested for BLF and MWI.
    pub subscribe_expires_secs: u32,
    /// The parking extension (stock `parking_lot_extension`); empty: the
    /// phone cannot park.
    pub park_extension: String,
    /// Park with a blind transfer to the parking extension (stock
    /// `parking_lot_enable_blind_transfer`).  Otherwise the parking
    /// extension is called as a consultation call, to be completed as an
    /// attended transfer.
    pub park_blind_transfer: bool,
    /// Session timers (RFC 4028): offer and accept them.
    pub session_timers: bool,
    /// The session interval we ask for, and the smallest we accept.
    pub session_expires_secs: u32,
    pub min_se_secs: u32,
    /// Act on a `check-sync` NOTIFY by fetching the configuration again
    /// (stock `enable_check_sync`).
    pub check_sync: bool,
    /// Port to accept SIP over TCP on (stock `transport_tcp_port`), when
    /// an account uses TCP.  `None`: do not listen.
    pub tcp_port: Option<u16>,
    /// Port to accept SIP over TLS on (stock `transport_tls_port`), when
    /// an account uses TLS and [`TlsConfig`] has a server certificate.
    pub tls_port: Option<u16>,
}

impl SipConfig {
    pub fn new(model: Model, local_ip: IpAddr) -> Self {
        SipConfig {
            model,
            version: FirmwareVersion::CURRENT,
            local_ip,
            local_port: 5060,
            accounts: Vec::new(),
            codecs: vec![
                CodecPref { codec: Codec::G722, ptime: 20 },
                CodecPref { codec: Codec::Pcmu, ptime: 20 },
                CodecPref { codec: Codec::Pcma, ptime: 20 },
                CodecPref { codec: Codec::G726_32, ptime: 20 },
            ],
            events: vec![("digium.incomingCall.voicemail".into(), EventAction::App)],
            blf: Vec::new(),
            page_line_keys: model == Model::D65,
            blf_page_return_secs: 0,
            tls: TlsConfig::default(),
            udp_keepalive_secs: 30,
            call_waiting: true,
            max_calls: 4,
            allow_auto_answer: true,
            no_answer_secs: 20,
            ended_linger_secs: 3,
            subscribe_expires_secs: 3600,
            park_extension: String::new(),
            park_blind_transfer: false,
            session_timers: true,
            session_expires_secs: 1800,
            min_se_secs: 90,
            check_sync: true,
            tcp_port: None,
            tls_port: None,
        }
    }

    /// `Digium <model> <version>`: DPMA reads the model from the second
    /// word, and the dialog-info supplement keys on the vendor name.
    pub fn user_agent(&self) -> String {
        self.model.user_agent(&self.version)
    }
}

impl Default for SipConfig {
    fn default() -> Self {
        SipConfig::new(Model::D60, IpAddr::V4(Ipv4Addr::LOCALHOST))
    }
}
