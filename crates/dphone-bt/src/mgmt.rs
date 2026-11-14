//! The kernel's Bluetooth management interface: packet encoding and
//! decoding.  The layouts are those of `include/net/bluetooth/mgmt.h` in
//! Linux 7.3.  No sockets here; see [`crate::sock`].

use crate::Addr;

/// "No controller": for commands and events that are not about one.
pub const INDEX_NONE: u16 = 0xffff;

/// BR/EDR address type in `mgmt_addr_info`.
pub const ADDR_BREDR: u8 = 0;

pub const NAME_LEN: usize = 249;
pub const SHORT_NAME_LEN: usize = 11;

pub mod op {
    pub const READ_INDEX_LIST: u16 = 0x0003;
    pub const READ_INFO: u16 = 0x0004;
    pub const SET_POWERED: u16 = 0x0005;
    pub const SET_DISCOVERABLE: u16 = 0x0006;
    pub const SET_CONNECTABLE: u16 = 0x0007;
    pub const SET_BONDABLE: u16 = 0x0009;
    pub const SET_SSP: u16 = 0x000b;
    pub const SET_DEV_CLASS: u16 = 0x000e;
    pub const SET_LOCAL_NAME: u16 = 0x000f;
    pub const ADD_UUID: u16 = 0x0010;
    pub const LOAD_LINK_KEYS: u16 = 0x0012;
    pub const DISCONNECT: u16 = 0x0014;
    pub const PIN_CODE_REPLY: u16 = 0x0016;
    pub const PIN_CODE_NEG_REPLY: u16 = 0x0017;
    pub const SET_IO_CAPABILITY: u16 = 0x0018;
    pub const PAIR_DEVICE: u16 = 0x0019;
    pub const CANCEL_PAIR_DEVICE: u16 = 0x001a;
    pub const UNPAIR_DEVICE: u16 = 0x001b;
    pub const USER_CONFIRM_REPLY: u16 = 0x001c;
    pub const USER_CONFIRM_NEG_REPLY: u16 = 0x001d;
    pub const START_DISCOVERY: u16 = 0x0023;
    pub const STOP_DISCOVERY: u16 = 0x0024;
}

mod ev {
    pub const CMD_COMPLETE: u16 = 0x0001;
    pub const CMD_STATUS: u16 = 0x0002;
    pub const INDEX_ADDED: u16 = 0x0004;
    pub const INDEX_REMOVED: u16 = 0x0005;
    pub const NEW_SETTINGS: u16 = 0x0006;
    pub const NEW_LINK_KEY: u16 = 0x0009;
    pub const DEVICE_CONNECTED: u16 = 0x000b;
    pub const DEVICE_DISCONNECTED: u16 = 0x000c;
    pub const CONNECT_FAILED: u16 = 0x000d;
    pub const PIN_CODE_REQUEST: u16 = 0x000e;
    pub const USER_CONFIRM_REQUEST: u16 = 0x000f;
    pub const AUTH_FAILED: u16 = 0x0011;
    pub const DEVICE_FOUND: u16 = 0x0012;
    pub const DISCOVERING: u16 = 0x0013;
}

/// `current_settings` bit: the controller is powered.
pub const SETTING_POWERED: u32 = 1 << 0;

/// IO capability "display with yes/no": the phone has a screen and keys.
pub const IO_DISPLAY_YES_NO: u8 = 0x01;

/// A stored link key (`mgmt_link_key_info` without the address type, which
/// is always BR/EDR here).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkKey {
    pub addr: Addr,
    pub key_type: u8,
    pub value: [u8; 16],
    pub pin_len: u8,
}

/// One command packet: header and parameters.
pub fn command(opcode: u16, index: u16, params: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(6 + params.len());
    p.extend_from_slice(&opcode.to_le_bytes());
    p.extend_from_slice(&index.to_le_bytes());
    p.extend_from_slice(&(params.len() as u16).to_le_bytes());
    p.extend_from_slice(params);
    p
}

fn addr_info(addr: Addr) -> [u8; 7] {
    let mut out = [0u8; 7];
    out[..6].copy_from_slice(&addr.0);
    out[6] = ADDR_BREDR;
    out
}

/// A command whose only parameter is a mode byte (power, connectable,
/// bondable, SSP, IO capability).
pub fn set_mode(opcode: u16, index: u16, value: u8) -> Vec<u8> {
    command(opcode, index, &[value])
}

pub fn set_discoverable(index: u16, on: bool, timeout_s: u16) -> Vec<u8> {
    let t = timeout_s.to_le_bytes();
    command(op::SET_DISCOVERABLE, index, &[on as u8, t[0], t[1]])
}

pub fn set_dev_class(index: u16, major: u8, minor: u8) -> Vec<u8> {
    command(op::SET_DEV_CLASS, index, &[major, minor])
}

pub fn set_local_name(index: u16, name: &str) -> Vec<u8> {
    let mut params = [0u8; NAME_LEN + SHORT_NAME_LEN];
    // Both fields are NUL-terminated; cut on a character boundary.
    let mut end = name.len().min(NAME_LEN - 1);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    params[..end].copy_from_slice(&name.as_bytes()[..end]);
    let mut short = end.min(SHORT_NAME_LEN - 1);
    while !name.is_char_boundary(short) {
        short -= 1;
    }
    params[NAME_LEN..NAME_LEN + short].copy_from_slice(&name.as_bytes()[..short]);
    command(op::SET_LOCAL_NAME, index, &params)
}

/// Register a 16-bit service class; `hint` is ORed into the service bits of
/// the class of device.
pub fn add_uuid16(index: u16, uuid: u16, hint: u8) -> Vec<u8> {
    // The Bluetooth base UUID, least significant byte first, with the
    // 16-bit value at bytes 12 and 13.
    let mut params = [
        0xfb, 0x34, 0x9b, 0x5f, 0x80, 0x00, 0x00, 0x80, 0x00, 0x10, 0x00, 0x00, 0, 0, 0x00, 0x00, 0,
    ];
    params[12..14].copy_from_slice(&uuid.to_le_bytes());
    params[16] = hint;
    command(op::ADD_UUID, index, &params)
}

pub fn load_link_keys(index: u16, keys: &[LinkKey]) -> Vec<u8> {
    let mut params = Vec::with_capacity(3 + keys.len() * 25);
    params.push(0); // no debug keys
    params.extend_from_slice(&(keys.len() as u16).to_le_bytes());
    for k in keys {
        params.extend_from_slice(&addr_info(k.addr));
        params.push(k.key_type);
        params.extend_from_slice(&k.value);
        params.push(k.pin_len);
    }
    command(op::LOAD_LINK_KEYS, index, &params)
}

/// A command whose only parameter is a device address (disconnect, cancel
/// pairing, confirm replies, negative PIN reply).
pub fn for_device(opcode: u16, index: u16, addr: Addr) -> Vec<u8> {
    command(opcode, index, &addr_info(addr))
}

pub fn pair_device(index: u16, addr: Addr, io_cap: u8) -> Vec<u8> {
    let mut params = addr_info(addr).to_vec();
    params.push(io_cap);
    command(op::PAIR_DEVICE, index, &params)
}

pub fn unpair_device(index: u16, addr: Addr, disconnect: bool) -> Vec<u8> {
    let mut params = addr_info(addr).to_vec();
    params.push(disconnect as u8);
    command(op::UNPAIR_DEVICE, index, &params)
}

pub fn pin_code_reply(index: u16, addr: Addr, pin: &[u8]) -> Vec<u8> {
    let len = pin.len().min(16);
    let mut params = addr_info(addr).to_vec();
    params.push(len as u8);
    let mut code = [0u8; 16];
    code[..len].copy_from_slice(&pin[..len]);
    params.extend_from_slice(&code);
    command(op::PIN_CODE_REPLY, index, &params)
}

pub fn start_discovery(index: u16) -> Vec<u8> {
    // Address type mask: BR/EDR only.
    command(op::START_DISCOVERY, index, &[1 << ADDR_BREDR])
}

pub fn stop_discovery(index: u16) -> Vec<u8> {
    command(op::STOP_DISCOVERY, index, &[1 << ADDR_BREDR])
}

/// Controller information (`mgmt_rp_read_info`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Info {
    pub addr: Addr,
    pub current_settings: u32,
    pub name: String,
}

/// A decoded event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MgmtEvent {
    /// A command finished.  `status` 0 is success.
    Complete { opcode: u16, status: u8, params: Vec<u8> },
    IndexAdded,
    IndexRemoved,
    NewSettings(u32),
    NewLinkKey { store: bool, key: LinkKey },
    Connected { addr: Addr, name: Option<String> },
    Disconnected { addr: Addr },
    ConnectFailed { addr: Addr, status: u8 },
    PinRequest { addr: Addr },
    /// `hint`: the kernel only wants a yes or no (no number to compare).
    ConfirmRequest { addr: Addr, hint: bool, value: u32 },
    AuthFailed { addr: Addr, status: u8 },
    Found { addr: Addr, rssi: i8, name: Option<String>, class: Option<u32> },
    Discovering(bool),
    Other(u16),
}

fn addr_at(p: &[u8]) -> Option<Addr> {
    Some(Addr(p.get(..6)?.try_into().ok()?))
}

fn le16(p: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(p.get(at..at + 2)?.try_into().ok()?))
}

fn le32(p: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(p.get(at..at + 4)?.try_into().ok()?))
}

/// Parse one packet read from the management socket.  Returns the
/// controller index and the event; `None` for a malformed packet.
pub fn parse_event(packet: &[u8]) -> Option<(u16, MgmtEvent)> {
    let code = le16(packet, 0)?;
    let index = le16(packet, 2)?;
    let len = le16(packet, 4)? as usize;
    let p = packet.get(6..6 + len)?;
    let event = match code {
        ev::CMD_COMPLETE => MgmtEvent::Complete {
            opcode: le16(p, 0)?,
            status: *p.get(2)?,
            params: p[3..].to_vec(),
        },
        ev::CMD_STATUS => MgmtEvent::Complete { opcode: le16(p, 0)?, status: *p.get(2)?, params: Vec::new() },
        ev::INDEX_ADDED => MgmtEvent::IndexAdded,
        ev::INDEX_REMOVED => MgmtEvent::IndexRemoved,
        ev::NEW_SETTINGS => MgmtEvent::NewSettings(le32(p, 0)?),
        ev::NEW_LINK_KEY => MgmtEvent::NewLinkKey {
            store: *p.first()? != 0,
            key: LinkKey {
                addr: addr_at(p.get(1..)?)?,
                key_type: *p.get(8)?,
                value: p.get(9..25)?.try_into().ok()?,
                pin_len: *p.get(25)?,
            },
        },
        ev::DEVICE_CONNECTED => {
            let eir_len = le16(p, 11)? as usize;
            let eir = Eir::parse(p.get(13..13 + eir_len)?);
            MgmtEvent::Connected { addr: addr_at(p)?, name: eir.name }
        }
        ev::DEVICE_DISCONNECTED => MgmtEvent::Disconnected { addr: addr_at(p)? },
        ev::CONNECT_FAILED => MgmtEvent::ConnectFailed { addr: addr_at(p)?, status: *p.get(7)? },
        ev::PIN_CODE_REQUEST => MgmtEvent::PinRequest { addr: addr_at(p)? },
        ev::USER_CONFIRM_REQUEST => {
            MgmtEvent::ConfirmRequest { addr: addr_at(p)?, hint: *p.get(7)? != 0, value: le32(p, 8)? }
        }
        ev::AUTH_FAILED => MgmtEvent::AuthFailed { addr: addr_at(p)?, status: *p.get(7)? },
        ev::DEVICE_FOUND => {
            let eir_len = le16(p, 12)? as usize;
            let eir = Eir::parse(p.get(14..14 + eir_len)?);
            MgmtEvent::Found { addr: addr_at(p)?, rssi: *p.get(7)? as i8, name: eir.name, class: eir.class }
        }
        ev::DISCOVERING => MgmtEvent::Discovering(*p.get(1)? != 0),
        other => MgmtEvent::Other(other),
    };
    Some((index, event))
}

/// Parse the reply parameters of `READ_INFO`.
pub fn parse_info(params: &[u8]) -> Option<Info> {
    // bdaddr 6, version 1, manufacturer 2, supported 4, current 4,
    // class 3, name 249, short name 11.
    let name = params.get(20..20 + NAME_LEN)?;
    let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    Some(Info {
        addr: addr_at(params)?,
        current_settings: le32(params, 13)?,
        name: String::from_utf8_lossy(&name[..end]).into_owned(),
    })
}

/// Parse the reply parameters of `READ_INDEX_LIST`.
pub fn parse_index_list(params: &[u8]) -> Option<Vec<u16>> {
    let n = le16(params, 0)? as usize;
    (0..n).map(|i| le16(params, 2 + 2 * i)).collect()
}

/// The parts of extended inquiry response data this crate uses.
#[derive(Default)]
struct Eir {
    name: Option<String>,
    class: Option<u32>,
}

impl Eir {
    fn parse(mut data: &[u8]) -> Eir {
        let mut out = Eir::default();
        while let Some((&len, rest)) = data.split_first() {
            let len = len as usize;
            if len == 0 || rest.len() < len {
                break;
            }
            let (field, tail) = rest.split_at(len);
            match field[0] {
                // Complete name wins over a shortened one.
                0x09 => out.name = Some(String::from_utf8_lossy(&field[1..]).into_owned()),
                0x08 if out.name.is_none() => {
                    out.name = Some(String::from_utf8_lossy(&field[1..]).into_owned());
                }
                0x0d if field.len() >= 4 => {
                    out.class = Some(u32::from_le_bytes([field[1], field[2], field[3], 0]));
                }
                _ => {}
            }
            data = tail;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(code: u16, index: u16, params: &[u8]) -> Vec<u8> {
        command(code, index, params)
    }

    #[test]
    fn command_header() {
        assert_eq!(set_mode(op::SET_POWERED, 0, 1), [0x05, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01]);
        assert_eq!(command(op::READ_INDEX_LIST, INDEX_NONE, &[]), [0x03, 0x00, 0xff, 0xff, 0x00, 0x00]);
    }

    #[test]
    fn local_name_is_padded_and_terminated() {
        let p = set_local_name(0, "Open DPhone D65");
        assert_eq!(p.len(), 6 + 260);
        assert_eq!(&p[6..21], b"Open DPhone D65");
        assert_eq!(p[21], 0);
        // Short name: at most ten bytes and a NUL.
        assert_eq!(&p[6 + 249..6 + 259], b"Open DPhon");
        assert_eq!(p[6 + 259], 0);
    }

    #[test]
    fn uuid16_in_base_uuid() {
        let p = add_uuid16(0, 0x111f, 0x40);
        assert_eq!(p.len(), 6 + 17);
        assert_eq!(&p[6 + 12..6 + 14], [0x1f, 0x11]);
        assert_eq!(p[6 + 16], 0x40);
    }

    #[test]
    fn link_keys_round_trip_through_the_event() {
        let key = LinkKey { addr: "00:1B:DC:0F:A1:02".parse().unwrap(), key_type: 4, value: [7; 16], pin_len: 0 };
        let load = load_link_keys(0, std::slice::from_ref(&key));
        assert_eq!(load.len(), 6 + 3 + 25);
        assert_eq!(&load[6..9], [0, 1, 0]);
        // The event carries the same 25-byte record after a store hint.
        let mut params = vec![1];
        params.extend_from_slice(&load[9..]);
        let (index, ev) = parse_event(&event(0x0009, 0, &params)).unwrap();
        assert_eq!(index, 0);
        assert_eq!(ev, MgmtEvent::NewLinkKey { store: true, key });
    }

    #[test]
    fn device_found_with_name_and_class() {
        let addr: Addr = "00:1B:DC:0F:A1:02".parse().unwrap();
        let mut p = addr.0.to_vec();
        p.push(0); // BR/EDR
        p.push(0xc4); // rssi -60
        p.extend_from_slice(&0u32.to_le_bytes());
        let eir = [4, 0x0d, 0x04, 0x04, 0x20, 8, 0x09, b'H', b'e', b'a', b'd', b's', b'e', b't'];
        p.extend_from_slice(&(eir.len() as u16).to_le_bytes());
        p.extend_from_slice(&eir);
        let (_, ev) = parse_event(&event(0x0012, 0, &p)).unwrap();
        assert_eq!(ev, MgmtEvent::Found { addr, rssi: -60, name: Some("Headset".into()), class: Some(0x200404) });
    }

    #[test]
    fn confirm_request_and_truncation() {
        let addr: Addr = "00:1B:DC:0F:A1:02".parse().unwrap();
        let mut p = addr.0.to_vec();
        p.extend_from_slice(&[0, 1]);
        p.extend_from_slice(&123456u32.to_le_bytes());
        let packet = event(0x000f, 0, &p);
        let (_, ev) = parse_event(&packet).unwrap();
        assert_eq!(ev, MgmtEvent::ConfirmRequest { addr, hint: true, value: 123456 });
        assert!(parse_event(&packet[..packet.len() - 1]).is_none());
    }

    #[test]
    fn info_and_index_list() {
        let mut p = vec![0u8; 6 + 1 + 2 + 4 + 4 + 3 + 249 + 11];
        p[..6].copy_from_slice(&[1, 2, 3, 4, 5, 6]);
        p[13..17].copy_from_slice(&1u32.to_le_bytes());
        p[20..23].copy_from_slice(b"D65");
        let info = parse_info(&p).unwrap();
        assert_eq!(info.addr.0, [1, 2, 3, 4, 5, 6]);
        assert_eq!(info.current_settings & SETTING_POWERED, 1);
        assert_eq!(info.name, "D65");
        assert_eq!(parse_index_list(&[2, 0, 0, 0, 1, 0]).unwrap(), [0, 1]);
        assert!(parse_index_list(&[2, 0, 0, 0]).is_none());
    }
}
