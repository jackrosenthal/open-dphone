//! The two run-time switches of the web server: web configuration
//! (`web_ui_enabled`) and the metrics endpoint (`metrics_enabled`).  The
//! server listens in every combination.

use dphone_config::{ConfigStore, Nvdata};
use dphone_state::{Command, Domain, Model, Reply, Store};
use dphone_web::{WebOptions, WebServer, WebSwitch};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

struct Phone {
    addr: SocketAddr,
    store: Store,
    config: Arc<Mutex<ConfigStore>>,
    switch: WebSwitch,
    dir: PathBuf,
}

impl Drop for Phone {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A phone whose switches start as its configuration has them, after
/// `prepare` has had its way with the configuration.
async fn phone(prepare: impl FnOnce(&mut ConfigStore)) -> Phone {
    let mut id = [0u8; 8];
    getrandom::fill(&mut id).unwrap();
    let dir = std::env::temp_dir().join(format!("dphone-web-switch-{}", id.map(|b| format!("{b:02x}")).concat()));
    let nv = Nvdata::new(dir.join("nvdata"));
    nv.ensure_layout().unwrap();
    let mut config = ConfigStore::open(nv, dphone_config::Model::D60, dir.join("tmp")).unwrap();
    prepare(&mut config);
    let switch = WebSwitch::from_config(&config.config());
    let config = Arc::new(Mutex::new(config));
    let store = Store::new(Model::D60);
    let options = WebOptions {
        http: Some("127.0.0.1:0".parse().unwrap()),
        https: None,
        upload_dir: dir.join("upload"),
        ..Default::default()
    };
    let bound = WebServer::new(store.clone(), config.clone(), options).with_switch(switch.clone()).bind().await.unwrap();
    let addr = bound.http_addr().unwrap();
    tokio::spawn(bound.serve());
    Phone { addr, store, config, switch, dir }
}

impl Phone {
    /// What the main role does on every configuration change.
    fn follow(&self) {
        self.switch.follow(&self.config.lock().unwrap().config());
    }

    fn provision(&self, settings: &str) {
        self.config.lock().unwrap().apply_fetched(&format!("<config>{settings}</config>")).unwrap();
        self.follow();
    }
}

struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Resp {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| panic!("{e}: {:?}", String::from_utf8_lossy(&self.body)))
    }
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

fn dechunk(mut data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(eol) = data.windows(2).position(|w| w == b"\r\n") {
        let Ok(n) = usize::from_str_radix(std::str::from_utf8(&data[..eol]).unwrap_or("").trim(), 16) else { break };
        data = &data[eol + 2..];
        if n == 0 || data.len() < n {
            break;
        }
        out.extend_from_slice(&data[..n]);
        data = &data[(n + 2).min(data.len())..];
    }
    out
}

async fn request(addr: SocketAddr, method: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> Resp {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
    for (n, v) in headers {
        head.push_str(&format!("{n}: {v}\r\n"));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes()).await.unwrap();
    s.write_all(body).await.unwrap();
    let mut raw = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), s.read_to_end(&mut raw)).await.expect("response in time").unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("header end");
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
    let headers: Vec<(String, String)> =
        lines.filter_map(|l| l.split_once(':').map(|(n, v)| (n.trim().to_owned(), v.trim().to_owned()))).collect();
    let chunked = headers.iter().any(|(n, v)| n.eq_ignore_ascii_case("transfer-encoding") && v.contains("chunked"));
    let body = if chunked { dechunk(&raw[split + 4..]) } else { raw[split + 4..].to_vec() };
    Resp { status, headers, body }
}

async fn get(addr: SocketAddr, path: &str) -> Resp {
    request(addr, "GET", path, &[], b"").await
}

struct Session {
    cookie: String,
    csrf: String,
}

async fn post(addr: SocketAddr, s: Option<&Session>, path: &str, body: Value) -> Resp {
    let mut headers = vec![("Content-Type", "application/json")];
    if let Some(s) = s {
        headers.push(("Cookie", &s.cookie));
        headers.push(("X-Csrf-Token", &s.csrf));
    }
    request(addr, "POST", path, &headers, body.to_string().as_bytes()).await
}

async fn login(addr: SocketAddr) -> Result<Session, Resp> {
    let r = post(addr, None, "/dphone.v1.AuthService/Login", json!({ "user": "admin", "password": "789" })).await;
    if r.status != 200 {
        return Err(r);
    }
    let cookie = r.header("set-cookie").unwrap().split(';').next().unwrap().to_owned();
    let csrf = r.json()["session"]["csrfToken"].as_str().unwrap().to_owned();
    Ok(Session { cookie, csrf })
}

/// Everything that must be refused while web configuration is disabled,
/// with and without the credentials of a session opened before.
async fn assert_closed(addr: SocketAddr, old: Option<&Session>) {
    for session in [None, old] {
        for path in [
            "/dphone.v1.AuthService/Login",
            "/dphone.v1.AuthService/Logout",
            "/dphone.v1.AuthService/ChangePassword",
            "/dphone.v1.StatusService/GetStatus",
            "/dphone.v1.StatusService/WatchStatus",
            "/dphone.v1.ConfigService/GetSettings",
            "/dphone.v1.SystemService/Reboot",
            "/dphone.v1.SystemService/SetMetricsEnabled",
            "/dphone.v1.ShellService/OpenShell",
            "/dphone.v1.ShellService/WriteShell",
            "/dphone.v1.ScreenService/WatchScreen",
            "/dphone.v1.ScreenService/PressKey",
            "/dphone.v1.SshService/AddSshKey",
            "/upload/firmware",
            "/upload/app",
            "/anything-else",
        ] {
            let r = post(addr, session, path, json!({ "user": "admin", "password": "789" })).await;
            assert_eq!(r.status, 403, "{path}: {}", r.text());
            assert_eq!(r.json()["code"], "permission_denied", "{path}");
            assert_eq!(r.header("set-cookie"), None, "{path}");
        }
        // The one question that is answered: the state, and nothing else.
        let r = post(addr, session, "/dphone.v1.AuthService/GetSession", json!({})).await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json(), json!({ "session": { "webDisabled": true } }));
    }
    // The frontend's files are still served, so that it can say so.
    let r = get(addr, "/").await;
    assert_eq!(r.status, 200);
    assert!(r.header("content-type").unwrap().starts_with("text/html"));
}

async fn assert_open(addr: SocketAddr) -> Session {
    let r = post(addr, None, "/dphone.v1.AuthService/GetSession", json!({})).await;
    assert_eq!(r.status, 200);
    let v = r.json();
    assert_eq!(v["session"]["model"], "D60");
    assert!(v["session"].get("webDisabled").is_none(), "{v}");
    // Without a session the services want a login, as ever.
    let r = post(addr, None, "/dphone.v1.StatusService/GetStatus", json!({})).await;
    assert_eq!((r.status, r.json()["code"].as_str().map(str::to_owned)), (401, Some("unauthenticated".into())));
    let s = login(addr).await.unwrap_or_else(|r| panic!("login: {}", r.text()));
    let r = post(addr, Some(&s), "/dphone.v1.StatusService/GetStatus", json!({})).await;
    assert_eq!(r.status, 200, "{}", r.text());
    s
}

async fn assert_metrics(addr: SocketAddr, served: bool) {
    let r = get(addr, "/metrics").await;
    if served {
        assert_eq!(r.status, 200);
        assert!(r.text().contains("dphone_build_info"), "{}", r.text());
    } else {
        assert_eq!(r.status, 404);
        assert!(!r.text().contains("dphone_"));
    }
    let r = request(addr, "HEAD", "/metrics", &[], b"").await;
    assert_eq!(r.status, if served { 200 } else { 404 });
}

const WEB_OFF: &str = r#"<setting id="web_ui_enabled" value="0"/>"#;
const WEB_ON: &str = r#"<setting id="web_ui_enabled" value="1"/>"#;
const METRICS_OFF: &str = r#"<setting id="dphone_metrics_enabled" value="0"/>"#;

/// All four combinations, each as the state the server starts in.
#[tokio::test]
async fn four_combinations_at_start() {
    for (web, metrics) in [(true, true), (true, false), (false, true), (false, false)] {
        let p = phone(|c| {
            let mut settings = String::new();
            if !web {
                settings.push_str(WEB_OFF);
            }
            if !metrics {
                settings.push_str(METRICS_OFF);
            }
            c.apply_fetched(&format!("<config>{settings}</config>")).unwrap();
        })
        .await;
        assert_eq!((p.switch.enabled(), p.switch.metrics_enabled()), (web, metrics));
        assert_metrics(p.addr, metrics).await;
        if web {
            assert_open(p.addr).await;
        } else {
            assert_closed(p.addr, None).await;
        }
        // Metrics are unaffected by what was just asked of the rest.
        assert_metrics(p.addr, metrics).await;
    }
}

/// The defaults: with nothing set anywhere, both are on.
#[tokio::test]
async fn defaults_are_on() {
    let p = phone(|_| {}).await;
    assert!(p.switch.enabled() && p.switch.metrics_enabled());
    assert_open(p.addr).await;
    assert_metrics(p.addr, true).await;
}

/// The switches move while the server runs, through all four combinations
/// and back, without the listener going away.
#[tokio::test]
async fn switching_at_run_time() {
    let p = phone(|_| {}).await;
    let first = assert_open(p.addr).await;

    // A stream that is open when web configuration goes off ends.
    let mut watch = TcpStream::connect(p.addr).await.unwrap();
    let frame = [0u8, 0, 0, 0, 2, b'{', b'}'];
    let head = format!(
        "POST /dphone.v1.StatusService/WatchStatus HTTP/1.1\r\nHost: {}\r\nContent-Type: application/connect+json\r\nCookie: {}\r\nX-Csrf-Token: {}\r\nContent-Length: {}\r\n\r\n",
        p.addr,
        first.cookie,
        first.csrf,
        frame.len()
    );
    watch.write_all(head.as_bytes()).await.unwrap();
    watch.write_all(&frame).await.unwrap();
    let mut buf = [0u8; 4096];
    let n = tokio::time::timeout(Duration::from_secs(10), watch.read(&mut buf)).await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&buf[..n]).starts_with("HTTP/1.1 200"), "the stream opened");

    // Web configuration off (as provisioning does it); metrics stay.
    p.provision(WEB_OFF);
    assert_closed(p.addr, Some(&first)).await;
    assert_metrics(p.addr, true).await;
    let ended = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match watch.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    })
    .await;
    assert!(ended.is_ok(), "the open stream went on after web configuration was disabled");

    // Metrics off as well.
    p.provision(&format!("{WEB_OFF}{METRICS_OFF}"));
    assert_closed(p.addr, Some(&first)).await;
    assert_metrics(p.addr, false).await;

    // Web configuration back on, metrics still off.  The session from
    // before does not come back; a new login works.
    p.provision(&format!("{WEB_ON}{METRICS_OFF}"));
    let r = post(p.addr, Some(&first), "/dphone.v1.StatusService/GetStatus", json!({})).await;
    assert_eq!(r.status, 401, "{}", r.text());
    let second = assert_open(p.addr).await;
    assert_metrics(p.addr, false).await;

    // Metrics on from the web interface: stored locally, in effect at once.
    let r = post(p.addr, Some(&second), "/dphone.v1.SystemService/GetMetricsEnabled", json!({})).await;
    assert_eq!(r.json()["metricsEnabled"], json!({ "source": "SETTING_SOURCE_PROVISIONED" }));
    let r = post(p.addr, Some(&second), "/dphone.v1.SystemService/SetMetricsEnabled", json!({ "metricsEnabled": true })).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["metricsEnabled"], json!({ "value": true, "source": "SETTING_SOURCE_LOCAL" }));
    assert!(std::fs::read_to_string(p.dir.join("nvdata/dphone_settings.txtpb")).unwrap().contains("metrics_enabled: true"));
    assert_metrics(p.addr, true).await;
    // And off again the same way.
    let r = post(p.addr, Some(&second), "/dphone.v1.SystemService/SetMetricsEnabled", json!({})).await;
    assert_eq!(r.json()["metricsEnabled"], json!({ "source": "SETTING_SOURCE_LOCAL" }));
    assert_metrics(p.addr, false).await;
    assert!(p.switch.enabled(), "the metrics switch leaves web configuration alone");
}

/// A value the provisioning server froze cannot be changed from the web
/// interface, and wins over a local one.
#[tokio::test]
async fn frozen_provisioned_metrics_setting() {
    let p = phone(|c| {
        c.update_dphone_settings(|s| s.metrics_enabled = Some(true)).unwrap();
    })
    .await;
    let s = assert_open(p.addr).await;
    assert_metrics(p.addr, true).await;

    p.provision(r#"<setting id="dphone_metrics_enabled" value="0" conflict="freeze"/>"#);
    assert_metrics(p.addr, false).await;
    let r = post(p.addr, Some(&s), "/dphone.v1.SystemService/GetMetricsEnabled", json!({})).await;
    assert_eq!(r.json()["metricsEnabled"], json!({ "source": "SETTING_SOURCE_PROVISIONED", "locked": true }));
    let r = post(p.addr, Some(&s), "/dphone.v1.SystemService/SetMetricsEnabled", json!({ "metricsEnabled": true })).await;
    assert_eq!((r.status, r.json()["code"].as_str().map(str::to_owned)), (400, Some("failed_precondition".into())));
    assert_metrics(p.addr, false).await;

    // Frozen on: it cannot be turned off here either.
    p.provision(r#"<setting id="dphone_metrics_enabled" value="1" conflict="freeze"/>"#);
    assert_metrics(p.addr, true).await;
    let r = post(p.addr, Some(&s), "/dphone.v1.SystemService/SetMetricsEnabled", json!({})).await;
    assert_eq!(r.status, 400);
    assert_metrics(p.addr, true).await;

    // The server lets go: the local value applies again.
    p.provision("");
    assert_metrics(p.addr, true).await;
    let r = post(p.addr, Some(&s), "/dphone.v1.SystemService/GetMetricsEnabled", json!({})).await;
    assert_eq!(r.json()["metricsEnabled"], json!({ "value": true, "source": "SETTING_SOURCE_LOCAL" }));
}

/// Erasing nvdata from the recovery page: guarded by the model name, and
/// handed to the main role as a command.
#[tokio::test]
async fn erase_nvdata_is_a_guarded_command() {
    let p = phone(|_| {}).await;
    let s = assert_open(p.addr).await;
    let got = Arc::new(Mutex::new(Vec::new()));
    {
        let (store, got) = (p.store.clone(), got.clone());
        tokio::spawn(async move {
            let rx = store.handle(Domain::System);
            while let Some(envelope) = rx.next().await {
                let (cmd, responder) = envelope.split();
                got.lock().unwrap().push(cmd);
                responder.reply(Ok(Reply::Done));
            }
        });
    }
    let r = post(p.addr, Some(&s), "/dphone.v1.SystemService/EraseNvdata", json!({ "confirm": "D62" })).await;
    assert_eq!(r.status, 400, "{}", r.text());
    assert!(got.lock().unwrap().is_empty());
    let r = post(p.addr, Some(&s), "/dphone.v1.SystemService/EraseNvdata", json!({ "confirm": "d60" })).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(matches!(got.lock().unwrap().as_slice(), [Command::EraseNvdata]));
}

/// Built-in apps are switched on and off one at a time; the list is one of
/// Open DPhone's own settings.
#[tokio::test]
async fn built_in_apps_are_switched_off_and_on() {
    use dphone_state::{AppInfo, Apps};
    let p = phone(|_| {}).await;
    let app = |name: &str, builtin| AppInfo { name: name.into(), builtin, ..Default::default() };
    p.store.update::<Apps, _>(|a| {
        a.installed = vec![app("calendar", true), app("extra", false)];
        a.disabled = vec![app("parking", true)];
    });
    let s = assert_open(p.addr).await;
    let set = |name: &'static str, enabled: bool| {
        let (addr, s) = (p.addr, &s);
        async move { post(addr, Some(s), "/dphone.v1.AppsService/SetAppEnabled", json!({ "name": name, "enabled": enabled })).await }
    };
    let file = || std::fs::read_to_string(p.dir.join("nvdata/dphone_settings.txtpb")).unwrap_or_default();

    // The list of apps carries the ones that are off, in one order.
    let r = post(p.addr, Some(&s), "/dphone.v1.AppsService/ListApps", json!({})).await;
    let apps: Vec<(String, bool)> = r.json()["snapshot"]["apps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| (a["name"].as_str().unwrap().to_owned(), a["disabled"].as_bool().unwrap_or(false)))
        .collect();
    assert_eq!(apps, [("calendar".into(), false), ("extra".into(), false), ("parking".into(), true)]);

    let r = post(p.addr, Some(&s), "/dphone.v1.AppsService/GetDisabledApps", json!({})).await;
    assert_eq!(r.json()["disabledApps"], json!({ "source": "SETTING_SOURCE_DEFAULT" }));
    let r = set("calendar", false).await;
    assert_eq!(r.json()["disabledApps"], json!({ "names": ["calendar"], "source": "SETTING_SOURCE_LOCAL" }));
    let r = set("parking", false).await;
    assert_eq!(r.json()["disabledApps"], json!({ "names": ["calendar", "parking"], "source": "SETTING_SOURCE_LOCAL" }));
    assert!(file().contains("disabled_apps: \"calendar,parking\""));
    // Only built-in apps.
    assert_eq!(set("extra", false).await.status, 404);
    assert_eq!(set("nope", false).await.status, 404);
    // Everything back on: the setting returns to its default.
    set("calendar", true).await;
    let r = set("parking", true).await;
    assert_eq!(r.json()["disabledApps"], json!({ "source": "SETTING_SOURCE_DEFAULT" }));
    assert!(!file().contains("disabled_apps"));

    // Over a provisioned list, switching the app on is a local, empty list.
    p.provision(r#"<setting id="dphone_disabled_apps" value="calendar"/>"#);
    let r = post(p.addr, Some(&s), "/dphone.v1.AppsService/GetDisabledApps", json!({})).await;
    assert_eq!(r.json()["disabledApps"], json!({ "names": ["calendar"], "source": "SETTING_SOURCE_PROVISIONED" }));
    let r = set("calendar", true).await;
    assert_eq!(r.json()["disabledApps"], json!({ "source": "SETTING_SOURCE_LOCAL" }));
    assert!(file().contains("disabled_apps: \"\""));

    // Frozen by the server: it cannot be changed here.
    p.provision(r#"<setting id="dphone_disabled_apps" value="parking" conflict="freeze"/>"#);
    let r = set("parking", true).await;
    assert_eq!((r.status, r.json()["code"].as_str().map(str::to_owned)), (400, Some("failed_precondition".into())));
    let r = post(p.addr, Some(&s), "/dphone.v1.AppsService/GetDisabledApps", json!({})).await;
    assert_eq!(r.json()["disabledApps"], json!({ "names": ["parking"], "source": "SETTING_SOURCE_PROVISIONED", "locked": true }));
}
