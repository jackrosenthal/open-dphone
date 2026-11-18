use std::fmt;
use std::io;

use opendeskphone_hal::HalError;

/// Why an update, a bank switch, or an inspection failed.
#[derive(Debug)]
pub enum UpdateError {
    /// Flash access failed.
    Hal(HalError),
    /// Reading the image stream or writing nvdata failed.
    Io(io::Error),
    /// The image is not a well-formed EFF/OTA, or a CRC does not match.
    Image(String),
    /// The image is for another model.
    WrongModel { image: String, phone: String },
    /// The flash layout is not what an update needs (missing partition,
    /// image larger than the usable part of its partition).
    Layout(String),
    /// The `ctrl` partition holds no usable record, or a record could not
    /// be written so that boot2 would read it back.
    Ctrl(String),
    /// Data read back from flash differs from what was written.
    Verify(String),
    /// The operation would leave the phone without a bootable bank, or
    /// makes no sense in the current state; nothing was changed.
    Refused(String),
}

pub type Result<T> = std::result::Result<T, UpdateError>;

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpdateError::Hal(e) => write!(f, "flash: {e}"),
            UpdateError::Io(e) => write!(f, "{e}"),
            UpdateError::Image(m) => write!(f, "bad firmware image: {m}"),
            UpdateError::WrongModel { image, phone } => {
                write!(f, "firmware image is for the {image}, this phone is a {phone}")
            }
            UpdateError::Layout(m) => write!(f, "flash layout: {m}"),
            UpdateError::Ctrl(m) => write!(f, "ctrl record: {m}"),
            UpdateError::Verify(m) => write!(f, "verification failed: {m}"),
            UpdateError::Refused(m) => write!(f, "refused: {m}"),
        }
    }
}

impl std::error::Error for UpdateError {}

impl From<HalError> for UpdateError {
    fn from(e: HalError) -> Self {
        UpdateError::Hal(e)
    }
}

impl From<io::Error> for UpdateError {
    fn from(e: io::Error) -> Self {
        // The eff readers report format errors through `io::Error` when
        // used as `Read`; keep those distinguishable.
        if e.kind() == io::ErrorKind::InvalidData || e.kind() == io::ErrorKind::UnexpectedEof {
            UpdateError::Image(e.to_string())
        } else {
            UpdateError::Io(e)
        }
    }
}

impl From<eff::Error> for UpdateError {
    fn from(e: eff::Error) -> Self {
        match e {
            eff::Error::Io(e) => e.into(),
            eff::Error::Format(m) => UpdateError::Image(m),
        }
    }
}

impl From<UpdateError> for io::Error {
    fn from(e: UpdateError) -> Self {
        match e {
            UpdateError::Io(e) => e,
            other => io::Error::other(other.to_string()),
        }
    }
}
