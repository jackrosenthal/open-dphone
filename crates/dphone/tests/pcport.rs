//! The PC jack as a phone with default settings runs it: the main role's
//! network subsystem (configuration mapping, flag file, fallback, and the
//! `dphone net` commands) over the real `NetManager`, in a private network
//! namespace where veth pairs stand in for the two jacks.
//!
//! "lan" and "pc" are the phone's ports, "sw" is the switch port the LAN
//! jack is plugged into (with a DHCP server), and "pcpeer" is the PC.
//!
//! Needs unprivileged user and network namespaces; where those are missing
//! it prints a notice and passes without running.  It is a `harness = false`
//! test because `unshare(CLONE_NEWUSER)` needs a single-threaded process.
//!
//! What this cannot show is the b53 switch doing the same in hardware: veth
//! ports and the software bridge have no switch behind them.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dphone::diag::Output;
use dphone::mainrole::Context;
use dphone::mainrole::subsystems::net;
use dphone::mainrole::subsystems::system::ConfigHub;
use dphone::model::Model;
use dphone::paths::Paths;
use dphone_config::{ConfigStore, Nvdata};
use dphone_net::dhcp4::{self, Message, MsgType};
use dphone_net::netlink::Netlink;
use dphone_net::packet::PacketSocket;
use dphone_net::MacAddr;
use tokio::net::UdpSocket;
use tokio::time::{sleep, timeout, Instant};

const SERVER_IP: Ipv4Addr = Ipv4Addr::new(10, 99, 0, 1);
const LEASED_IP: Ipv4Addr = Ipv4Addr::new(10, 99, 0, 50);
/// Seconds without an address before the subsystem falls back (150 on a
/// phone).  Longer than any bring-up here takes: with the default settings
/// that is the wait for LLDP-MED, the active discovery after it, and the
/// address check, about 20 s.
const FALLBACK: Duration = Duration::from_secs(32);
/// An experimental EtherType, for frames that are nobody's protocol.
const ETH_TEST: u16 = 0x88b5;
const ETH_PAE: u16 = 0x888e;
const PC_VLAN: u16 = 20;

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
        eprintln!("    [{}] {}", r.level(), r.args());
    }
    fn flush(&self) {}
}

fn main() {
    if !enter_namespace() {
        println!("pcport: SKIPPED (unprivileged user and network namespaces are not available)");
        return;
    }
    if std::env::var_os("NET_TEST_LOG").is_some() {
        log::set_logger(&StderrLog).unwrap();
        log::set_max_level(log::LevelFilter::Info);
    }
    // The phone and its DHCP server share this namespace; let the kernel
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
            println!("pcport: SKIPPED (the host kernel cannot create a VLAN-filtering bridge)");
            return;
        }
        let probe = idx(&mut nl, "probe0").await;
        nl.del_link(probe).await.unwrap();
        if nl.add_veth("lan", "sw").await.is_err() {
            println!("pcport: SKIPPED (the host kernel has no veth)");
            return;
        }
        run(nl).await;
        println!("pcport: ok");
    });
}

async fn idx(nl: &mut Netlink, name: &str) -> u32 {
    nl.link_by_name(name).await.unwrap().unwrap_or_else(|| panic!("no interface {name}")).index
}

/// A minimal DHCP server on `ifname` leasing [`LEASED_IP`].  While `deaf`
/// is set it answers nothing.
async fn dhcp_server(ifname: &'static str, lease_secs: Arc<AtomicU32>, deaf: Arc<AtomicBool>) {
    let sock = std::net::UdpSocket::bind("0.0.0.0:67").unwrap();
    sock.set_broadcast(true).unwrap();
    sock.set_nonblocking(true).unwrap();
    dphone_net::qos::bind_to_device(&sock, ifname).unwrap();
    let sock = UdpSocket::from_std(sock).unwrap();
    let mut buf = [0u8; 1500];
    loop {
        let (n, from) = sock.recv_from(&mut buf).await.unwrap();
        let Some(req) = Message::parse(&buf[..n]) else { continue };
        if deaf.load(Ordering::Relaxed) {
            continue;
        }
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
        reply.options.insert(dhcp4::OPT_LEASE_TIME, lease_secs.load(Ordering::Relaxed).to_be_bytes().to_vec());
        reply.options.insert(dhcp4::OPT_SUBNET, vec![255, 255, 255, 0]);
        reply.options.insert(dhcp4::OPT_ROUTER, SERVER_IP.octets().to_vec());
        let dest: SocketAddr = if req.ciaddr.is_unspecified() { (Ipv4Addr::BROADCAST, 68).into() } else { from };
        sock.send_to(&reply.encode(), dest).await.unwrap();
    }
}

/// Send a frame on `from` and report whether it shows up on `to`.
async fn frame_crosses(from: u32, to: u32, ethertype: u16, dst: MacAddr) -> bool {
    let tx = PacketSocket::new(from, ethertype).unwrap();
    let rx = PacketSocket::new(to, ethertype).unwrap();
    let payload = [0x5au8; 60];
    for _ in 0..5 {
        tx.send(dst, &payload).await.unwrap();
        let mut buf = [0u8; 128];
        match timeout(Duration::from_millis(200), rx.recv(&mut buf)).await {
            Ok(Ok((n, _))) if buf[..n] == payload[..] => return true,
            _ => {}
        }
    }
    false
}

/// What the test needs to know about the phone's side, read from the kernel.
#[derive(Debug, Clone, PartialEq)]
struct Kernel {
    bridge: bool,
    lan_bridged: bool,
    pc_bridged: bool,
    pc_up: bool,
    /// The interface that holds the leased address.
    address_on: Option<String>,
}

async fn kernel(nl: &mut Netlink) -> Kernel {
    let links = nl.links().await.unwrap();
    let br = links.iter().find(|l| l.name == "br0").map(|l| l.index);
    let by_name = |n: &str| links.iter().find(|l| l.name == n).unwrap();
    let addrs = nl.addrs(Some(false)).await.unwrap();
    let address_on = addrs
        .iter()
        .find(|a| a.addr == IpAddr::V4(LEASED_IP))
        .and_then(|a| links.iter().find(|l| l.index == a.index))
        .map(|l| l.name.clone());
    Kernel {
        bridge: br.is_some(),
        lan_bridged: br.is_some() && by_name("lan").master == br,
        pc_bridged: br.is_some() && by_name("pc").master == br,
        pc_up: by_name("pc").is_admin_up(),
        address_on,
    }
}

async fn wait_kernel(nl: &mut Netlink, what: &str, limit: Duration, f: impl Fn(&Kernel) -> bool) -> Kernel {
    let end = Instant::now() + limit;
    loop {
        let k = kernel(nl).await;
        if f(&k) {
            return k;
        }
        assert!(Instant::now() < end, "timed out waiting for {what}; the kernel has {k:?}");
        sleep(Duration::from_millis(100)).await;
    }
}

/// Run a `dphone` diagnostic command as the socket would.
async fn dphone(ctx: &Context, args: &[&str]) -> String {
    let (out, mut rx) = Output::channel(64);
    let status = ctx.diag.dispatch(args.iter().map(|a| a.to_string()).collect(), out).await;
    let mut text = String::new();
    while let Ok(frame) = rx.try_recv() {
        // Frame: one kind byte, a four-byte length, the text.
        text.push_str(&String::from_utf8_lossy(&frame[5..]));
    }
    assert_eq!(status, 0, "dphone {args:?}: {text}");
    text
}

async fn run(mut nl: Netlink) {
    nl.add_veth("pc", "pcpeer").await.unwrap();
    let sw = idx(&mut nl, "sw").await;
    let pcpeer = idx(&mut nl, "pcpeer").await;
    nl.set_up(sw, true).await.unwrap();
    nl.set_up(pcpeer, true).await.unwrap();
    nl.addr_add(sw, IpAddr::V4(SERVER_IP), 24).await.unwrap();
    // Where the switch would see the PC VLAN.
    nl.add_vlan("sw.pc", sw, PC_VLAN, &[], &[]).await.unwrap();
    let sw_pc = idx(&mut nl, "sw.pc").await;
    nl.set_up(sw_pc, true).await.unwrap();

    let lease_secs = Arc::new(AtomicU32::new(600));
    let deaf = Arc::new(AtomicBool::new(false));
    let server = tokio::spawn(dhcp_server("sw", lease_secs.clone(), deaf.clone()));

    // A phone's nvdata with nothing in it but its MAC address: every
    // setting is the built-in default.
    let dir = tempfile::tempdir().unwrap();
    let nvdata = dir.path().join("nvdata");
    let nv = Nvdata::new(&nvdata);
    nv.ensure_layout().unwrap();
    std::fs::write(nvdata.join("etc/enet.cfg"), "ENETCFG_MAC=00:0f:d3:01:02:03\n").unwrap();
    let ctx = Context::standalone(Paths::new(&nvdata, dir.path().join("run")), Model::D60);
    std::fs::create_dir_all(ctx.paths.open_dir()).unwrap();
    std::fs::create_dir_all(&ctx.paths.run).unwrap();
    let store = ConfigStore::open(nv, dphone_config::Model::D60, ctx.paths.run.clone()).unwrap();
    let config = Arc::new(Mutex::new(store));
    let hub = ConfigHub::default();
    let handle = net::start_with_fallback(&ctx, Some(config.clone()), &hub, FALLBACK);
    hub.on_change(move |_| handle.config_changed());
    // A settings change as `dphone config set` or the web UI makes it.
    let set = |changes: &[(&str, &str)]| {
        let cfg = config.lock().unwrap().set_many(changes).unwrap();
        hub.changed(&ctx.store, &cfg);
    };

    let pc_dst = MacAddr::parse("02:00:00:00:00:99").unwrap();
    let pae = MacAddr::parse("01:80:c2:00:00:03").unwrap();
    let bridged_up = |k: &Kernel| k.bridge && k.lan_bridged && k.pc_bridged && k.pc_up && k.address_on.as_deref() == Some("br0");
    let standalone_up = |k: &Kernel| !k.bridge && !k.pc_up && k.address_on.as_deref() == Some("lan");

    // --- Default settings: the jacks are bridged --------------------------
    let started = Instant::now();
    wait_kernel(&mut nl, "the default arrangement with a lease", FALLBACK, bridged_up).await;
    let bring_up = started.elapsed();
    assert!(bring_up + Duration::from_secs(3) < FALLBACK, "bring-up took {bring_up:?}; FALLBACK is too short for this test");
    assert!(frame_crosses(pcpeer, sw, ETH_TEST, pc_dst).await, "PC to LAN");
    assert!(frame_crosses(sw, pcpeer, ETH_TEST, pc_dst).await, "LAN to PC");
    assert!(!frame_crosses(pcpeer, sw, ETH_PAE, pae).await, "EAPOL crossed with 8021x_passthrough off");
    assert!(!frame_crosses(pcpeer, sw_pc, ETH_TEST, pc_dst).await, "PC frames are tagged without a PC VLAN");
    let status = dphone(&ctx, &["net", "status"]).await;
    assert!(status.contains("PC jack bridged") && status.contains("br0 (bridged)"), "{status}");
    sleep(Duration::from_millis(300)).await;
    let n = ctx.store.get::<dphone_state::Network>();
    assert!(n.pc_port_enabled && n.pc.up && n.ipv4.address == Some(LEASED_IP), "{n:?}");
    assert!(ctx.health.is_healthy());
    println!("pcport: default settings bridge the PC jack ({bring_up:?} to a lease)");

    // The rest changes settings many times; without the VLAN discovery
    // each bring-up is short.
    set(&[("network_vlan_discovery_mode", "NONE")]);
    sleep(Duration::from_secs(1)).await;
    wait_kernel(&mut nl, "the lease without VLAN discovery", Duration::from_secs(30), bridged_up).await;

    // --- 8021x_passthrough ------------------------------------------------
    set(&[("8021x_passthrough", "1")]);
    let end = Instant::now() + Duration::from_secs(30);
    while !frame_crosses(pcpeer, sw, ETH_PAE, pae).await {
        assert!(Instant::now() < end, "EAPOL does not cross with 8021x_passthrough on");
    }
    assert!(frame_crosses(sw, pcpeer, ETH_PAE, pae).await, "EAPOL toward the PC");
    wait_kernel(&mut nl, "the lease with pass-through", Duration::from_secs(30), bridged_up).await;
    set(&[("8021x_passthrough", "0")]);
    let end = Instant::now() + Duration::from_secs(30);
    while frame_crosses(pcpeer, sw, ETH_PAE, pae).await {
        assert!(Instant::now() < end, "EAPOL still crosses with 8021x_passthrough off again");
    }
    wait_kernel(&mut nl, "the lease without pass-through", Duration::from_secs(30), bridged_up).await;
    println!("pcport: 8021x_passthrough ok");

    // --- pc_vlan_id: the PC's untagged traffic leaves tagged --------------
    set(&[("pc_vlan_id", "20"), ("pc_qos", "5")]);
    let end = Instant::now() + Duration::from_secs(30);
    while !frame_crosses(pcpeer, sw_pc, ETH_TEST, pc_dst).await {
        assert!(Instant::now() < end, "PC frames do not arrive on VLAN {PC_VLAN}");
    }
    assert!(frame_crosses(sw_pc, pcpeer, ETH_TEST, pc_dst).await, "VLAN {PC_VLAN} toward the PC, untagged");
    wait_kernel(&mut nl, "the lease with a PC VLAN", Duration::from_secs(30), bridged_up).await;
    assert_eq!(ctx.store.get::<dphone_state::Network>().vlan.pc_id, Some(PC_VLAN));
    set(&[("pc_vlan_id", ""), ("pc_qos", "1")]);
    let end = Instant::now() + Duration::from_secs(30);
    while frame_crosses(pcpeer, sw_pc, ETH_TEST, pc_dst).await {
        assert!(Instant::now() < end, "PC frames still arrive on VLAN {PC_VLAN}");
    }
    wait_kernel(&mut nl, "the lease without a PC VLAN", Duration::from_secs(30), bridged_up).await;
    assert!(frame_crosses(pcpeer, sw, ETH_TEST, pc_dst).await);
    println!("pcport: pc_vlan_id ok");

    // --- pc_port_mode off: stock's switch for the jack --------------------
    set(&[("pc_port_mode", "off")]);
    wait_kernel(&mut nl, "the PC jack off", Duration::from_secs(30), standalone_up).await;
    assert!(!frame_crosses(pcpeer, sw, ETH_TEST, pc_dst).await, "a PC jack that is off must not forward");
    sleep(Duration::from_millis(300)).await;
    assert!(!ctx.store.get::<dphone_state::Network>().pc_port_enabled);
    set(&[("pc_port_mode", "auto")]);
    wait_kernel(&mut nl, "the PC jack on again", Duration::from_secs(30), bridged_up).await;
    println!("pcport: pc_port_mode ok");

    // --- The bring-up hold ------------------------------------------------
    let flag = net::bridge_hold_flag(&ctx);
    let text = dphone(&ctx, &["net", "bridge", "off", "--save"]).await;
    assert!(flag.exists(), "{text}");
    wait_kernel(&mut nl, "the held arrangement", Duration::from_secs(30), standalone_up).await;
    assert!(!frame_crosses(pcpeer, sw, ETH_TEST, pc_dst).await);
    // Neither re-applying the settings nor a settings change lifts it.
    dphone(&ctx, &["net", "configured"]).await;
    wait_kernel(&mut nl, "the held arrangement after 'configured'", Duration::from_secs(30), standalone_up).await;
    set(&[("pc_vlan_id", "20")]);
    sleep(Duration::from_secs(1)).await;
    set(&[("pc_vlan_id", "")]);
    wait_kernel(&mut nl, "the held arrangement after a settings change", Duration::from_secs(30), standalone_up).await;
    assert!(dphone(&ctx, &["net", "status"]).await.contains("PC jack off"));
    // `bridge on` alone bridges now and leaves the file.
    dphone(&ctx, &["net", "bridge", "on"]).await;
    wait_kernel(&mut nl, "the bridge by command", Duration::from_secs(30), bridged_up).await;
    assert!(flag.exists());
    dphone(&ctx, &["net", "configured"]).await;
    wait_kernel(&mut nl, "the held arrangement again", Duration::from_secs(30), standalone_up).await;
    let text = dphone(&ctx, &["net", "bridge", "on", "--save"]).await;
    assert!(!flag.exists(), "{text}");
    wait_kernel(&mut nl, "the bridge after the hold was lifted", Duration::from_secs(30), bridged_up).await;
    assert!(frame_crosses(pcpeer, sw, ETH_TEST, pc_dst).await);
    println!("pcport: the hold file and 'dphone net bridge' ok");

    // --- The address is lost while bridged: back to the plain arrangement -
    // The fault to survive: the bridged arrangement passes nothing, the
    // plain one works.  Here the DHCP server stops answering for as long as
    // the bridge exists.
    lease_secs.store(6, Ordering::Relaxed);
    // A new bring-up, for a new (short) lease.
    dphone(&ctx, &["net", "plain"]).await;
    wait_kernel(&mut nl, "the plain arrangement by command", Duration::from_secs(30), standalone_up).await;
    dphone(&ctx, &["net", "configured"]).await;
    wait_kernel(&mut nl, "a short lease", Duration::from_secs(30), bridged_up).await;
    let watcher = {
        let deaf = deaf.clone();
        tokio::spawn(async move {
            let mut nl = Netlink::new().unwrap();
            loop {
                let bridge = nl.link_by_name("br0").await.unwrap().is_some();
                deaf.store(bridge, Ordering::Relaxed);
                sleep(Duration::from_millis(50)).await;
            }
        })
    };
    let lost = Instant::now();
    wait_kernel(&mut nl, "the lease to run out", Duration::from_secs(20), |k| k.address_on.is_none()).await;
    assert!(kernel(&mut nl).await.bridge, "the bridge stays while the fallback time runs");
    wait_kernel(&mut nl, "the fallback to the plain arrangement", FALLBACK + Duration::from_secs(40), standalone_up).await;
    let took = lost.elapsed();
    assert!(took >= FALLBACK, "fell back after {took:?}, before the fallback time");
    let status = dphone(&ctx, &["net", "status"]).await;
    assert!(status.contains("(plain)") && status.contains("PC jack off"), "{status}");
    assert!(!frame_crosses(pcpeer, sw, ETH_TEST, pc_dst).await);
    // It stays there: no return to the arrangement that failed.
    sleep(Duration::from_secs(8)).await;
    assert!(standalone_up(&kernel(&mut nl).await), "the plain arrangement should stay");
    println!("pcport: fallback to the plain arrangement after {took:?} without an address ok");

    // --- Told otherwise: the settings are applied again -------------------
    watcher.abort();
    deaf.store(false, Ordering::Relaxed);
    lease_secs.store(600, Ordering::Relaxed);
    dphone(&ctx, &["net", "configured"]).await;
    wait_kernel(&mut nl, "the bridge after 'dphone net configured'", Duration::from_secs(30), bridged_up).await;
    println!("pcport: 'dphone net configured' returns to the settings");
    server.abort();
}
