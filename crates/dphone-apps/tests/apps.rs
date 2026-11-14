//! The six built-in apps under the real runtime, with a fake screen and a
//! scripted PBX.

mod common;

use common::Rig;
use dphone_apps::testutil::FakeConfig;
use dphone_state::{
    AccountId, Call, CallId, CallState, Command, Direction, EndCause, Event, Key, Party,
};
use serde_json::{Value, json};

fn rig() -> Rig {
    Rig::new(FakeConfig::dasp())
}

#[test]
fn status_lists_options_and_sets_one() {
    let mut r = rig();
    r.pbx.on("switchvox.users.presence.options.getList", |_| {
        Ok(json!({"presence_options": {"presence_option": [
            {"id": "1", "presence": "available", "sub_presence": "", "owner": "system", "active": "1"},
            {"id": "2", "presence": "away", "sub_presence": "Lunch", "owner": "system", "active": "0"},
            {"id": "3", "presence": "dnd", "sub_presence": "", "owner": "system", "active": "0"},
        ]}}))
    });
    r.pbx.on("switchvox.users.presence.getInfo", |_| {
        Ok(json!({"presence": {"id": "1", "presence": "available", "sub_presence": "", "message": ""}}))
    });
    r.pbx.on("switchvox.users.presence.update", |_| {
        Ok(json!({"success": {}}))
    });
    r.launch("status");
    assert_eq!(r.ui.top().as_deref(), Some("status"));
    assert_eq!(
        r.ui.items("status"),
        vec![
            ("Available".to_string(), "*".to_string()),
            ("Away - Lunch".to_string(), String::new()),
            ("Do Not Disturb".to_string(), String::new()),
        ]
    );
    assert_eq!(r.ui.softkeys("status"), ["Select", "Message", "Exit"]);
    // The host added the account and the credentials stayed out of the app.
    assert_eq!(
        r.pbx.last("switchvox.users.presence.getInfo").unwrap()["account_id"],
        "100"
    );
    let raw = &r.pbx.raw_requests()[0];
    assert_eq!(
        (raw.url.as_str(), raw.username.as_str()),
        ("dasp://pbx.example/json", "100")
    );

    r.key("status", Key::Down);
    r.key("status", Key::Select);
    let sent = r.pbx.last("switchvox.users.presence.update").unwrap();
    assert_eq!(sent["presence_option_id"], "2");
    // The phone's own presence follows, and the app leaves the screen.
    assert!(
        r.commands()
            .iter()
            .any(|c| matches!(c, Command::SetPresence { status, .. }
        if *status == dphone_state::PresenceStatus::Away))
    );
    assert_eq!(r.ui.top(), None);
    assert!(
        !r.mgr.is_running("status"),
        "a foreground app exits when it leaves the screen"
    );
}

#[test]
fn status_message_screen_and_error() {
    let mut r = rig();
    // No handlers: the PBX answers with an error.
    r.launch("status");
    assert!(r.ui.shows("status", "Unknown method"));
    assert_eq!(r.ui.softkeys("status"), ["Retry", "Exit"]);
    r.key("status", Key::Cancel);
    assert_eq!(r.ui.top(), None);
}

#[test]
fn parking_lists_and_retrieves() {
    let mut r = rig();
    r.pbx.on("switchvox.users.parkingLots.getList", |_| {
        Ok(json!({"parking_lots": {"parking_lot": [{
        "account_id": "100", "parking_lot_start": "701", "parking_lot_stop": "720",
        "parked_calls": {"parked_call": [
            {"from_caller_id_name": "Alice", "from_caller_id_number": "101",
             "duration": "75", "lotnum": "702", "current_call_id": "PJSIP/101-1"},
            {"from_caller_id_name": "", "from_caller_id_number": "5551234",
             "duration": "5", "lotnum": "701", "current_call_id": "PJSIP/t-2"},
        ]}}]}}))
    });
    r.launch("parking");
    assert_eq!(
        r.ui.items("parking"),
        vec![
            ("701  5551234".to_string(), "0:05".to_string()),
            ("702  Alice".to_string(), "1:15".to_string()),
        ]
    );
    // A park push refreshes the list.
    let before = r.pbx.count("switchvox.users.parkingLots.getList");
    r.store.emit(Event::Push {
        name: "digium.parkingLot.parked".into(),
        body: r#"{"lotnumber":"703"}"#.into(),
    });
    r.settle();
    assert_eq!(
        r.pbx.count("switchvox.users.parkingLots.getList"),
        before + 1
    );

    r.key("parking", Key::Down);
    r.softkey("parking", "Retrieve");
    assert!(
        r.commands()
            .iter()
            .any(|c| matches!(c, Command::Dial { target, .. } if target == "702"))
    );
    assert_eq!(r.ui.top(), None);
}

#[test]
fn queues_login_pause_and_details() {
    let mut r = rig();
    let state = std::sync::Arc::new(std::sync::Mutex::new(("logged_out".to_string(), false)));
    let s = state.clone();
    r.pbx.on("switchvox.users.callQueues.getList", move |_| {
        let (status, paused) = s.lock().unwrap().clone();
        let mut sales = json!({"name": "Sales", "account_id": "600", "loggedin_status": status,
            "total_members": "4", "members_logged_in": "2", "callers_waiting": "1"});
        if paused {
            sales["paused_time"] = json!("1");
        }
        Ok(json!({"callQueues": {"callQueue": [sales,
            {"name": "Support", "account_id": "601", "loggedin_status": "permanent"}]}}))
    });
    let s = state.clone();
    r.pbx.on("switchvox.users.callQueues.login", move |_| {
        s.lock().unwrap().0 = "logged_in".into();
        Ok(json!({"success": {}}))
    });
    let s = state.clone();
    r.pbx.on("switchvox.users.callQueues.pause", move |_| {
        s.lock().unwrap().1 = true;
        Ok(json!({"success": {}}))
    });
    r.pbx.on("switchvox.users.callQueues.getTodaysStatus", |_| {
        Ok(
            json!({"callQueue": {"overview": {"total_members": "4", "logged_in_count": "2",
            "members_on_call": "1", "callers_waiting": "3", "total_calls": "40",
            "completed_calls_today": "35", "abandoned_calls": "5"}}}),
        )
    });
    r.launch("queues");
    assert_eq!(
        r.ui.items("queues"),
        vec![
            ("Sales".to_string(), "Out".to_string()),
            ("Support".to_string(), "Always".to_string())
        ]
    );
    assert_eq!(r.ui.softkeys("queues"), ["Log In", "Details", "Exit"]);

    r.softkey("queues", "Log In");
    assert_eq!(
        r.pbx.last("switchvox.users.callQueues.login").unwrap()["call_queue_account_id"],
        "600"
    );
    assert_eq!(r.ui.items("queues")[0].1, "In");
    assert_eq!(
        r.ui.softkeys("queues"),
        ["Log Out", "Pause", "Details", "Exit"]
    );

    r.softkey("queues", "Pause");
    r.ui.type_text("queues", "Lunch");
    r.settle();
    r.softkey("queues", "Pause");
    let p = r.pbx.last("switchvox.users.callQueues.pause").unwrap();
    assert_eq!(
        (p["call_queue_account_id"].as_str(), p["reason"].as_str()),
        (Some("600"), Some("Lunch"))
    );
    assert_eq!(r.ui.items("queues")[0].1, "Paused");

    r.softkey("queues", "Details");
    assert!(r.ui.shows("queues", "Waiting: 3"));
    assert!(r.ui.shows("queues", "Abandoned: 5"));
    r.key("queues", Key::Cancel);
    assert_eq!(r.ui.items("queues").len(), 2);

    // Queues keeps running in the background, as the stock app does.
    r.key("queues", Key::Cancel);
    assert_eq!(r.ui.top(), None);
    assert!(r.mgr.is_running("queues"));
}

#[test]
fn hotdesking_logs_in_and_out() {
    let mut r = rig();
    r.launch("hotdesking");
    assert_eq!(r.ui.softkeys("hotdesking"), ["Log In", "Cancel"]);
    r.ui.type_text("hotdesking", "101");
    r.settle();
    r.key("hotdesking", Key::Select); // moves to the PIN field
    r.ui.type_text("hotdesking", "4321");
    r.settle();
    r.softkey("hotdesking", "Log In");
    assert!(
        r.commands()
            .iter()
            .any(|c| matches!(c, Command::SelectUser { id, pin } if id == "101" && pin == "4321"))
    );
    assert_eq!(r.ui.top(), None);

    r.store
        .update::<dphone_state::Provisioning, _>(|p| p.assigned_user = "101".into());
    r.launch("hotdesking");
    assert!(r.ui.shows("hotdesking", "Logged in as 101"));
    r.softkey("hotdesking", "Log Out");
    r.softkey("hotdesking", "Yes");
    assert!(
        r.commands()
            .iter()
            .any(|c| matches!(c, Command::SelectUser { id, .. } if id.is_empty()))
    );
}

const CONTACTS: &str = r#"<contacts group_name="Internal Extensions" editable="0" >
<contact first_name="Alice" last_name="Zeta" contact_type="sip" subscribe_to="auto_hint_101">
<numbers><number dial="101" label="Extension" primary="1" /><number dial="5550101" label="Mobile" /></numbers></contact>
<contact first_name="Bob" last_name="Young" contact_type="sip" subscribe_to="auto_hint_102">
<numbers><number dial="102" label="Extension" primary="1" /></numbers></contact>
</contacts>"#;

fn contacts_config(extra_group: bool) -> FakeConfig {
    let mut c = FakeConfig::dasp();
    c.contact_lists.push(("0".into(), CONTACTS.into()));
    if extra_group {
        c.contact_lists.push((
            "1".into(),
            r#"<contacts group_name="Vendors"><contact first_name="Carol" last_name="">
               <numbers><number dial="918005550199" label="Office" primary="1"/></numbers></contact></contacts>"#
                .into(),
        ));
    }
    // The phone's merged copy must not make everything show twice.
    c.contact_lists.push((
        "serialized_contacts.xml".into(),
        format!("<merged_contacts>{CONTACTS}</merged_contacts>"),
    ));
    c
}

#[test]
fn contacts_browse_search_and_dial() {
    let mut r = Rig::new(contacts_config(false));
    r.store.update::<dphone_state::Presence, _>(|p| {
        p.entries.push(dphone_state::BlfEntry {
            id: "c1".into(),
            number: "101".into(),
            state: dphone_state::BlfState::InUse,
            ..Default::default()
        })
    });
    r.launch("contacts");
    assert_eq!(
        r.ui.items("contacts"),
        vec![
            ("Alice Zeta".to_string(), "On call".to_string()),
            ("Bob Young".to_string(), String::new())
        ]
    );
    // Presence changes update the list in place.
    r.store
        .update::<dphone_state::Presence, _>(|p| p.entries[0].state = dphone_state::BlfState::Idle);
    r.settle();
    assert_eq!(r.ui.items("contacts")[0].1, "Available");

    r.softkey("contacts", "Search");
    r.ui.type_text("contacts", "bo");
    r.settle();
    assert_eq!(
        r.ui.items("contacts"),
        vec![("Bob Young".to_string(), String::new())]
    );
    // Digits match names by keypad letters (9 = w, x, y, z).
    r.ui.type_text("contacts", "93");
    r.settle();
    assert_eq!(r.ui.items("contacts")[0].0, "Alice Zeta");
    r.softkey("contacts", "Details");
    assert_eq!(
        r.ui.items("contacts"),
        vec![
            ("Extension".to_string(), "101".to_string()),
            ("Mobile".to_string(), "5550101".to_string())
        ]
    );
    r.key("contacts", Key::Down);
    r.softkey("contacts", "Dial");
    assert!(
        r.commands()
            .iter()
            .any(|c| matches!(c, Command::Dial { target, .. } if target == "5550101"))
    );
    assert_eq!(r.ui.top(), None);
}

#[test]
fn contacts_groups() {
    let mut r = Rig::new(contacts_config(true));
    r.launch("contacts");
    assert_eq!(
        r.ui.items("contacts"),
        vec![
            ("Internal Extensions".to_string(), "2".to_string()),
            ("Vendors".to_string(), "1".to_string())
        ]
    );
    r.key("contacts", Key::Down);
    r.key("contacts", Key::Select);
    assert_eq!(
        r.ui.items("contacts"),
        vec![("Carol".to_string(), String::new())]
    );
    r.key("contacts", Key::Select);
    assert!(
        r.commands()
            .iter()
            .any(|c| matches!(c, Command::Dial { target, .. } if target == "918005550199"))
    );
}

fn voicemail_pbx(r: &Rig) {
    r.pbx.on("switchvox.users.voicemail.getList", |p| {
        assert_eq!(p["sort_field"], "date");
        if p["folder"] != "INBOX" {
            return Ok(json!({"messages": {"folder": p["folder"], "message": []}}));
        }
        Ok(
            json!({"messages": {"folder": "INBOX", "page_number": "1", "total_pages": "1",
            "items_per_page": "30", "total_items": "2", "message": [
            {"id": "msg0001", "caller_id": "\"Alice\" <101>", "caller_id_number": "101",
             "caller_id_name": "Alice", "date": "1794607500", "duration": "65", "read": "0"},
            {"id": "msg0000", "caller_id": "5551234", "caller_id_number": "5551234",
             "caller_id_name": "", "date": "1794500000", "duration": "9", "read": "1"}]}}),
        )
    });
    r.pbx.on("switchvox.users.voicemail.getFolderList", |_| {
        Ok(json!({"folders": {"folder": [{"name": "INBOX", "count": "2"}, {"name": "Old", "count": "0"}]}}))
    });
    for m in ["play", "markRead", "markUnread", "remove", "move"] {
        r.pbx.on(&format!("switchvox.users.voicemail.{m}"), |_| {
            Ok(json!({"success": {}}))
        });
    }
}

#[test]
fn voicemail_list_play_and_controls() {
    let mut r = rig();
    voicemail_pbx(&r);
    r.launch("voicemail");
    let items = r.ui.items("voicemail");
    assert_eq!(items[0].0, "* Alice");
    assert_eq!(items[0].1, "Nov 13, 2026 10:05 PM");
    assert_eq!(items[1].0, "5551234");

    r.softkey("voicemail", "Play");
    let play = r.pbx.last("switchvox.users.voicemail.play").unwrap();
    assert_eq!(
        (play["folder"].as_str(), play["message_id"].as_str()),
        (Some("INBOX"), Some("msg0001"))
    );
    assert!(r.ui.shows("voicemail", "Connecting"));

    // The PBX rings the phone with the playback event; the app answers.
    let mut call = Call::new(
        CallId(9),
        AccountId(0),
        Direction::Incoming,
        Party::default(),
    );
    call.state = CallState::Ringing;
    r.store
        .update::<dphone_state::Calls, _>(|c| c.calls.push(call.clone()));
    r.store.emit(Event::Push {
        name: "digium.incomingCall.voicemail".into(),
        body: String::new(),
    });
    r.settle();
    assert!(
        r.commands()
            .iter()
            .any(|c| matches!(c, Command::Answer { call, .. } if call.0 == 9))
    );

    r.key("voicemail", Key::Star);
    assert!(
        r.commands()
            .iter()
            .any(|c| matches!(c, Command::Dtmf { call, digit: '*' } if call.0 == 9))
    );

    // Hanging up ends playback and marks the message read.
    call.state = CallState::Ended(EndCause::RemoteHangup);
    r.store
        .update::<dphone_state::Calls, _>(|c| c.calls.clear());
    r.store.emit(Event::CallEnded { call });
    r.settle();
    let mark = r.pbx.last("switchvox.users.voicemail.markRead").unwrap();
    assert_eq!(mark["message_ids"], json!(["msg0001"]));
    assert_eq!(r.ui.items("voicemail").len(), 2, "back on the list");
}

#[test]
fn voicemail_rejects_playback_call_when_not_on_screen() {
    let mut r = rig();
    voicemail_pbx(&r);
    r.launch("voicemail");
    r.key("voicemail", Key::Cancel);
    assert!(r.mgr.is_running("voicemail"));
    let mut call = Call::new(
        CallId(11),
        AccountId(0),
        Direction::Incoming,
        Party::default(),
    );
    call.state = CallState::Ringing;
    r.store
        .update::<dphone_state::Calls, _>(|c| c.calls.push(call));
    r.store.emit(Event::Push {
        name: "digium.incomingCall.voicemail".into(),
        body: r#"{"method":"INVITE","call":11,"data":""}"#.into(),
    });
    r.settle();
    assert!(
        r.commands()
            .iter()
            .any(|c| matches!(c, Command::Reject { call, .. } if call.0 == 11))
    );
}

#[test]
fn voicemail_delete_move_and_folders() {
    let mut r = rig();
    voicemail_pbx(&r);
    r.launch("voicemail");
    r.softkey("voicemail", "Delete");
    assert!(r.ui.shows("voicemail", "Delete the message from Alice?"));
    r.softkey("voicemail", "Yes");
    let rm = r.pbx.last("switchvox.users.voicemail.remove").unwrap();
    assert_eq!(
        (rm["folder"].as_str(), &rm["message_ids"]),
        (Some("INBOX"), &json!(["msg0001"]))
    );

    r.key("voicemail", Key::Down);
    r.softkey("voicemail", "Move");
    assert_eq!(
        r.ui.items("voicemail")[1],
        ("Old".to_string(), "0".to_string())
    );
    r.key("voicemail", Key::Down);
    r.softkey("voicemail", "Move");
    let mv = r.pbx.last("switchvox.users.voicemail.move").unwrap();
    assert_eq!(
        (
            mv["folder_from"].as_str(),
            mv["folder_to"].as_str(),
            &mv["message_ids"]
        ),
        (Some("INBOX"), Some("Old"), &json!(["msg0000"]))
    );

    r.softkey("voicemail", "Folders");
    r.key("voicemail", Key::Down);
    r.key("voicemail", Key::Select);
    assert!(r.ui.shows("voicemail", "No messages."));
    let last: Value = r.pbx.last("switchvox.users.voicemail.getList").unwrap();
    assert_eq!(last["folder"], "Old");
}

#[test]
fn apps_fit_every_skin() {
    // Each app must start and draw on the small gray and large gray
    // screens too (240x102 and 320x140 content areas).
    for model in [dphone_state::Model::D40, dphone_state::Model::D70] {
        let mut r = Rig::with_root(common::staged(), contacts_config(false), model);
        voicemail_pbx(&r);
        for app in [
            "contacts",
            "voicemail",
            "status",
            "parking",
            "queues",
            "hotdesking",
        ] {
            r.launch(app);
            assert!(
                r.ui.widget_count(app) > 0,
                "{app} drew nothing on {model:?}"
            );
            r.mgr.close(app);
        }
    }
}
