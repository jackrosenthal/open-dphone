//! The CRC the vendor updater and boot2 use: reflected CRC-32 (polynomial
//! 0xEDB88320), initial value 0xffffffff, **no** final inversion.

const fn table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

static TABLE: [u32; 256] = table();

/// Running CRC state.
#[derive(Clone, Copy, Debug)]
pub struct Crc(u32);

impl Default for Crc {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc {
    pub const fn new() -> Self {
        Crc(0xffff_ffff)
    }

    pub fn update(&mut self, data: &[u8]) {
        let mut c = self.0;
        for &b in data {
            c = TABLE[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
        }
        self.0 = c;
    }

    /// The value as stored in OTA headers and the ctrl record.
    pub const fn value(&self) -> u32 {
        self.0
    }
}

/// One-shot CRC of `data`.
pub fn crc(data: &[u8]) -> u32 {
    let mut c = Crc::new();
    c.update(data);
    c.value()
}

#[cfg(test)]
mod tests {
    #[test]
    fn check_value() {
        // Standard CRC-32 of "123456789" is 0xcbf43926; ours omits the final inversion.
        assert_eq!(super::crc(b"123456789"), !0xcbf4_3926);
    }
}
