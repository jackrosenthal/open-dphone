//! An in-memory NAND for tests: small geometry, bad blocks, failing blocks,
//! and power cuts that leave the interrupted erase or write half done.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use dphone_hal::{HalError, Mtd, MtdInfo, MtdPartition, Result};

pub const PAGE: u32 = 512;
pub const BLOCK: u32 = 4096;

/// Stock partition order, so that `root0` is mtd6 and `root1` mtd7.
pub const PARTS: [(&str, u32); 9] = [
    ("boot1", 1),
    ("boot2", 1),
    ("ctrl", 4),
    ("nvdata", 1),
    ("k0", 6),
    ("k1", 6),
    ("root0", 10),
    ("root1", 10),
    ("userdata", 1),
];

struct Part {
    info: MtdInfo,
    data: Vec<u8>,
    bad: Vec<bool>,
}

struct Inner {
    parts: Vec<Part>,
    /// Erase/write/mark operations performed so far.
    ops: u64,
    /// Power fails during this operation (0-based).
    cut_at: Option<u64>,
    dead: bool,
    rng: u64,
    /// (partition, block): programs here flip a bit.
    corrupt_writes: HashSet<(String, usize)>,
    /// (partition, block): erases here fail.
    failing_erases: HashSet<(String, usize)>,
}

impl Inner {
    fn rnd(&mut self) -> u64 {
        // xorshift64
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }
    /// Counts a mutating operation; true when power fails during it.
    fn step(&mut self) -> Result<bool> {
        if self.dead {
            return Err(off());
        }
        let cut = self.cut_at == Some(self.ops);
        self.ops += 1;
        if cut {
            self.dead = true;
        }
        Ok(cut)
    }
}

fn off() -> HalError {
    HalError::Unavailable("power is off".into())
}

#[derive(Clone)]
pub struct MemNand(Arc<Mutex<Inner>>);

impl MemNand {
    pub fn new() -> Self {
        let parts = PARTS
            .iter()
            .enumerate()
            .map(|(i, (name, blocks))| Part {
                info: MtdInfo {
                    name: name.to_string(),
                    index: i as u32,
                    size: (*blocks * BLOCK) as u64,
                    erase_size: BLOCK,
                    write_size: PAGE,
                    oob_size: 16,
                },
                data: vec![0xff; (*blocks * BLOCK) as usize],
                bad: vec![false; *blocks as usize],
            })
            .collect();
        MemNand(Arc::new(Mutex::new(Inner {
            parts,
            ops: 0,
            cut_at: None,
            dead: false,
            rng: 0x9e37_79b9_7f4a_7c15,
            corrupt_writes: HashSet::new(),
            failing_erases: HashSet::new(),
        })))
    }

    fn with<R>(&self, name: &str, f: impl FnOnce(&mut Part) -> R) -> R {
        let mut g = self.0.lock().unwrap();
        f(g.parts.iter_mut().find(|p| p.info.name == name).expect("partition"))
    }

    pub fn contents(&self, name: &str) -> Vec<u8> {
        self.with(name, |p| p.data.clone())
    }
    /// Loads raw contents at the start of a partition, bypassing NAND rules.
    pub fn load(&self, name: &str, data: &[u8]) {
        self.with(name, |p| {
            p.data.fill(0xff);
            p.data[..data.len()].copy_from_slice(data);
        })
    }
    /// Flips one bit, bypassing NAND rules (flash that changed by itself).
    pub fn flip_bit(&self, name: &str, offset: usize) {
        self.with(name, |p| p.data[offset] ^= 0x04)
    }
    /// An independent flash with the same contents and bad blocks.
    pub fn clone_contents(&self) -> MemNand {
        let copy = MemNand::new();
        let g = self.0.lock().unwrap();
        let mut c = copy.0.lock().unwrap();
        for (to, from) in c.parts.iter_mut().zip(&g.parts) {
            to.data.clone_from(&from.data);
            to.bad.clone_from(&from.bad);
        }
        drop(c);
        copy
    }
    pub fn set_bad(&self, name: &str, block: usize) {
        self.with(name, |p| p.bad[block] = true)
    }
    pub fn is_bad(&self, name: &str, block: usize) -> bool {
        self.with(name, |p| p.bad[block])
    }
    pub fn corrupt_writes(&self, name: &str, block: usize) {
        self.0.lock().unwrap().corrupt_writes.insert((name.to_string(), block));
    }
    pub fn fail_erases(&self, name: &str, block: usize) {
        self.0.lock().unwrap().failing_erases.insert((name.to_string(), block));
    }
    pub fn ops(&self) -> u64 {
        self.0.lock().unwrap().ops
    }
    /// Arms a power cut during the `n`th mutating operation from now.
    pub fn cut_after(&self, n: u64, seed: u64) {
        let mut g = self.0.lock().unwrap();
        g.cut_at = Some(g.ops + n);
        g.rng = seed.wrapping_mul(0x2545_f491_4f6c_dd1d) | 1;
    }
    pub fn is_dead(&self) -> bool {
        self.0.lock().unwrap().dead
    }
    pub fn power_on(&self) {
        let mut g = self.0.lock().unwrap();
        g.dead = false;
        g.cut_at = None;
    }
}

impl Mtd for MemNand {
    fn partitions(&self) -> Result<Vec<MtdInfo>> {
        Ok(self.0.lock().unwrap().parts.iter().map(|p| p.info.clone()).collect())
    }
    fn open(&mut self, name: &str) -> Result<Box<dyn MtdPartition>> {
        let g = self.0.lock().unwrap();
        let (idx, p) = g
            .parts
            .iter()
            .enumerate()
            .find(|(_, p)| p.info.name == name)
            .ok_or_else(|| HalError::Unavailable(format!("no MTD partition named {name}")))?;
        Ok(Box::new(MemPart { nand: self.clone(), idx, info: p.info.clone() }))
    }
}

struct MemPart {
    nand: MemNand,
    idx: usize,
    info: MtdInfo,
}

impl MtdPartition for MemPart {
    fn info(&self) -> &MtdInfo {
        &self.info
    }

    fn is_bad(&mut self, offset: u64) -> Result<bool> {
        let g = self.nand.0.lock().unwrap();
        if g.dead {
            return Err(off());
        }
        Ok(g.parts[self.idx].bad[(offset / BLOCK as u64) as usize])
    }

    fn erase(&mut self, offset: u64) -> Result<()> {
        assert_eq!(offset % BLOCK as u64, 0, "erase must be block aligned");
        let mut g = self.nand.0.lock().unwrap();
        let cut = g.step()?;
        let block = (offset / BLOCK as u64) as usize;
        if g.parts[self.idx].bad[block] {
            return Err(HalError::BadBlock(offset));
        }
        if g.failing_erases.contains(&(self.info.name.clone(), block)) {
            return Err(HalError::io("erase", std::io::Error::other("erase failed")));
        }
        let range = offset as usize..offset as usize + BLOCK as usize;
        if cut {
            // Half-erased: bits drift toward 1.
            for i in range {
                let r = g.rnd() as u8;
                g.parts[self.idx].data[i] |= r;
            }
            return Err(off());
        }
        g.parts[self.idx].data[range].fill(0xff);
        Ok(())
    }

    fn read(&mut self, offset: u64, buf: &mut [u8]) -> Result<()> {
        let g = self.nand.0.lock().unwrap();
        if g.dead {
            return Err(off());
        }
        let d = &g.parts[self.idx].data;
        let end = offset as usize + buf.len();
        assert!(end <= d.len(), "read past the end of {}", self.info.name);
        buf.copy_from_slice(&d[offset as usize..end]);
        Ok(())
    }

    fn write(&mut self, offset: u64, data: &[u8]) -> Result<()> {
        assert_eq!(offset % PAGE as u64, 0, "write must be page aligned");
        let mut g = self.nand.0.lock().unwrap();
        let cut = g.step()?;
        let start = offset as usize;
        assert!(start + data.len() <= g.parts[self.idx].data.len(), "write past the end");
        let first = start / BLOCK as usize;
        let last = (start + data.len().max(1) - 1) / BLOCK as usize;
        if (first..=last).any(|b| g.parts[self.idx].bad[b]) {
            return Err(HalError::BadBlock(offset));
        }
        let pages = data.len().div_ceil(PAGE as usize);
        let (whole, torn) = if cut {
            let k = (g.rnd() % pages.max(1) as u64) as usize;
            (k, true)
        } else {
            (pages, false)
        };
        let whole_len = (whole * PAGE as usize).min(data.len());
        for (i, d) in data[..whole_len].iter().enumerate() {
            // NAND programming only clears bits.
            g.parts[self.idx].data[start + i] &= *d;
        }
        if torn {
            let end = (whole_len + PAGE as usize).min(data.len());
            for (i, d) in data[..end].iter().enumerate().skip(whole_len) {
                let r = g.rnd() as u8;
                g.parts[self.idx].data[start + i] &= *d | r;
            }
            return Err(off());
        }
        if !data.is_empty() && (first..=last).any(|b| g.corrupt_writes.contains(&(self.info.name.clone(), b))) {
            g.parts[self.idx].data[start] ^= 0x10;
        }
        Ok(())
    }

    fn mark_bad(&mut self, offset: u64) -> Result<()> {
        let mut g = self.nand.0.lock().unwrap();
        g.step()?;
        g.parts[self.idx].bad[(offset / BLOCK as u64) as usize] = true;
        Ok(())
    }
}
