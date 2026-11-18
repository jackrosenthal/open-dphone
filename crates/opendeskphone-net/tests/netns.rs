//! End-to-end tests in a private network namespace: rtnetlink operations,
//! and the whole `NetManager` against a fake switch (LLDP-MED), DHCP
//! server, DNS server, and time server on the far end of a veth pair.
//!
//! The test needs unprivileged user and network namespaces.  Where those
//! are unavailable it prints a notice and passes without running.
//!
//! This is a `harness = false` test because `unshare(CLONE_NEWUSER)` only
//! works in a single-threaded process.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use opendeskphone_net::dhcp4::{self, Message, MsgType};
use opendeskphone_net::dns;
use opendeskphone_net::lldp;
use opendeskphone_net::netlink::{BridgeOpts, Netlink, VlanFlags};
use opendeskphone_net::packet::PacketSocket;
use opendeskphone_net::{Ipv4Config, MacAddr, NetConfig, NetManager, Phase, VlanDiscovery, VlanSource};
use tokio::net::UdpSocket;
use tokio::time::{sleep, timeout};

const PHONE_MAC: &str = "00:0f:d3:01:02:03";
const SERVER_IP: Ipv4Addr = Ipv4Addr::new(10, 99, 0, 1);
const LEASED_IP: Ipv4Addr = Ipv4Addr::new(10, 99, 0, 50);

fn enter_namespace() -> bool {
    // Given by a wrapper that already ran `unshare -Urn` (needed under
    // emulators whose own threads make `unshare(CLONE_NEWUSER)` fail).
    if std::env::args().any(|a| a == "--in-namespace") {
        return true;
    }
    // SAFETY: plain system calls with no pointers.
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    // SAFETY: the process is single-threaded here.
    if unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET) } != 0 {
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
        test_inputs::unavailable("netns: unprivileged user and network namespaces are not available");
        return;
    }
    // `cargo test --test netns -- --log`: the manager's log on stderr.
    if std::env::args().any(|a| a == "--log") {
        log::set_logger(&StderrLog).unwrap();
        log::set_max_level(log::LevelFilter::Debug);
    }
    // The phone and its servers share this one namespace, so each side sees
    // the other's packets arrive from an address that is also local.  Let
    // the kernel accept that.
    for (key, value) in [
        ("all/accept_local", "1"),
        ("all/rp_filter", "0"),
        ("default/rp_filter", "0"),
    ] {
        std::fs::write(format!("/proc/sys/net/ipv4/conf/{key}"), value).unwrap();
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let caps = netlink_operations().await;
        if caps.bridge && caps.vlan {
            manager_end_to_end().await;
        } else {
            test_inputs::unavailable(&format!("netns: manager test: the host kernel lacks bridge or 802.1Q support: {caps:?}"));
            manager_without_bridge().await;
        }
    });
    println!("netns: ok");
}

#[derive(Debug)]
struct Caps {
    bridge: bool,
    vlan: bool,
}

async fn idx(nl: &mut Netlink, name: &str) -> u32 {
    nl.link_by_name(name)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("no interface {name}"))
        .index
}

/// Exercise every rtnetlink operation the crate has.
async fn netlink_operations() -> Caps {
    let mut nl = Netlink::new().unwrap();
    assert!(nl.link_by_name("nope0").await.unwrap().is_none());
    let lo = idx(&mut nl, "lo").await;
    nl.set_up(lo, true).await.unwrap();

    nl.add_veth("t0", "t1").await.unwrap();
    let (t0, t1) = (idx(&mut nl, "t0").await, idx(&mut nl, "t1").await);
    let mac = MacAddr::parse("02:00:00:00:00:aa").unwrap();
    nl.set_mac(t0, mac).await.unwrap();
    nl.set_mtu(t0, 1400).await.unwrap();
    nl.set_up(t0, true).await.unwrap();
    nl.set_up(t1, true).await.unwrap();
    let l = nl.link_by_index(t0).await.unwrap().unwrap();
    assert_eq!(
        (l.name.as_str(), l.mac, l.mtu, l.kind.as_deref()),
        ("t0", Some(mac), 1400, Some("veth"))
    );
    assert!(l.is_admin_up());
    // Carrier arrives once both ends are up.
    let mut carrier = false;
    for _ in 0..50 {
        if nl.link_by_index(t0).await.unwrap().unwrap().has_carrier() {
            carrier = true;
            break;
        }
        sleep(Duration::from_millis(20)).await;
    }
    assert!(carrier, "veth never reported carrier");

    // Addresses and routes.
    let a = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10));
    nl.addr_add(t0, a, 24).await.unwrap();
    nl.addr_add(t0, a, 24).await.unwrap(); // replace is not an error
    let addrs = nl.addrs(Some(false)).await.unwrap();
    assert!(
        addrs
            .iter()
            .any(|x| x.index == t0 && x.addr == a && x.prefix_len == 24)
    );
    let gw = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
    nl.route_add_default(t0, gw, 0, opendeskphone_net::netlink::RTPROT_DHCP)
        .await
        .unwrap();
    let far_gw = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 1));
    nl.route_add_onlink(t0, far_gw, 32, opendeskphone_net::netlink::RTPROT_DHCP)
        .await
        .unwrap();
    nl.route_add_default(t0, far_gw, 1, opendeskphone_net::netlink::RTPROT_DHCP)
        .await
        .unwrap();
    let routes = nl.routes(false).await.unwrap();
    let defaults: Vec<_> = routes
        .iter()
        .filter(|r| r.dst_len == 0 && r.oif == Some(t0) && r.table == 254)
        .collect();
    assert_eq!(defaults.len(), 2, "{routes:?}");
    assert!(
        defaults
            .iter()
            .any(|r| r.gateway == Some(gw) && r.metric == 0)
    );
    assert!(
        defaults
            .iter()
            .any(|r| r.gateway == Some(far_gw) && r.metric == 1)
    );
    nl.route_flush(Some(t0), false, &[opendeskphone_net::netlink::RTPROT_DHCP])
        .await
        .unwrap();
    let routes = nl.routes(false).await.unwrap();
    assert!(
        !routes
            .iter()
            .any(|r| r.protocol == opendeskphone_net::netlink::RTPROT_DHCP && r.table == 254),
        "{routes:?}"
    );
    nl.addr_flush_v4(t0).await.unwrap();
    assert!(
        !nl.addrs(Some(false))
            .await
            .unwrap()
            .iter()
            .any(|x| x.index == t0)
    );

    // VLAN device with priority maps.
    let vlan = match nl
        .add_vlan("t0.7", t0, 7, &[(5, 6), (3, 3)], &[(5, 6)])
        .await
    {
        Ok(()) => {
            let v = nl.link_by_name("t0.7").await.unwrap().unwrap();
            assert_eq!((v.kind.as_deref(), v.parent), (Some("vlan"), Some(t0)));
            let egress = std::fs::read_to_string("/proc/net/vlan/t0.7").unwrap_or_default();
            if !egress.is_empty() {
                assert!(egress.contains("5:6") && egress.contains("3:3"), "{egress}");
            }
            nl.del_link(v.index).await.unwrap();
            true
        }
        Err(e) => {
            println!("netns: no 802.1Q support here: {e}");
            false
        }
    };

    // Bridge with VLAN filtering and the EAPOL group-forward bit.
    let opts = BridgeOpts {
        vlan_filtering: true,
        group_fwd_mask: 8,
        default_pvid: 1,
    };
    let bridge = match nl.add_bridge("tbr", &opts).await {
        Ok(()) => {
            let br = idx(&mut nl, "tbr").await;
            nl.set_master(t0, Some(br)).await.unwrap();
            assert_eq!(
                nl.link_by_index(t0).await.unwrap().unwrap().master,
                Some(br)
            );
            nl.bridge_vlan_add(t0, 100, VlanFlags::default(), false)
                .await
                .unwrap();
            nl.bridge_vlan_add(br, 100, VlanFlags::default(), true)
                .await
                .unwrap();
            nl.bridge_vlan_add(
                t0,
                20,
                VlanFlags {
                    pvid: true,
                    untagged: true,
                },
                false,
            )
            .await
            .unwrap();
            nl.bridge_vlan_del(t0, 1, false).await.unwrap();
            assert_eq!(nl.bridge_opts(br).await.unwrap(), Some(opts));
            assert_eq!(nl.bridge_opts(t0).await.unwrap(), None);
            let plain = BridgeOpts {
                vlan_filtering: false,
                group_fwd_mask: 0,
                default_pvid: 1,
            };
            nl.set_bridge_opts(br, &plain).await.unwrap();
            assert_eq!(nl.bridge_opts(br).await.unwrap(), Some(plain));
            nl.set_master(t0, None).await.unwrap();
            assert_eq!(nl.link_by_index(t0).await.unwrap().unwrap().master, None);
            nl.del_link(br).await.unwrap();
            true
        }
        Err(e) => {
            println!("netns: no bridge support here: {e}");
            false
        }
    };

    // The monitor reports link changes.
    let mut mon = opendeskphone_net::netlink::Monitor::new().unwrap();
    nl.set_up(t1, false).await.unwrap();
    let ev = timeout(Duration::from_secs(2), async {
        loop {
            match mon.next().await.unwrap() {
                opendeskphone_net::netlink::Event::Link(l) if l.index == t0 && !l.has_carrier() => {
                    return l;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("no link-down event");
    assert_eq!(ev.name, "t0");

    nl.del_link(t0).await.unwrap();
    assert!(nl.link_by_name("t1").await.unwrap().is_none());
    println!("netns: rtnetlink operations ok (bridge: {bridge}, vlan: {vlan})");
    Caps { bridge, vlan }
}

#[derive(Default)]
struct ServerLog {
    vendor_class: Option<String>,
    discovers: u32,
    requests: u32,
    renewals: u32,
    reboots: u32,
    releases: u32,
    param_list: Vec<u8>,
    client_id: Vec<u8>,
}

/// A minimal DHCP server on `ifname`, leasing one address for `lease_secs`.
async fn dhcp_server(ifname: &str, lease_secs: u32, log: Arc<Mutex<ServerLog>>) {
    let sock = std::net::UdpSocket::bind("0.0.0.0:67").unwrap();
    sock.set_broadcast(true).unwrap();
    sock.set_nonblocking(true).unwrap();
    opendeskphone_net::qos::bind_to_device(&sock, ifname).unwrap();
    let sock = UdpSocket::from_std(sock).unwrap();
    let mut buf = [0u8; 1500];
    loop {
        let (n, from) = sock.recv_from(&mut buf).await.unwrap();
        let Some(req) = Message::parse(&buf[..n]) else {
            continue;
        };
        log::debug!(
            "test server: {:?} from {from} ciaddr {}",
            req.msg_type(),
            req.ciaddr
        );
        let mut reply = req.clone();
        reply.op = 2;
        reply.options.clear();
        reply.yiaddr = LEASED_IP;
        reply.siaddr = SERVER_IP;
        let kind = match req.msg_type() {
            Some(MsgType::Discover) => {
                let mut l = log.lock().unwrap();
                l.discovers += 1;
                l.vendor_class = req.opt_str(dhcp4::OPT_VENDOR_CLASS);
                l.param_list = req
                    .options
                    .get(&dhcp4::OPT_PARAM_LIST)
                    .cloned()
                    .unwrap_or_default();
                l.client_id = req
                    .options
                    .get(&dhcp4::OPT_CLIENT_ID)
                    .cloned()
                    .unwrap_or_default();
                MsgType::Offer
            }
            Some(MsgType::Request) => {
                let mut l = log.lock().unwrap();
                l.requests += 1;
                if !req.ciaddr.is_unspecified() {
                    l.renewals += 1;
                } else if req.opt_ip(dhcp4::OPT_SERVER_ID).is_none() {
                    l.reboots += 1;
                }
                MsgType::Ack
            }
            Some(MsgType::Release) => {
                log.lock().unwrap().releases += 1;
                continue;
            }
            _ => continue,
        };
        reply.set_type(kind);
        reply
            .options
            .insert(dhcp4::OPT_SERVER_ID, SERVER_IP.octets().to_vec());
        reply
            .options
            .insert(dhcp4::OPT_LEASE_TIME, lease_secs.to_be_bytes().to_vec());
        reply
            .options
            .insert(dhcp4::OPT_SUBNET, vec![255, 255, 255, 0]);
        reply
            .options
            .insert(dhcp4::OPT_ROUTER, SERVER_IP.octets().to_vec());
        reply
            .options
            .insert(dhcp4::OPT_DNS, SERVER_IP.octets().to_vec());
        reply
            .options
            .insert(dhcp4::OPT_NTP, SERVER_IP.octets().to_vec());
        reply
            .options
            .insert(dhcp4::OPT_DOMAIN, b"lab.test".to_vec());
        reply
            .options
            .insert(dhcp4::OPT_TFTP_SERVER, b"sip:proxy@10.99.0.1:5060".to_vec());
        let dest: SocketAddr = if req.ciaddr.is_unspecified() {
            (Ipv4Addr::BROADCAST, 68).into()
        } else {
            from
        };
        sock.send_to(&reply.encode(), dest).await.unwrap();
    }
}

async fn dns_server() {
    let sock = UdpSocket::bind((SERVER_IP, 53)).await.unwrap();
    let mut buf = [0u8; 512];
    loop {
        let (n, from) = sock.recv_from(&mut buf).await.unwrap();
        let Some(q) = dns::Message::parse(&buf[..n]) else {
            continue;
        };
        let mut r = dns::Message {
            id: q.id,
            flags: 0x8180,
            questions: q.questions.clone(),
            ..Default::default()
        };
        let name = dns::normalize(&q.questions[0].name);
        if q.questions[0].qtype == dns::TYPE_A && (name == "pbx.lab.test" || name == "pool.ntp.org")
        {
            r.answers.push(dns::Record {
                name: q.questions[0].name.clone(),
                rtype: dns::TYPE_A,
                class: dns::CLASS_IN,
                ttl: 30,
                data: dns::RData::A(if name == "pbx.lab.test" {
                    Ipv4Addr::new(10, 99, 0, 7)
                } else {
                    SERVER_IP
                }),
            });
        } else {
            r.flags |= 3;
        }
        sock.send_to(&r.encode().unwrap(), from).await.unwrap();
    }
}

async fn time_server(hits: Arc<Mutex<u32>>) {
    let sock = UdpSocket::bind((SERVER_IP, 123)).await.unwrap();
    let mut buf = [0u8; 64];
    loop {
        let (n, from) = sock.recv_from(&mut buf).await.unwrap();
        if n < 48 {
            continue;
        }
        let mut r = [0u8; 48];
        r[0] = 0x24;
        r[1] = 2;
        r[24..32].copy_from_slice(&buf[40..48]);
        // Any plausible time will do; the test never sets the clock.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 2_208_988_800;
        r[32..36].copy_from_slice(&(now as u32).to_be_bytes());
        r[40..44].copy_from_slice(&(now as u32).to_be_bytes());
        *hits.lock().unwrap() += 1;
        sock.send_to(&r, from).await.unwrap();
    }
}

/// What a switch port with a voice VLAN sends.
fn switch_lldpdu(voice_vid: u16) -> Vec<u8> {
    let mut f = vec![
        0x02, 0x07, 0x04, 0x02, 0x00, 0x00, 0x00, 0x00, 0x99, // chassis ID
        0x04, 0x04, 0x05, b'g', b'e', b'1', // port ID
        0x06, 0x02, 0x00, 0x78, // TTL
    ];
    f.extend_from_slice(&[0xfe, 0x07, 0x00, 0x12, 0xbb, 0x01, 0x00, 0x0f, 0x04]);
    for (app, prio, dscp) in [(1u8, 5u32, 46u32), (2, 4, 24)] {
        let v: u32 = 1 << 22 | (voice_vid as u32) << 9 | prio << 6 | dscp;
        f.extend_from_slice(&[
            0xfe,
            0x08,
            0x00,
            0x12,
            0xbb,
            0x02,
            app,
            (v >> 16) as u8,
            (v >> 8) as u8,
            v as u8,
        ]);
    }
    f.extend_from_slice(&[0, 0]);
    f
}

/// The fake switch: answers the phone's first LLDPDU with its own (so the
/// phone must transmit before it learns anything), and records what the
/// phone advertises.
async fn lldp_switch(ifindex: u32, heard: Arc<Mutex<Vec<lldp::Neighbor>>>) {
    let sock = PacketSocket::new(ifindex, 0x88cc).unwrap();
    sock.add_multicast(lldp::LLDP_MULTICAST).unwrap();
    let mut buf = [0u8; 1600];
    loop {
        // The port goes down and up during the link-flap step.
        let Ok((n, _)) = sock.recv(&mut buf).await else {
            sleep(Duration::from_millis(50)).await;
            continue;
        };
        if let Some(nb) = lldp::decode(&buf[..n]) {
            heard.lock().unwrap().push(nb);
            let _ = sock.send(lldp::LLDP_MULTICAST, &switch_lldpdu(100)).await;
        }
    }
}

/// Send one frame on `from` and report whether it shows up on `to`.
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

/// Whether a frame sent from `from` stays off `to`.  A frame that does
/// cross (as in [`frame_crosses`]) is sent after it: once that one is
/// through, the first one would have been as well.
async fn frame_held_back(from: u32, to: u32, ethertype: u16, dst: MacAddr) -> bool {
    let rx = PacketSocket::new(to, ethertype).unwrap();
    PacketSocket::new(from, ethertype).unwrap().send(dst, &[0x5au8; 60]).await.unwrap();
    assert!(
        frame_crosses(from, to, 0x88b5, MacAddr::BROADCAST).await,
        "the frame sent after it did not cross"
    );
    let mut buf = [0u8; 128];
    timeout(Duration::from_millis(50), rx.recv(&mut buf)).await.is_err()
}

async fn wait_for(
    mgr: &NetManager,
    what: &str,
    limit: Duration,
    f: impl Fn(&opendeskphone_net::NetState) -> bool,
) -> opendeskphone_net::NetState {
    let mut rx = mgr.state();
    let r = timeout(limit, async {
        loop {
            if f(&rx.borrow()) {
                return rx.borrow().clone();
            }
            rx.changed().await.unwrap();
        }
    })
    .await;
    match r {
        Ok(s) => s,
        Err(_) => panic!("timed out waiting for {what}; state: {:#?}", mgr.snapshot()),
    }
}

fn test_config() -> NetConfig {
    let mut cfg = NetConfig::new("D60", "3_0_0", MacAddr::parse(PHONE_MAC));
    cfg.ifaces.conduit = None;
    cfg.link_wait = Duration::from_secs(5);
    cfg.lldp_wait = Duration::from_secs(3);
    cfg.ntp.set_clock = false;
    cfg
}

async fn manager_end_to_end() {
    let mut nl = Netlink::new().unwrap();
    // "lan" and "pc" are the phone's ports; "sw" is the switch port the LAN
    // jack is plugged into and "pcpeer" is the PC.
    nl.add_veth("lan", "sw").await.unwrap();
    nl.add_veth("pc", "pcpeer").await.unwrap();
    let sw = idx(&mut nl, "sw").await;
    let pcpeer = idx(&mut nl, "pcpeer").await;
    nl.set_up(sw, true).await.unwrap();
    nl.set_up(pcpeer, true).await.unwrap();
    nl.add_vlan("sw.100", sw, 100, &[], &[]).await.unwrap();
    let sw100 = idx(&mut nl, "sw.100").await;
    nl.set_up(sw100, true).await.unwrap();
    nl.addr_add(sw100, IpAddr::V4(SERVER_IP), 24).await.unwrap();

    let dhcp_log = Arc::new(Mutex::new(ServerLog::default()));
    let heard = Arc::new(Mutex::new(Vec::new()));
    let ntp_hits = Arc::new(Mutex::new(0));
    let servers = [
        tokio::spawn(dhcp_server("sw.100", 6, dhcp_log.clone())),
        tokio::spawn(dns_server()),
        tokio::spawn(time_server(ntp_hits.clone())),
        tokio::spawn(lldp_switch(sw, heard.clone())),
    ];

    // --- DHCP on an LLDP-MED voice VLAN, PC port bridged -----------------
    let mut cfg = test_config();
    cfg.vlan = VlanDiscovery::Lldp;
    let mgr = NetManager::start(cfg.clone()).unwrap();
    let st = wait_for(&mgr, "a DHCP lease", Duration::from_secs(25), |s| {
        s.phase == Phase::Up && s.ipv4.is_some()
    })
    .await;
    assert_eq!(st.interface, "br0.100");
    assert_eq!(
        (st.vlan_id, st.vlan_source),
        (Some(100), Some(VlanSource::Lldp))
    );
    assert!(st.bridged);
    assert!(
        st.lan.present && st.lan.link && st.pc.present && st.pc.link,
        "{st:#?}"
    );
    let v4 = st.ipv4.clone().unwrap();
    assert_eq!(
        (v4.address, v4.prefix_len, v4.gateways.clone()),
        (LEASED_IP, 24, vec![SERVER_IP])
    );
    assert_eq!(v4.dhcp_server, Some(SERVER_IP));
    assert_eq!(st.dns, vec![IpAddr::V4(SERVER_IP)]);
    assert_eq!(st.domain.as_deref(), Some("lab.test"));
    assert_eq!(st.tftp_server.as_deref(), Some("sip:proxy@10.99.0.1:5060"));
    assert_eq!(st.ntp_servers, vec![IpAddr::V4(SERVER_IP)]);
    // Markings from the switch's policy: RTP 5/46, SIP 4/24.
    assert_eq!(
        (
            st.qos.rtp_pcp,
            st.qos.rtp_dscp,
            st.qos.sip_pcp,
            st.qos.sip_dscp
        ),
        (5, 46, 4, 24)
    );
    assert_eq!(st.mac, MacAddr::parse(PHONE_MAC));
    {
        let l = dhcp_log.lock().unwrap();
        assert_eq!(l.vendor_class.as_deref(), Some("digium_D60_3_0_0"));
        assert_eq!(l.param_list, [1, 3, 6, 12, 15, 28, 42, 66]);
        assert_eq!(l.client_id, [1, 0x00, 0x0f, 0xd3, 0x01, 0x02, 0x03]);
    }
    println!("netns: DHCP lease on the LLDP-MED voice VLAN ok");

    // The kernel agrees with the reported state.
    for name in ["lan", "pc", "br0", "br0.100"] {
        let l = nl
            .link_by_name(name)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{name} missing"));
        assert_eq!(l.mac, MacAddr::parse(PHONE_MAC), "{name}");
        assert!(l.is_admin_up(), "{name}");
    }
    let br = idx(&mut nl, "br0").await;
    assert_eq!(
        nl.link_by_name("lan").await.unwrap().unwrap().master,
        Some(br)
    );
    assert_eq!(
        nl.link_by_name("pc").await.unwrap().unwrap().master,
        Some(br)
    );
    let vlan_if = idx(&mut nl, "br0.100").await;
    assert!(
        nl.addrs(Some(false))
            .await
            .unwrap()
            .iter()
            .any(|a| a.index == vlan_if && a.addr == IpAddr::V4(LEASED_IP))
    );
    assert!(
        nl.routes(false)
            .await
            .unwrap()
            .iter()
            .any(|r| r.dst_len == 0
                && r.gateway == Some(IpAddr::V4(SERVER_IP))
                && r.oif == Some(vlan_if))
    );
    let egress = std::fs::read_to_string("/proc/net/vlan/br0.100").unwrap_or_default();
    if !egress.is_empty() {
        assert!(egress.contains("5:5") && egress.contains("3:4"), "{egress}");
    }

    // The resolver follows the lease, including the search domain.
    let r = mgr.resolver();
    assert_eq!(
        r.lookup_ipv4("pbx").await.unwrap(),
        vec![Ipv4Addr::new(10, 99, 0, 7)]
    );
    assert!(matches!(
        r.lookup_ipv4("nothing.lab.test").await,
        Err(dns::Error::NoSuchName)
    ));

    // SNTP ran against the option-42... no: `ntp_prefer_option42` is off, so
    // it resolved the default pool name through the fake DNS server.
    wait_for(&mgr, "time sync", Duration::from_secs(10), |s| {
        s.time_synced
    })
    .await;
    assert!(*ntp_hits.lock().unwrap() >= 1);
    println!("netns: DNS and SNTP ok");

    // What the phone told the switch.
    wait_for(
        &mgr,
        "the switch as an LLDP neighbor",
        Duration::from_secs(5),
        |s| !s.lldp_neighbors.is_empty(),
    )
    .await;
    sleep(Duration::from_millis(300)).await;
    {
        let h = heard.lock().unwrap();
        let first = h.first().unwrap();
        assert_eq!(first.med_device_class, Some(lldp::MED_CLASS_ENDPOINT_III));
        assert_eq!(first.system_name.as_deref(), Some("d60-000FD3010203"));
        assert!(
            first.policies.iter().all(|p| p.unknown),
            "first frame should carry unknown policies"
        );
        let last = h.last().unwrap();
        let voice = last
            .policies
            .iter()
            .find(|p| p.app_type == lldp::APP_VOICE)
            .unwrap();
        assert_eq!(
            (
                voice.unknown,
                voice.tagged,
                voice.vlan_id,
                voice.priority,
                voice.dscp
            ),
            (false, true, 100, 5, 46)
        );
        assert_eq!(last.management_address, Some(LEASED_IP));
    }
    println!("netns: LLDP-MED exchange ok");

    // Untagged traffic crosses between the PC jack and the LAN jack.
    let pc_frame = MacAddr::parse("02:00:00:00:00:77").unwrap();
    assert!(
        frame_crosses(pcpeer, sw, 0x88b5, pc_frame).await,
        "PC to LAN"
    );
    assert!(
        frame_crosses(sw, pcpeer, 0x88b5, MacAddr::BROADCAST).await,
        "LAN to PC"
    );
    // EAPOL does not cross without pass-through.
    let pae = MacAddr::parse("01:80:c2:00:00:03").unwrap();
    assert!(
        frame_held_back(pcpeer, sw, 0x888e, pae).await,
        "EAPOL crossed without pass-through"
    );
    println!("netns: PC-port bridging ok");

    // The six-second lease renews at three seconds, by unicast.
    timeout(Duration::from_secs(10), async {
        while dhcp_log.lock().unwrap().renewals == 0 {
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("no renewal seen");
    assert!(mgr.snapshot().ipv4.is_some());
    println!("netns: DHCP renewal ok");

    // A link flap: the phone confirms its address (INIT-REBOOT) and keeps it.
    nl.set_up(sw, false).await.unwrap();
    wait_for(&mgr, "link down", Duration::from_secs(3), |s| !s.lan.link).await;
    nl.set_up(sw, true).await.unwrap();
    nl.set_up(sw100, true).await.unwrap();
    wait_for(&mgr, "link up", Duration::from_secs(3), |s| s.lan.link).await;
    timeout(Duration::from_secs(10), async {
        while dhcp_log.lock().unwrap().reboots == 0 {
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("no INIT-REBOOT request after the link came back");
    sleep(Duration::from_millis(300)).await;
    assert_eq!(mgr.snapshot().ipv4.map(|v| v.address), Some(LEASED_IP));
    println!("netns: link flap ok");

    // --- Settings that apply in place -------------------------------------
    // Selecting the network profile by the leased address changes the time
    // server and the DSCP values on every boot.  That must not cost a new
    // bring-up: the address stays, and no DHCP exchange happens.
    let exchanges = |l: &ServerLog| (l.discovers, l.requests);
    let before = exchanges(&dhcp_log.lock().unwrap());
    let ntp_before = *ntp_hits.lock().unwrap();
    let mut live = cfg.clone();
    live.ntp.server = Some("pool.ntp.org".into());
    live.ntp.prefer_option42 = true;
    live.qos.rtp_dscp = Some(0);
    live.qos.sip_dscp = Some(0);
    live.ipv4 = Ipv4Config::Dhcp { arp_check: true, requested_ip: Some(LEASED_IP) };
    assert!(!cfg.needs_restart(&live));
    let mut watch = mgr.state();
    watch.borrow_and_update();
    let started = std::time::Instant::now();
    mgr.reconfigure(live.clone()).await;
    timeout(Duration::from_secs(5), async {
        while *ntp_hits.lock().unwrap() == ntp_before {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the new time settings did not trigger a synchronization");
    let live_time = started.elapsed();
    // Every state published since the change still has the address.
    while watch.has_changed().unwrap() {
        let st = watch.borrow_and_update().clone();
        assert_eq!(st.ipv4.as_ref().map(|v| v.address), Some(LEASED_IP), "the address was dropped");
        assert_eq!(st.phase, Phase::Up);
    }
    assert_eq!(exchanges(&dhcp_log.lock().unwrap()), before, "a live change must not restart DHCP");
    assert!(nl.link_by_name("br0.100").await.unwrap().is_some());
    // For comparison: a change that does rearrange the network.
    let mut hard = live.clone();
    hard.eapol_passthrough = true;
    assert!(live.needs_restart(&hard));
    let started = std::time::Instant::now();
    mgr.reconfigure(hard).await;
    wait_for(&mgr, "address loss", Duration::from_secs(5), |s| s.ipv4.is_none()).await;
    wait_for(&mgr, "the address after a restart", Duration::from_secs(25), |s| s.phase == Phase::Up && s.ipv4.is_some()).await;
    println!(
        "netns: live settings change ok ({} ms with the address kept; a full bring-up took {} ms)",
        live_time.as_millis(),
        started.elapsed().as_millis()
    );

    // --- Reconfigure: static address, no VLAN, 802.1X pass-through --------
    let mut cfg2 = test_config();
    cfg2.eapol_passthrough = true;
    cfg2.eapol_logoff = true;
    cfg2.lldp.enabled = false;
    cfg2.link_wait = Duration::from_secs(2);
    cfg2.ipv4 = Ipv4Config::Static {
        address: Ipv4Addr::new(10, 98, 0, 2),
        prefix_len: 24,
        gateways: vec![Ipv4Addr::new(10, 98, 0, 1), Ipv4Addr::new(10, 97, 0, 1)],
        dns: vec![Ipv4Addr::new(10, 98, 0, 53)],
        domain: Some("static.test".into()),
    };
    mgr.reconfigure(cfg2).await;
    let st = wait_for(&mgr, "the static address", Duration::from_secs(10), |s| {
        s.phase == Phase::Up
            && s.ipv4
                .as_ref()
                .is_some_and(|v| v.address == Ipv4Addr::new(10, 98, 0, 2))
    })
    .await;
    assert_eq!(st.interface, "br0");
    assert_eq!(st.vlan_id, None);
    assert_eq!(st.dns, vec![IpAddr::V4(Ipv4Addr::new(10, 98, 0, 53))]);
    assert_eq!(st.domain.as_deref(), Some("static.test"));
    assert!(st.tftp_server.is_none());
    assert!(
        nl.link_by_name("br0.100").await.unwrap().is_none(),
        "the VLAN device should be gone"
    );
    assert_eq!(
        dhcp_log.lock().unwrap().releases,
        0,
        "a reconfiguration does not release the lease"
    );
    let br = idx(&mut nl, "br0").await;
    let routes = nl.routes(false).await.unwrap();
    let defaults: Vec<_> = routes
        .iter()
        .filter(|r| r.dst_len == 0 && r.oif == Some(br))
        .collect();
    assert_eq!(defaults.len(), 2, "{routes:?}");
    // The off-subnet gateway got a host route and the higher metric.
    assert!(
        defaults
            .iter()
            .any(|r| r.gateway == Some(IpAddr::V4(Ipv4Addr::new(10, 97, 0, 1))) && r.metric == 1)
    );
    assert!(
        frame_crosses(pcpeer, sw, 0x888e, pae).await,
        "EAPOL should cross with pass-through"
    );
    assert!(
        frame_crosses(sw, pcpeer, 0x888e, pae).await,
        "EAPOL should cross with pass-through"
    );
    // The PC authenticates through the phone, then is unplugged: the phone
    // logs it off the upstream port.
    {
        use opendeskphone_net::eapol;
        use opendeskphone_net::packet::RawSocket;
        let pc_mac = MacAddr::parse("02:00:00:00:00:77").unwrap();
        let pc_side = RawSocket::new(pcpeer, eapol::ETH_P_PAE).unwrap();
        let upstream = RawSocket::new(sw, eapol::ETH_P_PAE).unwrap();
        let mut start = eapol::logoff_frame(pc_mac);
        start[15] = 1; // EAPOL-Start
        pc_side.send(&start).await.unwrap();
        let mut buf = [0u8; 128];
        let n = timeout(Duration::from_secs(2), upstream.recv(&mut buf)).await.expect("EAPOL-Start did not cross").unwrap();
        assert_eq!(eapol::classify(&buf[..n]), Some(eapol::Seen::Supplicant(pc_mac)));
        wait_for(&mgr, "PC link", Duration::from_secs(3), |s| s.pc.link).await;
        nl.set_up(pcpeer, false).await.unwrap();
        let n = timeout(Duration::from_secs(3), upstream.recv(&mut buf)).await.expect("no EAPOL-Logoff after the PC went away").unwrap();
        assert_eq!(buf[..n], eapol::logoff_frame(pc_mac)[..n]);
        nl.set_up(pcpeer, true).await.unwrap();
        wait_for(&mgr, "PC link again", Duration::from_secs(3), |s| s.pc.link).await;
    }
    println!("netns: static reconfiguration, 802.1X pass-through, and logoff on PC disconnect ok");

    // --- Expansion modules behind the PC jack -----------------------------
    // The module talks on VLAN 3182 at 172.20.x.y; the phone gets `exp`
    // with its own address there, and none of it shows on the LAN jack.
    {
        use opendeskphone_net::packet::RawSocket;
        use opendeskphone_net::topology::{EXPANSION_IF, EXPANSION_VID};
        let phone_addr = Ipv4Addr::new(172, 20, 2, 3);
        let module_addr = Ipv4Addr::new(172, 20, 9, 9);
        nl.add_vlan("module", pcpeer, EXPANSION_VID, &[], &[]).await.unwrap();
        let module_if = idx(&mut nl, "module").await;
        nl.addr_add(module_if, IpAddr::V4(module_addr), 16).await.unwrap();
        nl.set_up(module_if, true).await.unwrap();
        let module = std::net::UdpSocket::bind((module_addr, 10025)).unwrap();
        module.set_nonblocking(true).unwrap();
        opendeskphone_net::qos::bind_to_device(&module, "module").unwrap();
        let module = UdpSocket::from_std(module).unwrap();
        // Everything that crosses the LAN jack with an 802.1Q tag.
        let lan_tap = RawSocket::new(sw, 0x8100).unwrap();

        for bridged in [true, false] {
            let mut c = test_config();
            c.lldp.enabled = false;
            c.link_wait = Duration::from_secs(2);
            c.ipv4 = Ipv4Config::Static { address: Ipv4Addr::new(10, 98, 0, 2), prefix_len: 24, gateways: vec![], dns: vec![], domain: None };
            c.expansion_address = Some(phone_addr);
            c.pc_port = if bridged { opendeskphone_net::PcPort::Bridged } else { opendeskphone_net::PcPort::Disabled };
            mgr.reconfigure(c).await;
            let st = wait_for(&mgr, "the expansion interface", Duration::from_secs(10), |s| s.phase == Phase::Up && s.expansion.is_some() && s.bridged == bridged).await;
            assert_eq!(st.expansion, Some(phone_addr));
            let exp = nl.link_by_name(EXPANSION_IF).await.unwrap().expect("no exp interface");
            assert_eq!(exp.kind.as_deref(), Some("vlan"));
            let parent = idx(&mut nl, if bridged { "br0" } else { "pc" }).await;
            assert_eq!(exp.parent, Some(parent));
            assert!(nl.addrs(Some(false)).await.unwrap().iter().any(|a| a.index == exp.index && a.addr == IpAddr::V4(phone_addr) && a.prefix_len == 16));

            // The phone's side, bound to the interface as the expansion
            // subsystem binds its sockets.
            let phone = std::net::UdpSocket::bind((phone_addr, 10025)).unwrap();
            phone.set_nonblocking(true).unwrap();
            opendeskphone_net::qos::bind_to_device(&phone, EXPANSION_IF).unwrap();
            let phone = UdpSocket::from_std(phone).unwrap();
            let mut buf = [0u8; 64];
            let mut answered = false;
            for _ in 0..10 {
                phone.send_to(b"to the module", (module_addr, 10025)).await.unwrap();
                if let Ok(Ok((n, from))) = timeout(Duration::from_millis(300), module.recv_from(&mut buf)).await {
                    assert_eq!((&buf[..n], from.ip()), (&b"to the module"[..], IpAddr::V4(phone_addr)));
                    module.send_to(b"to the phone", from).await.unwrap();
                    let (n, _) = timeout(Duration::from_secs(1), phone.recv_from(&mut buf)).await.expect("no answer from the module").unwrap();
                    assert_eq!(&buf[..n], b"to the phone");
                    answered = true;
                    break;
                }
            }
            assert!(answered, "the module was not reachable over {EXPANSION_IF} (bridged: {bridged})");
            // Broadcast discovery from the module must not leave by the LAN jack.
            module.set_broadcast(true).unwrap();
            module.send_to(b"discover", (Ipv4Addr::new(172, 20, 255, 255), 10026)).await.unwrap();
            let mut frame = [0u8; 256];
            let leaked = timeout(Duration::from_millis(400), async {
                loop {
                    let n = lan_tap.recv(&mut frame).await.unwrap();
                    if n >= 16 && u16::from_be_bytes([frame[14], frame[15]]) & 0xfff == EXPANSION_VID {
                        return;
                    }
                }
            })
            .await
            .is_ok();
            assert!(!leaked, "expansion VLAN traffic reached the LAN jack (bridged: {bridged})");
        }
        nl.del_link(module_if).await.unwrap();
    }
    println!("netns: expansion-module VLAN ok, bridged and with the PC jack standing alone");

    // --- Reconfigure: manual VLAN, PC port disabled, DHCP -----------------
    let mut cfg3 = test_config();
    cfg3.vlan = VlanDiscovery::Manual(100);
    cfg3.pc_port = opendeskphone_net::PcPort::Disabled;
    cfg3.lldp.enabled = false;
    cfg3.ipv4 = Ipv4Config::Dhcp {
        arp_check: false,
        requested_ip: Some(LEASED_IP),
    };
    mgr.reconfigure(cfg3).await;
    let st = wait_for(&mgr, "a lease on lan.100", Duration::from_secs(15), |s| {
        s.phase == Phase::Up && s.ipv4.as_ref().is_some_and(|v| v.address == LEASED_IP)
    })
    .await;
    assert_eq!(st.interface, "lan.100");
    assert_eq!(
        (st.vlan_id, st.vlan_source, st.bridged),
        (Some(100), Some(VlanSource::Manual), false)
    );
    assert!(
        nl.link_by_name("br0").await.unwrap().is_none(),
        "the bridge should be gone"
    );
    assert!(
        !nl.link_by_name("pc").await.unwrap().unwrap().is_admin_up(),
        "the PC port should be down"
    );
    println!("netns: manual VLAN with the PC port disabled ok");

    // --- Shutdown releases the lease and removes what was set up ----------
    mgr.shutdown().await;
    sleep(Duration::from_millis(200)).await;
    assert_eq!(dhcp_log.lock().unwrap().releases, 1);
    assert!(nl.link_by_name("lan.100").await.unwrap().is_none());
    let lan = idx(&mut nl, "lan").await;
    assert!(
        !nl.addrs(Some(false))
            .await
            .unwrap()
            .iter()
            .any(|a| a.index == lan)
    );
    println!("netns: shutdown ok");
    for s in servers {
        s.abort();
    }
}

/// On a host without bridge or 802.1Q support, at least check that the
/// manager falls back to the bare LAN port and gets a lease there.
async fn manager_without_bridge() {
    let mut nl = Netlink::new().unwrap();
    nl.add_veth("lan", "sw").await.unwrap();
    let sw = idx(&mut nl, "sw").await;
    nl.set_up(sw, true).await.unwrap();
    nl.addr_add(sw, IpAddr::V4(SERVER_IP), 24).await.unwrap();
    let log = Arc::new(Mutex::new(ServerLog::default()));
    let server = tokio::spawn(dhcp_server("sw", 60, log.clone()));
    let mut cfg = test_config();
    cfg.lldp.enabled = false;
    cfg.ntp.enabled = false;
    let mgr = NetManager::start(cfg).unwrap();
    let st = wait_for(&mgr, "a DHCP lease", Duration::from_secs(20), |s| {
        s.ipv4.is_some()
    })
    .await;
    assert_eq!(st.interface, "lan");
    mgr.shutdown().await;
    server.abort();
}
