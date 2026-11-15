//! BLF and rapid-dial items on the keys: the published `Keys` state for a
//! D65 (paged line keys) and a D70 (strip), page changes, and the return
//! to the first page.

mod common;

use common::{base_config, phone_with, wait};
use dphone_sip::{AccountConfig, BlfConfig, BlfLocation};
use dphone_state::{
    AccountId, BlfState, Blink, Command, Keys, LampColor, LampState, LineKeyBinding, Model, Presence,
};

fn item(id: &str, location: BlfLocation, index: u8) -> BlfConfig {
    BlfConfig {
        id: id.into(),
        label: format!("Label {id}"),
        number: format!("1{id}"),
        subscribe: String::new(),
        account: AccountId(0),
        pickup_ringing: false,
        location,
        index,
        paging: true,
        blank: false,
    }
}

fn config(model: Model, items: Vec<BlfConfig>) -> dphone_sip::SipConfig {
    let mut cfg = base_config();
    cfg.model = model;
    cfg.page_line_keys = model == Model::D65;
    let mut a = AccountConfig::new(AccountId(0), "100", "pw", "127.0.0.1");
    a.register = false;
    a.label = "Line 100".into();
    cfg.accounts.push(a);
    cfg.blf = items;
    cfg
}

fn bindings(keys: &Keys) -> Vec<String> {
    keys.line_keys
        .iter()
        .map(|k| match &k.binding {
            LineKeyBinding::Account(a) => format!("acct{}", a.0),
            LineKeyBinding::Blf(e) => e.clone(),
            LineKeyBinding::Unused => "-".into(),
        })
        .collect()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn d65_line_keys_page() {
    let mut items: Vec<BlfConfig> = (0..7).map(|i| item(&format!("{i}"), BlfLocation::Side, 0)).collect();
    // A blank reserves its key; the same contact may sit on two keys.
    items[1].blank = true;
    items[6].id = "0".into();
    let mut cfg = config(Model::D65, items);
    cfg.blf_page_return_secs = 15;
    let p = phone_with("100", cfg, 4000).await;
    let keys = wait("the keys", || Some(p.store.get::<Keys>()).filter(|k| k.line_keys.len() == 6 && k.rapid_pages == 2)).await;
    assert_eq!(bindings(&keys), ["acct0", "0", "-", "2", "3", "4"]);
    assert_eq!(keys.line_keys[1].label, "Label 0");
    assert_eq!((keys.rapid_page, keys.rapid_dial.len()), (0, 0));
    // One presence entry per contact; none for the blank.
    let ids: Vec<String> = p.store.get::<Presence>().entries.iter().map(|e| e.id.clone()).collect();
    assert_eq!(ids, ["0", "2", "3", "4", "5"]);

    // The lamp of a key follows its contact.
    p.store.update::<Presence, _>(|pr| pr.entries[0].state = BlfState::Ringing);
    p.store.send(Command::Reregister { account: None }).await.ok();
    p.store.send(Command::SetRapidDialPage { page: 1 }).await.expect("page 1");
    let keys = p.store.get::<Keys>();
    assert_eq!(keys.rapid_page, 1);
    assert_eq!(bindings(&keys), ["acct0", "5", "0", "-", "-", "-"]);
    assert_eq!(keys.line_keys[2].lamp, LampState::blinking(LampColor::Red, Blink::Fast));
    assert_eq!(keys.line_keys[0].label, "Line 100");
    assert!(p.store.send(Command::SetRapidDialPage { page: 2 }).await.is_err());

    // After the configured time the first page shows again.
    tokio::time::sleep(std::time::Duration::from_secs(14)).await;
    assert_eq!(p.store.get::<Keys>().rapid_page, 1);
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let keys = wait("the first page", || Some(p.store.get::<Keys>()).filter(|k| k.rapid_page == 0)).await;
    assert_eq!(bindings(&keys), ["acct0", "0", "-", "2", "3", "4"]);
}

#[tokio::test(flavor = "current_thread")]
async fn d70_strip_and_main_items() {
    let items = vec![
        item("a", BlfLocation::Side, 0),
        item("b", BlfLocation::Side, 9),
        item("c", BlfLocation::Side, 0),
        item("d", BlfLocation::Main, 4),
        item("e", BlfLocation::Main, 0),
    ];
    let p = phone_with("100", config(Model::D70, items), 4000).await;
    let keys = wait("the keys", || Some(p.store.get::<Keys>()).filter(|k| k.rapid_pages == 2)).await;
    let strip: Vec<(u8, u8, &str, &str)> =
        keys.rapid_dial.iter().map(|k| (k.page, k.index, k.entry.as_str(), k.number.as_str())).collect();
    assert_eq!(strip, [(0, 0, "a", "1a"), (0, 9, "b", "1b"), (1, 0, "c", "1c")]);
    // The main item is on its line key on both pages; the one aimed at
    // the account's key is dropped.
    assert_eq!(bindings(&keys), ["acct0", "-", "-", "-", "d", "-"]);
    p.store.send(Command::SetRapidDialPage { page: 1 }).await.expect("page 1");
    assert_eq!(bindings(&p.store.get::<Keys>()), ["acct0", "-", "-", "-", "d", "-"]);
}
