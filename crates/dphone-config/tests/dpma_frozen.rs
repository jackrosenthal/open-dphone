//! The Open Desk Phone settings as Open DPMA provisions them: every one is
//! sent with `conflict="freeze"`, so a value set on the phone does not go
//! before the server's.

mod common;

use std::net::Ipv4Addr;

use common::TempDir;
use dphone_config::{ConfigStore, IdleFunction, IdleSoftkey, Model, Nvdata, Resolved, SettingSource};

/// The settings as the module writes them into a phone's configuration
/// (`build_config` of `res_open_dpma`; the module's suite `config-xml`
/// checks the same lines).
const FROM_OPEN_DPMA: &str = concat!(
    "<config>\n",
    "<setting id=\"dphone_ssh_enabled\" value=\"0\" conflict=\"freeze\" />\n",
    "<setting id=\"dphone_ignore_provisioned_firmware\" value=\"1\" conflict=\"freeze\" />\n",
    "<setting id=\"dphone_metrics_enabled\" value=\"0\" conflict=\"freeze\" />\n",
    "<setting id=\"dphone_serial_console\" value=\"1\" conflict=\"freeze\" />\n",
    "<setting id=\"dphone_static_address\" value=\"192.0.2.20/23\" conflict=\"freeze\" />\n",
    "<setting id=\"dphone_static_gateway\" value=\"192.0.2.1\" conflict=\"freeze\" />\n",
    "<setting id=\"dphone_static_dns\" value=\"192.0.2.1,192.0.2.2,192.0.2.4\" conflict=\"freeze\" />\n",
    "<setting id=\"dphone_disabled_apps\" value=\"calendar,parking,queues\" conflict=\"freeze\" />\n",
    "<setting id=\"dphone_idle_softkeys\" value=\"call_log,app:parking,menu,dnd\" conflict=\"freeze\" />\n",
    "</config>\n",
);

fn store() -> (TempDir, Nvdata, ConfigStore) {
    let dir = TempDir::new("dpma-frozen");
    let nv = Nvdata::new(dir.path().join("nvdata"));
    nv.ensure_layout().unwrap();
    let store = ConfigStore::open(nv.clone(), Model::D60, dir.path().join("tmp")).unwrap();
    (dir, nv, store)
}

/// Every setting set on the phone to the opposite of what the server
/// says.
fn set_on_the_phone(s: &mut ConfigStore) {
    s.update_dphone_settings(|d| {
        d.ssh_enabled = Some(true);
        d.ignore_provisioned_firmware = Some(false);
        d.metrics_enabled = Some(true);
        d.serial_console = Some(false);
        d.set_disabled_apps(Some(&["status".to_owned()]));
        d.set_idle_softkeys(&[IdleSoftkey::Function(IdleFunction::Redial)]);
    })
    .unwrap();
    s.set_many(&[
        ("network_enable_dhcp", "0"),
        ("network_static_ip_address", "192.168.7.9"),
        ("network_subnet_mask", "255.255.255.0"),
        ("network_default_gateway", "192.168.7.1"),
    ])
    .unwrap();
}

#[test]
fn the_servers_values_hold_against_the_phones() {
    use SettingSource::*;
    let (_d, _nv, mut s) = store();
    set_on_the_phone(&mut s);
    let c = s.config();
    assert_eq!(c.ssh_enabled(), Resolved { value: true, source: Local, locked: false });
    assert_eq!(c.static_ipv4().unwrap().source, Local);

    fn frozen<T>(value: T) -> Resolved<T> {
        Resolved { value, source: SettingSource::Provisioned, locked: true }
    }
    let check = |c: &dphone_config::Config| {
        assert_eq!(c.ssh_enabled(), frozen(false));
        assert_eq!(c.ignore_provisioned_firmware(), frozen(true));
        assert_eq!(c.metrics_enabled(), frozen(false));
        assert_eq!(c.serial_console(), frozen(true));
        assert_eq!(c.disabled_apps(), frozen(vec!["calendar".to_owned(), "parking".to_owned(), "queues".to_owned()]));
        let keys = c.idle_softkeys();
        assert_eq!(
            keys.keys,
            [
                IdleSoftkey::Function(IdleFunction::CallLog),
                IdleSoftkey::App("parking".into()),
                IdleSoftkey::Function(IdleFunction::Menu),
                IdleSoftkey::Function(IdleFunction::Dnd),
            ]
        );
        assert_eq!((keys.source, keys.locked), (Provisioned, true));
        let a = c.static_ipv4().unwrap();
        assert_eq!((a.address, a.prefix_len, a.source, a.locked), (Ipv4Addr::new(192, 0, 2, 20), 23, Provisioned, true));
        assert_eq!(a.gateway, Some(Ipv4Addr::new(192, 0, 2, 1)));
        assert_eq!(a.dns, [Ipv4Addr::new(192, 0, 2, 1), Ipv4Addr::new(192, 0, 2, 2), Ipv4Addr::new(192, 0, 2, 4)]);
        assert!(!c.access().ssh_enabled && c.access().ignore_provisioned_firmware);
    };
    let c = s.apply_fetched(FROM_OPEN_DPMA).unwrap();
    check(&c);

    // A value set on the phone afterward changes nothing either.
    set_on_the_phone(&mut s);
    check(&s.config());

    // The same settings without the freeze, as the module sent them before:
    // the phone's values go first.
    let c = s.apply_fetched(&FROM_OPEN_DPMA.replace(" conflict=\"freeze\"", "")).unwrap();
    assert_eq!(c.ssh_enabled(), Resolved { value: true, source: Local, locked: false });
    assert_eq!(c.disabled_apps(), Resolved { value: vec!["status".to_owned()], source: Local, locked: false });
    assert_eq!(c.static_ipv4().unwrap().source, Local);

    // The server stops sending the settings: the phone's values apply.
    s.apply_fetched(FROM_OPEN_DPMA).unwrap();
    let c = s.apply_fetched("<config/>").unwrap();
    assert_eq!(c.ssh_enabled(), Resolved { value: true, source: Local, locked: false });
    assert_eq!(c.idle_softkeys().keys, [IdleSoftkey::Function(IdleFunction::Redial)]);
}
