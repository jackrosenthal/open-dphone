//! Link speed, duplex, and advertisement through the legacy `ethtool`
//! ioctl.  The phone's kernel is built without `ETHTOOL_NETLINK`.

use std::io;
use std::mem;
use std::os::fd::AsRawFd;

use crate::sys;

const SIOCETHTOOL: u32 = 0x8946;
const ETHTOOL_GSET: u32 = 1;
const ETHTOOL_SSET: u32 = 2;
const ETHTOOL_NWAY_RST: u32 = 9;

const ADVERTISED_10_HALF: u32 = 1 << 0;
const ADVERTISED_10_FULL: u32 = 1 << 1;
const ADVERTISED_100_HALF: u32 = 1 << 2;
const ADVERTISED_100_FULL: u32 = 1 << 3;
const ADVERTISED_1000_HALF: u32 = 1 << 4;
const ADVERTISED_1000_FULL: u32 = 1 << 5;
const ALL_SPEEDS: u32 = ADVERTISED_10_HALF
    | ADVERTISED_10_FULL
    | ADVERTISED_100_HALF
    | ADVERTISED_100_FULL
    | ADVERTISED_1000_HALF
    | ADVERTISED_1000_FULL;
const AUTONEG_ENABLE: u8 = 1;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct EthtoolCmd {
    cmd: u32,
    supported: u32,
    advertising: u32,
    speed: u16,
    duplex: u8,
    port: u8,
    phy_address: u8,
    transceiver: u8,
    autoneg: u8,
    mdio_support: u8,
    maxtxpkt: u32,
    maxrxpkt: u32,
    speed_hi: u16,
    eth_tp_mdix: u8,
    eth_tp_mdix_ctrl: u8,
    lp_advertising: u32,
    reserved: [u32; 2],
}

#[repr(C)]
struct EthtoolValue {
    cmd: u32,
    data: u32,
}

/// The `lan_port_mode` and `pc_port_mode` vocabulary.  As in the stock
/// firmware, a fixed mode restricts what autonegotiation advertises; it
/// does not force the link.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PortMode {
    #[default]
    Auto,
    Half10,
    Full10,
    Half100,
    Full100,
    Full1000,
    /// PC port only: the port is taken down.
    Off,
}

impl PortMode {
    /// Parse a setting value.  Unknown values become `Auto`, as stock does.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "10hd" => Self::Half10,
            "10fd" => Self::Full10,
            "100hd" => Self::Half100,
            "100fd" => Self::Full100,
            "1000fd" => Self::Full1000,
            "off" => Self::Off,
            _ => Self::Auto,
        }
    }

    fn advertise_mask(self) -> u32 {
        match self {
            Self::Auto | Self::Off => ALL_SPEEDS,
            Self::Half10 => ADVERTISED_10_HALF,
            Self::Full10 => ADVERTISED_10_FULL,
            Self::Half100 => ADVERTISED_100_HALF,
            Self::Full100 => ADVERTISED_100_FULL,
            Self::Full1000 => ADVERTISED_1000_FULL,
        }
    }
}

/// Negotiated link parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkMode {
    /// Mb/s; `None` when the link is down or the speed is unknown.
    pub speed: Option<u32>,
    pub full_duplex: Option<bool>,
}

fn ioctl<T>(ifname: &str, data: &mut T) -> io::Result<()> {
    let sock = sys::socket(libc::AF_INET, libc::SOCK_DGRAM, 0)?;
    // SAFETY: zero is a valid bit pattern for `ifreq`.
    let mut ifr: libc::ifreq = unsafe { mem::zeroed() };
    let name = ifname.as_bytes();
    if name.len() >= ifr.ifr_name.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "interface name too long",
        ));
    }
    for (d, s) in ifr.ifr_name.iter_mut().zip(name) {
        *d = *s as libc::c_char;
    }
    ifr.ifr_ifru.ifru_data = data as *mut T as *mut libc::c_char;
    // SAFETY: `ifr` names the interface and points at a command structure
    // that lives for the duration of the call.
    sys::cvt(unsafe { libc::ioctl(sock.as_raw_fd(), SIOCETHTOOL as _, &mut ifr) })?;
    Ok(())
}

/// Read the negotiated speed and duplex.
pub fn link_mode(ifname: &str) -> io::Result<LinkMode> {
    let mut cmd = EthtoolCmd {
        cmd: ETHTOOL_GSET,
        ..Default::default()
    };
    ioctl(ifname, &mut cmd)?;
    let speed = (cmd.speed_hi as u32) << 16 | cmd.speed as u32;
    Ok(LinkMode {
        speed: if speed == 0 || speed == 0xffff || speed == u32::MAX {
            None
        } else {
            Some(speed)
        },
        full_duplex: match cmd.duplex {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        },
    })
}

/// Restrict the advertised modes to `mode` (intersected with what the PHY
/// supports) and restart autonegotiation.  Returns `Ok(false)` when the PHY
/// supports none of the requested modes, in which case nothing is changed.
pub fn set_port_mode(ifname: &str, mode: PortMode) -> io::Result<bool> {
    let mut cmd = EthtoolCmd {
        cmd: ETHTOOL_GSET,
        ..Default::default()
    };
    ioctl(ifname, &mut cmd)?;
    let wanted = cmd.supported & mode.advertise_mask();
    if wanted == 0 {
        // Nothing to restrict on a device that reports no modes at all.
        return Ok(mode == PortMode::Auto);
    }
    let advertising = (cmd.advertising & !ALL_SPEEDS & cmd.supported) | wanted;
    if advertising == cmd.advertising && cmd.autoneg == AUTONEG_ENABLE {
        return Ok(true);
    }
    cmd.cmd = ETHTOOL_SSET;
    cmd.advertising = advertising;
    cmd.autoneg = AUTONEG_ENABLE;
    ioctl(ifname, &mut cmd)?;
    let mut rst = EthtoolValue {
        cmd: ETHTOOL_NWAY_RST,
        data: 0,
    };
    // Setting the advertisement already renegotiates on most drivers.
    let _ = ioctl(ifname, &mut rst);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_size_matches_kernel() {
        assert_eq!(mem::size_of::<EthtoolCmd>(), 44);
    }

    #[test]
    fn parse_modes() {
        assert_eq!(PortMode::parse("100FD"), PortMode::Full100);
        assert_eq!(PortMode::parse("bogus"), PortMode::Auto);
        assert_eq!(PortMode::parse("off"), PortMode::Off);
    }
}
