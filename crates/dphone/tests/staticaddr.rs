//! A static address from the provisioning server, from the first boot by
//! DHCP to the address's removal: the main role's network subsystem over
//! the real `NetManager`, with the real DPMA client and the discovery feed
//! that tells it about the network, against the in-process DPMA server of
//! `dphone-dpma`'s tests and a DHCP server, in a private network namespace.
//!
//! The phone runs in a child process (this test binary again, with
//! `--phone`), so that it can be switched off and booted again with the
//! nvdata it left.  The parent is the network: the switch port "sw" that
//! the LAN jack "lan" is plugged into, the DHCP server, and the DPMA
//! server.  Around the network subsystem the child wires SIP and DPMA the
//! way `telephony` does (the SIP service follows the phone's address, the
//! DPMA client saves a fetched configuration into the configuration store
//! and announces it), without accounts, audio, or a UI.
//!
//! Needs unprivileged user and network namespaces; where those are missing
//! it prints a notice and passes without running.  It is a `harness = false`
//! test because `unshare(CLONE_NEWUSER)` needs a single-threaded process.
//!
//! What this cannot show: the phone and the servers share one namespace, so
//! the kernel delivers their packets to each other without the wire, from
//! whichever address.  An address that does not work on the network is
//! therefore played by the server, which does not answer a phone that says
//! it is there (the SIP Contact of its requests).  That the address a phone
//! holds is the one it is reached at is for a real network to show.

#[path = "../../dphone-dpma/tests/common/mod.rs"]
mod common;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::FakeServer;
use dphone::mainrole::subsystems::system::ConfigHub;
use dphone::mainrole::subsystems::{net, provisioning};
use dphone::mainrole::Context;
use dphone::model::Model;
use dphone::paths::Paths;
use dphone_config::{ConfigStore, Nvdata};
use dphone_dpma::Message;
use dphone_net::dhcp4::{self, MsgType};
use dphone_net::netlink::Netlink;
use dphone_sip::{NullMedia, OutMessage, SipConfig};
use dphone_state::Topic;
use tokio::net::UdpSocket;
use tokio::time::{sleep, Instant};

const SERVER_IP: Ipv4Addr = Ipv4Addr::new(10, 99, 0, 1);
const LEASED_IP: Ipv4Addr = Ipv4Addr::new(10, 99, 0, 50);
/// The static address the server gives the phone.
const STATIC_IP: Ipv4Addr = Ipv4Addr::new(10, 99, 0, 60);
/// A static address at which the server never hears the phone.
const DEAD_IP: Ipv4Addr = Ipv4Addr::new(10, 99, 0, 77);
const SERVER_SIP_PORT: u16 = 5070;
const PHONE_SIP_PORT: u16 = 5062;
/// The fallback time (150 s on a phone).  Longer than a bring-up and the
/// DPMA handshake after it take here, a few seconds.
const FALLBACK: Duration = Duration::from_secs(20);

fn enter_namespace() -> bool {
    // SAFETY: plain system calls with no pointers.
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    // The UTS namespace is for the hostname the subsystem sets.
    // SAFETY: the process is single-threaded here.
    if unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET | libc::CLONE_NEWUTS) } != 0 {
        return false;
    }
    std::fs::write("/proc/self/setgroups", "deny").is_ok()
        && std::fs::write("/proc/self/uid_map", format!("0 {uid} 1")).is_ok()
        && std::fs::write("/proc/self/gid_map", format!("0 {gid} 1")).is_ok()
}

struct StderrLog;
impl log::Log for StderrLog {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, r: &log::Record) {
        eprintln!("[{}] {}", r.level(), r.args());
    }
    fn flush(&self) {}
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let [_, flag, nvdata] = args.as_slice()
        && flag == "--phone"
    {
        // The child: already inside the parent's namespaces.
        log::set_logger(&StderrLog).unwrap();
        log::set_max_level(log::LevelFilter::Info);
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(phone(PathBuf::from(nvdata)));
        return;
    }
    if !enter_namespace() {
        println!("staticaddr: SKIPPED (unprivileged user and network namespaces are not available)");
        return;
    }
    // The phone and its servers share this namespace; let the kernel
    // accept packets from an address that is also local.
    for (key, value) in [("all/accept_local", "1"), ("all/rp_filter", "0"), ("default/rp_filter", "0")] {
        std::fs::write(format!("/proc/sys/net/ipv4/conf/{key}"), value).unwrap();
    }
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let mut nl = Netlink::new().unwrap();
        let can_bridge = nl
            .add_bridge("probe0", &dphone_net::netlink::BridgeOpts { vlan_filtering: true, group_fwd_mask: 0, default_pvid: 1 })
            .await
            .is_ok();
        if !can_bridge {
            println!("staticaddr: SKIPPED (the host kernel cannot create a VLAN-filtering bridge)");
            return;
        }
        let probe = idx(&mut nl, "probe0").await;
        nl.del_link(probe).await.unwrap();
        if nl.add_veth("lan", "sw").await.is_err() {
            println!("staticaddr: SKIPPED (the host kernel has no veth)");
            return;
        }
        network(nl).await;
        println!("staticaddr: ok");
    });
}

async fn idx(nl: &mut Netlink, name: &str) -> u32 {
    nl.link_by_name(name).await.unwrap().unwrap_or_else(|| panic!("no interface {name}")).index
}

// ------------------------------------------------------------------------
// The phone (the child process).

/// The DPMA client's configuration sink over the configuration store, as
/// `telephony` has it.
struct Sink {
    config: Arc<Mutex<ConfigStore>>,
    hub: ConfigHub,
    store: dphone_state::Store,
}

impl Sink {
    fn nv(&self) -> Nvdata {
        self.config.lock().unwrap().nvdata().clone()
    }
}

impl dphone_dpma::ConfigSink for Sink {
    fn configurator(&self) -> Option<Vec<u8>> {
        self.nv().read("configurator.xml").ok().flatten()
    }
    fn write_configurator(&self, xml: &[u8]) -> Result<(), String> {
        self.nv().write("configurator.xml", xml).map_err(|e| e.to_string())?;
        let cfg = self.config.lock().unwrap().reload().map_err(|e| e.to_string())?;
        self.hub.changed(&self.store, &cfg);
        Ok(())
    }
    fn cached_md5(&self) -> Option<String> {
        self.nv().cached_md5("config").ok().flatten()
    }
    fn fetched_config(&self) -> Option<Vec<u8>> {
        self.nv().read("fetched-config.xml").ok().flatten()
    }
    fn apply(&self, fetched_config: &[u8], md5: &str) -> Result<(), String> {
        let xml = std::str::from_utf8(fetched_config).map_err(|e| e.to_string())?;
        let cfg = self.config.lock().unwrap().apply_fetched(xml).map_err(|e| e.to_string())?;
        self.nv().write("md5s/config", md5.as_bytes()).map_err(|e| e.to_string())?;
        self.hub.changed(&self.store, &cfg);
        Ok(())
    }
    fn forget(&self) -> Result<(), String> {
        Ok(())
    }
}

async fn phone(nvdata: PathBuf) {
    let run = nvdata.parent().unwrap().join("run");
    let ctx = Context::standalone(Paths::new(&nvdata, &run), Model::D60);
    std::fs::create_dir_all(ctx.paths.open_dir()).unwrap();
    std::fs::create_dir_all(&ctx.paths.run).unwrap();
    let store = ConfigStore::open(Nvdata::new(&nvdata), dphone_config::Model::D60, ctx.paths.run.clone()).unwrap();
    let config = Arc::new(Mutex::new(store));
    let hub = ConfigHub::default();
    let handle = net::start_with_fallback(&ctx, Some(config.clone()), &hub, FALLBACK);
    {
        let handle = handle.clone();
        hub.on_change(move |_| handle.config_changed());
    }

    // SIP, once there is an address, and DPMA over it.
    let sub = ctx.store.subscribe(Topic::Network);
    let address = || ctx.store.read::<dphone_state::Network, _>(|n| n.ipv4.address).map(IpAddr::V4);
    let ip = loop {
        if let Some(ip) = address() {
            break ip;
        }
        sub.changed().await;
    };
    let sip_config = |ip: IpAddr| {
        let mut cfg = SipConfig::new(dphone_state::Model::D60, ip);
        cfg.local_port = PHONE_SIP_PORT;
        cfg
    };
    let mut current = sip_config(ip);
    let sip = dphone_sip::start(ctx.store.clone(), current.clone(), Arc::new(NullMedia::default()), None).await.unwrap();
    let mac = ctx.store.read::<dphone_state::System, _>(|s| s.mac).unwrap().iter().map(|b| format!("{b:02x}")).collect::<String>();
    let (transport, incoming) = dphone_dpma::sip::transport(&sip, &mac);
    let mut cfg = dphone_dpma::DpmaConfig::new(mac, dphone_state::Model::D60.user_agent(&dphone_state::FirmwareVersion::CURRENT));
    cfg.timeout = Duration::from_secs(3);
    cfg.retry_secs = 2;
    let sink = Arc::new(Sink { config: config.clone(), hub: hub.clone(), store: ctx.store.clone() });
    let dpma = dphone_dpma::start(ctx.store.clone(), cfg, transport, incoming, Arc::new(common::MemStorage::default()), sink);
    provisioning::start_discovery(&ctx, &handle, &dpma);

    // Follow the address, as `telephony` does.
    loop {
        sub.changed().await;
        let Some(ip) = address() else { continue };
        let next = sip_config(ip);
        if next != current {
            log::info!("sip: address {ip}");
            sip.reconfigure(next.clone());
            current = next;
        }
    }
}

// ------------------------------------------------------------------------
// The network (the parent process).

/// A minimal DHCP server on `ifname` leasing [`LEASED_IP`].  Counts the
/// messages it hears.
async fn dhcp_server(ifname: &'static str, heard: Arc<AtomicU32>) {
    let sock = std::net::UdpSocket::bind("0.0.0.0:67").unwrap();
    sock.set_broadcast(true).unwrap();
    sock.set_nonblocking(true).unwrap();
    dphone_net::qos::bind_to_device(&sock, ifname).unwrap();
    let sock = UdpSocket::from_std(sock).unwrap();
    let mut buf = [0u8; 1500];
    loop {
        let (n, from) = sock.recv_from(&mut buf).await.unwrap();
        let Some(req) = dhcp4::Message::parse(&buf[..n]) else { continue };
        heard.fetch_add(1, Ordering::Relaxed);
        let kind = match req.msg_type() {
            Some(MsgType::Discover) => MsgType::Offer,
            Some(MsgType::Request) => MsgType::Ack,
            _ => continue,
        };
        let mut reply = req.clone();
        reply.op = 2;
        reply.options.clear();
        reply.yiaddr = LEASED_IP;
        reply.siaddr = SERVER_IP;
        reply.set_type(kind);
        reply.options.insert(dhcp4::OPT_SERVER_ID, SERVER_IP.octets().to_vec());
        reply.options.insert(dhcp4::OPT_LEASE_TIME, 600u32.to_be_bytes().to_vec());
        reply.options.insert(dhcp4::OPT_SUBNET, vec![255, 255, 255, 0]);
        reply.options.insert(dhcp4::OPT_ROUTER, SERVER_IP.octets().to_vec());
        let dest: SocketAddr = if req.ciaddr.is_unspecified() { (Ipv4Addr::BROADCAST, 68).into() } else { from };
        sock.send_to(&reply.encode(), dest).await.unwrap();
    }
}

/// What the DPMA server saw of the phone.
#[derive(Default)]
struct Seen {
    /// The phone's address (the host of its SIP Contact) for every request
    /// the server answered, with the request's type, in arrival order.
    answered: Vec<(Ipv4Addr, String)>,
    /// Requests left unanswered because the phone was at [`DEAD_IP`].
    unheard: usize,
    /// The Contact of the last answered request, for pushes.
    contact: Option<String>,
}

fn contact_host(uri: &str) -> Option<Ipv4Addr> {
    let rest = uri.split_once('@').map_or(uri, |(_, r)| r);
    let rest = rest.trim_start_matches("sip:").trim_start_matches('<');
    rest.split(|c: char| !(c.is_ascii_digit() || c == '.')).next()?.parse().ok()
}

/// The DPMA server behind a SIP endpoint of its own.  With `deaf` set it
/// answers nothing at all (a server that is down).
async fn dpma_server(fake: Arc<Mutex<FakeServer>>, seen: Arc<Mutex<Seen>>, deaf: Arc<AtomicBool>, mut pushes: tokio::sync::mpsc::UnboundedReceiver<Vec<Message>>) {
    let mut cfg = SipConfig::new(dphone_state::Model::D60, IpAddr::V4(SERVER_IP));
    cfg.local_port = SERVER_SIP_PORT;
    let sip = dphone_sip::start(dphone_state::Store::new(dphone_state::Model::D60), cfg, Arc::new(NullMedia::default()), None).await.unwrap();
    let mut inbox = sip.messages();
    let send = |sip: &dphone_sip::SipHandle, to: String, o: Message| {
        let reply = OutMessage {
            account: None,
            to,
            from_user: Some("proxy".into()),
            headers: o.headers.iter().map(|(k, v)| (k.clone(), String::from_utf8_lossy(v).into_owned())).collect(),
            content_type: "text/plain".into(),
            body: o.body.clone(),
        };
        let sip = sip.clone();
        tokio::spawn(async move {
            let _ = sip.send_message(reply).await;
        });
    };
    loop {
        tokio::select! {
            m = inbox.recv() => {
                let Some(m) = m else { return };
                let host = contact_host(&m.contact_uri);
                if deaf.load(Ordering::Relaxed) {
                    continue;
                }
                if host == Some(DEAD_IP) {
                    seen.lock().unwrap().unheard += 1;
                    continue;
                }
                let message = Message { headers: m.headers.iter().map(|(k, v)| (k.clone(), v.clone().into_bytes())).collect(), body: m.body.clone() };
                let (out, kind) = {
                    let mut fake = fake.lock().unwrap();
                    let before = fake.log.len();
                    let out = fake.handle(message);
                    (out, fake.log.get(before).cloned())
                };
                {
                    let mut seen = seen.lock().unwrap();
                    if let (Some(host), Some(kind)) = (host, kind) {
                        seen.answered.push((host, kind));
                    }
                    seen.contact = Some(m.contact_uri.clone());
                }
                for o in out {
                    send(&sip, m.contact_uri.clone(), o);
                }
            }
            push = pushes.recv() => {
                let Some(push) = push else { return };
                let Some(to) = seen.lock().unwrap().contact.clone() else { continue };
                for o in push {
                    send(&sip, to.clone(), o);
                }
            }
        }
    }
}

/// The configuration document the server provisions, with `extra` settings.
fn document(extra: &str) -> Vec<u8> {
    format!("<config>\n<setting id=\"login_password\" value=\"789\" />\n{extra}</config>\n").into_bytes()
}

fn static_settings(address: Ipv4Addr) -> String {
    format!(
        "<setting id=\"dphone_static_address\" value=\"{address}/24\" />\n<setting id=\"dphone_static_gateway\" value=\"{SERVER_IP}\" />\n<setting id=\"dphone_static_dns\" value=\"{SERVER_IP},10.99.0.2\" />\n"
    )
}

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Prints the phone's log when the test fails.
struct ShowOnPanic(PathBuf);

impl Drop for ShowOnPanic {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("--- the phone's log ---\n{}", std::fs::read_to_string(&self.0).unwrap_or_default());
        }
    }
}

/// The addresses on the phone's interfaces ("lan", or the bridge over it).
async fn phone_addresses(nl: &mut Netlink) -> Vec<Ipv4Addr> {
    let links = nl.links().await.unwrap();
    let phone: Vec<u32> = links.iter().filter(|l| l.name == "lan" || l.name == "br0").map(|l| l.index).collect();
    nl.addrs(Some(false))
        .await
        .unwrap()
        .iter()
        .filter(|a| phone.contains(&a.index))
        .filter_map(|a| match a.addr {
            IpAddr::V4(a) => Some(a),
            IpAddr::V6(_) => None,
        })
        .collect()
}

struct Net {
    nl: Netlink,
    nvdata: PathBuf,
    log: PathBuf,
    fake: Arc<Mutex<FakeServer>>,
    seen: Arc<Mutex<Seen>>,
    pushes: tokio::sync::mpsc::UnboundedSender<Vec<Message>>,
    dhcp_heard: Arc<AtomicU32>,
}

impl Net {
    /// Wait until the phone holds exactly `address`.
    async fn address(&mut self, what: &str, limit: Duration, address: Ipv4Addr) {
        let end = Instant::now() + limit;
        loop {
            let have = phone_addresses(&mut self.nl).await;
            if have == [address] {
                return;
            }
            assert!(Instant::now() < end, "timed out waiting for {what}; the phone has {have:?}");
            sleep(Duration::from_millis(100)).await;
        }
    }

    /// The number of requests the server answered so far.
    fn mark(&self) -> usize {
        self.seen.lock().unwrap().answered.len()
    }

    /// Wait for the server to answer a request of type `kind` from the
    /// phone at `address`, among those after `mark`.
    async fn answered(&mut self, what: &str, limit: Duration, mark: usize, address: Ipv4Addr, kind: &str) {
        let end = Instant::now() + limit;
        loop {
            let found = self.seen.lock().unwrap().answered[mark..].iter().any(|(a, k)| *a == address && k == kind);
            if found {
                return;
            }
            assert!(Instant::now() < end, "timed out waiting for {what}; the server answered {:?}", &self.seen.lock().unwrap().answered[mark..]);
            sleep(Duration::from_millis(100)).await;
        }
    }

    /// Wait for a line in the phone's log, among those after `from` bytes.
    async fn logged(&self, what: &str, limit: Duration, from: usize, needle: &str) {
        let end = Instant::now() + limit;
        while !self.log_text()[from..].contains(needle) {
            assert!(Instant::now() < end, "timed out waiting for {what} ({needle:?} in the phone's log)");
            sleep(Duration::from_millis(100)).await;
        }
    }

    fn log_text(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    fn saved(&self) -> String {
        std::fs::read_to_string(self.nvdata.join("fetched-config.xml")).unwrap_or_default()
    }

    /// The server has another configuration for the phone and says so.
    fn provision(&self, extra: &str) {
        let push = {
            let mut fake = self.fake.lock().unwrap();
            fake.config = document(extra);
            fake.event("digium.config.update", b"<config></config>")
        };
        assert!(!push.is_empty(), "the server has no session to push to");
        self.pushes.send(push).unwrap();
    }

    /// Switch the phone on.  Its interfaces are as a boot finds them.
    async fn boot(&mut self) -> KillOnDrop {
        if let Some(br) = self.nl.link_by_name("br0").await.unwrap() {
            self.nl.del_link(br.index).await.unwrap();
        }
        let lan = idx(&mut self.nl, "lan").await;
        self.nl.addr_flush_v4(lan).await.unwrap();
        self.nl.set_up(lan, false).await.unwrap();
        let log = std::fs::OpenOptions::new().create(true).append(true).open(&self.log).unwrap();
        KillOnDrop(
            Command::new(std::env::current_exe().unwrap())
                .arg("--phone")
                .arg(&self.nvdata)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(log)
                .spawn()
                .unwrap(),
        )
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

async fn network(mut nl: Netlink) {
    nl.add_veth("pc", "pcpeer").await.unwrap();
    let sw = idx(&mut nl, "sw").await;
    let pcpeer = idx(&mut nl, "pcpeer").await;
    nl.set_up(sw, true).await.unwrap();
    nl.set_up(pcpeer, true).await.unwrap();
    nl.addr_add(sw, IpAddr::V4(SERVER_IP), 24).await.unwrap();

    let dhcp_heard = Arc::new(AtomicU32::new(0));
    tokio::spawn(dhcp_server("sw", dhcp_heard.clone()));
    let mut fake = FakeServer::new();
    fake.token_reqs = "mac".into();
    fake.userlist_reqs = "disabled".into();
    fake.md5_of_document = true;
    fake.config = document("");
    let fake = Arc::new(Mutex::new(fake));
    let seen = Arc::new(Mutex::new(Seen::default()));
    let deaf = Arc::new(AtomicBool::new(false));
    let (pushes, pushes_rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(dpma_server(fake.clone(), seen.clone(), deaf.clone(), pushes_rx));

    // A phone's nvdata as stock's setup leaves it: its MAC address, the
    // server it was pointed at, and the network settings in ui-config.xml
    // (DHCP), which are merged after anything the server provisions.
    // Without the VLAN discovery each bring-up is short.
    let dir = tempfile::tempdir().unwrap();
    let nvdata = dir.path().join("nvdata");
    Nvdata::new(&nvdata).ensure_layout().unwrap();
    write(&nvdata.join("etc/enet.cfg"), "ENETCFG_MAC=00:0f:d3:01:02:03\n");
    write(
        &nvdata.join("configurator.xml"),
        &format!("<config>\n    <setting id=\"config_server_url\" value=\"sip:proxy@{SERVER_IP}:{SERVER_SIP_PORT}\" />\n</config>\n"),
    );
    write(
        &nvdata.join("ui-config.xml"),
        "<config>\n    <setting id=\"network_enable_dhcp\" value=\"1\" />\n    <setting id=\"network_static_ip_address\" value=\"\" />\n    <setting id=\"network_subnet_mask\" value=\"\" />\n    <setting id=\"network_default_gateway\" value=\"\" />\n    <setting id=\"network_primary_dns_server\" value=\"\" />\n    <setting id=\"network_secondary_dns_server\" value=\"\" />\n    <setting id=\"network_vlan_discovery_mode\" value=\"NONE\" />\n</config>\n",
    );
    let log = dir.path().join("phone.log");
    let _show = ShowOnPanic(log.clone());
    let mut net = Net { nl, nvdata, log, fake, seen, pushes, dhcp_heard };
    let long = FALLBACK + Duration::from_secs(40);
    let soon = Duration::from_secs(15);

    // --- First boot: by DHCP, then the configuration ----------------------
    let phone = net.boot().await;
    net.address("the first lease", Duration::from_secs(30), LEASED_IP).await;
    net.answered("the first configuration", Duration::from_secs(30), 0, LEASED_IP, "ConfigRequest").await;
    assert!(!net.saved().contains("dphone_static"), "{}", net.saved());
    println!("staticaddr: first boot by DHCP, provisioned ok");

    // --- The server gives a static address: the phone moves ---------------
    let (mark, from) = (net.mark(), net.log_text().len());
    net.provision(&static_settings(STATIC_IP));
    net.address("the move to the static address", soon, STATIC_IP).await;
    // The DPMA session is made anew from the new address...
    net.answered("the handshake from the static address", soon, mark, STATIC_IP, "Handshake").await;
    // ...which is the server's answer the network subsystem waits for.
    net.logged("the server's answer", soon, from, "the provisioning server answers at the static address").await;
    assert!(net.saved().contains("dphone_static_address"), "{}", net.saved());
    // The gateway is the default route, and the address stays past the
    // fallback time.
    let routes = net.nl.routes(false).await.unwrap();
    assert!(routes.iter().any(|r| r.dst_len == 0 && r.gateway == Some(IpAddr::V4(SERVER_IP))), "{routes:?}");
    sleep(FALLBACK + Duration::from_secs(3)).await;
    assert_eq!(phone_addresses(&mut net.nl).await, [STATIC_IP]);
    assert!(!net.log_text()[from..].contains("falling back"), "{}", net.log_text());
    println!("staticaddr: provisioned static address, moved at run time, DPMA from the new address ok");

    // --- A boot with the saved static address ------------------------------
    drop(phone);
    let (mark, from, dhcp) = (net.mark(), net.log_text().len(), net.dhcp_heard.load(Ordering::Relaxed));
    let phone = net.boot().await;
    net.address("the saved static address", Duration::from_secs(30), STATIC_IP).await;
    net.answered("the handshake after the boot", Duration::from_secs(30), mark, STATIC_IP, "Handshake").await;
    net.logged("the server's answer after the boot", soon, from, "the provisioning server answers at the static address").await;
    sleep(FALLBACK + Duration::from_secs(3)).await;
    assert_eq!(phone_addresses(&mut net.nl).await, [STATIC_IP]);
    // From the start: this boot never asked the DHCP server for anything.
    assert_eq!(net.dhcp_heard.load(Ordering::Relaxed), dhcp, "the boot with a saved static address used DHCP");
    assert!(!net.seen.lock().unwrap().answered[mark..].iter().any(|(a, _)| *a != STATIC_IP));
    println!("staticaddr: boot with the saved static address, no DHCP ok");

    // --- The server was down at boot: no bad address is assumed ------------
    // The phones and the PBX restart together, and the PBX takes longer.
    drop(phone);
    deaf.store(true, Ordering::Relaxed);
    let (mark, from) = (net.mark(), net.log_text().len());
    let phone = net.boot().await;
    net.address("the saved static address with the server down", Duration::from_secs(30), STATIC_IP).await;
    net.address("the fallback to DHCP with the server down", long, LEASED_IP).await;
    net.logged("the reason", soon, from, "no answer from the provisioning server").await;
    // While the server stays down, so does the plain arrangement.
    sleep(Duration::from_secs(8)).await;
    assert_eq!(phone_addresses(&mut net.nl).await, [LEASED_IP]);
    // The server is back: it answers the lease, and the phone returns to
    // its static address and stays.
    deaf.store(false, Ordering::Relaxed);
    net.address("the return to the static address", long, STATIC_IP).await;
    net.answered("the handshake after the return", Duration::from_secs(30), mark, STATIC_IP, "Handshake").await;
    sleep(FALLBACK + Duration::from_secs(3)).await;
    assert_eq!(phone_addresses(&mut net.nl).await, [STATIC_IP]);
    println!("staticaddr: server down at boot, DHCP meanwhile, static address again when it answers ok");

    // --- A static address that does not work: fallback, one retry ----------
    let (mark, from) = (net.mark(), net.log_text().len());
    net.provision(&static_settings(DEAD_IP));
    net.address("the move to the dead address", soon, DEAD_IP).await;
    let moved = Instant::now();
    net.address("the fallback to DHCP", long, LEASED_IP).await;
    assert!(moved.elapsed() >= FALLBACK - Duration::from_secs(1), "fell back after {:?}, before the fallback time", moved.elapsed());
    // The server answers the lease, so the address gets its second try...
    net.answered("the handshake from the lease", Duration::from_secs(30), mark, LEASED_IP, "Handshake").await;
    net.address("the second try", Duration::from_secs(30), DEAD_IP).await;
    net.logged("the second try in the log", soon, from, "trying the settings once more").await;
    // ...and after that the lease stays.
    net.address("the second fallback", long, LEASED_IP).await;
    net.logged("the final fallback in the log", soon, from, "on the second try either").await;
    let mark_final = net.mark();
    net.answered("the handshake from the lease again", Duration::from_secs(30), mark_final.saturating_sub(1).max(mark), LEASED_IP, "Handshake").await;
    sleep(FALLBACK + Duration::from_secs(5)).await;
    assert_eq!(phone_addresses(&mut net.nl).await, [LEASED_IP], "the plain arrangement should stay");
    assert_eq!(net.log_text()[from..].matches("trying the settings once more").count(), 1, "exactly one retry");
    assert!(net.seen.lock().unwrap().unheard > 0);
    assert!(net.saved().contains(&DEAD_IP.to_string()), "the saved configuration still names the dead address");
    println!("staticaddr: unusable static address falls back to DHCP, is retried once, then DHCP stays ok");

    // --- A boot with a saved static address that does not work -------------
    drop(phone);
    let mark = net.mark();
    let phone = net.boot().await;
    net.address("the saved dead address", Duration::from_secs(30), DEAD_IP).await;
    net.address("the fallback to DHCP after the boot", long, LEASED_IP).await;
    net.answered("the handshake from the lease after the boot", Duration::from_secs(30), mark, LEASED_IP, "Handshake").await;
    println!("staticaddr: boot with an unusable saved static address reaches the server by DHCP ok");

    // --- The server corrects the address: the fallback ends ----------------
    // (The second try of the dead address may be under way; either way the
    // new settings are what the phone ends up with.)
    net.address("the lease before the correction", long + FALLBACK, LEASED_IP).await;
    let (mark, from) = (net.mark(), net.log_text().len());
    net.answered("a session to push to", long, mark.saturating_sub(1), LEASED_IP, "Handshake").await;
    net.provision(&static_settings(STATIC_IP));
    net.address("the corrected static address", soon, STATIC_IP).await;
    net.logged("the server's answer at the corrected address", soon, from, "the provisioning server answers at the static address").await;
    println!("staticaddr: a corrected address ends the fallback ok");

    // --- The server takes the address away: back to DHCP -------------------
    let mark = net.mark();
    net.provision("");
    net.address("the lease after the removal", Duration::from_secs(30), LEASED_IP).await;
    net.answered("the handshake from the lease after the removal", Duration::from_secs(30), mark, LEASED_IP, "Handshake").await;
    assert!(!net.saved().contains("dphone_static"), "{}", net.saved());
    sleep(Duration::from_secs(3)).await;
    assert_eq!(phone_addresses(&mut net.nl).await, [LEASED_IP]);
    println!("staticaddr: the address taken away, back to DHCP ok");
    drop(phone);
}
