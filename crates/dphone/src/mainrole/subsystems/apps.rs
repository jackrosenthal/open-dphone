//! dphone-apps: the WebAssembly app runtime, on its own thread.
//!
//! The manager owns the `Apps` state and command domain.  Apps reach the
//! PBX through DPMA (`dasp://`), their store of keys and values there
//! through `appstore`, HTTP and HTTPS through the client in `apphttp`, and
//! the configuration through the merged view.

use std::path::PathBuf;
use std::sync::Arc;

use dphone_apps::services::{
    AppAccount, Completion, ConfigSource, HttpClient, HttpReply, HttpRequest, PbxRequest, PbxRequests, Services,
};
use dphone_apps::{AppManager, ManagerConfig};
use dphone_config::xml::Element;
use dphone_state::{HealthStatus, Subsystem};
use dphone_ui::UiHandle;
use log::{error, info, warn};

use super::SharedConfig;
use super::net::NetHandle;
use super::telephony::Telephony;
use crate::mainrole::{Context, state_model};

// Installing an app compiles its WebAssembly on the phone.  A phone
// binary built without the compiler (the feature `compile` of `dphone-apps`,
// which `crates/dphone/Cargo.toml` turns on) runs the built-in apps and
// refuses every app file, and no test on the build machine would notice:
// the tests have the compiler for their own use.  So the build fails.
const _: () = assert!(
    dphone_apps::AppEngine::can_compile(),
    "the phone binary must be built with dphone-apps' feature `compile`: without it no app can be installed"
);

/// Where the simulator looks for staged apps (see `crates/dphone-apps/README.md`).
pub const APPS_ROOT_ENV: &str = "DPHONE_APPS_ROOT";

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
    fn tls_policy(&self) -> dphone_sip::TlsConfig {
        let Some(store) = self.config.as_ref().and_then(|c| c.lock().ok()) else {
            return dphone_sip::TlsConfig::default();
        };
        let cfg = store.config();
        dphone_sip::TlsConfig {
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
    /// The provisioned `<application id="app">` element, wherever the
    /// configuration lists it.
    fn application(&self, app: &str) -> Option<Element> {
        let cfg = self.config.as_ref()?.lock().ok()?.config();
        cfg.root()
            .children_named("appconfig")
            .flat_map(|a| a.children_named("application"))
            .find(|e| e.attr("id") == Some(app))
            .cloned()
    }

    fn merged(&self) -> Option<Arc<dphone_config::Config>> {
        Some(self.config.as_ref()?.lock().ok()?.config())
    }
}

/// An app's `<application>` element as the app may read it: without the
/// credentials, which stay with the host (it adds them to PBX requests).
/// These are the values `config.app-setting` withholds: the attributes
/// `password` and `secret` of the element and of its `<account>`s, and
/// the `<setting>`s with those ids.
fn without_credentials(mut application: Element) -> Element {
    const WITHHELD: [&str; 2] = ["password", "secret"];
    for name in WITHHELD {
        application.remove_attr(name);
    }
    application.remove_children(|e| e.name == "setting" && e.attr("id").is_some_and(|id| WITHHELD.contains(&id)));
    for account in application.elements_mut().filter(|e| e.name == "account") {
        for name in WITHHELD {
            account.remove_attr(name);
        }
    }
    application
}

impl ConfigSource for AppConfig {
    fn app_config_xml(&self, app: &str) -> String {
        self.application(app).map(|e| without_credentials(e).to_document()).unwrap_or_default()
    }

    fn app_account(&self, app: &str) -> Option<AppAccount> {
        let application = self.application(app)?;
        let account = application.child("account")?;
        let get = |name: &str| account.attr(name).unwrap_or_default().to_string();
        Some(AppAccount {
            account_id: get("id"),
            username: get("username"),
            password: get("password"),
            appserver: get("appserver"),
            apiformat: account.attr("apiformat").filter(|f| !f.is_empty()).unwrap_or("json").to_string(),
        })
    }

    fn app_setting(&self, app: &str, name: &str) -> Option<String> {
        let application = self.application(app)?;
        application
            .attr(name)
            .map(str::to_string)
            .or_else(|| application.child_with_id("setting", name).and_then(|s| s.attr("value")).map(str::to_string))
    }

    fn app_enabled(&self, app: &str) -> bool {
        self.merged().is_none_or(|c| !c.disabled_apps().value.iter().any(|d| d == app))
    }

    fn setting(&self, id: &str) -> Option<String> {
        let cfg = self.merged()?;
        // The app interface's names for two stock settings differ.
        let id = match id {
            "timezone" => "time_zone",
            "time_format" => "time_short_format",
            "date_format" => "date_short_format",
            other => other,
        };
        cfg.setting(id).map(str::to_string)
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
            .and_then(|c| c.setting("time_short_format").map(|f| f.contains('H') || f.contains('k')))
            .unwrap_or(false)
    }
}

pub fn start(ctx: &Context, ui: Option<UiHandle>, config: Option<SharedConfig>, telephony: Telephony, net: NetHandle) {
    ctx.store.set_health(Subsystem::Apps, HealthStatus::Starting);
    let Some(ui) = ui else {
        warn!("apps: no UI; the app runtime is not started");
        ctx.store.set_health(Subsystem::Apps, HealthStatus::Failed("no UI".into()));
        return;
    };
    let manager_config = if ctx.sim {
        match std::env::var_os(APPS_ROOT_ENV) {
            Some(root) => ManagerConfig::rooted(&PathBuf::from(root), &ctx.paths.nvdata),
            None => {
                info!("apps: simulator without {APPS_ROOT_ENV}; no built-in apps");
                ManagerConfig::rooted(&ctx.paths.run.join("no-apps"), &ctx.paths.nvdata)
            }
        }
    } else {
        ManagerConfig::phone()
    };
    let runtime = tokio::runtime::Handle::current();
    let mac = ctx
        .store
        .read::<dphone_state::System, _>(|s| s.mac)
        .map(|m| m.iter().map(|b| format!("{b:02x}")).collect::<String>())
        .unwrap_or_default();
    let app_store = super::appstore::AppStore::new(telephony.clone(), runtime.clone());
    app_store.follow(ctx.store.clone());
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
            let shell = Box::new(dphone_apps::uishell::UiShell::new(ui, model));
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
    use super::*;

    #[test]
    fn an_app_reads_its_configuration_without_the_credentials() {
        let application = Element::parse(
            r#"<application id="voicemail" secret="s1" refresh="30">
                 <account id="100" username="100" password="p1" appserver="dasp://pbx/json" />
                 <setting id="folder" value="INBOX" />
                 <setting id="password" value="p2" />
               </application>"#,
        )
        .unwrap();
        let text = without_credentials(application).to_document();
        for withheld in ["p1", "p2", "s1", "password", "secret"] {
            assert!(!text.contains(withheld), "{withheld} in {text}");
        }
        let read = Element::parse(&text).unwrap();
        assert_eq!(read.attr("refresh"), Some("30"));
        let account = read.child("account").unwrap();
        assert_eq!((account.attr("id"), account.attr("username")), (Some("100"), Some("100")));
        assert_eq!(account.attr("appserver"), Some("dasp://pbx/json"));
        assert_eq!(read.child_with_id("setting", "folder").and_then(|s| s.attr("value")), Some("INBOX"));
    }
}
