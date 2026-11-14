//! How the switch ports, the bridge, and VLANs are arranged.
//!
//! Under the mainline DSA driver the two jacks are separate interfaces
//! (`lan` and `pc`) that do not forward to each other until a bridge joins
//! them; the switch then forwards in hardware.  Stock programmed the switch
//! directly instead.  This module turns the stock settings into a plan of
//! bridge and VLAN operations, and applies it.

use std::io;

use crate::config::{NetConfig, PcPort};
use crate::ethtool::PortMode;
use crate::netlink::{BridgeOpts, Netlink, VlanFlags};
use crate::sys::MacAddr;

/// The bridge's default VLAN for untagged traffic when VLAN filtering is on.
pub const NATIVE_VID: u16 = 1;

/// Bit in the bridge's group-forward mask for `01:80:c2:00:00:03`, the
/// 802.1X PAE address.
pub const GROUP_FWD_EAPOL: u16 = 1 << 3;

/// One bridge VLAN table entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VlanEntry {
    /// Interface the entry is on.
    pub ifname: String,
    pub vid: u16,
    pub flags: VlanFlags,
    /// The entry is on the bridge device itself (the phone's own side).
    pub on_bridge: bool,
}

/// The arrangement to set up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// `Some` when the jacks are bridged.
    pub bridge: Option<BridgeOpts>,
    /// Whether the PC port is brought up.
    pub pc_up: bool,
    /// Interface untagged phone traffic uses: the bridge or the LAN port.
    pub base_if: String,
    /// Bridge VLAN entries to add.
    pub add: Vec<VlanEntry>,
    /// Bridge VLAN entries to remove (ports leaving the native VLAN).
    pub remove: Vec<(String, u16)>,
    /// The voice VLAN device to create on `base_if`, if any.
    pub voice_vid: Option<u16>,
}

fn valid_vid(v: u16) -> bool {
    (1..=4094).contains(&v)
}

/// Compute the arrangement.
///
/// * `has_pc`: the PC port interface exists.
/// * `bridge_ok`: the kernel can create bridges.
/// * `voice_vid`: the voice VLAN decided on (manual or LLDP-MED).
pub fn plan(cfg: &NetConfig, has_pc: bool, bridge_ok: bool, voice_vid: Option<u16>) -> Plan {
    let voice_vid = voice_vid.filter(|v| valid_vid(*v));
    let pc_vlan = cfg.pc_vlan_id.filter(|v| valid_vid(*v));
    let pc_wanted = has_pc && cfg.pc_port == PcPort::Bridged && cfg.pc_port_mode != PortMode::Off;
    let names = &cfg.ifaces;

    if !pc_wanted || !bridge_ok {
        return Plan {
            bridge: None,
            pc_up: false,
            base_if: names.lan.clone(),
            add: Vec::new(),
            remove: Vec::new(),
            voice_vid,
        };
    }
    let pc = names.pc.clone().expect("has_pc implies a PC port name");
    let group_fwd_mask = if cfg.eapol_passthrough {
        GROUP_FWD_EAPOL
    } else {
        0
    };

    if voice_vid.is_none() && pc_vlan.is_none() {
        // A plain transparent bridge, like the stock switch with VLANs off.
        return Plan {
            bridge: Some(BridgeOpts {
                vlan_filtering: false,
                group_fwd_mask,
                default_pvid: NATIVE_VID,
            }),
            pc_up: true,
            base_if: names.bridge.clone(),
            add: Vec::new(),
            remove: Vec::new(),
            voice_vid: None,
        };
    }

    // VLANs are in play: filter, with explicit membership.  Every port
    // starts in the native VLAN (the bridge's default PVID).
    let tagged = VlanFlags::default();
    let mut add = Vec::new();
    let mut remove = Vec::new();
    if let Some(v) = voice_vid {
        add.push(VlanEntry {
            ifname: names.lan.clone(),
            vid: v,
            flags: tagged,
            on_bridge: false,
        });
        add.push(VlanEntry {
            ifname: names.bridge.clone(),
            vid: v,
            flags: tagged,
            on_bridge: true,
        });
    }
    match pc_vlan {
        Some(p) => {
            // Untagged PC traffic lives in the PC VLAN, tagged on the LAN
            // side and untagged on the PC side, apart from the phone's
            // native VLAN (stock: PVID on port 1, untag mask 0x002).
            remove.push((pc.clone(), NATIVE_VID));
            add.push(VlanEntry {
                ifname: pc.clone(),
                vid: p,
                flags: VlanFlags {
                    pvid: true,
                    untagged: true,
                },
                on_bridge: false,
            });
            if Some(p) != voice_vid {
                add.push(VlanEntry {
                    ifname: names.lan.clone(),
                    vid: p,
                    flags: tagged,
                    on_bridge: false,
                });
            }
        }
        None => {
            // Let a device behind the PC jack (e.g., a second phone) reach
            // the voice VLAN with tags, as the transparent stock switch did.
            if let Some(v) = voice_vid {
                add.push(VlanEntry {
                    ifname: pc.clone(),
                    vid: v,
                    flags: tagged,
                    on_bridge: false,
                });
            }
        }
    }
    Plan {
        bridge: Some(BridgeOpts {
            vlan_filtering: true,
            group_fwd_mask,
            default_pvid: NATIVE_VID,
        }),
        pc_up: true,
        base_if: names.bridge.clone(),
        add,
        remove,
        voice_vid,
    }
}

/// Name of the voice VLAN device on `base_if`.
pub fn vlan_ifname(base_if: &str, vid: u16) -> String {
    format!("{base_if}.{vid}")
}

async fn index(nl: &mut Netlink, name: &str) -> io::Result<u32> {
    nl.link_by_name(name)
        .await?
        .map(|l| l.index)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no interface {name}")))
}

/// Whether an error means "the kernel lacks this feature".
pub fn is_unsupported(e: &io::Error) -> bool {
    matches!(
        e.raw_os_error(),
        Some(libc::EOPNOTSUPP)
            | Some(libc::EAFNOSUPPORT)
            | Some(libc::EPROTONOSUPPORT)
            | Some(libc::ENOSYS)
    )
}

/// Create the bridge (if planned), enslave the ports, and install the VLAN
/// entries.  The ports must already carry the right MAC address.  Leaves
/// every interface in the plan administratively up, except a PC port the
/// plan keeps down.
pub async fn apply(
    nl: &mut Netlink,
    cfg: &NetConfig,
    plan: &Plan,
    mac: Option<MacAddr>,
) -> io::Result<()> {
    let names = &cfg.ifaces;
    let lan = index(nl, &names.lan).await?;
    let pc = match &names.pc {
        Some(p) => nl.link_by_name(p).await?.map(|l| l.index),
        None => None,
    };
    if let Some(opts) = &plan.bridge {
        let br = match nl.link_by_name(&names.bridge).await? {
            Some(l) => {
                nl.set_bridge_opts(l.index, opts).await?;
                l.index
            }
            None => {
                nl.add_bridge(&names.bridge, opts).await?;
                index(nl, &names.bridge).await?
            }
        };
        if let Some(m) = mac {
            // Pin the bridge's address so it does not follow its ports.
            nl.set_mac(br, m).await?;
        }
        nl.set_master(lan, Some(br)).await?;
        if let Some(pc) = pc {
            nl.set_master(pc, Some(br)).await?;
        }
        for (ifname, vid) in &plan.remove {
            let i = index(nl, ifname).await?;
            if let Err(e) = nl.bridge_vlan_del(i, *vid, false).await {
                log::debug!("net: removing VLAN {vid} from {ifname}: {e}");
            }
        }
        for e in &plan.add {
            let i = index(nl, &e.ifname).await?;
            nl.bridge_vlan_add(i, e.vid, e.flags, e.on_bridge).await?;
        }
        nl.set_up(br, true).await?;
    }
    nl.set_up(lan, true).await?;
    if let Some(pc) = pc {
        nl.set_up(pc, plan.pc_up).await?;
    }
    Ok(())
}

/// Undo [`apply`]: delete the voice VLAN device and the bridge.  Missing
/// pieces are not an error.
pub async fn teardown(nl: &mut Netlink, cfg: &NetConfig) {
    let names = &cfg.ifaces;
    if let Ok(links) = nl.links().await {
        // VLAN devices this crate made are named `<base>.<vid>`.
        for l in &links {
            let ours = l.kind.as_deref() == Some("vlan")
                && (l.name.starts_with(&format!("{}.", names.bridge))
                    || l.name.starts_with(&format!("{}.", names.lan)));
            if ours {
                let _ = nl.del_link(l.index).await;
            }
        }
        if let Some(br) = links
            .iter()
            .find(|l| l.name == names.bridge && l.kind.as_deref() == Some("bridge"))
        {
            let _ = nl.del_link(br.index).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> NetConfig {
        NetConfig::new("D60", "3_0_0", MacAddr::parse("00:0f:d3:01:02:03"))
    }

    #[test]
    fn default_is_a_transparent_bridge() {
        let p = plan(&cfg(), true, true, None);
        assert_eq!(
            p.bridge,
            Some(BridgeOpts {
                vlan_filtering: false,
                group_fwd_mask: 0,
                default_pvid: 1
            })
        );
        assert_eq!(p.base_if, "br0");
        assert!(p.pc_up && p.add.is_empty() && p.remove.is_empty());
    }

    #[test]
    fn no_bridge_without_a_pc_port_or_bridge_support() {
        for p in [
            plan(&cfg(), false, true, Some(100)),
            plan(&cfg(), true, false, Some(100)),
        ] {
            assert_eq!(p.bridge, None);
            assert_eq!(p.base_if, "lan");
            assert!(!p.pc_up);
            assert_eq!(p.voice_vid, Some(100));
        }
        let mut c = cfg();
        c.pc_port = PcPort::Disabled;
        assert_eq!(plan(&c, true, true, None).bridge, None);
        let mut c = cfg();
        c.pc_port_mode = PortMode::Off;
        assert!(!plan(&c, true, true, None).pc_up);
    }

    #[test]
    fn voice_vlan_filters_and_tags() {
        let p = plan(&cfg(), true, true, Some(100));
        assert!(p.bridge.unwrap().vlan_filtering);
        let tagged = VlanFlags::default();
        assert_eq!(
            p.add,
            vec![
                VlanEntry {
                    ifname: "lan".into(),
                    vid: 100,
                    flags: tagged,
                    on_bridge: false
                },
                VlanEntry {
                    ifname: "br0".into(),
                    vid: 100,
                    flags: tagged,
                    on_bridge: true
                },
                VlanEntry {
                    ifname: "pc".into(),
                    vid: 100,
                    flags: tagged,
                    on_bridge: false
                },
            ]
        );
        assert!(p.remove.is_empty());
        assert_eq!(vlan_ifname(&p.base_if, 100), "br0.100");
    }

    #[test]
    fn pc_vlan_moves_the_pc_port() {
        let mut c = cfg();
        c.pc_vlan_id = Some(20);
        c.eapol_passthrough = true;
        let p = plan(&c, true, true, Some(100));
        assert_eq!(p.bridge.unwrap().group_fwd_mask, GROUP_FWD_EAPOL);
        assert_eq!(p.remove, vec![("pc".to_string(), 1)]);
        assert!(p.add.contains(&VlanEntry {
            ifname: "pc".into(),
            vid: 20,
            flags: VlanFlags {
                pvid: true,
                untagged: true
            },
            on_bridge: false
        }));
        assert!(p.add.contains(&VlanEntry {
            ifname: "lan".into(),
            vid: 20,
            flags: VlanFlags::default(),
            on_bridge: false
        }));
        // The PC does not get the voice VLAN when it has its own.
        assert!(!p.add.iter().any(|e| e.ifname == "pc" && e.vid == 100));
        // Out-of-range IDs are ignored.
        c.pc_vlan_id = Some(4095);
        assert!(!plan(&c, true, true, None).bridge.unwrap().vlan_filtering);
        assert_eq!(plan(&c, true, true, Some(0)).voice_vid, None);
    }
}
