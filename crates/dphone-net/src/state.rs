//! The observable result of network bring-up.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::lldp::Neighbor;
use crate::qos::QosMarks;
use crate::sys::MacAddr;

/// Where bring-up stands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Starting,
    /// The switch interfaces have not appeared yet (drivers still loading).
    WaitingForInterfaces,
    WaitingForLink,
    /// Listening for the switch's LLDP-MED policy.
    LldpDiscovery,
    /// Interfaces are set up; waiting for an address.
    AcquiringAddress,
    /// An address is configured.
    Up,
}

/// One switch port.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PortState {
    pub present: bool,
    pub link: bool,
    /// Mb/s.
    pub speed: Option<u32>,
    pub full_duplex: Option<bool>,
}

/// How the voice VLAN was chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VlanSource {
    Manual,
    Lldp,
}

/// How the IPv4 address was obtained.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddressSource {
    Dhcp,
    Static,
}

/// The configured IPv4 address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ipv4State {
    pub address: Ipv4Addr,
    pub prefix_len: u8,
    pub gateways: Vec<Ipv4Addr>,
    pub source: AddressSource,
    /// DHCP only.
    pub dhcp_server: Option<Ipv4Addr>,
    /// DHCP only; `u32::MAX` is infinite.
    pub lease_secs: Option<u32>,
}

/// Everything consumers need to know about the network.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetState {
    pub phase: Phase,
    pub lan: PortState,
    pub pc: PortState,
    pub mac: Option<MacAddr>,
    /// The interface that carries the phone's IP traffic: the bridge, the
    /// LAN port, or a VLAN device on one of them.  (Stock wrote this to
    /// `/var/tmp/network-interface`.)
    pub interface: String,
    /// Whether the PC and LAN jacks are bridged.
    pub bridged: bool,
    pub vlan_id: Option<u16>,
    pub vlan_source: Option<VlanSource>,
    /// Markings for SIP and RTP sockets, after LLDP-MED overrides.
    pub qos: QosMarks,
    pub ipv4: Option<Ipv4State>,
    /// Global IPv6 addresses on the IP interface.
    pub ipv6: Vec<Ipv6Addr>,
    /// Resolvers in use, in order.
    pub dns: Vec<IpAddr>,
    pub domain: Option<String>,
    /// DHCP option 42.
    pub ntp_servers: Vec<IpAddr>,
    /// DHCP option 66; a `sip:` value names the provisioning proxy.
    pub tftp_server: Option<String>,
    /// DHCP option 67.
    pub bootfile: Option<String>,
    /// Every option in the current DHCPv4 lease.
    pub dhcp_options: BTreeMap<u8, Vec<u8>>,
    /// Whether SNTP has succeeded since boot.
    pub time_synced: bool,
    pub lldp_neighbors: Vec<Neighbor>,
    /// The phone's address on the expansion-module VLAN, once the
    /// interface for it exists.
    pub expansion: Option<Ipv4Addr>,
    /// The most recent problem worth showing a user, if any.
    pub last_error: Option<String>,
}

impl NetState {
    /// True once an IPv4 or global IPv6 address is configured.
    pub fn has_address(&self) -> bool {
        self.ipv4.is_some() || !self.ipv6.is_empty()
    }
}
