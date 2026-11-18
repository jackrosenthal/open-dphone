//! opendeskphone-apps: the WebAssembly app runtime, on its own thread.
//!
//! The manager owns the `Apps` state and command domain.  Apps reach the
//! PBX through DPMA (`dasp://`), their store of keys and values there
//! through `appstore`, HTTP and HTTPS through the client in `apphttp`, and
//! the configuration through the merged view.

use std::sync::Arc;

use opendeskphone_apps::services::{
    AppAccount, Completion, ConfigSource, HttpClient, HttpReply, HttpRequest, PbxRequest, PbxRequests, Services,
};
use opendeskphone_apps::{AppManager, ManagerConfig};
use opendeskphone_config::xml::Element;
use opendeskphone_state::{HealthStatus, Subsystem};
use opendeskphone_ui::UiHandle;
use log::{error, info, warn};

use super::SharedConfig;
use super::net::NetHandle;
use super::telephony::Telephony;
use crate::mainrole::{Context, state_model};

// Installing an app compiles its WebAssembly on the phone.  A phone binary
// built without the compiler (the feature `compile` of `opendeskphone-apps`,
// which `crates/opendeskphone/Cargo.toml` turns on) runs the built-in apps and
// refuses every app file, and no test on the build machine would notice: the
// tests have the compiler for their own use.  So the build fails.
const _: () = assert!(
    opendeskphone_apps::AppEngine::can_compile(),
    "the phone binary must be built with opendeskphone-apps' feature `compile`: without it no app can be installed"
);

struct Pbx {
    telephony: Telephony,
    runtime: tokio::runtime::Handle,
}

impl PbxRequests for Pbx {
    fn request(&self, request: PbxRequest, done: Completion<Result<Vec<u8>, String>>) {
        let dpma = self.telephony.dpma.lock().unwrap().clone();
        self.runtime.spawn(async move {
            let Some(dpma) = dpma else {
                done.complete(Err("not provisioned: no DPMA session".into()));
                return;
            };
            let credentials = format!("{}:{}", request.username, request.password);
            let result = dpma
                .app_request(&request.url, Some(&credentials), request.body.as_bytes())
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| if r.status < 400 { Ok(r.body) } else { Err(format!("server answered {}", r.status)) });
            done.complete(result);
        });
    }
}

/// HTTP and HTTPS for apps (see [`super::apphttp`]).
struct Http {
    runtime: tokio::runtime::Handle,
    net: NetHandle,
    config: Option<SharedConfig>,
}

impl Http {
    /// As on stock (libcurl with `/nvdata/certs/certs.crt`): servers are
    /// checked against the trust bundle unless `allow_insecure_ssl` is
    /// set (the same policy as SIP over TLS).
    fn tls_policy(&self) -> opendeskphone_sip::TlsConfig {
        let Some(store) = self.config.as_ref().and_then(|c| c.lock().ok()) else {
            return opendeskphone_sip::TlsConfig::default();
        };
        let cfg = store.config();
        opendeskphone_sip::TlsConfig {
            verify: !cfg.sip().allow_insecure_ssl,
            roots_pem: super::trust::roots(store.nvdata(), &cfg),
            ..Default::default()
        }
    }
}

impl HttpClient for Http {
    fn request(&self, request: HttpRequest, done: Completion<Result<HttpReply, String>>) {
        let net = self.net.clone();
        let tls = self.tls_policy();
        self.runtime.spawn(async move { done.complete(super::apphttp::request(Some(&net), &tls, request).await) });
    }
}

struct AppConfig {
    config: Option<SharedConfig>,
    mac: String,
}

impl AppConfig {
    /// The app's configuration: the `<appconfig id="app">` element, as
    /// DPMA sends it and as the stock engine hands it to a stock app
    /// (`digium.app.config.xml`, which `lib/app.js` reads as `appconfig`
    /// with its `account` and `settings`).
    fn appconfig(&self, app: &str) -> Option<Element> {
        let cfg = self.config.as_ref()?.lock().ok()?.config();
        cfg.appconfig(app).cloned()
    }

    fn merged(&self) -> Option<Arc<opendeskphone_config::Config>> {
        Some(self.config.as_ref()?.lock().ok()?.config())
    }
}

/// An app's `<appconfig>` element as the app may read it: without the
/// credentials, which stay with the host (it adds them to PBX requests).
/// These are the values `config.app-setting` withholds: the attributes
/// `password` and `secret` of the element and of its children (its
/// `<account>` and its `<settings>`).
fn without_credentials(mut appconfig: Element) -> Element {
    const WITHHELD: [&str; 2] = ["password", "secret"];
    for name in WITHHELD {
        appconfig.remove_attr(name);
    }
    for child in appconfig.elements_mut() {
        for name in WITHHELD {
            child.remove_attr(name);
        }
    }
    appconfig
}

/// Whether an app's `<appconfig>` disables it, as stock's `app.isDisabled()`
/// decides: `lib/app.js` makes an object of the element
/// (`util.restructureXmlToJson`), in which a child `<disabled>` is an
/// object and an attribute `disabled`, which replaces it, is a string, and
/// the app is disabled when that value is truthy.
fn disables(appconfig: &Element) -> bool {
    match appconfig.attr("disabled") {
        Some(value) => !value.is_empty(),
        None => appconfig.child("disabled").is_some(),
    }
}

impl ConfigSource for AppConfig {
    fn app_config_xml(&self, app: &str) -> String {
        self.appconfig(app).map(|e| without_credentials(e).to_document()).unwrap_or_default()
    }

    /// The `<account>` of the app's `<appconfig>`, with the attributes
    /// stock's `lib/app.js` and `lib/pbx.js` read.
    fn app_account(&self, app: &str) -> Option<AppAccount> {
        let appconfig = self.appconfig(app)?;
        let account = appconfig.child("account")?;
        let get = |name: &str| account.attr(name).unwrap_or_default().to_string();
        Some(AppAccount {
            account_id: get("account_id"),
            username: get("username"),
            password: get("password"),
            appserver: get("appserver"),
            apiformat: account.attr("apiformat").filter(|f| !f.is_empty()).unwrap_or("json").to_string(),
        })
    }

    /// An attribute of the app's `<settings>` (where DPMA puts an app's
    /// settings), else of its `<appconfig>`.
    fn app_setting(&self, app: &str, name: &str) -> Option<String> {
        let appconfig = self.appconfig(app)?;
        appconfig.child("settings").and_then(|s| s.attr(name)).or_else(|| appconfig.attr(name)).map(str::to_string)
    }

    /// Off when `disabled_apps` lists the app, or when its `<appconfig>`
    /// disables it as stock's `app.isDisabled()` reads it.
    fn app_enabled(&self, app: &str) -> bool {
        self.merged().is_none_or(|c| {
            !c.disabled_apps().value.iter().any(|d| d == app) && !c.appconfig(app).is_some_and(disables)
        })
    }

    fn setting(&self, id: &str) -> Option<String> {
        let cfg = self.merged()?;
        // The app interface's names for three stock settings differ, and
        // the formats are the locale's unless the phone sets its own.
        match id {
            "timezone" => cfg.setting("time_zone"),
            "time_format" => cfg.localization_setting("time_short_format"),
            "date_format" => cfg.localization_setting("date_short_format"),
            other => cfg.setting(other),
        }
        .map(str::to_string)
    }

    fn mac_address(&self) -> String {
        self.mac.clone()
    }

    fn contact_lists(&self) -> Vec<(String, String)> {
        let Some(nv) = self.config.as_ref().and_then(|c| c.lock().ok().map(|s| s.nvdata().clone())) else {
            return Vec::new();
        };
        nv.contact_list_names()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|name| {
                let text = String::from_utf8(nv.contact_list(&name).ok()??).ok()?;
                Some((name, text))
            })
            .collect()
    }

    fn utc_offset_secs(&self) -> i32 {
        self.merged().map(|c| crate::tz::utc_offset(&c.time().time_zone, crate::sys::now_secs())).unwrap_or(0)
    }

    fn clock_24h(&self) -> bool {
        self.merged()
            .and_then(|c| c.localization_setting("time_short_format").map(|f| f.contains('H') || f.contains('k')))
            .unwrap_or(false)
    }
}

pub fn start(
    ctx: &Context,
    ui: Option<UiHandle>,
    config: Option<SharedConfig>,
    telephony: Telephony,
    net: NetHandle,
    app_store: super::appstore::AppStore,
) {
    ctx.store.set_health(Subsystem::Apps, HealthStatus::Starting);
    let Some(ui) = ui else {
        warn!("apps: no UI; the app runtime is not started");
        ctx.store.set_health(Subsystem::Apps, HealthStatus::Failed("no UI".into()));
        return;
    };
    let manager_config = if ctx.sim {
        match &ctx.paths.apps {
            Some(root) => ManagerConfig::rooted(root, &ctx.paths.nvdata),
            None => {
                info!("apps: simulator without --apps-root; no built-in apps");
                ManagerConfig::rooted(&ctx.paths.run.join("no-apps"), &ctx.paths.nvdata)
            }
        }
    } else {
        ManagerConfig::phone()
    };
    let runtime = tokio::runtime::Handle::current();
    let mac = ctx
        .store
        .read::<opendeskphone_state::System, _>(|s| s.mac)
        .map(|m| m.iter().map(|b| format!("{b:02x}")).collect::<String>())
        .unwrap_or_default();
    let services = Services {
        pbx: Arc::new(Pbx { telephony, runtime: runtime.clone() }),
        store: Arc::new(app_store),
        http: Arc::new(Http { runtime, net, config: config.clone() }),
        config: Arc::new(AppConfig { config, mac }),
    };
    let store = ctx.store.clone();
    let model = state_model(ctx.model);
    let spawned = std::thread::Builder::new().name("apps".into()).spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let shell = Box::new(opendeskphone_apps::uishell::UiShell::new(ui, model));
            let mut manager = match AppManager::new(manager_config, store.clone(), shell, services) {
                Ok(m) => m,
                Err(e) => {
                    error!("apps: engine: {e}");
                    store.set_health(Subsystem::Apps, HealthStatus::Failed(e.to_string()));
                    return;
                }
            };
            manager.scan();
            manager.autostart();
            info!("apps: runtime started");
            store.set_health(Subsystem::Apps, HealthStatus::Ok);
            manager.run();
        }));
        if result.is_err() {
            error!("apps: the app runtime thread panicked");
            store.set_health(Subsystem::Apps, HealthStatus::Failed("the app runtime thread panicked".into()));
        }
    });
    if let Err(e) = spawned {
        error!("apps: cannot start the app runtime thread: {e}");
        ctx.store.set_health(Subsystem::Apps, HealthStatus::Failed(e.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use opendeskphone_config::{ConfigStore, Model, Nvdata};

    use super::*;

    /// The app parts of a phone's configuration as Open DPMA writes them
    /// (`build_config` of `res_open_dpma`, which writes what the original
    /// `res_digium_phone` does): a phone `alice` with PIN 4321 and line
    /// 101 on the network `network1`, and the custom application `weather`
    /// with the options `units=metric` and `password=p2`.  The application
    /// screen's list names the apps; each app's `<appconfig>` has its
    /// account and settings.
    const FROM_OPEN_DPMA: &str = concat!(
        "<config>\n",
        "<networks>\n",
        "<network id=\"network1\" display_name=\"Office\" cidr=\"0.0.0.0/0\" />\n",
        "</networks>\n",
        "<appconfig id=\"appscreen\">\n",
        "<application id=\"voicemail\"/>\n",
        "<application id=\"parking\"/>\n",
        "<application id=\"status\"/>\n",
        "<application id=\"contacts\"/>\n",
        "<application id=\"queues\"/>\n",
        "<auto_start />\n",
        "</appconfig>\n",
        "<appconfig id=\"status\" network_id=\"network1\">\n",
        "<send_dnd status=\"dnd\" /><send_dnd status=\"xa\" />",
        "<account server_uuid=\"1c0ffee0\" account_id=\"101\" username=\"alice\" password=\"4321\" appserver=\"dasp://10.10.0.2/json\" apiformat=\"json\" />\n",
        "<auto_start />\n",
        "<full_application />\n",
        "</appconfig>\n",
        "<appconfig id=\"queues\" network_id=\"network1\">\n",
        "<account server_uuid=\"1c0ffee0\" account_id=\"101\" username=\"alice\" password=\"4321\" appserver=\"dasp://10.10.0.2/json\" apiformat=\"json\" />\n",
        "<auto_start />\n",
        "</appconfig>\n",
        "<appconfig id=\"weather\" network_id=\"network1\">\n",
        "<url url=\"http://10.10.0.2/files/weather.zip\" />\n",
        "<account server_uuid=\"1c0ffee0\" account_id=\"101\" username=\"alice\" password=\"4321\" appserver=\"dasp://10.10.0.2/json\" apiformat=\"json\" />\n",
        "<settings \n",
        "units=\"metric\" \n",
        "password=\"p2\" \n",
        "/>\n",
        "</appconfig>\n",
        "<appconfig id=\"parking\" network_id=\"network1\">\n",
        "<account server_uuid=\"1c0ffee0\" account_id=\"101\" username=\"alice\" password=\"4321\" appserver=\"dasp://10.10.0.2/json\" apiformat=\"json\" />\n",
        "<auto_start />\n",
        "</appconfig>\n",
        "<appconfig id=\"voicemail\" network_id=\"network1\">\n",
        "<account server_uuid=\"1c0ffee0\" account_id=\"101\" username=\"alice\" password=\"4321\" appserver=\"dasp://10.10.0.2/json\" apiformat=\"json\" />\n",
        "<settings require_password=\"1\"/>\n",
        "</appconfig>\n",
        "</config>\n",
    );

    /// What the app host reads of the configuration of a phone (its nvdata
    /// in a temporary directory) that was sent `config`.
    fn phone_with(config: &str) -> (tempfile::TempDir, AppConfig) {
        let dir = tempfile::tempdir().unwrap();
        let nv = Nvdata::new(dir.path().join("nvdata"));
        nv.ensure_layout().unwrap();
        let mut store = ConfigStore::open(nv, Model::D60, dir.path().join("tmp")).unwrap();
        store.apply_fetched(config).unwrap();
        let apps = AppConfig { config: Some(Arc::new(Mutex::new(store))), mac: "000fd3000001".into() };
        (dir, apps)
    }

    #[test]
    fn the_pbx_apps_get_their_accounts_as_open_dpma_provisions_them() {
        let (_dir, apps) = phone_with(FROM_OPEN_DPMA);
        let account = AppAccount {
            account_id: "101".into(),
            username: "alice".into(),
            password: "4321".into(),
            appserver: "dasp://10.10.0.2/json".into(),
            apiformat: "json".into(),
        };
        for app in ["voicemail", "status", "parking", "queues"] {
            assert_eq!(apps.app_account(app).as_ref(), Some(&account), "{app}");
        }
    }

    #[test]
    fn an_app_reads_its_appconfig_without_the_credentials() {
        let (_dir, apps) = phone_with(FROM_OPEN_DPMA);
        let expected = Element::parse(concat!(
            "<appconfig id=\"weather\" network_id=\"network1\">",
            "<url url=\"http://10.10.0.2/files/weather.zip\" />",
            "<account server_uuid=\"1c0ffee0\" account_id=\"101\" username=\"alice\" appserver=\"dasp://10.10.0.2/json\" apiformat=\"json\" />",
            "<settings units=\"metric\" />",
            "</appconfig>",
        ))
        .unwrap();
        assert_eq!(Element::parse(&apps.app_config_xml("weather")).unwrap(), expected);
    }

    #[test]
    fn an_app_setting_is_an_attribute_of_the_apps_settings_or_appconfig() {
        let (_dir, apps) = phone_with(FROM_OPEN_DPMA);
        assert_eq!(apps.app_setting("weather", "units").as_deref(), Some("metric"));
        assert_eq!(apps.app_setting("voicemail", "require_password").as_deref(), Some("1"));
        assert_eq!(apps.app_setting("status", "network_id").as_deref(), Some("network1"));
    }

    /// As stock's `app.isDisabled()`: a `<disabled>` child, or a
    /// `disabled` attribute with a value.
    #[test]
    fn an_apps_appconfig_disables_it() {
        let (_dir, apps) = phone_with(concat!(
            "<config>\n",
            "<appconfig id=\"voicemail\"><disabled/></appconfig>\n",
            "<appconfig id=\"parking\" disabled=\"1\"/>\n",
            "</config>\n",
        ));
        assert!(!apps.app_enabled("voicemail"));
        assert!(!apps.app_enabled("parking"));
    }

    #[test]
    fn an_app_reads_the_time_and_date_formats_of_the_locale() {
        let (_dir, apps) = phone_with("<config/>");
        assert_eq!(apps.setting("time_format").as_deref(), Some("h:mm a"));
        assert_eq!(apps.setting("date_format").as_deref(), Some("M/d/yy"));
    }

    #[test]
    fn the_apps_clock_has_24_hours_when_the_locales_time_format_has() {
        let (_dir, apps) = phone_with(concat!(
            "<config>\n",
            "<setting id=\"locale\" value=\"de_DE\" conflict=\"freeze\" />\n",
            "<locales><locale id=\"de_DE\"><setting id=\"time_short_format\" value=\"HH:mm\"/></locale></locales>\n",
            "</config>\n",
        ));
        assert!(apps.clock_24h());
    }
}
