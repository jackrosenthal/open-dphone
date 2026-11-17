//! Provisioned firmware reaches the updater: the real binary in simulator
//! mode, provisioned over SIP on loopback by the in-process DPMA server of
//! `dphone-dpma`'s tests, with a configuration that names a firmware image.
//!
//! The DPMA client is created long after the main role's start-up code has
//! run (it needs an address and SIP), so the updater has to be handed to it
//! when it appears.  The simulator has a flash (in memory) but is not
//! running from a firmware bank, so the check ends where the updater says
//! it cannot tell what is running: nothing is downloaded or written, and
//! the log line shows that the updater was asked.  Without the hand-over
//! the client has no updater and that line never comes.
//!
//! With "ignore firmware provisioned from DPMA" on, the phone says it
//! ignores the firmware and the firmware entries are not saved: set on the
//! phone before boot (`/nvdata/dphone_settings.txtpb`, what a bench phone
//! relies on), or by the very configuration that names the firmware.

#[path = "../../dphone-dpma/tests/common/mod.rs"]
mod common;

use common::FakeServer;
use dphone_dpma::Message;
use dphone_sip::{NullMedia, OutMessage, SipConfig};
use dphone_state::{Model, Store};
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_dphone");

const FIRMWARE: &str = "<firmwares>\n<firmware model=\"D60\" version=\"2_9_28\" url=\"http://127.0.0.1:9/2_9_28_D60_firmware.eff\" network_id=\"n\" />\n</firmwares>";
const IGNORE: &str = "<setting id=\"dphone_ignore_provisioned_firmware\" value=\"1\" />";

/// What the firmware check logs when it has asked the updater for the
/// running image (and, in the simulator, got no answer).
const UPDATER_ASKED: &str = "provisioned firmware not checked: the running image's checksum is not known";
const IGNORED: &str = "ignoring provisioned firmware 2_9_28";

fn config(extra: &str) -> String {
    format!("<config>\n<setting id=\"login_password\" value=\"789\" />\n{extra}\n{FIRMWARE}\n<accounts><account id=\"100\" username=\"100\"/></accounts>\n</config>")
}

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Prints the phone's log when the test that holds it fails.
struct ShowOnPanic(PathBuf);

impl Drop for ShowOnPanic {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("--- {} ---\n{}", self.0.display(), std::fs::read_to_string(&self.0).unwrap_or_default());
        }
    }
}

fn wait_for<T>(what: &str, timeout: Duration, mut check: impl FnMut() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(v) = check() {
            return v;
        }
        assert!(start.elapsed() < timeout, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn diag(socket: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new(BIN).args(args).env("DPHONE_DIAG_SOCKET", socket).output().unwrap();
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn free_udp_port() -> u16 {
    std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port()
}

/// The DPMA server behind a SIP endpoint of its own, on a thread with its
/// own runtime.  Returns its SIP port.
fn serve(fake: Arc<Mutex<FakeServer>>) -> u16 {
    let (port_tx, port_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async move {
            let mut cfg = SipConfig::new(Model::D60, IpAddr::V4(Ipv4Addr::LOCALHOST));
            cfg.local_port = 0;
            let sip = dphone_sip::start(Store::new(Model::D60), cfg, Arc::new(NullMedia::default()), None).await.unwrap();
            let mut inbox = sip.messages();
            port_tx.send(sip.local_port()).unwrap();
            while let Some(m) = inbox.recv().await {
                let message = Message {
                    headers: m.headers.iter().map(|(k, v)| (k.clone(), v.clone().into_bytes())).collect(),
                    body: m.body.clone(),
                };
                let out = fake.lock().unwrap().handle(message);
                for o in out {
                    let reply = OutMessage {
                        account: None,
                        to: m.contact_uri.clone(),
                        from_user: Some("proxy".into()),
                        headers: o.headers.iter().map(|(k, v)| (k.clone(), String::from_utf8_lossy(v).into_owned())).collect(),
                        content_type: "text/plain".into(),
                        body: o.body.clone(),
                    };
                    let _ = sip.send_message(reply).await;
                }
            }
        });
    });
    port_rx.recv_timeout(Duration::from_secs(30)).expect("the server's SIP endpoint")
}

/// One boot of the simulator against a server that provisions `document`.
/// Returns the configuration the phone saved, taken once `done` says the
/// phone's log is complete.
fn boot(document: &str, local_ignore: bool, done: impl Fn(&str) -> bool) -> String {
    let mut fake = FakeServer::new();
    fake.token_reqs = "mac".into();
    fake.userlist_reqs = "disabled".into();
    fake.config = document.as_bytes().to_vec();
    let server_port = serve(Arc::new(Mutex::new(fake)));

    let dir = tempfile::Builder::new().prefix("dp").tempdir().unwrap();
    let (nv, run) = (dir.path().join("nv"), dir.path().join("run"));
    let phone_log = dir.path().join("phone.log");
    let _show = ShowOnPanic(phone_log.clone());
    // The server saved as stock saves it, and a SIP port of the phone's
    // own, so that nothing else on this machine is in the way.
    write(
        &nv.join("configurator.xml"),
        &format!("<config>\n    <setting id=\"config_server_url\" value=\"sip:proxy@127.0.0.1:{server_port}\" />\n</config>\n"),
    );
    write(
        &nv.join("ui-config.xml"),
        &format!("<config>\n    <setting id=\"transport_udp_port\" value=\"{}\" />\n</config>\n", free_udp_port()),
    );
    if local_ignore {
        write(&nv.join("dphone_settings.txtpb"), "ignore_provisioned_firmware: true\n");
    }
    let _main = KillOnDrop(
        Command::new(BIN)
            .args(["main", "--sim", "--model", "d60", "--no-ssh", "--nvdata"])
            .arg(&nv)
            .arg("--run-dir")
            .arg(&run)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&phone_log).unwrap())
            .spawn()
            .unwrap(),
    );
    let socket = run.join("diag.sock");
    wait_for("the diag socket", Duration::from_secs(30), || socket.exists().then_some(()));
    let provisioning = wait_for("provisioning", Duration::from_secs(90), || {
        diag(&socket, &["state", "provisioning"]).filter(|s| s.contains("phase: Provisioned"))
    });
    assert!(provisioning.contains("\"2_9_28\""), "the offer is reported either way: {provisioning}");
    let read_log = || std::fs::read_to_string(&phone_log).unwrap_or_default();
    wait_for("the firmware decision in the log", Duration::from_secs(30), || done(&read_log()).then_some(()));
    std::fs::read_to_string(nv.join("fetched-config.xml")).unwrap()
}

#[test]
fn provisioned_firmware_reaches_the_updater_unless_ignored() {
    // By default the phone follows the firmware its server names: the
    // entry is saved, and the updater is asked what is running.
    let saved = boot(&config(""), false, |log| log.contains(UPDATER_ASKED));
    assert!(saved.contains("<firmware "), "{saved}");

    // The setting stored on the phone before boot.
    let saved = boot(&config(""), true, |log| log.contains(IGNORED));
    assert!(!saved.contains("<firmware"), "{saved}");

    // The setting in the configuration that names the firmware.
    let saved = boot(&config(IGNORE), false, |log| log.contains(IGNORED));
    assert!(!saved.contains("<firmware"), "{saved}");
    assert!(saved.contains("dphone_ignore_provisioned_firmware"), "{saved}");
}
