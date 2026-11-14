//! Checks against the stock firmware's own `configmerge`, run under
//! `qemu-arm`.  Skipped unless `DPHONE_STOCK_ROOTFS` names an extracted
//! stock root filesystem (containing `app/bin/configmerge`); `qemu-arm` must
//! be on `PATH`.  With `DPHONE_TEST_REQUIRE_VENDOR` set (the Nix check sets
//! it, with both: nix/vendor-firmware.nix) the tests fail instead of
//! skipping.
//!
//! Two things are checked:
//!
//! 1. Our merge produces byte-identical output to the stock binary on the
//!    same inputs (edge cases beyond the 22 golden fixtures, and our own
//!    compiled-in base and factory layers).
//! 2. An nvdata tree written by this crate is accepted by the stock merge
//!    together with stock's own base and factory files: it exits cleanly
//!    and the result carries what we saved, which is what flashing back to
//!    stock relies on.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{FETCHED, TempDir};
use dphone_config::merge::{NetContext, merge_layers};
use dphone_config::model::BASE_CONFIG;
use dphone_config::{ConfigStore, Element, Model, Nvdata};

fn rootfs() -> Option<PathBuf> {
    let Some(p) = std::env::var_os("DPHONE_STOCK_ROOTFS").map(PathBuf::from) else {
        assert!(
            std::env::var_os("DPHONE_TEST_REQUIRE_VENDOR").is_none(),
            "DPHONE_TEST_REQUIRE_VENDOR is set, but DPHONE_STOCK_ROOTFS is not"
        );
        eprintln!("SKIPPED: set DPHONE_STOCK_ROOTFS to an extracted stock root filesystem to run this test");
        return None;
    };
    assert!(p.join("app/bin/configmerge").exists(), "no app/bin/configmerge under {}", p.display());
    Some(p)
}

/// Run the stock binary; returns (exit code, stdout).
fn stock_merge(rootfs: &Path, files: &[PathBuf]) -> (i32, String) {
    let out = Command::new("qemu-arm")
        .arg("-L")
        .arg(rootfs)
        .arg(rootfs.join("app/bin/configmerge"))
        .args(files)
        .output()
        .expect("qemu-arm runs");
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned())
}

fn ours(files: &[PathBuf]) -> String {
    let layers = files.iter().map(|f| Element::parse(&std::fs::read_to_string(f).unwrap()).unwrap());
    merge_layers(layers, &NetContext::default()).to_document()
}

fn write_inputs(dir: &Path, inputs: &[&str]) -> Vec<PathBuf> {
    inputs
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let p = dir.join(format!("{i}.xml"));
            std::fs::write(&p, text).unwrap();
            p
        })
        .collect()
}

const CASES: &[&[&str]] = &[
    // Identity attributes other than "id".
    &[r#"<config><firmware model="a" v="1"/><firmware model="b" v="2"/></config>"#, r#"<config><firmware model="a" v="3"/></config>"#],
    &[r#"<config><foo model="a" v="1"/><foo model="b" v="2"/></config>"#, r#"<config><foo model="a" v="3"/></config>"#],
    &[r#"<config><r><alert alert_info="a" x="1"/><alert alert_info="b" x="2"/></r></config>"#, r#"<config><r><alert alert_info="b" x="3"/></r></config>"#],
    &[r#"<config><contacts id="0" url="a"/><contacts id="1" url="b"/></config>"#, r#"<config><contacts id="1" url="c"/></config>"#],
    &[r#"<config><p><public_firmware url="u" v="1"/><send_dnd status="a" x="1"/><send_dnd status="b"/></p></config>"#, r#"<config><p><public_firmware url="u" v="2"/><send_dnd status="a" x="2"/></p></config>"#],
    // Text content.
    &[r#"<config><setting id="a">text1</setting></config>"#, r#"<config><setting id="a">text2</setting></config>"#],
    &[r#"<config><setting id="a">text1</setting></config>"#, r#"<config><setting id="a"/></config>"#],
    &[r#"<config><k id="1"><arg>one</arg></k></config>"#, r#"<config><k id="1"><arg>two</arg></k></config>"#],
    // Escaping, comments, declaration.
    &["<?xml version=\"1.0\"?><!-- c --><config><!-- inner --><setting id=\"a\" value=\"x&amp;y&lt;&gt;&quot;&apos;\u{e9}\"/></config>"],
    // login_password and account credential rules.
    &[r#"<config><setting id="login_password" value="secret"/></config>"#, r#"<config><setting id="login_password" value=""/></config>"#],
    &[r#"<config><x><account index="1" password="p" passcode="c" x="1"/></x></config>"#, r#"<config><x><account index="1" password="" passcode="" x=""/></x></config>"#],
    &[r#"<config><accounts><account index="1" password="p"/></accounts></config>"#, r#"<config><accounts conflict="x"><account index="1" password=""/></accounts></config>"#],
    // Replace and freeze in both positions.
    &[r#"<config><a id="1" conflict="replace" x="1"><b/></a></config>"#, r#"<config><a id="1" y="2"/></config>"#],
    &[r#"<config><a id="1" x="1"><b/></a></config>"#, r#"<config><a id="1" y="2" conflict="replace"/></config>"#],
    &[r#"<config><a id="1" x="1"><b/></a></config>"#, r#"<config><a id="1" y="2" conflict="freeze"/></config>"#, r#"<config><a id="1" z="3"/></config>"#],
    // Default-replace collections and self-merge exemptions.
    &[r#"<config><blf_items><blf_item id="1"/><blf_item id="1"/></blf_items><z><i id="1" a="1"/><i id="1" b="1"/></z></config>"#],
    &[r#"<config><use_proxy_for><h a="1"/></use_proxy_for></config>"#, r#"<config><use_proxy_for><h b="1"/><h c="1"/></use_proxy_for></config>"#],
    &[r#"<config><smart_blf><blf_items url="a"/></smart_blf></config>"#, r#"<config><smart_blf><blf_items url="b"/><x/></smart_blf></config>"#],
    // Duplicate siblings where the later one is frozen or replacing.
    &[r#"<config><s id="1" a="1"/><s id="1" b="2" conflict="freeze"/><s id="1" c="3"/></config>"#],
    &[r#"<config><k id="line1" length="short" action="a" conflict="freeze"/><k id="line1" length="long" action="b" conflict="freeze"/></config>"#],
    // Root names and attributes.
    &[r#"<other a="1"><setting id="a"/></other>"#, r#"<config2 b="2"><setting id="b"/></config2>"#],
    // Renamed setting.
    &[r#"<config><setting id="rtp_qos" value="6"/></config>"#, r#"<config><setting id="network_vlan_qos" value="4"/></config>"#],
    // Explicit network selection.
    &[r#"<config><netselect id="n1"/></config>"#, r#"<config><s id="a" network_id="n1" v="1"/><s id="c" network_id="n2" v="1"/><t id="d" v="1"/></config>"#],
    &[r#"<config><netselect id="n2"/></config>"#, r#"<config><accounts><account index="0" network_id="n1" u="1"/><account index="0" network_id="n2" u="2"/></accounts></config>"#],
    // Network-scoped elements with nothing selected are dropped.
    &[r#"<config><s id="a" network_id="n1" v="1"/><t id="b"/></config>"#],
];

#[test]
fn differential_edge_cases() {
    let Some(rootfs) = rootfs() else { return };
    let dir = TempDir::new("diff");
    for (n, inputs) in CASES.iter().enumerate() {
        let files = write_inputs(dir.path(), inputs);
        let (code, stock) = stock_merge(&rootfs, &files);
        assert_eq!(code, 0, "case {n}");
        assert_eq!(ours(&files), stock, "case {n}: {inputs:?}");
    }
}

#[test]
fn our_base_and_factory_layers_merge_identically_on_stock() {
    let Some(rootfs) = rootfs() else { return };
    let dir = TempDir::new("base");
    for model in Model::ALL {
        let files = write_inputs(dir.path(), &[BASE_CONFIG, model.factory_config()]);
        let (code, stock) = stock_merge(&rootfs, &files);
        assert_eq!(code, 0, "{model}");
        assert_eq!(ours(&files), stock, "{model}");
    }
}

#[test]
fn stock_accepts_trees_we_wrote() {
    let Some(rootfs) = rootfs() else { return };
    for model in Model::ALL {
        let dir = TempDir::new("tree");
        let nv = Nvdata::new(dir.path().join("nvdata"));
        nv.ensure_layout().unwrap();
        // What stock's boot leaves there.
        nv.write("g729-legacy-support.xml", b"<config>\n    <setting id=\"g729_authorized\" value=\"0\"/>\n</config>\n").unwrap();
        let mut s = ConfigStore::open(nv.clone(), model, dir.path().join("tmp")).unwrap();
        s.apply_fetched(FETCHED).unwrap();
        s.select_network(Some("office")).unwrap();
        s.write_layer(
            dphone_config::Layer::Configurator,
            &Element::parse(r#"<config><setting id="config_server_url" value="sip:proxy@10.10.0.2"/></config>"#).unwrap(),
        )
        .unwrap();
        s.set_many(&[("ringer_volume", "8"), ("network_enable_dhcp", "0")]).unwrap();
        // Our own settings file sits at the top of nvdata, where stock never
        // looks: stock's merge takes its ten fixed paths and nothing else.
        s.update_dphone_settings(|d| d.ssh_enabled = Some(false)).unwrap();
        assert!(nv.exists("dphone_settings.txtpb"));
        s.set_web_password("pa&ss\"<wd>'").unwrap();
        let ours_cfg = s.config();

        // Stock merges its own base and factory files with the nvdata layers
        // in its fixed order.
        let mut files = vec![rootfs.join("etc/base-config.xml"), rootfs.join("etc/factory-config.xml")];
        for f in ["g729-legacy-support.xml", "g729-config.xml", "netselect.xml", "configurator.xml", "fetched-config.xml", "ui-config.xml", "hack.xml"] {
            if nv.exists(f) {
                files.push(nv.path(f));
            }
        }
        let (code, out) = stock_merge(&rootfs, &files);
        assert_eq!(code, 0, "{model}");
        let stock = Element::parse(&out).expect("stock output parses");
        let get = |id: &str| stock.child_with_id("setting", id).and_then(|e| e.attr("value")).map(str::to_owned);
        assert_eq!(get("ringer_volume").as_deref(), Some("8"));
        assert_eq!(get("network_enable_dhcp").as_deref(), Some("0"));
        assert_eq!(get("login_password").as_deref(), Some("pa&ss\"<wd>'"));
        assert_eq!(get("time_zone").as_deref(), Some("America/Denver"));
        assert_eq!(get("config_server_url").as_deref(), Some("sip:proxy@10.10.0.2"));
        assert!(!out.contains("ssh_enabled"));
        // What we leave in config.xml is the stock layers' merge and no more.
        assert!(!nv.read_string("config.xml").unwrap().unwrap().contains("ssh_enabled"));
        // The same accounts both firmwares see for the selected network
        // (stock applies its own account limit at run time, not in the merge).
        let stock_accounts: Vec<_> =
            stock.child("accounts").unwrap().children_named("account").filter_map(|a| a.attr("username")).collect();
        assert_eq!(stock_accounts, ["101", "102", "103"]);
        assert_eq!(ours_cfg.accounts().len(), model.max_accounts().min(3));

        // And the same layer files merged by us give the same bytes.
        assert_eq!(ours(&files), out, "{model}");
    }
}

/// The DPMA extension must leave a stock phone unaffected: the stock merge
/// accepts a provisioned configuration carrying the `dphone_*` settings and
/// the key element, and apart from carrying those unknown entries along
/// inertly its result is identical to the one without them.
#[test]
fn stock_is_unaffected_by_the_dpma_extension() {
    let Some(rootfs) = rootfs() else { return };
    let dir = TempDir::new("ext");
    let base = [rootfs.join("etc/base-config.xml"), rootfs.join("etc/factory-config.xml")];
    let run = |fetched: &str, ui: &str| {
        let extra = write_inputs(
            dir.path(),
            &[r#"<config><netselect id="office"/></config>"#, fetched, ui],
        );
        let files: Vec<PathBuf> = base.iter().cloned().chain(extra).collect();
        let (code, out) = stock_merge(&rootfs, &files);
        assert_eq!(code, 0);
        // Our merge agrees byte for byte on the same inputs.
        assert_eq!(ours(&files), out);
        out
    };
    let ui = r#"<config><setting id="ringer_volume" value="8"/></config>"#;
    let without = run(FETCHED, ui);
    let with = run(&common::fetched_with_extension(), ui);
    assert_ne!(with, without);

    // Remove what the extension added and nothing else differs.
    let mut stripped = Element::parse(&with).unwrap();
    stripped.remove_children(|e| {
        e.name == "dphone_ssh_authorized_keys"
            || (e.name == "setting" && e.attr("id").is_some_and(|id| id.starts_with("dphone_")))
    });
    assert_eq!(stripped.to_document(), without);

    // No stock setting ID starts with the prefix, so nothing stock looks up
    // by ID can collide with the extension.
    for f in &base {
        assert!(!std::fs::read_to_string(f).unwrap().contains("dphone_"));
    }
}
