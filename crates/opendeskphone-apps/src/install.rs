//! Installing and removing downloaded apps.
//!
//! Layout: built-in apps live read-only under the built-in root
//! (`/usr/share/opendeskphone/apps/<name>/`), installed apps under the
//! installed root (`/nvdata/opendeskphone/wasmapps/<name>/`), which sits
//! beside the stock JavaScript directory (`/nvdata/jsapps`) and never touches
//! it, so a flash back to stock finds its own apps as it left them.
//!
//! An app arrives as one file, a WebAssembly component with its manifest
//! in it ([`AppFile`]); its data is in its own code.  In its directory on
//! the phone it is the phone's record of it ([`RECORD`]: the manifest,
//! with the icons and with the interfaces the component imports), the
//! component (`<name>.wasm`, kept to compile again after a firmware
//! update), and the code this phone compiled of it (`.code.cwasm`).  A
//! built-in app has the record and `<name>.cwasm`, compiled at build time,
//! and no component.  Nothing is unpacked.
//!
//! Unlike stock, replacement is atomic: the new app is staged in a
//! sibling directory and swapped in with renames, and the old one is
//! removed only afterwards.

use crate::host::usable_space;
use crate::manifest::{self, Manifest, ManifestError, RECORD};
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum InstallError {
    Manifest(ManifestError),
    /// The file is larger than any app may be.
    TooLarge {
        size: u64,
        max: u64,
    },
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
    /// The component holds more code than the phone compiles.
    ComponentTooLarge {
        size: u64,
        max: u64,
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
            InstallError::Manifest(
                ManifestError::NoManifest | ManifestError::NotWasm | ManifestError::JavaScript,
            ) => -101,
            InstallError::Manifest(ManifestError::BadName(_) | ManifestError::Malformed(_)) => -102,
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
            InstallError::Manifest(e) => write!(f, "{e}"),
            InstallError::TooLarge { size, max } => {
                write!(
                    f,
                    "the file is too large for an app: {size} bytes, and the phone takes {max}"
                )
            }
            InstallError::NameMismatch { expected, found } => {
                write!(f, "the app is {found:?}, not {expected:?}")
            }
            InstallError::ShadowsBuiltin(n) => write!(f, "{n:?} is a built-in app"),
            InstallError::NoSpace { needed, available } => {
                write!(
                    f,
                    "not enough space: {needed} bytes needed, {available} available"
                )
            }
            InstallError::ComponentTooLarge { size, max } => write!(
                f,
                "the app has too much code: {size} bytes, and the phone compiles {max}"
            ),
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

impl From<ManifestError> for InstallError {
    fn from(e: ManifestError) -> Self {
        InstallError::Manifest(e)
    }
}

/// The largest app file read at all.  The compiler works through the code
/// alone, which has its own limit ([`crate::MAX_COMPONENT`]); the rest of a
/// file is the app's data, which goes into the app's memory when it starts
/// (16 MiB at most).
pub const MAX_APP_FILE: u64 = 8 << 20;

/// An app file, read into memory and checked as far as its manifest goes.
#[derive(Clone, Debug)]
pub struct AppFile {
    /// The manifest.  Its capabilities are empty until the component has
    /// been compiled (they are the component's imports).
    pub manifest: Manifest,
    /// The component, as it came.
    pub component: Vec<u8>,
}

impl AppFile {
    pub fn open(path: &Path) -> Result<AppFile, InstallError> {
        let size = std::fs::metadata(path)?.len();
        if size > MAX_APP_FILE {
            return Err(InstallError::TooLarge {
                size,
                max: MAX_APP_FILE,
            });
        }
        AppFile::read(std::fs::read(path)?)
    }

    pub fn read(component: Vec<u8>) -> Result<AppFile, InstallError> {
        let manifest = Manifest::from_wasm(&component)?;
        Ok(AppFile {
            manifest,
            component,
        })
    }

    /// The bytes of code in the component: what the compiler has to work
    /// through.
    pub fn code_size(&self) -> u64 {
        manifest::code_size(&self.component)
    }

    /// What the app takes on the flash, without its compiled code.
    pub fn unpacked_size(&self) -> u64 {
        (self.component.len() + self.manifest.record().len()) as u64
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
    /// `/nvdata/appdata`, for removal.
    pub nv_root: PathBuf,
}

impl Installer {
    pub fn is_builtin(&self, name: &str) -> bool {
        self.builtin_root.join(name).join(RECORD).is_file()
    }

    /// Checks everything that can be checked before touching the flash.
    /// `extra` is space the caller will also need (e.g., compiled code).
    pub fn check(
        &self,
        package: &AppFile,
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
        // Capabilities are the component's imports, known once it is
        // compiled; an app importing an interface that is for built-in
        // apps is refused then (`prepare`).
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
        package: &AppFile,
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
            let own = [
                (RECORD.to_string(), package.manifest.record()),
                (package.manifest.wasm.clone(), package.component.clone()),
            ];
            for (n, data) in own.iter().chain(extra) {
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

    /// Removes an installed app with its store.
    pub fn remove(&self, name: &str) -> Result<(), InstallError> {
        crate::manifest::validate_name(name).map_err(InstallError::Manifest)?;
        let dir = self.installed_root.join(name);
        if !dir.is_dir() {
            return Err(InstallError::Io(std::io::ErrorKind::NotFound.into()));
        }
        std::fs::remove_dir_all(&dir)?;
        let _ = std::fs::remove_dir_all(self.nv_root.join(name));
        sync_dir(&self.installed_root);
        Ok(())
    }

    /// The apps under a root, from the phone's records of them: (directory,
    /// manifest).  Stock JavaScript apps and broken records are skipped
    /// with a log entry.
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
            let Ok(record) = std::fs::read(dir.join(RECORD)) else {
                if dir.join("app.json").is_file() {
                    log::info!(
                        "skipping JavaScript app {dir_name:?}: it cannot run on this firmware"
                    );
                }
                continue;
            };
            match Manifest::from_record(&record) {
                Ok(m) if m.name == dir_name => found.push((dir, m)),
                Ok(m) => log::warn!("app directory {dir_name:?} holds app {:?}; skipped", m.name),
                Err(err) => log::warn!("skipping app {dir_name:?}: {err}"),
            }
        }
        found.sort_by(|a, b| a.1.name.cmp(&b.1.name));
        found
    }
}
