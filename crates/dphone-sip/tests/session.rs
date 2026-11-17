//! Session timers (RFC 4028), check-sync, and the CANCEL reason, against a
//! hand-written UDP peer.

mod common;

use common::{base_config, phone_with, wait, Msg, Server};
use dphone_sip::AccountConfig;
use dphone_state::{AccountId, CallState, Calls, Command, Domain, EndCause, Reply};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

const SDP: &str = "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 30000 RTP/AVP 0 101\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:101 telephone-event/8000\r\na=sendrecv\r\n";

/// A phone whose only account points at `peer`, with short session times.
async fn phone_for(peer: SocketAddr) -> common::Phone {
    let mut cfg = base_config();
    cfg.session_expires_secs = 4;
    cfg.min_se_secs = 2;
    let mut a = AccountConfig::new(AccountId(0), "100", "pw", "127.0.0.1");
    a.primary.port = peer.port();
    a.register = false;
    a.mwi_subscribe = false;
    cfg.accounts.push(a);
    phone_with("100", cfg, 4000).await
}

fn invite(peer: SocketAddr, phone: u16, call_id: &str, extra: &str) -> String {
    format!(
        "INVITE sip:100@127.0.0.1:{phone} SIP/2.0\r\nVia: SIP/2.0/UDP {peer};branch=z9hG4bK{call_id}\r\nMax-Forwards: 70\r\n\
         From: \"Caller\" <sip:200@{peer}>;tag=s1\r\nTo: <sip:100@127.0.0.1:{phone}>\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\n\
         Contact: <sip:200@{peer}>\r\nAllow: INVITE, ACK, BYE, CANCEL\r\n{extra}Content-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{SDP}",
        SDP.len()
    )
}

fn response(server: &Server, call_id: &str, code: &str) -> Option<Msg> {
    server.log.lock().unwrap().iter().find(|m| m.start.starts_with(&format!("SIP/2.0 {code}")) && m.header("Call-ID") == Some(call_id)).cloned()
}

#[tokio::test(flavor = "current_thread")]
async fn incoming_session_timer_is_answered_and_enforced() {
    let server = Server::start(Box::new(|m: &Msg| if m.method() == "ACK" { vec![] } else { vec![m.reply(200, "OK", &[], "")] })).await;
    let p = phone_for(server.addr).await;
    let phone = p.handle.local_port();
    let to = format!("127.0.0.1:{phone}");

    // An interval below our minimum is refused with the minimum.
    server.socket.send_to(invite(server.addr, phone, "small", "Supported: timer\r\nSession-Expires: 1\r\n").as_bytes(), &to).await.unwrap();
    let refused = wait("422", || response(&server, "small", "422")).await;
    assert_eq!(refused.header("Min-SE"), Some("2"));
    assert!(p.store.get::<Calls>().calls.is_empty());

    // They ask for a timer and say they will refresh.
    let extra = "Supported: timer\r\nSession-Expires: 4;refresher=uac\r\nMin-SE: 2\r\n";
    server.socket.send_to(invite(server.addr, phone, "timed", extra).as_bytes(), &to).await.unwrap();
    let call = wait("ringing", || p.store.get::<Calls>().ringing().next().map(|c| c.id)).await;
    p.store.send(Command::Answer { call, path: None }).await.unwrap();
    let ok = wait("200", || response(&server, "timed", "200")).await;
    assert_eq!(ok.header("Session-Expires"), Some("4;refresher=uac"));
    assert_eq!(ok.header("Require"), Some("timer"));
    assert!(ok.header("Supported").unwrap().contains("timer"));
    let ack = format!(
        "ACK sip:100@127.0.0.1:{phone} SIP/2.0\r\nVia: SIP/2.0/UDP {};branch=z9hG4bKack\r\nMax-Forwards: 70\r\n\
         From: \"Caller\" <sip:200@{}>;tag=s1\r\nTo: {}\r\nCall-ID: timed\r\nCSeq: 1 ACK\r\nContent-Length: 0\r\n\r\n",
        server.addr,
        server.addr,
        ok.header("To").unwrap()
    );
    server.socket.send_to(ack.as_bytes(), &to).await.unwrap();
    assert_eq!(p.store.get::<Calls>().get(call).map(|c| c.state.clone()), Some(CallState::Active));

    // They never refresh: the phone ends the session (4 s less a third).
    let bye = wait("BYE after the session ran out", || server.requests("BYE").pop()).await;
    assert!(bye.header("Reason").is_some_and(|r| r.contains("Session expired")), "{:?}", bye.header("Reason"));
    wait("call ended", || {
        matches!(p.store.get::<Calls>().get(call).map(|c| c.state.clone()), Some(CallState::Ended(EndCause::Error(_))) | None)
            .then_some(())
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn outgoing_session_timer_follows_422_and_refreshes() {
    let invites = Arc::new(Mutex::new(0u32));
    let n = invites.clone();
    let me = Arc::new(Mutex::new(None::<SocketAddr>));
    let me2 = me.clone();
    let server = Server::start(Box::new(move |m: &Msg| match m.method() {
        "INVITE" => {
            let mut n = n.lock().unwrap();
            *n += 1;
            if *n == 1 {
                vec![m.reply(422, "Session Interval Too Small", &[("Min-SE", "3600".into())], "")]
            } else {
                vec![m.reply(
                    200,
                    "OK",
                    &[
                        ("Contact", format!("<sip:200@{}>", me2.lock().unwrap().expect("address set"))),
                        ("Allow", "INVITE, ACK, BYE, CANCEL, UPDATE".into()),
                        ("Require", "timer".into()),
                        ("Session-Expires", "3;refresher=uac".into()),
                        ("Content-Type", "application/sdp".into()),
                    ],
                    SDP,
                )]
            }
        }
        "UPDATE" => vec![m.reply(200, "OK", &[("Session-Expires", "3;refresher=uac".into())], "")],
        "ACK" => vec![],
        _ => vec![m.reply(200, "OK", &[], "")],
    }))
    .await;
    *me.lock().unwrap() = Some(server.addr);
    let p = phone_for(server.addr).await;
    let call = match p.store.send(Command::Dial { target: "200".into(), account: None, path: None }).await {
        Ok(Reply::Call(id)) => id,
        other => panic!("{other:?}"),
    };
    wait("active", || (p.store.get::<Calls>().get(call).map(|c| c.state.clone()) == Some(CallState::Active)).then_some(())).await;
    let sent = server.requests("INVITE");
    assert_eq!(sent[0].header("Session-Expires"), Some("4"));
    assert_eq!(sent[0].header("Min-SE"), Some("2"));
    assert!(sent[0].header("Supported").unwrap().contains("timer"));
    assert_eq!(sent[1].header("Session-Expires"), Some("3600"), "the retry asks for their minimum");

    // We are the refresher of a 3 s session: UPDATEs keep coming, with our
    // role stated, and the call stays up.
    wait("two refreshes", || (server.requests("UPDATE").len() >= 2).then_some(())).await;
    let update = server.requests("UPDATE").remove(0);
    assert_eq!(update.header("Session-Expires"), Some("3;refresher=uac"));
    assert!(update.body.is_empty());
    assert_eq!(p.store.get::<Calls>().get(call).map(|c| c.state.clone()), Some(CallState::Active));
    assert!(server.requests("BYE").is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn check_sync_and_cancel_reason() {
    let server = Server::start(Box::new(|m: &Msg| if m.method() == "ACK" { vec![] } else { vec![m.reply(200, "OK", &[], "")] })).await;
    let p = phone_for(server.addr).await;
    let phone = p.handle.local_port();
    let to = format!("127.0.0.1:{phone}");

    // check-sync: answered, and the configuration is asked for again.
    let asked = Arc::new(Mutex::new(0u32));
    let rx = p.store.handle(Domain::Provisioning);
    let seen = asked.clone();
    std::thread::spawn(move || {
        while let Some(env) = rx.recv() {
            if env.command == Command::Reprovision {
                *seen.lock().unwrap() += 1;
            }
            env.done();
        }
    });
    let notify = format!(
        "NOTIFY sip:100@{to} SIP/2.0\r\nVia: SIP/2.0/UDP {};branch=z9hG4bKcs\r\nMax-Forwards: 70\r\nFrom: <sip:100@{}>;tag=cs\r\n\
         To: <sip:100@{to}>\r\nCall-ID: cs\r\nCSeq: 1 NOTIFY\r\nEvent: check-sync\r\nContent-Length: 0\r\n\r\n",
        server.addr, server.addr
    );
    server.socket.send_to(notify.as_bytes(), &to).await.unwrap();
    wait("200 to check-sync", || response(&server, "cs", "200")).await;
    wait("reprovision", || (*asked.lock().unwrap() == 1).then_some(())).await;

    // A CANCEL that says another phone took the call is not a missed call
    // in the usual sense: the cause records it.
    server.socket.send_to(invite(server.addr, phone, "elsewhere", "").as_bytes(), &to).await.unwrap();
    let call = wait("ringing", || p.store.get::<Calls>().ringing().next().map(|c| c.id)).await;
    let cancel = format!(
        "CANCEL sip:100@{to} SIP/2.0\r\nVia: SIP/2.0/UDP {};branch=z9hG4bKelsewhere\r\nMax-Forwards: 70\r\n\
         From: \"Caller\" <sip:200@{}>;tag=s1\r\nTo: <sip:100@{to}>\r\nCall-ID: elsewhere\r\nCSeq: 1 CANCEL\r\n\
         Reason: SIP;cause=200;text=\"Call completed elsewhere\"\r\nContent-Length: 0\r\n\r\n",
        server.addr, server.addr
    );
    server.socket.send_to(cancel.as_bytes(), &to).await.unwrap();
    wait("canceled", || {
        (p.store.get::<Calls>().get(call).map(|c| c.state.clone())
            == Some(CallState::Ended(EndCause::Canceled { answered_elsewhere: true })))
        .then_some(())
    })
    .await;
    assert!(response(&server, "elsewhere", "487").is_some());
}
