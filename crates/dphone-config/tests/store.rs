mod common;

use std::net::Ipv4Addr;

use common::{FETCHED, TempDir};
use dphone_config::view::Transport;
use dphone_config::{ConfigError, ConfigStore, Element, Layer, Model, Nvdata, SavedTime};

fn store(model: Model) -> (TempDir, ConfigStore) {
    let dir = TempDir::new("store");
    let nv = Nvdata::new(dir.path().join("nvdata"));
    nv.ensure_layout().unwrap();
    let store = ConfigStore::open(nv, model, dir.path().join("tmp")).unwrap();
    (dir, store)
}

#[test]
fn factory_layers_freeze_model_facts() {
    let expect = [
        (Model::D40, 2, "240", "120"),
        (Model::D45, 2, "240", "120"),
        (Model::D50, 4, "240", "120"),
        (Model::D60, 2, "480", "272"),
        (Model::D62, 2, "480", "272"),
        (Model::D65, 6, "480", "272"),
        (Model::D70, 6, "320", "160"),
    ];
    for (model, accounts, w, h) in expect {
        let (_d, mut s) = store(model);
        let c = s.config();
        assert!(s.warnings().is_empty());
        assert_eq!(c.setting("phone_model"), Some(model.name()));
        assert_eq!(c.max_accounts(), accounts);
        assert_eq!(c.setting("width"), Some(w));
        assert_eq!(c.setting("height"), Some(h));
        for id in ["phone_model", "max_accounts", "width", "height"] {
            assert!(c.setting_frozen(id), "{model} {id}");
            assert!(matches!(s.set(id, "9"), Err(ConfigError::Frozen(_))));
        }
        assert_eq!(c.setting("enable_blf_on_unused_line_keys"), Some("1"));
        assert_eq!(c.softkeys("idle").len(), 6);
        assert_eq!(c.keymap_states().len(), if model == Model::D70 { 18 } else { 19 });
        assert_eq!(c.enabled_codecs().iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), [
            "PCMU", "PCMA", "G722", "G726-32"
        ]);
        // A server cannot raise the account limit either.
        s.apply_fetched(r#"<config><setting id="max_accounts" value="9"/></config>"#).unwrap();
        assert_eq!(s.config().max_accounts(), accounts);
    }
}

#[test]
fn set_goes_to_the_ui_layer_and_config_xml_stays_stock() {
    let (_d, mut s) = store(Model::D60);
    let g0 = s.config().generation();
    let c = s.set_many(&[("ringer_volume", "7")]).unwrap();
    assert!(c.generation() > g0);
    assert_eq!(c.setting("ringer_volume"), Some("7"));

    let nv = s.nvdata().clone();
    let ui = nv.read_string("ui-config.xml").unwrap().unwrap();
    assert_eq!(ui, "<config>\n    <setting id=\"ringer_volume\" value=\"7\" />\n</config>\n");
    // The retired private IDs are refused, so they cannot reach a stock file.
    assert!(matches!(s.set("open_ssh_enabled", "1"), Err(ConfigError::NotAStockSetting(_))));
    assert_eq!(nv.read_string("ui-config.xml").unwrap().unwrap(), ui);
    // The effective file stock reads carries nothing of ours.
    let effective = nv.read_string("config.xml").unwrap().unwrap();
    assert!(effective.contains("<setting id=\"ringer_volume\" value=\"7\" />"));
    assert!(!effective.contains("open_"));
    assert!(!nv.exists("open/config.xml"));

    // Same value again: no new generation.
    let g1 = c.generation();
    assert_eq!(s.set("ringer_volume", "7").unwrap().generation(), g1);
    // Unset returns to the default.
    assert_eq!(s.unset("ringer_volume").unwrap().setting("ringer_volume"), Some("5"));
    // No temp files left behind.
    assert!(std::fs::read_dir(nv.root()).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".tmp")));
}

#[test]
fn fetched_config_network_selection_and_typed_view() {
    let (_d, mut s) = store(Model::D60);
    s.apply_fetched(FETCHED).unwrap();

    // No address yet: only the /0 profile matches.
    let c = s.config();
    assert_eq!(c.selected_network(), Some("any"));
    let a = c.accounts();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].primary.as_ref().unwrap().server, "pbx.example.com");
    assert_eq!(c.logging().server, None);

    // On the office network the longer prefix wins.
    let c = s.set_ip(Some(Ipv4Addr::new(10, 10, 15, 87))).unwrap();
    assert_eq!(c.selected_network(), Some("office"));
    let a = c.accounts();
    // Three provisioned, but a D60 has two accounts.
    assert_eq!(a.len(), 2);
    assert_eq!((a[0].username.as_str(), a[0].authname.as_str(), a[0].line_label.as_str()), ("101", "101", "Alice"));
    let p = a[0].primary.as_ref().unwrap();
    assert_eq!((p.server.as_str(), p.port, p.transport, p.reregister, p.retry), ("10.10.0.2", 5061, Transport::Tls, 120, 20));
    assert_eq!(p.media_encryption.as_deref(), Some("sdes"));
    assert_eq!(a[0].alternate.as_ref().unwrap().server, "10.10.0.3");
    assert!(a[0].visual_voicemail && a[0].register && a[0].enabled);
    assert_eq!(a[1].primary.as_ref().unwrap().transport, Transport::Tcp);

    let log = c.logging();
    assert!(log.remote_enabled);
    assert_eq!((log.server.as_deref(), log.port), (Some("10.10.0.5"), 514));
    assert_eq!(c.time().time_zone, "America/Denver");
    assert_eq!(c.firmware_for_model().unwrap().version, "2_9_28");
    assert_eq!(c.firmwares().len(), 2);
    assert!(!c.access().ignore_provisioned_firmware);
    assert_eq!(c.contact_sources()[0].url, "http://10.10.0.2/contacts.xml");
    assert_eq!(c.blf_items_url(), Some("http://10.10.0.2/blf.xml"));
    let r = c.ringtones();
    assert_eq!(r.active, "Chime");
    assert_eq!(r.tones.len(), 4);
    assert_eq!(r.alerts[0].ring_type, "answer");
    assert_eq!(c.multicast_listeners()[0].addr, "239.1.1.1");
    assert_eq!(c.multicast_listeners()[0].interrupt_calls, 1);
    let groups = c.multicast_broadcasts();
    assert_eq!(groups.iter().map(|b| (b.id.as_str(), b.name.as_str(), b.codec.as_str())).collect::<Vec<_>>(), [
        ("b1", "Warehouse", "G722"),
        ("b2", "Office", "PCMU"),
    ]);
    let idle = c.softkeys("idle");
    assert_eq!(idle.len(), 2);
    assert_eq!(idle[1].args, ["voicemail"]);
    assert_eq!(c.softkeys("hold").len(), 4);
    assert_eq!(c.activity_indicator("hold").unwrap().color, "red");
    assert_eq!(c.locale_setting("tone_country"), Some("us"));
    assert_eq!(c.network().sip_dscp, 26);

    // A local value beats the provisioned one.
    assert_eq!(s.set("ringer_volume", "9").unwrap().setting("ringer_volume"), Some("9"));

    // Explicit selection overrides the address match.
    let c = s.select_network(Some("any")).unwrap();
    assert_eq!(c.selected_network(), Some("any"));
    assert_eq!(c.accounts().len(), 1);
    let c = s.select_network(None).unwrap();
    assert_eq!(c.selected_network(), Some("office"));

    // Local accounts replace the provisioned collection.
    let acct = Element::parse(
        r#"<account index="0" username="200" authname="200" number="200" password="x" passcode="x" display="200" line_label="200" register="1" status="1" type="sip"><host_primary server="192.0.2.1" port="5060"/></account>"#,
    )
    .unwrap();
    let c = s.set_local_accounts(std::slice::from_ref(&acct)).unwrap();
    assert_eq!(c.accounts().len(), 1);
    assert_eq!(c.accounts()[0].username, "200");
    assert!(matches!(
        s.set_local_accounts(&[acct.clone(), acct.clone(), acct.clone()]),
        Err(ConfigError::TooManyAccounts { max: 2, given: 3 })
    ));
    assert_eq!(s.clear_local_accounts().unwrap().accounts().len(), 2);
}

#[test]
fn lldp_layer_and_unparsable_layers() {
    let (_d, mut s) = store(Model::D40);
    let c = s.set_lldp(Some(&[("network_vlan_id", "20"), ("network_vlan_qos", "5")])).unwrap();
    assert_eq!(c.network().vlan_id, Some(20));
    // The stock merge renames this legacy ID.
    assert_eq!(c.network().rtp_qos, 5);
    assert_eq!(s.set_lldp(None).unwrap().network().vlan_id, None);

    // A truncated layer is skipped with a warning; the rest still merges.
    s.nvdata().write("fetched-config.xml", b"<config><setting id=\"ringer_volume\" value=\"1\"/>").unwrap();
    s.set("handset_volume", "2").unwrap();
    assert_eq!(s.warnings().len(), 1);
    assert_eq!(s.config().setting("ringer_volume"), Some("5"));
    assert_eq!(s.config().setting("handset_volume"), Some("2"));
    assert!(matches!(s.write_layer(Layer::G729Legacy, &Element::new("config")), Err(ConfigError::ReadOnlyLayer(_))));
}

#[test]
fn web_password() {
    let (_d, mut s) = store(Model::D60);
    // No digest file: the factory default, admin / 789, and nothing else.
    assert!(!s.nvdata().exists("etc/.passwd"));
    assert!(s.verify_web_password("admin", "789").unwrap());
    assert!(!s.verify_web_password("admin", "nope").unwrap());
    assert!(!s.verify_web_password("admin", "").unwrap());
    assert!(!s.verify_web_password("root", "789").unwrap());
    // The same hash stock ships in its default digest file.
    assert_eq!(s.web_ha1("admin").unwrap().as_deref(), Some("42c4089ad2eabaeb4d56e37078e21d62"));
    // Checking a login writes nothing.
    assert!(!s.nvdata().exists("etc/.passwd"));
    s.set_web_password("s3cret").unwrap();
    assert!(s.verify_web_password("admin", "s3cret").unwrap());
    assert!(!s.verify_web_password("admin", "789").unwrap());
    let file = s.nvdata().read_string("etc/.passwd").unwrap().unwrap();
    // MD5("admin:Digium Phone Admin Login:s3cret"), htdigest format, no newline.
    assert_eq!(file, format!("admin:Digium Phone Admin Login:{}", s.web_ha1("admin").unwrap().unwrap()));
    assert_eq!(file.len(), 63);
    assert_eq!(s.config().setting("login_password"), Some("s3cret"));
    // The RFC 2617 example credentials.
    assert_eq!(
        dphone_config::nvdata::htdigest_ha1("Mufasa", "testrealm@host.com", "Circle Of Life"),
        "939e7578ed9e3c518a452acee763bce9"
    );
}

#[test]
fn default_web_password_without_file_or_setting() {
    // A configuration with no login_password at all (not even the base
    // default) still accepts exactly admin / 789.
    let (_d, mut s) = store(Model::D40);
    s.apply_fetched(r#"<config><setting id="login_password" conflict="replace"/></config>"#).unwrap();
    assert_eq!(s.config().setting("login_password"), None);
    assert!(!s.nvdata().exists("etc/.passwd"));
    assert!(s.verify_web_password("admin", "789").unwrap());
    assert!(!s.verify_web_password("admin", "").unwrap());
    assert!(!s.verify_web_password("admin", "7890").unwrap());
    // An empty digest file has no entry for the user either.
    s.nvdata().write("etc/.passwd", b"").unwrap();
    assert!(s.verify_web_password("admin", "789").unwrap());
    // First change: written in the stock htdigest form, and then only the
    // new password works.
    s.set_web_password("fresh").unwrap();
    assert_eq!(
        s.nvdata().read_string("etc/.passwd").unwrap().unwrap(),
        format!("admin:Digium Phone Admin Login:{}", dphone_config::nvdata::htdigest_ha1("admin", "Digium Phone Admin Login", "fresh"))
    );
    assert!(s.verify_web_password("admin", "fresh").unwrap());
    assert!(!s.verify_web_password("admin", "789").unwrap());
}

#[test]
fn nvdata_files() {
    let dir = TempDir::new("nv");
    let nv = Nvdata::new(dir.path());
    nv.ensure_layout().unwrap();

    nv.write("etc/enet.cfg", b"ENETCFG_MAC=00:0f:d3:01:02:03\nENETCFG_DHCP=1\nENETCFG_IPADDR=\n").unwrap();
    assert_eq!(nv.mac().unwrap(), Some([0x00, 0x0f, 0xd3, 1, 2, 3]));
    let mut cfg = nv.enet_cfg().unwrap().unwrap();
    assert!(cfg.dhcp());
    cfg.set("ENETCFG_DHCP", "0");
    nv.write_enet_cfg(&cfg).unwrap();
    assert_eq!(nv.read_string("etc/enet.cfg").unwrap().unwrap(), "ENETCFG_MAC=00:0f:d3:01:02:03\nENETCFG_DHCP=0\nENETCFG_IPADDR=\n");

    let t = SavedTime::parse("Nov 12 16:14:43 2026\n").unwrap();
    assert_eq!(t.to_unix_utc(), 1_794_500_083);
    assert_eq!(SavedTime::from_unix_utc(1_794_500_083), t);
    assert_eq!(t.to_text(), "Nov 12 16:14:43 2026\n");
    nv.write_systime(&t).unwrap();
    assert_eq!(nv.systime().unwrap(), Some(t));

    nv.write_reboot_reason("dphone", "firmware update").unwrap();
    assert_eq!(nv.reboot_reason().unwrap().as_deref(), Some("dphone: firmware update"));

    // .tmp files are never read.
    std::fs::write(dir.path().join("config.xml.tmp"), b"<config>").unwrap();
    assert!(nv.read("config.xml.tmp").is_err());
    assert!(!nv.list("").unwrap().contains(&"config.xml.tmp".to_owned()));

    nv.open_put("ssh_host_ed25519", b"key").unwrap();
    assert_eq!(nv.open_get("ssh_host_ed25519").unwrap().as_deref(), Some(&b"key"[..]));
    assert!(nv.open_put("../x", b"").is_err());
    assert!(nv.open_get("a/b").is_err());
    assert_eq!(nv.open_names().unwrap(), ["ssh_host_ed25519"]);

    nv.write_authorized_keys(&["ssh-ed25519 AAAA dev".to_owned()]).unwrap();
    std::fs::write(nv.path("root/.ssh/authorized_keys"), "# c\n\nssh-ed25519 AAAA dev\n").unwrap();
    assert_eq!(nv.authorized_keys().unwrap(), ["ssh-ed25519 AAAA dev"]);

    nv.write_cached_md5("config", "0123456789abcdef0123456789abcdef").unwrap();
    assert_eq!(nv.cached_md5("config").unwrap().unwrap().len(), 32);
    assert_eq!(dphone_config::nvdata::md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
}

#[test]
fn factory_reset_follows_stock_whitelist() {
    let dir = TempDir::new("reset");
    let nv = Nvdata::new(dir.path());
    nv.ensure_layout().unwrap();
    for f in [
        "etc/enet.cfg", "etc/.passwd", "etc/passwd", "reboot_reason", "phoneservice_lockdown",
        "g729-legacy-support.xml", "g729-config.xml", "if-setup-debug", "config.xml", "ui-config.xml",
        "fetched-config.xml", "systime", "certs/a.pem", "appdata/x/y", "root/.ssh/authorized_keys",
        "open/kv/ssh_host", "open/kv/trial", "dphone_settings.txtpb",
    ] {
        nv.write(f, b"x").unwrap();
    }
    assert!(!ConfigStore::run_pending_factory_reset(&nv, &[]).unwrap());
    nv.request_factory_reset().unwrap();
    assert!(ConfigStore::run_pending_factory_reset(&nv, &["open/kv/trial"]).unwrap());

    let mut left = Vec::new();
    fn walk(base: &std::path::Path, p: &std::path::Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(p).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                walk(base, &e.path(), out);
            } else {
                out.push(e.path().strip_prefix(base).unwrap().to_string_lossy().into_owned());
            }
        }
    }
    walk(dir.path(), dir.path(), &mut left);
    left.sort();
    assert_eq!(left, [
        "etc/enet.cfg", "g729-config.xml", "g729-legacy-support.xml", "if-setup-debug",
        "open/kv/trial", "phoneservice_lockdown", "reboot_reason",
    ]);
    assert!(!nv.factory_reset_requested());
}

#[test]
fn contacts_document() {
    let groups = dphone_config::contacts::parse_contacts(
        r#"<phonebooks><contacts id="example" group_name="Example">
            <contact id="1001" first_name="Reception" last_name="" contact_type="sip" subscribe_to="1001">
              <numbers><number dial="1001" label="Extension" primary="1"/><number dial="5551001" label="Cell"/></numbers>
            </contact></contacts></phonebooks>"#,
    )
    .unwrap();
    assert_eq!(groups.len(), 1);
    let c = &groups[0].contacts[0];
    assert_eq!((c.id.as_str(), c.first_name.as_str(), c.subscribe_to.as_deref()), ("1001", "Reception", Some("1001")));
    assert_eq!(c.primary_number().unwrap().dial, "1001");
}
