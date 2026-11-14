//! SSH public keys as the web UI handles them: checking a key someone
//! pasted or uploaded, and describing a key the way `ssh-keygen -l` does.

use base64::Engine as _;
use sha2::{Digest, Sha256};

/// Key types that can be added.
pub const ACCEPTED_TYPES: &[&str] =
    &["ssh-ed25519", "ssh-rsa", "ecdsa-sha2-nistp256", "ecdsa-sha2-nistp384", "ecdsa-sha2-nistp521"];

const MAX_COMMENT: usize = 200;
/// Longest key line accepted (a 16384-bit RSA key is under 3 KiB).
const MAX_LINE: usize = 8 << 10;
const MIN_RSA_BITS: u32 = 1024;

/// A public key, checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicKey {
    pub key_type: String,
    /// The key, base64, as it goes in `authorized_keys`.
    pub blob: String,
    pub comment: String,
    /// `SHA256:` and the unpadded base64 of the key's SHA-256.
    pub fingerprint: String,
    pub bits: u32,
}

impl PublicKey {
    /// The `authorized_keys` line.
    pub fn line(&self) -> String {
        if self.comment.is_empty() {
            format!("{} {}", self.key_type, self.blob)
        } else {
            format!("{} {} {}", self.key_type, self.blob, self.comment)
        }
    }
}

/// Why a key was refused.  The messages are shown to the user; none of
/// them repeats the input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyError {
    Empty,
    PrivateKey,
    SeveralKeys,
    TooLong,
    HasOptions,
    UnsupportedType,
    Malformed,
    TypeMismatch,
    WeakRsa,
}

impl std::fmt::Display for KeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            KeyError::Empty => "No key was given.",
            KeyError::PrivateKey => {
                "That is a private key, and it was not stored.  Use the public key (the .pub file)."
            }
            KeyError::SeveralKeys => "Add one key at a time.",
            KeyError::TooLong => "That is too long to be a public key.",
            KeyError::HasOptions => "The line must start with the key type, e.g., ssh-ed25519.",
            KeyError::UnsupportedType => {
                "Unsupported key type.  Accepted: ssh-ed25519, ssh-rsa, and ecdsa-sha2-nistp256/384/521."
            }
            KeyError::Malformed => "That is not a valid public key.",
            KeyError::TypeMismatch => "The key data is of a different type than the line says.",
            KeyError::WeakRsa => "RSA keys shorter than 1024 bits are not accepted.",
        })
    }
}

/// True for anything that looks like private key material.
fn looks_private(text: &str) -> bool {
    text.contains("PRIVATE KEY") || text.contains("PuTTY-User-Key-File") || text.contains("-----BEGIN")
}

/// Checks one public key line (`type base64 [comment]`), as in a `.pub`
/// file.
pub fn parse_new_key(input: &str) -> Result<PublicKey, KeyError> {
    if looks_private(input) {
        return Err(KeyError::PrivateKey);
    }
    let mut lines = input.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#'));
    let line = lines.next().ok_or(KeyError::Empty)?;
    if lines.next().is_some() {
        return Err(KeyError::SeveralKeys);
    }
    if line.len() > MAX_LINE {
        return Err(KeyError::TooLong);
    }
    let mut parts = line.splitn(3, |c: char| c.is_ascii_whitespace());
    let key_type = parts.next().unwrap_or_default();
    if !ACCEPTED_TYPES.contains(&key_type) {
        let known_shape = key_type.starts_with("ssh-") || key_type.starts_with("ecdsa-") || key_type.starts_with("sk-");
        return Err(if known_shape { KeyError::UnsupportedType } else { KeyError::HasOptions });
    }
    let blob = parts.next().ok_or(KeyError::Malformed)?;
    let comment: String = parts.next().unwrap_or_default().trim().chars().filter(|c| !c.is_control()).take(MAX_COMMENT).collect();
    describe(key_type, blob, &comment)
}

/// Describes a key from an existing `authorized_keys` line; `None` when
/// the data is not a key of the type the line names.
pub fn describe_existing(key_type: &str, blob: &str, comment: &str) -> Option<PublicKey> {
    describe(key_type, blob, comment).ok()
}

fn describe(key_type: &str, blob: &str, comment: &str) -> Result<PublicKey, KeyError> {
    let raw = base64::engine::general_purpose::STANDARD.decode(blob).map_err(|_| KeyError::Malformed)?;
    let mut r = Reader(&raw);
    let inner = r.string().ok_or(KeyError::Malformed)?;
    if inner != key_type.as_bytes() {
        return Err(KeyError::TypeMismatch);
    }
    let bits = match key_type {
        "ssh-ed25519" => {
            let key = r.string().ok_or(KeyError::Malformed)?;
            if key.len() != 32 {
                return Err(KeyError::Malformed);
            }
            256
        }
        "ssh-rsa" => {
            let e = r.mpint_bits().ok_or(KeyError::Malformed)?;
            let n = r.mpint_bits().ok_or(KeyError::Malformed)?;
            if e < 2 {
                return Err(KeyError::Malformed);
            }
            if n < MIN_RSA_BITS {
                return Err(KeyError::WeakRsa);
            }
            n
        }
        "ecdsa-sha2-nistp256" | "ecdsa-sha2-nistp384" | "ecdsa-sha2-nistp521" => {
            let curve = &key_type["ecdsa-sha2-".len()..];
            let (bits, field_bytes) = match curve {
                "nistp256" => (256, 32),
                "nistp384" => (384, 48),
                _ => (521, 66),
            };
            if r.string().ok_or(KeyError::Malformed)? != curve.as_bytes() {
                return Err(KeyError::TypeMismatch);
            }
            let point = r.string().ok_or(KeyError::Malformed)?;
            // An uncompressed point: 0x04, X, Y.
            if point.len() != 1 + 2 * field_bytes || point[0] != 4 {
                return Err(KeyError::Malformed);
            }
            bits
        }
        // Other types in an existing file (e.g., ssh-dss): describable by
        // fingerprint only.
        _ => 0,
    };
    if bits != 0 && !r.0.is_empty() {
        return Err(KeyError::Malformed);
    }
    let digest = Sha256::digest(&raw);
    let fingerprint = format!("SHA256:{}", base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest));
    Ok(PublicKey {
        key_type: key_type.to_owned(),
        // Canonical padding, so the same key always compares equal.
        blob: base64::engine::general_purpose::STANDARD.encode(&raw),
        comment: comment.to_owned(),
        fingerprint,
        bits,
    })
}

/// SSH wire format reader (RFC 4251 section 5).
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn string(&mut self) -> Option<&'a [u8]> {
        let len = u32::from_be_bytes(self.0.get(..4)?.try_into().ok()?) as usize;
        let s = self.0.get(4..4 + len)?;
        self.0 = &self.0[4 + len..];
        Some(s)
    }

    /// The bit length of a non-negative multiple-precision integer.
    fn mpint_bits(&mut self) -> Option<u32> {
        let s = self.string()?;
        if s.first().is_some_and(|b| b & 0x80 != 0) {
            return None;
        }
        let s = &s[s.iter().take_while(|&&b| b == 0).count()..];
        let first = *s.first()?;
        Some((s.len() as u32 - 1) * 8 + (8 - first.leading_zeros()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Throwaway keys made with ssh-keygen; the expected values are what
    // `ssh-keygen -l -f` printed for each.
    const ED: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINxjRiA9rj/Ivh47IO5Y/LiMYa5eF7rf3mEK6rzozUOj ed@test";
    const RSA: &str = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQCSLDFcmVVDtuElLHcdK6TdGFmXtRJAMFCbgf0AiBpMWr4OcIUPlxq/nIfOdMAR5JKJh/PZsl9w5UPRqSAOsjx6wITP9Nu2NjH54JebxeMhaiKOfYipDyvV5uHE10G6y25h3frrQN/Uong2zGHh+uc2gWojbL3LZHgGLthrU2JLhYtulPhR0DWioA56NZPS/viyjji+khFoRZEObJ+NxnQwJISvJPswoDJqPw3UN20NH9NhtG22zIFAooIFHsaPueR0A1kV1LgHn+FnmwZwjFVLLzCrBdbIUQxJaxj5TTojhsTW4qdfGS39C04eTEfLX1X/vBrmQJq/B0Ge/ll47i7p rsa key with spaces";
    const EC256: &str = "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBGFiy0K6rIDTVhBd2p4Yar54GvVgqQvPdHdH/oO62UPlWZbIj2fia21ebA/ZQ2RLqkNJAZ730vLQ/KHDMJh6HN4= ec256";
    const EC521: &str = "ecdsa-sha2-nistp521 AAAAE2VjZHNhLXNoYTItbmlzdHA1MjEAAAAIbmlzdHA1MjEAAACFBAHTAt/OqI8cMVCq7FShX8Fpr4zNUf75Vz+TFIzfHTOgmlIkG8HWvahqZigHh5eauvuiSDVcdDhfBRWxmO4nVO2aXABIPbvLEbLCoLHholJ3ichhCm0hsi+UoodIPkBAYvBHQtXPQBTTfgR6+BHD6gUnfDb75/rIWFUuVdJ/TexiZOs9Gw== ec521";

    #[test]
    fn matches_ssh_keygen() {
        let cases = [
            (ED, 256, "SHA256:ChrmgdKKoWT6FLWac+x9RMS4LL11uAOZpq33PI1lmj4", "ed@test"),
            (RSA, 2048, "SHA256:K7XNopFDix/fd+hzoBGesaXrH+WknWIzmBIra/KHjeA", "rsa key with spaces"),
            (EC256, 256, "SHA256:VMNvUiMbWDOWfOuU2Ck8livrtYwwmciKtXXs+NJ2V00", "ec256"),
            (EC521, 521, "SHA256:/sLUHbCNgxChJLg/C26CpgpmdiT538KrvGw2a/u1gR8", "ec521"),
        ];
        for (line, bits, fingerprint, comment) in cases {
            let key = parse_new_key(line).unwrap();
            assert_eq!((key.bits, key.fingerprint.as_str(), key.comment.as_str()), (bits, fingerprint, comment));
            assert_eq!(key.line(), line);
        }
        assert_eq!(parse_new_key(&format!("\n# mine\n  {ED}  \n")).unwrap().line(), ED);
    }

    #[test]
    fn refusals() {
        assert_eq!(parse_new_key(""), Err(KeyError::Empty));
        assert_eq!(parse_new_key("hello"), Err(KeyError::HasOptions));
        assert_eq!(parse_new_key(&format!("no-pty {ED}")), Err(KeyError::HasOptions));
        assert_eq!(parse_new_key(&format!("{ED}\n{RSA}")), Err(KeyError::SeveralKeys));
        assert_eq!(parse_new_key("ssh-dss AAAAB3NzaC1kc3M= old"), Err(KeyError::UnsupportedType));
        assert_eq!(parse_new_key("ssh-ed25519"), Err(KeyError::Malformed));
        assert_eq!(parse_new_key("ssh-ed25519 not*base64"), Err(KeyError::Malformed));
        // An RSA key's data under the ed25519 name.
        let rsa_blob = RSA.split(' ').nth(1).unwrap();
        assert_eq!(parse_new_key(&format!("ssh-ed25519 {rsa_blob}")), Err(KeyError::TypeMismatch));
        // Truncated data.
        let ed_blob = ED.split(' ').nth(1).unwrap();
        assert_eq!(parse_new_key(&format!("ssh-ed25519 {}", &ed_blob[..40])), Err(KeyError::Malformed));
        assert_eq!(parse_new_key(&"x".repeat(MAX_LINE + 1)), Err(KeyError::TooLong));
    }

    #[test]
    fn private_keys_are_refused() {
        for text in [
            "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH PRIVATE KEY-----\n",
            "-----BEGIN RSA PRIVATE KEY-----\nMIIE\n",
            "PuTTY-User-Key-File-3: ssh-ed25519\nEncryption: none\n",
        ] {
            assert_eq!(parse_new_key(text), Err(KeyError::PrivateKey));
        }
        assert!(!KeyError::PrivateKey.to_string().contains("b3Bl"));
    }

    #[test]
    fn comment_is_cleaned() {
        let key = parse_new_key(&format!("{} tab\there\u{7}", &ED[..ED.rfind(' ').unwrap()])).unwrap();
        assert_eq!(key.comment, "tabhere");
    }
}
