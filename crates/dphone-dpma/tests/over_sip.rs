//! End to end over real SIP on loopback: the DPMA client on one
//! `dphone-sip` endpoint, the in-process DPMA server behind another.
#![cfg(feature = "sip")]

mod common;

use common::{wait, FakeServer, MemSink, MemStorage};
use dphone_dpma::{DpmaConfig, Message};
use dphone_sip::{NullMedia, OutMessage, SipConfig};
use dphone_state::{Model, ProvisionPhase, Provisioning, Store};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};

async fn endpoint() -> (Store, dphone_sip::SipHandle) {
    let store = Store::new(Model::D60);
    let mut cfg = SipConfig::new(Model::D60, IpAddr::V4(Ipv4Addr::LOCALHOST));
    cfg.local_port = 0;
    let handle = dphone_sip::start(store.clone(), cfg, Arc::new(NullMedia::default()), None).await.unwrap();
    (store, handle)
}

#[tokio::test(flavor = "current_thread")]
async fn provision_over_sip_messages() {
    // The server: every DPMA MESSAGE goes through the fake server, and its
    // replies go back to the Contact of the request.
    let mut fake = FakeServer::new();
    fake.token_reqs = "mac".into();
    fake.userlist_reqs = "disabled".into();
    // Big enough that the ConfigResponse needs several MESSAGEs.
    let mut config = String::from("<config>\n");
    for i in 0..120 {
        config.push_str(&format!("<setting id=\"filler_{i}\" value=\"{}\" />\n", "x".repeat(20)));
    }
    config.push_str("</config>");
    fake.config = config.clone().into_bytes();
    let fake = Arc::new(Mutex::new(fake));
    let (_server_store, server_sip) = endpoint().await;
    let mut inbox = server_sip.messages();
    {
        let (fake, sip) = (fake.clone(), server_sip.clone());
        tokio::spawn(async move {
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
                    sip.send_message(reply).await.expect("reply delivered");
                }
            }
        });
    }

    // The phone.
    let (store, phone_sip) = endpoint().await;
    let mac = "000fd3aabbcc";
    let (transport, incoming) = dphone_dpma::sip::transport(&phone_sip, mac);
    let sink = Arc::new(MemSink::default());
    let dpma = dphone_dpma::start(
        store.clone(),
        DpmaConfig::new(mac, "Digium D60 3_0_0"),
        transport,
        incoming,
        Arc::new(MemStorage::default()),
        sink.clone(),
    );
    // DHCP option 66 names the server, and the user picks it.
    let server_uri = format!("sip:proxy@127.0.0.1:{}", server_sip.local_port());
    dpma.network(true, Some(server_uri.clone()), Vec::new());
    store.send(dphone_state::Command::StartProvisioning { server: Some(server_uri) }).await.unwrap();
    wait("provisioned over SIP", || (store.get::<Provisioning>().phase == ProvisionPhase::Provisioned).then_some(())).await;
    assert_eq!(sink.applied.lock().unwrap()[0], config.into_bytes());
    assert_eq!(fake.lock().unwrap().log, ["Handshake", "TokenRequest", "ConfigRequest"]);
    assert_eq!(dpma.file("ringtone1").await.unwrap(), b"RIFFdata");
}
