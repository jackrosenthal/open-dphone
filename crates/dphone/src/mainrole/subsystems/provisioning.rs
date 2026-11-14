//! What the DPMA client needs from the rest of the main role beyond the
//! configuration sink in `telephony`: the files a provisioned configuration
//! names, the "ignore provisioned firmware" setting as a fetched
//! configuration would leave it, and the discovery inputs (DHCP option 66
//! and mDNS), kept current.

use std::sync::Arc;
use std::time::Duration;

use dphone_config::{Config, DphoneSettings, Element, Nvdata};
use dphone_dpma::DpmaHandle;
use dphone_dpma::resources::{Files, Kind, PushedApp, Resource, Wanted};
use dphone_state::{DiscoveredServer, DiscoverySource, ProvisionPhase, Provisioning, Topic};
use log::info;

use super::net::NetHandle;
use crate::mainrole::Context;

/// The files the merged configuration names, as stock reads them:
/// `<contacts id url md5>`, `<ringtones><tones><tone id url md5>`,
/// `<smart_blf><blf_items url md5>`, `<smart_blf><datasets url md5>`, and,
/// for the log only, the app packages of `<appconfig id><url url/>`.
pub fn wanted_resources(cfg: &Config) -> Wanted {
    let mut wanted = Wanted::default();
    for c in cfg.contact_sources() {
        wanted.resources.push(Resource { kind: Kind::ContactList, id: c.id, url: c.url, md5: c.md5 });
    }
    for t in cfg.ringtones().tones {
        if let Some(url) = t.url {
            wanted.resources.push(Resource { kind: Kind::Ringtone, id: t.id, url, md5: t.md5 });
        }
    }
    let root = cfg.root();
    if let Some(smart) = root.child("smart_blf") {
        for (name, kind) in [("blf_items", Kind::BlfItems), ("datasets", Kind::Datasets)] {
            for e in smart.children_named(name) {
                if let Some(url) = e.attr("url").filter(|u| !u.is_empty()) {
                    wanted.resources.push(Resource {
                        kind,
                        id: String::new(),
                        url: url.to_string(),
                        md5: e.attr("md5").map(str::to_string),
                    });
                }
            }
        }
    }
    for app in root.children_named("appconfig") {
        let url = app.child("url").and_then(|u| u.attr("url")).filter(|u| !u.is_empty());
        if let (Some(id), Some(url)) = (app.attr("id").filter(|i| !i.is_empty()), url) {
            wanted.apps.push(PushedApp { id: id.to_string(), url: url.to_string() });
        }
    }
    wanted
}

/// Whether provisioned firmware is ignored once `fetched` (a fetched
/// configuration) is in effect, given the setting stored on the phone.
/// `None` when the document does not parse: the value in effect stays.
pub fn ignores_firmware(fetched: &[u8], model: dphone_config::Model, local: &DphoneSettings) -> Option<bool> {
    let root = Element::parse(std::str::from_utf8(fetched).ok()?).ok()?;
    Some(Config::from_merged(root, model, None).with_dphone_settings(local.clone()).ignore_provisioned_firmware().value)
}

/// nvdata, for the resource pass.
pub struct NvFiles(pub Nvdata);

impl Files for NvFiles {
    fn read(&self, rel: &str) -> Option<Vec<u8>> {
        self.0.read(rel).ok().flatten()
    }
    fn write(&self, rel: &str, data: &[u8]) -> Result<(), String> {
        if let Some(dir) = self.0.path(rel).parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        self.0.write(rel, data).map_err(|e| e.to_string())
    }
    fn remove(&self, rel: &str) {
        let _ = self.0.remove(rel);
    }
    fn list(&self, dir: &str) -> Vec<String> {
        self.0.list(dir).unwrap_or_default()
    }
}

/// The download client: names are resolved by the network manager's
/// resolver (the one DHCP feeds), or by the host system's when the manager
/// has none (the simulator).
fn downloader(net: &NetHandle) -> dphone_dpma::fetch::HttpSource {
    use std::net::{SocketAddr, ToSocketAddrs};
    let (runtime, net) = (tokio::runtime::Handle::current(), net.clone());
    dphone_dpma::fetch::HttpSource::with_lookup(Box::new(move |host, port| {
        let Some(resolver) = net.resolver() else {
            return (host, port).to_socket_addrs()?.next().ok_or_else(|| std::io::Error::other("no address"));
        };
        let addrs = runtime.block_on(resolver.lookup_ip(host, false, false)).map_err(|e| std::io::Error::other(format!("{e:?}")))?;
        addrs.first().map(|ip| SocketAddr::new(*ip, port)).ok_or_else(|| std::io::Error::other("no address"))
    }))
}

/// The HTTP client for firmware images.  Like stock's (`curl --insecure`),
/// it does not check an `https://` server's certificate.  Call it on the
/// runtime; the downloads themselves run on blocking threads.
pub fn http_source(net: &NetHandle) -> Arc<dyn dphone_dpma::firmware::FirmwareSource> {
    Arc::new(downloader(net))
}

/// The HTTP client for the files a configuration names by URL (contact
/// lists, ringtones).  As on stock, an `https://` server is checked against
/// the trust bundle ([`super::trust::roots`]) unless `allow_insecure_ssl`
/// is set: the policy of SIP over TLS and of the apps' requests.  Call it
/// on the runtime.
pub fn resource_source(net: &NetHandle, config: super::SharedConfig) -> Arc<dyn dphone_dpma::firmware::FirmwareSource> {
    use dphone_dpma::fetch::Trust;
    Arc::new(downloader(net).with_trust(Box::new(move || {
        let store = config.lock().unwrap();
        let cfg = store.config();
        if cfg.sip().allow_insecure_ssl { Trust::Any } else { Trust::Roots(super::trust::roots(store.nvdata(), &cfg)) }
    })))
}

/// A `_digiumproxy._udp` announcement as a provisioning server: the TXT
/// record's `sipUrl` when it has one, else the address and port.
fn announced(s: &dphone_net::mdns::Service) -> Option<DiscoveredServer> {
    let sip_url = s.txt.iter().find(|(k, v)| k.eq_ignore_ascii_case("sipUrl") && v.starts_with("sip")).map(|(_, v)| v.clone());
    let address = match sip_url {
        Some(url) => url,
        None => {
            let host = s.addrs.first().map(|a| a.to_string()).or_else(|| (!s.host.is_empty()).then(|| s.host.clone()))?;
            format!("{host}:{}", s.port)
        }
    };
    Some(DiscoveredServer { source: DiscoverySource::Mdns, name: s.instance.clone(), address })
}

/// How long one mDNS browse listens.
const BROWSE_WINDOW: Duration = Duration::from_secs(3);
/// How often the phone browses again while it has no server.  (Stock keeps
/// a browser running and hears announcements as they come.)
const BROWSE_EVERY: Duration = Duration::from_secs(30);

/// Keeps the DPMA client's discovery inputs current: whether the network
/// is up, DHCP option 66 (whenever it changes, not only with the address),
/// and the servers announced over mDNS.  The network is browsed when it
/// comes up and, while the phone waits for a server, every
/// [`BROWSE_EVERY`]; the last result is passed along with every report.
pub async fn feed_discovery(ctx: Context, net: NetHandle, dpma: DpmaHandle) {
    let sub = ctx.store.subscribe([Topic::Network, Topic::Provisioning]);
    let mut mdns: Vec<DiscoveredServer> = Vec::new();
    let mut told: Option<(bool, Option<String>, Vec<DiscoveredServer>)> = None;
    let mut was_up = false;
    let mut browse_at = tokio::time::Instant::now();
    loop {
        let (up, option66) = if ctx.sim {
            (true, None)
        } else {
            ctx.store.read::<dphone_state::Network, _>(|n| (n.ipv4.address.is_some(), Some(n.dhcp_tftp_server.clone()).filter(|s| !s.is_empty())))
        };
        let waiting = ctx.store.read::<Provisioning, _>(|p| p.phase == ProvisionPhase::AwaitingServer);
        let now = tokio::time::Instant::now();
        let browse = up && !ctx.sim && (!was_up || (waiting && now >= browse_at));
        was_up = up;
        let report = (up, option66, mdns.clone());
        if told.as_ref() != Some(&report) {
            dpma.network(report.0, report.1.clone(), report.2.clone());
            told = Some(report);
        }
        if browse {
            let mut found: Vec<DiscoveredServer> = net.browse_proxies(BROWSE_WINDOW).await.iter().filter_map(announced).collect();
            found.sort_by(|a, b| (&a.name, &a.address).cmp(&(&b.name, &b.address)));
            found.dedup();
            if found != mdns {
                info!("dpma: {} server(s) announced over mDNS", found.len());
                mdns = found;
            }
            browse_at = tokio::time::Instant::now() + BROWSE_EVERY;
            // The store may have moved on during the browse.
            continue;
        }
        tokio::select! {
            _ = sub.changed() => {}
            _ = tokio::time::sleep_until(browse_at), if waiting && up && !ctx.sim => {}
        }
    }
}

/// Starts [`feed_discovery`].
pub fn start_discovery(ctx: &Context, net: &NetHandle, dpma: &DpmaHandle) {
    tokio::spawn(feed_discovery(ctx.clone(), net.clone(), dpma.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(xml: &str) -> Config {
        Config::from_merged(Element::parse(xml).unwrap(), dphone_config::Model::D60, None)
    }

    #[test]
    fn resources_of_a_provisioned_configuration() {
        // The shapes `res_dpma_rs` builds.
        let cfg = config(
            r#"<config>
<appconfig id="contacts"><account id="100"/></appconfig>
<appconfig id="queues" network_id="n"><url url="file://queues.zip" md5="aa" /><auto_start/></appconfig>
<contacts url="file://team.xml" id="0" />
<contacts id="1" url="http://pbx/files/all.xml" md5="0123456789abcdef0123456789abcdef" network_id="n" />
<ringtones>
<tones>
<tone id="Digium" display="Digium" type="phone" />
<tone id="Chimes" display="Chimes" url="file://chimes.sln" type="user" network_id="n" />
</tones>
</ringtones>
<smart_blf>
<blf_items url="file://blf.xml" network_id="n"/>
<datasets url="http://pbx/files/data.xml" md5="ff"/>
</smart_blf>
</config>"#,
        );
        let w = wanted_resources(&cfg);
        let r = |kind, id: &str, url: &str, md5: Option<&str>| Resource { kind, id: id.into(), url: url.into(), md5: md5.map(str::to_string) };
        assert_eq!(
            w.resources,
            vec![
                r(Kind::ContactList, "0", "file://team.xml", None),
                r(Kind::ContactList, "1", "http://pbx/files/all.xml", Some("0123456789abcdef0123456789abcdef")),
                r(Kind::Ringtone, "Chimes", "file://chimes.sln", None),
                r(Kind::BlfItems, "", "file://blf.xml", None),
                r(Kind::Datasets, "", "http://pbx/files/data.xml", Some("ff")),
            ]
        );
        // The built-in app's configuration is not a package; the pushed
        // one is, and is only ever logged.
        assert_eq!(w.apps, vec![PushedApp { id: "queues".into(), url: "file://queues.zip".into() }]);
        assert_eq!(wanted_resources(&config("<config/>")), Wanted::default());
    }

    #[test]
    fn ignore_setting_of_a_fetched_configuration() {
        let model = dphone_config::Model::D60;
        let unset = DphoneSettings::default();
        let on = br#"<config><setting id="dphone_ignore_provisioned_firmware" value="1" /><firmwares><firmware model="D60" version="2_9_28" url="http://pbx/f.eff"/></firmwares></config>"#;
        let off = br#"<config><setting id="dphone_ignore_provisioned_firmware" value="0" /></config>"#;
        let silent = br#"<config><setting id="login_password" value="789" /></config>"#;
        assert_eq!(ignores_firmware(on, model, &unset), Some(true));
        assert_eq!(ignores_firmware(off, model, &unset), Some(false));
        assert_eq!(ignores_firmware(silent, model, &unset), Some(false), "off by default");
        // The value stored on the phone outranks an ordinary provisioned
        // one, as for every Open DPhone setting.
        let local_on = DphoneSettings { ignore_provisioned_firmware: Some(true), ..Default::default() };
        let local_off = DphoneSettings { ignore_provisioned_firmware: Some(false), ..Default::default() };
        assert_eq!(ignores_firmware(silent, model, &local_on), Some(true));
        assert_eq!(ignores_firmware(on, model, &local_off), Some(false));
        assert_eq!(ignores_firmware(b"<config", model, &unset), None);
    }

    #[test]
    fn nvdata_as_resource_files() {
        let dir = std::env::temp_dir().join(format!("dphone-provisioning-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let nv = Nvdata::new(&dir);
        let f = NvFiles(nv.clone());
        // The directories need not exist yet.
        f.write("contact_lists/0", b"<contacts/>").unwrap();
        f.write("md5s/0", b"0123456789abcdef0123456789abcdef").unwrap();
        f.write("blf_items.xml", b"<config/>").unwrap();
        assert_eq!(f.read("contact_lists/0").as_deref(), Some(&b"<contacts/>"[..]));
        assert_eq!(f.list("contact_lists"), vec!["0"]);
        // The same files through the names the rest of the firmware uses.
        assert_eq!(nv.contact_list("0").unwrap().as_deref(), Some(&b"<contacts/>"[..]));
        assert_eq!(nv.cached_md5("0").unwrap().as_deref(), Some("0123456789abcdef0123456789abcdef"));
        assert!(nv.blf_items().unwrap().is_some());
        f.remove("contact_lists/0");
        assert_eq!(f.read("contact_lists/0"), None);
        assert!(f.list("ringtones").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn announcements() {
        let mut s = dphone_net::mdns::Service {
            instance: "Digium Phone Config Server".into(),
            fullname: "Digium Phone Config Server._digiumproxy._udp.local".into(),
            host: "pbx.local".into(),
            port: 5060,
            addrs: vec!["10.0.0.5".parse().unwrap()],
            txt: [("serviceType".to_string(), "asterisk".to_string())].into(),
        };
        assert_eq!(announced(&s).unwrap().address, "10.0.0.5:5060");
        s.txt.insert("sipUrl".into(), "sip:proxy@10.0.0.5:5062;transport=tcp".into());
        let d = announced(&s).unwrap();
        assert_eq!((d.source, d.address.as_str()), (DiscoverySource::Mdns, "sip:proxy@10.0.0.5:5062;transport=tcp"));
        s.txt.clear();
        s.addrs.clear();
        assert_eq!(announced(&s).unwrap().address, "pbx.local:5060");
        s.host.clear();
        assert!(announced(&s).is_none());
    }
}
