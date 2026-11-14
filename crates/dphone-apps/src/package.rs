//! App packages: a ZIP archive holding `app.json` and one `.wasm`
//! component (plus optional data files), at the top level or inside one
//! directory, as stock packages are laid out.

use crate::manifest::{Manifest, ManifestError};
use std::fmt;
use std::io::{Read, Seek, Write};
use std::path::Path;

/// Largest unpacked package accepted.
pub const MAX_UNPACKED: u64 = 8 << 20;
/// Most files accepted in a package.
pub const MAX_FILES: usize = 64;

#[derive(Debug)]
pub enum PackageError {
    Io(std::io::Error),
    Zip(String),
    /// No `app.json` in the archive (stock status -101).
    NoManifest,
    Manifest(ManifestError),
    /// The manifest's wasm file is not in the archive.
    NoWasm,
    /// An entry is absolute, escapes the package, is a link, or has a
    /// path separator where none is allowed.
    BadEntry(String),
    TooLarge,
}

impl fmt::Display for PackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackageError::Io(e) => write!(f, "{e}"),
            PackageError::Zip(e) => write!(f, "bad archive: {e}"),
            PackageError::NoManifest => write!(f, "the package has no app.json"),
            PackageError::Manifest(e) => write!(f, "{e}"),
            PackageError::NoWasm => write!(f, "the package lacks the wasm file its manifest names"),
            PackageError::BadEntry(n) => write!(f, "bad archive entry {n:?}"),
            PackageError::TooLarge => write!(f, "the package is too large"),
        }
    }
}

impl std::error::Error for PackageError {}

impl From<std::io::Error> for PackageError {
    fn from(e: std::io::Error) -> Self {
        PackageError::Io(e)
    }
}

/// An unpacked package held in memory.
#[derive(Clone, Debug)]
pub struct Package {
    pub manifest: Manifest,
    pub manifest_text: String,
    /// (file name, contents); flat, no directories.
    pub files: Vec<(String, Vec<u8>)>,
}

impl Package {
    /// Reads and checks a package archive.
    pub fn read(reader: impl Read + Seek) -> Result<Package, PackageError> {
        let mut zip = zip::ZipArchive::new(reader).map_err(|e| PackageError::Zip(e.to_string()))?;
        if zip.len() > MAX_FILES {
            return Err(PackageError::TooLarge);
        }
        // Find the manifest and the directory it sits in.
        let mut prefix: Option<String> = None;
        for i in 0..zip.len() {
            let f = zip
                .by_index(i)
                .map_err(|e| PackageError::Zip(e.to_string()))?;
            let name = f
                .name()
                .map_err(|e| PackageError::Zip(e.to_string()))?
                .to_string();
            if name.starts_with('/') || name.contains('\\') || name.split('/').any(|p| p == "..") {
                return Err(PackageError::BadEntry(name));
            }
            if prefix.is_none() && (name == "app.json" || name.ends_with("/app.json")) {
                prefix = Some(name[..name.len() - "app.json".len()].to_string());
            }
        }
        let prefix = prefix.ok_or(PackageError::NoManifest)?;
        let mut files = Vec::new();
        let mut total = 0u64;
        for i in 0..zip.len() {
            let mut f = zip
                .by_index(i)
                .map_err(|e| PackageError::Zip(e.to_string()))?;
            let name = f
                .name()
                .map_err(|e| PackageError::Zip(e.to_string()))?
                .to_string();
            if f.is_dir() || !name.starts_with(&prefix) {
                continue;
            }
            let leaf = &name[prefix.len()..];
            if f.is_symlink() || leaf.is_empty() || leaf.contains('/') || leaf.starts_with('.') {
                return Err(PackageError::BadEntry(name));
            }
            // Compiled code is never accepted from a package.
            if leaf.ends_with(".cwasm") {
                return Err(PackageError::BadEntry(name));
            }
            total += f.size();
            if total > MAX_UNPACKED {
                return Err(PackageError::TooLarge);
            }
            let mut data = Vec::new();
            (&mut f).take(MAX_UNPACKED + 1).read_to_end(&mut data)?;
            if data.len() as u64 > MAX_UNPACKED {
                return Err(PackageError::TooLarge);
            }
            files.push((leaf.to_string(), data));
        }
        let manifest_text = files
            .iter()
            .find(|(n, _)| n == "app.json")
            .map(|(_, d)| String::from_utf8_lossy(d).into_owned())
            .ok_or(PackageError::NoManifest)?;
        let manifest = Manifest::parse(&manifest_text).map_err(PackageError::Manifest)?;
        if !files.iter().any(|(n, _)| *n == manifest.wasm) {
            return Err(PackageError::NoWasm);
        }
        Ok(Package {
            manifest,
            manifest_text,
            files,
        })
    }

    pub fn open(path: &Path) -> Result<Package, PackageError> {
        Package::read(std::fs::File::open(path)?)
    }

    pub fn wasm(&self) -> &[u8] {
        self.files
            .iter()
            .find(|(n, _)| *n == self.manifest.wasm)
            .map(|(_, d)| &d[..])
            .unwrap_or(&[])
    }

    pub fn unpacked_size(&self) -> u64 {
        self.files.iter().map(|(_, d)| d.len() as u64).sum()
    }
}

/// Writes a package archive from a directory holding `app.json`, the wasm
/// file, and data files (flat).  Timestamps are fixed so the output is
/// reproducible.
pub fn pack(dir: &Path, out: impl Write + Seek) -> Result<(), PackageError> {
    let manifest_text =
        std::fs::read_to_string(dir.join("app.json")).map_err(|_| PackageError::NoManifest)?;
    let manifest = Manifest::parse(&manifest_text).map_err(PackageError::Manifest)?;
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.') && !n.ends_with(".cwasm"))
        .collect();
    names.sort();
    if !names.contains(&manifest.wasm) {
        return Err(PackageError::NoWasm);
    }
    let mut zip = zip::ZipWriter::new(out);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    for n in names {
        zip.start_file(format!("{}/{n}", manifest.name), opts)
            .map_err(|e| PackageError::Zip(e.to_string()))?;
        zip.write_all(&std::fs::read(dir.join(&n))?)?;
    }
    zip.finish().map_err(|e| PackageError::Zip(e.to_string()))?;
    Ok(())
}
