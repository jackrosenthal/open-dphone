//! Keys and the hookswitch through evdev.  Devices are found by
//! capability (`EV_KEY`), never by `event0`.

use super::sys::*;
use crate::*;
use std::collections::VecDeque;
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, RawFd};
use std::time::Duration;

/// `struct input_event`.  The two time fields are `__kernel_ulong_t` on
/// 32-bit (16 bytes total) and `long` on 64-bit (24 bytes).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct InputEventRaw {
    sec: usize,
    usec: usize,
    type_: u16,
    code: u16,
    value: i32,
}

const EV_KEY: u16 = 1;
pub(crate) const KEY_MAX: usize = 0x2ff;
const KEY_BYTES: usize = KEY_MAX / 8 + 1;

pub(crate) const fn eviocgbit(ev: u8, len: usize) -> u32 {
    ioc(IOC_READ, b'E', 0x20 + ev, len)
}
pub(crate) const fn eviocgkey(len: usize) -> u32 {
    ioc(IOC_READ, b'E', 0x18, len)
}

// Linux key codes used by the port's keymaps (`DIGIUM_KEYS_PROPOSED`).
const KEY_UP: u16 = 103;
const KEY_LEFT: u16 = 105;
const KEY_RIGHT: u16 = 106;
const KEY_DOWN: u16 = 108;
const KEY_MENU: u16 = 139;
const KEY_VOLUMEDOWN: u16 = 114;
const KEY_VOLUMEUP: u16 = 115;
/// Hookswitch: down while the handset is on the cradle.
pub(crate) const KEY_PHONE: u16 = 169;
const KEY_CANCEL: u16 = 223;
const KEY_MICMUTE: u16 = 248;
const KEY_SELECT: u16 = 0x161;
const KEY_INFO: u16 = 0x166;
const KEY_VOICEMAIL: u16 = 0x1ac;
const KEY_ADDRESSBOOK: u16 = 0x1ad;
const KEY_APPSELECT: u16 = 0x244;
const KEY_NUMERIC_0: u16 = 0x200;
const KEY_NUMERIC_STAR: u16 = 0x20a;
const KEY_NUMERIC_POUND: u16 = 0x20b;
const KEY_MACRO1: u16 = 0x290;
const KEY_KBD_LCD_MENU1: u16 = 0x2b8;

/// Maps a Linux key code from the phone's keypad to a [`Key`].  Returns
/// `None` for the hookswitch code.  A code no keymap uses comes out as
/// [`Key::Unknown`].
///
/// The keys only the D62, D65, and D70 keymaps have (Menu, Status, Apps,
/// Contacts, the two BLF page keys, and line keys 5 and 6) are mapped from
/// the vendor phone application's key table, not from key presses; see
/// `digium-keys.h` in the kernel's devicetree directory.
pub fn key_from_code(code: u16) -> Option<Key> {
    Some(match code {
        KEY_PHONE => return None,
        c @ KEY_NUMERIC_0..=0x209 => Key::Digit((c - KEY_NUMERIC_0) as u8),
        KEY_NUMERIC_STAR => Key::Star,
        KEY_NUMERIC_POUND => Key::Pound,
        KEY_UP => Key::Up,
        KEY_DOWN => Key::Down,
        KEY_LEFT => Key::Left,
        KEY_RIGHT => Key::Right,
        KEY_SELECT => Key::Select,
        KEY_CANCEL => Key::Cancel,
        KEY_VOLUMEDOWN => Key::VolumeDown,
        KEY_VOLUMEUP => Key::VolumeUp,
        c @ KEY_KBD_LCD_MENU1..=0x2bb => Key::Soft((c - KEY_KBD_LCD_MENU1) as u8 + 1),
        KEY_MICMUTE => Key::Mute,
        KEY_VOICEMAIL => Key::Messages,
        KEY_INFO => Key::Info,
        KEY_MENU => Key::Menu,
        KEY_APPSELECT => Key::Apps,
        KEY_ADDRESSBOOK => Key::Contacts,
        c @ KEY_MACRO1..=0x2ad => match c - KEY_MACRO1 + 1 {
            1 => Key::Speaker,
            2 => Key::Headset,
            3 => Key::Transfer,
            4 => Key::Hold,
            5 => Key::Blf(7),
            6 => Key::Redial,
            8 => Key::Blf(9),
            9 => Key::Line(2),
            10 => Key::Line(1),
            11 => Key::Line(4),
            12 => Key::Blf(8),
            13 => Key::Blf(10),
            15 => Key::Conference,
            16 => Key::Line(3),
            n @ 17..=22 => Key::Blf((n - 16) as u8),
            24 => Key::Status,
            26 => Key::BlfPageLeft,
            27 => Key::BlfPageRight,
            28 => Key::Line(5),
            29 => Key::Line(6),
            _ => Key::Unknown(c),
        },
        c => Key::Unknown(c),
    })
}

struct Dev {
    file: File,
    has_hook: bool,
}

/// The keypad and hookswitch.
pub struct EvdevInput {
    devs: Vec<Dev>,
    pending: VecDeque<InputEvent>,
}

fn bit(bits: &[u8], n: usize) -> bool {
    bits.get(n / 8).is_some_and(|b| b & (1 << (n % 8)) != 0)
}

impl EvdevInput {
    pub fn open() -> Result<EvdevInput> {
        let mut devs = Vec::new();
        let mut paths: Vec<_> = std::fs::read_dir("/dev/input")
            .map_err(|e| HalError::io("/dev/input", e))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("event")))
            .collect();
        paths.sort();
        for p in paths {
            let Ok(file) = open_ro(&p, libc::O_NONBLOCK) else { continue };
            let mut types = [0u8; 4];
            if ioctl(file.as_raw_fd(), eviocgbit(0, types.len()), &mut types).is_err()
                || !bit(&types, EV_KEY as usize)
            {
                continue;
            }
            let mut keys = [0u8; KEY_BYTES];
            if ioctl(file.as_raw_fd(), eviocgbit(EV_KEY as u8, keys.len()), &mut keys).is_err() {
                continue;
            }
            // Only devices that carry phone keys: the keypad (numeric keys)
            // or the hookswitch.
            let has_hook = bit(&keys, KEY_PHONE as usize);
            if !has_hook && !bit(&keys, KEY_NUMERIC_0 as usize) {
                continue;
            }
            devs.push(Dev { file, has_hook });
        }
        if devs.is_empty() {
            return Err(HalError::Unavailable("no keypad or hookswitch input device".into()));
        }
        Ok(EvdevInput { devs, pending: VecDeque::new() })
    }

    fn drain(&mut self, idx: usize) -> Result<()> {
        let mut raw = [InputEventRaw::default(); 32];
        loop {
            // SAFETY: `InputEventRaw` is plain data; the byte view covers
            // exactly the array.
            let bytes = unsafe {
                std::slice::from_raw_parts_mut(raw.as_mut_ptr() as *mut u8, size_of_val(&raw))
            };
            let n = match self.devs[idx].file.read(bytes) {
                Ok(0) => return Ok(()),
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(HalError::io("read input event", e)),
            };
            for ev in &raw[..n / size_of::<InputEventRaw>()] {
                if ev.type_ != EV_KEY || ev.value > 1 {
                    continue; // not a key, or auto-repeat
                }
                self.pending.push_back(if ev.code == KEY_PHONE {
                    InputEvent::Hook { off_hook: ev.value == 0 }
                } else if let Some(key) = key_from_code(ev.code) {
                    InputEvent::Key { key, down: ev.value == 1 }
                } else {
                    continue;
                });
            }
        }
    }
}

impl Input for EvdevInput {
    fn next_event(&mut self, timeout: Option<Duration>) -> Result<Option<InputEvent>> {
        if let Some(e) = self.pending.pop_front() {
            return Ok(Some(e));
        }
        let fds: Vec<RawFd> = self.devs.iter().map(|d| d.file.as_raw_fd()).collect();
        let ms = match timeout {
            None => -1,
            Some(t) => t.as_millis().min(i32::MAX as u128) as i32,
        };
        let ready = poll_readable(&fds, ms).map_err(|e| HalError::io("poll input", e))?;
        for (i, r) in ready.into_iter().enumerate() {
            if r {
                self.drain(i)?;
            }
        }
        Ok(self.pending.pop_front())
    }

    fn off_hook(&mut self) -> Result<bool> {
        let Some(d) = self.devs.iter().find(|d| d.has_hook) else { return Ok(false) };
        let mut keys = [0u8; KEY_BYTES];
        ioctl(d.file.as_raw_fd(), eviocgkey(keys.len()), &mut keys)
            .map_err(|e| HalError::io("EVIOCGKEY", e))?;
        // Key down = handset on the cradle.
        Ok(!bit(&keys, KEY_PHONE as usize))
    }

    fn poll_fds(&self) -> Vec<RawFd> {
        self.devs.iter().map(|d| d.file.as_raw_fd()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_codes() {
        assert_eq!(key_from_code(0x200), Some(Key::Digit(0)));
        assert_eq!(key_from_code(0x209), Some(Key::Digit(9)));
        assert_eq!(key_from_code(0x20a), Some(Key::Star));
        assert_eq!(key_from_code(0x2b8), Some(Key::Soft(1)));
        assert_eq!(key_from_code(0x2bb), Some(Key::Soft(4)));
        assert_eq!(key_from_code(0x290), Some(Key::Speaker));
        assert_eq!(key_from_code(0x290 + 9), Some(Key::Line(1)));
        assert_eq!(key_from_code(0x290 + 8), Some(Key::Line(2)));
        assert_eq!(key_from_code(0x290 + 16), Some(Key::Blf(1)));
        assert_eq!(key_from_code(0x290 + 21), Some(Key::Blf(6)));
        assert_eq!(key_from_code(0x290 + 4), Some(Key::Blf(7)));
        assert_eq!(key_from_code(0x290 + 22), Some(Key::Unknown(0x290 + 22)));
        // The keys of the D62, D65, and D70 keymaps: DGK_MENU, DGK_APPS,
        // DGK_CONTACTS, and KEY_MACRO24, 26, 27, 28, and 29.
        assert_eq!(key_from_code(139), Some(Key::Menu));
        assert_eq!(key_from_code(0x244), Some(Key::Apps));
        assert_eq!(key_from_code(0x1ad), Some(Key::Contacts));
        assert_eq!(key_from_code(0x290 + 23), Some(Key::Status));
        assert_eq!(key_from_code(0x290 + 25), Some(Key::BlfPageLeft));
        assert_eq!(key_from_code(0x290 + 26), Some(Key::BlfPageRight));
        assert_eq!(key_from_code(0x290 + 27), Some(Key::Line(5)));
        assert_eq!(key_from_code(0x290 + 28), Some(Key::Line(6)));
        assert_eq!(key_from_code(0x290 + 24), Some(Key::Unknown(0x290 + 24)));
        assert_eq!(key_from_code(0x290 + 29), Some(Key::Unknown(0x290 + 29)));
        assert_eq!(key_from_code(0x1ac), Some(Key::Messages));
        assert_eq!(key_from_code(169), None);
    }
}
