//! `/nvdata/dphone_settings.txtpb` through the configuration store.

mod common;

use common::TempDir;
use dphone_config::{ConfigStore, DphoneSettings, Model, Nvdata, Resolved, SettingSource};

const FILE: &str = "dphone_settings.txtpb";

fn store() -> (TempDir, Nvdata, ConfigStore) {
    let dir = TempDir::new("settings");
    let nv = Nvdata::new(dir.path().join("nvdata"));
    nv.ensure_layout().unwrap();
    let store = ConfigStore::open(nv.clone(), Model::D60, dir.path().join("tmp")).unwrap();
    (dir, nv, store)
}

fn reopen(dir: &TempDir, nv: &Nvdata) -> ConfigStore {
    ConfigStore::open(nv.clone(), Model::D60, dir.path().join("tmp")).unwrap()
}

#[test]
fn defaults_with_no_file() {
    let (_d, nv, s) = store();
    assert!(!nv.exists(FILE));
    assert_eq!(*s.dphone_settings(), DphoneSettings::default());
    let c = s.config();
    let off = Resolved { value: false, source: SettingSource::Default, locked: false };
    let on = Resolved { value: true, ..off };
    assert_eq!((c.ssh_enabled(), c.ignore_provisioned_firmware()), (off, off));
    // UART A is the EHS headset adapter's unless the console is asked for.
    assert_eq!((c.serial_console(), c.metrics_enabled()), (off, on));
    let a = c.access();
    // SSH off; web configuration on; the server's firmware is followed, as
    // on stock.
    assert!(!a.ssh_enabled && a.web_ui_enabled && !a.ignore_provisioned_firmware);
    // Opening the store does not create the file.
    assert!(!nv.exists(FILE));
}

#[test]
fn update_round_trip_presence_and_generation() {
    let (dir, nv, mut s) = store();
    let g0 = s.config().generation();
    let c = s.update_dphone_settings(|d| d.ssh_enabled = Some(false)).unwrap();
    assert_eq!(c.generation(), g0 + 1);
    assert!(!c.access().ssh_enabled);
    assert!(!c.access().ignore_provisioned_firmware);
    // Only the field that was set is written.
    let text = nv.read_string(FILE).unwrap().unwrap();
    let fields: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(fields, ["ssh_enabled: false"]);
    // No temp file left behind.
    assert_eq!(nv.list("").unwrap().iter().filter(|n| n.starts_with("dphone_settings")).count(), 1);

    // A change that changes nothing: no write, no new generation.
    let before = std::fs::metadata(nv.path(FILE)).unwrap().modified().unwrap();
    let c = s.update_dphone_settings(|d| d.ssh_enabled = Some(false)).unwrap();
    assert_eq!(c.generation(), g0 + 1);
    assert_eq!(std::fs::metadata(nv.path(FILE)).unwrap().modified().unwrap(), before);

    // Explicit false differs from unset, though both read as disabled.
    assert_eq!(c.dphone_settings().ssh_enabled, Some(false));
    let c = s.update_dphone_settings(|d| d.ssh_enabled = Some(true)).unwrap();
    assert_eq!(c.generation(), g0 + 2);
    assert_eq!(c.dphone_settings().ssh_enabled, Some(true));
    assert!(c.access().ssh_enabled);
    assert!(nv.read_string(FILE).unwrap().unwrap().contains("ssh_enabled: true"));
    let c = s.update_dphone_settings(|d| d.ssh_enabled = None).unwrap();
    assert_eq!(c.generation(), g0 + 3);
    assert_eq!(c.dphone_settings().ssh_enabled, None);
    assert!(!c.access().ssh_enabled);
    assert!(!nv.read_string(FILE).unwrap().unwrap().contains("ssh_enabled"));

    // Survives a restart.
    s.update_dphone_settings(|d| {
        d.ssh_enabled = Some(true);
        d.ignore_provisioned_firmware = Some(true);
    })
    .unwrap();
    let s2 = reopen(&dir, &nv);
    let a = s2.config().access();
    assert!(a.ssh_enabled && a.ignore_provisioned_firmware);

    // The stock-facing files are untouched by all this.
    assert!(!nv.read_string("config.xml").unwrap().unwrap().contains("ssh"));
    assert!(!nv.exists("ui-config.xml"));

    // A stock setting change does not disturb the settings file either.
    let text = nv.read_string(FILE).unwrap().unwrap();
    s.set("ringer_volume", "2").unwrap();
    assert_eq!(nv.read_string(FILE).unwrap().unwrap(), text);
}

#[test]
fn file_edited_by_hand_is_picked_up_on_reload() {
    let (_d, nv, mut s) = store();
    let g0 = s.config().generation();
    nv.write(FILE, b"# by hand\nssh_enabled: false\n").unwrap();
    let c = s.reload().unwrap();
    assert_eq!(c.generation(), g0 + 1);
    assert!(!c.access().ssh_enabled);
}

#[test]
fn unknown_fields_survive_an_update() {
    let (dir, nv, mut s) = store();
    nv.write(FILE, b"ssh_enabled: true\nring_pattern: \"double\"  # from a newer firmware\nled { brightness: 3 }\n")
        .unwrap();
    let c = s.update_dphone_settings(|d| d.ssh_enabled = Some(false)).unwrap();
    assert!(!c.access().ssh_enabled);
    let text = nv.read_string(FILE).unwrap().unwrap();
    assert!(text.contains("ssh_enabled: false\n"));
    assert!(text.contains("ring_pattern: \"double\"  # from a newer firmware\n"));
    assert!(text.contains("led { brightness: 3 }\n"));
    // And again through a restart and a second update.
    let mut s2 = reopen(&dir, &nv);
    s2.update_dphone_settings(|d| d.ignore_provisioned_firmware = Some(true)).unwrap();
    let text = nv.read_string(FILE).unwrap().unwrap();
    assert!(text.contains("ring_pattern: \"double\"") && text.contains("led { brightness: 3 }"));
    assert!(!nv.exists("dphone_settings.txtpb.bad"));
}

#[test]
fn malformed_file_is_set_aside_and_defaults_apply() {
    let dir = TempDir::new("settings-bad");
    let nv = Nvdata::new(dir.path().join("nvdata"));
    nv.ensure_layout().unwrap();
    let bad = b"ssh_enabled: true\nignore_provisioned_firmware: {{{ not text format\n";
    nv.write(FILE, bad).unwrap();
    let mut s = reopen(&dir, &nv);
    // Configuration still comes up, on defaults.
    let c = s.config();
    assert!(!c.access().ssh_enabled && !c.access().ignore_provisioned_firmware);
    assert_eq!(c.setting("phone_model"), Some("D60"));
    // The bad file is kept for inspection, not deleted and not left in place.
    assert!(!nv.exists(FILE));
    assert_eq!(nv.read("dphone_settings.txtpb.bad").unwrap().unwrap(), bad);
    // And the store keeps working.
    let c = s.update_dphone_settings(|d| d.ssh_enabled = Some(true)).unwrap();
    assert!(c.access().ssh_enabled);
    assert!(nv.exists(FILE));
}

#[test]
fn legacy_open_layer_is_migrated_once() {
    let dir = TempDir::new("settings-migrate");
    let nv = Nvdata::new(dir.path().join("nvdata"));
    nv.ensure_layout().unwrap();
    nv.write(
        "open/config.xml",
        b"<config>\n    <setting id=\"open_ssh_enabled\" value=\"0\" />\n    <setting id=\"open_allow_provisioned_firmware\" value=\"1\" />\n    <setting id=\"open_https_enabled\" value=\"0\" />\n    <setting id=\"open_dev_mode\" value=\"1\" />\n</config>\n",
    )
    .unwrap();
    nv.open_put("ssh_host_ed25519", b"key").unwrap();
    let s = reopen(&dir, &nv);
    let d = s.dphone_settings().clone();
    assert_eq!(d.ssh_enabled, Some(false));
    // The old "allow provisioned firmware" value is dropped, not carried:
    // provisioned firmware is now followed unless explicitly ignored.
    assert_eq!(d.ignore_provisioned_firmware, None);
    assert!(!s.config().access().ignore_provisioned_firmware);
    assert!(!nv.exists("open/config.xml"));
    // The key/value area is untouched.
    assert_eq!(nv.open_get("ssh_host_ed25519").unwrap().as_deref(), Some(&b"key"[..]));
    let text = nv.read_string(FILE).unwrap().unwrap();
    assert!(!text.contains("dev") && !text.contains("firmware") && !text.contains("https"));
    // Nothing of it reaches the merged configuration.
    assert_eq!(s.config().setting("open_ssh_enabled"), None);
    assert_eq!(s.config().setting("open_dev_mode"), None);

    // A value already in the new file wins over the old layer.
    nv.write("open/config.xml", b"<config><setting id=\"open_ssh_enabled\" value=\"1\"/></config>").unwrap();
    let s = reopen(&dir, &nv);
    assert_eq!(s.dphone_settings().ssh_enabled, Some(false));
    assert!(!nv.exists("open/config.xml"));

    // An old layer that does not parse is dropped without blocking startup.
    nv.write("open/config.xml", b"<config><setting").unwrap();
    let s = reopen(&dir, &nv);
    assert_eq!(s.dphone_settings().ssh_enabled, Some(false));
    assert!(!nv.exists("open/config.xml"));
}

#[test]
fn factory_reset_returns_the_settings_to_defaults() {
    let (dir, nv, mut s) = store();
    s.update_dphone_settings(|d| {
        d.ssh_enabled = Some(true);
        d.ignore_provisioned_firmware = Some(true);
    })
    .unwrap();
    nv.write("dphone_settings.txtpb.bad", b"x").unwrap();
    nv.request_factory_reset().unwrap();
    assert!(ConfigStore::run_pending_factory_reset(&nv, &[]).unwrap());
    assert!(!nv.exists(FILE));
    assert!(!nv.exists("dphone_settings.txtpb.bad"));
    let s = reopen(&dir, &nv);
    assert_eq!(*s.dphone_settings(), DphoneSettings::default());
    assert!(!s.config().access().ssh_enabled && !s.config().access().ignore_provisioned_firmware);
}

// ---- provisioning from DPMA ------------------------------------------------

const KEY_A: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa alice@laptop";
const KEY_B: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb bob@desk";
const KEY_LOCAL: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICcccccccccccccccccccccccccccccccccccccccccc local";

fn provisioned(extra: &str) -> String {
    format!("<config><setting id=\"ringer_volume\" value=\"3\"/>{extra}</config>")
}

#[test]
fn provisioned_settings_precedence_and_source() {
    use SettingSource::*;
    let (_d, nv, mut s) = store();
    // Provisioned, nothing local: the provisioned value applies.
    let c = s
        .apply_fetched(&provisioned(
            r#"<setting id="dphone_ssh_enabled" value="0"/><setting id="dphone_ignore_provisioned_firmware" value="1"/>"#,
        ))
        .unwrap();
    assert_eq!(c.ssh_enabled(), Resolved { value: false, source: Provisioned, locked: false });
    assert_eq!(c.ignore_provisioned_firmware(), Resolved { value: true, source: Provisioned, locked: false });
    // The retired HTTPS setting is not recognized: it is inert, like any
    // unknown setting.
    let c = s.apply_fetched(&provisioned(r#"<setting id="dphone_ssh_enabled" value="0"/><setting id="dphone_ignore_provisioned_firmware" value="1"/><setting id="dphone_https_enabled" value="0"/>"#)).unwrap();
    assert_eq!(c.ssh_enabled(), Resolved { value: false, source: Provisioned, locked: false });
    assert!(!c.access().ssh_enabled && c.access().ignore_provisioned_firmware);
    // Provisioning never writes the local file.
    assert!(!nv.exists(FILE));

    // A local value beats an ordinary provisioned one, as ui-config.xml
    // beats fetched-config.xml on stock.
    let c = s.update_dphone_settings(|d| d.ssh_enabled = Some(true)).unwrap();
    assert_eq!(c.ssh_enabled(), Resolved { value: true, source: Local, locked: false });
    // Unsetting it locally shows the provisioned value again.
    let c = s.update_dphone_settings(|d| d.ssh_enabled = None).unwrap();
    assert_eq!(c.ssh_enabled(), Resolved { value: false, source: Provisioned, locked: false });

    // conflict="freeze" from the server wins over the local value.
    s.update_dphone_settings(|d| d.ssh_enabled = Some(true)).unwrap();
    let c = s
        .apply_fetched(&provisioned(r#"<setting id="dphone_ssh_enabled" value="0" conflict="freeze"/>"#))
        .unwrap();
    assert_eq!(c.ssh_enabled(), Resolved { value: false, source: Provisioned, locked: true });
    // The local value is kept, and applies again once the server lets go.
    let c = s.apply_fetched(&provisioned("")).unwrap();
    assert_eq!(c.ssh_enabled(), Resolved { value: true, source: Local, locked: false });
    assert_eq!(c.ignore_provisioned_firmware(), Resolved { value: false, source: Default, locked: false });

    // `metrics_enabled` follows the same rules; its default is on.
    assert_eq!(c.metrics_enabled(), Resolved { value: true, source: Default, locked: false });
    let c = s.apply_fetched(&provisioned(r#"<setting id="dphone_metrics_enabled" value="0"/>"#)).unwrap();
    assert_eq!(c.metrics_enabled(), Resolved { value: false, source: Provisioned, locked: false });
    let c = s.update_dphone_settings(|d| d.metrics_enabled = Some(true)).unwrap();
    assert_eq!(c.metrics_enabled(), Resolved { value: true, source: Local, locked: false });
    assert!(nv.read_string(FILE).unwrap().unwrap().contains("metrics_enabled: true"));
    let c = s
        .apply_fetched(&provisioned(r#"<setting id="dphone_metrics_enabled" value="0" conflict="freeze"/>"#))
        .unwrap();
    assert_eq!(c.metrics_enabled(), Resolved { value: false, source: Provisioned, locked: true });
    let c = s.apply_fetched(&provisioned("")).unwrap();
    assert_eq!(c.metrics_enabled(), Resolved { value: true, source: Local, locked: false });
    let c = s.update_dphone_settings(|d| d.metrics_enabled = None).unwrap();
    assert_eq!(c.metrics_enabled(), Resolved { value: true, source: Default, locked: false });

    // The provisioned IDs cannot be written through the generic path into
    // ui-config.xml.
    assert!(s.set("dphone_metrics_enabled", "0").is_err());
    assert!(s.set("dphone_ssh_enabled", "1").is_err());
    assert!(!nv.exists("ui-config.xml"));
}

#[test]
fn provisioned_static_address_and_the_stock_network_settings() {
    use std::net::Ipv4Addr;
    use SettingSource::*;
    let (_d, _nv, mut s) = store();
    let address = |c: &dphone_config::Config| c.static_ipv4().map(|a| (a.address, a.prefix_len, a.source, a.locked));
    // Nothing set, and what every phone that went through stock's setup
    // has in ui-config.xml: DHCP.
    assert_eq!(s.config().static_ipv4(), None);
    let c = s.set_many(&[("network_enable_dhcp", "1"), ("network_static_ip_address", ""), ("network_subnet_mask", "")]).unwrap();
    assert_eq!(c.static_ipv4(), None);

    // The server's address applies over that, with its gateway and DNS
    // servers; what is not an address is left out.
    let c = s
        .apply_fetched(&provisioned(
            r#"<setting id="dphone_static_address" value="10.10.14.120/23"/><setting id="dphone_static_gateway" value="10.10.14.1"/><setting id="dphone_static_dns" value="10.10.14.1, 10.10.14.2,dns,10.10.14.1"/>"#,
        ))
        .unwrap();
    let a = c.static_ipv4().unwrap();
    assert_eq!((a.address, a.prefix_len, a.source, a.locked), (Ipv4Addr::new(10, 10, 14, 120), 23, Provisioned, false));
    assert_eq!(a.gateway, Some(Ipv4Addr::new(10, 10, 14, 1)));
    assert_eq!(a.dns, [Ipv4Addr::new(10, 10, 14, 1), Ipv4Addr::new(10, 10, 14, 2)]);
    // Stock's own view of the network settings is untouched.
    assert!(c.network().dhcp);

    // A static address set on the phone beats an ordinary provisioned one...
    let c = s
        .set_many(&[("network_enable_dhcp", "0"), ("network_static_ip_address", "192.168.7.9"), ("network_subnet_mask", "255.255.255.0"), ("network_default_gateway", "192.168.7.1")])
        .unwrap();
    let a = c.static_ipv4().unwrap();
    assert_eq!((a.address, a.prefix_len, a.gateway, a.source), (Ipv4Addr::new(192, 168, 7, 9), 24, Some(Ipv4Addr::new(192, 168, 7, 1)), Local));
    // ...but not a frozen one.
    let c = s.apply_fetched(&provisioned(r#"<setting id="dphone_static_address" value="10.10.14.120/23" conflict="freeze"/>"#)).unwrap();
    assert_eq!(address(&c), Some((Ipv4Addr::new(10, 10, 14, 120), 23, Provisioned, true)));
    // The server lets go: the phone's own address, then DHCP again.
    let c = s.apply_fetched(&provisioned("")).unwrap();
    assert_eq!(address(&c), Some((Ipv4Addr::new(192, 168, 7, 9), 24, Local, false)));
    let c = s.set("network_enable_dhcp", "1").unwrap();
    assert_eq!(c.static_ipv4(), None);

    // An address that is not one for a host is ignored whole.
    for bad in ["10.10.14.120", "10.10.14.120/32", "10.10.14.0/23", "10.10.15.255/23", "224.0.0.9/4", "0.0.0.1/8", "10.10.14.120/255.255.254.0", "ten/8", ""] {
        let c = s
            .apply_fetched(&provisioned(&format!(r#"<setting id="dphone_static_address" value="{bad}"/><setting id="dphone_static_gateway" value="10.10.14.1"/>"#)))
            .unwrap();
        assert_eq!(c.static_ipv4(), None, "{bad:?}");
    }
    // The IDs cannot be written into ui-config.xml.
    assert!(s.set("dphone_static_address", "10.0.0.2/8").is_err());
}

#[test]
fn provisioned_ssh_keys_combine_with_local_ones() {
    let (dir, nv, mut s) = store();
    let mirror = "open/provisioned_authorized_keys";
    nv.write_authorized_keys(&[KEY_LOCAL.to_owned()]).unwrap();
    assert!(!nv.exists(mirror));

    let keys = |list: &[&str]| {
        let items: String =
            list.iter().enumerate().map(|(i, k)| format!("<key id=\"{}\" value=\"{k}\"/>", i + 1)).collect();
        provisioned(&format!("<dphone_ssh_authorized_keys>{items}</dphone_ssh_authorized_keys>"))
    };
    // Two keys, a duplicate, and two entries that are not plain key lines.
    let c = s
        .apply_fetched(&keys(&[KEY_A, KEY_B, KEY_A, "command=&quot;sh&quot; ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA", "junk"]))
        .unwrap();
    assert_eq!(c.provisioned_ssh_keys(), [KEY_A, KEY_B]);
    assert_eq!(s.ssh_authorized_keys().unwrap(), [KEY_LOCAL, KEY_A, KEY_B]);
    assert_eq!(nv.read_string(mirror).unwrap().unwrap(), format!("{KEY_A}\n{KEY_B}\n"));
    // The local file is never touched by provisioning.
    assert_eq!(nv.authorized_keys().unwrap(), [KEY_LOCAL]);

    // The server drops one key: it is gone, with nothing stale left behind.
    let c = s.apply_fetched(&keys(&[KEY_B])).unwrap();
    assert_eq!(c.provisioned_ssh_keys(), [KEY_B]);
    assert_eq!(nv.read_string(mirror).unwrap().unwrap(), format!("{KEY_B}\n"));
    // Still right after a restart.
    let s2 = reopen(&dir, &nv);
    assert_eq!(s2.ssh_authorized_keys().unwrap(), [KEY_LOCAL, KEY_B]);

    // The server stops sending keys: only the local one remains.
    s.apply_fetched(&provisioned("")).unwrap();
    assert!(!nv.exists(mirror));
    assert_eq!(s.ssh_authorized_keys().unwrap(), [KEY_LOCAL]);

    // A stale mirror (for example, after fetched-config.xml was removed
    // while the firmware was not running) is cleaned up at the next start.
    nv.write(mirror, format!("{KEY_A}\n").as_bytes()).unwrap();
    let s3 = reopen(&dir, &nv);
    assert!(!nv.exists(mirror));
    assert_eq!(s3.ssh_authorized_keys().unwrap(), [KEY_LOCAL]);
}
