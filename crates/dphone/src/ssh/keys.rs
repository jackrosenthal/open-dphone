//! Host key and authorized keys.

use std::io;
use std::path::Path;

use log::{info, warn};
use russh::keys::ssh_key::LineEnding;
use russh::keys::{Algorithm, PrivateKey, PublicKey};

use crate::paths::Paths;

/// Load the Ed25519 host key, creating it on first boot.  It lives under
/// `/nvdata/open/`; stock's Dropbear key is not touched.
pub fn load_or_create_host_key(path: &Path) -> io::Result<PrivateKey> {
    match std::fs::read_to_string(path) {
        Ok(pem) => match PrivateKey::from_openssh(pem.as_bytes()) {
            Ok(key) => return Ok(key),
            Err(e) => warn!("ssh: host key {} unreadable ({e}); making a new one", path.display()),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => warn!("ssh: host key {}: {e}; making a new one", path.display()),
    }
    let key = PrivateKey::random(&mut rand_core::OsRng, Algorithm::Ed25519).map_err(io::Error::other)?;
    let pem = key.to_openssh(LineEnding::LF).map_err(io::Error::other)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    match crate::paths::write_atomic(path, pem.as_bytes(), 0o600) {
        Ok(()) => info!("ssh: generated host key {}", path.display()),
        // Still usable for this boot.
        Err(e) => warn!("ssh: could not save host key {}: {e}", path.display()),
    }
    Ok(key)
}

/// Parse `authorized_keys` text.  Lines with leading options are accepted
/// (the options are ignored); unparsable lines are skipped.
pub fn parse_authorized_keys(text: &str) -> Vec<PublicKey> {
    let mut keys = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Ok(key) = PublicKey::from_openssh(line) {
            keys.push(key);
            continue;
        }
        // "options keytype base64 comment": find the key type token.
        let found = line.match_indices(' ').find_map(|(i, _)| {
            let rest = line[i + 1..].trim_start();
            let is_type = rest.starts_with("ssh-") || rest.starts_with("ecdsa-") || rest.starts_with("sk-");
            if is_type { PublicKey::from_openssh(rest).ok() } else { None }
        });
        if let Some(key) = found {
            keys.push(key);
        }
    }
    keys
}

/// Every key allowed to log in right now: the nvdata file (read on each
/// attempt, so edits apply at once) and the key baked in for development.
pub fn authorized_keys(paths: &Paths) -> Vec<PublicKey> {
    let mut keys = match std::fs::read_to_string(paths.authorized_keys()) {
        Ok(text) => parse_authorized_keys(&text),
        Err(_) => Vec::new(),
    };
    if let Some(dev) = super::dev_key() {
        keys.extend(parse_authorized_keys(dev));
    }
    keys
}

pub fn is_authorized(paths: &Paths, key: &PublicKey) -> bool {
    authorized_keys(paths).iter().any(|k| k.key_data() == key.key_data())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ED: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAILM+rvN+ot98qgEN796jTiQfZfG1KaT0PtFDJ/XFSqti user@example.com";

    #[test]
    fn parses_lines() {
        let text = format!("# comment\n\n{ED}\nno-pty,command=\"x y\" {ED}\nnot a key\n");
        let keys = parse_authorized_keys(&text);
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].key_data(), keys[1].key_data());
    }

    #[test]
    fn host_key_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("open/ssh_host_ed25519_key");
        let a = load_or_create_host_key(&path).unwrap();
        let b = load_or_create_host_key(&path).unwrap();
        assert_eq!(a.public_key().key_data(), b.public_key().key_data());
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
}
