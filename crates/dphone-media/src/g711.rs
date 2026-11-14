// SPDX-License-Identifier: GPL-2.0-or-later
//! G.711 mu-law and A-law.  These run on the ARM because the DSP has no
//! G.711 job (the stock firmware codes them host-side as well).  Tables are
//! built at compile time; coding is a lookup.

const fn ulaw_decode(u: u8) -> i16 {
    let u = !u;
    let exp = ((u >> 4) & 7) as i32;
    let mant = (u & 0x0f) as i32;
    let s = (((mant << 3) + 0x84) << exp) - 0x84;
    if u & 0x80 != 0 { -s as i16 } else { s as i16 }
}

const fn alaw_decode(a: u8) -> i16 {
    let a = a ^ 0x55;
    let mut t = ((a & 0x0f) as i32) << 4;
    let seg = ((a & 0x70) >> 4) as i32;
    match seg {
        0 => t += 8,
        1 => t += 0x108,
        _ => {
            t += 0x108;
            t <<= seg - 1;
        }
    }
    if a & 0x80 != 0 { t as i16 } else { -t as i16 }
}

const fn ulaw_encode(pcm: i16) -> u8 {
    let mut s = pcm as i32;
    let sign = if s < 0 {
        s = -s;
        0x80
    } else {
        0
    };
    if s > 32635 {
        s = 32635;
    }
    s += 0x84;
    let mut exp = 7;
    let mut mask = 0x4000;
    while exp > 0 && s & mask == 0 {
        exp -= 1;
        mask >>= 1;
    }
    let mant = (s >> (exp + 3)) & 0x0f;
    !((sign | (exp << 4) | mant) as u8)
}

const fn alaw_encode(pcm: i16) -> u8 {
    let mut v = (pcm as i32) >> 3;
    let mask = if v >= 0 {
        0xd5
    } else {
        v = -v - 1;
        0x55
    };
    let mut seg = 0;
    let mut end = 0x1f;
    while seg < 8 && v > end {
        seg += 1;
        end = (end << 1) | 1;
    }
    if seg >= 8 {
        return (0x7f ^ mask) as u8;
    }
    let low = if seg < 2 { (v >> 1) & 0x0f } else { (v >> seg) & 0x0f };
    (((seg << 4) | low) ^ mask) as u8
}

const fn decode_table(f: fn(u8) -> i16, ulaw: bool) -> [i16; 256] {
    let _ = f;
    let mut t = [0i16; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = if ulaw { ulaw_decode(i as u8) } else { alaw_decode(i as u8) };
        i += 1;
    }
    t
}

/// Encode table indexed by the top 14 bits of the sample (offset binary).
const fn encode_table(ulaw: bool) -> [u8; 16384] {
    let mut t = [0u8; 16384];
    let mut i = 0;
    while i < 16384 {
        let pcm = (((i as i32) - 8192) << 2) as i16;
        t[i] = if ulaw { ulaw_encode(pcm) } else { alaw_encode(pcm) };
        i += 1;
    }
    t
}

static ULAW_DEC: [i16; 256] = decode_table(ulaw_decode, true);
static ALAW_DEC: [i16; 256] = decode_table(alaw_decode, false);
static ULAW_ENC: [u8; 16384] = encode_table(true);
static ALAW_ENC: [u8; 16384] = encode_table(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Law {
    Mu,
    A,
}

impl Law {
    #[inline]
    pub fn decode_sample(self, b: u8) -> i16 {
        match self {
            Law::Mu => ULAW_DEC[usize::from(b)],
            Law::A => ALAW_DEC[usize::from(b)],
        }
    }

    #[inline]
    pub fn encode_sample(self, s: i16) -> u8 {
        let i = ((i32::from(s) >> 2) + 8192) as usize;
        match self {
            Law::Mu => ULAW_ENC[i],
            Law::A => ALAW_ENC[i],
        }
    }

    /// `out` and `input` have the same length.
    pub fn decode(self, input: &[u8], out: &mut [i16]) {
        for (o, &b) in out.iter_mut().zip(input) {
            *o = self.decode_sample(b);
        }
    }

    pub fn encode(self, input: &[i16], out: &mut [u8]) {
        for (o, &s) in out.iter_mut().zip(input) {
            *o = self.encode_sample(s);
        }
    }

    /// The byte that decodes to silence.
    pub fn silence(self) -> u8 {
        match self {
            Law::Mu => 0xff,
            Law::A => 0xd5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values() {
        assert_eq!(Law::Mu.encode_sample(0), 0xff);
        assert_eq!(Law::A.encode_sample(0), 0xd5);
        assert_eq!(Law::Mu.decode_sample(0xff), 0);
        assert_eq!(Law::Mu.decode_sample(0x00), -32124);
        assert_eq!(Law::Mu.decode_sample(0x80), 32124);
        assert_eq!(Law::A.decode_sample(0xd5), 8);
        assert_eq!(Law::A.decode_sample(0x2a), -32256);
        assert_eq!(Law::A.decode_sample(0xaa), 32256);
        // full-scale inputs
        assert_eq!(Law::Mu.encode_sample(i16::MAX), 0x80);
        assert_eq!(Law::Mu.encode_sample(i16::MIN), 0x00);
        assert_eq!(Law::A.encode_sample(i16::MAX), 0xaa);
        assert_eq!(Law::A.encode_sample(i16::MIN), 0x2a);
    }

    #[test]
    fn round_trip_error_is_bounded() {
        for law in [Law::Mu, Law::A] {
            for s in (i16::MIN..=i16::MAX).step_by(7) {
                let d = law.decode_sample(law.encode_sample(s));
                let err = (i32::from(d) - i32::from(s)).abs();
                // quantization step grows with level: at most about 1/16 of it
                let bound = (i32::from(s).abs() >> 4) + 16;
                assert!(err <= bound, "{law:?} {s} -> {d}");
                // decoding and re-encoding is stable
                assert_eq!(law.decode_sample(law.encode_sample(d)), d);
            }
        }
    }
}
