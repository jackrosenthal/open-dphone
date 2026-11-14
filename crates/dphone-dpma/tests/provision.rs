//! The provisioning state machine against an in-process DPMA server made
//! of `dpma-proto`'s server-side pieces.

mod common;

use common::{wait, FakeServer, Mem, MemSink, MemStorage};
use dphone_dpma::xml::Configurator;
use dphone_dpma::{files, DpmaConfig, DpmaError, DpmaHandle};
use dphone_state::{Command, DiscoveredServer, DiscoverySource, Event, Model, ProvisionPhase, Provisioning, Store, Topic};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

const CONFIG: &str = "<config>\n<setting id=\"login_password\" value=\"789\" />\n<firmwares>\n<firmware model=\"D60\" version=\"2_9_28\" url=\"http://pbx/f.eff\" network_id=\"n\" />\n</firmwares>\n<accounts><account id=\"100\" username=\"100\"/></accounts>\n</config>";

struct Rig {
    store: Store,
    server: Arc<Mutex<FakeServer>>,
    transport: Arc<Mem>,
    storage: Arc<MemStorage>,
    sink: Arc<MemSink>,
    dpma: DpmaHandle,
    to_phone: mpsc::UnboundedSender<dphone_dpma::Message>,
}

fn rig_with(server: Arc<Mutex<FakeServer>>, storage: Arc<MemStorage>, sink: Arc<MemSink>) -> Rig {
    let store = Store::new(Model::D60);
    let (to_phone, incoming) = mpsc::unbounded_channel();
    let transport = Arc::new(Mem { server: server.clone(), to_phone: to_phone.clone(), down: Mutex::new(false) });
    let mut cfg = DpmaConfig::new("00:0F:D3:AA:BB:CC", "Digium D60 3_0_0");
    cfg.timeout = Duration::from_millis(500);
    cfg.retry_secs = 1;
    cfg.ping_secs = 0;
    let dpma = dphone_dpma::start(store.clone(), cfg, transport.clone(), incoming, storage.clone(), sink.clone());
    Rig { store, server, transport, storage, sink, dpma, to_phone }
}

fn rig() -> Rig {
    let mut s = FakeServer::new();
    s.config = CONFIG.as_bytes().to_vec();
    rig_with(Arc::new(Mutex::new(s)), Arc::new(MemStorage::default()), Arc::new(MemSink::default()))
}

fn phase(store: &Store) -> ProvisionPhase {
    store.get::<Provisioning>().phase.clone()
}

async fn until(store: &Store, what: &str, want: ProvisionPhase) {
    wait(what, || (phase(store) == want).then_some(())).await;
}

#[tokio::test(flavor = "current_thread")]
async fn first_boot_user_token_config_and_pushes() {
    let r = rig();
    let events = r.store.subscribe_events(Topic::Provisioning);
    let pushes = |seen: &mut Vec<(String, String)>| {
        while let Some(w) = events.try_take() {
            for e in w.events {
                if let Event::Push { name, body } = e {
                    seen.push((name, body));
                }
            }
        }
    };
    let mut seen = Vec::new();

    // No server known: wait for the user.
    r.dpma.network(true, None, Vec::new());
    until(&r.store, "awaiting a server", ProvisionPhase::AwaitingServer).await;

    // DHCP option 66 in its sip: form names the server.
    r.dpma.network(true, Some("sip:proxy@pbx.test:5060".into()), Vec::new());
    until(&r.store, "awaiting a user", ProvisionPhase::AwaitingUser).await;
    let p = r.store.get::<Provisioning>();
    assert_eq!(p.server.as_ref().unwrap().source, DiscoverySource::DhcpOption66);
    assert_eq!(p.users.len(), 1);
    assert_eq!((p.users[0].id.as_str(), p.users[0].name.as_str(), p.users[0].needs_pin), ("100", "Desk", true));
    assert_eq!(r.server.lock().unwrap().user_agent, "Digium D60 3_0_0");
    // The identity was minted and stored; the server verified it against
    // the provisioning CA.
    assert!(r.storage.read_file(files::IDENTITY_CERT).starts_with(b"-----BEGIN CERTIFICATE-----"));
    assert!(r.storage.read_file(files::IDENTITY_KEY).starts_with(b"-----BEGIN RSA PRIVATE KEY-----"));

    // A wrong PIN is reported with the stock error code.
    r.store.send(Command::SelectUser { id: "100".into(), pin: "0000".into() }).await.unwrap();
    wait("PIN refused", || matches!(phase(&r.store), ProvisionPhase::Failed { code: Some(-199), .. }).then_some(())).await;

    // The right one provisions.
    r.store.send(Command::SelectUser { id: "100".into(), pin: "1234".into() }).await.unwrap();
    until(&r.store, "provisioned", ProvisionPhase::Provisioned).await;
    let p = r.store.get::<Provisioning>();
    assert!(p.attached);
    assert_eq!(p.assigned_user, "100");
    assert_eq!(p.offered_firmware.as_deref(), Some("2_9_28"));
    let applied = r.sink.applied.lock().unwrap().clone();
    assert_eq!(applied.len(), 1);
    let text = String::from_utf8(applied[0].clone()).unwrap();
    assert!(text.contains("login_password") && text.contains("<accounts>"));
    assert!(text.contains("<firmware model=\"D60\""), "provisioned firmware is passed on by default");
    // No digest from the server: the cache holds the MD5 of the download.
    assert_eq!(p.config_md5.len(), 32);
    assert_eq!(r.sink.md5.lock().unwrap().as_deref(), Some(p.config_md5.as_str()));
    // configurator.xml carries stock's settings.
    let c = Configurator::parse(&r.sink.configurator.lock().unwrap().clone().unwrap());
    assert_eq!(c.get("config_server_type"), Some("boot66"));
    assert_eq!(c.get("config_server_url"), Some("sip:proxy@pbx.test:5060"));
    assert_eq!(c.get("config_server_user"), Some("100"));
    assert_eq!(c.get("config_user_password"), Some("1234"));
    assert!(c.get("config_server_token").unwrap().starts_with("100/000FD3AABBCC/"));

    // App requests ride the session.
    let doc = br#"{"request":{"method":"switchvox.users.voicemail.getList","parameters":{"account_id":"100"}}}"#;
    let reply = r.dpma.app_request("dasp://pbx.test/json", Some("100:1234"), doc).await.unwrap();
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, br#"{"response":{"result":{"messages":[]}}}"#);
    assert_eq!(reply.headers[0].0, "Content-Type");
    assert_eq!(r.dpma.file("ringtone1").await.unwrap(), b"RIFFdata");
    assert_eq!(r.dpma.app_request("file://ringtone1", None, b"").await.unwrap().body, b"RIFFdata");
    assert!(matches!(r.dpma.file("nothing").await, Err(DpmaError::Server { code: -98, .. })));

    // digium.config.update: fetch again with the same token.
    r.server.lock().unwrap().config = CONFIG.replace("789", "555").into_bytes();
    let push = r.server.lock().unwrap().event("digium.config.update", b"<config></config>");
    for m in push {
        r.to_phone.send(m).unwrap();
    }
    wait("second download", || (r.sink.applied.lock().unwrap().len() == 2).then_some(())).await;
    assert!(String::from_utf8_lossy(&r.sink.applied.lock().unwrap()[1]).contains("555"));
    assert_eq!(r.server.lock().unwrap().count("TokenRequest"), 2, "one refused, one accepted, none for the push");
    // The update itself and other server events are passed on.
    let push = r.server.lock().unwrap().event("digium.voicemail.play.location.change", b"{\"position\":3}");
    for m in push {
        r.to_phone.send(m).unwrap();
    }
    wait("push events", || {
        pushes(&mut seen);
        (seen.len() >= 2).then_some(())
    })
    .await;
    assert_eq!(seen[0].0, "digium.config.update");
    assert_eq!(seen[1], ("digium.voicemail.play.location.change".to_string(), "{\"position\":3}".to_string()));

    // The server forgets the session: the next request gets -398 and the
    // client handshakes again by itself and re-verifies its token.
    r.server.lock().unwrap().forget_sessions();
    let handshakes = r.server.lock().unwrap().count("Handshake");
    assert!(matches!(r.dpma.file("ringtone1").await, Err(DpmaError::Server { code: -398, .. })));
    wait("new handshake", || (r.server.lock().unwrap().count("Handshake") > handshakes).then_some(())).await;
    until(&r.store, "provisioned again", ProvisionPhase::Provisioned).await;
    assert_eq!(r.dpma.file("ringtone1").await.unwrap(), b"RIFFdata");

    // A session the server holds but refuses: its -398 comes encrypted
    // under a password we do not share, and is still understood.
    r.server.lock().unwrap().rekey_sessions();
    let handshakes = r.server.lock().unwrap().count("Handshake");
    assert!(matches!(r.dpma.file("ringtone1").await, Err(DpmaError::Server { code: -398, .. })));
    wait("handshake after an undecryptable error", || (r.server.lock().unwrap().count("Handshake") > handshakes).then_some(())).await;
    until(&r.store, "provisioned a third time", ProvisionPhase::Provisioned).await;

    // Reassignment: back to the handshake, token and user dropped.
    let push = r.server.lock().unwrap().reassign();
    for m in push {
        r.to_phone.send(m).unwrap();
    }
    until(&r.store, "awaiting a user after reassignment", ProvisionPhase::AwaitingUser).await;
    let c = Configurator::parse(&r.sink.configurator.lock().unwrap().clone().unwrap());
    assert_eq!(c.get("config_server_token"), None);
}

#[tokio::test(flavor = "current_thread")]
async fn later_boot_md5_cache() {
    // First boot, MAC-keyed assignment: no user interaction at all.
    let mut s = FakeServer::new();
    s.config = CONFIG.as_bytes().to_vec();
    s.token_reqs = "mac".into();
    s.userlist_reqs = "disabled".into();
    s.md5 = Some("0123456789abcdef0123456789abcdef".into());
    let server = Arc::new(Mutex::new(s));
    let (storage, sink) = (Arc::new(MemStorage::default()), Arc::new(MemSink::default()));
    let mdns = vec![DiscoveredServer {
        source: DiscoverySource::Mdns,
        name: "Digium Phone Config Server".into(),
        address: "sip:proxy@10.0.0.5:5060".into(),
    }];
    let r = rig_with(server.clone(), storage.clone(), sink.clone());
    r.dpma.network(true, None, mdns.clone());
    until(&r.store, "provisioned by MAC", ProvisionPhase::Provisioned).await;
    // The server's digest is stored verbatim.
    assert_eq!(sink.md5.lock().unwrap().as_deref(), Some("0123456789abcdef0123456789abcdef"));
    assert_eq!(server.lock().unwrap().count("ConfigRequest"), 1);
    let identity = storage.read_file(files::IDENTITY_CERT);
    drop(r);

    // Next boot: same storage.  The saved server is used without
    // discovery, the token verifies, the digest matches: no download.
    let r = rig_with(server.clone(), storage.clone(), sink.clone());
    r.dpma.network(true, None, Vec::new());
    until(&r.store, "provisioned from the cache", ProvisionPhase::Provisioned).await;
    assert_eq!(r.store.get::<Provisioning>().server.as_ref().unwrap().source, DiscoverySource::Saved);
    assert_eq!(server.lock().unwrap().count("VerifyRequest"), 1);
    assert_eq!(server.lock().unwrap().count("ConfigRequest"), 1, "md5sum_match skips the download");
    assert_eq!(storage.read_file(files::IDENTITY_CERT), identity, "the identity is minted once");
    drop(r);

    // A boot after the configuration changed: digest differs, download.
    server.lock().unwrap().md5 = Some("ffffffffffffffffffffffffffffffff".into());
    let r = rig_with(server.clone(), storage.clone(), sink.clone());
    r.dpma.network(true, None, Vec::new());
    until(&r.store, "provisioned after a change", ProvisionPhase::Provisioned).await;
    assert_eq!(server.lock().unwrap().count("ConfigRequest"), 2);
    assert_eq!(sink.md5.lock().unwrap().as_deref(), Some("ffffffffffffffffffffffffffffffff"));

    // Forgetting drops the saved server, token, and identity.
    r.store.send(Command::ForgetProvisioning).await.unwrap();
    until(&r.store, "awaiting a server after forgetting", ProvisionPhase::AwaitingServer).await;
    assert!(sink.configurator.lock().unwrap().is_none());
    assert!(storage.0.lock().unwrap().get(files::IDENTITY_CERT).is_none());
    // And skipping leaves the phone standalone.
    r.store.send(Command::SkipProvisioning).await.unwrap();
    assert_eq!(phase(&r.store), ProvisionPhase::Standalone);
    assert!(matches!(r.dpma.file("x").await, Err(DpmaError::NotAttached)));
}

#[tokio::test(flavor = "current_thread")]
async fn unreachable_server_retries() {
    let mut s = FakeServer::new();
    s.config = CONFIG.as_bytes().to_vec();
    s.token_reqs = "mac".into();
    let r = rig_with(Arc::new(Mutex::new(s)), Arc::new(MemStorage::default()), Arc::new(MemSink::default()));
    *r.transport.down.lock().unwrap() = true;
    r.store.send(Command::StartProvisioning { server: Some("10.0.0.9:5060".into()) }).await.unwrap();
    r.dpma.network(true, None, Vec::new());
    wait("failure", || matches!(phase(&r.store), ProvisionPhase::Failed { .. }).then_some(())).await;
    assert_eq!(r.store.get::<Provisioning>().server.as_ref().unwrap().source, DiscoverySource::Manual);
    // The server comes back; the retry timer provisions without help.
    *r.transport.down.lock().unwrap() = false;
    until(&r.store, "provisioned on retry", ProvisionPhase::Provisioned).await;
    let c = Configurator::parse(&r.sink.configurator.lock().unwrap().clone().unwrap());
    assert_eq!(c.get("config_server_manual"), Some("1"));
    assert_eq!(c.get("config_server_type"), Some("digium"));
}

trait ReadFile {
    fn read_file(&self, name: &str) -> Vec<u8>;
}
impl ReadFile for MemStorage {
    fn read_file(&self, name: &str) -> Vec<u8> {
        self.0.lock().unwrap().get(name).cloned().unwrap_or_default()
    }
}

/// Counts how often the firmware check asks what is running.
struct CountingInstaller(std::sync::atomic::AtomicUsize);
impl dphone_dpma::firmware::FirmwareInstaller for CountingInstaller {
    fn running_image(&self) -> Result<dphone_dpma::firmware::ImageSum, String> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err("test".into())
    }
    fn install(&self, _: &mut dyn std::io::Read) -> Result<(), dphone_dpma::firmware::InstallError> {
        unreachable!("the running image is unknown, so nothing may be installed")
    }
}
struct NoSource;
impl dphone_dpma::firmware::FirmwareSource for NoSource {
    fn open(&self, _: &str, _: &str) -> Result<Box<dyn std::io::Read + Send>, dphone_dpma::firmware::FetchError> {
        Err(dphone_dpma::firmware::FetchError { status: None, message: "test".into() })
    }
}

#[tokio::test(flavor = "current_thread")]
async fn provisioned_firmware_is_checked_unless_ignored() {
    let mut s = FakeServer::new();
    s.config = CONFIG.as_bytes().to_vec();
    s.token_reqs = "mac".into();
    let server = Arc::new(Mutex::new(s));
    let r = rig_with(server.clone(), Arc::new(MemStorage::default()), Arc::new(MemSink::default()));
    let installer = Arc::new(CountingInstaller(Default::default()));
    r.dpma.set_firmware_updater(Arc::new(NoSource), installer.clone());
    let checks = || installer.0.load(std::sync::atomic::Ordering::SeqCst);

    // By default the provisioned firmware is respected: the entry is kept
    // and the check runs after the configuration is applied.
    r.dpma.network(true, Some("sip:proxy@pbx.test".into()), Vec::new());
    until(&r.store, "provisioned", ProvisionPhase::Provisioned).await;
    wait("the firmware check", || (checks() == 1).then_some(())).await;
    assert!(String::from_utf8_lossy(&r.sink.applied.lock().unwrap()[0]).contains("<firmware model=\"D60\""));
    assert_eq!(r.store.get::<Provisioning>().offered_firmware.as_deref(), Some("2_9_28"));

    // With the development setting on, the entry is dropped and nothing
    // is checked; the offer is still reported.
    r.dpma.set_ignore_provisioned_firmware(true);
    r.store.send(Command::Reprovision).await.unwrap();
    wait("second fetch", || (r.sink.applied.lock().unwrap().len() == 2).then_some(())).await;
    until(&r.store, "provisioned again", ProvisionPhase::Provisioned).await;
    assert!(!String::from_utf8_lossy(&r.sink.applied.lock().unwrap()[1]).contains("firmware"));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(checks(), 1, "no check while ignoring");
    assert_eq!(r.store.get::<Provisioning>().offered_firmware.as_deref(), Some("2_9_28"));

    // And back.
    r.dpma.set_ignore_provisioned_firmware(false);
    r.store.send(Command::Reprovision).await.unwrap();
    wait("the check resumes", || (checks() == 2).then_some(())).await;
}

/// Stock and this firmware share the saved token in `configurator.xml`,
/// and the server ties a token to the spelling of the MAC it was last
/// presented with.  A token stock obtained must carry this client through
/// a whole boot (verify, then download), and must still be good for stock
/// afterward.
#[tokio::test(flavor = "current_thread")]
async fn a_token_saved_by_stock_stays_valid() {
    const STOCK_MAC: &[u8] = b"000FD3AABBCC";
    let mut s = FakeServer::new();
    s.config = CONFIG.as_bytes().to_vec();
    s.token_reqs = "mac".into();
    s.userlist_reqs = "disabled".into();
    // Stock provisioned the phone and left its state in nvdata.
    let token = String::from_utf8(s.issue_token("100", STOCK_MAC)).unwrap();
    let server = Arc::new(Mutex::new(s));
    let (storage, sink) = (Arc::new(MemStorage::default()), Arc::new(MemSink::default()));
    let mut c = Configurator::default();
    c.set("config_server_type", "digium");
    c.set("config_server_url", "sip:proxy@10.0.0.5:5060");
    c.set("config_server_user", "");
    c.set("config_server_token", &token);
    *sink.configurator.lock().unwrap() = Some(c.render());

    // Two boots of this firmware (a main-role restart and a reboot look
    // the same to the server: a new handshake, then the saved token).
    for boot in 1..=2 {
        let r = rig_with(server.clone(), storage.clone(), sink.clone());
        r.dpma.network(true, None, Vec::new());
        until(&r.store, "provisioned with the saved token", ProvisionPhase::Provisioned).await;
        let s = server.lock().unwrap();
        assert_eq!(s.count("VerifyRequest"), boot);
        assert_eq!(s.count("ConfigRequest"), boot, "the download follows the verification with the same token");
        assert_eq!(s.count("TokenRequest"), 0, "the saved token was never refused");
        assert!(s.macs.iter().all(|m| m == STOCK_MAC), "every request spells the MAC as stock does: {:?}", s.macs);
        drop(s);
        drop(r);
    }
    assert_eq!(sink.applied.lock().unwrap().len(), 2);
    let saved = Configurator::parse(&sink.configurator.lock().unwrap().clone().unwrap());
    assert_eq!(saved.get("config_server_token"), Some(token.as_str()), "the saved token is left as stock wrote it");

    // Stock boots next: VerifyRequest, then ConfigRequest, both with the
    // saved token and its own MAC spelling.
    let mut s = server.lock().unwrap();
    assert!(s.present_token(token.as_bytes(), STOCK_MAC), "stock's VerifyRequest");
    assert!(s.present_token(token.as_bytes(), STOCK_MAC), "stock's ConfigRequest");
}

/// A token of our own is spelled so that stock can use it.
#[tokio::test(flavor = "current_thread")]
async fn a_token_we_save_is_good_for_stock() {
    let mut s = FakeServer::new();
    s.config = CONFIG.as_bytes().to_vec();
    s.token_reqs = "mac".into();
    s.userlist_reqs = "disabled".into();
    let server = Arc::new(Mutex::new(s));
    let (storage, sink) = (Arc::new(MemStorage::default()), Arc::new(MemSink::default()));
    let r = rig_with(server.clone(), storage.clone(), sink.clone());
    r.store.send(Command::StartProvisioning { server: Some("10.0.0.9:5060".into()) }).await.unwrap();
    r.dpma.network(true, None, Vec::new());
    until(&r.store, "provisioned", ProvisionPhase::Provisioned).await;
    let saved = Configurator::parse(&sink.configurator.lock().unwrap().clone().unwrap());
    let token = saved.get("config_server_token").unwrap().to_string();
    assert!(token.starts_with("100/000FD3AABBCC/"), "{token}");
    let mut s = server.lock().unwrap();
    assert!(s.present_token(token.as_bytes(), b"000FD3AABBCC"), "stock's VerifyRequest");
    assert!(s.present_token(token.as_bytes(), b"000FD3AABBCC"), "stock's ConfigRequest");
    // The server's rule, for the record: the other spelling passes once
    // and is refused from then on.
    assert!(s.present_token(token.as_bytes(), b"000fd3aabbcc"));
    assert!(!s.present_token(token.as_bytes(), b"000fd3aabbcc"));
}
