//! The network manager: one task that brings the phone's network up and
//! keeps it up.
//!
//! Order of work, following the stock scripts: set the MAC address, bring
//! the switch ports up, wait for carrier, listen for LLDP-MED, decide the
//! voice VLAN, arrange bridge and VLANs, then address the chosen interface
//! by DHCP or statically, and keep DNS and the clock current.

use std::future::pending;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::{Instant, sleep, sleep_until};

use crate::config::{Ipv4Config, Ipv6Config, NetConfig, VlanDiscovery};
use crate::dns::Resolver;
use crate::ethtool::{self, PortMode};
use crate::lldp::{self, LocalInfo};
use crate::netlink::{self, Link, Monitor, Netlink, RTPROT_DHCP, RTPROT_STATIC};
use crate::qos::{QosMarks, SKB_PRIORITY_RTP, SKB_PRIORITY_SIP};
use crate::state::{AddressSource, Ipv4State, NetState, Phase, PortState, VlanSource};
use crate::sys::MacAddr;
use crate::{dhcp4, dhcp6, eapol, mdns, sntp, topology};

const IFA_F_TENTATIVE: u32 = 0x40;
const IFA_F_DEPRECATED: u32 = 0x20;

enum Cmd {
    Reconfigure(Box<NetConfig>),
    SyncTime,
    Shutdown(oneshot::Sender<()>),
}

/// Why a session ended.
enum Next {
    Reconfigure(Box<NetConfig>),
    Shutdown(Option<oneshot::Sender<()>>),
}

/// Handle to the network manager task.
pub struct NetManager {
    state: watch::Receiver<NetState>,
    resolver: Resolver,
    cmd: mpsc::Sender<Cmd>,
    task: tokio::task::JoinHandle<()>,
}

impl NetManager {
    /// Start managing the network.  Must be called inside a tokio runtime,
    /// with the privileges to configure interfaces.
    pub fn start(cfg: NetConfig) -> io::Result<Self> {
        let nl = Netlink::new()?;
        let monitor = Monitor::new()?;
        let (stx, srx) = watch::channel(NetState {
            mac: cfg.mac,
            ..Default::default()
        });
        let (ctx, crx) = mpsc::channel(8);
        let resolver = Resolver::new();
        let runner = Runner {
            cfg,
            state: stx,
            resolver: resolver.clone(),
            nl,
            monitor,
            cmd: crx,
            live_pending: false,
            live_ntp: false,
        };
        let task = tokio::spawn(runner.run());
        Ok(Self {
            state: srx,
            resolver,
            cmd: ctx,
            task,
        })
    }

    /// A receiver for the current state; `changed()` fires on every update.
    pub fn state(&self) -> watch::Receiver<NetState> {
        self.state.clone()
    }

    /// A snapshot of the current state.
    pub fn snapshot(&self) -> NetState {
        self.state.borrow().clone()
    }

    /// The resolver, kept pointed at the current DNS servers.
    pub fn resolver(&self) -> Resolver {
        self.resolver.clone()
    }

    /// Apply a new configuration.  A configuration equal to the current
    /// one is ignored; anything else restarts bring-up.
    pub async fn reconfigure(&self, cfg: NetConfig) {
        let _ = self.cmd.send(Cmd::Reconfigure(Box::new(cfg))).await;
    }

    /// Ask for a time synchronization now.
    pub async fn sync_time(&self) {
        let _ = self.cmd.send(Cmd::SyncTime).await;
    }

    /// Wait until an address is configured, or `limit` passes.
    pub async fn wait_up(&self, limit: Duration) -> bool {
        let mut rx = self.state.clone();
        let deadline = Instant::now() + limit;
        loop {
            if rx.borrow().has_address() {
                return true;
            }
            match tokio::time::timeout_at(deadline, rx.changed()).await {
                Ok(Ok(())) => {}
                _ => return rx.borrow().has_address(),
            }
        }
    }

    /// Look for DPMA provisioning servers announced over mDNS.
    pub async fn browse_digium_proxy(&self, window: Duration) -> io::Result<Vec<mdns::Service>> {
        let (ifname, addr) = {
            let s = self.state.borrow();
            (s.interface.clone(), s.ipv4.as_ref().map(|v| v.address))
        };
        let addr =
            addr.ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "no IPv4 address"))?;
        mdns::browse(&ifname, addr, mdns::DIGIUM_PROXY_SERVICE, window).await
    }

    /// Release the DHCP lease, remove what was set up, and stop.
    pub async fn shutdown(self) {
        let (tx, rx) = oneshot::channel();
        if self.cmd.send(Cmd::Shutdown(tx)).await.is_ok() {
            let _ = tokio::time::timeout(Duration::from_secs(5), rx).await;
        }
        self.task.abort();
    }
}

impl Drop for NetManager {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Markings from configuration, with the stock fallbacks for missing or
/// out-of-range values, overridden by an LLDP-MED policy when there is one.
pub fn effective_qos(cfg: &NetConfig, lldp: Option<&lldp::VoicePolicy>) -> QosMarks {
    let d = QosMarks::default();
    let pick = |lldp: Option<u8>, conf: Option<u8>, max: u8, default: u8| {
        lldp.or(conf).filter(|v| *v <= max).unwrap_or(default)
    };
    QosMarks {
        rtp_pcp: pick(lldp.and_then(|p| p.rtp_pcp), cfg.qos.rtp_pcp, 7, d.rtp_pcp),
        rtp_dscp: pick(
            lldp.and_then(|p| p.rtp_dscp),
            cfg.qos.rtp_dscp,
            63,
            d.rtp_dscp,
        ),
        sip_pcp: pick(lldp.and_then(|p| p.sip_pcp), cfg.qos.sip_pcp, 7, d.sip_pcp),
        sip_dscp: pick(
            lldp.and_then(|p| p.sip_dscp),
            cfg.qos.sip_dscp,
            63,
            d.sip_dscp,
        ),
    }
}

fn in_subnet(addr: Ipv4Addr, prefix: u8, other: Ipv4Addr) -> bool {
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix.min(32) as u32)
    };
    u32::from(addr) & mask == u32::from(other) & mask
}

fn write_sysctl(path: &str, value: &str) {
    if let Err(e) = std::fs::write(path, value) {
        if e.kind() != io::ErrorKind::NotFound {
            log::debug!("net: writing {path}: {e}");
        }
    }
}

fn ipv6_sysctl(ifname: &str, key: &str, value: &str) {
    write_sysctl(&format!("/proc/sys/net/ipv6/conf/{ifname}/{key}"), value);
}

async fn recv_opt<T>(rx: Option<&mut mpsc::Receiver<T>>) -> Option<T> {
    match rx {
        Some(r) => r.recv().await,
        None => pending().await,
    }
}

/// Live pieces of one bring-up.
struct Session {
    lan: u32,
    pc: Option<u32>,
    ip_if: String,
    ip_index: u32,
    dhcp4: Option<dhcp4::Client>,
    dhcp6: Option<dhcp6::Client>,
    lldp: Option<lldp::Agent>,
    lease4: Option<dhcp4::Lease>,
    lease6: Option<dhcp6::Lease>,
    rdnss: Vec<Ipv6Addr>,
    static_dns: Vec<IpAddr>,
    static_domain: Option<String>,
    lan_link: bool,
    sntp_due: Option<Instant>,
    sntp_running: bool,
    local_info: Option<LocalInfo>,
    pc_link: bool,
    logoff: Option<eapol::LogoffProxy>,
    /// The voice VLAN in use, if any.
    voice_vid: Option<u16>,
    /// The LLDP-MED policy the markings came from, if any.
    voice_policy: Option<lldp::VoicePolicy>,
}

struct Runner {
    cfg: NetConfig,
    state: watch::Sender<NetState>,
    resolver: Resolver,
    nl: Netlink,
    monitor: Monitor,
    cmd: mpsc::Receiver<Cmd>,
    /// `cfg` was replaced by one that differs only in settings that apply
    /// to a running session; the session has yet to pick them up.
    live_pending: bool,
    /// The pending live change includes the time settings.
    live_ntp: bool,
}

impl Runner {
    fn update(&self, f: impl FnOnce(&mut NetState)) {
        self.state.send_if_modified(|s| {
            let old = s.clone();
            f(s);
            *s != old
        });
    }

    fn fail(&self, what: &str, e: &io::Error) {
        log::warn!("net: {what}: {e}");
        let msg = format!("{what}: {e}");
        self.update(|s| s.last_error = Some(msg));
    }

    async fn run(mut self) {
        loop {
            let (next, session) = match self.bring_up().await {
                Ok(mut session) => {
                    let next = self.serve(&mut session).await;
                    (next, Some(session))
                }
                Err(next) => (next, None),
            };
            self.tear_down(session, matches!(next, Next::Shutdown(_)))
                .await;
            match next {
                Next::Reconfigure(cfg) => self.cfg = *cfg,
                Next::Shutdown(ack) => {
                    if let Some(ack) = ack {
                        let _ = ack.send(());
                    }
                    return;
                }
            }
        }
    }

    /// Turn a command into the reason to stop the current session, if it is one.
    ///
    /// A new configuration that differs only in settings a running session
    /// can adopt (markings, time, resolver order) does not interrupt: it
    /// replaces `cfg` and is applied by [`Runner::apply_live`].  This
    /// matters at every boot, because selecting the network profile by the
    /// leased address changes such settings just after the lease arrives.
    fn interrupt(&mut self, c: Option<Cmd>) -> Option<Next> {
        match c {
            Some(Cmd::Reconfigure(cfg)) if *cfg == self.cfg => None,
            Some(Cmd::Reconfigure(cfg)) if !self.cfg.needs_restart(&cfg) => {
                log::info!("net: settings changed that apply without a restart");
                self.live_ntp |= cfg.ntp != self.cfg.ntp;
                self.cfg = *cfg;
                self.live_pending = true;
                None
            }
            Some(Cmd::Reconfigure(cfg)) => Some(Next::Reconfigure(cfg)),
            Some(Cmd::SyncTime) => None,
            Some(Cmd::Shutdown(ack)) => Some(Next::Shutdown(Some(ack))),
            None => Some(Next::Shutdown(None)),
        }
    }

    /// Sleep, unless a command ends the session first.
    async fn pause(&mut self, d: Duration) -> Result<(), Next> {
        let until = Instant::now() + d;
        loop {
            tokio::select! {
                _ = sleep_until(until) => return Ok(()),
                c = self.cmd.recv() => {
                    if let Some(next) = self.interrupt(c) {
                        return Err(next);
                    }
                }
            }
        }
    }

    async fn port_state(&mut self, index: Option<u32>) -> PortState {
        let Some(index) = index else {
            return PortState::default();
        };
        match self.nl.link_by_index(index).await {
            Ok(Some(l)) => Self::port_state_of(&l),
            _ => PortState::default(),
        }
    }

    fn port_state_of(l: &Link) -> PortState {
        let link = l.has_carrier();
        let mode = if link {
            ethtool::link_mode(&l.name).ok()
        } else {
            None
        };
        PortState {
            present: true,
            link,
            speed: mode.and_then(|m| m.speed),
            full_duplex: mode.and_then(|m| m.full_duplex),
        }
    }

    /// Set the phone's MAC address on an interface, taking it down first
    /// when needed.
    async fn set_mac(&mut self, name: &str, mac: MacAddr) {
        let Ok(Some(l)) = self.nl.link_by_name(name).await else {
            return;
        };
        if l.mac == Some(mac) {
            return;
        }
        if l.is_admin_up() {
            let _ = self.nl.set_up(l.index, false).await;
        }
        if let Err(e) = self.nl.set_mac(l.index, mac).await {
            self.fail(&format!("setting the MAC address on {name}"), &e);
        }
    }

    async fn bring_up(&mut self) -> Result<Session, Next> {
        let cfg = self.cfg.clone();
        let names = cfg.ifaces.clone();
        self.update(|s| {
            *s = NetState {
                phase: Phase::WaitingForInterfaces,
                mac: cfg.mac,
                interface: String::new(),
                ..Default::default()
            };
        });

        // The switch driver may still be loading.
        let mut warned = false;
        let lan = loop {
            match self.nl.link_by_name(&names.lan).await {
                Ok(Some(l)) => break l,
                Ok(None) => {
                    if !warned {
                        log::info!("net: waiting for interface {}", names.lan);
                        warned = true;
                    }
                }
                Err(e) => self.fail("reading interfaces", &e),
            }
            self.pause(Duration::from_millis(500)).await?;
        };

        // Remove leftovers from an earlier run before touching addresses.
        topology::teardown(&mut self.nl, &cfg).await;

        if let Ok(Some(lo)) = self.nl.link_by_name("lo").await {
            if !lo.is_admin_up() {
                let _ = self.nl.set_up(lo.index, true).await;
            }
        }

        // The MAC goes on the conduit and on both user ports, before any of
        // them comes up: the user ports copied the conduit's boot-time
        // address when they were created.
        let valid_mac = cfg.mac.filter(|m| m.is_valid_unicast());
        if cfg.mac.is_some() && valid_mac.is_none() {
            log::warn!(
                "net: configured MAC address is not a unicast address; leaving the kernel's"
            );
        }
        if let Some(mac) = valid_mac {
            for name in names
                .conduit
                .iter()
                .chain([&names.lan])
                .chain(names.pc.iter())
            {
                self.set_mac(name, mac).await;
            }
        }
        if let Some(conduit) = &names.conduit {
            if let Ok(Some(l)) = self.nl.link_by_name(conduit).await {
                ipv6_sysctl(conduit, "disable_ipv6", "1");
                if let Err(e) = self.nl.set_up(l.index, true).await {
                    self.fail(&format!("bringing up {conduit}"), &e);
                }
            }
        }
        let pc = match &names.pc {
            Some(p) => self.nl.link_by_name(p).await.ok().flatten(),
            None => None,
        };
        let actual_mac = self
            .nl
            .link_by_index(lan.index)
            .await
            .ok()
            .flatten()
            .and_then(|l| l.mac)
            .or(valid_mac);
        self.update(|s| s.mac = actual_mac);

        // Advertisement limits.  Failures are expected on interfaces
        // without a PHY (the simulator's).
        let lan_mode = if cfg.lan_port_mode == PortMode::Off {
            PortMode::Auto
        } else {
            cfg.lan_port_mode
        };
        match ethtool::set_port_mode(&names.lan, lan_mode) {
            Ok(false) => log::warn!(
                "net: {} does not support the configured port mode; using auto",
                names.lan
            ),
            Ok(true) => {}
            Err(e) => log::debug!("net: port mode on {}: {e}", names.lan),
        }
        if let (Some(p), true) = (&pc, cfg.pc_port_mode != PortMode::Off) {
            match ethtool::set_port_mode(&p.name, cfg.pc_port_mode) {
                Ok(false) => log::warn!(
                    "net: {} does not support the configured port mode; using auto",
                    p.name
                ),
                Ok(true) => {}
                Err(e) => log::debug!("net: port mode on {}: {e}", p.name),
            }
        }

        // First arrangement.  With LLDP discovery the VLAN is not known
        // yet, so the PC port is bridged now and VLANs are added later.
        let known_vid = match cfg.vlan {
            VlanDiscovery::Manual(v) => Some(v),
            _ => None,
        };
        let mut bridge_ok = true;
        let mut plan = topology::plan(&cfg, pc.is_some(), bridge_ok, known_vid);
        if let Err(e) = topology::apply(&mut self.nl, &cfg, &plan, valid_mac).await {
            if plan.bridge.is_some() {
                if topology::is_unsupported(&e) {
                    log::warn!("net: this kernel cannot bridge; the PC port stays down");
                } else {
                    self.fail("setting up the bridge; continuing without it", &e);
                }
                topology::teardown(&mut self.nl, &cfg).await;
                bridge_ok = false;
                plan = topology::plan(&cfg, pc.is_some(), bridge_ok, known_vid);
                if let Err(e) = topology::apply(&mut self.nl, &cfg, &plan, valid_mac).await {
                    self.fail("bringing up the LAN port", &e);
                }
            } else {
                self.fail("bringing up the LAN port", &e);
            }
        }
        let bridged = plan.bridge.is_some();
        self.update(|s| {
            s.bridged = bridged;
            s.phase = Phase::WaitingForLink;
        });

        // Wait for carrier, as stock does, then carry on regardless.
        let deadline = Instant::now() + cfg.link_wait;
        let mut lan_link = false;
        loop {
            let st = self.port_state(Some(lan.index)).await;
            let pc_st = self.port_state(pc.as_ref().map(|p| p.index)).await;
            self.update(|s| {
                s.lan = st;
                s.pc = pc_st;
            });
            if st.link {
                lan_link = true;
                break;
            }
            if Instant::now() >= deadline {
                log::info!(
                    "net: no link on {} after {:?}; continuing",
                    names.lan,
                    cfg.link_wait
                );
                break;
            }
            self.pause(Duration::from_millis(250)).await?;
        }

        // LLDP runs on the LAN port itself: a bridge hands LLDP frames to
        // the port they arrived on, not to the bridge device.
        let mut qos = effective_qos(&cfg, None);
        let mut voice_policy = None;
        let mut voice_vid = known_vid.filter(|v| (1..=4094).contains(v));
        let mut vlan_source = voice_vid.map(|_| VlanSource::Manual);
        let mut local_info = None;
        let mut agent = None;
        if cfg.lldp.enabled {
            let info = self.local_info(
                &cfg,
                lan.index,
                actual_mac.unwrap_or_default(),
                LocalInfo::unknown_policies(),
                None,
            );
            match lldp::Agent::spawn(&names.lan, info.clone()) {
                Ok(a) => {
                    agent = Some(a);
                    local_info = Some(info);
                }
                Err(e) => self.fail("starting LLDP", &e),
            }
        }
        if let (VlanDiscovery::Lldp, Some(a)) = (cfg.vlan, agent.as_mut()) {
            self.update(|s| s.phase = Phase::LldpDiscovery);
            // Stock: five seconds of listening, then up to ten one-second
            // rounds of fast transmission, soliciting on the odd ones.
            let mut found = tokio::select! {
                f = a.wait_for_med_policy(cfg.lldp_wait) => f,
                c = self.cmd.recv() => match self.interrupt(c) {
                    Some(next) => return Err(next),
                    None => lldp::have_med_policy(&a.neighbors.borrow()),
                },
            };
            if !found {
                log::info!("net: no LLDP-MED policy heard; starting active discovery");
                a.fast_tx(true).await;
                for attempt in 1..=10 {
                    if lldp::have_med_policy(&a.neighbors.borrow()) {
                        found = true;
                        break;
                    }
                    if attempt % 2 == 1 {
                        a.update().await;
                    }
                    self.pause(Duration::from_secs(1)).await?;
                }
                a.fast_tx(false).await;
                found = found || lldp::have_med_policy(&a.neighbors.borrow());
            }
            let neighbors = a.neighbors.borrow().clone();
            if found {
                if let Some(policy) = lldp::select_voice_policy(&neighbors) {
                    qos = effective_qos(&cfg, Some(&policy));
                    voice_policy = Some(policy);
                    if (1..=4094).contains(&policy.vlan_id) {
                        voice_vid = Some(policy.vlan_id);
                        vlan_source = Some(VlanSource::Lldp);
                    }
                }
            }
            self.update(|s| s.lldp_neighbors = neighbors);
        }

        // Final arrangement, now that the VLAN is decided.
        if voice_vid != plan.voice_vid {
            plan = topology::plan(&cfg, pc.is_some(), bridge_ok, voice_vid);
            if let Err(e) = topology::apply(&mut self.nl, &cfg, &plan, valid_mac).await {
                self.fail("arranging VLANs on the bridge", &e);
            }
        }
        let mut ip_if = plan.base_if.clone();
        if let Some(vid) = plan.voice_vid {
            let name = topology::vlan_ifname(&plan.base_if, vid);
            let parent = self
                .nl
                .link_by_name(&plan.base_if)
                .await
                .ok()
                .flatten()
                .map(|l| l.index)
                .unwrap_or(lan.index);
            if let Ok(Some(old)) = self.nl.link_by_name(&name).await {
                let _ = self.nl.del_link(old.index).await;
            }
            // Stock maps socket priority 5 to the RTP 802.1p value and 3 to
            // the SIP value, and installs the same pairs as the ingress map.
            let map = [
                (SKB_PRIORITY_RTP, qos.rtp_pcp as u32),
                (SKB_PRIORITY_SIP, qos.sip_pcp as u32),
            ];
            match self.nl.add_vlan(&name, parent, vid, &map, &map).await {
                Ok(()) => ip_if = name,
                Err(e) => {
                    self.fail(
                        &format!("creating voice VLAN {vid}; using the untagged network"),
                        &e,
                    );
                    voice_vid = None;
                    vlan_source = None;
                }
            }
        } else {
            voice_vid = None;
            vlan_source = None;
        }
        let ip_link = loop {
            match self.nl.link_by_name(&ip_if).await {
                Ok(Some(l)) => break l,
                _ => {
                    // Should not happen: the interface was just created or found.
                    self.pause(Duration::from_millis(500)).await?;
                }
            }
        };
        if let Err(e) = self.nl.set_up(ip_link.index, true).await {
            self.fail(&format!("bringing up {ip_if}"), &e);
        }

        // Tell the switch what was decided (stock `lldpcli configure med policy`).
        if let (Some(a), Some(_)) = (agent.as_ref(), local_info.as_ref()) {
            let policies = LocalInfo::decided_policies(
                voice_vid,
                qos.rtp_pcp,
                qos.rtp_dscp,
                qos.sip_pcp,
                qos.sip_dscp,
            );
            let info = self.local_info(
                &cfg,
                lan.index,
                actual_mac.unwrap_or_default(),
                policies,
                None,
            );
            a.set_local(info.clone()).await;
            local_info = Some(info);
        }

        // IPv6 lives only on the IP interface.
        for name in [&names.lan, &names.bridge]
            .into_iter()
            .chain(names.pc.iter())
        {
            if *name != ip_if {
                ipv6_sysctl(name, "disable_ipv6", "1");
            }
        }
        match &cfg.ipv6 {
            Ipv6Config::Disabled => ipv6_sysctl(&ip_if, "disable_ipv6", "1"),
            Ipv6Config::Auto => {
                ipv6_sysctl(&ip_if, "accept_ra", "1");
                ipv6_sysctl(&ip_if, "autoconf", "1");
                ipv6_sysctl(&ip_if, "disable_ipv6", "0");
            }
            Ipv6Config::Static { .. } => {
                ipv6_sysctl(&ip_if, "accept_ra", "0");
                ipv6_sysctl(&ip_if, "autoconf", "0");
                ipv6_sysctl(&ip_if, "disable_ipv6", "0");
            }
        }

        let mut session = Session {
            lan: lan.index,
            pc: pc.as_ref().map(|p| p.index),
            ip_if: ip_if.clone(),
            ip_index: ip_link.index,
            dhcp4: None,
            dhcp6: None,
            lldp: agent,
            lease4: None,
            lease6: None,
            rdnss: Vec::new(),
            static_dns: Vec::new(),
            static_domain: None,
            lan_link,
            sntp_due: None,
            sntp_running: false,
            local_info,
            pc_link: false,
            logoff: None,
            voice_vid,
            voice_policy,
        };
        if let (true, true, true, Some(pc)) = (cfg.eapol_passthrough, cfg.eapol_logoff, plan.bridge.is_some(), pc.as_ref()) {
            match eapol::LogoffProxy::spawn(pc.index, lan.index) {
                Ok(p) => session.logoff = Some(p),
                Err(e) => self.fail("starting the EAPOL logoff watch on the PC jack", &e),
            }
        }
        self.update(|s| {
            s.interface = ip_if.clone();
            s.vlan_id = voice_vid;
            s.vlan_source = vlan_source;
            s.qos = qos;
            s.phase = Phase::AcquiringAddress;
        });

        let _ = self.nl.addr_flush_v4(ip_link.index).await;
        let _ = self
            .nl
            .route_flush(Some(ip_link.index), false, &[RTPROT_DHCP, RTPROT_STATIC])
            .await;
        let dhcp_mac = ip_link.mac.or(actual_mac).unwrap_or_default();
        match &cfg.ipv4 {
            Ipv4Config::Dhcp {
                arp_check,
                requested_ip,
            } => {
                session.dhcp4 = Some(dhcp4::Client::spawn(dhcp4::Config {
                    ifname: ip_if.clone(),
                    mac: dhcp_mac,
                    vendor_class: cfg.vendor_class(),
                    hostname: None,
                    requested_ip: *requested_ip,
                    arp_check: *arp_check,
                    priority: SKB_PRIORITY_SIP,
                }));
            }
            Ipv4Config::Static {
                address,
                prefix_len,
                gateways,
                dns,
                domain,
            } => {
                session.static_dns = dns.iter().map(|d| IpAddr::V4(*d)).collect();
                session.static_domain = domain.clone();
                match self
                    .nl
                    .addr_add(ip_link.index, IpAddr::V4(*address), *prefix_len)
                    .await
                {
                    Ok(()) => {
                        self.install_routes(
                            ip_link.index,
                            *address,
                            *prefix_len,
                            gateways,
                            RTPROT_STATIC,
                        )
                        .await;
                        let v4 = Ipv4State {
                            address: *address,
                            prefix_len: *prefix_len,
                            gateways: gateways.clone(),
                            source: AddressSource::Static,
                            dhcp_server: None,
                            lease_secs: None,
                        };
                        self.update(|s| {
                            s.ipv4 = Some(v4);
                            s.phase = Phase::Up;
                        });
                        session.sntp_due = Some(Instant::now());
                    }
                    Err(e) => self.fail("setting the static address", &e),
                }
            }
        }
        match &cfg.ipv6 {
            Ipv6Config::Disabled => {}
            Ipv6Config::Auto => {
                session.dhcp6 = Some(dhcp6::Client::spawn(dhcp6::Config {
                    ifname: ip_if.clone(),
                    mac: dhcp_mac,
                    user_class: cfg.vendor_class(),
                }));
            }
            Ipv6Config::Static {
                address,
                prefix_len,
                gateway,
            } => {
                match self
                    .nl
                    .addr_add(ip_link.index, IpAddr::V6(*address), *prefix_len)
                    .await
                {
                    Ok(()) => {
                        if let Some(gw) = gateway {
                            if let Err(e) = self
                                .nl
                                .route_add_default(
                                    ip_link.index,
                                    IpAddr::V6(*gw),
                                    1024,
                                    RTPROT_STATIC,
                                )
                                .await
                            {
                                self.fail("setting the IPv6 default route", &e);
                            }
                        }
                    }
                    Err(e) => self.fail("setting the static IPv6 address", &e),
                }
            }
        }
        self.refresh_dns(&session);
        Ok(session)
    }

    #[allow(clippy::too_many_arguments)]
    fn local_info(
        &self,
        cfg: &NetConfig,
        ifindex: u32,
        mac: MacAddr,
        policies: Vec<lldp::NetworkPolicy>,
        management_address: Option<Ipv4Addr>,
    ) -> LocalInfo {
        LocalInfo {
            mac,
            port_description: cfg.ifaces.lan.clone(),
            system_name: cfg.hostname.clone(),
            system_description: cfg.system_description(),
            management_address,
            ifindex,
            power_mw: cfg.lldp.power_mw,
            policies,
            hardware_rev: cfg.lldp.hardware_rev.clone(),
            firmware_rev: cfg.version.clone(),
            software_rev: cfg.version.clone(),
            serial: cfg.lldp.serial.clone(),
            manufacturer: cfg.lldp.manufacturer.clone(),
            model: cfg.model.clone(),
            tx_interval: 30,
            tx_hold: 4,
        }
    }

    /// Default routes through `gateways`, with increasing metrics, as the
    /// stock scripts install them.
    async fn install_routes(
        &mut self,
        index: u32,
        address: Ipv4Addr,
        prefix: u8,
        gateways: &[Ipv4Addr],
        protocol: u8,
    ) {
        let _ = self.nl.route_flush(Some(index), false, &[protocol]).await;
        for (i, gw) in gateways.iter().enumerate() {
            if !in_subnet(address, prefix, *gw) {
                if let Err(e) = self
                    .nl
                    .route_add_onlink(index, IpAddr::V4(*gw), 32, protocol)
                    .await
                {
                    self.fail(&format!("adding a route to gateway {gw}"), &e);
                }
            }
            if let Err(e) = self
                .nl
                .route_add_default(index, IpAddr::V4(*gw), i as u32, protocol)
                .await
            {
                self.fail(&format!("adding the default route through {gw}"), &e);
            }
        }
    }

    /// Recompute the resolver list from every source and publish it.
    fn refresh_dns(&self, s: &Session) {
        let mut v4: Vec<IpAddr> = s.static_dns.clone();
        if let Some(l) = &s.lease4 {
            v4.extend(l.dns.iter().map(|d| IpAddr::V4(*d)));
        }
        let mut v6: Vec<IpAddr> = Vec::new();
        if let Some(l) = &s.lease6 {
            v6.extend(l.dns.iter().map(|d| IpAddr::V6(*d)));
        }
        v6.extend(s.rdnss.iter().map(|d| IpAddr::V6(*d)));
        let mut all: Vec<IpAddr> = Vec::new();
        let ordered = if self.cfg.ipv6_preferred {
            [v6, v4]
        } else {
            [v4, v6]
        };
        for ip in ordered
            .into_iter()
            .flatten()
            .chain(self.cfg.dns_fallback.iter().copied())
        {
            if !all.contains(&ip) {
                all.push(ip);
            }
        }
        let domain = s
            .static_domain
            .clone()
            .or_else(|| s.lease4.as_ref().and_then(|l| l.domain.clone()))
            .or_else(|| s.lease6.as_ref().and_then(|l| l.domains.first().cloned()));
        self.resolver.set_servers(&all);
        self.resolver.set_search_domain(domain.as_deref());
        self.update(|st| {
            st.dns = all;
            st.domain = domain;
        });
    }

    async fn apply_lease4(&mut self, s: &mut Session, lease: dhcp4::Lease) {
        let idx = s.ip_index;
        if let Some(old) = &s.lease4 {
            if old.address != lease.address || old.prefix_len != lease.prefix_len {
                let _ = self
                    .nl
                    .addr_del(idx, IpAddr::V4(old.address), old.prefix_len)
                    .await;
            }
        }
        if let Err(e) = self
            .nl
            .addr_add(idx, IpAddr::V4(lease.address), lease.prefix_len)
            .await
        {
            self.fail("setting the leased address", &e);
            return;
        }
        self.install_routes(
            idx,
            lease.address,
            lease.prefix_len,
            &lease.routers,
            RTPROT_DHCP,
        )
        .await;
        let v4 = Ipv4State {
            address: lease.address,
            prefix_len: lease.prefix_len,
            gateways: lease.routers.clone(),
            source: AddressSource::Dhcp,
            dhcp_server: Some(lease.server),
            lease_secs: Some(lease.lease_secs),
        };
        log::info!(
            "net: {} has {}/{} from {}",
            s.ip_if,
            lease.address,
            lease.prefix_len,
            lease.server
        );
        let first = s.lease4.is_none();
        s.lease4 = Some(lease.clone());
        self.update(|st| {
            st.ipv4 = Some(v4);
            st.ntp_servers = lease.ntp.iter().map(|a| IpAddr::V4(*a)).collect();
            st.tftp_server = lease.tftp_server.clone();
            st.bootfile = lease.bootfile.clone();
            st.dhcp_options = lease.options.clone();
            st.phase = Phase::Up;
            st.last_error = None;
        });
        self.refresh_dns(s);
        if first || s.sntp_due.is_none() {
            s.sntp_due = Some(Instant::now());
        }
        self.advertise_address(s, Some(lease.address)).await;
    }

    async fn advertise_address(&mut self, s: &mut Session, addr: Option<Ipv4Addr>) {
        if let (Some(agent), Some(info)) = (s.lldp.as_ref(), s.local_info.as_mut()) {
            if info.management_address != addr {
                info.management_address = addr;
                agent.set_local(info.clone()).await;
            }
        }
    }

    async fn drop_lease4(&mut self, s: &mut Session) {
        let _ = self.nl.addr_flush_v4(s.ip_index).await;
        let _ = self
            .nl
            .route_flush(Some(s.ip_index), false, &[RTPROT_DHCP])
            .await;
        s.lease4 = None;
        self.update(|st| {
            st.ipv4 = None;
            st.ntp_servers.clear();
            st.tftp_server = None;
            st.bootfile = None;
            st.dhcp_options.clear();
            if !st.has_address() {
                st.phase = Phase::AcquiringAddress;
            }
        });
        self.refresh_dns(s);
        self.advertise_address(s, None).await;
    }

    async fn apply_lease6(&mut self, s: &mut Session, lease: Option<dhcp6::Lease>) {
        let old: Vec<Ipv6Addr> = s
            .lease6
            .as_ref()
            .map(|l| l.addresses.iter().map(|a| a.addr).collect())
            .unwrap_or_default();
        let new: Vec<Ipv6Addr> = lease
            .as_ref()
            .map(|l| l.addresses.iter().map(|a| a.addr).collect())
            .unwrap_or_default();
        for a in old.iter().filter(|a| !new.contains(a)) {
            let _ = self.nl.addr_del(s.ip_index, IpAddr::V6(*a), 128).await;
        }
        for a in new.iter().filter(|a| !old.contains(a)) {
            // DHCPv6 gives no prefix length; on-link prefixes come from
            // router advertisements.
            if let Err(e) = self.nl.addr_add(s.ip_index, IpAddr::V6(*a), 128).await {
                self.fail("setting the DHCPv6 address", &e);
            }
        }
        s.lease6 = lease;
        self.refresh_dns(s);
    }

    async fn on_netlink(&mut self, s: &mut Session, ev: netlink::Event) {
        match ev {
            netlink::Event::Link(l) if l.index == s.lan => {
                let st = Self::port_state_of(&l);
                self.update(|state| state.lan = st);
                if st.link != s.lan_link {
                    s.lan_link = st.link;
                    log::info!(
                        "net: {} link {}",
                        l.name,
                        if st.link { "up" } else { "down" }
                    );
                    if let Some(c) = &s.dhcp4 {
                        c.command(if st.link {
                            dhcp4::Command::LinkUp
                        } else {
                            dhcp4::Command::LinkDown
                        })
                        .await;
                    }
                    if st.link {
                        if let Some(c) = &s.dhcp6 {
                            c.link_up().await;
                        }
                        if let Some(a) = &s.lldp {
                            a.update().await;
                        }
                    }
                }
            }
            netlink::Event::Link(l) if Some(l.index) == s.pc => {
                let st = Self::port_state_of(&l);
                self.update(|state| state.pc = st);
                if s.pc_link && !st.link {
                    if let Some(p) = &s.logoff {
                        p.pc_link_down().await;
                    }
                }
                s.pc_link = st.link;
            }
            netlink::Event::AddrAdded(a) | netlink::Event::AddrRemoved(a)
                if a.index == s.ip_index && a.addr.is_ipv6() =>
            {
                self.refresh_ipv6(s).await;
            }
            netlink::Event::Rdnss {
                index,
                lifetime_secs,
                servers,
            } if index == s.ip_index => {
                // Lifetimes are not tracked beyond withdrawal; a router that
                // stops advertising leaves its servers in the list until the
                // next advertisement or link change.
                if lifetime_secs == 0 {
                    s.rdnss.retain(|a| !servers.contains(a));
                } else {
                    for a in servers {
                        if !s.rdnss.contains(&a) {
                            s.rdnss.push(a);
                        }
                    }
                }
                self.refresh_dns(s);
            }
            netlink::Event::Overrun => {
                let lan = self.port_state(Some(s.lan)).await;
                let pc = self.port_state(s.pc).await;
                self.update(|state| {
                    state.lan = lan;
                    state.pc = pc;
                });
                self.refresh_ipv6(s).await;
            }
            _ => {}
        }
    }

    async fn refresh_ipv6(&mut self, s: &mut Session) {
        let Ok(addrs) = self.nl.addrs(Some(true)).await else {
            return;
        };
        let global: Vec<Ipv6Addr> = addrs
            .into_iter()
            .filter(|a| {
                a.index == s.ip_index
                    && a.scope == 0
                    && a.flags & (IFA_F_TENTATIVE | IFA_F_DEPRECATED) == 0
            })
            .filter_map(|a| {
                if let IpAddr::V6(v) = a.addr {
                    Some(v)
                } else {
                    None
                }
            })
            .collect();
        let had = self.state.borrow().has_address();
        self.update(|st| {
            st.ipv6 = global;
            if st.has_address() {
                st.phase = Phase::Up;
            } else {
                st.phase = Phase::AcquiringAddress;
            }
        });
        if !had && self.state.borrow().has_address() && s.sntp_due.is_none() {
            s.sntp_due = Some(Instant::now());
        }
    }

    /// The steady state: react to the kernel, the DHCP clients, LLDP, the
    /// clock, and commands, until told to stop or reconfigure.
    async fn serve(&mut self, s: &mut Session) -> Next {
        let (sntp_tx, mut sntp_rx) = mpsc::channel::<bool>(1);
        // Link events queued during bring-up are history; start from what
        // the kernel says now.
        while let Ok(Ok(_)) = tokio::time::timeout(Duration::ZERO, self.monitor.next()).await {}
        let lan = self.port_state(Some(s.lan)).await;
        let pc = self.port_state(s.pc).await;
        s.lan_link = lan.link;
        s.pc_link = pc.link;
        self.update(|st| {
            st.lan = lan;
            st.pc = pc;
        });
        if !lan.link {
            if let Some(c) = &s.dhcp4 {
                c.command(dhcp4::Command::LinkDown).await;
            }
        }
        loop {
            if self.live_pending {
                if let Some(next) = self.apply_live(s).await {
                    return next;
                }
            }
            let sntp_wake = async {
                match s.sntp_due {
                    Some(t) if !s.sntp_running && self.cfg.ntp.enabled => sleep_until(t).await,
                    _ => pending().await,
                }
            };
            let lldp_changed = async {
                match s.lldp.as_mut() {
                    Some(a) => a.neighbors.changed().await.is_ok(),
                    None => pending().await,
                }
            };
            tokio::select! {
                ev = self.monitor.next() => match ev {
                    Ok(ev) => self.on_netlink(s, ev).await,
                    Err(e) => {
                        log::warn!("net: link monitor: {e}");
                        sleep(Duration::from_secs(1)).await;
                    }
                },
                ev = recv_opt(s.dhcp4.as_mut().map(|c| &mut c.events)) => match ev {
                    Some(dhcp4::Event::Bound(l)) => self.apply_lease4(s, l).await,
                    Some(dhcp4::Event::Renewed(l)) => s.lease4 = Some(l),
                    Some(dhcp4::Event::Lost) => self.drop_lease4(s).await,
                    Some(dhcp4::Event::Conflict(ip)) => {
                        let msg = format!("address {ip} offered by DHCP is already in use");
                        log::warn!("net: {msg}");
                        self.update(|st| st.last_error = Some(msg));
                    }
                    None => s.dhcp4 = None,
                },
                ev = recv_opt(s.dhcp6.as_mut().map(|c| &mut c.events)) => match ev {
                    Some(dhcp6::Event::Bound(l)) => self.apply_lease6(s, Some(l)).await,
                    Some(dhcp6::Event::Lost) => self.apply_lease6(s, None).await,
                    None => s.dhcp6 = None,
                },
                ok = lldp_changed => {
                    if ok {
                        let n = s.lldp.as_ref().map(|a| a.neighbors.borrow().clone()).unwrap_or_default();
                        self.update(|st| st.lldp_neighbors = n);
                    } else {
                        s.lldp = None;
                    }
                }
                _ = sntp_wake => {
                    s.sntp_running = true;
                    let (prefer, servers, configured) = {
                        let st = self.state.borrow();
                        (self.cfg.ntp.prefer_option42, st.ntp_servers.clone(), self.cfg.ntp.server.clone())
                    };
                    let hosts = sntp::server_list(prefer, &servers, configured.as_deref());
                    let resolver = self.resolver.clone();
                    let set_clock = self.cfg.ntp.set_clock;
                    let tx = sntp_tx.clone();
                    tokio::spawn(async move {
                        let mut ok = false;
                        for h in hosts {
                            match sntp::sync_once(&resolver, &h, set_clock).await {
                                Ok(sample) => {
                                    log::info!("net: time from {h}: offset {} ms", sample.offset_ns / 1_000_000);
                                    ok = true;
                                    break;
                                }
                                Err(e) => log::info!("net: time server {h}: {e}"),
                            }
                        }
                        let _ = tx.send(ok).await;
                    });
                }
                r = sntp_rx.recv() => {
                    s.sntp_running = false;
                    let ok = r.unwrap_or(false);
                    let resync = if self.cfg.ntp.resync.is_zero() { sntp::DEFAULT_RESYNC } else { self.cfg.ntp.resync };
                    s.sntp_due = Some(Instant::now() + if ok { resync } else { sntp::RETRY_INTERVAL });
                    if ok {
                        self.update(|st| st.time_synced = true);
                    }
                }
                c = self.cmd.recv() => {
                    if let Some(Cmd::SyncTime) = &c {
                        s.sntp_due = Some(Instant::now());
                    }
                    if let Some(next) = self.interrupt(c) {
                        return next;
                    }
                }
            }
        }
    }

    /// Adopt a configuration change that needs no new bring-up.  Returns
    /// the reason to restart after all in the one case that cannot be done
    /// in place: new 802.1p values while a voice VLAN device carries the
    /// old ones in its priority map.
    async fn apply_live(&mut self, s: &mut Session) -> Option<Next> {
        self.live_pending = false;
        let qos = effective_qos(&self.cfg, s.voice_policy.as_ref());
        let old = self.state.borrow().qos;
        if qos != old {
            if s.voice_vid.is_some() && (qos.rtp_pcp != old.rtp_pcp || qos.sip_pcp != old.sip_pcp) {
                return Some(Next::Reconfigure(Box::new(self.cfg.clone())));
            }
            self.update(|st| st.qos = qos);
            if let (Some(agent), Some(info)) = (s.lldp.as_ref(), s.local_info.as_mut()) {
                info.policies =
                    LocalInfo::decided_policies(s.voice_vid, qos.rtp_pcp, qos.rtp_dscp, qos.sip_pcp, qos.sip_dscp);
                agent.set_local(info.clone()).await;
            }
        }
        self.refresh_dns(s);
        if std::mem::take(&mut self.live_ntp) && self.state.borrow().has_address() {
            s.sntp_due = Some(Instant::now());
        }
        None
    }

    async fn tear_down(&mut self, session: Option<Session>, release: bool) {
        if let Some(mut s) = session {
            if let Some(c) = s.dhcp4.take() {
                if release {
                    c.release().await
                } else {
                    c.abort()
                }
            }
            drop(s.dhcp6.take());
            drop(s.lldp.take());
            let _ = self.nl.addr_flush_v4(s.ip_index).await;
            let _ = self.nl.addr_flush_v6_global(s.ip_index).await;
            let _ = self
                .nl
                .route_flush(Some(s.ip_index), false, &[RTPROT_DHCP, RTPROT_STATIC])
                .await;
            let _ = self
                .nl
                .route_flush(Some(s.ip_index), true, &[RTPROT_STATIC])
                .await;
        }
        topology::teardown(&mut self.nl, &self.cfg).await;
        self.resolver.set_servers(&[]);
        self.update(|st| {
            st.ipv4 = None;
            st.ipv6.clear();
            st.dns.clear();
            st.phase = Phase::Starting;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::QosConfig;

    #[test]
    fn qos_fallbacks() {
        let mut cfg = NetConfig::new("D60", "3_0_0", None);
        assert_eq!(
            effective_qos(&cfg, None),
            QosMarks {
                rtp_dscp: 46,
                sip_dscp: 26,
                rtp_pcp: 6,
                sip_pcp: 3
            }
        );
        cfg.qos = QosConfig {
            rtp_pcp: Some(9),
            rtp_dscp: Some(34),
            sip_pcp: Some(4),
            sip_dscp: Some(99),
        };
        // Out-of-range values fall back to the stock defaults.
        assert_eq!(
            effective_qos(&cfg, None),
            QosMarks {
                rtp_dscp: 34,
                sip_dscp: 26,
                rtp_pcp: 6,
                sip_pcp: 4
            }
        );
        let policy = lldp::VoicePolicy {
            vlan_id: 100,
            rtp_pcp: Some(5),
            rtp_dscp: Some(46),
            sip_pcp: None,
            sip_dscp: None,
        };
        assert_eq!(
            effective_qos(&cfg, Some(&policy)),
            QosMarks {
                rtp_dscp: 46,
                sip_dscp: 26,
                rtp_pcp: 5,
                sip_pcp: 4
            }
        );
    }

    #[test]
    fn subnet_membership() {
        let a = Ipv4Addr::new(10, 10, 15, 87);
        assert!(in_subnet(a, 24, Ipv4Addr::new(10, 10, 15, 1)));
        assert!(!in_subnet(a, 24, Ipv4Addr::new(10, 10, 14, 1)));
        assert!(in_subnet(a, 0, Ipv4Addr::new(192, 168, 1, 1)));
        assert!(!in_subnet(a, 32, Ipv4Addr::new(10, 10, 15, 88)));
    }
}
