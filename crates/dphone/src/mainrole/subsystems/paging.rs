//! Multicast paging: listening to page groups and paging a group.
//!
//! The configuration is the stock `<multicastpage>` element: `<listener>`
//! children are groups to listen to, `<broadcast>` children are groups this
//! phone can page.  What follows is what the stock firmware does, recovered
//! from its `core` (`src/multicast.cpp`) and `phone` binaries:
//!
//! - A page is plain RTP sent to the group address and port, with payload
//!   type 0 (PCMU), 8 (PCMA), or 9 (G.722).  A stream is known by its SSRC.
//! - A page that arrives is dropped while this phone is itself paging, and,
//!   when a call is connected, if the listener's `interrupt_calls` is 2 or
//!   the call is with the listener's `extension`.  With `interrupt_calls`
//!   1 the call is put on hold and the page plays.
//! - Paging a group puts a connected call on hold and ends a call that is
//!   still being dialed, then sends the microphone as RTP with the
//!   broadcast's codec (PCMU unless it says PCMA or G722).
//!
//! Not recovered, and so decided here:
//!
//! - With `interrupt_calls` 0 and a call connected, stock goes on to its
//!   page screen without holding the call; what is then heard is not
//!   known.  Here the page is not played, and a notice says it arrived.
//! - How stock detects the end of a page.  Here: no RTP from its source for
//!   [`PAGE_END`].
//! - How `priority` orders two pages at once.  Here the page that is
//!   playing goes on, and others are ignored until it ends.
//! - Datagrams that begin with `d1 61 55 4d` are a control protocol
//!   (pings, checked against a list governed by the `pingacct` attribute)
//!   whose purpose and replies are not recovered; they are ignored.
//!
//! The audio engine takes one socket per stream and call control owns its
//! own; so a page that is played is relayed, datagram by datagram, from the
//! group socket here to a stream the audio service opens on the loopback
//! address.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket as StdUdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dphone_config::Config;
use dphone_media::PayloadCodec;
use dphone_state::{
    CallId, CallState, Calls, Command, CommandError, Domain, Network, NoticeLevel, PageActivity, PageGroup, Paging, Reply,
};
use log::{info, warn};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, watch};

use super::audio::{AudioService, Stream};
use crate::mainrole::Context;

/// Silence from a page's source after which the page is over.
pub const PAGE_END: Duration = Duration::from_millis(1000);
/// First bytes of the control datagrams that share the group with RTP.
const CONTROL_MAGIC: [u8; 4] = [0xd1, 0x61, 0x55, 0x4d];

#[derive(Clone, Debug, PartialEq, Eq)]
struct Listener {
    group: PageGroup,
    addr: Ipv4Addr,
    interrupt_calls: u8,
    extension: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Broadcast {
    group: PageGroup,
    addr: Ipv4Addr,
    codec: PayloadCodec,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Settings {
    listeners: Vec<Listener>,
    broadcasts: Vec<Broadcast>,
}

fn multicast(addr: &str) -> Option<Ipv4Addr> {
    addr.parse::<Ipv4Addr>().ok().filter(Ipv4Addr::is_multicast)
}

/// The codec a `<broadcast codec>` names; anything but PCMA and G722 is
/// PCMU, as in stock.
fn broadcast_codec(name: &str) -> PayloadCodec {
    if name.eq_ignore_ascii_case("PCMA") {
        PayloadCodec::Pcma
    } else if name.eq_ignore_ascii_case("G722") {
        PayloadCodec::G722
    } else {
        PayloadCodec::Pcmu
    }
}

fn settings(config: &Config) -> Settings {
    let mut out = Settings::default();
    for l in config.multicast_listeners() {
        match multicast(&l.addr) {
            Some(addr) if l.port != 0 => out.listeners.push(Listener {
                group: PageGroup { id: l.id, name: l.name, addr: l.addr, port: l.port },
                addr,
                interrupt_calls: l.interrupt_calls,
                extension: l.extension,
            }),
            _ => warn!("paging: listener '{}' has no usable multicast address and port ({}:{})", l.name, l.addr, l.port),
        }
    }
    for b in config.multicast_broadcasts() {
        match multicast(&b.addr) {
            Some(addr) => out.broadcasts.push(Broadcast {
                codec: broadcast_codec(&b.codec),
                group: PageGroup { id: b.id, name: b.name, addr: b.addr, port: b.port },
                addr,
            }),
            None => warn!("paging: broadcast '{}' has no usable multicast address ({})", b.name, b.addr),
        }
    }
    out
}

/// What the header of a datagram on a page group says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Datagram {
    /// RTP with a page codec.
    Page { codec: PayloadCodec, payload_type: u8, ssrc: u32 },
    /// The control protocol, or anything else that is not a page.
    Other,
}

/// Classify a datagram received on a page group.
pub fn classify(data: &[u8]) -> Datagram {
    if data.len() < 12 || data[..4] == CONTROL_MAGIC || data[0] >> 6 != 2 {
        return Datagram::Other;
    }
    let payload_type = data[1] & 0x7f;
    let codec = match payload_type {
        0 => PayloadCodec::Pcmu,
        8 => PayloadCodec::Pcma,
        9 => PayloadCodec::G722,
        _ => return Datagram::Other,
    };
    Datagram::Page { codec, payload_type, ssrc: u32::from_be_bytes([data[8], data[9], data[10], data[11]]) }
}

/// What to do with a page that starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrival {
    Play,
    /// Hold this call, then play.
    HoldAndPlay(CallId),
    /// Say that it arrived; do not play it.
    Blocked,
    Drop,
}

/// The stock arrival rules (see the module text).  `connected` is the call
/// in conversation, with the far end's number.
pub fn arrival(sending: bool, connected: Option<(CallId, &str)>, interrupt_calls: u8, extension: &str) -> Arrival {
    if sending {
        return Arrival::Drop;
    }
    match connected {
        None => Arrival::Play,
        Some((_, number)) if interrupt_calls == 2 || (!extension.is_empty() && number == extension) => Arrival::Drop,
        Some((call, _)) if interrupt_calls == 1 => Arrival::HoldAndPlay(call),
        Some(_) => Arrival::Blocked,
    }
}

/// A page being played.
struct Playing {
    listener: usize,
    ssrc: u32,
    call: CallId,
    relay: StdUdpSocket,
    to: SocketAddr,
    last_rx: Instant,
}

/// A page source that is being ignored until it falls silent.
struct Ignored {
    listener: usize,
    ssrc: u32,
    last_rx: Instant,
    /// Shown as blocked (as opposed to dropped or dismissed).
    blocked: bool,
}

struct Service {
    ctx: Context,
    audio: AudioService,
    settings: Settings,
    playing: Option<Playing>,
    ignored: Vec<Ignored>,
    sending: Option<(String, CallId)>,
}

impl Service {
    fn set_activity(&self, activity: PageActivity, error: Option<String>) {
        self.ctx.store.update::<Paging, _>(|p| {
            p.activity = activity;
            if let Some(e) = error {
                p.error = e;
            }
        });
    }

    fn publish_groups(&self) {
        let (listeners, broadcasts) = (
            self.settings.listeners.iter().map(|l| l.group.clone()).collect(),
            self.settings.broadcasts.iter().map(|b| b.group.clone()).collect(),
        );
        self.ctx.store.update::<Paging, _>(|p| {
            p.listeners = listeners;
            p.broadcasts = broadcasts;
        });
    }

    /// The call in conversation, with the far end's number.
    fn connected_call(&self) -> Option<(CallId, String)> {
        self.ctx.store.read::<Calls, _>(|c| {
            c.calls.iter().find(|c| c.state == CallState::Active).map(|c| (c.id, c.remote.number.clone()))
        })
    }

    fn on_datagram(&mut self, listener: usize, data: &[u8]) {
        let Datagram::Page { codec, payload_type, ssrc } = classify(data) else { return };
        let now = Instant::now();
        if let Some(p) = self.playing.as_mut() {
            if p.listener == listener && p.ssrc == ssrc {
                p.last_rx = now;
                let _ = p.relay.send_to(data, p.to);
            }
            // Another page while one plays: ignored (priority order is not
            // recovered).
            return;
        }
        if let Some(i) = self.ignored.iter_mut().find(|i| i.listener == listener && i.ssrc == ssrc) {
            i.last_rx = now;
            return;
        }
        let Some(l) = self.settings.listeners.get(listener).cloned() else { return };
        let connected = self.connected_call();
        let decision = arrival(
            self.sending.is_some(),
            connected.as_ref().map(|(id, number)| (*id, number.as_str())),
            l.interrupt_calls,
            &l.extension,
        );
        info!("paging: page on '{}' ({}:{}), source {ssrc:08x}: {decision:?}", l.group.name, l.group.addr, l.group.port);
        let mut ignore = |blocked| self.ignored.push(Ignored { listener, ssrc, last_rx: now, blocked });
        match decision {
            Arrival::Drop => ignore(false),
            Arrival::Blocked => {
                ignore(true);
                self.set_activity(PageActivity::Blocked { group: l.group.id.clone(), name: l.group.name.clone() }, None);
                self.ctx.store.post(Command::Notify {
                    level: NoticeLevel::Info,
                    title: "Page".into(),
                    text: l.group.name.clone(),
                    timeout_ms: 5000,
                });
            }
            Arrival::Play | Arrival::HoldAndPlay(_) => {
                if let Arrival::HoldAndPlay(call) = decision {
                    self.ctx.store.post(Command::Hold { call });
                }
                match self.play(listener, &l, codec, payload_type, ssrc) {
                    Ok(playing) => {
                        let _ = playing.relay.send_to(data, playing.to);
                        self.playing = Some(playing);
                        self.set_activity(
                            PageActivity::Receiving { group: l.group.id.clone(), name: l.group.name.clone() },
                            Some(String::new()),
                        );
                    }
                    Err(e) => {
                        warn!("paging: cannot play the page on '{}': {e}", l.group.name);
                        self.ignored.push(Ignored { listener, ssrc, last_rx: now, blocked: false });
                        self.set_activity(PageActivity::Idle, Some(e));
                    }
                }
            }
        }
    }

    fn play(&self, listener: usize, l: &Listener, codec: PayloadCodec, payload_type: u8, ssrc: u32) -> Result<Playing, String> {
        if !self.audio.codecs().contains(&codec) {
            return Err(format!("{codec:?} is not available"));
        }
        let call = self.ctx.store.next_call_id();
        let port = self.audio.open(call)?;
        let relay = StdUdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| format!("relay socket: {e}"))?;
        let _ = relay.set_nonblocking(true);
        let from = relay.local_addr().map_err(|e| format!("relay socket: {e}"))?;
        let stream = Stream { codec, tx_pt: payload_type, remote: from, ptime_ms: 20, dtmf_pt: None, srtp: None, send: false, recv: true };
        if let Err(e) = self.audio.start(call, &stream) {
            self.audio.close(call);
            return Err(e);
        }
        self.audio.connect_page(Some(call));
        let _ = l;
        Ok(Playing { listener, ssrc, call, relay, to: SocketAddr::from((Ipv4Addr::LOCALHOST, port)), last_rx: Instant::now() })
    }

    /// Stop the page that is playing.  When `dismissed`, the rest of it is
    /// ignored instead of starting again.
    fn stop_playing(&mut self, dismissed: bool) {
        if let Some(p) = self.playing.take() {
            self.audio.connect_page(None);
            self.audio.close(p.call);
            if dismissed {
                self.ignored.push(Ignored { listener: p.listener, ssrc: p.ssrc, last_rx: Instant::now(), blocked: false });
            }
            info!("paging: page ended");
            self.set_activity(PageActivity::Idle, None);
        }
    }

    fn start_sending(&mut self, id: &str) -> Result<(), CommandError> {
        let b = self
            .settings
            .broadcasts
            .iter()
            .find(|b| b.group.id == id)
            .cloned()
            .ok_or_else(|| CommandError::Invalid(format!("no page group '{id}'")))?;
        if self.sending.is_some() {
            return Err(CommandError::Failed("a page is already being sent".into()));
        }
        if !self.audio.codecs().contains(&b.codec) {
            return Err(CommandError::Failed(format!("{:?} is not available", b.codec)));
        }
        let payload_type = b.codec.static_payload_type().ok_or_else(|| CommandError::Failed("no payload type".into()))?;
        self.stop_playing(true);
        // As stock: a call still being dialed ends; a connected one holds.
        let (dialing, active): (Vec<CallId>, Vec<CallId>) = self.ctx.store.read::<Calls, _>(|c| {
            let ids = |f: fn(&CallState) -> bool| c.calls.iter().filter(|c| f(&c.state)).map(|c| c.id).collect();
            (ids(|s| matches!(s, CallState::Dialing)), ids(|s| matches!(s, CallState::Active)))
        });
        for call in dialing {
            self.ctx.store.post(Command::Hangup { call });
        }
        for call in active {
            self.ctx.store.post(Command::Hold { call });
        }
        let call = self.ctx.store.next_call_id();
        let fail = |e: String| CommandError::Failed(e);
        self.audio.open(call).map_err(fail)?;
        let stream = Stream {
            codec: b.codec,
            tx_pt: payload_type,
            remote: SocketAddr::new(IpAddr::V4(b.addr), b.group.port),
            ptime_ms: 20,
            dtmf_pt: None,
            srtp: None,
            send: true,
            recv: false,
        };
        if let Err(e) = self.audio.start(call, &stream) {
            self.audio.close(call);
            self.set_activity(PageActivity::Idle, Some(e.clone()));
            return Err(fail(e));
        }
        self.audio.connect_page(Some(call));
        info!("paging: paging '{}' ({}:{}, {:?})", b.group.name, b.group.addr, b.group.port, b.codec);
        self.sending = Some((b.group.id.clone(), call));
        self.set_activity(PageActivity::Sending { group: b.group.id, name: b.group.name }, Some(String::new()));
        Ok(())
    }

    fn stop_sending(&mut self) {
        if let Some((_, call)) = self.sending.take() {
            self.audio.connect_page(None);
            self.audio.close(call);
            info!("paging: stopped paging");
            self.set_activity(PageActivity::Idle, None);
        }
    }

    /// Pages whose source has fallen silent are over.
    fn expire(&mut self) {
        let now = Instant::now();
        if self.playing.as_ref().is_some_and(|p| now.duration_since(p.last_rx) > PAGE_END) {
            self.stop_playing(false);
        }
        let before = self.ignored.iter().filter(|i| i.blocked).count();
        self.ignored.retain(|i| now.duration_since(i.last_rx) <= PAGE_END);
        if before > 0 && !self.ignored.iter().any(|i| i.blocked) && self.playing.is_none() && self.sending.is_none() {
            self.set_activity(PageActivity::Idle, None);
        }
    }

    fn apply(&mut self, settings: Settings) {
        // A group that went away takes its page with it.
        if let Some(p) = &self.playing
            && settings.listeners.get(p.listener) != self.settings.listeners.get(p.listener)
        {
            self.stop_playing(false);
        }
        if let Some((id, _)) = &self.sending
            && !settings.broadcasts.iter().any(|b| &b.group.id == id)
        {
            self.stop_sending();
        }
        self.ignored.clear();
        self.settings = settings;
        self.publish_groups();
    }
}

/// Receive on one group and hand each datagram to the service.
async fn receive(index: usize, socket: UdpSocket, tx: mpsc::Sender<(usize, Vec<u8>)>) {
    let mut buf = [0u8; 1500];
    loop {
        match socket.recv_from(&mut buf).await {
            Ok((n, _)) => {
                if tx.send((index, buf[..n].to_vec())).await.is_err() {
                    return;
                }
            }
            Err(e) => {
                warn!("paging: receive: {e}");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

/// Join every listener's group on the interface with address `local`.
fn join(listeners: &[Listener], local: Ipv4Addr, tx: &mpsc::Sender<(usize, Vec<u8>)>) -> Vec<tokio::task::JoinHandle<()>> {
    let mut tasks = Vec::new();
    for (index, l) in listeners.iter().enumerate() {
        let socket = dphone_media::net::bind_multicast_rx(l.addr, l.group.port, local).and_then(|s| {
            s.set_nonblocking(true)?;
            UdpSocket::from_std(s)
        });
        match socket {
            Ok(socket) => {
                info!("paging: listening to '{}' on {}:{}", l.group.name, l.group.addr, l.group.port);
                tasks.push(tokio::spawn(receive(index, socket, tx.clone())));
            }
            Err(e) => warn!("paging: cannot listen to '{}' on {}:{}: {e}", l.group.name, l.group.addr, l.group.port),
        }
    }
    tasks
}

async fn run(ctx: Context, audio: AudioService, mut config: watch::Receiver<Settings>) {
    let commands = ctx.store.handle(Domain::Paging);
    let network = ctx.store.subscribe(dphone_state::Topic::Network);
    let (tx, mut datagrams) = mpsc::channel::<(usize, Vec<u8>)>(64);
    let mut service = Service { ctx: ctx.clone(), audio, settings: Settings::default(), playing: None, ignored: Vec::new(), sending: None };
    let mut tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    let mut joined_on: Option<Ipv4Addr> = None;
    let mut rejoin = true;
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    service.apply(config.borrow_and_update().clone());
    loop {
        if rejoin {
            rejoin = false;
            for t in tasks.drain(..) {
                t.abort();
            }
            // In the simulator there is no network state; the loopback
            // interface carries the groups.
            let local = ctx.store.read::<Network, _>(|n| n.ipv4.address).or(ctx.sim.then_some(Ipv4Addr::LOCALHOST));
            joined_on = local;
            if let Some(local) = local {
                tasks = join(&service.settings.listeners, local, &tx);
            }
        }
        tokio::select! {
            Some((index, data)) = datagrams.recv() => service.on_datagram(index, &data),
            _ = tick.tick() => service.expire(),
            changed = config.changed() => {
                if changed.is_err() {
                    return;
                }
                let new = config.borrow_and_update().clone();
                if new != service.settings {
                    rejoin = new.listeners != service.settings.listeners;
                    service.apply(new);
                }
            }
            _ = network.changed() => {
                let local = ctx.store.read::<Network, _>(|n| n.ipv4.address).or(ctx.sim.then_some(Ipv4Addr::LOCALHOST));
                rejoin = local != joined_on;
            }
            Some(envelope) = commands.next() => {
                let (cmd, responder) = envelope.split();
                let result = match cmd {
                    Command::StartPage { id } => service.start_sending(&id).map(|()| Reply::Done),
                    Command::StopPage => {
                        service.stop_sending();
                        service.stop_playing(true);
                        Ok(Reply::Done)
                    }
                    _ => Err(CommandError::Invalid("not a paging command".into())),
                };
                responder.reply(result);
            }
        }
    }
}

/// Handle for configuration changes.
#[derive(Clone)]
pub struct PagingService {
    settings: Arc<watch::Sender<Settings>>,
}

impl PagingService {
    pub fn apply_config(&self, config: &Config) {
        self.settings.send_if_modified(|s| {
            let new = settings(config);
            let changed = *s != new;
            *s = new;
            changed
        });
    }
}

pub fn start(ctx: &Context, audio: AudioService) -> PagingService {
    let (tx, rx) = watch::channel(Settings::default());
    tokio::spawn(run(ctx.clone(), audio, rx));
    let store = ctx.store.clone();
    ctx.diag.register("page", "multicast paging: page | page start ID | page stop", move |args: Vec<String>, out: crate::diag::Output| {
        let store = store.clone();
        async move {
            let command = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
                [] => None,
                ["start", id] => Some(Command::StartPage { id: id.to_string() }),
                ["stop"] => Some(Command::StopPage),
                _ => {
                    out.err("usage: dphone page | dphone page start ID | dphone page stop\n").await;
                    return 2;
                }
            };
            if let Some(command) = command {
                let pending = store.send(command);
                let result = tokio::task::spawn_blocking(move || pending.wait_timeout(Duration::from_secs(5))).await;
                if let Ok(Err(e)) = result {
                    out.err(&format!("dphone page: {e:?}\n")).await;
                    return 1;
                }
            }
            let p = store.get::<Paging>();
            let mut text = format!("activity: {:?}\n", p.activity);
            if !p.error.is_empty() {
                text.push_str(&format!("error: {}\n", p.error));
            }
            for g in &p.listeners {
                text.push_str(&format!("listens to {} '{}' on {}:{}\n", g.id, g.name, g.addr, g.port));
            }
            for g in &p.broadcasts {
                text.push_str(&format!("can page {} '{}' on {}:{}\n", g.id, g.name, g.addr, g.port));
            }
            out.out(&text).await;
            0
        }
    });
    PagingService { settings: Arc::new(tx) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rtp(payload_type: u8, ssrc: u32) -> Vec<u8> {
        let mut p = vec![0x80, payload_type, 0, 1, 0, 0, 0, 160];
        p.extend_from_slice(&ssrc.to_be_bytes());
        p.extend_from_slice(&[0xff; 160]);
        p
    }

    #[test]
    fn datagrams() {
        assert_eq!(classify(&rtp(0, 7)), Datagram::Page { codec: PayloadCodec::Pcmu, payload_type: 0, ssrc: 7 });
        assert_eq!(classify(&rtp(8, 0xdead_beef)), Datagram::Page { codec: PayloadCodec::Pcma, payload_type: 8, ssrc: 0xdead_beef });
        // The marker bit does not change the payload type.
        assert_eq!(classify(&rtp(0x80 | 9, 1)), Datagram::Page { codec: PayloadCodec::G722, payload_type: 9, ssrc: 1 });
        // Other payload types, short datagrams, and the control protocol.
        assert_eq!(classify(&rtp(18, 1)), Datagram::Other);
        assert_eq!(classify(&[0x80, 0, 0, 1]), Datagram::Other);
        let mut control = CONTROL_MAGIC.to_vec();
        control.extend_from_slice(b"p whatever follows");
        assert_eq!(classify(&control), Datagram::Other);
    }

    #[test]
    fn arrival_rules() {
        let call = CallId(4);
        // Idle phone: play.
        assert_eq!(arrival(false, None, 0, ""), Arrival::Play);
        assert_eq!(arrival(false, None, 2, "100"), Arrival::Play);
        // Paging ourselves: drop.
        assert_eq!(arrival(true, None, 1, ""), Arrival::Drop);
        // In a call: by interrupt_calls.
        assert_eq!(arrival(false, Some((call, "200")), 1, ""), Arrival::HoldAndPlay(call));
        assert_eq!(arrival(false, Some((call, "200")), 2, ""), Arrival::Drop);
        assert_eq!(arrival(false, Some((call, "200")), 0, ""), Arrival::Blocked);
        // In a call with the page's own extension: drop.
        assert_eq!(arrival(false, Some((call, "200")), 1, "200"), Arrival::Drop);
    }

    #[test]
    fn codecs() {
        assert_eq!(broadcast_codec("pcma"), PayloadCodec::Pcma);
        assert_eq!(broadcast_codec("G722"), PayloadCodec::G722);
        assert_eq!(broadcast_codec("PCMU"), PayloadCodec::Pcmu);
        assert_eq!(broadcast_codec("opus"), PayloadCodec::Pcmu);
        assert!(multicast("239.1.1.1").is_some());
        assert!(multicast("10.0.0.1").is_none());
    }
}
