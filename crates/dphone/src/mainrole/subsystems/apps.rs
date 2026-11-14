//! dphone-apps: the WebAssembly app runtime, on its own thread.
//!
//! The manager owns the `Apps` state and command domain.  Apps reach the
//! PBX through DPMA (`dasp://`), plain HTTP through a small client here,
//! and the configuration through the merged view.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use dphone_apps::services::{
    AppAccount, Completion, ConfigSource, HttpClient, HttpReply, HttpRequest, PbxRequest, PbxRequests, Services,
};
use dphone_apps::{AppManager, ManagerConfig};
use dphone_config::xml::Element;
use dphone_state::{HealthStatus, Subsystem};
use dphone_ui::UiHandle;
use log::{error, info, warn};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::SharedConfig;
use super::telephony::Telephony;
use crate::mainrole::{Context, state_model};

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

/// Plain HTTP/1.1 for apps.  HTTPS is not offered here yet: apps that need
/// a secure channel go through DPMA.
struct Http {
    runtime: tokio::runtime::Handle,
}

const HTTP_LIMIT: usize = 1 << 20;

async fn http(request: HttpRequest) -> Result<HttpReply, String> {
    let rest = request
        .url
        .strip_prefix("http://")
        .ok_or_else(|| "only http:// URLs are supported for direct requests".to_string())?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let target = if authority.contains(':') { authority.to_string() } else { format!("{authority}:80") };
    let mut stream = tokio::time::timeout(Duration::from_secs(10), tokio::net::TcpStream::connect(&target))
        .await
        .map_err(|_| format!("connecting to {target} timed out"))?
        .map_err(|e| format!("{target}: {e}"))?;
    let method = if request.method.is_empty() { "GET" } else { request.method.as_str() };
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\nUser-Agent: Open DPhone\r\n");
    for (name, value) in &request.headers {
        if name.contains(['\r', '\n']) || value.contains(['\r', '\n']) {
            return Err("header contains a line break".into());
        }
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if let Some((user, pass)) = &request.credentials {
        head.push_str(&format!("Authorization: Basic {}\r\n", base64(format!("{user}:{pass}").as_bytes())));
    }
    head.push_str(&format!("Content-Length: {}\r\n\r\n", request.body.len()));
    let io = async {
        stream.write_all(head.as_bytes()).await?;
        stream.write_all(&request.body).await?;
        let mut raw = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = stream.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            raw.extend_from_slice(&buf[..n]);
            if raw.len() > HTTP_LIMIT {
                return Err(std::io::Error::other("reply too large"));
            }
        }
        Ok(raw)
    };
    let raw = tokio::time::timeout(Duration::from_secs(30), io)
        .await
        .map_err(|_| "the request timed out".to_string())?
        .map_err(|e| e.to_string())?;
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or("malformed reply")?;
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.split_ascii_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or("malformed status line")?;
    let headers: Vec<(String, String)> =
        lines.filter_map(|l| l.split_once(':')).map(|(n, v)| (n.trim().to_string(), v.trim().to_string())).collect();
    let mut body = raw[split + 4..].to_vec();
    if headers.iter().any(|(n, v)| n.eq_ignore_ascii_case("transfer-encoding") && v.eq_ignore_ascii_case("chunked")) {
        body = dechunk(&body).ok_or("malformed chunked body")?;
    }
    Ok(HttpReply { status, headers, body })
}

fn dechunk(mut data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let end = data.windows(2).position(|w| w == b"\r\n")?;
        let size_text = std::str::from_utf8(&data[..end]).ok()?;
        let size = usize::from_str_radix(size_text.split(';').next()?.trim(), 16).ok()?;
        data = &data[end + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(data.get(..size)?);
        data = data.get(size + 2..)?;
    }
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

impl HttpClient for Http {
    fn request(&self, request: HttpRequest, done: Completion<Result<HttpReply, String>>) {
        self.runtime.spawn(async move { done.complete(http(request).await) });
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

impl ConfigSource for AppConfig {
    fn app_config_xml(&self, app: &str) -> String {
        self.application(app).map(|e| e.to_document()).unwrap_or_default()
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

pub fn start(ctx: &Context, ui: Option<UiHandle>, config: Option<SharedConfig>, telephony: Telephony) {
    ctx.store.set_health(Subsystem::Apps, HealthStatus::Starting);
    let Some(ui) = ui else {
        warn!("apps: no UI; the app runtime is not started");
        ctx.store.set_health(Subsystem::Apps, HealthStatus::Failed("no UI".into()));
        return;
    };
    let manager_config = if ctx.sim {
        match std::env::var_os(APPS_ROOT_ENV) {
            Some(root) => ManagerConfig::rooted(&PathBuf::from(root), &ctx.paths.nvdata, &ctx.paths.run),
            None => {
                info!("apps: simulator without {APPS_ROOT_ENV}; no built-in apps");
                ManagerConfig::rooted(&ctx.paths.run.join("no-apps"), &ctx.paths.nvdata, &ctx.paths.run)
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
    let services = Services {
        pbx: Arc::new(Pbx { telephony, runtime: runtime.clone() }),
        http: Arc::new(Http { runtime }),
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
    fn encodings() {
        assert_eq!(base64(b"user:pass"), "dXNlcjpwYXNz");
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(dechunk(b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n").unwrap(), b"Wikipedia");
        assert!(dechunk(b"zz\r\n").is_none());
    }
}
