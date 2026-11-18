//! The embedded frontend, precompressed at build time by `build.rs`.

/// One embedded file.
pub struct Asset {
    pub path: &'static str,
    pub mime: &'static str,
    pub etag: &'static str,
    /// Content-hashed name: may be cached forever.
    pub immutable: bool,
    /// Uncompressed length.
    #[allow(dead_code)]
    pub len: usize,
    /// The file as is; absent when compressed copies exist.
    pub raw: Option<&'static [u8]>,
    pub gzip: Option<&'static [u8]>,
    pub brotli: Option<&'static [u8]>,
}

include!(concat!(env!("OUT_DIR"), "/assets.rs"));

/// Whether the real frontend is embedded (as opposed to the placeholder
/// page of a build without `web/dist`).
pub fn have_frontend() -> bool {
    HAVE_FRONTEND
}

pub fn find(path: &str) -> Option<&'static Asset> {
    let path = if path == "/" { "/index.html" } else { path };
    ASSETS.iter().find(|a| a.path == path)
}

/// Inflates a gzip member written by `build.rs` or [`gzip`].
pub fn gunzip(gz: &[u8]) -> Option<Vec<u8>> {
    if gz.len() < 18 || gz[0] != 0x1f || gz[1] != 0x8b || gz[2] != 8 {
        return None;
    }
    let flags = gz[3];
    let mut pos = 10;
    if flags & 0x04 != 0 {
        let n = u16::from_le_bytes([*gz.get(pos)?, *gz.get(pos + 1)?]) as usize;
        pos += 2 + n;
    }
    for bit in [0x08, 0x10] {
        if flags & bit != 0 {
            pos += gz.get(pos..)?.iter().position(|b| *b == 0)? + 1;
        }
    }
    if flags & 0x02 != 0 {
        pos += 2;
    }
    let body = gz.get(pos..gz.len().checked_sub(8)?)?;
    let out = miniz_oxide::inflate::decompress_to_vec_with_limit(body, 64 << 20).ok()?;
    let crc = u32::from_le_bytes(gz[gz.len() - 8..gz.len() - 4].try_into().ok()?);
    (crc32fast::hash(&out) == crc).then_some(out)
}

/// A gzip member around `data`.
pub fn gzip(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
    out.extend(miniz_oxide::deflate::compress_to_vec(data, 6));
    out.extend(crc32fast::hash(data).to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_present() {
        let a = find("/").expect("index.html is always embedded");
        assert!(a.mime.starts_with("text/html"));
        assert!(a.raw.is_some() || a.gzip.is_some());
        if let Some(gz) = a.gzip {
            assert_eq!(gunzip(gz).unwrap().len(), a.len);
        }
    }

    #[test]
    fn gzip_round_trip() {
        let data = b"hello hello hello hello".repeat(50);
        assert_eq!(gunzip(&gzip(&data)).unwrap(), data);
        assert!(gunzip(b"not gzip").is_none());
    }
}
