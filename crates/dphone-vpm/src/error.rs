// SPDX-License-Identifier: GPL-2.0-or-later
use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The DSP is not started.
    NotStarted,
    /// The DSP reported an assertion or the transport lost synchronization;
    /// restart the DSP.
    Failed,
    /// No result arrived in time.
    Timeout,
    /// An earlier job timed out and its result is still outstanding.
    Busy,
    /// The result does not belong to the request.
    Protocol,
    /// Bad channel, client, or argument.
    Invalid,
    /// The body is too long.
    TooLong,
    /// The result is shorter than the job's fixed frame.
    ShortReply { want: usize, got: usize, client: u16, channel: u16, opcode: u16 },
    /// The driver speaks another ABI version.
    AbiVersion(u32),
    /// Not available on this family (e.g., the BAND client on the old one).
    NotOnThisFamily,
    /// A codec the DSP may have, but whose init body was never captured.
    Unsupported(&'static str),
    /// Any other errno from the driver or the file system.
    Os(i32),
}

impl Error {
    pub fn from_errno(e: i32) -> Self {
        match e {
            libc::ENODEV => Error::NotStarted,
            libc::EIO => Error::Failed,
            libc::ETIMEDOUT => Error::Timeout,
            libc::EBUSY => Error::Busy,
            libc::EPROTO => Error::Protocol,
            libc::EINVAL => Error::Invalid,
            libc::EMSGSIZE => Error::TooLong,
            e => Error::Os(e),
        }
    }

    pub fn last_os() -> Self {
        Self::from_errno(std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO))
    }

    /// After these, nothing further can succeed until the DSP is restarted.
    pub fn is_fatal(&self) -> bool {
        matches!(self, Error::NotStarted | Error::Failed)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::from_errno(e.raw_os_error().unwrap_or(libc::EIO))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotStarted => f.write_str("the DSP is not started"),
            Error::Failed => f.write_str("the DSP failed (assertion or transport error)"),
            Error::Timeout => f.write_str("no result from the DSP in time"),
            Error::Busy => f.write_str("an earlier DSP job is still outstanding"),
            Error::Protocol => f.write_str("the DSP result does not belong to the request"),
            Error::Invalid => f.write_str("invalid channel, client, or argument"),
            Error::TooLong => f.write_str("job body too long"),
            Error::ShortReply { want, got, client, channel, opcode } => {
                write!(f, "short DSP result: {got} bytes, expected {want} (client {client}, channel {channel}, operation {opcode})")
            }
            Error::AbiVersion(v) => write!(f, "driver ABI version {v} is not supported"),
            Error::NotOnThisFamily => f.write_str("not available on this DSP family"),
            Error::Unsupported(what) => write!(f, "unsupported: {what}"),
            Error::Os(e) => write!(f, "{}", std::io::Error::from_raw_os_error(*e)),
        }
    }
}

impl std::error::Error for Error {}
