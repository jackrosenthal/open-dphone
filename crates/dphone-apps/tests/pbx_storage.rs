//! The PBX locations of the app interface `storage` end to end on this
//! machine: the test guest `probe` on two phones of one PBX, whose store
//! of app data is `FakeStore` (the keys kept as the DPMA module keeps
//! them).

mod common;

use common::Rig;
use dphone_apps::testutil::FakeStore;
use dphone_apps::{StoreOp, StoreReply, StoreScope};

/// Two phones of one PBX with the probe on screen: Alice's and Bob's.
fn two_phones() -> (Rig, Rig) {
    let pbx = FakeStore::default();
    let mut alice = Rig::on_pbx(pbx.phone("101", "Alice"));
    let mut bob = Rig::on_pbx(pbx.phone("102", "Bob"));
    alice.launch("probe");
    bob.launch("probe");
    (alice, bob)
}

fn set(store: &FakeStore, scope: StoreScope, key: &str, value: &str, expect: Option<u64>) -> StoreReply {
    store.run(StoreOp::Set {
        app: "probe".into(),
        scope,
        key: key.into(),
        value: value.as_bytes().to_vec(),
        expect,
    })
}

#[test]
fn the_probe_uses_the_interface() {
    assert!(common::capabilities("probe").contains(&"storage"));
    // An app that does not call it does not have it.
    assert!(!common::capabilities("diagnostics").contains(&"storage"));
}

#[test]
fn a_shared_key_written_on_one_phone_is_read_on_another() {
    let (mut alice, mut bob) = two_phones();
    alice.softkey("probe", "Top");
    assert_eq!(alice.ui.texts("probe"), ["top=Alice"]);
    bob.softkey("probe", "Top");
    assert_eq!(bob.ui.texts("probe"), ["top=Alice,Bob"]);
    assert_eq!(
        alice.app_data.value("probe", StoreScope::Shared, "best/top").as_deref(),
        Some(&b"Alice,Bob"[..])
    );
    // The list names who wrote each entry.
    alice.softkey("probe", "Board");
    assert_eq!(alice.ui.texts("probe"), ["board=best/top:Alice,Bob@Bob more=false"]);
}

#[test]
fn an_app_that_meets_a_conflict_writes_again_over_the_newer_entry() {
    // The PBX answers the probe's read; then, before the probe's write
    // arrives, another phone writes.  A store that slips that write in:
    let pbx = FakeStore::default();
    let alice_side = pbx.phone("101", "Alice");
    let bob_side = pbx.phone("102", "Bob");
    struct Racing {
        inner: FakeStore,
        other: FakeStore,
        raced: std::sync::atomic::AtomicBool,
    }
    impl dphone_apps::PbxStore for Racing {
        fn request(&self, op: StoreOp, done: dphone_apps::Completion<StoreReply>) {
            let first_write = matches!(op, StoreOp::Set { .. })
                && !self.raced.swap(true, std::sync::atomic::Ordering::SeqCst);
            if first_write {
                // Bob's phone writes first.
                let r = set(&self.other, StoreScope::Shared, "best/top", "Bob", Some(0));
                assert!(matches!(r, StoreReply::Stored(_)));
            }
            self.inner.request(op, done);
        }
        fn user(&self) -> Option<dphone_apps::StoreUser> {
            self.inner.user()
        }
        fn online(&self) -> bool {
            self.inner.online()
        }
        fn watch(&self, apps: &[String]) {
            self.inner.watch(apps)
        }
        fn subscribe(&self, f: Box<dyn Fn(dphone_apps::StoreSignal) + Send + Sync>) {
            self.inner.subscribe(f)
        }
    }
    let racing = Racing {
        inner: alice_side.clone(),
        other: bob_side,
        raced: false.into(),
    };
    let mut alice = common::Rig::with_services(|mut services| {
        services.store = std::sync::Arc::new(racing);
        services
    });
    alice.launch("probe");
    alice.softkey("probe", "Top");
    // Both names are there: the probe was shown Bob's entry and wrote
    // over that one.
    assert_eq!(alice.ui.texts("probe"), ["top=Bob,Alice"]);
    let writes: Vec<Option<u64>> = alice_side
        .ops()
        .into_iter()
        .filter_map(|op| match op {
            StoreOp::Set { expect, .. } => Some(expect),
            _ => None,
        })
        .collect();
    assert_eq!(writes.len(), 2, "one refused write and one over the newer entry");
    assert_eq!(writes[0], Some(0));
    assert!(writes[1].is_some_and(|v| v > 0));
}

#[test]
fn a_user_has_keys_of_their_own() {
    let (mut alice, mut bob) = two_phones();
    alice.softkey("probe", "Save");
    assert_eq!(alice.ui.texts("probe"), ["saved=level 5 by Alice (101)"]);
    bob.softkey("probe", "Load");
    assert_eq!(bob.ui.texts("probe"), ["loaded=None"]);
    alice.softkey("probe", "Load");
    assert_eq!(alice.ui.texts("probe"), ["loaded=Some(\"level 5\")"]);

    // The phone is reset: nothing of the app is left on it, and the same
    // user is provisioned again.  The progress is still on the PBX.
    let pbx = alice.app_data.clone();
    drop(alice);
    let mut fresh = Rig::on_pbx(pbx.phone("101", "Alice"));
    fresh.launch("probe");
    fresh.softkey("probe", "Load");
    assert_eq!(fresh.ui.texts("probe"), ["loaded=Some(\"level 5\")"]);
}

#[test]
fn an_app_reaches_only_its_own_keys() {
    let (mut alice, _bob) = two_phones();
    // Another app's key of the same name.
    alice.app_data.run(StoreOp::Set {
        app: "other".into(),
        scope: StoreScope::User,
        key: "progress".into(),
        value: b"not the probe's".to_vec(),
        expect: None,
    });
    alice.softkey("probe", "Load");
    assert_eq!(alice.ui.texts("probe"), ["loaded=None"]);
    // Every request of the app carries the app's name, from the phone.
    let apps: Vec<String> = alice
        .app_data
        .ops()
        .into_iter()
        .map(|op| match op {
            StoreOp::Get { app, .. }
            | StoreOp::List { app, .. }
            | StoreOp::Set { app, .. }
            | StoreOp::Remove { app, .. } => app,
        })
        .collect();
    assert_eq!(apps, ["other", "probe"]);
}

#[test]
fn a_watching_app_hears_of_another_phones_change() {
    let (mut alice, mut bob) = two_phones();
    assert!(alice.app_data.watched().is_empty());
    alice.softkey("probe", "Board");
    assert_eq!(alice.ui.texts("probe"), ["board= more=false"]);
    assert_eq!(alice.app_data.watched(), ["probe"]);

    bob.softkey("probe", "Top");
    assert_eq!(bob.ui.texts("probe"), ["top=Bob"]);
    alice.settle();
    assert_eq!(alice.ui.texts("probe"), ["changed=StorageLocation::PbxShared:best/top"]);
    // Bob's app does not watch, and a phone does not hear of its own
    // writes or of a user's keys.
    alice.softkey("probe", "Top");
    alice.softkey("probe", "Save");
    bob.settle();
    assert_eq!(bob.ui.texts("probe"), ["top=Bob"]);
    assert_eq!(alice.ui.texts("probe"), ["saved=level 5 by Alice (101)"]);

    // An app that has stopped watches no longer.
    alice.mgr.close("probe");
    alice.settle();
    assert!(alice.app_data.watched().is_empty());
}

#[test]
fn a_phone_of_another_tenant_has_another_store() {
    let pbx = FakeStore::default();
    let mut north = Rig::on_pbx(pbx.phone_of_tenant("101", "Alice", "north"));
    let mut south = Rig::on_pbx(pbx.phone_of_tenant("201", "Sam", "south"));
    north.launch("probe");
    south.launch("probe");
    south.softkey("probe", "Board");
    north.softkey("probe", "Top");
    assert_eq!(north.ui.texts("probe"), ["top=Alice"]);
    south.settle();
    south.softkey("probe", "Board");
    assert_eq!(south.ui.texts("probe"), ["board= more=false"]);
    south.softkey("probe", "Top");
    assert_eq!(south.ui.texts("probe"), ["top=Sam"]);
}

#[test]
fn an_app_is_told_when_the_pbx_cannot_be_reached_and_when_it_can_again() {
    let (mut alice, _bob) = two_phones();
    alice.softkey("probe", "Who");
    assert_eq!(alice.ui.texts("probe"), ["who=Some(\"101/Alice/101\") online=true"]);

    alice.app_data.set_online(false);
    alice.softkey("probe", "Who");
    // Who the user is, the phone learned when it last reached the PBX.
    assert_eq!(alice.ui.texts("probe"), ["who=Some(\"101/Alice/101\") online=false"]);
    alice.softkey("probe", "Save");
    assert_eq!(alice.ui.texts("probe"), ["unreachable=not attached to a DPMA server"]);
    // Nothing was kept to send later.
    alice.app_data.set_online(true);
    alice.settle();
    assert_eq!(alice.ui.texts("probe"), ["online"]);
    assert_eq!(alice.app_data.value("probe", StoreScope::User, "progress"), None);
    // The app writes again.
    alice.softkey("probe", "Save");
    assert_eq!(alice.ui.texts("probe"), ["saved=level 5 by Alice (101)"]);
}

#[test]
fn what_the_pbx_refuses_is_reported() {
    let (mut alice, _bob) = two_phones();
    alice.softkey("probe", "Bad");
    assert_eq!(alice.ui.texts("probe"), ["refused=`key` is not a key"]);
}

#[test]
fn a_phone_without_a_pbx_store_answers_unreachable() {
    // The services of the other tests' rigs have a store; these do not.
    let mut r = common::Rig::with_services(|mut services| {
        services.store = std::sync::Arc::new(dphone_apps::NoStore);
        services
    });
    r.launch("probe");
    r.softkey("probe", "Load");
    assert_eq!(r.ui.texts("probe"), ["unreachable=this phone has no PBX"]);
    r.softkey("probe", "Who");
    assert_eq!(r.ui.texts("probe"), ["who=None online=false"]);
}
