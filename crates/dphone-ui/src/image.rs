//! Images that arrive as files at run time: an installed app's launcher
//! icon and the pictures an app shows from its package.  PNG only (what
//! stock app packages carry).  Built-in artwork does not come through here;
//! it is rasterized at build time (see [`crate::icons`]).

use std::sync::Arc;

/// The most pixels a file may hold; a larger one is refused before any
/// pixel is decoded (the main screens are 480x272 at most).
const MAX_PIXELS: u64 = 1 << 20;
/// The most memory the decoder may use for one file.
const DECODE_LIMIT: usize = 8 << 20;

/// Decoded pixels: row-major RGBA, `width * height * 4` bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u16,
    pub height: u16,
    pub pixels: Arc<[u8]>,
}

/// Decodes a PNG file (any color type and depth; the first frame of an
/// animated one) to RGBA.
pub fn decode_png(data: &[u8]) -> Result<Image, String> {
    let mut decoder = png::Decoder::new_with_limits(std::io::Cursor::new(data), png::Limits { bytes: DECODE_LIMIT });
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let (w, h) = reader.info().size();
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS || w > u32::from(u16::MAX) || h > u32::from(u16::MAX) {
        return Err(format!("a {w}x{h} image is too large"));
    }
    let size = reader.output_buffer_size().ok_or("the image is too large")?;
    let mut buf = vec![0u8; size];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let n = info.color_type.samples();
    let count = info.width as usize * info.height as usize;
    let src = buf.get(..count * n).ok_or("the image is truncated")?;
    let mut pixels = Vec::with_capacity(count * 4);
    for p in src.chunks_exact(n) {
        match n {
            1 => pixels.extend_from_slice(&[p[0], p[0], p[0], 255]),
            2 => pixels.extend_from_slice(&[p[0], p[0], p[0], p[1]]),
            3 => pixels.extend_from_slice(&[p[0], p[1], p[2], 255]),
            _ => pixels.extend_from_slice(&p[..4]),
        }
    }
    Ok(Image { width: info.width as u16, height: info.height as u16, pixels: pixels.into() })
}

impl Image {
    /// The image no larger than `max_w` by `max_h`, keeping its shape:
    /// itself when it fits, else reduced by averaging (alpha-weighted, so
    /// transparent pixels do not darken edges).  Never enlarged.
    pub fn fit(&self, max_w: u16, max_h: u16) -> Image {
        let (w, h) = (u32::from(self.width), u32::from(self.height));
        let (mw, mh) = (u32::from(max_w.max(1)), u32::from(max_h.max(1)));
        if w <= mw && h <= mh {
            return self.clone();
        }
        // The tighter of the two ratios decides.
        let (nw, nh) = if w * mh >= h * mw { (mw, (h * mw / w).max(1)) } else { ((w * mh / h).max(1), mh) };
        let mut out = Vec::with_capacity((nw * nh * 4) as usize);
        for y in 0..nh {
            let (y0, y1) = (y * h / nh, ((y + 1) * h / nh).max(y * h / nh + 1));
            for x in 0..nw {
                let (x0, x1) = (x * w / nw, ((x + 1) * w / nw).max(x * w / nw + 1));
                let (mut r, mut g, mut b, mut a, mut n) = (0u32, 0u32, 0u32, 0u32, 0u32);
                for sy in y0..y1 {
                    for sx in x0..x1 {
                        let p = &self.pixels[((sy * w + sx) * 4) as usize..];
                        let pa = u32::from(p[3]);
                        r += u32::from(p[0]) * pa;
                        g += u32::from(p[1]) * pa;
                        b += u32::from(p[2]) * pa;
                        a += pa;
                        n += 1;
                    }
                }
                // Fully transparent blocks have no color to average.
                let channel = |sum: u32| sum.checked_div(a).unwrap_or(0) as u8;
                out.extend_from_slice(&[channel(r), channel(g), channel(b), (a / n) as u8]);
            }
        }
        Image { width: nw as u16, height: nh as u16, pixels: out.into() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(color);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(data).unwrap();
        out
    }

    #[test]
    fn color_types_become_rgba() {
        let i = decode_png(&png(2, 1, png::ColorType::Rgb, &[1, 2, 3, 4, 5, 6])).unwrap();
        assert_eq!((i.width, i.height, &i.pixels[..]), (2, 1, &[1, 2, 3, 255, 4, 5, 6, 255][..]));
        let i = decode_png(&png(1, 2, png::ColorType::GrayscaleAlpha, &[9, 100, 8, 0])).unwrap();
        assert_eq!((i.width, i.height, &i.pixels[..]), (1, 2, &[9, 9, 9, 100, 8, 8, 8, 0][..]));
        let i = decode_png(&png(1, 1, png::ColorType::Rgba, &[1, 2, 3, 4])).unwrap();
        assert_eq!(&i.pixels[..], &[1, 2, 3, 4]);
        assert!(decode_png(b"not a png").is_err());
        assert!(decode_png(&png(1, 1, png::ColorType::Rgba, &[1, 2, 3, 4])[..40]).is_err());
    }

    #[test]
    fn oversized_images_are_refused_before_decoding() {
        // A header that claims 5000x5000 with no pixel data behind it.
        let mut data = png(1, 1, png::ColorType::Grayscale, &[0]);
        data[16..20].copy_from_slice(&5000u32.to_be_bytes());
        data[20..24].copy_from_slice(&5000u32.to_be_bytes());
        let crc = {
            // CRC-32 of the IHDR type and body.
            let mut c = 0xffff_ffffu32;
            for &b in &data[12..29] {
                c ^= u32::from(b);
                for _ in 0..8 {
                    c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
                }
            }
            !c
        };
        data[29..33].copy_from_slice(&crc.to_be_bytes());
        assert_eq!(decode_png(&data).unwrap_err(), "a 5000x5000 image is too large");
    }

    #[test]
    fn fitting_keeps_the_shape_and_never_enlarges() {
        let px: Vec<u8> = (0..8 * 4).flat_map(|i| if i % 8 < 4 { [200, 0, 0, 255] } else { [0, 0, 0, 0] }).collect();
        let i = Image { width: 8, height: 4, pixels: px.into() };
        assert_eq!(i.fit(8, 4), i);
        assert_eq!(i.fit(100, 100), i);
        let half = i.fit(4, 4);
        assert_eq!((half.width, half.height), (4, 2));
        // The opaque red half stays red; the transparent half stays clear.
        assert_eq!(&half.pixels[..8], &[200, 0, 0, 255, 200, 0, 0, 255]);
        assert_eq!(&half.pixels[8..16], &[0, 0, 0, 0, 0, 0, 0, 0]);
        let narrow = i.fit(100, 1);
        assert_eq!((narrow.width, narrow.height), (2, 1));
        // An edge pixel that is half covered keeps its color at half alpha.
        let edge = Image { width: 2, height: 1, pixels: vec![90, 60, 30, 255, 0, 0, 0, 0].into() }.fit(1, 1);
        assert_eq!(&edge.pixels[..], &[90, 60, 30, 127]);
    }
}
