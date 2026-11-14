//! JFFS2's checksum: the kernel's `crc32(0, data, len)`, i.e., reflected
//! CRC-32 with initial value 0 and no final inversion.

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

pub fn crc32(data: &[u8]) -> u32 {
    let mut c = 0u32;
    for &b in data {
        c = TABLE[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    c
}
