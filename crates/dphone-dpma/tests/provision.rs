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

    // DHCP option 66 in its sip: form names a server; the user picks it.
    r.dpma.network(true, Some("sip:proxy@pbx.test:5060".into()), Vec::new());
    r.store.send(Command::StartProvisioning { server: Some("sip:proxy@pbx.test:5060".into()) }).await.unwrap();
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

    // So does app data, which the server keeps for the phone the token
    // names.
    use dphone_dpma::appdata::{Op, Outcome, Scope};
    let set = |value: &[u8], expect| Op::Set { app: "game".into(), scope: Scope::Shared, key: "best".into(), value: value.to_vec(), expect };
    let stored = r.dpma.app_data(&set(b"53", Some(0))).await.unwrap();
    let user = stored.user.unwrap();
    assert_eq!((user.id.as_str(), user.name.as_str()), ("100", "Desk"));
    let Outcome::Stored(entry) = stored.outcome else { panic!("{:?}", stored.outcome) };
    assert_eq!((entry.value.as_slice(), entry.author_name.as_str()), (&b"53"[..], "Desk"));
    let again = r.dpma.app_data(&set(b"54", Some(0))).await.unwrap();
    assert_eq!(again.outcome, Outcome::Conflict(Some(entry.clone())));
    let got = r.dpma.app_data(&Op::Get { app: "game".into(), scope: Scope::Shared, key: "best".into() }).await.unwrap();
    assert_eq!(got.outcome, Outcome::Entry(Some(entry)));
    // A value larger than one MESSAGE carries goes in pieces, both ways.
    let big: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
    let stored = r.dpma.app_data(&Op::Set { app: "game".into(), scope: Scope::User, key: "save".into(), value: big.clone(), expect: None }).await.unwrap();
    assert!(matches!(stored.outcome, Outcome::Stored(e) if e.value == big));
    let hello = r.dpma.app_data(&Op::Hello { watch: vec!["game".into()] }).await.unwrap();
    assert_eq!(hello.outcome, Outcome::Hello);
    assert!(r.server.lock().unwrap().app_data.0.contains_key("watch/100"));

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
    // First boot, MAC-keyed assignment: the user picks the server, and
    // nothing else is asked.
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
    r.store.send(Command::StartProvisioning { server: Some("sip:proxy@10.0.0.5:5060".into()) }).await.unwrap();
    until(&r.store, "provisioned by MAC", ProvisionPhase::Provisioned).await;
    assert_eq!(r.store.get::<Provisioning>().server.as_ref().unwrap().source, DiscoverySource::Mdns, "a pick from the list keeps its source");
    // The server's digest is stored verbatim.
    assert_eq!(sink.md5.lock().unwrap().as_deref(), Some("0123456789abcdef0123456789abcdef"));
    assert_eq!(server.lock().unwrap().count("ConfigRequest"), 1);
    let identity = storage.read_file(files::IDENTITY_CERT);
    drop(r);

    // Next boot: same storage.  The saved server is used without
    // discovery, and the token verifies.
    let r = rig_with(server.clone(), storage.clone(), sink.clone());
    r.dpma.network(true, None, Vec::new());
    until(&r.store, "provisioned from the cache", ProvisionPhase::Provisioned).await;
    assert_eq!(r.store.get::<Provisioning>().server.as_ref().unwrap().source, DiscoverySource::Saved);
    assert_eq!(server.lock().unwrap().count("VerifyRequest"), 1);
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
    r.store.send(Command::StartProvisioning { server: Some("sip:proxy@pbx.test".into()) }).await.unwrap();
    until(&r.store, "provisioned", ProvisionPhase::Provisioned).await;
    wait("the firmware check", || (checks() == 1).then_some(())).await;
    assert!(String::from_utf8_lossy(&r.sink.applied.lock().unwrap()[0]).contains("<firmware model=\"D60\""));
    assert_eq!(r.store.get::<Provisioning>().offered_firmware.as_deref(), Some("2_9_28"));

    // With the development setting on, the entry is dropped; the offer is
    // still reported.
    r.dpma.set_ignore_provisioned_firmware(true);
    r.store.send(Command::Reprovision).await.unwrap();
    wait("second fetch", || (r.sink.applied.lock().unwrap().len() == 2).then_some(())).await;
    until(&r.store, "provisioned again", ProvisionPhase::Provisioned).await;
    assert!(!String::from_utf8_lossy(&r.sink.applied.lock().unwrap()[1]).contains("firmware"));
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
}

/// The Handshake carries the certificate chain; a server that leaves that
/// unanswered gets the phone's certificate alone, and is remembered.
#[tokio::test(flavor = "current_thread")]
async fn handshake_sends_the_chain_and_falls_back_to_the_single_certificate() {
    let server = || {
        let mut s = FakeServer::new();
        s.config = CONFIG.as_bytes().to_vec();
        s.token_reqs = "mac".into();
        s.userlist_reqs = "disabled".into();
        s
    };

    // A server that reads the chain form.
    let r = rig_with(Arc::new(Mutex::new(server())), Arc::new(MemStorage::default()), Arc::new(MemSink::default()));
    r.dpma.network(true, Some("sip:proxy@pbx.test".into()), Vec::new());
    r.store.send(Command::StartProvisioning { server: Some("sip:proxy@pbx.test".into()) }).await.unwrap();
    until(&r.store, "provisioned with the chain form", ProvisionPhase::Provisioned).await;
    assert_eq!(r.server.lock().unwrap().handshake_certificates, vec![3]);
    drop(r);

    // A server that does not.
    let mut s = server();
    s.refuse_chain = true;
    let r = rig_with(Arc::new(Mutex::new(s)), Arc::new(MemStorage::default()), Arc::new(MemSink::default()));
    r.dpma.network(true, Some("sip:proxy@pbx.test".into()), Vec::new());
    r.store.send(Command::StartProvisioning { server: Some("sip:proxy@pbx.test".into()) }).await.unwrap();
    until(&r.store, "provisioned with the single certificate", ProvisionPhase::Provisioned).await;
    assert_eq!(r.server.lock().unwrap().handshake_certificates, vec![3, 1]);
    assert_eq!(r.sink.applied.lock().unwrap().len(), 1);
    // The next handshake with that server goes straight to the form it
    // answered.
    r.store.send(Command::Reprovision).await.unwrap();
    wait("second fetch", || (r.sink.applied.lock().unwrap().len() == 2).then_some(())).await;
    assert_eq!(r.server.lock().unwrap().handshake_certificates, vec![3, 1, 1]);
}

/// What the server required at the handshake still holds on a pass that
/// reuses the session: after `digium.config.update` with `delete_token`,
/// the TokenRequest carries the password again.
#[tokio::test(flavor = "current_thread")]
async fn requirements_are_kept_for_a_token_request_without_a_handshake() {
    let r = rig();
    r.dpma.network(true, Some("sip:proxy@pbx.test".into()), Vec::new());
    r.store.send(Command::StartProvisioning { server: Some("sip:proxy@pbx.test".into()) }).await.unwrap();
    until(&r.store, "awaiting a user", ProvisionPhase::AwaitingUser).await;
    r.store.send(Command::SelectUser { id: "100".into(), pin: "1234".into() }).await.unwrap();
    until(&r.store, "provisioned", ProvisionPhase::Provisioned).await;
    assert_eq!(r.server.lock().unwrap().token_requests, vec![("100".to_string(), "1234".to_string(), String::new())]);

    let handshakes = r.server.lock().unwrap().count("Handshake");
    r.server.lock().unwrap().config = CONFIG.replace("789", "555").into_bytes();
    let push = r.server.lock().unwrap().event(
        "digium.config.update",
        b"<config>\t<section id=\"all\"/>\t<option id=\"delete_token\" value=\"true\"/></config>",
    );
    for m in push {
        r.to_phone.send(m).unwrap();
    }
    wait("download with a new token", || (r.sink.applied.lock().unwrap().len() == 2).then_some(())).await;
    until(&r.store, "provisioned again", ProvisionPhase::Provisioned).await;
    let s = r.server.lock().unwrap();
    assert_eq!(s.count("Handshake"), handshakes, "the session was reused");
    assert_eq!(s.token_requests.len(), 2);
    assert_eq!(s.token_requests[1], ("100".to_string(), "1234".to_string(), String::new()), "user and password as the server required");
    drop(s);
    let c = Configurator::parse(&r.sink.configurator.lock().unwrap().clone().unwrap());
    assert!(c.get("config_server_token").is_some(), "the new token is saved");
}

/// A configuration that turns "ignore provisioned firmware" on and offers
/// firmware at once: the setting applies to that very configuration.
#[tokio::test(flavor = "current_thread")]
async fn ignore_setting_in_the_fetched_configuration_covers_its_own_firmware() {
    let with_setting = CONFIG.replace("<firmwares>", "<setting id=\"dphone_ignore_provisioned_firmware\" value=\"1\" />\n<firmwares>");
    let mut s = FakeServer::new();
    s.config = with_setting.into_bytes();
    s.token_reqs = "mac".into();
    let server = Arc::new(Mutex::new(s));
    let sink = Arc::new(MemSink { reads_ignore_setting: true, ..Default::default() });
    let r = rig_with(server.clone(), Arc::new(MemStorage::default()), sink);
    let installer = Arc::new(CountingInstaller(Default::default()));
    r.dpma.set_firmware_updater(Arc::new(NoSource), installer.clone());
    let checks = || installer.0.load(std::sync::atomic::Ordering::SeqCst);

    r.dpma.network(true, Some("sip:proxy@pbx.test".into()), Vec::new());
    r.store.send(Command::StartProvisioning { server: Some("sip:proxy@pbx.test".into()) }).await.unwrap();
    until(&r.store, "provisioned", ProvisionPhase::Provisioned).await;
    let applied = String::from_utf8(r.sink.applied.lock().unwrap()[0].clone()).unwrap();
    assert!(applied.contains("dphone_ignore_provisioned_firmware"));
    assert!(!applied.contains("<firmware"), "the entries are dropped from the saved configuration: {applied}");
    assert_eq!(r.store.get::<Provisioning>().offered_firmware.as_deref(), Some("2_9_28"), "the offer is still reported");

    // The server takes the setting back: the same pass acts on the
    // firmware again.
    server.lock().unwrap().config = CONFIG.as_bytes().to_vec();
    let push = server.lock().unwrap().event("digium.config.update", b"<config></config>");
    for m in push {
        r.to_phone.send(m).unwrap();
    }
    wait("the firmware check", || (checks() == 1).then_some(())).await;
    assert!(String::from_utf8_lossy(&r.sink.applied.lock().unwrap()[1]).contains("<firmware model=\"D60\""));
}

/// Serves HTTP URLs from a table.
struct MapSource(std::collections::HashMap<String, Vec<u8>>, Mutex<Vec<String>>);
impl dphone_dpma::firmware::FirmwareSource for MapSource {
    fn open(&self, url: &str, _: &str) -> Result<Box<dyn std::io::Read + Send>, dphone_dpma::firmware::FetchError> {
        self.1.lock().unwrap().push(url.to_string());
        match self.0.get(url) {
            Some(data) => Ok(Box::new(std::io::Cursor::new(data.clone()))),
            None => Err(dphone_dpma::firmware::FetchError { status: Some(404), message: "Not Found".into() }),
        }
    }
}

fn md5_hex(data: &[u8]) -> String {
    use md5::{Digest, Md5};
    Md5::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// After a configuration is in effect, the files it names are fetched and
/// kept where stock keeps them; a matching digest skips the download, on
/// the later-boot path too (VerifyResponse with the configuration's
/// digest: no ConfigRequest, and the files are still looked after).
#[tokio::test(flavor = "current_thread")]
async fn provisioned_files_are_fetched_checked_and_pruned() {
    use common::MemFiles;
    use dphone_dpma::resources::{Kind, PushedApp, Resource, Wanted, RINGTONE_BUDGET};

    const CONTACTS: &[u8] = b"<contacts group_name=\"Team\"><contact id=\"c1\" first_name=\"Ann\"/></contacts>";
    const BLF: &[u8] = b"<config><smart_blf><blf_items><item location=\"side\" index=\"0\" contact_id=\"c1\"/></blf_items></smart_blf></config>";
    let chimes = vec![7u8; 4000];
    let web_list = b"<contacts group_name=\"Web\"/>".to_vec();

    let mut s = FakeServer::new();
    s.config = CONFIG.as_bytes().to_vec();
    s.token_reqs = "mac".into();
    s.userlist_reqs = "disabled".into();
    s.md5 = Some("0123456789abcdef0123456789abcdef".into());
    s.files.insert("contacts.xml".into(), CONTACTS.to_vec());
    s.files.insert("blf.xml".into(), BLF.to_vec());
    s.files.insert("chimes.sln".into(), chimes.clone());
    let server = Arc::new(Mutex::new(s));

    let files = Arc::new(MemFiles::default());
    // Left by earlier passes.
    files.put("contact_lists/gone", b"old");
    files.put("md5s/gone", b"00000000000000000000000000000000");
    files.put("ringtones/Old.sln16", b"old tone");
    files.put("md5s/Old", b"33333333333333333333333333333333");
    files.put("datasets.xml", b"old datasets");
    files.put("md5s/datasets", b"44444444444444444444444444444444");

    let res = |kind, id: &str, url: &str, md5: Option<&str>| Resource { kind, id: id.into(), url: url.into(), md5: md5.map(str::to_string) };
    let wanted = Wanted {
        resources: vec![
            res(Kind::ContactList, "0", "file://contacts.xml", None),
            res(Kind::ContactList, "web.xml", "http://pbx.test/lists/web.xml", None),
            // The server has no such file.
            res(Kind::ContactList, "3", "file://missing.xml", None),
            // Not usable, as on stock: no id, or no URL.
            res(Kind::ContactList, "", "file://contacts.xml", None),
            res(Kind::ContactList, "9", "", None),
            res(Kind::Ringtone, "Chimes", "file://chimes.sln", Some(&md5_hex(&chimes).to_uppercase())),
            res(Kind::Ringtone, "Huge", "http://pbx.test/huge.sln", None),
            res(Kind::BlfItems, "", "file://blf.xml", None),
        ],
        apps: vec![PushedApp { id: "queues".into(), url: "file://queues.zip".into() }],
    };
    let sink = Arc::new(MemSink { wanted: Mutex::new(wanted), files: Mutex::new(Some(files.clone())), ..Default::default() });
    let storage = Arc::new(MemStorage::default());
    let r = rig_with(server.clone(), storage.clone(), sink.clone());
    let http = Arc::new(MapSource(
        [
            ("http://pbx.test/lists/web.xml".to_string(), web_list.clone()),
            ("http://pbx.test/huge.sln".to_string(), vec![0u8; RINGTONE_BUDGET + 1]),
        ]
        .into(),
        Mutex::new(Vec::new()),
    ));
    r.dpma.set_http_source(http.clone());
    let changes = || sink.resource_changes.load(std::sync::atomic::Ordering::SeqCst);

    r.dpma.network(true, Some("sip:proxy@pbx.test".into()), Vec::new());
    r.store.send(Command::StartProvisioning { server: Some("sip:proxy@pbx.test".into()) }).await.unwrap();
    until(&r.store, "provisioned", ProvisionPhase::Provisioned).await;
    wait("the files", || (changes() == 1).then_some(())).await;

    // Where stock keeps them, with the digest of what was downloaded.
    assert_eq!(files.get("contact_lists/0").as_deref(), Some(CONTACTS));
    assert_eq!(files.get("md5s/0"), Some(md5_hex(CONTACTS).into_bytes()));
    assert_eq!(files.get("contact_lists/web%2Exml"), Some(web_list.clone()));
    assert_eq!(files.get("md5s/web%2Exml"), Some(md5_hex(&web_list).into_bytes()));
    assert_eq!(files.get("ringtones/Chimes.sln16"), Some(chimes.clone()));
    assert_eq!(files.get("md5s/Chimes"), Some(md5_hex(&chimes).into_bytes()));
    assert_eq!(files.get("blf_items.xml").as_deref(), Some(BLF));
    assert_eq!(files.get("md5s/blf_items"), Some(md5_hex(BLF).into_bytes()));
    // No longer named, and downloaded by an earlier pass: removed with
    // their digests.
    for gone in ["contact_lists/gone", "md5s/gone", "ringtones/Old.sln16", "md5s/Old", "datasets.xml", "md5s/datasets"] {
        assert_eq!(files.get(gone), None, "{gone}");
    }
    // What was asked for, in order.
    let asked = server.lock().unwrap().file_requests.clone();
    assert_eq!(asked, vec!["contacts.xml", "missing.xml", "chimes.sln", "blf.xml"]);
    assert_eq!(*http.1.lock().unwrap(), vec!["http://pbx.test/lists/web.xml", "http://pbx.test/huge.sln"]);
    drop(r);

    // The next boot: the token verifies, and the VerifyResponse carries the
    // digest of the configuration on the phone.  Its files are looked
    // after: the entries without a digest are asked for again.
    let before = changes();
    let r = rig_with(server.clone(), storage.clone(), sink.clone());
    r.dpma.set_http_source(http.clone());
    r.dpma.network(true, None, Vec::new());
    until(&r.store, "provisioned from the cache", ProvisionPhase::Provisioned).await;
    wait("the second pass over the files", || (server.lock().unwrap().file_requests.len() == 7).then_some(())).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    {
        let s = server.lock().unwrap();
        assert_eq!(s.count("VerifyRequest"), 1);
        assert_eq!(s.file_requests[4..], ["contacts.xml", "missing.xml", "blf.xml"], "the ringtone's digest matched");
    }

    // The server's contact list changes: the next pass replaces the file
    // and says so.
    server.lock().unwrap().files.insert("contacts.xml".into(), b"<contacts group_name=\"Team\"/>".to_vec());
    r.store.send(Command::Reprovision).await.unwrap();
    wait("the new contact list", || (files.get("contact_lists/0").as_deref() == Some(&b"<contacts group_name=\"Team\"/>"[..])).then_some(())).await;
    wait("the change is reported", || (changes() == before + 1).then_some(())).await;
    assert_eq!(files.get("md5s/0"), Some(md5_hex(b"<contacts group_name=\"Team\"/>").into_bytes()));
}

/// A phone with no saved server lists what discovery finds, whether that
/// is one server or several (the state after a factory reset, which
/// removes `configurator.xml`), and provisions from the one its user picks.
#[tokio::test(flavor = "current_thread")]
async fn discovery_inputs_are_followed() {
    let r = rig();
    let mdns = |addr: &str| vec![DiscoveredServer { source: DiscoverySource::Mdns, name: "Digium Phone Config Server".into(), address: addr.into() }];
    let listed = |r: &Rig| r.store.get::<Provisioning>().discovered.iter().map(|d| (d.source, d.address.clone())).collect::<Vec<_>>();
    r.dpma.network(true, None, Vec::new());
    until(&r.store, "awaiting a server", ProvisionPhase::AwaitingServer).await;

    // One server appears over mDNS while the phone waits: it is listed.
    r.dpma.network(true, None, mdns("10.0.0.5:5060"));
    wait("the announced server is listed", || (listed(&r) == [(DiscoverySource::Mdns, "10.0.0.5:5060".to_string())]).then_some(())).await;

    // DHCP option 66 arrives while the address stays: listed first.
    r.dpma.network(true, Some("sip:proxy@pbx.test:5060".into()), mdns("10.0.0.5:5060"));
    wait("option 66 is listed", || (listed(&r).first().map(|d| d.0) == Some(DiscoverySource::DhcpOption66)).then_some(())).await;

    // The user picks the announced server.
    r.store.send(Command::StartProvisioning { server: Some("10.0.0.5:5060".into()) }).await.unwrap();
    until(&r.store, "awaiting a user", ProvisionPhase::AwaitingUser).await;
    assert_eq!(r.store.get::<Provisioning>().server.as_ref().unwrap().source, DiscoverySource::Mdns);
    assert_eq!(r.server.lock().unwrap().count("Handshake"), 1);

    r.store.send(Command::SelectUser { id: "100".into(), pin: "1234".into() }).await.unwrap();
    until(&r.store, "provisioned", ProvisionPhase::Provisioned).await;

    // Attached: new discovery results are listed.
    r.dpma.network(true, Some("sip:proxy@pbx.test:5060".into()), mdns("10.0.0.6:5060"));
    wait("the list follows", || {
        r.store.get::<Provisioning>().discovered.iter().any(|d| d.address == "10.0.0.6:5060").then_some(())
    })
    .await;
}

/// A server that sends the MD5 of the configuration document on both the
/// ConfigResponse and the VerifyResponse (Open DPMA; dpma-rs from 0.3.1
/// on): its digest is the cache, and a changed document is downloaded.
#[tokio::test(flavor = "current_thread")]
async fn document_digest_is_the_cache_and_a_changed_document_is_downloaded() {
    let mut s = FakeServer::new();
    s.config = CONFIG.as_bytes().to_vec();
    s.token_reqs = "mac".into();
    s.userlist_reqs = "disabled".into();
    s.md5_of_document = true;
    let server = Arc::new(Mutex::new(s));
    let (storage, sink) = (Arc::new(MemStorage::default()), Arc::new(MemSink::default()));
    let boot = || rig_with(server.clone(), storage.clone(), sink.clone());

    let r = boot();
    r.dpma.network(true, Some("sip:proxy@pbx.test".into()), Vec::new());
    r.store.send(Command::StartProvisioning { server: Some("sip:proxy@pbx.test".into()) }).await.unwrap();
    until(&r.store, "provisioned", ProvisionPhase::Provisioned).await;
    assert_eq!(sink.md5.lock().unwrap().as_deref(), Some(md5_hex(CONFIG.as_bytes()).as_str()), "the server's digest is the cache");
    drop(r);

    let r = boot();
    r.dpma.network(true, None, Vec::new());
    until(&r.store, "provisioned from the cache", ProvisionPhase::Provisioned).await;
    assert_eq!(server.lock().unwrap().count("VerifyRequest"), 1);
    drop(r);

    let changed = CONFIG.replace("789", "555");
    server.lock().unwrap().config = changed.clone().into_bytes();
    let r = boot();
    r.dpma.network(true, None, Vec::new());
    until(&r.store, "provisioned after a change", ProvisionPhase::Provisioned).await;
    assert_eq!(server.lock().unwrap().count("ConfigRequest"), 2);
    assert_eq!(sink.applied.lock().unwrap().len(), 2);
    assert_eq!(sink.md5.lock().unwrap().as_deref(), Some(md5_hex(changed.as_bytes()).as_str()));
}

fn selection(store: &Store) -> Option<Result<(), String>> {
    store.get::<Provisioning>().selection.clone()
}

/// A login from the phone's menus or an app while the phone has a user
/// (hot desking): the PBX is asked about the new user, a refusal says why
/// and leaves the phone with the user it had, and a logout leaves the
/// phone without one.
#[tokio::test(flavor = "current_thread")]
async fn a_refused_login_brings_the_previous_assignment_back() {
    let r = rig();
    r.server.lock().unwrap().users.push(("200".into(), "Other".into(), "4321".into()));
    r.dpma.network(true, Some("sip:proxy@pbx.test".into()), Vec::new());
    r.store.send(Command::StartProvisioning { server: Some("sip:proxy@pbx.test".into()) }).await.unwrap();
    until(&r.store, "awaiting a user", ProvisionPhase::AwaitingUser).await;
    r.store.send(Command::SelectUser { id: "100".into(), pin: "1234".into() }).await.unwrap();
    wait("the first login ended", || selection(&r.store)).await.unwrap();
    assert_eq!(phase(&r.store), ProvisionPhase::Provisioned);

    // A wrong PIN for another user: the PBX is asked, and refuses.
    r.store.send(Command::SelectUser { id: "200".into(), pin: "0000".into() }).await.unwrap();
    let refused = wait("the login was refused", || selection(&r.store)).await.unwrap_err();
    assert!(refused.contains("-199"), "{refused}");
    assert_eq!(r.server.lock().unwrap().token_requests.last().unwrap().0, "200");
    // The phone is the first user's again, attached, with nothing saved
    // for the user it was refused as.
    until(&r.store, "provisioned again", ProvisionPhase::Provisioned).await;
    let p = r.store.get::<Provisioning>();
    assert!(p.attached);
    assert_eq!(p.assigned_user, "100");
    let c = Configurator::parse(&r.sink.configurator.lock().unwrap().clone().unwrap());
    assert_eq!(c.get("config_server_user"), Some("100"));
    assert!(c.get("config_server_token").unwrap().starts_with("100/"));
    // The refusal is still what the state says of the login.
    assert!(selection(&r.store).unwrap().is_err());

    // The right PIN: the phone becomes the other user's.
    r.store.send(Command::SelectUser { id: "200".into(), pin: "4321".into() }).await.unwrap();
    wait("the login ended", || selection(&r.store)).await.unwrap();
    let p = r.store.get::<Provisioning>();
    assert_eq!((p.phase.clone(), p.assigned_user.as_str(), p.attached), (ProvisionPhase::Provisioned, "200", true));
    let c = Configurator::parse(&r.sink.configurator.lock().unwrap().clone().unwrap());
    assert_eq!(c.get("config_server_user"), Some("200"));
    assert!(c.get("config_server_token").unwrap().starts_with("200/"));

    // A logout: this PBX has no configuration for a phone without a user,
    // so the phone waits for one, and keeps nothing of the user it had.
    r.store.send(Command::SelectUser { id: String::new(), pin: String::new() }).await.unwrap();
    wait("the logout ended", || selection(&r.store)).await.unwrap();
    let p = r.store.get::<Provisioning>();
    assert_eq!((p.phase.clone(), p.assigned_user.as_str()), (ProvisionPhase::AwaitingUser, ""));
    let c = Configurator::parse(&r.sink.configurator.lock().unwrap().clone().unwrap());
    assert_eq!((c.get("config_server_user"), c.get("config_server_token")), (None, None));
}
