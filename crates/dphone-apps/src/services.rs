//! What the app host needs from the rest of the firmware, as traits the
//! integrator implements: the DPMA application transport, an HTTP client,
//! and configuration.  Call control, presence, and audio go through the
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
    /// none).
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
    pub http: Arc<dyn HttpClient>,
    pub config: Arc<dyn ConfigSource>,
}
