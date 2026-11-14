//! Paired devices: address, link key, and name, in one text file under the
//! firmware's own nvdata directory (`/nvdata/open/bluetooth/devices`), which
//! the stock firmware ignores and its factory reset removes.
//!
//! One device per line: `ADDRESS KEY_TYPE KEY_HEX PIN_LENGTH NAME`.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::Addr;
use crate::mgmt::LinkKey;

pub const DEFAULT_DIR: &str = "/nvdata/open/bluetooth";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub key: LinkKey,
    pub name: String,
}

pub struct Store {
    path: PathBuf,
    devices: Vec<Device>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex16(text: &str) -> Option<[u8; 16]> {
    if text.len() != 32 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

fn parse_line(line: &str) -> Option<Device> {
    let mut parts = line.splitn(5, ' ');
    let addr: Addr = parts.next()?.parse().ok()?;
    let key_type = parts.next()?.parse().ok()?;
    let value = unhex16(parts.next()?)?;
    let pin_len = parts.next()?.parse().ok()?;
    let name = parts.next().unwrap_or("").to_string();
    Some(Device { key: LinkKey { addr, key_type, value, pin_len }, name })
}

fn format_line(d: &Device) -> String {
    // A name is one line.
    let name: String = d.name.chars().filter(|c| !c.is_control()).collect();
    format!("{} {} {} {} {}\n", d.key.addr, d.key.key_type, hex(&d.key.value), d.key.pin_len, name)
}

impl Store {
    /// Open the store in `dir`.  A missing file is an empty store; lines
    /// that do not parse are dropped.
    pub fn open(dir: &Path) -> Store {
        let path = dir.join("devices");
        let devices = fs::read_to_string(&path).map(|t| t.lines().filter_map(parse_line).collect()).unwrap_or_default();
        Store { path, devices }
    }

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    pub fn keys(&self) -> Vec<LinkKey> {
        self.devices.iter().map(|d| d.key.clone()).collect()
    }

    pub fn get(&self, addr: Addr) -> Option<&Device> {
        self.devices.iter().find(|d| d.key.addr == addr)
    }

    /// Add or replace a device's key.  The name is kept when `name` is
    /// `None` and the device is known.
    pub fn put(&mut self, key: LinkKey, name: Option<String>) -> io::Result<()> {
        match self.devices.iter_mut().find(|d| d.key.addr == key.addr) {
            Some(d) => {
                d.key = key;
                if let Some(name) = name {
                    d.name = name;
                }
            }
            None => self.devices.push(Device { key, name: name.unwrap_or_default() }),
        }
        self.save()
    }

    pub fn set_name(&mut self, addr: Addr, name: &str) -> io::Result<()> {
        match self.devices.iter_mut().find(|d| d.key.addr == addr) {
            Some(d) if d.name != name => {
                d.name = name.to_string();
                self.save()
            }
            _ => Ok(()),
        }
    }

    pub fn remove(&mut self, addr: Addr) -> io::Result<bool> {
        let before = self.devices.len();
        self.devices.retain(|d| d.key.addr != addr);
        if self.devices.len() == before {
            return Ok(false);
        }
        self.save()?;
        Ok(true)
    }

    /// Temporary file, `fsync`, rename.
    fn save(&self) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("new");
        let mut f = File::create(&tmp)?;
        for d in &self.devices {
            f.write_all(format_line(d).as_bytes())?;
        }
        f.sync_all()?;
        fs::rename(&tmp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(last: u8) -> LinkKey {
        LinkKey { addr: Addr([last, 2, 3, 4, 5, 6]), key_type: 4, value: [last; 16], pin_len: 0 }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dphone-bt-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn devices_persist() {
        let dir = temp_dir("persist");
        let mut s = Store::open(&dir);
        assert!(s.devices().is_empty());
        s.put(key(1), Some("Jabra Evolve 65".into())).unwrap();
        s.put(key(2), None).unwrap();
        // A new key for a known device keeps its name.
        let mut newer = key(1);
        newer.value = [9; 16];
        s.put(newer.clone(), None).unwrap();
        s.set_name(key(2).addr, "Plantronics\nVoyager").unwrap();

        let s = Store::open(&dir);
        assert_eq!(s.devices().len(), 2);
        assert_eq!(s.get(key(1).addr).unwrap(), &Device { key: newer, name: "Jabra Evolve 65".into() });
        assert_eq!(s.get(key(2).addr).unwrap().name, "PlantronicsVoyager");
        assert_eq!(s.keys().len(), 2);

        let mut s = s;
        assert!(s.remove(key(1).addr).unwrap());
        assert!(!s.remove(key(1).addr).unwrap());
        assert_eq!(Store::open(&dir).devices().len(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn bad_lines_are_dropped() {
        let dir = temp_dir("bad");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("devices"),
            "garbage\n06:05:04:03:02:01 4 0101 0 short key\n06:05:04:03:02:07 4 07070707070707070707070707070707 0\n",
        )
        .unwrap();
        let s = Store::open(&dir);
        assert_eq!(s.devices().len(), 1);
        assert_eq!(s.devices()[0].key, key(7));
        assert_eq!(s.devices()[0].name, "");
        fs::remove_dir_all(&dir).unwrap();
    }
}
