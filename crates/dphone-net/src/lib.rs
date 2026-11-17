//! Open Desk Phone networking.
//!
//! One crate for everything between the kernel's network devices and the
//! phone's SIP, provisioning, and web code:
//!
//! * [`netlink`]: links, addresses, routes, bridges, and VLANs over rtnetlink.
//! * [`topology`]: the PC-port bridge, PC-port VLAN, voice VLAN, and
//!   802.1X pass-through on the mainline DSA switch driver.
//! * [`dhcp4`], [`dhcp6`]: address configuration.
//! * [`dns`]: a stub resolver with the record types SIP needs.
//! * [`sntp`]: the clock.
//! * [`lldp`]: LLDP with LLDP-MED, and the stock voice-VLAN selection rule.
//! * [`mdns`]: discovery of `_digiumproxy._udp` provisioning servers.
//! * [`eapol`]: EAPOL logoff for a PC unplugged from the PC jack.
//! * [`qos`]: DSCP and 802.1p marking for sockets.
//! * [`manager`]: [`NetManager`], which runs all of the above from one
//!   [`NetConfig`] and publishes a [`NetState`].
//!
//! Everything is async for a single-threaded tokio runtime, has no C
//! dependencies, and knows nothing about the other `dphone-*` crates.

#![allow(clippy::collapsible_if, clippy::chunks_exact_to_as_chunks)]

pub mod config;
pub mod dhcp4;
pub mod dhcp6;
pub mod dns;
pub mod eapol;
pub mod ethtool;
pub mod lldp;
pub mod manager;
pub mod mdns;
pub mod netlink;
#[doc(hidden)]
pub mod packet;
pub mod qos;
pub mod sntp;
pub mod state;
mod sys;
pub mod topology;

pub use config::{
    IfaceNames, Ipv4Config, Ipv6Config, LldpConfig, NetConfig, NtpConfig, PcPort, QosConfig,
    VlanDiscovery,
};
pub use dns::Resolver;
pub use ethtool::PortMode;
pub use manager::NetManager;
pub use qos::{QosMarks, TrafficClass};
pub use state::{AddressSource, Ipv4State, NetState, Phase, PortState, VlanSource};
pub use sys::{MacAddr, if_index, parse_enet_cfg_mac, set_realtime_clock};
