//! JFFS2 data compressors: `rtime` and `zlib`.

use crate::{Result, bad};

pub const COMPR_NONE: u8 = 0;
pub const COMPR_ZERO: u8 = 1;
pub const COMPR_RTIME: u8 = 2;
pub const COMPR_RUBINMIPS: u8 = 3;
pub const COMPR_COPY: u8 = 4;
pub const COMPR_DYNRUBIN: u8 = 5;
pub const COMPR_ZLIB: u8 = 6;
pub const COMPR_LZO: u8 = 7;

/// `rtime` compression (fs/jffs2/compr_rtime.c).  Returns `None` unless the
/// whole input compresses to fewer bytes than it has.
pub fn rtime_compress(data: &[u8]) -> Option<Vec<u8>> {
    let mut positions = [0usize; 256];
    let mut out = Vec::with_capacity(data.len());
    let mut pos = 0;
    while pos < data.len() {
        if out.len() + 2 > data.len() {
            return None;
        }
        let value = data[pos];
        out.push(value);
        pos += 1;
        let mut backpos = positions[value as usize];
        positions[value as usize] = pos;
        let mut runlen = 0u8;
        while backpos < pos && pos < data.len() && data[pos] == data[backpos] && runlen < 255 {
            backpos += 1;
            pos += 1;
            runlen += 1;
        }
        out.push(runlen);
    }
    (out.len() < data.len()).then_some(out)
}

pub fn rtime_decompress(input: &[u8], destlen: usize) -> Result<Vec<u8>> {
    let mut positions = [0usize; 256];
    let mut out = Vec::with_capacity(destlen);
    let mut pos = 0;
    while out.len() < destlen {
        if pos + 2 > input.len() {
            return bad("rtime data ends early");
        }
        let value = input[pos];
        let repeat = input[pos + 1] as usize;
        pos += 2;
        out.push(value);
        let mut backoffs = positions[value as usize];
        positions[value as usize] = out.len();
        let end = (out.len() + repeat).min(destlen);
        while out.len() < end {
            let b = out[backoffs];
            out.push(b);
            backoffs += 1;
        }
    }
    Ok(out)
}

/// zlib compression as a standard zlib stream, which is what `mkfs.jffs2`
/// writes and the kernel accepts.  Returns `None` if it does not shrink.
pub fn zlib_compress(data: &[u8]) -> Option<Vec<u8>> {
    let out = miniz_oxide::deflate::compress_to_vec_zlib(data, 9);
    (out.len() < data.len()).then_some(out)
}

pub fn zlib_decompress(input: &[u8], destlen: usize) -> Result<Vec<u8>> {
    // The kernel treats the data as raw deflate unless the first two bytes
    // look like a zlib header without a preset dictionary.
    let looks_zlib = input.len() >= 2
        && input[0] & 0x0f == 8
        && input[1] & 0x20 == 0
        && (u16::from(input[0]) << 8 | u16::from(input[1])) % 31 == 0;
    let res = if looks_zlib {
        // Like the kernel, do not insist on the Adler-32 trailer.
        miniz_oxide::inflate::decompress_to_vec_with_limit(&input[2..], destlen)
    } else {
        miniz_oxide::inflate::decompress_to_vec_with_limit(input, destlen)
    };
    match res {
        Ok(v) if v.len() == destlen => Ok(v),
        Ok(v) => bad(format!(
            "zlib data inflates to {} bytes, expected {destlen}",
            v.len()
        )),
        Err(e) => bad(format!("zlib data does not inflate: {:?}", e.status)),
    }
}

/// Decompress one data node's payload.
pub fn decompress(compr: u8, input: &[u8], destlen: usize) -> Result<Vec<u8>> {
    match compr {
        COMPR_NONE => {
            if input.len() != destlen {
                return bad("uncompressed node with csize != dsize");
            }
            Ok(input.to_vec())
        }
        COMPR_ZERO => Ok(vec![0; destlen]),
        COMPR_RTIME => rtime_decompress(input, destlen),
        COMPR_ZLIB => zlib_decompress(input, destlen),
        other => bad(format!("unsupported JFFS2 compression type {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtime_round_trip() {
        let mut data = Vec::new();
        for i in 0..4096u32 {
            data.push(b"abcabcabcd    \n"[(i % 15) as usize]);
        }
        let c = rtime_compress(&data).unwrap();
        assert!(c.len() < data.len());
        assert_eq!(rtime_decompress(&c, data.len()).unwrap(), data);
        // Overlapping copy: a long run of one byte.
        let zeros = vec![7u8; 1000];
        let c = rtime_compress(&zeros).unwrap();
        assert_eq!(rtime_decompress(&c, 1000).unwrap(), zeros);
        // Incompressible input is refused.
        let noise: Vec<u8> = (0..=255u8).collect();
        assert!(rtime_compress(&noise).is_none());
    }

    #[test]
    fn zlib_round_trip() {
        let data = b"hello hello hello hello hello hello hello".repeat(20);
        let c = zlib_compress(&data).unwrap();
        assert_eq!(zlib_decompress(&c, data.len()).unwrap(), data);
        let raw = miniz_oxide::deflate::compress_to_vec(&data, 6);
        assert_eq!(zlib_decompress(&raw, data.len()).unwrap(), data);
    }
}
