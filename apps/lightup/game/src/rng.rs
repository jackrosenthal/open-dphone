//! A small seedable random number generator (xoshiro256**, seeded through
//! SplitMix64).  The same seed always gives the same puzzle.

pub struct Rng([u64; 4]);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let mut z = seed;
        let mut next = || {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            x ^ (x >> 31)
        };
        Rng([next(), next(), next(), next()])
    }

    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.0;
        let out = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        out
    }

    /// A uniform number below `n` (`n` above 0).
    pub fn below(&mut self, n: u32) -> u32 {
        // Multiply and keep the high half; redraw the few values that
        // would make some results more likely than others.
        let reject = n.wrapping_neg() % n;
        loop {
            let m = u64::from((self.next_u64() >> 32) as u32) * u64::from(n);
            if (m as u32) >= reject {
                return (m >> 32) as u32;
            }
        }
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i as u32 + 1) as usize);
        }
    }
}
