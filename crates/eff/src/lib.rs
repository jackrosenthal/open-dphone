//! Digium D-series firmware container formats.
//!
//! * [`envelope`]: the `.eff` envelope (100-byte label, OpenSSL `Salted__`
//!   AES-256-CBC, four 100-byte inner fields, then the OTA).
//! * [`ota`]: the type-0 (application) OTA container: kernel plus rootfs with
//!   three CRCs.
//! * [`ctrl`]: the 304-byte bank-selection record in the `ctrl` partition.
//!
//! Everything here is usable on the phone (streaming, no allocation
//! proportional to the image) and on the build host.

pub mod crc;
pub mod ctrl;
pub mod envelope;
pub mod ota;

pub use envelope::{EffHeader, EffReader, EffWriter, OTA_PASSPHRASE};
pub use ota::{AppHeader, OTA_MAGIC, OtaHeader};

/// Errors from parsing or building any of the formats.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// The data is not what the format requires; the message says what.
    Format(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Format(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<Error> for std::io::Error {
    fn from(e: Error) -> Self {
        match e {
            Error::Io(e) => e,
            Error::Format(m) => std::io::Error::new(std::io::ErrorKind::InvalidData, m),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn bad<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Format(msg.into()))
}
