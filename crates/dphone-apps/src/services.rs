//! What the app host needs from the rest of the firmware, as traits the
//! integrator implements: the DPMA application transport, the apps' store
//! on the PBX, an HTTP client, and configuration.  Call control, presence, and audio go through the
//! `dphone-state` command bus and need no adapter.

use std::sync::Arc;

/// A one-shot reply path.  It may be called from any thread; it queues the
/// result for the app and wakes the manager.
pub struct Completion<T>(Box<dyn FnOnce(T) + Send>);

impl<T> Completion<T> {
    pub fn new(f: impl FnOnce(T) + Send + 'static) -> Self {
        Completion(Box::new(f))
    }
    pub fn complete(self, value: T) {
        (self.0)(value)
    }
}

/// The provisioned application account of an app (`<application>` /
/// `<account>` in the configuration).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppAccount {
    pub account_id: String,
    pub username: String,
    pub password: String,
    /// `dasp://HOST/json`, or an HTTP(S) URL.
    pub appserver: String,
    /// `json` (default) or `xml`.
    pub apiformat: String,
}

/// A request for the PBX application server over `dasp://`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PbxRequest {
    /// The `dasp://HOST/path` URL from the app account.
    pub url: String,
    /// Credentials for the `X-Digium-AppServer-Digest-Auth=USER:SECRET`
    /// line.
    pub username: String,
    pub password: String,
    /// The request document, e.g.,
    /// `{"request":{"method":"...","parameters":{...}}}`.
    pub body: String,
    /// App that asked (the DPMA application id field).
    pub app: String,
}

/// The DPMA application transport (`AsteriskRequest`).  The reply is the
/// response document with the HTTP-style preamble already stripped.
pub trait PbxRequests: Send + Sync {
    fn request(&self, request: PbxRequest, done: Completion<Result<Vec<u8>, String>>);
}

/// Whose keys of an app's store on the PBX.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StoreScope {
    Shared,
    User,
}

/// The phone's user, as the PBX knows them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StoreUser {
    pub id: String,
    pub name: String,
    pub extension: String,
}

/// One key of an app's store on the PBX.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StoreEntry {
    pub key: String,
    pub value: Vec<u8>,
    pub version: u64,
    pub modified: u64,
    pub author_id: String,
    pub author_name: String,
}

/// A request to the PBX's store for the app `app`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreOp {
    Get {
        app: String,
        scope: StoreScope,
        key: String,
    },
    List {
        app: String,
        scope: StoreScope,
        prefix: String,
        after: String,
    },
    Set {
        app: String,
        scope: StoreScope,
        key: String,
        value: Vec<u8>,
        expect: Option<u64>,
    },
    Remove {
        app: String,
        scope: StoreScope,
        key: String,
        expect: Option<u64>,
    },
}

/// How a [`StoreOp`] ended (`storage-outcome` of the app interface).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreReply {
    Entry(Option<StoreEntry>),
    Entries(Vec<StoreEntry>, bool),
    Stored(StoreEntry),
    Removed,
    Conflict(Option<StoreEntry>),
    Unreachable(String),
    Refused(String),
}

/// What the store tells the app host without being asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreSignal {
    /// The store can be reached (again), and the user is known.
    Online,
    /// Another phone set or removed a key of `app`.
    Changed {
        app: String,
        scope: StoreScope,
        key: String,
    },
}

/// The apps' store of keys and values on the PBX (the DPMA
/// `AppDataRequest`).
pub trait PbxStore: Send + Sync {
    fn request(&self, op: StoreOp, done: Completion<StoreReply>);
    /// The phone's user, once the PBX has said who it is.
    fn user(&self) -> Option<StoreUser>;
    /// Whether the store can be reached now.
    fn online(&self) -> bool;
    /// The apps whose changes are wanted as [`StoreSignal::Changed`]: all
    /// of them, each time the set changes.
    fn watch(&self, apps: &[String]);
    /// Registers who gets the signals.  They may come from any thread.
    fn subscribe(&self, f: Box<dyn Fn(StoreSignal) + Send + Sync>);
}

/// A [`PbxStore`] for a phone without a PBX to keep one.
pub struct NoStore;

impl PbxStore for NoStore {
    fn request(&self, _op: StoreOp, done: Completion<StoreReply>) {
        done.complete(StoreReply::Unreachable("this phone has no PBX".into()));
    }
    fn user(&self) -> Option<StoreUser> {
        None
    }
    fn online(&self) -> bool {
        false
    }
    fn watch(&self, _apps: &[String]) {}
    fn subscribe(&self, _f: Box<dyn Fn(StoreSignal) + Send + Sync>) {}
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Credentials to offer (Basic or Digest as the server asks), if any.
    pub credentials: Option<(String, String)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HttpReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub trait HttpClient: Send + Sync {
    fn request(&self, request: HttpRequest, done: Completion<Result<HttpReply, String>>);
}

/// Configuration the apps read.
pub trait ConfigSource: Send + Sync {
    /// The provisioned `<application>` element for the app, as XML ("" if
    /// none), without the credentials: the app reads this text as it is,
    /// so the attributes `password` and `secret` of the element and of
    /// its `<account>`s, and the `<setting>`s with those ids, must not be
    /// in it.
    fn app_config_xml(&self, app: &str) -> String;
    /// The app's application account, if provisioned.
    fn app_account(&self, app: &str) -> Option<AppAccount>;
    /// A named attribute or `<setting>` of the app's configuration.
    fn app_setting(&self, app: &str, name: &str) -> Option<String>;
    /// False when the configuration disables or does not list a built-in.
    fn app_enabled(&self, _app: &str) -> bool {
        true
    }
    /// A phone setting by stock id.  The host only passes ids on its
    /// allowlist ([`APP_VISIBLE_SETTINGS`]).
    fn setting(&self, id: &str) -> Option<String>;
    fn mac_address(&self) -> String;
    /// Provisioned contact lists: (file name, XML text).
    fn contact_lists(&self) -> Vec<(String, String)>;
    /// Local time offset from UTC in seconds, for `l10n.format-time`.
    fn utc_offset_secs(&self) -> i32 {
        0
    }
    /// 24-hour clock.
    fn clock_24h(&self) -> bool {
        false
    }
    /// Seconds since the Unix epoch, for `timers.wall-time`.
    fn wall_time(&self) -> u64 {
        dphone_state::clock::unix_secs()
    }
}

/// Phone settings apps may read through `config.setting`.
pub const APP_VISIBLE_SETTINGS: &[&str] = &[
    "locale",
    "timezone",
    "time_format",
    "date_format",
    "contacts_max_subscriptions",
    "blf_contact_group",
    "default_ringtone",
    "name_format",
    "brightness",
];

#[derive(Clone)]
pub struct Services {
    pub pbx: Arc<dyn PbxRequests>,
    pub store: Arc<dyn PbxStore>,
    pub http: Arc<dyn HttpClient>,
    pub config: Arc<dyn ConfigSource>,
}
