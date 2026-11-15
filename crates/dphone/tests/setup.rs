//! Setup on a phone with no network: the DPMA client starts only once the
//! phone has an address, and until then the choices of the setup screens
//! are taken by the main role (`provisioning::early_commands`) and handed
//! to the client when it starts.  The real client runs here against the
//! in-process DPMA server of `dphone-dpma`'s tests.

#[path = "../../dphone-dpma/tests/common/mod.rs"]
mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{wait, FakeServer, Mem, MemSink, MemStorage};
use dphone::mainrole::subsystems::provisioning::{early_commands, follow_setup, EarlyCommands};
use dphone_dpma::{DpmaConfig, DpmaHandle};
use dphone_state::{Command, DiscoveredServer, DiscoverySource, Model, Phase, ProvisionPhase, Provisioning, Store, System};
use tokio::sync::mpsc;

/// A factory-reset phone when start-up has ended: in its setup, with no
/// address, so with no DPMA client.
fn fresh_phone() -> (Store, EarlyCommands) {
    let store = Store::new(Model::D60);
    store.update::<System, _>(|s| s.phase = Phase::Provisioning);
    tokio::spawn(follow_setup(store.clone()));
    let early = early_commands(&store);
    (store, early)
}

/// The phone got its address: the client starts, as in `telephony`.
fn client_starts(store: &Store, early: EarlyCommands) -> (DpmaHandle, Arc<Mutex<FakeServer>>) {
    let mut s = FakeServer::new();
    s.config = b"<config>\n<accounts><account id=\"100\" username=\"100\"/></accounts>\n</config>".to_vec();
    s.token_reqs = "mac".into();
    s.userlist_reqs = "disabled".into();
    let server = Arc::new(Mutex::new(s));
    let (to_phone, incoming) = mpsc::unbounded_channel();
    let transport = Arc::new(Mem { server: server.clone(), to_phone, down: Mutex::new(false) });
    let mut cfg = DpmaConfig::new("00:0F:D3:AA:BB:CC", "Digium D60 3_0_0");
    cfg.timeout = Duration::from_millis(500);
    cfg.ping_secs = 0;
    let dpma = dphone_dpma::start(store.clone(), cfg, transport, incoming, Arc::new(MemStorage::default()), Arc::new(MemSink::default()));
    early.hand_over(store);
    (dpma, server)
}

fn phases(store: &Store) -> (Phase, ProvisionPhase) {
    (store.get::<System>().phase, store.get::<Provisioning>().phase.clone())
}

fn announced() -> Vec<DiscoveredServer> {
    vec![DiscoveredServer { source: DiscoverySource::Mdns, name: "Digium Phone Config Server".into(), address: "sip:proxy@10.0.0.5:5060".into() }]
}

#[tokio::test(flavor = "current_thread")]
async fn use_without_a_server_works_with_no_network() {
    let (store, early) = fresh_phone();
    // The setup screens' softkeys post their commands; with no handler a
    // post is dropped.
    assert!(store.post(Command::SkipProvisioning), "the choice is taken without the client");
    wait("the idle screen", || (phases(&store) == (Phase::Running, ProvisionPhase::Standalone)).then_some(())).await;

    // The network comes up later, with a server announced: the phone stays
    // as its user left it, and nothing is sent to the server.
    let (dpma, server) = client_starts(&store, early);
    dpma.network(true, None, announced());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(phases(&store), (Phase::Running, ProvisionPhase::Standalone));
    assert_eq!(server.lock().unwrap().count("Handshake"), 0);

    // Until the user asks for provisioning again ("Reprovision" in the
    // Reset menu): then the server found is offered.
    store.send(Command::Reprovision).await.unwrap();
    wait("the server menu", || (phases(&store) == (Phase::Provisioning, ProvisionPhase::AwaitingServer)).then_some(())).await;
    assert_eq!(store.get::<Provisioning>().discovered, announced());
}

#[tokio::test(flavor = "current_thread")]
async fn an_address_entered_with_no_network_is_used_when_the_client_starts() {
    let (store, early) = fresh_phone();
    store.send(Command::StartProvisioning { server: Some("10.0.0.9:5060".into()) }).await.unwrap();
    // Shown on the setup screen while the phone waits for its network.
    let p = store.get::<Provisioning>();
    assert_eq!(p.phase, ProvisionPhase::Idle);
    assert_eq!(p.server.as_ref().map(|s| (s.source, s.address.as_str())), Some((DiscoverySource::Manual, "10.0.0.9:5060")));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(store.get::<System>().phase, Phase::Provisioning, "still in setup");
    // What needs the client is refused, not lost.
    assert!(store.send(Command::SelectUser { id: "100".into(), pin: String::new() }).await.is_err());

    // The network comes up, with another server announced: the address
    // the user entered is the one used, without the menu or its countdown.
    let (dpma, server) = client_starts(&store, early);
    dpma.network(true, None, announced());
    wait("provisioned", || (phases(&store) == (Phase::Running, ProvisionPhase::Provisioned)).then_some(())).await;
    let p = store.get::<Provisioning>();
    assert_eq!(p.server.as_ref().map(|s| (s.source, s.address.as_str())), Some((DiscoverySource::Manual, "10.0.0.9:5060")));
    assert_eq!(server.lock().unwrap().count("Handshake"), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn nothing_chosen_means_the_menu_when_the_client_starts() {
    let (store, early) = fresh_phone();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(phases(&store), (Phase::Provisioning, ProvisionPhase::Idle));
    let (dpma, server) = client_starts(&store, early);
    dpma.network(true, None, announced());
    wait("the server menu", || (phases(&store) == (Phase::Provisioning, ProvisionPhase::AwaitingServer)).then_some(())).await;
    assert_eq!(store.get::<Provisioning>().discovered, announced());
    assert_eq!(server.lock().unwrap().count("Handshake"), 0, "the menu picks, not the client");
}
