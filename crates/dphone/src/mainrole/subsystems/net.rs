//! dphone-net: the network manager on the main role's runtime.
//!
//! The network is the way into the phone, so this module is conservative:
//!
//! - The PC jack is **not** bridged by default.  Only a standalone `lan`
//!   port with DHCP has run on hardware; bridging over the b53 switch is
//!   untested.  `dphone net bridge on` tries it at run time, and
//!   `/nvdata/open/net-bridge` (written by `--save`) makes it the default.
//! - Whatever configuration is in force, if the phone has no address
//!   [`FALLBACK_SECS`] after it was applied, the manager is switched to the
//!   plainest configuration (standalone `lan`, DHCP, no VLAN) and stays
//!   there until told otherwise.

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use dphone_net::{
    AddressSource, Ipv4Config, Ipv6Config, MacAddr, NetConfig, NetManager, NetState, PcPort, Phase, PortMode,
    VlanDiscovery,
};
use dphone_state::{HealthStatus, Subsystem};
use log::{error, info, warn};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

use super::SharedConfig;
use crate::diag::Output;
use crate::mainrole::Context;

/// Seconds without an address before falling back to the plain configuration.
pub const FALLBACK_SECS: u64 = 150;

/// Flag file: bridge the PC jack at start-up.
pub fn bridge_flag(ctx: &Context) -> std::path::PathBuf {
    ctx.paths.open_dir().join("net-bridge")
}

enum Cmd {
    /// The merged configuration changed.
    Changed,
    Show(oneshot::Sender<String>),
    Bridge { on: bool, save: bool, reply: oneshot::Sender<String> },
    Plain(oneshot::Sender<String>),
    Configured(oneshot::Sender<String>),
    SyncTime,
    Browse(Duration, oneshot::Sender<Vec<dphone_net::mdns::Service>>),
}

/// Handle for the rest of the main role.
#[derive(Clone)]
pub struct NetHandle {
    tx: mpsc::Sender<Cmd>,
    resolver: Arc<std::sync::Mutex<Option<dphone_net::Resolver>>>,
    qos: Arc<std::sync::Mutex<dphone_net::QosMarks>>,
}

impl NetHandle {
    /// The DSCP and 802.1p values for SIP and RTP sockets in force now
    /// (configuration, overridden by the switch's LLDP-MED policy).  Apply
    /// them with `dphone_net::qos::mark_socket`.
    pub fn qos(&self) -> dphone_net::QosMarks {
        *self.qos.lock().unwrap()
    }

    /// Re-apply the network configuration from the current settings.
    pub fn reconfigure(&self) {
        let (reply, _) = oneshot::channel();
        let _ = self.tx.try_send(Cmd::Configured(reply));
    }

    /// The merged configuration changed: re-run bring-up only if the
    /// effective network configuration differs from the one in use.
    pub fn config_changed(&self) {
        let _ = self.tx.try_send(Cmd::Changed);
    }

    pub fn sync_time(&self) {
        let _ = self.tx.try_send(Cmd::SyncTime);
    }

    /// The resolver that follows the network's DNS servers; `None` until the
    /// manager runs (and always in the simulator).
    pub fn resolver(&self) -> Option<dphone_net::Resolver> {
        self.resolver.lock().unwrap().clone()
    }

    /// Look for DPMA servers announced over mDNS (`_digiumproxy._udp`).
    pub async fn browse_proxies(&self, window: Duration) -> Vec<dphone_net::mdns::Service> {
        let (reply, rx) = oneshot::channel();
        if self.tx.send(Cmd::Browse(window, reply)).await.is_err() {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }
}

pub fn start(ctx: &Context, config: Option<SharedConfig>, hub: &super::system::ConfigHub) -> NetHandle {
    ctx.store.set_health(Subsystem::Net, HealthStatus::Starting);
    let mac = read_mac(ctx, config.as_ref());
    match mac {
        Some(m) => info!("net: MAC {}", m.bare_hex()),
        None => warn!("net: no ENETCFG_MAC in enet.cfg; keeping the kernel's random MAC"),
    }
    ctx.store.update::<dphone_state::System, _>(|s| s.mac = mac.map(|m| m.0));

    let (tx, rx) = mpsc::channel::<Cmd>(8);
    register_diag(ctx, tx.clone());
    let handle = NetHandle {
        tx,
        resolver: Arc::new(std::sync::Mutex::new(None)),
        qos: Arc::new(std::sync::Mutex::new(dphone_net::QosMarks::default())),
    };

    if ctx.sim {
        info!("net: simulator; the host's network is left alone");
        ctx.store.set_health(Subsystem::Net, HealthStatus::Stopped);
        tokio::spawn(sim_task(rx));
        return handle;
    }
    ctx.health.require("net");
    tokio::spawn(run(ctx.clone(), config, hub.clone(), mac, rx, handle.resolver.clone(), handle.qos.clone()));
    handle
}

async fn sim_task(mut rx: mpsc::Receiver<Cmd>) {
    while let Some(cmd) = rx.recv().await {
        let text = "network not managed in the simulator\n".to_string();
        match cmd {
            Cmd::Show(r) | Cmd::Plain(r) | Cmd::Configured(r) => drop(r.send(text)),
            Cmd::Bridge { reply, .. } => drop(reply.send(text)),
            Cmd::SyncTime | Cmd::Changed => {}
            Cmd::Browse(_, reply) => drop(reply.send(Vec::new())),
        }
    }
}

fn read_mac(ctx: &Context, config: Option<&SharedConfig>) -> Option<MacAddr> {
    if let Some(mac) = config
        .and_then(|c| c.lock().ok().and_then(|s| s.nvdata().mac().ok().flatten()))
    {
        return Some(MacAddr(mac));
    }
    // Without the config crate (it failed, or safe mode found it broken),
    // read the one line we need directly.
    let from_file = std::fs::read_to_string(ctx.paths.nvdata.join("etc/enet.cfg"))
        .ok()
        .and_then(|text| dphone_net::parse_enet_cfg_mac(&text));
    // The simulator needs an identity too (DPMA is keyed by MAC).
    from_file.or(ctx.sim.then_some(MacAddr([0x00, 0x0f, 0xd3, 0x00, 0x00, 0x01])))
}

/// The plainest configuration: standalone `lan`, DHCP, no VLAN, PC jack down.
fn plain_config(ctx: &Context, mac: Option<MacAddr>) -> NetConfig {
    let mut cfg = NetConfig::new(ctx.model.name(), &crate::stock_version(), mac);
    cfg.pc_port = PcPort::Disabled;
    // Development and emulator use: `dphone.lan=<interface>` on the kernel
    // command line names a plain Ethernet interface to use in place of the
    // switch (no conduit, no PC jack).
    let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
    if let Some(name) = cmdline.split_ascii_whitespace().find_map(|w| w.strip_prefix("dphone.lan=")) {
        cfg.ifaces.conduit = None;
        cfg.ifaces.pc = None;
        cfg.ifaces.lan = name.to_string();
    }
    cfg
}

/// The configured network: the merged phone configuration mapped onto the
/// manager's input.
fn configured(ctx: &Context, config: Option<&SharedConfig>, mac: Option<MacAddr>, bridged: bool) -> NetConfig {
    let mut cfg = plain_config(ctx, mac);
    let Some(merged) = config.and_then(|c| c.lock().ok().map(|s| s.config())) else {
        return cfg;
    };
    let n = merged.network();
    cfg.lan_port_mode = PortMode::parse(&n.lan_port_mode);
    cfg.pc_port_mode = PortMode::parse(&n.pc_port_mode);
    cfg.pc_port = if bridged { PcPort::Bridged } else { PcPort::Disabled };
    cfg.pc_vlan_id = n.pc_vlan_id.filter(|v| (1..=4094).contains(v));
    cfg.pc_qos = n.pc_qos.min(7);
    // Stock, new family only: an expansion module takes the PC VLAN away.
    if ctx.model.family() == crate::model::Family::New && merged.setting_bool("expansion_enable", false) {
        cfg.pc_vlan_id = None;
    }
    cfg.eapol_passthrough = n.dot1x_passthrough;
    // Stock enables the logoff together with pass-through, and has a
    // setting of its own for it as well.
    cfg.eapol_logoff = n.dot1x_passthrough || n.dot1x_eapol_on_disconnect;
    cfg.vlan = match n.vlan_discovery_mode.to_ascii_uppercase().as_str() {
        "MANUAL" => n.vlan_id.map(VlanDiscovery::Manual).unwrap_or(VlanDiscovery::None),
        "LLDP" => VlanDiscovery::Lldp,
        _ => VlanDiscovery::None,
    };
    cfg.qos.rtp_pcp = Some(n.rtp_qos);
    cfg.qos.rtp_dscp = Some(n.rtp_dscp);
    cfg.qos.sip_pcp = Some(n.sip_qos);
    cfg.qos.sip_dscp = Some(n.sip_dscp);
    cfg.ipv4 = match (n.dhcp, n.static_ip, n.subnet_mask.and_then(dphone_net::config::netmask_to_prefix)) {
        (false, Some(address), Some(prefix_len)) => Ipv4Config::Static {
            address,
            prefix_len,
            gateways: n.gateway.into_iter().collect(),
            dns: n.dns.clone(),
            domain: n.domain.clone(),
        },
        (false, _, _) => {
            warn!("net: static addressing configured without a usable address and mask; using DHCP");
            Ipv4Config::Dhcp { arp_check: !n.disable_arping, requested_ip: None }
        }
        (true, _, _) => Ipv4Config::Dhcp { arp_check: !n.disable_arping, requested_ip: last_address(ctx) },
    };
    cfg.ipv6 = match (n.ipv6_enabled, n.dhcp) {
        (false, _) => Ipv6Config::Disabled,
        (true, true) => Ipv6Config::Auto,
        // Static mode: the configured IPv6 address, if there is one.
        (true, false) => match n.ipv6_static.as_deref().and_then(|a| a.trim().parse().ok()) {
            Some(address) => Ipv6Config::Static {
                address,
                prefix_len: n.ipv6_mask.as_deref().and_then(|m| m.trim().trim_start_matches('/').parse().ok()).filter(|p| *p <= 128).unwrap_or(64),
                gateway: n.ipv6_gateway.as_deref().and_then(|g| g.trim().parse().ok()),
            },
            None => Ipv6Config::Disabled,
        },
    };
    cfg.ipv6_preferred = n.ipv6_preferred;
    let t = merged.time();
    cfg.ntp.enabled = !t.time_source.eq_ignore_ascii_case("manual");
    cfg.ntp.server = Some(t.ntp_server).filter(|s| !s.is_empty());
    cfg.ntp.prefer_option42 = t.ntp_prefer_option42;
    cfg.ntp.resync = Duration::from_secs(t.ntp_resync_secs as u64);
    cfg
}

/// The address stock held last (`/nvdata/etc/ip-addr-eth0`), to ask the
/// DHCP server for the same one.  Read only.
fn last_address(ctx: &Context) -> Option<Ipv4Addr> {
    let text = std::fs::read_to_string(ctx.paths.nvdata.join("etc/ip-addr-eth0")).ok()?;
    text.split(|c: char| !(c.is_ascii_digit() || c == '.')).find_map(|w| w.parse().ok())
}

fn set_hostname(name: &str) {
    // SAFETY: the pointer and length describe `name`'s bytes.
    let rc = unsafe { libc::sethostname(name.as_ptr().cast(), name.len()) };
    if rc != 0 {
        warn!("net: sethostname: {}", std::io::Error::last_os_error());
    }
}

async fn run(
    ctx: Context,
    config: Option<SharedConfig>,
    hub: super::system::ConfigHub,
    mac: Option<MacAddr>,
    mut cmds: mpsc::Receiver<Cmd>,
    resolver_slot: Arc<std::sync::Mutex<Option<dphone_net::Resolver>>>,
    qos_slot: Arc<std::sync::Mutex<dphone_net::QosMarks>>,
) {
    let plain = plain_config(&ctx, mac);
    let bridged_default = bridge_flag(&ctx).exists();
    // Safe mode goes straight to the plain configuration.
    let wanted = if ctx.safe { plain.clone() } else { configured(&ctx, config.as_ref(), mac, bridged_default) };
    set_hostname(&wanted.hostname);
    ctx.store.update::<dphone_state::System, _>(|s| s.hostname = wanted.hostname.clone());

    // Start the manager; retry for as long as it takes.
    let manager = loop {
        match NetManager::start(wanted.clone()) {
            Ok(m) => break Arc::new(m),
            Err(e) => {
                error!("net: cannot start the network manager: {e}; retrying");
                ctx.store.set_health(Subsystem::Net, HealthStatus::Failed(e.to_string()));
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        }
    };
    if wanted.ifaces.conduit.is_none() {
        warn!("net: using interface {} in place of the switch (dphone.lan= on the command line)", wanted.ifaces.lan);
    }
    info!(
        "net: manager started ({}{})",
        describe(&wanted),
        if wanted == plain { "" } else { "; falls back to plain DHCP on lan if no address" }
    );

    *resolver_slot.lock().unwrap() = Some(manager.resolver());
    let mut state_rx = manager.state();
    let mut active = wanted;
    let mut deadline = (active != plain).then(|| Instant::now() + Duration::from_secs(FALLBACK_SECS));
    let mut was_up = false;
    loop {
        tokio::select! {
            changed = state_rx.changed() => {
                if changed.is_err() {
                    error!("net: the network manager task ended");
                    ctx.store.set_health(Subsystem::Net, HealthStatus::Failed("manager task ended".into()));
                    ctx.health.fail("net");
                    return;
                }
                let state = state_rx.borrow_and_update().clone();
                *qos_slot.lock().unwrap() = state.qos;
                publish(&ctx, &state, &active);
                let up = state.has_address();
                // The merge selects the network profile (and with it the
                // accounts) by the phone's address, as stock does.
                if let (Some(config), Some(v4)) = (&config, state.ipv4.as_ref()) {
                    let merged = {
                        let mut c = config.lock().unwrap();
                        let before = c.config().generation();
                        match c.set_ip(Some(v4.address)) {
                            Ok(cfg) if cfg.generation() != before => Some(cfg),
                            Ok(_) => None,
                            Err(e) => {
                                warn!("net: configuration merge for {}: {e}", v4.address);
                                None
                            }
                        }
                    };
                    if let Some(cfg) = merged {
                        info!("net: network profile selected for {}", v4.address);
                        hub.changed(&ctx.store, &cfg);
                    }
                }
                if up && !was_up {
                    let addr = state.ipv4.as_ref().map(|v| format!("{}/{}", v.address, v.prefix_len)).unwrap_or_default();
                    info!("net: up on {} {addr}", state.interface);
                    ctx.health.satisfy("net");
                    ctx.store.set_health(Subsystem::Net, HealthStatus::Ok);
                    deadline = None;
                } else if !up && was_up {
                    warn!("net: address lost");
                    if active != plain {
                        deadline = Some(Instant::now() + Duration::from_secs(FALLBACK_SECS));
                    }
                }
                was_up = up;
            }
            _ = async { tokio::time::sleep_until(deadline.unwrap_or_else(Instant::now)).await }, if deadline.is_some() => {
                deadline = None;
                // A settings change that the manager applied without a new
                // bring-up re-arms the timer while the address stays.
                if manager.snapshot().has_address() {
                    continue;
                }
                warn!("net: no address after {FALLBACK_SECS} s with {}; falling back to plain DHCP on lan", describe(&active));
                active = plain.clone();
                manager.reconfigure(active.clone()).await;
            }
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else { return };
                match cmd {
                    Cmd::Show(reply) => {
                        let _ = reply.send(show(&manager.snapshot(), &active, &plain, deadline));
                    }
                    Cmd::SyncTime => manager.sync_time().await,
                    Cmd::Browse(window, reply) => {
                        // Off this task: the browse takes its whole window.
                        let manager = manager.clone();
                        tokio::spawn(async move {
                            let found = manager.browse_digium_proxy(window).await.unwrap_or_default();
                            let _ = reply.send(found);
                        });
                    }
                    Cmd::Plain(reply) => {
                        active = plain.clone();
                        deadline = None;
                        manager.reconfigure(active.clone()).await;
                        let _ = reply.send("plain configuration applied\n".into());
                    }
                    Cmd::Changed => {
                        let now = configured(&ctx, config.as_ref(), mac, bridge_flag(&ctx).exists());
                        if now != active {
                            let restart = active.needs_restart(&now);
                            info!(
                                "net: network settings changed; {} {}",
                                if restart { "bringing the network up again with" } else { "applying in place:" },
                                describe(&now)
                            );
                            active = now;
                            if restart {
                                deadline = (active != plain).then(|| Instant::now() + Duration::from_secs(FALLBACK_SECS));
                            }
                            manager.reconfigure(active.clone()).await;
                        }
                    }
                    Cmd::Configured(reply) => {
                        active = configured(&ctx, config.as_ref(), mac, bridge_flag(&ctx).exists());
                        deadline = (active != plain).then(|| Instant::now() + Duration::from_secs(FALLBACK_SECS));
                        manager.reconfigure(active.clone()).await;
                        let _ = reply.send(format!("applied: {}\n", describe(&active)));
                    }
                    Cmd::Bridge { on, save, reply } => {
                        let mut text = String::new();
                        if save {
                            let flag = bridge_flag(&ctx);
                            let result = if on { std::fs::write(&flag, b"1\n") } else {
                                match std::fs::remove_file(&flag) {
                                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                                    _ => Ok(()),
                                }
                            };
                            if let Err(e) = result {
                                text.push_str(&format!("could not update {}: {e}\n", flag.display()));
                            }
                        }
                        let mut next = active.clone();
                        next.pc_port = if on { PcPort::Bridged } else { PcPort::Disabled };
                        active = next;
                        deadline = (active != plain).then(|| Instant::now() + Duration::from_secs(FALLBACK_SECS));
                        manager.reconfigure(active.clone()).await;
                        text.push_str(&format!(
                            "bridging {}; this session may drop while the network is rearranged{}\n",
                            if on { "on" } else { "off" },
                            if on { format!(" (reverts by itself if there is no address after {FALLBACK_SECS} s)") } else { String::new() },
                        ));
                        let _ = reply.send(text);
                    }
                }
            }
        }
    }
}

fn describe(cfg: &NetConfig) -> String {
    let ip = match &cfg.ipv4 {
        Ipv4Config::Dhcp { .. } => "DHCP".to_string(),
        Ipv4Config::Static { address, prefix_len, .. } => format!("static {address}/{prefix_len}"),
    };
    let vlan = match cfg.vlan {
        VlanDiscovery::None => "no VLAN".to_string(),
        VlanDiscovery::Manual(id) => format!("VLAN {id}"),
        VlanDiscovery::Lldp => "VLAN from LLDP".to_string(),
    };
    let pc = match cfg.pc_port {
        PcPort::Bridged => "PC jack bridged",
        PcPort::Disabled => "PC jack off",
    };
    format!("{ip}, {vlan}, {pc}")
}

fn show(state: &NetState, active: &NetConfig, plain: &NetConfig, deadline: Option<Instant>) -> String {
    let mut t = String::new();
    t.push_str(&format!("config:    {}{}\n", describe(active), if active == plain { " (plain)" } else { "" }));
    if let Some(d) = deadline {
        t.push_str(&format!(
            "fallback:  to plain in {} s unless an address arrives\n",
            d.saturating_duration_since(Instant::now()).as_secs()
        ));
    }
    t.push_str(&format!("phase:     {}\n", words(&state.phase)));
    t.push_str(&format!("hostname:  {}\n", active.hostname));
    if let Some(m) = state.mac {
        t.push_str(&format!("mac:       {}\n", m.bare_hex()));
    }
    let port = |p: &dphone_net::PortState| {
        if !p.present {
            "absent".to_string()
        } else if !p.link {
            "no link".to_string()
        } else {
            format!(
                "{} Mb/s {}",
                p.speed.map(|s| s.to_string()).unwrap_or_else(|| "?".into()),
                match p.full_duplex {
                    Some(true) => "full",
                    Some(false) => "half",
                    None => "?",
                }
            )
        }
    };
    t.push_str(&format!("lan:       {}\n", port(&state.lan)));
    t.push_str(&format!("pc:        {}\n", port(&state.pc)));
    t.push_str(&format!("interface: {}{}\n", state.interface, if state.bridged { " (bridged)" } else { "" }));
    if let Some(v) = state.vlan_id {
        t.push_str(&format!("vlan:      {v} ({})\n", words(&state.vlan_source)));
    }
    match &state.ipv4 {
        Some(v) => {
            t.push_str(&format!("ipv4:      {}/{} ({})\n", v.address, v.prefix_len, words(&v.source)));
            let gw: Vec<String> = v.gateways.iter().map(|g| g.to_string()).collect();
            t.push_str(&format!("gateway:   {}\n", gw.join(" ")));
            if let Some(s) = v.dhcp_server {
                t.push_str(&format!("dhcp:      server {s}, lease {} s\n", v.lease_secs.unwrap_or(0)));
            }
        }
        None => t.push_str("ipv4:      none\n"),
    }
    for a in &state.ipv6 {
        t.push_str(&format!("ipv6:      {a}\n"));
    }
    let dns: Vec<String> = state.dns.iter().map(|d| d.to_string()).collect();
    t.push_str(&format!("dns:       {}\n", dns.join(" ")));
    if let Some(s) = &state.tftp_server {
        t.push_str(&format!("option 66: {s}\n"));
    }
    t.push_str(&format!("time sync: {}\n", if state.time_synced { "yes" } else { "no" }));
    for n in &state.lldp_neighbors {
        t.push_str(&format!(
            "lldp:      {} {}\n",
            n.system_name.as_deref().unwrap_or("?"),
            n.port_description.as_deref().unwrap_or("")
        ));
    }
    if let Some(e) = &state.last_error {
        t.push_str(&format!("last error: {e}\n"));
    }
    t
}

/// Copy the manager's state into the state store.
fn publish(ctx: &Context, state: &NetState, active: &NetConfig) {
    use dphone_state as st;
    let port = |p: &dphone_net::PortState| st::PortLink {
        up: p.present && p.link,
        speed: if p.link { p.speed.unwrap_or(0).min(u16::MAX as u32) as u16 } else { 0 },
        full_duplex: p.link && p.full_duplex.unwrap_or(false),
    };
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    let synced = state.time_synced;
    ctx.store.update::<st::Network, _>(|n| {
        n.phase = match state.phase {
            Phase::Up => st::NetPhase::Up,
            Phase::Starting | Phase::WaitingForInterfaces | Phase::WaitingForLink => st::NetPhase::Down,
            Phase::LldpDiscovery | Phase::AcquiringAddress => st::NetPhase::Configuring,
        };
        n.detail = state.last_error.clone().unwrap_or_default();
        n.lan = port(&state.lan);
        n.pc = port(&state.pc);
        n.pc_port_enabled = active.pc_port == PcPort::Bridged;
        n.vlan.voice_id = state.vlan_id;
        n.vlan.source = match state.vlan_source {
            Some(dphone_net::VlanSource::Manual) => st::VlanSource::Config,
            Some(dphone_net::VlanSource::Lldp) => st::VlanSource::Lldp,
            None => st::VlanSource::None,
        };
        n.vlan.voice_priority = state.qos.rtp_pcp;
        n.vlan.dscp = state.qos.rtp_dscp;
        n.vlan.pc_id = active.pc_vlan_id;
        n.ipv4 = match &state.ipv4 {
            Some(v) => {
                let keep = n.ipv4.address == Some(v.address) && n.ipv4.lease_obtained_mono_ms != 0;
                st::Ipv4State {
                    source: match v.source {
                        AddressSource::Dhcp => st::AddrSource::Dhcp,
                        AddressSource::Static => st::AddrSource::Static,
                    },
                    address: Some(v.address),
                    prefix_len: v.prefix_len,
                    gateway: v.gateways.first().copied(),
                    lease_secs: v.lease_secs.unwrap_or(0),
                    lease_obtained_mono_ms: if keep { n.ipv4.lease_obtained_mono_ms } else { st::clock::mono_ms() },
                    dhcp_server: v.dhcp_server,
                }
            }
            None => st::Ipv4State::default(),
        };
        n.ipv6 = state
            .ipv6
            .iter()
            .map(|a| st::Ipv6Address { address: *a, prefix_len: 64, source: st::AddrSource::Slaac })
            .collect();
        n.dns = state.dns.clone();
        n.domain = state.domain.clone().unwrap_or_default();
        n.dhcp_tftp_server = state.tftp_server.clone().unwrap_or_default();
        n.ntp_servers = state.ntp_servers.iter().map(|a| a.to_string()).collect();
        if synced && n.ntp_synced_mono_ms.is_none() {
            n.ntp_synced_mono_ms = Some(st::clock::mono_ms());
        }
        n.lldp = state.lldp_neighbors.first().map(|l| st::LldpNeighbor {
            chassis_id: hex(&l.chassis_id),
            port_id: hex(&l.port_id),
            system_name: l.system_name.clone().unwrap_or_default(),
            port_description: l.port_description.clone().unwrap_or_default(),
        });
    });
    if synced {
        ctx.store.update::<st::System, _>(|s| s.time_synced = true);
    }
}

fn register_diag(ctx: &Context, tx: mpsc::Sender<Cmd>) {
    ctx.diag.register(
        "net",
        "net [status] | bridge on|off [--save] | plain | configured | sync-time",
        move |args: Vec<String>, out: Output| {
            let tx = tx.clone();
            async move {
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                let (reply_tx, reply_rx) = oneshot::channel();
                let cmd = match args.as_slice() {
                    [] | ["status"] => Cmd::Show(reply_tx),
                    ["bridge", onoff @ ("on" | "off")] => Cmd::Bridge { on: *onoff == "on", save: false, reply: reply_tx },
                    ["bridge", onoff @ ("on" | "off"), "--save"] => {
                        Cmd::Bridge { on: *onoff == "on", save: true, reply: reply_tx }
                    }
                    ["plain"] => Cmd::Plain(reply_tx),
                    ["configured"] => Cmd::Configured(reply_tx),
                    ["sync-time"] => {
                        let _ = tx.send(Cmd::SyncTime).await;
                        out.out("time synchronization requested\n").await;
                        return 0;
                    }
                    _ => {
                        out.err("usage: dphone net [status] | bridge on|off [--save] | plain | configured | sync-time\n").await;
                        return 2;
                    }
                };
                if tx.send(cmd).await.is_err() {
                    out.err("dphone net: the network task is not running\n").await;
                    return 1;
                }
                match tokio::time::timeout(Duration::from_secs(10), reply_rx).await {
                    Ok(Ok(text)) => {
                        out.out(&text).await;
                        0
                    }
                    _ => {
                        out.err("dphone net: no answer from the network task (still starting?)\n").await;
                        1
                    }
                }
            }
        },
    );
}

/// A state name for people, from an enum's variant name: "LldpDiscovery"
/// becomes "LLDP discovery", "Dhcp" becomes "DHCP".
fn words(value: &impl std::fmt::Debug) -> String {
    const ACRONYMS: [&str; 6] = ["LLDP", "DHCP", "VLAN", "DNS", "NTP", "IP"];
    let name = format!("{value:?}");
    let mut parts: Vec<String> = Vec::new();
    for c in name.chars() {
        if c.is_ascii_uppercase() || parts.is_empty() {
            parts.push(String::new());
        }
        parts.last_mut().unwrap().push(c);
    }
    let mut out = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        match ACRONYMS.iter().find(|a| a.eq_ignore_ascii_case(part)) {
            Some(a) => out.push_str(a),
            None if i == 0 => out.push_str(part),
            None => out.push_str(&part.to_ascii_lowercase()),
        }
    }
    out
}
