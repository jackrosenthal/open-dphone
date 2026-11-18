//! EAPOL logoff on behalf of a device behind the PC jack.
//!
//! With 802.1X pass-through, the switch port upstream authenticates the PC,
//! not the phone, and stays authorized after the PC is unplugged, because
//! the upstream port never loses link.  The stock firmware closed that hole
//! by sending an EAPOL-Logoff for the PC when the PC jack lost link
//! (`eapLogoffEnable`).  This module does the same: it remembers the address
//! of the supplicant seen on the PC jack and, when that jack goes down,
//! sends a logoff from that address out of the LAN jack.

use std::io;

use tokio::sync::mpsc;

use crate::packet::RawSocket;
use crate::sys::MacAddr;

/// EtherType of EAP over LAN.
pub const ETH_P_PAE: u16 = 0x888e;
/// The port access entity group address.
pub const PAE_GROUP: MacAddr = MacAddr([0x01, 0x80, 0xc2, 0x00, 0x00, 0x03]);

const EAPOL_TYPE_LOGOFF: u8 = 2;

/// What an EAPOL frame from the PC jack says about its sender.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seen {
    /// A supplicant with this address is talking to the authenticator.
    Supplicant(MacAddr),
    /// The supplicant logged itself off; nothing is left to do for it.
    LoggedOff(MacAddr),
}

/// Classify an Ethernet frame received on the PC jack.
pub fn classify(frame: &[u8]) -> Option<Seen> {
    if frame.len() < 18 || frame[12..14] != ETH_P_PAE.to_be_bytes() {
        return None;
    }
    let mut mac = [0u8; 6];
    mac.copy_from_slice(&frame[6..12]);
    let mac = MacAddr(mac);
    if !mac.is_valid_unicast() {
        return None;
    }
    Some(if frame[15] == EAPOL_TYPE_LOGOFF { Seen::LoggedOff(mac) } else { Seen::Supplicant(mac) })
}

/// An EAPOL-Logoff frame from `supplicant` to the PAE group address.
pub fn logoff_frame(supplicant: MacAddr) -> Vec<u8> {
    let mut f = Vec::with_capacity(60);
    f.extend_from_slice(&PAE_GROUP.0);
    f.extend_from_slice(&supplicant.0);
    f.extend_from_slice(&ETH_P_PAE.to_be_bytes());
    // Protocol version 1, type Logoff, body length zero.
    f.extend_from_slice(&[1, EAPOL_TYPE_LOGOFF, 0, 0]);
    f.resize(60, 0);
    f
}

/// Handle to the task that watches the PC jack.
pub struct LogoffProxy {
    link_down: mpsc::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

impl LogoffProxy {
    /// Start watching `pc` and sending logoffs out of `lan` (interface
    /// indexes).  Must be called inside a tokio runtime.
    pub fn spawn(pc: u32, lan: u32) -> io::Result<Self> {
        let rx = RawSocket::new(pc, ETH_P_PAE)?;
        let tx = RawSocket::new(lan, ETH_P_PAE)?;
        let (link_down, mut down) = mpsc::channel(4);
        let task = tokio::spawn(async move {
            let mut supplicant: Option<MacAddr> = None;
            let mut buf = [0u8; 1600];
            loop {
                tokio::select! {
                    r = rx.recv(&mut buf) => match r {
                        Ok(n) => match classify(&buf[..n]) {
                            Some(Seen::Supplicant(mac)) => supplicant = Some(mac),
                            Some(Seen::LoggedOff(mac)) if supplicant == Some(mac) => supplicant = None,
                            _ => {}
                        },
                        // The jack going down surfaces here as an error.
                        Err(_) => tokio::time::sleep(std::time::Duration::from_millis(200)).await,
                    },
                    d = down.recv() => {
                        if d.is_none() {
                            return;
                        }
                        if let Some(mac) = supplicant.take() {
                            log::info!("net: PC jack lost link; logging {mac} off the 802.1X port");
                            if let Err(e) = tx.send(&logoff_frame(mac)).await {
                                log::warn!("net: sending the EAPOL logoff: {e}");
                            }
                        }
                    }
                }
            }
        });
        Ok(Self { link_down, task })
    }

    /// The PC jack lost link.
    pub async fn pc_link_down(&self) {
        let _ = self.link_down.send(()).await;
    }
}

impl Drop for LogoffProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logoff_layout() {
        let pc = MacAddr([0x02, 0, 0, 0, 0, 0x77]);
        let f = logoff_frame(pc);
        assert_eq!(f.len(), 60);
        assert_eq!(&f[..18], &[0x01, 0x80, 0xc2, 0, 0, 0x03, 0x02, 0, 0, 0, 0, 0x77, 0x88, 0x8e, 1, 2, 0, 0]);
        assert_eq!(classify(&f), Some(Seen::LoggedOff(pc)));
    }

    #[test]
    fn learns_the_supplicant() {
        let pc = MacAddr([0x02, 0, 0, 0, 0, 0x77]);
        let mut start = logoff_frame(pc);
        start[15] = 1; // EAPOL-Start
        assert_eq!(classify(&start), Some(Seen::Supplicant(pc)));
        let mut other = start.clone();
        other[12] = 0x08;
        assert_eq!(classify(&other), None);
        assert_eq!(classify(&start[..16]), None);
        // A multicast source address is not a supplicant.
        start[6] = 0x01;
        assert_eq!(classify(&start), None);
    }
}
