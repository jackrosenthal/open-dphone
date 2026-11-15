//! App files for Open DPhone: the manifest an app carries in its
//! WebAssembly ([`manifest`]), and, with the feature `tool` (on by
//! default), what turns an app crate into the file the phone installs: the
//! command `cargo dphone-app`.
//!
//! The firmware reads manifests with this crate (without `tool`), so the
//! tool and the phone agree on what an app file is.

pub mod manifest;

pub use manifest::{AppType, Capability, Manifest, ManifestError};

#[cfg(feature = "tool")]
mod build;
#[cfg(feature = "tool")]
pub use build::{BuiltApp, build, componentize};
