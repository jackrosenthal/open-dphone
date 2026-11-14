//! Provisioned certificates: the TLS trust bundle `certs/certs.crt` and the
//! client certificates and keys under `clientcerts/`, kept the way stock
//! keeps them.
//!
//! The configuration carries them inline (stock `core`, `updateTrustedCerts`,
//! D60 2_9_28 at `0xb6ab4`):
//!
//! ```xml
//! <config>
//!   <certs>
//!     <cert label="pbx-ca" md5="...">MIIB...</cert>
//!     <clientcert label="phone.crt" md5="...">MIIB...</clientcert>
//!     <clientkey label="phone.key" md5="..." type="rsa">MIIE...</clientkey>
//!   </certs>
//! </config>
//! ```
//!
//! The element and attribute names are the ones that function reads; no
//! captured provisioning carries a `<certs>` section.  The merge tells
//! siblings apart by `id` (not by `label`), so several entries of one kind
//! survive it only when each has its own `id`.
//!
//! - `certs/certs.crt` is the base bundle of the root file system
//!   ([`BASE_BUNDLE_PATH`]) followed by every `<cert>`, each wrapped in PEM
//!   certificate markers.  It is what TLS clients on the phone trust when
//!   `allow_insecure_ssl` is off: SIP over TLS, HTTPS requests of apps, and
//!   HTTPS downloads of provisioned files.
//!
//!   Stock rebuilds the file from those two sources at every configuration
//!   merge (the merge routine at `0xbdc74` calls `updateTrustedCerts`
//!   unconditionally), through `certs.crt.tmp` and a rename; what the file
//!   held before plays no part.  Its start-up script copies the base bundle
//!   there when the file is missing (after a factory reset, which removes
//!   `certs/`).  A certificate that is also in the base bundle, or that is
//!   provisioned twice, is in the file twice: stock does not compare them.
//!
//!   The image's base bundle is the Mozilla CA bundle of nixpkgs (`cacert`);
//!   stock's is the same list from curl's `mk-ca-bundle`, as of its build.
//!   So a `certs.crt` that stock wrote is replaced with ours at the first
//!   start, and stock replaces it with its own again when it next runs.
//! - Each `<clientcert>` is written to `clientcerts/<label>` in the same
//!   wrapping, and each `<clientkey>` to `clientcerts/<label>` as a
//!   `PRIVATE KEY` block, or `<TYPE> PRIVATE KEY` when it has a `type`
//!   (upper-cased, as stock does).  Stock's consumer of these is the wired
//!   802.1X login.
//! - An entry's `md5` attribute is recorded in `md5s/<label>`.
//!
//! Files are rewritten only when their content changes, and never removed:
//! stock does not remove them either.

use std::io;

use crate::nvdata::Nvdata;
use crate::view::Config;

/// The base bundle of the root file system: public certificate authorities,
/// PEM.  Stock keeps its bundle at this path, and so does our image.
pub const BASE_BUNDLE_PATH: &str = "/usr/share/ssl/certs/cacert.pem";

/// The trust bundle inside nvdata.
pub const TRUST_BUNDLE: &str = "certs/certs.crt";

const CLIENT_DIR: &str = "clientcerts";

/// One `<cert>`, `<clientcert>`, or `<clientkey>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertEntry {
    pub label: String,
    pub md5: Option<String>,
    /// The element's text: the base64 body, without PEM markers.
    pub body: String,
    /// `<clientkey type="..">`, e.g., `rsa`.
    pub key_type: Option<String>,
}

/// The `<certs>` section of a merged configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProvisionedCerts {
    pub trusted: Vec<CertEntry>,
    pub client_certs: Vec<CertEntry>,
    pub client_keys: Vec<CertEntry>,
}

/// What [`Nvdata::update_trust`] wrote.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrustChange {
    /// Paths, relative to the nvdata root, whose content changed.
    pub written: Vec<String>,
    /// Entries skipped, with the reason.
    pub skipped: Vec<String>,
}

impl TrustChange {
    pub fn is_empty(&self) -> bool {
        self.written.is_empty()
    }
}

impl Config {
    /// Certificates and keys carried in the configuration.
    pub fn provisioned_certs(&self) -> ProvisionedCerts {
        let mut out = ProvisionedCerts::default();
        let Some(certs) = self.root().child("certs") else { return out };
        let entry = |e: &crate::xml::Element| CertEntry {
            label: e.attr("label").unwrap_or_default().to_owned(),
            md5: e.attr("md5").filter(|m| !m.is_empty()).map(str::to_owned),
            body: e.text().unwrap_or_default().trim().to_owned(),
            key_type: e.attr("type").filter(|t| !t.is_empty()).map(str::to_owned),
        };
        out.trusted = certs.children_named("cert").map(entry).collect();
        out.client_certs = certs.children_named("clientcert").map(entry).collect();
        out.client_keys = certs.children_named("clientkey").map(entry).collect();
        out
    }
}

fn pem_certificate(body: &str) -> String {
    format!("\n-----BEGIN CERTIFICATE-----\n{body}\n-----END CERTIFICATE-----\n")
}

fn pem_key(body: &str, key_type: Option<&str>) -> String {
    let kind = match key_type {
        Some(t) => format!("{} PRIVATE KEY", t.to_ascii_uppercase()),
        None => "PRIVATE KEY".to_owned(),
    };
    format!("\n-----BEGIN {kind}-----\n{body}\n-----END {kind}-----\n")
}

/// The trust bundle for a configuration: `base` (the root file system's
/// bundle) followed by every `<cert>`, as stock writes `certs.crt`.
pub fn trust_bundle(config: &Config, base: Option<&[u8]>) -> Vec<u8> {
    let mut out = base.map(<[u8]>::to_vec).unwrap_or_default();
    for cert in config.provisioned_certs().trusted {
        if !cert.body.is_empty() {
            out.extend_from_slice(pem_certificate(&cert.body).as_bytes());
        }
    }
    out
}

/// The PEM roots TLS clients trust: what `certs.crt` holds once
/// [`Nvdata::update_trust`] has run for `config`, whether or not it has
/// (it has not in recovery, where nothing is written).
///
/// `base` is the content of [`BASE_BUNDLE_PATH`].  Where that file is
/// missing (a run outside an image), `certs.crt` cannot be rebuilt and is
/// not written; then the roots are the file as found (`on_disk`) followed
/// by the provisioned certificates.
pub fn roots(config: &Config, base: Option<&[u8]>, on_disk: Option<&[u8]>) -> Vec<u8> {
    match base {
        Some(base) => trust_bundle(config, Some(base)),
        None => {
            let mut out = on_disk.map(<[u8]>::to_vec).unwrap_or_default();
            out.extend_from_slice(&trust_bundle(config, None));
            out
        }
    }
}

impl Nvdata {
    /// The trust bundle on disk (`certs/certs.crt`), if there is one.
    pub fn trust_bundle(&self) -> io::Result<Option<Vec<u8>>> {
        self.read(TRUST_BUNDLE)
    }

    /// A client certificate or key (`clientcerts/<label>`), PEM.
    pub fn client_cert(&self, label: &str) -> io::Result<Option<Vec<u8>>> {
        if !crate::nvdata::valid_name(label) {
            return Err(crate::nvdata::bad_name(label));
        }
        self.read(&format!("{CLIENT_DIR}/{label}"))
    }

    /// Bring `certs/certs.crt`, `clientcerts/`, and the `md5s/` entries of
    /// the certificates in line with `config`.  `base` is the content of
    /// [`BASE_BUNDLE_PATH`].  The bundle is rebuilt from `base` and the
    /// configuration alone, so a file from stock, or from an image with
    /// another base bundle, is replaced; a missing one (first start, factory
    /// reset) is created.
    pub fn update_trust(&self, config: &Config, base: Option<&[u8]>) -> io::Result<TrustChange> {
        let mut change = TrustChange::default();
        let certs = config.provisioned_certs();

        let bundle = trust_bundle(config, base);
        // Nothing to trust and no bundle yet: leave the directory alone.
        if !(bundle.is_empty() && !self.exists(TRUST_BUNDLE)) {
            self.write_if_changed(TRUST_BUNDLE, &bundle, &mut change)?;
        }
        for cert in &certs.trusted {
            self.note_md5(cert, &mut change)?;
        }
        self.write_client_certs(&certs, &mut change)?;
        Ok(change)
    }

    /// The `clientcerts/` half of [`Nvdata::update_trust`] alone.
    pub fn update_client_certs(&self, config: &Config) -> io::Result<TrustChange> {
        let mut change = TrustChange::default();
        self.write_client_certs(&config.provisioned_certs(), &mut change)?;
        Ok(change)
    }

    fn write_client_certs(&self, certs: &ProvisionedCerts, change: &mut TrustChange) -> io::Result<()> {
        let files = certs
            .client_certs
            .iter()
            .map(|c| (c, pem_certificate(&c.body)))
            .chain(certs.client_keys.iter().map(|k| (k, pem_key(&k.body, k.key_type.as_deref()))));
        for (entry, pem) in files {
            if !crate::nvdata::valid_name(&entry.label) {
                change.skipped.push(format!("client certificate or key with unusable label {:?}", entry.label));
                continue;
            }
            if entry.body.is_empty() {
                change.skipped.push(format!("{}: empty", entry.label));
                continue;
            }
            self.write_if_changed(&format!("{CLIENT_DIR}/{}", entry.label), pem.as_bytes(), change)?;
            self.note_md5(entry, change)?;
        }
        Ok(())
    }

    fn write_if_changed(&self, rel: &str, data: &[u8], change: &mut TrustChange) -> io::Result<()> {
        if self.read(rel)?.as_deref() == Some(data) {
            return Ok(());
        }
        self.write(rel, data)?;
        change.written.push(rel.to_owned());
        Ok(())
    }

    fn note_md5(&self, entry: &CertEntry, change: &mut TrustChange) -> io::Result<()> {
        let Some(md5) = &entry.md5 else { return Ok(()) };
        if !crate::nvdata::valid_name(&entry.label) {
            return Ok(());
        }
        if self.cached_md5(&entry.label)?.as_deref() != Some(md5.trim()) {
            self.write_cached_md5(&entry.label, md5)?;
            change.written.push(format!("md5s/{}", entry.label));
        }
        Ok(())
    }
}
