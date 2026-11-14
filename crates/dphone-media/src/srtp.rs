// SPDX-License-Identifier: GPL-2.0-or-later
//! SRTP and SRTCP (RFC 3711) with SDES keys (RFC 4568):
//! AES_CM_128_HMAC_SHA1_80 and AES_CM_128_HMAC_SHA1_32, on RustCrypto.  The
//! stock firmware used the SoC's crypto engine for this; the port has no
//! driver for it.  No MKI, no key lifetime, key derivation rate 0.

use aes::Aes128;
use aes::cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};
use hmac::{Hmac, Mac};
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

pub const KEY_LEN: usize = 16;
pub const SALT_LEN: usize = 14;
/// Bytes SRTP may add to an RTP packet.
pub const SRTP_MAX_OVERHEAD: usize = 10;
/// Bytes SRTCP adds to an RTCP packet: index word and tag.
pub const SRTCP_OVERHEAD: usize = 14;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Suite {
    AesCm128HmacSha1_80,
    AesCm128HmacSha1_32,
}

impl Suite {
    /// The name used in an SDP `a=crypto` line.
    pub fn sdp_name(self) -> &'static str {
        match self {
            Suite::AesCm128HmacSha1_80 => "AES_CM_128_HMAC_SHA1_80",
            Suite::AesCm128HmacSha1_32 => "AES_CM_128_HMAC_SHA1_32",
        }
    }

    pub fn from_sdp_name(name: &str) -> Option<Self> {
        match name {
            "AES_CM_128_HMAC_SHA1_80" => Some(Suite::AesCm128HmacSha1_80),
            "AES_CM_128_HMAC_SHA1_32" => Some(Suite::AesCm128HmacSha1_32),
            _ => None,
        }
    }

    /// RTP authentication tag length.  (SRTCP always uses 10 bytes.)
    pub fn tag_len(self) -> usize {
        match self {
            Suite::AesCm128HmacSha1_80 => 10,
            Suite::AesCm128HmacSha1_32 => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SrtpError {
    /// Too short to be a protected packet.
    Short,
    /// The authentication tag does not match.
    Auth,
    /// Already seen, or too old for the replay window.
    Replay,
    /// Malformed key material.
    Key,
}

impl std::fmt::Display for SrtpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SrtpError::Short => "packet too short for SRTP",
            SrtpError::Auth => "SRTP authentication failed",
            SrtpError::Replay => "SRTP replay",
            SrtpError::Key => "malformed SRTP key",
        })
    }
}

impl std::error::Error for SrtpError {}

/// Master key and salt, as carried in an SDES `inline:` parameter.
#[derive(Clone, PartialEq, Eq)]
pub struct MasterKey {
    pub key: [u8; KEY_LEN],
    pub salt: [u8; SALT_LEN],
}

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MasterKey(..)")
    }
}

impl MasterKey {
    /// A fresh random key.
    pub fn generate() -> Result<Self, SrtpError> {
        let mut raw = [0u8; KEY_LEN + SALT_LEN];
        getrandom::getrandom(&mut raw).map_err(|_| SrtpError::Key)?;
        Ok(Self::from_raw(&raw))
    }

    fn from_raw(raw: &[u8; KEY_LEN + SALT_LEN]) -> Self {
        let mut k = Self { key: [0; KEY_LEN], salt: [0; SALT_LEN] };
        k.key.copy_from_slice(&raw[..KEY_LEN]);
        k.salt.copy_from_slice(&raw[KEY_LEN..]);
        k
    }

    /// From the base64 after `inline:` (anything from the first `|` on, a
    /// lifetime or MKI, is ignored).
    pub fn from_sdes_inline(text: &str) -> Result<Self, SrtpError> {
        let b64 = text.split('|').next().unwrap_or("").trim();
        let raw = base64_decode(b64).ok_or(SrtpError::Key)?;
        let raw: &[u8; KEY_LEN + SALT_LEN] = raw.as_slice().try_into().map_err(|_| SrtpError::Key)?;
        Ok(Self::from_raw(raw))
    }

    /// The base64 to put after `inline:`.
    pub fn to_sdes_inline(&self) -> String {
        let mut raw = [0u8; KEY_LEN + SALT_LEN];
        raw[..KEY_LEN].copy_from_slice(&self.key);
        raw[KEY_LEN..].copy_from_slice(&self.salt);
        base64_encode(&raw)
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let v = u32::from(c[0]) << 16 | u32::from(*c.get(1).unwrap_or(&0)) << 8 | u32::from(*c.get(2).unwrap_or(&0));
        out.push(B64[(v >> 18) as usize & 63] as char);
        out.push(B64[(v >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { B64[(v >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { B64[v as usize & 63] as char } else { '=' });
    }
    out
}

pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let text = text.trim_end_matches('=');
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for ch in text.bytes() {
        let v = B64.iter().position(|&b| b == ch)? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// AES in counter mode as SRTP uses it: XORs the keystream for `iv` (its
/// last two bytes are the block counter, starting at 0) into `data`.
fn aes_cm(cipher: &Aes128, iv: &[u8; 16], data: &mut [u8]) {
    let mut ctr = *iv;
    for (i, chunk) in data.chunks_mut(16).enumerate() {
        ctr[14..16].copy_from_slice(&(i as u16).to_be_bytes());
        let mut block = GenericArray::clone_from_slice(&ctr);
        cipher.encrypt_block(&mut block);
        for (d, k) in chunk.iter_mut().zip(block.iter()) {
            *d ^= k;
        }
    }
}

/// The SRTP key derivation function with index 0 (derivation rate 0).
fn derive(master: &MasterKey, label: u8, out: &mut [u8]) {
    let cipher = Aes128::new(GenericArray::from_slice(&master.key));
    let mut iv = [0u8; 16];
    iv[..SALT_LEN].copy_from_slice(&master.salt);
    iv[7] ^= label;
    out.fill(0);
    aes_cm(&cipher, &iv, out);
}

struct Keys {
    cipher: Aes128,
    salt: [u8; SALT_LEN],
    mac: HmacSha1,
}

impl Keys {
    fn new(master: &MasterKey, first_label: u8) -> Self {
        let mut key = [0u8; KEY_LEN];
        let mut auth = [0u8; 20];
        let mut salt = [0u8; SALT_LEN];
        derive(master, first_label, &mut key);
        derive(master, first_label + 1, &mut auth);
        derive(master, first_label + 2, &mut salt);
        Self {
            cipher: Aes128::new(GenericArray::from_slice(&key)),
            salt,
            mac: <HmacSha1 as Mac>::new_from_slice(&auth).expect("HMAC takes any key length"),
        }
    }

    fn iv(&self, ssrc: &[u8], index: u64) -> [u8; 16] {
        let mut iv = [0u8; 16];
        iv[..SALT_LEN].copy_from_slice(&self.salt);
        for (d, s) in iv[4..8].iter_mut().zip(ssrc) {
            *d ^= s;
        }
        for (d, s) in iv[8..14].iter_mut().zip(&index.to_be_bytes()[2..]) {
            *d ^= s;
        }
        iv
    }

    fn tag(&self, data: &[u8], trailer: &[u8]) -> [u8; 20] {
        let mut mac = self.mac.clone();
        mac.update(data);
        mac.update(trailer);
        mac.finalize().into_bytes().into()
    }
}

fn tags_equal(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A 64-packet replay window over a growing index.
#[derive(Clone, Copy, Debug, Default)]
struct Replay {
    started: bool,
    top: u64,
    mask: u64,
}

impl Replay {
    fn check(&self, index: u64) -> bool {
        if !self.started || index > self.top {
            return true;
        }
        let back = self.top - index;
        back < 64 && self.mask & (1 << back) == 0
    }

    fn update(&mut self, index: u64) {
        if !self.started || index > self.top {
            let shift = if self.started { index - self.top } else { 64 };
            self.mask = if shift >= 64 { 1 } else { (self.mask << shift) | 1 };
            self.top = index;
            self.started = true;
        } else {
            self.mask |= 1 << (self.top - index);
        }
    }
}

/// One direction of an SRTP session (one SSRC).
pub struct SrtpContext {
    suite: Suite,
    rtp: Keys,
    rtcp: Keys,
    /// Rollover counter and highest sequence number.
    roc: u32,
    s_l: u16,
    started: bool,
    replay: Replay,
    rtcp_index: u32,
    rtcp_replay: Replay,
}

impl SrtpContext {
    pub fn new(suite: Suite, master: &MasterKey) -> Self {
        Self {
            suite,
            rtp: Keys::new(master, 0),
            rtcp: Keys::new(master, 3),
            roc: 0,
            s_l: 0,
            started: false,
            replay: Replay::default(),
            rtcp_index: 0,
            rtcp_replay: Replay::default(),
        }
    }

    pub fn suite(&self) -> Suite {
        self.suite
    }

    fn header_len(pkt: &[u8]) -> Option<usize> {
        if pkt.len() < 12 {
            return None;
        }
        let mut off = 12 + 4 * usize::from(pkt[0] & 0x0f);
        if pkt[0] & 0x10 != 0 {
            if pkt.len() < off + 4 {
                return None;
            }
            off += 4 + 4 * usize::from(u16::from_be_bytes([pkt[off + 2], pkt[off + 3]]));
        }
        (off <= pkt.len()).then_some(off)
    }

    /// Protects the RTP packet in `buf[..len]` in place; `buf` must have
    /// room for the tag.  Returns the new length.  Sequence numbers must be
    /// sent in order (they are, by this sender).
    pub fn protect(&mut self, buf: &mut [u8], len: usize) -> Result<usize, SrtpError> {
        let tag_len = self.suite.tag_len();
        if buf.len() < len + tag_len {
            return Err(SrtpError::Short);
        }
        let hdr = Self::header_len(&buf[..len]).ok_or(SrtpError::Short)?;
        let seq = u16::from_be_bytes([buf[2], buf[3]]);
        if self.started && seq < self.s_l && self.s_l - seq > 0x8000 {
            self.roc = self.roc.wrapping_add(1);
        }
        self.started = true;
        self.s_l = seq;
        let index = u64::from(self.roc) << 16 | u64::from(seq);
        let iv = self.rtp.iv(&buf[8..12], index);
        aes_cm(&self.rtp.cipher, &iv, &mut buf[hdr..len]);
        let tag = self.rtp.tag(&buf[..len], &self.roc.to_be_bytes());
        buf[len..len + tag_len].copy_from_slice(&tag[..tag_len]);
        Ok(len + tag_len)
    }

    /// Checks and decrypts the SRTP packet in `pkt` in place.  Returns the
    /// length of the RTP packet.
    pub fn unprotect(&mut self, pkt: &mut [u8]) -> Result<usize, SrtpError> {
        let tag_len = self.suite.tag_len();
        if pkt.len() < 12 + tag_len {
            return Err(SrtpError::Short);
        }
        let len = pkt.len() - tag_len;
        let hdr = Self::header_len(&pkt[..len]).ok_or(SrtpError::Short)?;
        let seq = u16::from_be_bytes([pkt[2], pkt[3]]);
        // RFC 3711 section 3.3.1: guess the rollover counter
        let v = if !self.started {
            self.roc
        } else if self.s_l < 0x8000 {
            if seq > self.s_l && seq - self.s_l > 0x8000 { self.roc.wrapping_sub(1) } else { self.roc }
        } else if self.s_l - 0x8000 > seq {
            self.roc.wrapping_add(1)
        } else {
            self.roc
        };
        let index = u64::from(v) << 16 | u64::from(seq);
        if !self.replay.check(index) {
            return Err(SrtpError::Replay);
        }
        let tag = self.rtp.tag(&pkt[..len], &v.to_be_bytes());
        if !tags_equal(&tag[..tag_len], &pkt[len..]) {
            return Err(SrtpError::Auth);
        }
        let iv = self.rtp.iv(&pkt[8..12], index);
        let (head, _) = pkt.split_at_mut(len);
        aes_cm(&self.rtp.cipher, &iv, &mut head[hdr..]);
        self.replay.update(index);
        if !self.started || index > (u64::from(self.roc) << 16 | u64::from(self.s_l)) {
            self.roc = v;
            self.s_l = seq;
            self.started = true;
        }
        Ok(len)
    }

    /// Protects the compound RTCP packet in `buf[..len]` in place; `buf`
    /// needs [`SRTCP_OVERHEAD`] bytes of room.  Returns the new length.
    pub fn protect_rtcp(&mut self, buf: &mut [u8], len: usize) -> Result<usize, SrtpError> {
        if len < 8 || buf.len() < len + SRTCP_OVERHEAD {
            return Err(SrtpError::Short);
        }
        let index = self.rtcp_index & 0x7fff_ffff;
        self.rtcp_index = self.rtcp_index.wrapping_add(1);
        let iv = self.rtcp.iv(&buf[4..8], u64::from(index));
        aes_cm(&self.rtcp.cipher, &iv, &mut buf[8..len]);
        buf[len..len + 4].copy_from_slice(&(index | 0x8000_0000).to_be_bytes());
        let tag = self.rtcp.tag(&buf[..len + 4], &[]);
        buf[len + 4..len + 14].copy_from_slice(&tag[..10]);
        Ok(len + SRTCP_OVERHEAD)
    }

    /// Checks and decrypts an SRTCP packet in place.  Returns the length of
    /// the RTCP packet.
    pub fn unprotect_rtcp(&mut self, pkt: &mut [u8]) -> Result<usize, SrtpError> {
        if pkt.len() < 8 + SRTCP_OVERHEAD {
            return Err(SrtpError::Short);
        }
        let len = pkt.len() - SRTCP_OVERHEAD;
        let word = u32::from_be_bytes([pkt[len], pkt[len + 1], pkt[len + 2], pkt[len + 3]]);
        let index = u64::from(word & 0x7fff_ffff);
        if !self.rtcp_replay.check(index) {
            return Err(SrtpError::Replay);
        }
        let tag = self.rtcp.tag(&pkt[..len + 4], &[]);
        if !tags_equal(&tag[..10], &pkt[len + 4..]) {
            return Err(SrtpError::Auth);
        }
        if word & 0x8000_0000 != 0 {
            let iv = self.rtcp.iv(&pkt[4..8], index);
            let (head, _) = pkt.split_at_mut(len);
            aes_cm(&self.rtcp.cipher, &iv, &mut head[8..]);
        }
        self.rtcp_replay.update(index);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// RFC 3711 appendix B.2: AES-CM keystream.
    #[test]
    fn rfc3711_keystream() {
        let cipher = Aes128::new(GenericArray::from_slice(&hex("2B7E151628AED2A6ABF7158809CF4F3C")));
        let mut iv = [0u8; 16];
        iv[..14].copy_from_slice(&hex("F0F1F2F3F4F5F6F7F8F9FAFBFCFD"));
        let mut ks = [0u8; 48];
        aes_cm(&cipher, &iv, &mut ks);
        assert_eq!(
            ks.to_vec(),
            hex("E03EAD0935C95E80E166B16DD92B4EB4 D23513162B02D0F72A43A2FE4A5F97AB 41E95B3BB0A2E8DD477901E4FCA894C0")
        );
    }

    /// RFC 3711 appendix B.3: key derivation.
    #[test]
    fn rfc3711_key_derivation() {
        let master = MasterKey {
            key: hex("E1F97A0D3E018BE0D64FA32C06DE4139").try_into().unwrap(),
            salt: hex("0EC675AD498AFEEBB6960B3AABE6").try_into().unwrap(),
        };
        let mut key = [0u8; 16];
        derive(&master, 0, &mut key);
        assert_eq!(key.to_vec(), hex("C61E7A93744F39EE10734AFE3FF7A087"));
        let mut salt = [0u8; 14];
        derive(&master, 2, &mut salt);
        assert_eq!(salt.to_vec(), hex("30CBBC08863D8C85D49DB34A9AE1"));
        let mut auth = [0u8; 94];
        derive(&master, 1, &mut auth);
        assert_eq!(
            auth.to_vec(),
            hex("CEBE321F6FF7716B6FD4AB49AF256A15 6D38BAA48F0A0ACF3C34E2359E6CDBCE
                 E049646C43D9327AD175578EF7227098 6371C10C9A369AC2F94A8C5FBCDDDC25
                 6D6E919A48B610EF17C2041E47403576 6B68642C59BBFC2F34DB60DBDFB2")
        );
    }

    fn rtp(seq: u16, payload: &[u8]) -> Vec<u8> {
        let mut p = vec![0x80, 0x00];
        p.extend_from_slice(&seq.to_be_bytes());
        p.extend_from_slice(&(u32::from(seq) * 160).to_be_bytes());
        p.extend_from_slice(&0xcafe_babeu32.to_be_bytes());
        p.extend_from_slice(payload);
        p
    }

    fn pair(suite: Suite) -> (SrtpContext, SrtpContext) {
        let key = MasterKey::from_sdes_inline("WVNfX19zZW1jdGwgKCkgewkyMjA7fQp9CnVubGVz|2^20|1:4").unwrap();
        (SrtpContext::new(suite, &key), SrtpContext::new(suite, &key))
    }

    #[test]
    fn round_trip_tamper_replay() {
        for suite in [Suite::AesCm128HmacSha1_80, Suite::AesCm128HmacSha1_32] {
            let (mut tx, mut rx) = pair(suite);
            let payload: Vec<u8> = (0..160).map(|i| i as u8).collect();
            let plain = rtp(1, &payload);
            let mut buf = plain.clone();
            buf.resize(plain.len() + SRTP_MAX_OVERHEAD, 0);
            let n = tx.protect(&mut buf, plain.len()).unwrap();
            assert_eq!(n, plain.len() + suite.tag_len());
            assert_eq!(&buf[..12], &plain[..12]);
            assert_ne!(&buf[12..plain.len()], &plain[12..]);

            let mut bad = buf[..n].to_vec();
            bad[20] ^= 1;
            assert_eq!(rx.unprotect(&mut bad), Err(SrtpError::Auth));

            let mut good = buf[..n].to_vec();
            let m = rx.unprotect(&mut good).unwrap();
            assert_eq!(&good[..m], &plain[..]);

            let mut again = buf[..n].to_vec();
            assert_eq!(rx.unprotect(&mut again), Err(SrtpError::Replay));
            assert_eq!(rx.unprotect(&mut [0u8; 8]), Err(SrtpError::Short));
        }
    }

    #[test]
    fn rollover_and_reordering() {
        let (mut tx, mut rx) = pair(Suite::AesCm128HmacSha1_80);
        let mut held: Option<Vec<u8>> = None;
        // cross the sequence wrap, delivering one packet late
        for i in 0..40u32 {
            let seq = 65_520u16.wrapping_add(i as u16);
            let plain = rtp(seq, &[i as u8; 20]);
            let mut buf = plain.clone();
            buf.resize(plain.len() + SRTP_MAX_OVERHEAD, 0);
            let n = tx.protect(&mut buf, plain.len()).unwrap();
            buf.truncate(n);
            if seq == 65_534 {
                held = Some(buf);
                continue;
            }
            let m = rx.unprotect(&mut buf).unwrap();
            assert_eq!(&buf[..m], &plain[..], "seq {seq}");
            if seq == 3 {
                // the packet from before the wrap arrives now
                let mut late = held.take().unwrap();
                let m = rx.unprotect(&mut late).unwrap();
                assert_eq!(&late[12..m], &[14u8; 20]);
            }
        }
        assert_eq!(tx.roc, 1);
        assert_eq!(rx.roc, 1);
    }

    #[test]
    fn rtcp_round_trip() {
        let (mut tx, mut rx) = pair(Suite::AesCm128HmacSha1_32);
        let mut plain = Vec::new();
        crate::rtp::build_report(&mut plain, 77, None, None, "a@b", false);
        for _ in 0..3 {
            let mut buf = plain.clone();
            buf.resize(plain.len() + SRTCP_OVERHEAD, 0);
            let n = tx.protect_rtcp(&mut buf, plain.len()).unwrap();
            assert_eq!(&buf[..8], &plain[..8]);
            assert_ne!(&buf[8..plain.len()], &plain[8..]);
            let mut copy = buf[..n].to_vec();
            let m = rx.unprotect_rtcp(&mut buf[..n]).unwrap();
            assert_eq!(&buf[..m], &plain[..]);
            assert_eq!(rx.unprotect_rtcp(&mut copy), Err(SrtpError::Replay));
        }
    }

    #[test]
    fn sdes_keys() {
        let k = MasterKey::generate().unwrap();
        let text = k.to_sdes_inline();
        assert_eq!(text.len(), 40);
        assert_eq!(MasterKey::from_sdes_inline(&text).unwrap(), k);
        assert_eq!(MasterKey::from_sdes_inline("AAAA"), Err(SrtpError::Key));
        assert_eq!(base64_encode(b"hi"), "aGk=");
        assert_eq!(base64_decode("aGk=").unwrap(), b"hi");
        assert_eq!(Suite::from_sdp_name("AES_CM_128_HMAC_SHA1_32"), Some(Suite::AesCm128HmacSha1_32));
    }
}
