//! JFFS2 image writer and reader.
//!
//! The writer produces what `mkfs.jffs2 -l -n -e 0x20000` does and what the
//! stock Digium root images are: little-endian nodes, no in-band
//! cleanmarkers (NAND keeps them in the OOB area, and the updater writes
//! only data), nodes never crossing an erase-block boundary, the tail of a
//! block filled with `0xff`, and no padding after the last node.
//!
//! The reader scans an image (or a raw partition dump), keeps the newest
//! version of every directory entry and inode, and rebuilds the tree.  It is
//! used for tests and for listing or extracting stock images.

pub mod compr;
mod crc;
pub mod node;
mod reader;
mod tree;
mod writer;

pub use reader::{ScanStats, read_image};
pub use tree::{Entry, Kind, Tree};
pub use writer::{Compression, Options, write_image};

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
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

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn bad<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Format(msg.into()))
}
