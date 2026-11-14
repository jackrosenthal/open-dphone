//! Two and three phones calling each other directly over loopback UDP:
//! call set-up, hold and resume, DTMF, blind and attended transfer,
//! cancel, reject, and do-not-disturb.

mod common;

use common::{phone, wait};
use dphone_sip::media::MediaCall;
use dphone_state::{
    CallId, CallLog, CallLogKind, CallState, Calls, Command, EndCause, Reply, Store,
};

fn state(store: &Store, id: CallId) -> Option<CallState> {
    store.get::<Calls>().get(id).map(|c| c.state.clone())
}

fn only_call(store: &Store) -> Option<CallId> {
    store.get::<Calls>().live().next().map(|c| c.id)
}

async fn dial(p: &common::Phone, target: &str) -> CallId {
    match p.store.send(Command::Dial { target: target.into(), account: None, path: None }).await {
        Ok(Reply::Call(id)) => id,
        other => panic!("dial failed: {other:?}"),
    }
}

async fn connect(a: &common::Phone, b: &common::Phone) -> (CallId, CallId) {
    let ca = dial(a, &b.uri()).await;
    let cb = wait("B to ring", || only_call(&b.store).filter(|id| state(&b.store, *id) == Some(CallState::Ringing))).await;
    wait("A to hear ringback", || {
        matches!(state(&a.store, ca), Some(CallState::Ringback { .. })).then_some(())
    })
    .await;
    b.store.send(Command::Answer { call: cb, path: None }).await.expect("answer");
    wait("A active", || (state(&a.store, ca) == Some(CallState::Active)).then_some(())).await;
    wait("B active", || (state(&b.store, cb) == Some(CallState::Active)).then_some(())).await;
    (ca, cb)
}

#[tokio::test(flavor = "current_thread")]
async fn call_hold_dtmf_hangup() {
    let a = phone("alice", 4000).await;
    let b = phone("bob", 5000).await;
    let (ca, cb) = connect(&a, &b).await;

    // Both ends agreed on a codec and each other's RTP ports.
    let sa = a.media.last_start(ca).expect("A stream");
    let sb = b.media.last_start(cb).expect("B stream");
    assert_eq!(sa.codec, sb.codec);
    assert_eq!(sa.remote.port(), 5000);
    assert_eq!(sb.remote.port(), 4000);
    assert!(sa.send && sa.recv && sb.send && sb.recv);
    assert_eq!(sa.dtmf_tx_pt, Some(101));
    let call_b = b.store.get::<Calls>().get(cb).cloned().unwrap();
    assert_eq!(call_b.remote.number, "alice");
    assert_eq!(call_b.remote.name, "alice display");
    assert!(call_b.connected_mono_ms.is_some());

    // Hold from A: A is held locally, B sees a remote hold.
    a.store.send(Command::Hold { call: ca }).await.unwrap();
    wait("A held", || (state(&a.store, ca) == Some(CallState::Held { local: true, remote: false })).then_some(())).await;
    wait("B held", || (state(&b.store, cb) == Some(CallState::Held { local: false, remote: true })).then_some(())).await;
    let sa = a.media.last_start(ca).unwrap();
    assert!(!sa.send && !sa.recv);
    let sb = b.media.last_start(cb).unwrap();
    assert!(!sb.send, "B must not send to a holding peer");

    a.store.send(Command::Resume { call: ca }).await.unwrap();
    wait("A resumed", || (state(&a.store, ca) == Some(CallState::Active)).then_some(())).await;
    wait("B resumed", || (state(&b.store, cb) == Some(CallState::Active)).then_some(())).await;
    assert!(b.media.last_start(cb).unwrap().send);

    // DTMF goes out as telephone events through the media engine.
    a.store.send(Command::Dtmf { call: ca, digit: '5' }).await.unwrap();
    assert!(a.media.calls().contains(&MediaCall::Dtmf(ca, '5')));
    assert!(a.store.send(Command::Dtmf { call: ca, digit: 'x' }).await.is_err());

    a.store.send(Command::Hangup { call: ca }).await.unwrap();
    wait("B ended", || (state(&b.store, cb) == Some(CallState::Ended(EndCause::RemoteHangup))).then_some(())).await;
    assert_eq!(state(&a.store, ca), Some(CallState::Ended(EndCause::LocalHangup)));
    assert!(a.media.calls().contains(&MediaCall::Close(ca)));
    // Call logs: placed on A, received on B.
    assert_eq!(a.store.get::<CallLog>().entries[0].kind, CallLogKind::Placed);
    assert_eq!(b.store.get::<CallLog>().entries[0].kind, CallLogKind::Received);
    // Ended calls leave the state after the linger time.
    wait("A idle", || a.store.get::<Calls>().calls.is_empty().then_some(())).await;
    a.handle.shutdown().await;
    b.handle.shutdown().await;
}

#[tokio::test(flavor = "current_thread")]
async fn blind_transfer() {
    let a = phone("alice", 4000).await;
    let b = phone("bob", 5000).await;
    let c = phone("carol", 6000).await;
    let (ca, cb) = connect(&a, &b).await;

    // B sends A to C.
    b.store.send(Command::TransferBlind { call: cb, target: c.uri() }).await.unwrap();
    assert_eq!(state(&b.store, cb), Some(CallState::Transferring));
    let cc = wait("C to ring", || only_call(&c.store)).await;
    assert_eq!(c.store.get::<Calls>().get(cc).unwrap().remote.number, "alice");
    c.store.send(Command::Answer { call: cc, path: None }).await.unwrap();
    // B hears the transfer succeeded and drops out; A's first call ends.
    wait("B transferred", || (state(&b.store, cb) == Some(CallState::Ended(EndCause::Transferred))).then_some(())).await;
    wait("A's first call ended", || state(&a.store, ca).is_none_or(|s| s.is_ended()).then_some(())).await;
    let ca2 = wait("A talking to C", || {
        a.store.get::<Calls>().live().find(|x| x.state == CallState::Active && x.remote.number == "carol").map(|x| x.id)
    })
    .await;
    assert_eq!(a.media.last_start(ca2).unwrap().remote.port(), 6000);
    wait("C active", || (state(&c.store, cc) == Some(CallState::Active)).then_some(())).await;
    c.store.send(Command::Hangup { call: cc }).await.unwrap();
    wait("A hung up on", || (state(&a.store, ca2) == Some(CallState::Ended(EndCause::RemoteHangup))).then_some(())).await;
}

#[tokio::test(flavor = "current_thread")]
async fn attended_transfer() {
    let a = phone("alice", 4000).await;
    let b = phone("bob", 5000).await;
    let c = phone("carol", 6000).await;
    let (ca_b, cb) = connect(&a, &b).await;

    // A consults C; the call with B is held automatically.
    let ca_c = dial(&a, &c.uri()).await;
    wait("B put on hold", || state(&b.store, cb).is_some_and(|s| s.is_held()).then_some(())).await;
    let cc = wait("C to ring", || only_call(&c.store)).await;
    c.store.send(Command::Answer { call: cc, path: None }).await.unwrap();
    wait("A consulting", || (state(&a.store, ca_c) == Some(CallState::Active)).then_some(())).await;
    assert!(state(&a.store, ca_b).unwrap().is_held());

    // A connects B to C: B calls C replacing A's consultation call.
    a.store.send(Command::TransferAttended { call: ca_b, consult: ca_c }).await.unwrap();
    wait("A out of both calls", || a.store.get::<Calls>().live().next().is_none().then_some(())).await;
    let cb2 = wait("B talking to C", || {
        b.store.get::<Calls>().live().find(|x| x.state == CallState::Active && x.remote.number == "carol").map(|x| x.id)
    })
    .await;
    let cc2 = wait("C talking to B", || {
        c.store.get::<Calls>().live().find(|x| x.state == CallState::Active && x.remote.number == "bob").map(|x| x.id)
    })
    .await;
    assert_ne!(cc2, cc, "C's call with A was replaced by a new one");
    assert_eq!(b.media.last_start(cb2).unwrap().remote.port(), c.media.last_start(cc2).map(|_| 6002).unwrap());
}

#[tokio::test(flavor = "current_thread")]
async fn cancel_reject_and_dnd() {
    let a = phone("alice", 4000).await;
    let b = phone("bob", 5000).await;

    // Cancel: A gives up while B rings; B logs a missed call.
    let ca = dial(&a, &b.uri()).await;
    let cb = wait("B to ring", || only_call(&b.store)).await;
    wait("A ringback", || matches!(state(&a.store, ca), Some(CallState::Ringback { .. })).then_some(())).await;
    a.store.send(Command::Hangup { call: ca }).await.unwrap();
    wait("B cancelled", || {
        (state(&b.store, cb) == Some(CallState::Ended(EndCause::Cancelled { answered_elsewhere: false }))).then_some(())
    })
    .await;
    let log = b.store.get::<CallLog>();
    assert_eq!(log.entries[0].kind, CallLogKind::Missed);
    assert_eq!(log.missed_unseen, 1);
    wait("B idle", || b.store.get::<Calls>().calls.is_empty().then_some(())).await;
    wait("A idle", || a.store.get::<Calls>().calls.is_empty().then_some(())).await;

    // Reject: B declines; A hears busy.
    let ca = dial(&a, &b.uri()).await;
    let cb = wait("B to ring", || only_call(&b.store)).await;
    b.store.send(Command::Reject { call: cb, busy: true }).await.unwrap();
    wait("A busy", || (state(&a.store, ca) == Some(CallState::Ended(EndCause::Busy))).then_some(())).await;
    assert!(a.media.calls().contains(&MediaCall::Tone(Some(dphone_state::Tone::Busy))));
    wait("A idle", || a.store.get::<Calls>().calls.is_empty().then_some(())).await;

    // Do not disturb: B never rings.
    b.store.send(Command::SetDnd { on: true }).await.unwrap();
    let ca = dial(&a, &b.uri()).await;
    wait("A busy (DND)", || (state(&a.store, ca) == Some(CallState::Ended(EndCause::Busy))).then_some(())).await;
    assert!(b.store.get::<Calls>().live().next().is_none());

    // Forward always: B redirects A to C.
    let c = phone("carol", 6000).await;
    b.store.send(Command::SetDnd { on: false }).await.unwrap();
    b.store
        .send(Command::SetForward { kind: dphone_state::ForwardKind::Always, target: c.uri(), no_answer_secs: None })
        .await
        .unwrap();
    wait("A idle", || a.store.get::<Calls>().calls.is_empty().then_some(())).await;
    let _ca = dial(&a, &b.uri()).await;
    let cc = wait("C to ring instead", || only_call(&c.store)).await;
    assert_eq!(state(&c.store, cc), Some(CallState::Ringing));
}

#[tokio::test(flavor = "current_thread")]
async fn forward_on_no_answer() {
    // B rings, nobody answers, and B redirects the caller to C; A follows
    // the redirect.
    let a = phone("alice", 4000).await;
    let b = phone("bob", 5000).await;
    let c = phone("carol", 6000).await;
    b.store
        .send(Command::SetForward { kind: dphone_state::ForwardKind::NoAnswer, target: c.uri(), no_answer_secs: Some(1) })
        .await
        .unwrap();
    let ca = dial(&a, &b.uri()).await;
    let cb = wait("B to ring", || only_call(&b.store)).await;
    assert_eq!(state(&b.store, cb), Some(CallState::Ringing));
    let cc = wait("C to ring after the forward", || only_call(&c.store)).await;
    wait("B stopped ringing", || (state(&b.store, cb) == Some(CallState::Ended(EndCause::NoAnswer))).then_some(())).await;
    c.store.send(Command::Answer { call: cc, path: None }).await.unwrap();
    wait("A talking to C", || (state(&a.store, ca) == Some(CallState::Active)).then_some(())).await;
    assert_eq!(a.media.last_start(ca).unwrap().remote.port(), 6000);
}
