//! Firmware version in the stock forms.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// A firmware version.  Ours are `3.x`; stock releases are `2.9.x`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FirmwareVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

const fn parse_u16(b: &[u8], mut i: usize) -> (u16, usize) {
    let mut v = 0u16;
    while i < b.len() && b[i].is_ascii_digit() {
        v = v * 10 + (b[i] - b'0') as u16;
        i += 1;
    }
    (v, i)
}

const fn parse_const(s: &str) -> FirmwareVersion {
    let b = s.as_bytes();
    let (major, i) = parse_u16(b, 0);
    let (minor, i) = parse_u16(b, i + 1);
    let (patch, _) = parse_u16(b, i + 1);
    FirmwareVersion { major, minor, patch }
}

impl FirmwareVersion {
    /// The version of this build (the workspace package version).
    pub const CURRENT: FirmwareVersion = parse_const(env!("CARGO_PKG_VERSION"));

    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self { major, minor, patch }
    }

    /// `3_0_0`: the form used in the `User-Agent`, the EFF label and inner
    /// header, the OTA filename field, and the DHCP vendor class.
    pub fn underscored(&self) -> String {
        format!("{}_{}_{}", self.major, self.minor, self.patch)
    }

    /// `3.0.0`: for display.
    pub fn dotted(&self) -> String {
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }

    /// True for versions produced by this project (major 3 and up).
    pub const fn is_open(&self) -> bool {
        self.major >= 3
    }
}

impl fmt::Display for FirmwareVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}_{}_{}", self.major, self.minor, self.patch)
    }
}

/// Error from parsing a version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BadVersion(pub String);
impl fmt::Display for BadVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bad firmware version {:?}", self.0)
    }
}
impl std::error::Error for BadVersion {}

impl FromStr for FirmwareVersion {
    type Err = BadVersion;
    /// Accepts `3_0_0`, `3.0.0`, and a two-part `3_0`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || BadVersion(s.to_string());
        let mut it = s.trim().split(['_', '.']);
        let major = it.next().ok_or_else(bad)?.parse().map_err(|_| bad())?;
        let minor = it.next().ok_or_else(bad)?.parse().map_err(|_| bad())?;
        let patch = match it.next() {
            Some(p) => p.parse().map_err(|_| bad())?,
            None => 0,
        };
        if it.next().is_some() {
            return Err(bad());
        }
        Ok(Self { major, minor, patch })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms() {
        let v = FirmwareVersion::new(3, 0, 0);
        assert_eq!(v.underscored(), "3_0_0");
        assert_eq!(v.dotted(), "3.0.0");
        assert_eq!("2_9_28".parse::<FirmwareVersion>().unwrap(), FirmwareVersion::new(2, 9, 28));
        assert!(v > "2_9_28".parse().unwrap());
        assert_eq!(FirmwareVersion::CURRENT.major, 3);
        assert!("x".parse::<FirmwareVersion>().is_err());
        assert_eq!(
            crate::Model::D60.user_agent(&v),
            "Digium D60 3_0_0"
        );
    }
}
