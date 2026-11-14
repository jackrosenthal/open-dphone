//! Kernel images for the k0/k1 slots: a zImage with an appended DTB whose
//! header words 0x28 (start) and 0x2c (end) tell the stock boot2 where to
//! copy the image and how long it is.  boot2 enters the image at `start`.
//!
//! Format knowledge from static analysis of boot2 (kernel-re/boot/NOTES.md
//! in the reverse-engineering tree).

use std::fmt;

pub const ZIMAGE_MAGIC: u32 = 0x016f_2818;
pub const DTB_MAGIC: [u8; 4] = [0xd0, 0x0d, 0xfe, 0xed];
pub const BLOCK: u32 = 0x20000;
pub const DEFAULT_SLOT: u32 = 0x20_0000;
/// boot2 itself, its NAND bounce buffers, and the vectors live below this.
const LOW_KEEPOUT_END: u32 = 0x40000;
const ATAG_LEN: u32 = 0x500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Model {
    pub name: &'static str,
    /// Where boot2 builds its ATAG list.
    pub atag_base: u32,
    pub ram_size: u32,
}

pub const MODELS: [Model; 7] = [
    Model {
        name: "D40",
        atag_base: 0x20_0000,
        ram_size: 0x0800_0000,
    },
    Model {
        name: "D45",
        atag_base: 0x20_0000,
        ram_size: 0x0800_0000,
    },
    Model {
        name: "D50",
        atag_base: 0x20_0000,
        ram_size: 0x0800_0000,
    },
    Model {
        name: "D70",
        atag_base: 0x20_0000,
        ram_size: 0x1000_0000,
    },
    Model {
        name: "D60",
        atag_base: 0x40_0000,
        ram_size: 0x0800_0000,
    },
    Model {
        name: "D62",
        atag_base: 0x40_0000,
        ram_size: 0x1000_0000,
    },
    Model {
        name: "D65",
        atag_base: 0x40_0000,
        ram_size: 0x1000_0000,
    },
];

pub fn model(name: &str) -> Result<Model, String> {
    MODELS
        .iter()
        .copied()
        .find(|m| m.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| {
            format!("unknown model {name:?} (expected one of D40, D45, D50, D60, D62, D65, D70)")
        })
}

fn word(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(data[off..off + 4].try_into().unwrap())
}

/// Reasons boot2 would fail to load the range `[start, end)`.
pub fn span_problems(start: u32, end: u32, slot: u32, m: Model) -> Vec<String> {
    let mut p = Vec::new();
    let size = end.wrapping_sub(start);
    if size > slot {
        p.push(format!(
            "image is {size:#x} bytes but the slot is {slot:#x}: boot2 prints \"Flash read failed!\" and hangs \
             (and every bad block in the slot lowers the limit by 0x20000)"
        ));
    }
    if start < LOW_KEEPOUT_END {
        p.push("load range overlaps boot2 or its NAND buffers (below 0x40000)".into());
    }
    if start < m.atag_base + ATAG_LEN && end > m.atag_base {
        p.push(format!(
            "load range overlaps the ATAG list at {:#010x}",
            m.atag_base + 4
        ));
    }
    if start < m.ram_size && end > m.ram_size - 0x10000 {
        p.push(format!(
            "load range overlaps boot2 bss/stack at the top of RAM ({:#010x})",
            m.ram_size - 0x10000
        ));
    }
    if start >= m.ram_size {
        p.push(format!(
            "load address {start:#010x} is above this model's RAM"
        ));
    }
    p
}

pub struct Built {
    pub image: Vec<u8>,
    pub load: u32,
    pub end: u32,
}

impl fmt::Display for Built {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let len = self.image.len() as u32;
        write!(
            f,
            "kernel image: {len} bytes ({len:#x}), load {:#010x}..{:#010x}, {} of {} blocks, {} bytes spare",
            self.load,
            self.end,
            len.div_ceil(BLOCK),
            DEFAULT_SLOT / BLOCK,
            DEFAULT_SLOT as i64 - len as i64
        )
    }
}

/// Concatenate zImage and DTB, pad to a word, and patch the header.
pub fn build(
    zimage: &[u8],
    dtb: &[u8],
    m: Model,
    load: u32,
    slot: u32,
    force: bool,
) -> Result<Built, String> {
    if zimage.len() < 0x30 || word(zimage, 0x24) != ZIMAGE_MAGIC {
        return Err("input has no zImage magic at 0x24".into());
    }
    if dtb.len() < 8 || dtb[..4] != DTB_MAGIC {
        return Err("DTB does not start with 0xd00dfeed".into());
    }
    let dtb_len = u32::from_be_bytes(dtb[4..8].try_into().unwrap()) as usize;
    if dtb_len != dtb.len() {
        return Err(format!(
            "DTB header says {dtb_len} bytes but the file has {}",
            dtb.len()
        ));
    }
    // The decompressor looks for the appended DTB right after the zImage
    // proper, whose length is in its own header.
    let own = word(zimage, 0x2c).wrapping_sub(word(zimage, 0x28)) as usize;
    if own != zimage.len() {
        return Err(format!(
            "zImage header length ({own}) differs from the file length ({}): already has an appended DTB or is truncated",
            zimage.len()
        ));
    }
    if load & 3 != 0 {
        return Err("load address must be word aligned".into());
    }
    let mut image = Vec::with_capacity(zimage.len() + dtb.len() + 3);
    image.extend_from_slice(zimage);
    image.extend_from_slice(dtb);
    image.resize(image.len().next_multiple_of(4), 0);
    let end = load + image.len() as u32;
    let problems = span_problems(load, end, slot, m);
    if !problems.is_empty() && !force {
        return Err(format!("refusing: {}", problems.join("; ")));
    }
    image[0x28..0x2c].copy_from_slice(&load.to_le_bytes());
    image[0x2c..0x30].copy_from_slice(&end.to_le_bytes());
    Ok(Built { image, load, end })
}

/// Check a finished k0/k1 image the way boot2 will see it.
pub fn check(image: &[u8], m: Model, slot: u32) -> Result<(u32, u32), String> {
    if image.len() < 0x30 || word(image, 0x24) != ZIMAGE_MAGIC {
        return Err(
            "no zImage magic at 0x24: boot2 prints \"Image magic number not detected\" and hangs"
                .into(),
        );
    }
    let (start, end) = (word(image, 0x28), word(image, 0x2c));
    if end <= start {
        return Err("header end <= start: boot2 prints \"kstart >= kend\" and hangs".into());
    }
    if (end - start) as usize != image.len() {
        return Err(format!(
            "header length {:#x} differs from the file length {:#x}: boot2 would not load the whole image \
             (was the header patched with `kernel-image`?)",
            end - start,
            image.len()
        ));
    }
    let problems = span_problems(start, end, slot, m);
    if !problems.is_empty() {
        return Err(problems.join("; "));
    }
    Ok((start, end))
}
