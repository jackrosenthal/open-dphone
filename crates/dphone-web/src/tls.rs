//! HTTPS: a device-generated self-signed certificate (P-256, pure Rust)
//! and the rustls server configuration on the RustCrypto provider.

use dphone_config::Nvdata;
use p256::ecdsa::{DerSignature, SigningKey};
use p256::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::io;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use x509_cert::builder::{Builder, CertificateBuilder, Profile};
use x509_cert::der::asn1::{Ia5String, UtcTime};
use x509_cert::der::Encode;
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::ext::pkix::SubjectAltName;
use x509_cert::name::Name;
use x509_cert::serial_number::SerialNumber;
use x509_cert::spki::SubjectPublicKeyInfoOwned;
use x509_cert::time::{Time, Validity};

/// Where the HTTPS private key and certificate are kept between boots.
pub trait KeyStore: Send + Sync {
    fn get(&self, name: &str) -> io::Result<Option<Vec<u8>>>;
    fn put(&self, name: &str, data: &[u8]) -> io::Result<()>;
}

/// Keeps them under `/nvdata/open/`, leaving stock's own files alone.
pub struct NvdataKeyStore {
    nv: Nvdata,
}

impl NvdataKeyStore {
    pub fn new(nv: Nvdata) -> Self {
        NvdataKeyStore { nv }
    }
}

impl KeyStore for NvdataKeyStore {
    fn get(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        self.nv.open_get(name)
    }
    fn put(&self, name: &str, data: &[u8]) -> io::Result<()> {
        self.nv.open_put(name, data)
    }
}

const KEY_NAME: &str = "web-tls-key.der";
const CERT_NAME: &str = "web-tls-cert.der";

// The phone has no clock until SNTP answers, so the certificate's validity
// cannot depend on "now": it runs from 2020 to the last date UTCTime holds.
const NOT_BEFORE: u64 = 1_577_836_800; // 2020-01-01T00:00:00Z
const NOT_AFTER: u64 = 2_524_607_999; // 2049-12-31T23:59:59Z

fn err(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

fn new_key() -> io::Result<SigningKey> {
    // Draw scalars until one is in range (all but a 2^-32 sliver are).
    for _ in 0..8 {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(err)?;
        if let Ok(k) = SigningKey::from_slice(&bytes) {
            return Ok(k);
        }
    }
    Err(err("could not generate a key"))
}

/// Makes a self-signed certificate for `key`.
fn self_signed(key: &SigningKey, hostname: &str) -> io::Result<Vec<u8>> {
    let cn: String = hostname
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
        .take(60)
        .collect();
    let cn = if cn.is_empty() { "dphone".to_owned() } else { cn };
    let subject = Name::from_str(&format!("CN={cn},O=Open DPhone")).map_err(err)?;
    let mut serial = [0u8; 16];
    getrandom::fill(&mut serial).map_err(err)?;
    serial[0] &= 0x7f;
    serial[0] |= 0x01;
    let time = |secs| UtcTime::from_unix_duration(Duration::from_secs(secs)).map(Time::UtcTime).map_err(err);
    let validity = Validity { not_before: time(NOT_BEFORE)?, not_after: time(NOT_AFTER)? };
    let spki = SubjectPublicKeyInfoOwned::from_key(*key.verifying_key()).map_err(err)?;
    let profile = Profile::Leaf {
        issuer: subject.clone(),
        enable_key_agreement: true,
        enable_key_encipherment: false,
    };
    let mut builder =
        CertificateBuilder::new(profile, SerialNumber::new(&serial).map_err(err)?, validity, subject, spki, key)
            .map_err(err)?;
    let san = SubjectAltName(vec![GeneralName::DnsName(Ia5String::new(&cn).map_err(err)?)]);
    builder.add_extension(&san).map_err(err)?;
    let cert = builder.build::<DerSignature>().map_err(err)?;
    cert.to_der().map_err(err)
}

/// Loads the stored key and certificate, or makes and stores new ones.
pub(crate) fn key_and_cert(store: &dyn KeyStore, hostname: &str) -> io::Result<(Vec<u8>, Vec<u8>)> {
    if let (Some(key), Some(cert)) = (store.get(KEY_NAME)?, store.get(CERT_NAME)?) {
        if SigningKey::from_pkcs8_der(&key).is_ok() && !cert.is_empty() {
            return Ok((key, cert));
        }
        log::warn!("stored HTTPS key is unreadable; generating a new one");
    }
    let key = new_key()?;
    let cert = self_signed(&key, hostname)?;
    let key_der = key.to_pkcs8_der().map_err(err)?.as_bytes().to_vec();
    store.put(KEY_NAME, &key_der)?;
    store.put(CERT_NAME, &cert)?;
    log::info!("generated a new HTTPS certificate");
    Ok((key_der, cert))
}

/// The rustls configuration: RustCrypto provider, TLS 1.2 and 1.3, no
/// client certificates.
pub(crate) fn server_config(store: &dyn KeyStore, hostname: &str) -> io::Result<Arc<rustls::ServerConfig>> {
    let (key, cert) = key_and_cert(store, hostname)?;
    let provider = Arc::new(rustls_rustcrypto::provider());
    let mut cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(err)?
        .with_no_client_auth()
        .with_single_cert(vec![CertificateDer::from(cert)], PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)))
        .map_err(err)?;
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(cfg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Mem(Mutex<HashMap<String, Vec<u8>>>);
    impl KeyStore for Mem {
        fn get(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
            Ok(self.0.lock().unwrap().get(name).cloned())
        }
        fn put(&self, name: &str, data: &[u8]) -> io::Result<()> {
            self.0.lock().unwrap().insert(name.to_owned(), data.to_vec());
            Ok(())
        }
    }

    #[test]
    fn generates_once_and_reuses() {
        let store = Mem::default();
        let (k1, c1) = key_and_cert(&store, "d60-test").unwrap();
        let (k2, c2) = key_and_cert(&store, "other").unwrap();
        assert_eq!(k1, k2);
        assert_eq!(c1, c2);
        // The certificate parses and names the host.
        use x509_cert::der::Decode;
        let cert = x509_cert::Certificate::from_der(&c1).unwrap();
        assert!(cert.tbs_certificate.subject.to_string().contains("d60-test"));
        server_config(&store, "d60-test").unwrap();
    }

    #[test]
    fn replaces_damaged_key() {
        let store = Mem::default();
        store.put(KEY_NAME, b"junk").unwrap();
        store.put(CERT_NAME, b"junk").unwrap();
        let (k, _) = key_and_cert(&store, "x").unwrap();
        assert!(SigningKey::from_pkcs8_der(&k).is_ok());
    }
}
