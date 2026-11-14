//! Installing and removing downloaded apps.
//!
//! Layout: built-in apps live read-only under the built-in root
//! (`/usr/share/dphone/apps/<name>/`), installed apps under the installed
//! root (`/nvdata/wasmapps/<name>/`), which sits beside the stock
//! JavaScript directory (`/nvdata/jsapps`) and never touches it, so a
//! flash back to stock finds its own apps as it left them.
//!
//! Unlike stock, replacement is atomic: the new package is staged in a
//! sibling directory and swapped in with renames, and the old one is
//! removed only afterwards.

use crate::host::usable_space;
use crate::manifest::Manifest;
use crate::package::{Package, PackageError};
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum InstallError {
    Package(PackageError),
    /// The package's name differs from the one the caller required
    /// (stock status -107).
    NameMismatch {
        expected: String,
        found: String,
    },
    /// A built-in app has this name (stock status -103).
    ShadowsBuiltin(String),
    /// Not enough space after the 2 MiB reserve (stock status -110).
    NoSpace {
        needed: u64,
        available: u64,
    },
    /// The component does not load, or imports something the manifest
    /// does not declare.
    Unloadable(String),
    Io(std::io::Error),
}

impl InstallError {
    /// The stock installer's status code for this failure.
    pub fn stock_status(&self) -> i32 {
        match self {
            InstallError::Package(PackageError::NoManifest) => -101,
            InstallError::Package(PackageError::Manifest(
                crate::ManifestError::BadName(_) | crate::ManifestError::Unreadable(_),
            )) => -102,
            InstallError::ShadowsBuiltin(_) => -103,
            InstallError::NameMismatch { .. } => -107,
            InstallError::NoSpace { .. } => -110,
            _ => -104,
        }
    }
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstallError::Package(e) => write!(f, "{e}"),
            InstallError::NameMismatch { expected, found } => {
                write!(f, "the package is {found:?}, not {expected:?}")
            }
            InstallError::ShadowsBuiltin(n) => write!(f, "{n:?} is a built-in app"),
            InstallError::NoSpace { needed, available } => {
                write!(
                    f,
                    "not enough space: {needed} bytes needed, {available} available"
                )
            }
            InstallError::Unloadable(e) => write!(f, "the app cannot be loaded: {e}"),
            InstallError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for InstallError {}

impl From<std::io::Error> for InstallError {
    fn from(e: std::io::Error) -> Self {
        InstallError::Io(e)
    }
}

impl From<PackageError> for InstallError {
    fn from(e: PackageError) -> Self {
        InstallError::Package(e)
    }
}

fn dir_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

fn sync_dir(dir: &Path) {
    if let Ok(f) = std::fs::File::open(dir) {
        let _ = f.sync_all();
    }
}

#[derive(Clone, Debug)]
pub struct Installer {
    pub builtin_root: PathBuf,
    pub installed_root: PathBuf,
    /// `/nvdata/appdata` and `/tmp/appdata`, for removal.
    pub nv_root: PathBuf,
    pub tmp_root: PathBuf,
}

impl Installer {
    pub fn is_builtin(&self, name: &str) -> bool {
        self.builtin_root.join(name).join("app.json").is_file()
    }

    /// Checks everything that can be checked before touching the flash.
    /// `extra` is space the caller will also need (e.g., compiled code).
    pub fn check(
        &self,
        package: &Package,
        required_name: Option<&str>,
        extra: u64,
    ) -> Result<(), InstallError> {
        let name = &package.manifest.name;
        if let Some(expected) = required_name
            && !expected.is_empty()
            && expected != name
        {
            return Err(InstallError::NameMismatch {
                expected: expected.to_string(),
                found: name.clone(),
            });
        }
        if self.is_builtin(name) {
            return Err(InstallError::ShadowsBuiltin(name.clone()));
        }
        if let Some(c) = package
            .manifest
            .capabilities
            .iter()
            .find(|c| c.builtin_only())
        {
            return Err(InstallError::Unloadable(format!(
                "the capability {:?} is reserved for built-in apps",
                c.name()
            )));
        }
        std::fs::create_dir_all(&self.installed_root)?;
        // An upgrade is charged its growth, as stock does.
        let existing = dir_size(&self.installed_root.join(name));
        let needed = (package.unpacked_size() + extra).saturating_sub(existing);
        let available = usable_space(&self.installed_root);
        if needed > available {
            return Err(InstallError::NoSpace { needed, available });
        }
        Ok(())
    }

    /// Writes the package (and `extra` files such as compiled code) to a
    /// staging directory and swaps it in.  Returns the app directory.
    pub fn commit(
        &self,
        package: &Package,
        extra: &[(String, Vec<u8>)],
    ) -> Result<PathBuf, InstallError> {
        use std::io::Write;
        let name = &package.manifest.name;
        let final_dir = self.installed_root.join(name);
        let stage = self.installed_root.join(format!(".{name}.new"));
        let old = self.installed_root.join(format!(".{name}.old"));
        let _ = std::fs::remove_dir_all(&stage);
        let _ = std::fs::remove_dir_all(&old);
        let write_all = || -> std::io::Result<()> {
            std::fs::create_dir_all(&stage)?;
            for (n, data) in package.files.iter().chain(extra) {
                let mut f = std::fs::File::create(stage.join(n))?;
                f.write_all(data)?;
                f.sync_all()?;
            }
            sync_dir(&stage);
            if final_dir.exists() {
                std::fs::rename(&final_dir, &old)?;
            }
            if let Err(e) = std::fs::rename(&stage, &final_dir) {
                // Put the previous version back.
                let _ = std::fs::rename(&old, &final_dir);
                return Err(e);
            }
            sync_dir(&self.installed_root);
            Ok(())
        };
        let result = write_all();
        let _ = std::fs::remove_dir_all(&stage);
        let _ = std::fs::remove_dir_all(&old);
        result?;
        Ok(final_dir)
    }

    /// Finishes or undoes an install interrupted by power loss.
    pub fn recover(&self) {
        let Ok(rd) = std::fs::read_dir(&self.installed_root) else {
            return;
        };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if let Some(name) = n.strip_prefix('.').and_then(|s| s.strip_suffix(".old")) {
                let final_dir = self.installed_root.join(name);
                if final_dir.exists() {
                    let _ = std::fs::remove_dir_all(e.path());
                } else {
                    let _ = std::fs::rename(e.path(), final_dir);
                }
            } else if n.starts_with('.') && n.ends_with(".new") {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }

    /// Removes an installed app with its persistent and temporary data.
    pub fn remove(&self, name: &str) -> Result<(), InstallError> {
        crate::manifest::validate_name(name)
            .map_err(|e| InstallError::Package(PackageError::Manifest(e)))?;
        let dir = self.installed_root.join(name);
        if !dir.is_dir() {
            return Err(InstallError::Io(std::io::ErrorKind::NotFound.into()));
        }
        std::fs::remove_dir_all(&dir)?;
        let _ = std::fs::remove_dir_all(self.nv_root.join(name));
        let _ = std::fs::remove_dir_all(self.tmp_root.join(name));
        sync_dir(&self.installed_root);
        Ok(())
    }

    /// Manifests of the apps under a root: (directory, manifest).  Stock
    /// JavaScript apps and broken manifests are skipped with a log entry.
    pub fn scan_root(root: &Path) -> Vec<(PathBuf, Manifest)> {
        let mut found = Vec::new();
        let Ok(rd) = std::fs::read_dir(root) else {
            return found;
        };
        for e in rd.flatten() {
            let dir = e.path();
            let dir_name = e.file_name().to_string_lossy().into_owned();
            if dir_name.starts_with('.') || !dir.is_dir() {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(dir.join("app.json")) else {
                continue;
            };
            match Manifest::parse(&text) {
                Ok(m) if m.name == dir_name => found.push((dir, m)),
                Ok(m) => log::warn!("app directory {dir_name:?} holds app {:?}; skipped", m.name),
                Err(crate::ManifestError::JavaScript) => {
                    log::info!(
                        "skipping JavaScript app {dir_name:?}: it cannot run on this firmware"
                    )
                }
                Err(err) => log::warn!("skipping app {dir_name:?}: {err}"),
            }
        }
        found.sort_by(|a, b| a.1.name.cmp(&b.1.name));
        found
    }
}
