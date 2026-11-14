//! dphone-net: the network manager on the main role's runtime.
//!
//! The network is the way into the phone, so this module is conservative:
//!
//! - The PC jack is bridged to the LAN jack as the stock settings say
//!   (`pc_port_mode`, `pc_vlan_id`, `pc_qos`, `8021x_passthrough`), as on
//!   stock.  `/nvdata/open/net-no-bridge` holds it off whatever the settings
//!   say (`dphone net bridge off --save` writes the file, `on --save`
//!   removes it); `dphone net bridge on|off` alone changes the running
//!   arrangement only.  The hold exists for bring-up: bridging over the b53
//!   switch had not run on hardware when it became the default.
//! - Whatever configuration is in force, if the phone has no address
//!   [`FALLBACK_SECS`] after it was applied (or after the address was
//!   lost), the manager is switched to the plainest configuration
//!   (standalone `lan`, DHCP, no VLAN, PC jack down).  Once that one has an
//!   address (the network answers again: a cable back in, a DHCP server
//!   back up), the settings are tried once more.  If they go without an
//!   address for the same time again, the plain configuration stays until
//!   told otherwise (`dphone net configured`, a settings change, a restart
//!   of the main role, or a reboot), so settings that are themselves at
//!   fault cannot loop.

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

/// Flag file: keep the PC jack off (not bridged), whatever the settings say.
pub fn bridge_hold_flag(ctx: &Context) -> std::path::PathBuf {
    ctx.paths.open_dir().join("net-no-bridge")
}

/// Where the automatic fallback stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fallback {
    /// The arrangement in force was applied on purpose (or the settings
    /// have had an address since).
    None,
    /// Plain, after the settings got no address.  They are tried once more
    /// when the plain arrangement has an address.
    Waiting,
    /// The settings, tried again after [`Fallback::Waiting`].
    Retrying,
    /// Plain, after the second try failed as well: stays until told
    /// otherwise.
    Final,
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
    start_with_fallback(ctx, config, hub, Duration::from_secs(FALLBACK_SECS))
}

/// [`start`] with the time without an address before the fallback given
/// (host tests use a short one).
pub fn start_with_fallback(
    ctx: &Context,
    config: Option<SharedConfig>,
    hub: &super::system::ConfigHub,
    fallback: Duration,
) -> NetHandle {
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
    tokio::spawn(run(ctx.clone(), config, hub.clone(), mac, rx, handle.clone(), fallback));
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
///
/// Without a readable configuration this is the plain configuration, PC
/// jack down included: there are then no settings to bridge by.
fn configured(ctx: &Context, config: Option<&SharedConfig>, mac: Option<MacAddr>) -> NetConfig {
    let cfg = plain_config(ctx, mac);
    let Some(merged) = config.and_then(|c| c.lock().ok().map(|s| s.config())) else {
        return cfg;
    };
    let new_family = ctx.model.family() == crate::model::Family::New;
    let hold = bridge_hold_flag(ctx).exists();
    map_config(cfg, &merged, new_family, mac, hold, last_address(ctx))
}

/// Map the merged phone configuration onto `cfg` (the plain configuration
/// for this phone).  `hold_bridge_off` is the bring-up hold: the PC jack
/// stays down whatever the settings say.
fn map_config(
    mut cfg: NetConfig,
    merged: &dphone_config::Config,
    new_family: bool,
    mac: Option<MacAddr>,
    hold_bridge_off: bool,
    last_address: Option<Ipv4Addr>,
) -> NetConfig {
    let n = merged.network();
    cfg.lan_port_mode = PortMode::parse(&n.lan_port_mode);
    cfg.pc_port_mode = PortMode::parse(&n.pc_port_mode);
    // Stock's switch always forwards between the jacks; its one way to
    // turn the PC jack off is `pc_port_mode` "off", which the topology
    // honors.
    cfg.pc_port = if hold_bridge_off { PcPort::Disabled } else { PcPort::Bridged };
    cfg.pc_vlan_id = n.pc_vlan_id.filter(|v| (1..=4094).contains(v));
    cfg.pc_qos = n.pc_qos.min(7);
    // Stock, new family only: an expansion module takes the PC VLAN away,
    // and the phone joins the modules' private VLAN.
    if new_family && merged.setting_bool("expansion_enable", false) {
        cfg.pc_vlan_id = None;
        cfg.expansion_address = Some(dphone_hal::expansion::private_address(mac.map(|m| m.0).unwrap_or([0; 6])));
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
        (true, _, _) => Ipv4Config::Dhcp { arp_check: !n.disable_arping, requested_ip: last_address },
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
    // Only its resolver and marking slots are used here.
    handle: NetHandle,
    fallback: Duration,
) {
    let NetHandle { tx, resolver: resolver_slot, qos: qos_slot } = handle;
    // This task must not keep its own command channel open.
    drop(tx);
    let fallback_secs = fallback.as_secs();
    let plain = plain_config(&ctx, mac);
    // Safe mode goes straight to the plain configuration.
    let wanted = if ctx.safe { plain.clone() } else { configured(&ctx, config.as_ref(), mac) };
    if !ctx.safe && bridge_hold_flag(&ctx).exists() {
        warn!("net: {} exists; the PC jack is held off", bridge_hold_flag(&ctx).display());
    }
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
    // The arrangement the settings last gave, which a fallback leaves.
    let mut last_configured = active.clone();
    let mut fb = Fallback::None;
    let mut deadline = (active != plain).then(|| Instant::now() + fallback);
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
                    match fb {
                        Fallback::Waiting if !ctx.safe => {
                            // The network answers: the outage may have
                            // been its fault, not the settings'.
                            active = configured(&ctx, config.as_ref(), mac);
                            last_configured = active.clone();
                            if active == plain {
                                fb = Fallback::None;
                            } else {
                                fb = Fallback::Retrying;
                                info!(
                                    "net: the plain arrangement has an address, so the network answers; trying the settings once more ({})",
                                    describe(&active)
                                );
                                deadline = Some(Instant::now() + fallback);
                                manager.reconfigure(active.clone()).await;
                            }
                        }
                        Fallback::Retrying if active != plain => {
                            info!("net: the settings have an address again ({})", describe(&active));
                            fb = Fallback::None;
                        }
                        _ => {}
                    }
                } else if !up && was_up {
                    warn!("net: address lost");
                    if active != plain {
                        deadline = Some(Instant::now() + fallback);
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
                if fb == Fallback::Retrying {
                    fb = Fallback::Final;
                    warn!(
                        "net: no address after {fallback_secs} s with {} on the second try either; falling back to plain DHCP on lan and staying there until 'dphone net configured', a settings change, or a restart",
                        describe(&active)
                    );
                } else {
                    fb = Fallback::Waiting;
                    warn!(
                        "net: no address after {fallback_secs} s with {}; falling back to plain DHCP on lan (the settings are tried once more when that has an address)",
                        describe(&active)
                    );
                }
                active = plain.clone();
                manager.reconfigure(active.clone()).await;
            }
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else { return };
                match cmd {
                    Cmd::Show(reply) => {
                        let _ = reply.send(show(&manager.snapshot(), &active, &plain, deadline, fb));
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
                        fb = Fallback::None;
                        deadline = None;
                        manager.reconfigure(active.clone()).await;
                        let _ = reply.send("plain configuration applied\n".into());
                    }
                    Cmd::Changed => {
                        let now = configured(&ctx, config.as_ref(), mac);
                        // In a fallback the plain arrangement is in force
                        // on purpose: only settings that differ from the
                        // ones that failed end it.
                        let differs = match fb {
                            Fallback::Waiting | Fallback::Final => now != last_configured,
                            Fallback::None | Fallback::Retrying => now != active,
                        };
                        if differs {
                            if fb != Fallback::None {
                                info!("net: the settings changed; the fallback ends");
                                fb = Fallback::None;
                            }
                            let reasons = active.restart_reasons(&now);
                            let restart = !reasons.is_empty();
                            if restart {
                                info!(
                                    "net: network settings changed ({}); bringing the network up again with {}",
                                    reasons.join(", "),
                                    describe(&now)
                                );
                            } else {
                                info!("net: network settings changed; applying in place: {}", describe(&now));
                            }
                            active = now;
                            last_configured = active.clone();
                            if restart {
                                deadline = (active != plain).then(|| Instant::now() + fallback);
                            }
                            manager.reconfigure(active.clone()).await;
                        }
                    }
                    Cmd::Configured(reply) => {
                        active = configured(&ctx, config.as_ref(), mac);
                        last_configured = active.clone();
                        fb = Fallback::None;
                        deadline = (active != plain).then(|| Instant::now() + fallback);
                        manager.reconfigure(active.clone()).await;
                        let _ = reply.send(format!("applied: {}\n", describe(&active)));
                    }
                    Cmd::Bridge { on, save, reply } => {
                        let mut text = String::new();
                        if save {
                            let flag = bridge_hold_flag(&ctx);
                            let result = if on {
                                match std::fs::remove_file(&flag) {
                                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                                    _ => Ok(()),
                                }
                            } else {
                                crate::paths::write_atomic(&flag, b"1\n", 0o644)
                            };
                            match result {
                                Ok(()) if on => text.push_str(&format!("removed {}: the PC jack follows the settings from now on\n", flag.display())),
                                Ok(()) => text.push_str(&format!("wrote {}: the PC jack stays off until 'dphone net bridge on --save'\n", flag.display())),
                                Err(e) => text.push_str(&format!("could not update {}: {e}\n", flag.display())),
                            }
                        }
                        // The configured arrangement with the PC jack as
                        // asked; from the plain fallback, "on" returns to
                        // the settings.
                        let mut next = if on && active == plain && !ctx.safe {
                            configured(&ctx, config.as_ref(), mac)
                        } else {
                            active.clone()
                        };
                        next.pc_port = if on { PcPort::Bridged } else { PcPort::Disabled };
                        active = next;
                        fb = Fallback::None;
                        deadline = (active != plain).then(|| Instant::now() + fallback);
                        manager.reconfigure(active.clone()).await;
                        text.push_str(&format!(
                            "bridging {}; this session may drop while the network is rearranged{}\n",
                            if on { "on" } else { "off" },
                            if on { format!(" (reverts by itself if there is no address after {fallback_secs} s)") } else { String::new() },
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
    let pc = match (cfg.pc_port, cfg.pc_port_mode) {
        (PcPort::Bridged, PortMode::Off) => "PC jack off (pc_port_mode)",
        (PcPort::Bridged, _) => "PC jack bridged",
        (PcPort::Disabled, _) => "PC jack off",
    };
    format!("{ip}, {vlan}, {pc}")
}

fn show(state: &NetState, active: &NetConfig, plain: &NetConfig, deadline: Option<Instant>, fb: Fallback) -> String {
    let mut t = String::new();
    t.push_str(&format!("config:    {}{}\n", describe(active), if active == plain { " (plain)" } else { "" }));
    if let Some(d) = deadline {
        t.push_str(&format!(
            "fallback:  to plain in {} s unless an address arrives\n",
            d.saturating_duration_since(Instant::now()).as_secs()
        ));
    }
    match fb {
        Fallback::None => {}
        Fallback::Waiting => t.push_str("fallback:  plain after the settings got no address; they are tried once more when this has an address\n"),
        Fallback::Retrying => t.push_str("fallback:  the settings are on their second try\n"),
        Fallback::Final => t.push_str("fallback:  plain after the settings got no address twice; stays until 'dphone net configured', a settings change, or a restart\n"),
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
    match (state.expansion, active.expansion_address) {
        (Some(a), _) => t.push_str(&format!(
            "expansion: {} {a}/{} (VLAN {})\n",
            dphone_net::topology::EXPANSION_IF,
            dphone_net::topology::EXPANSION_PREFIX,
            dphone_net::topology::EXPANSION_VID
        )),
        (None, Some(_)) => t.push_str("expansion: enabled, but the interface is not up\n"),
        (None, None) => {}
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
        n.pc_port_enabled = active.pc_port == PcPort::Bridged && active.pc_port_mode != PortMode::Off;
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

#[cfg(test)]
mod tests {
    use super::*;
    use dphone_config::{ConfigStore, Model, Nvdata};

    /// The layout a D60 gets from its DPMA server: one network profile
    /// selected by address, and settings that only count on that network.
    /// (The ids and the `network_id` attributes are the ones on the
    /// bring-up phone; the addresses are examples.)
    const FETCHED: &str = r#"<config>
<networks>
<network id="lan" display_name="LAN" cidr="10.10.14.0/23" />
</networks>
<setting id="config_server_url" value="sip:proxy@10.10.14.3:5060;transport=udp" network_id="lan" />
<setting id="enable_logging" value="0" network_id="lan" />
<setting id="ntp_server" value="0.pool.ntp.org" network_id="lan" />
<setting id="pc_qos" value="0" network_id="lan" />
<setting id="pc_vlan_id" value="0" network_id="lan" />
<setting id="rtp_dscp" value="0" network_id="lan" />
<setting id="sip_dscp" value="0" network_id="lan" />
<setting id="udp_ka_interval" value="0" network_id="lan" />
<setting id="network_enable_dhcp" value="1" />
<setting id="network_vlan_discovery_mode" value="LLDP" />
<setting id="time_source" value="ntp" />
</config>"#;

    fn store() -> (tempfile::TempDir, ConfigStore) {
        let dir = tempfile::tempdir().unwrap();
        let nv = Nvdata::new(dir.path().join("nvdata"));
        nv.ensure_layout().unwrap();
        let mut store = ConfigStore::open(nv, Model::D60, dir.path().join("tmp")).unwrap();
        store.apply_fetched(FETCHED).unwrap();
        (dir, store)
    }

    fn mapped(store: &ConfigStore) -> NetConfig {
        let mac = MacAddr::parse("00:0f:d3:01:02:03");
        let mut plain = NetConfig::new("D60", "3_0_0", mac);
        plain.pc_port = PcPort::Disabled;
        map_config(plain, &store.config(), true, mac, false, None)
    }

    /// A phone with nothing but the built-in defaults bridges the PC jack,
    /// as stock does, and the stock settings steer it.
    #[test]
    fn pc_jack_is_bridged_by_default_and_follows_the_stock_settings() {
        use dphone_net::topology;
        let dir = tempfile::tempdir().unwrap();
        let nv = Nvdata::new(dir.path().join("nvdata"));
        nv.ensure_layout().unwrap();
        let mut store = ConfigStore::open(nv, Model::D60, dir.path().join("tmp")).unwrap();
        let cfg = mapped(&store);
        assert_eq!(cfg.pc_port, PcPort::Bridged);
        assert_eq!((cfg.pc_port_mode, cfg.pc_vlan_id, cfg.pc_qos), (PortMode::Auto, None, 1));
        assert!(!cfg.eapol_passthrough && !cfg.eapol_logoff);
        let plan = topology::plan(&cfg, true, true, None);
        assert_eq!(plan.bridge.map(|b| (b.vlan_filtering, b.group_fwd_mask)), Some((false, 0)));
        assert!(plan.pc_up);
        assert_eq!(describe(&cfg), "DHCP, VLAN from LLDP, PC jack bridged");

        // 802.1X pass-through, the PC VLAN, and its priority.
        store.set_many(&[("8021x_passthrough", "1"), ("pc_vlan_id", "20"), ("pc_qos", "5")]).unwrap();
        let cfg = mapped(&store);
        assert!(cfg.eapol_passthrough && cfg.eapol_logoff);
        assert_eq!((cfg.pc_vlan_id, cfg.pc_qos), (Some(20), 5));
        let plan = topology::plan(&cfg, true, true, None);
        assert_eq!(
            plan.bridge.map(|b| (b.vlan_filtering, b.group_fwd_mask)),
            Some((true, topology::GROUP_FWD_EAPOL))
        );
        assert_eq!(plan.pc_prio, 5);

        // Stock's own switch for the PC jack.
        store.set("pc_port_mode", "off").unwrap();
        let cfg = mapped(&store);
        let plan = topology::plan(&cfg, true, true, None);
        assert_eq!((plan.bridge, plan.pc_up), (None, false));
        assert_eq!(describe(&cfg), "DHCP, VLAN from LLDP, PC jack off (pc_port_mode)");
        store.set("pc_port_mode", "auto").unwrap();

        // The bring-up hold wins over the settings.
        let mac = MacAddr::parse("00:0f:d3:01:02:03");
        let held = map_config(NetConfig::new("D60", "3_0_0", mac), &store.config(), true, mac, true, None);
        assert_eq!(held.pc_port, PcPort::Disabled);
        assert_eq!(topology::plan(&held, true, true, None).bridge, None);
    }

    /// What the phone does at every boot: bring-up starts before there is
    /// an address, the lease selects the network profile, and the settings
    /// tied to that profile change.  None of that may restart bring-up.
    #[test]
    fn profile_selection_after_the_lease_applies_in_place() {
        let (_dir, mut store) = store();
        let before = mapped(&store);
        let generation = store.config().generation();
        let selected = store.set_ip(Some(Ipv4Addr::new(10, 10, 15, 87))).unwrap();
        assert_ne!(selected.generation(), generation, "the lease should select the profile");
        let after = mapped(&store);
        // The profile's settings took effect...
        assert_eq!(after.ntp.server.as_deref(), Some("0.pool.ntp.org"));
        assert_eq!((after.qos.rtp_dscp, after.qos.sip_dscp), (Some(0), Some(0)));
        // pc_qos is the one that used to force a restart: it changes from
        // the default to the profile's value, with no PC VLAN to apply to.
        assert_eq!((before.pc_qos, after.pc_qos, after.pc_vlan_id), (1, 0, None));
        assert_ne!(before, after);
        // ...and all of them are ones a running session adopts.
        assert_eq!(before.restart_reasons(&after), Vec::<&str>::new());
    }

    #[test]
    fn expansion_enable_asks_for_the_interface() {
        let (_dir, mut store) = store();
        store.set_ip(Some(Ipv4Addr::new(10, 10, 15, 87))).unwrap();
        let before = mapped(&store);
        assert_eq!(before.expansion_address, None);
        store.set("expansion_enable", "1").unwrap();
        let after = mapped(&store);
        assert_eq!(after.expansion_address, Some(Ipv4Addr::new(172, 20, 2, 3)));
        assert_eq!(before.restart_reasons(&after), ["expansion modules"]);
        // Old-family phones have no expansion modules.
        let mac = MacAddr::parse("00:0f:d3:01:02:03");
        let old = map_config(NetConfig::new("D40", "3_0_0", mac), &store.config(), false, mac, false, None);
        assert_eq!(old.expansion_address, None);
    }
}
