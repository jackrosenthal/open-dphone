//! LLDP (IEEE 802.1AB) with LLDP-MED (ANSI/TIA-1057): frame codec, the
//! transmit/receive agent, and the stock voice-VLAN selection rule.

use std::io;
use std::net::Ipv4Addr;
use std::time::Duration;

use tokio::sync::{mpsc, watch};
use tokio::time::{Instant, sleep_until};

use crate::packet::{ETH_P_LLDP, PacketSocket};
use crate::sys::{self, MacAddr};

/// Nearest-bridge group address.
pub const LLDP_MULTICAST: MacAddr = MacAddr([0x01, 0x80, 0xc2, 0x00, 0x00, 0x0e]);

const TLV_END: u8 = 0;
const TLV_CHASSIS_ID: u8 = 1;
const TLV_PORT_ID: u8 = 2;
const TLV_TTL: u8 = 3;
const TLV_PORT_DESC: u8 = 4;
const TLV_SYS_NAME: u8 = 5;
const TLV_SYS_DESC: u8 = 6;
const TLV_SYS_CAP: u8 = 7;
const TLV_MGMT_ADDR: u8 = 8;
const TLV_ORG: u8 = 127;

const OUI_TIA: [u8; 3] = [0x00, 0x12, 0xbb];
const OUI_8023: [u8; 3] = [0x00, 0x12, 0x0f];

const MED_CAPABILITIES: u8 = 1;
const MED_NETWORK_POLICY: u8 = 2;
const MED_EXTENDED_POWER: u8 = 4;
const MED_INV_HARDWARE: u8 = 5;
const MED_INV_FIRMWARE: u8 = 6;
const MED_INV_SOFTWARE: u8 = 7;
const MED_INV_SERIAL: u8 = 8;
const MED_INV_MANUFACTURER: u8 = 9;
const MED_INV_MODEL: u8 = 10;

/// System capability bits (802.1AB table 8-4).
pub const CAP_BRIDGE: u16 = 1 << 2;
pub const CAP_TELEPHONE: u16 = 1 << 5;

/// LLDP-MED device class of an endpoint that is a phone (class III).
pub const MED_CLASS_ENDPOINT_III: u8 = 3;
/// LLDP-MED device class of a switch ("Network Connectivity Device").
pub const MED_CLASS_NETWORK_CONNECTIVITY: u8 = 4;

/// LLDP-MED application types.
pub const APP_VOICE: u8 = 1;
pub const APP_VOICE_SIGNALING: u8 = 2;

/// An LLDP-MED network policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkPolicy {
    pub app_type: u8,
    /// The policy is required but not yet known.
    pub unknown: bool,
    pub tagged: bool,
    /// 0 means priority-tagged; 1 to 4094 a VLAN.
    pub vlan_id: u16,
    pub priority: u8,
    pub dscp: u8,
}

impl NetworkPolicy {
    fn encode(&self) -> [u8; 4] {
        let v: u32 = (self.unknown as u32) << 23
            | (self.tagged as u32) << 22
            | ((self.vlan_id as u32) & 0xfff) << 9
            | ((self.priority as u32) & 7) << 6
            | (self.dscp as u32) & 0x3f;
        [self.app_type, (v >> 16) as u8, (v >> 8) as u8, v as u8]
    }

    fn decode(b: &[u8]) -> Option<Self> {
        if b.len() < 4 {
            return None;
        }
        let v = (b[1] as u32) << 16 | (b[2] as u32) << 8 | b[3] as u32;
        Some(Self {
            app_type: b[0],
            unknown: v >> 23 & 1 != 0,
            tagged: v >> 22 & 1 != 0,
            vlan_id: (v >> 9 & 0xfff) as u16,
            priority: (v >> 6 & 7) as u8,
            dscp: (v & 0x3f) as u8,
        })
    }
}

/// What this phone advertises.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalInfo {
    pub mac: MacAddr,
    pub port_description: String,
    /// Stock uses the host name, `<sysname>-<MAC>`.
    pub system_name: String,
    pub system_description: String,
    pub management_address: Option<Ipv4Addr>,
    pub ifindex: u32,
    /// Power the phone draws as a PoE powered device, in milliwatts.
    pub power_mw: u32,
    pub policies: Vec<NetworkPolicy>,
    pub hardware_rev: String,
    pub firmware_rev: String,
    pub software_rev: String,
    pub serial: String,
    pub manufacturer: String,
    pub model: String,
    /// Seconds between transmissions (stock: 30).
    pub tx_interval: u16,
    /// TTL is `tx_interval * tx_hold` (stock: 4).
    pub tx_hold: u16,
}

impl LocalInfo {
    /// The policies stock advertises before it knows anything: voice and
    /// voice signaling, both "unknown".
    pub fn unknown_policies() -> Vec<NetworkPolicy> {
        [APP_VOICE, APP_VOICE_SIGNALING]
            .into_iter()
            .map(|app_type| NetworkPolicy {
                app_type,
                unknown: true,
                tagged: false,
                vlan_id: 0,
                priority: 0,
                dscp: 0,
            })
            .collect()
    }

    /// The policies stock advertises once the VLAN decision is made.
    pub fn decided_policies(
        vlan_id: Option<u16>,
        rtp_pcp: u8,
        rtp_dscp: u8,
        sip_pcp: u8,
        sip_dscp: u8,
    ) -> Vec<NetworkPolicy> {
        let (tagged, vid) = match vlan_id {
            Some(v) => (true, v),
            None => (false, 0),
        };
        vec![
            NetworkPolicy {
                app_type: APP_VOICE,
                unknown: false,
                tagged,
                vlan_id: vid,
                priority: if tagged { rtp_pcp } else { 0 },
                dscp: rtp_dscp,
            },
            NetworkPolicy {
                app_type: APP_VOICE_SIGNALING,
                unknown: false,
                tagged,
                vlan_id: vid,
                priority: if tagged { sip_pcp } else { 0 },
                dscp: sip_dscp,
            },
        ]
    }
}

fn tlv(out: &mut Vec<u8>, ty: u8, value: &[u8]) {
    let len = value.len().min(511);
    out.push(ty << 1 | (len >> 8) as u8);
    out.push(len as u8);
    out.extend_from_slice(&value[..len]);
}

fn org_tlv(out: &mut Vec<u8>, oui: [u8; 3], subtype: u8, value: &[u8]) {
    let mut v = Vec::with_capacity(4 + value.len());
    v.extend_from_slice(&oui);
    v.push(subtype);
    v.extend_from_slice(value);
    tlv(out, TLV_ORG, &v);
}

fn clip(s: &str, max: usize) -> &[u8] {
    let b = s.as_bytes();
    &b[..b.len().min(max)]
}

/// Encode the LLDPDU this phone transmits.  `ttl` zero is the shutdown frame.
pub fn encode(info: &LocalInfo, ttl: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(256);
    let mut id = vec![4u8]; // subtype: MAC address
    id.extend_from_slice(&info.mac.0);
    tlv(&mut out, TLV_CHASSIS_ID, &id);
    let mut id = vec![3u8]; // subtype: MAC address
    id.extend_from_slice(&info.mac.0);
    tlv(&mut out, TLV_PORT_ID, &id);
    tlv(&mut out, TLV_TTL, &ttl.to_be_bytes());
    if ttl == 0 {
        tlv(&mut out, TLV_END, &[]);
        return out;
    }
    tlv(&mut out, TLV_PORT_DESC, clip(&info.port_description, 255));
    tlv(&mut out, TLV_SYS_NAME, clip(&info.system_name, 255));
    tlv(&mut out, TLV_SYS_DESC, clip(&info.system_description, 255));
    let caps = CAP_BRIDGE | CAP_TELEPHONE;
    let mut v = caps.to_be_bytes().to_vec();
    v.extend_from_slice(&caps.to_be_bytes());
    tlv(&mut out, TLV_SYS_CAP, &v);
    if let Some(addr) = info.management_address {
        let mut v = vec![5, 1]; // address string length, subtype IPv4
        v.extend_from_slice(&addr.octets());
        v.push(2); // interface numbering: ifIndex
        v.extend_from_slice(&info.ifindex.to_be_bytes());
        v.push(0); // no OID
        tlv(&mut out, TLV_MGMT_ADDR, &v);
    }
    // LLDP-MED requires the 802.3 MAC/PHY TLV.  Autonegotiation supported
    // and enabled; advertised capabilities and MAU type left unspecified.
    org_tlv(&mut out, OUI_8023, 1, &[0x03, 0, 0, 0, 0]);
    // MED capabilities: LLDP-MED capabilities, network policy, extended
    // power PD, inventory; device class III.
    let med_caps: u16 = 1 << 0 | 1 << 1 | 1 << 4 | 1 << 5;
    org_tlv(
        &mut out,
        OUI_TIA,
        MED_CAPABILITIES,
        &[
            (med_caps >> 8) as u8,
            med_caps as u8,
            MED_CLASS_ENDPOINT_III,
        ],
    );
    for p in &info.policies {
        org_tlv(&mut out, OUI_TIA, MED_NETWORK_POLICY, &p.encode());
    }
    // Extended power: PD device, source "PSE and local" (stock: `source
    // both`), priority unknown; value in units of 0.1 W.
    let tenths = (info.power_mw / 100).min(1023) as u16;
    org_tlv(
        &mut out,
        OUI_TIA,
        MED_EXTENDED_POWER,
        &[0x01 << 6 | 0x03 << 4, (tenths >> 8) as u8, tenths as u8],
    );
    for (subtype, s) in [
        (MED_INV_HARDWARE, &info.hardware_rev),
        (MED_INV_FIRMWARE, &info.firmware_rev),
        (MED_INV_SOFTWARE, &info.software_rev),
        (MED_INV_SERIAL, &info.serial),
        (MED_INV_MANUFACTURER, &info.manufacturer),
        (MED_INV_MODEL, &info.model),
    ] {
        if !s.is_empty() {
            org_tlv(&mut out, OUI_TIA, subtype, clip(s, 32));
        }
    }
    tlv(&mut out, TLV_END, &[]);
    out
}

/// A neighbor as learned from one LLDPDU.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Neighbor {
    /// Raw chassis ID TLV value (subtype byte first).
    pub chassis_id: Vec<u8>,
    /// Raw port ID TLV value (subtype byte first).
    pub port_id: Vec<u8>,
    pub ttl: u16,
    pub port_description: Option<String>,
    pub system_name: Option<String>,
    pub system_description: Option<String>,
    pub capabilities_enabled: u16,
    pub management_address: Option<Ipv4Addr>,
    /// LLDP-MED device class, when the neighbor speaks LLDP-MED.
    pub med_device_class: Option<u8>,
    pub policies: Vec<NetworkPolicy>,
}

/// Decode an LLDPDU.  Returns `None` when the mandatory TLVs are missing
/// or malformed.
pub fn decode(mut b: &[u8]) -> Option<Neighbor> {
    let mut n = Neighbor::default();
    let mut index = 0;
    let mut have_ttl = false;
    loop {
        if b.len() < 2 {
            // A frame without an End TLV is tolerated once the mandatory TLVs are in.
            break;
        }
        let ty = b[0] >> 1;
        let len = ((b[0] as usize & 1) << 8) | b[1] as usize;
        let v = b.get(2..2 + len)?;
        b = &b[2 + len..];
        // The first three TLVs are fixed.
        match (index, ty) {
            (0, TLV_CHASSIS_ID) | (1, TLV_PORT_ID) | (2, TLV_TTL) => {}
            (0..=2, _) => return None,
            _ => {}
        }
        index += 1;
        let text = |v: &[u8]| {
            Some(
                String::from_utf8_lossy(v)
                    .trim_end_matches('\0')
                    .to_string(),
            )
        };
        match ty {
            TLV_END => break,
            TLV_CHASSIS_ID if !v.is_empty() => n.chassis_id = v.to_vec(),
            TLV_PORT_ID if !v.is_empty() => n.port_id = v.to_vec(),
            TLV_TTL if v.len() >= 2 => {
                n.ttl = u16::from_be_bytes([v[0], v[1]]);
                have_ttl = true;
            }
            TLV_PORT_DESC => n.port_description = text(v),
            TLV_SYS_NAME => n.system_name = text(v),
            TLV_SYS_DESC => n.system_description = text(v),
            TLV_SYS_CAP if v.len() >= 4 => {
                n.capabilities_enabled = u16::from_be_bytes([v[2], v[3]])
            }
            TLV_MGMT_ADDR if v.len() >= 6 && v[0] == 5 && v[1] == 1 => {
                n.management_address = Some(Ipv4Addr::new(v[2], v[3], v[4], v[5]));
            }
            TLV_ORG if v.len() >= 4 && v[0..3] == OUI_TIA => match v[3] {
                MED_CAPABILITIES if v.len() >= 7 => n.med_device_class = Some(v[6]),
                MED_NETWORK_POLICY => {
                    if let Some(p) = NetworkPolicy::decode(&v[4..]) {
                        n.policies.push(p);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    if n.chassis_id.is_empty() || n.port_id.is_empty() || !have_ttl {
        return None;
    }
    Some(n)
}

/// The voice VLAN and markings a switch asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoicePolicy {
    /// The raw VLAN ID from the voice policy; only 1 to 4094 creates a VLAN.
    pub vlan_id: u16,
    pub rtp_pcp: Option<u8>,
    pub rtp_dscp: Option<u8>,
    pub sip_pcp: Option<u8>,
    pub sip_dscp: Option<u8>,
}

/// Apply the stock selection rule to the current neighbors.
///
/// Only neighbors whose LLDP-MED device class is "Network Connectivity
/// Device" count.  Application type 1 (voice) selects the single voice
/// VLAN and the RTP markings.  Application type 2 (voice signaling)
/// contributes SIP markings only when its VLAN ID equals the voice VLAN
/// ID; it never creates a VLAN of its own.  The stock script walks the
/// neighbors in `lldpctl` order and resets its state at each connectivity
/// device, so the last such neighbor wins; the same is done here.
pub fn select_voice_policy(neighbors: &[Neighbor]) -> Option<VoicePolicy> {
    let mut result = None;
    for n in neighbors {
        if n.med_device_class != Some(MED_CLASS_NETWORK_CONNECTIVITY) {
            continue;
        }
        let mut current: Option<VoicePolicy> = None;
        for p in &n.policies {
            match p.app_type {
                APP_VOICE => {
                    current = Some(VoicePolicy {
                        vlan_id: p.vlan_id,
                        rtp_pcp: Some(p.priority),
                        rtp_dscp: Some(p.dscp),
                        sip_pcp: current.and_then(|c| c.sip_pcp),
                        sip_dscp: current.and_then(|c| c.sip_dscp),
                    });
                }
                APP_VOICE_SIGNALING => {
                    if let Some(c) = current.as_mut() {
                        if c.vlan_id == p.vlan_id {
                            c.sip_pcp = Some(p.priority);
                            c.sip_dscp = Some(p.dscp);
                        }
                    }
                }
                _ => {}
            }
        }
        // A connectivity device without a voice policy clears the result,
        // as the script's `unset` does.
        result = current;
    }
    result
}

/// Whether any connectivity-device neighbor advertises a network policy,
/// which is the stock test for "LLDP-MED information has arrived".
pub fn have_med_policy(neighbors: &[Neighbor]) -> bool {
    neighbors.iter().any(|n| {
        n.med_device_class == Some(MED_CLASS_NETWORK_CONNECTIVITY) && !n.policies.is_empty()
    })
}

enum Cmd {
    SetLocal(Box<LocalInfo>),
    /// Transmit now.
    Update,
    /// Transmit every second with a short TTL (stock's active discovery).
    FastTx(bool),
}

/// Handle to the LLDP agent task.
pub struct Agent {
    /// Current neighbors, refreshed on every change and expiry.
    pub neighbors: watch::Receiver<Vec<Neighbor>>,
    cmd: mpsc::Sender<Cmd>,
    task: tokio::task::JoinHandle<()>,
}

impl Agent {
    /// Start the agent on `ifname`.  Must be called inside a tokio runtime.
    ///
    /// The interface should be the switch's LAN port device, not a bridge
    /// above it: the bridge hands LLDP frames to the port they arrived on.
    pub fn spawn(ifname: &str, info: LocalInfo) -> io::Result<Self> {
        let ifindex = sys::if_index(ifname)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such interface"))?;
        let sock = PacketSocket::new(ifindex, ETH_P_LLDP)?;
        if let Err(e) = sock.add_multicast(LLDP_MULTICAST) {
            log::warn!("lldp: cannot join the LLDP group on {ifname}: {e}");
        }
        let (ntx, nrx) = watch::channel(Vec::new());
        let (ctx, crx) = mpsc::channel(8);
        let task = tokio::spawn(run(sock, info, ntx, crx));
        Ok(Self {
            neighbors: nrx,
            cmd: ctx,
            task,
        })
    }

    /// Replace what is advertised and transmit it.
    pub async fn set_local(&self, info: LocalInfo) {
        let _ = self.cmd.send(Cmd::SetLocal(Box::new(info))).await;
    }

    /// Transmit now (stock `lldpcli update`).
    pub async fn update(&self) {
        let _ = self.cmd.send(Cmd::Update).await;
    }

    /// Enter or leave one-second transmission with a short TTL.
    pub async fn fast_tx(&self, on: bool) {
        let _ = self.cmd.send(Cmd::FastTx(on)).await;
    }

    /// Wait until a connectivity device has sent a network policy, or
    /// `limit` passes.  Returns whether one arrived.
    pub async fn wait_for_med_policy(&mut self, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        loop {
            if have_med_policy(&self.neighbors.borrow()) {
                return true;
            }
            match tokio::time::timeout_at(deadline, self.neighbors.changed()).await {
                Ok(Ok(())) => {}
                _ => return have_med_policy(&self.neighbors.borrow()),
            }
        }
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Entry {
    neighbor: Neighbor,
    expires: Instant,
}

async fn run(
    sock: PacketSocket,
    mut info: LocalInfo,
    neighbors: watch::Sender<Vec<Neighbor>>,
    mut cmd: mpsc::Receiver<Cmd>,
) {
    let mut table: Vec<Entry> = Vec::new();
    let mut fast = false;
    let mut next_tx = Instant::now();
    let mut buf = [0u8; 1600];
    loop {
        let now = Instant::now();
        if now >= next_tx {
            let (interval, hold) = if fast {
                (1, 1)
            } else {
                (info.tx_interval.max(1), info.tx_hold.max(1))
            };
            let ttl = interval.saturating_mul(hold).saturating_add(1);
            if let Err(e) = sock.send(LLDP_MULTICAST, &encode(&info, ttl)).await {
                log::debug!("lldp: send failed: {e}");
            }
            next_tx = now + Duration::from_secs(interval as u64);
        }
        let next_expiry = table.iter().map(|e| e.expires).min();
        let wake = next_expiry.map_or(next_tx, |e| e.min(next_tx));
        tokio::select! {
            _ = sleep_until(wake) => {
                let now = Instant::now();
                let before = table.len();
                table.retain(|e| e.expires > now);
                if table.len() != before {
                    let _ = neighbors.send(table.iter().map(|e| e.neighbor.clone()).collect());
                }
            }
            r = sock.recv(&mut buf) => match r {
                Ok((n, _src)) => {
                    let Some(nb) = decode(&buf[..n]) else { continue };
                    let pos = table.iter().position(|e| e.neighbor.chassis_id == nb.chassis_id && e.neighbor.port_id == nb.port_id);
                    let mut changed = false;
                    if nb.ttl == 0 {
                        if let Some(p) = pos {
                            table.remove(p);
                            changed = true;
                        }
                    } else {
                        let expires = Instant::now() + Duration::from_secs(nb.ttl as u64);
                        match pos {
                            Some(p) => {
                                changed = table[p].neighbor != nb;
                                table[p] = Entry { neighbor: nb, expires };
                            }
                            None => {
                                // LLDP-MED fast start: answer a new neighbor promptly.
                                next_tx = Instant::now();
                                if table.len() < 8 {
                                    table.push(Entry { neighbor: nb, expires });
                                    changed = true;
                                }
                            }
                        }
                    }
                    if changed {
                        let _ = neighbors.send(table.iter().map(|e| e.neighbor.clone()).collect());
                    }
                }
                Err(e) => {
                    log::warn!("lldp: receive failed: {e}");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            },
            c = cmd.recv() => match c {
                Some(Cmd::SetLocal(i)) => {
                    if *i != info {
                        info = *i;
                        next_tx = Instant::now();
                    }
                }
                Some(Cmd::Update) => next_tx = Instant::now(),
                Some(Cmd::FastTx(on)) => {
                    if fast != on {
                        fast = on;
                        next_tx = Instant::now();
                    }
                }
                None => {
                    let _ = sock.send(LLDP_MULTICAST, &encode(&info, 0)).await;
                    return;
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local() -> LocalInfo {
        LocalInfo {
            mac: MacAddr([0x00, 0x0f, 0xd3, 0x01, 0x02, 0x03]),
            port_description: "lan".into(),
            system_name: "d60-000FD3010203".into(),
            system_description: "Open DPhone D60, firmware version 3_0_0".into(),
            management_address: Some(Ipv4Addr::new(10, 0, 0, 5)),
            ifindex: 3,
            power_mw: 6400,
            policies: LocalInfo::unknown_policies(),
            hardware_rev: "D60".into(),
            firmware_rev: "3_0_0".into(),
            software_rev: "3_0_0".into(),
            serial: "000FD3010203".into(),
            manufacturer: "Digium".into(),
            model: "D60".into(),
            tx_interval: 30,
            tx_hold: 4,
        }
    }

    /// LLDPDU sent by a switch, assembled TLV by TLV from the layouts in
    /// IEEE 802.1AB and ANSI/TIA-1057 (the same content a Cisco or ProCurve
    /// port with a voice VLAN sends).
    fn switch_frame(voice_vid: u16, signaling_vid: Option<u16>) -> Vec<u8> {
        let mut f = vec![
            0x02, 0x07, 0x04, 0x00, 0x1b, 0x21, 0xaa, 0xbb, 0xcc, // chassis ID: MAC
            0x04, 0x04, 0x05, b'g', b'e', b'1', // port ID: interface name
            0x06, 0x02, 0x00, 0x78, // TTL 120
            0x0a, 0x03, b's', b'w', b'1', // system name
            0x0e, 0x04, 0x00, 0x14, 0x00, 0x04, // capabilities: bridge+router / bridge
        ];
        // MED capabilities: caps 0x000f, device class 4.
        f.extend_from_slice(&[0xfe, 0x07, 0x00, 0x12, 0xbb, 0x01, 0x00, 0x0f, 0x04]);
        // Voice policy: tagged, VID, priority 5, DSCP 46.
        let v: u32 = 1 << 22 | (voice_vid as u32) << 9 | 5 << 6 | 46;
        f.extend_from_slice(&[
            0xfe,
            0x08,
            0x00,
            0x12,
            0xbb,
            0x02,
            0x01,
            (v >> 16) as u8,
            (v >> 8) as u8,
            v as u8,
        ]);
        if let Some(vid) = signaling_vid {
            let v: u32 = 1 << 22 | (vid as u32) << 9 | 3 << 6 | 24;
            f.extend_from_slice(&[
                0xfe,
                0x08,
                0x00,
                0x12,
                0xbb,
                0x02,
                0x02,
                (v >> 16) as u8,
                (v >> 8) as u8,
                v as u8,
            ]);
        }
        f.extend_from_slice(&[0x00, 0x00]);
        f
    }

    #[test]
    fn policy_bit_layout() {
        // TIA-1057 figure 7: U, T, X, VLAN ID (12), L2 priority (3), DSCP (6).
        let p = NetworkPolicy {
            app_type: 1,
            unknown: false,
            tagged: true,
            vlan_id: 100,
            priority: 5,
            dscp: 46,
        };
        // 0 1 0 000001100100 101 101110 = 0x40 0xc9 0x6e
        assert_eq!(p.encode(), [0x01, 0x40, 0xc9, 0x6e]);
        assert_eq!(NetworkPolicy::decode(&p.encode()), Some(p));
        let unknown = LocalInfo::unknown_policies()[0];
        assert_eq!(unknown.encode(), [0x01, 0x80, 0x00, 0x00]);
    }

    #[test]
    fn decodes_switch_frame() {
        let n = decode(&switch_frame(100, Some(100))).unwrap();
        assert_eq!(n.ttl, 120);
        assert_eq!(n.system_name.as_deref(), Some("sw1"));
        assert_eq!(n.med_device_class, Some(MED_CLASS_NETWORK_CONNECTIVITY));
        assert_eq!(n.capabilities_enabled, 0x0004);
        assert_eq!(n.policies.len(), 2);
        assert_eq!(
            n.policies[0],
            NetworkPolicy {
                app_type: 1,
                unknown: false,
                tagged: true,
                vlan_id: 100,
                priority: 5,
                dscp: 46
            }
        );
    }

    #[test]
    fn our_frame_round_trips() {
        let info = local();
        let f = encode(&info, 121);
        assert!(f.len() < 1500);
        // Mandatory TLV order.
        assert_eq!(
            &f[..9],
            &[0x02, 0x07, 0x04, 0x00, 0x0f, 0xd3, 0x01, 0x02, 0x03]
        );
        let n = decode(&f).unwrap();
        assert_eq!(n.ttl, 121);
        assert_eq!(
            n.system_description.as_deref(),
            Some("Open DPhone D60, firmware version 3_0_0")
        );
        assert_eq!(n.med_device_class, Some(MED_CLASS_ENDPOINT_III));
        assert_eq!(n.management_address, Some(Ipv4Addr::new(10, 0, 0, 5)));
        assert_eq!(n.policies, info.policies);
        assert_eq!(n.capabilities_enabled, CAP_BRIDGE | CAP_TELEPHONE);
        // Extended power TLV: 6400 mW is 64 tenths of a watt.
        let pos = f
            .windows(4)
            .position(|w| w == [0x00, 0x12, 0xbb, 0x04])
            .unwrap();
        assert_eq!(&f[pos + 4..pos + 7], &[0x70, 0x00, 0x40]);
        // Shutdown frame.
        let bye = encode(&info, 0);
        assert_eq!(decode(&bye).unwrap().ttl, 0);
    }

    #[test]
    fn rejects_malformed() {
        assert!(decode(&[]).is_none());
        assert!(decode(&[0x04, 0x02, 0x05, b'x']).is_none()); // port ID first
        let mut f = switch_frame(100, None);
        f.truncate(17); // cut inside the TTL TLV
        assert!(decode(&f).is_none());
        let mut f = switch_frame(100, None);
        f[1] = 0xff; // chassis TLV longer than the frame
        assert!(decode(&f).is_none());
    }

    #[test]
    fn stock_selection_rule() {
        let both = decode(&switch_frame(100, Some(100))).unwrap();
        assert_eq!(
            select_voice_policy(std::slice::from_ref(&both)),
            Some(VoicePolicy {
                vlan_id: 100,
                rtp_pcp: Some(5),
                rtp_dscp: Some(46),
                sip_pcp: Some(3),
                sip_dscp: Some(24)
            })
        );
        // A signaling policy on a different VLAN contributes nothing.
        let split = decode(&switch_frame(100, Some(200))).unwrap();
        assert_eq!(
            select_voice_policy(&[split]),
            Some(VoicePolicy {
                vlan_id: 100,
                rtp_pcp: Some(5),
                rtp_dscp: Some(46),
                sip_pcp: None,
                sip_dscp: None
            })
        );
        // Neighbors that are not connectivity devices are ignored.
        let mut phone = both.clone();
        phone.med_device_class = Some(MED_CLASS_ENDPOINT_III);
        assert_eq!(select_voice_policy(&[phone.clone()]), None);
        assert!(!have_med_policy(&[phone.clone()]));
        assert!(have_med_policy(&[phone, both]));
    }
}
