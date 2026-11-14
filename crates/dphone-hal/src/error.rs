use std::fmt;
use std::io;

/// HAL errors.
#[derive(Debug)]
pub enum HalError {
    /// An operating-system error, with what was being done.
    Io { context: String, source: io::Error },
    /// The device is not present or could not be opened.
    Unavailable(String),
    /// This model or backend has no such function.
    Unsupported(&'static str),
    /// Bad argument (out of range, misaligned, unknown name).
    Invalid(String),
    /// The flash block at this offset is marked bad.
    BadBlock(u64),
    /// Audio overrun/underrun; call `recover` and continue.
    Xrun,
    /// Nothing arrived within the timeout.
    Timeout,
}

pub type Result<T> = std::result::Result<T, HalError>;

impl HalError {
    pub fn io(context: impl Into<String>, source: io::Error) -> Self {
        HalError::Io { context: context.into(), source }
    }
    /// The last OS error, with context.
    pub fn last_os(context: impl Into<String>) -> Self {
        HalError::Io { context: context.into(), source: io::Error::last_os_error() }
    }
}

impl fmt::Display for HalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HalError::Io { context, source } => write!(f, "{context}: {source}"),
            HalError::Unavailable(s) => write!(f, "device unavailable: {s}"),
            HalError::Unsupported(s) => write!(f, "unsupported: {s}"),
            HalError::Invalid(s) => write!(f, "invalid: {s}"),
            HalError::BadBlock(o) => write!(f, "bad flash block at {o:#x}"),
            HalError::Xrun => f.write_str("audio xrun"),
            HalError::Timeout => f.write_str("timed out"),
        }
    }
}

impl std::error::Error for HalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HalError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<io::Error> for HalError {
    fn from(e: io::Error) -> Self {
        HalError::Io { context: "I/O".into(), source: e }
    }
}
