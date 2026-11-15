//! EXP100/EXP150 expansion modules (new family).
//!
//! The wire protocol is in `dphone_hal::expansion`; this is the phone's
//! side of it: discovery, the answers a module asks for, the key
//! configuration, lamp updates, and key presses.
//!
//! **The network.**  Modules sit behind the PC port on a private tagged
//! VLAN (3182).  This subsystem does not build that: it expects an
//! interface named [`INTERFACE`] carrying the VLAN, with the phone's
//! private address (`172.20.<MAC byte 4>.<MAC byte 5>/16`), and it waits
//! for that interface to appear.  Creating it, and keeping the module's
//! three UDP ports (10024 to 10026) off the LAN port as stock's switch
//! rules do, belongs to network bring-up.  In the simulator the loopback
//! interface stands in.
//!
//! **Configuration**, as in stock: `expansion_enable` (with its `type`
//! attribute, `EXP100` or `EXP150`) turns this on, and `blf_item` entries
//! with `location="expansion"` are the keys: `node` picks the module, and
//! `index` the key across its two pages of twenty.
//!
//! **Not done**, because the stock behavior is not recovered: module
//! firmware updates, BLF icons and photos, and the per-state icon number
//! (`BLF_STATUS`; the lamp number is sent).  The stock firmware also sends
//! a password in the configuration (`<PASS_WORD:>`); which setting that is
//! was not established, so it is sent empty.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::os::fd::FromRawFd;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dphone_config::{Config, Nvdata};
use dphone_hal::expansion::{self as proto, KeyConfig, ModuleSettings, Request};
use dphone_state::{
    Accounts, BlfState, Command, Expansion, ExpansionModule, LampState, Presence, Registration, Topic, Ui,
};
use log::{debug, info, warn};
use tokio::net::UdpSocket;
use tokio::sync::watch;

use crate::mainrole::Context;

/// The interface that carries the expansion VLAN.
pub const INTERFACE: &str = "exp";
/// A module that says nothing for this long is gone.
const NODE_TIMEOUT: Duration = Duration::from_secs(30);

/// One configured key.
#[derive(Clone, Debug, PartialEq, Eq)]
struct KeyItem {
    /// BLF entry (contact) ID; empty for an application key.
    entry: String,
    label: String,
    number: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Settings {
    enabled: bool,
    /// Keys by (node, index across both pages).
    keys: HashMap<(u8, usize), KeyItem>,
}

fn settings(config: &Config, nv: &Nvdata, supported: bool) -> Settings {
    let kind = config.setting_attr("expansion_enable", "type").unwrap_or("");
    let enabled = supported && config.setting_bool("expansion_enable", false) && matches!(kind, "EXP100" | "EXP150");
    let mut out = Settings { enabled, keys: HashMap::new() };
    if !enabled {
        return out;
    }
    let mut contacts = Vec::new();
    for name in nv.contact_list_names().unwrap_or_default() {
        if let Ok(Some(bytes)) = nv.contact_list(&name)
            && let Ok(text) = String::from_utf8(bytes)
            && let Ok(groups) = dphone_config::contacts::parse_contacts(&text)
        {
            contacts.extend(groups.into_iter().flat_map(|g| g.contacts));
        }
    }
    for item in dphone_config::ConfigStore::blf_items_from(nv, config) {
        if item.location != "expansion" || item.blank {
            continue;
        }
        let node: u8 = item.raw.attr("node").and_then(|v| v.parse().ok()).unwrap_or(0);
        let index = item.index as usize;
        if node as usize >= proto::MAX_NODES || index >= proto::PAGES * proto::KEYS_PER_PAGE {
            warn!("expansion: key index {index} on node {node} does not exist");
            continue;
        }
        let key = match (&item.contact_id, &item.app_id) {
            (Some(id), _) => match contacts.iter().find(|c| &c.id == id) {
                Some(c) => KeyItem {
                    entry: id.clone(),
                    label: format!("{} {}", c.first_name, c.last_name).trim().to_string(),
                    number: c.primary_number().map(|n| n.dial.clone()).unwrap_or_default(),
                },
                None => continue,
            },
            (None, Some(app)) => KeyItem { entry: String::new(), label: app.clone(), number: String::new() },
            (None, None) => continue,
        };
        out.keys.insert((node, index), key);
    }
    out
}

/// The lamp a contact's state shows on a module key: the same default
/// behavior as on the phone's own keys.
fn lamp(state: BlfState) -> LampState {
    state.lamp()
}

fn socket(bind: SocketAddrV4, device: &str, broadcast: bool) -> std::io::Result<UdpSocket> {
    // SAFETY: plain socket calls; every pointer is to a live local value of
    // the stated size, and the descriptor is owned by the returned socket
    // (or closed on error).
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK, 0);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let sock = std::net::UdpSocket::from_raw_fd(fd);
        let one: libc::c_int = 1;
        let opt = |level, name, value: *const libc::c_void, len: usize| {
            if libc::setsockopt(fd, level, name, value, len as libc::socklen_t) < 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
        };
        opt(libc::SOL_SOCKET, libc::SO_REUSEADDR, (&raw const one).cast(), size_of::<libc::c_int>())?;
        if broadcast {
            opt(libc::SOL_SOCKET, libc::SO_BROADCAST, (&raw const one).cast(), size_of::<libc::c_int>())?;
        }
        opt(libc::SOL_SOCKET, libc::SO_BINDTODEVICE, device.as_ptr().cast(), device.len())?;
        let addr = libc::sockaddr_in {
            sin_family: libc::AF_INET as libc::sa_family_t,
            sin_port: bind.port().to_be(),
            sin_addr: libc::in_addr { s_addr: u32::from(*bind.ip()).to_be() },
            sin_zero: [0; 8],
        };
        if libc::bind(fd, (&raw const addr).cast(), size_of::<libc::sockaddr_in>() as libc::socklen_t) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        UdpSocket::from_std(sock)
    }
}

struct Sockets {
    discovery: UdpSocket,
    announce: UdpSocket,
    node: UdpSocket,
}

fn open(device: &str, local: Ipv4Addr) -> std::io::Result<Sockets> {
    let discovery = socket(SocketAddrV4::new(proto::DISCOVERY_GROUP, proto::DISCOVERY_PORT), device, false)?;
    discovery.join_multicast_v4(proto::DISCOVERY_GROUP, local)?;
    let announce = socket(SocketAddrV4::new(proto::ANNOUNCE_GROUP, proto::ANNOUNCE_PORT), device, true)?;
    let node = socket(SocketAddrV4::new(local, proto::NODE_PORT), device, false)?;
    Ok(Sockets { discovery, announce, node })
}

struct Node {
    addr: Ipv4Addr,
    version: String,
    page: u8,
    last_heard: Instant,
    /// The module has stored a configuration from us.
    configured: bool,
    /// Lamp numbers last sent, by key index across both pages.
    leds: HashMap<usize, u8>,
}

struct Service {
    ctx: Context,
    settings: Settings,
    nodes: Vec<Option<Node>>,
    events: u32,
}

impl Service {
    fn event_id(&mut self) -> u32 {
        self.events = self.events.wrapping_add(1);
        self.events
    }

    fn publish(&self, error: &str) {
        let modules = self
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| n.as_ref().map(|n| ExpansionModule { node: i as u8, addr: n.addr.to_string(), version: n.version.clone(), page: n.page }))
            .collect();
        let enabled = self.settings.enabled;
        self.ctx.store.update::<Expansion, _>(|e| {
            e.enabled = enabled;
            e.modules = modules;
            e.error = error.to_string();
        });
    }

    fn node_of(&self, addr: Ipv4Addr) -> Option<u8> {
        self.nodes.iter().position(|n| n.as_ref().is_some_and(|n| n.addr == addr)).map(|i| i as u8)
    }

    /// Discovery: find or allocate the node and tell it its number.
    fn on_notify(&mut self, from: Ipv4Addr, version: String) -> Option<String> {
        let node = match self.node_of(from) {
            Some(n) => n,
            None => {
                let free = self.nodes.iter().position(Option::is_none)?;
                info!("expansion: module {free} at {from}, version '{version}'");
                self.nodes[free] = Some(Node { addr: from, version, page: 0, last_heard: Instant::now(), configured: false, leds: HashMap::new() });
                self.publish("");
                free as u8
            }
        };
        if let Some(n) = self.nodes[node as usize].as_mut() {
            n.last_heard = Instant::now();
        }
        Some(proto::notify_ack(node))
    }

    fn key_pages(&self, node: u8) -> Vec<Vec<KeyConfig>> {
        (0..proto::PAGES)
            .map(|page| {
                (0..proto::KEYS_PER_PAGE)
                    .map(|key| match self.settings.keys.get(&(node, page * proto::KEYS_PER_PAGE + key)) {
                        Some(item) => KeyConfig { dss_type: proto::DSS_KEY, value: item.number.clone(), label: item.label.clone(), bg: None, fg: None },
                        None => KeyConfig::default(),
                    })
                    .collect()
            })
            .collect()
    }

    fn module_settings(&self) -> ModuleSettings {
        ModuleSettings { password: String::new(), backlight_level: self.ctx.store.read::<Ui, _>(|u| u.backlight_level), key_dots: true }
    }

    /// A message from a module; returns what to send back to it.
    fn on_message(&mut self, node: u8, request: Request) -> Vec<String> {
        if let Some(n) = self.nodes[node as usize].as_mut() {
            n.last_heard = Instant::now();
        }
        match request {
            Request::SocketDetect => vec![proto::socket_detect_ack(node)],
            Request::SynAck => vec![proto::syn_ack_ok(node)],
            Request::VoicemailGet => vec![proto::VOICEMAIL_STATUS.to_string()],
            Request::DndGet => vec![proto::DND_STATUS.to_string()],
            Request::ForwardGet => vec![proto::FORWARD_STATUS.to_string()],
            Request::RegisterGet => {
                let accounts: Vec<u8> = self
                    .ctx
                    .store
                    .read::<Accounts, _>(|a| a.accounts.iter().map(|a| matches!(a.registration, Registration::Registered { .. }) as u8).collect());
                vec![proto::register_status(&accounts)]
            }
            Request::BacklightGet => vec![proto::backlight_status(self.ctx.store.read::<Ui, _>(|u| u.backlight_level))],
            Request::CfgGet => {
                let id = self.event_id();
                vec![proto::config(true, id, node, &self.key_pages(node), &self.module_settings())]
            }
            Request::Version { image, boot, rom } => {
                info!("expansion: module {node} firmware {image}, boot {boot}, ROM {rom}");
                if let Some(n) = self.nodes[node as usize].as_mut() {
                    n.version = image;
                }
                self.publish("");
                Vec::new()
            }
            Request::CfgSaveAck => {
                if let Some(n) = self.nodes[node as usize].as_mut() {
                    n.configured = true;
                    n.leds.clear();
                }
                // As stock: every key's state, then the title.
                let mut out = self.lamp_updates(node);
                out.push(self.title(node));
                out
            }
            // The firmware update is not implemented: nothing newer to offer.
            Request::CheckVersion => vec![proto::check_version_ack(false, node)],
            Request::Key { page, key, long, echo } => {
                self.press(node, page, key, long);
                if long { Vec::new() } else { vec![proto::key_ack(&echo)] }
            }
            Request::Page { page } => {
                let changed = self.nodes[node as usize].as_mut().is_some_and(|n| {
                    let changed = n.configured && n.page != page && (page as usize) < proto::PAGES;
                    if changed {
                        n.page = page;
                    }
                    changed
                });
                if changed {
                    self.publish("");
                    vec![self.title(node)]
                } else {
                    Vec::new()
                }
            }
            Request::BlfPhotoGet | Request::BlfStatusAck => Vec::new(),
            Request::Notify { .. } => Vec::new(),
            Request::Unknown(name) => {
                debug!("expansion: module {node}: unhandled message {name}");
                Vec::new()
            }
        }
    }

    fn title(&self, node: u8) -> String {
        let page = self.nodes[node as usize].as_ref().map_or(0, |n| n.page);
        proto::set_title(&format!("Page {}/{}", page + 1, proto::PAGES))
    }

    /// A key press: what the same key does on the phone's own rapid-dial
    /// keys (dial the contact; pick its call up instead when it rings and
    /// the contact allows that).  A long press does the same.
    fn press(&mut self, node: u8, page: u8, key: u8, long: bool) {
        let index = page as usize * proto::KEYS_PER_PAGE + key as usize;
        let Some(item) = self.settings.keys.get(&(node, index)) else { return };
        info!("expansion: module {node} key {index}{}: {}", if long { " (long)" } else { "" }, item.label);
        if item.entry.is_empty() {
            self.ctx.store.post(Command::LaunchApp { name: item.label.clone() });
            return;
        }
        let entry = self.ctx.store.read::<Presence, _>(|p| p.get(&item.entry).cloned());
        if entry.as_ref().is_some_and(|e| e.pickup_ringing && matches!(e.state, BlfState::Ringing | BlfState::OnHoldRinging)) {
            self.ctx.store.post(Command::Pickup { blf_entry: item.entry.clone() });
        } else if !item.number.is_empty() {
            self.ctx.store.post(Command::Dial { target: item.number.clone(), account: entry.and_then(|e| e.account), path: None });
        }
    }

    /// Lamp messages for every key of `node` whose lamp differs from what
    /// the module was last told.
    fn lamp_updates(&mut self, node: u8) -> Vec<String> {
        let presence = self.ctx.store.get::<Presence>();
        let mut changes = Vec::new();
        for ((n, index), item) in &self.settings.keys {
            if *n != node || item.entry.is_empty() {
                continue;
            }
            let state = presence.get(&item.entry).map_or(BlfState::Unknown, |e| e.state);
            changes.push((*index, proto::led_status(lamp(state))));
        }
        changes.sort();
        let mut out = Vec::new();
        for (index, led) in changes {
            let known = self.nodes[node as usize].as_ref().and_then(|n| n.leds.get(&index).copied());
            if known == Some(led) {
                continue;
            }
            let id = self.event_id();
            if let Some(n) = self.nodes[node as usize].as_mut() {
                n.leds.insert(index, led);
            }
            let (page, key) = ((index / proto::KEYS_PER_PAGE) as u8, (index % proto::KEYS_PER_PAGE) as u8);
            out.push(proto::blf_status(id, node, page, key, proto::BLF_STATUS_DEFAULT, led, None, None));
        }
        out
    }

    /// Drop modules that have gone quiet.
    fn expire(&mut self) {
        let mut changed = false;
        for (i, slot) in self.nodes.iter_mut().enumerate() {
            if slot.as_ref().is_some_and(|n| n.last_heard.elapsed() > NODE_TIMEOUT) {
                info!("expansion: module {i} is gone");
                *slot = None;
                changed = true;
            }
        }
        if changed {
            self.publish("");
        }
    }
}

async fn send(socket: &UdpSocket, to: Ipv4Addr, messages: Vec<String>) {
    for m in messages {
        if let Err(e) = socket.send_to(m.as_bytes(), SocketAddr::from((to, proto::NODE_PORT))).await {
            warn!("expansion: send to {to}: {e}");
        }
    }
}

async fn run(ctx: Context, mut config: watch::Receiver<Settings>) {
    let presence = ctx.store.subscribe(Topic::Presence);
    let mut service = Service { ctx: ctx.clone(), settings: Settings::default(), nodes: (0..proto::MAX_NODES).map(|_| None).collect(), events: 0 };
    loop {
        service.settings = config.borrow_and_update().clone();
        service.nodes.iter_mut().for_each(|n| *n = None);
        if !service.settings.enabled {
            service.publish("");
            if config.changed().await.is_err() {
                return;
            }
            continue;
        }
        // The interface and our address on it.
        let (device, local) = if ctx.sim {
            ("lo", Ipv4Addr::LOCALHOST)
        } else {
            let mac = ctx.store.read::<dphone_state::System, _>(|s| s.mac).unwrap_or([0; 6]);
            (INTERFACE, proto::private_address(mac))
        };
        let sockets = match open(device, local) {
            Ok(s) => s,
            Err(e) => {
                service.publish(&format!("interface {device} with address {local}: {e}"));
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(10)) => {}
                    r = config.changed() => if r.is_err() { return },
                }
                continue;
            }
        };
        info!("expansion: listening on {device} as {local}");
        service.publish("");
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        let mut dbuf = [0u8; proto::MAX_DATAGRAM];
        let mut nbuf = [0u8; proto::MAX_DATAGRAM];
        loop {
            tokio::select! {
                r = sockets.discovery.recv_from(&mut dbuf) => {
                    let Ok((n, SocketAddr::V4(from))) = r else { continue };
                    if let Some(Request::Notify { version }) = proto::parse(&dbuf[..n])
                        && let Some(ack) = service.on_notify(*from.ip(), version)
                    {
                        let to = SocketAddr::from((proto::ANNOUNCE_GROUP, proto::ANNOUNCE_PORT));
                        if let Err(e) = sockets.announce.send_to(ack.as_bytes(), to).await {
                            warn!("expansion: announce: {e}");
                        }
                    }
                }
                r = sockets.node.recv_from(&mut nbuf) => {
                    let Ok((n, SocketAddr::V4(from))) = r else { continue };
                    let Some(node) = service.node_of(*from.ip()) else { continue };
                    if let Some(request) = proto::parse(&nbuf[..n]) {
                        let replies = service.on_message(node, request);
                        send(&sockets.node, *from.ip(), replies).await;
                    }
                }
                _ = presence.changed() => {
                    for node in 0..proto::MAX_NODES as u8 {
                        let Some((addr, true)) = service.nodes[node as usize].as_ref().map(|n| (n.addr, n.configured)) else { continue };
                        let updates = service.lamp_updates(node);
                        send(&sockets.node, addr, updates).await;
                    }
                }
                _ = tick.tick() => service.expire(),
                r = config.changed() => {
                    if r.is_err() {
                        return;
                    }
                    // New keys or switched off: start over (modules ask for
                    // their configuration again when they are rediscovered).
                    break;
                }
            }
        }
    }
}

/// Handle for configuration changes.
#[derive(Clone)]
pub struct ExpansionService {
    settings: Arc<watch::Sender<Settings>>,
    nv: Nvdata,
    supported: bool,
}

impl ExpansionService {
    pub fn apply_config(&self, config: &Config) {
        let new = settings(config, &self.nv, self.supported);
        self.settings.send_if_modified(|s| {
            let changed = *s != new;
            *s = new.clone();
            changed
        });
    }
}

pub fn start(ctx: &Context) -> ExpansionService {
    let (tx, rx) = watch::channel(Settings::default());
    tokio::spawn(run(ctx.clone(), rx));
    let store = ctx.store.clone();
    ctx.diag.register("expansion", "show expansion modules", move |_args, out: crate::diag::Output| {
        let store = store.clone();
        async move {
            let e = store.get::<Expansion>();
            let mut text = format!("enabled: {}\n", e.enabled);
            if !e.error.is_empty() {
                text.push_str(&format!("error: {}\n", e.error));
            }
            for m in &e.modules {
                text.push_str(&format!("module {} at {} version '{}' page {}\n", m.node, m.addr, m.version, m.page + 1));
            }
            out.out(&text).await;
            0
        }
    });
    ExpansionService {
        settings: Arc::new(tx),
        nv: Nvdata::new(ctx.paths.nvdata.clone()),
        supported: super::super::state_model(ctx.model).expansion_modules(),
    }
}
