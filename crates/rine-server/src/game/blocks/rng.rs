//! Fast random source for block behaviour (vanilla uses the level's
//! `RandomSource`, seeded from the clock, so its exact sequence is not
//! observable; we keep the same distributions: `nextInt(bound)`,
//! `nextFloat`, `nextDouble`, `nextBoolean`).

/// xoroshiro128++.
#[derive(Clone, Copy, Debug)]
pub struct Rng {
    s0: u64,
    s1: u64,
}

impl Default for Rng {
    fn default() -> Self {
        Self::new(0x2545_F491_4F6C_DD1D)
    }
}

#[inline]
fn splitmix(x: &mut u64) -> u64 {
    *x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Rng {
    pub(super) fn feature_random(&self) -> rine_worldgen::feature::rng::WorldgenRng {
        rine_worldgen::feature::rng::WorldgenRng::direct(
            rine_worldgen::random::Xoroshiro::from_longs(self.s0 as i64, self.s1 as i64),
        )
    }

    pub(super) fn finish_feature_random(&mut self, rng: rine_worldgen::feature::rng::WorldgenRng) {
        (self.s0, self.s1) = rng.source().raw_state();
    }

    pub fn new(seed: u64) -> Self {
        let mut x = seed;
        let s0 = splitmix(&mut x);
        let s1 = splitmix(&mut x);
        Self { s0, s1: if s0 | s1 == 0 { 1 } else { s1 } }
    }

    /// Independent stream for a key (chunk position, tick...).
    pub fn derive(seed: u64, a: i64, b: i64) -> Self {
        let mut x = seed ^ (a as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (b as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
        let s0 = splitmix(&mut x);
        let s1 = splitmix(&mut x);
        Self { s0, s1: if s0 | s1 == 0 { 1 } else { s1 } }
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let (s0, mut s1) = (self.s0, self.s1);
        let r = s0.wrapping_add(s1).rotate_left(17).wrapping_add(s0);
        s1 ^= s0;
        self.s0 = s0.rotate_left(49) ^ s1 ^ (s1 << 21);
        self.s1 = s1.rotate_left(28);
        r
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Uniform in `0..bound` (`nextInt(bound)`); 0 for bound <= 0.
    #[inline]
    pub fn next_int(&mut self, bound: i32) -> i32 {
        if bound <= 0 {
            return 0;
        }
        // Lemire's multiply-shift with rejection: exact uniformity.
        let b = bound as u32;
        loop {
            let m = (self.next_u32() as u64) * (b as u64);
            let low = m as u32;
            if low >= b.wrapping_neg() % b {
                return (m >> 32) as i32;
            }
        }
    }

    /// Uniform in `lo..=hi` (`Mth.nextInt(random, lo, hi)`).
    #[inline]
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        if lo >= hi {
            lo
        } else {
            lo + self.next_int(hi - lo + 1)
        }
    }

    #[inline]
    pub fn next_float(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / (1u32 << 24) as f32)
    }

    #[inline]
    pub fn next_double(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    #[inline]
    pub fn next_bool(&mut self) -> bool {
        self.next_u64() >> 63 != 0
    }

    /// `nextLong` for sound seeds.
    #[inline]
    pub fn next_i64(&mut self) -> i64 {
        self.next_u64() as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_int_is_in_range_and_covers_it() {
        let mut r = Rng::new(42);
        let mut seen = [0u32; 7];
        for _ in 0..70_000 {
            let v = r.next_int(7);
            assert!((0..7).contains(&v));
            seen[v as usize] += 1;
        }
        for s in seen {
            assert!((9_000..11_000).contains(&s), "{seen:?}");
        }
        assert_eq!(r.next_int(0), 0);
        assert_eq!(r.next_int(1), 0);
    }

    #[test]
    fn floats_are_unit() {
        let mut r = Rng::new(7);
        for _ in 0..10_000 {
            let f = r.next_float();
            assert!((0.0..1.0).contains(&f));
            let d = r.next_double();
            assert!((0.0..1.0).contains(&d));
        }
    }
}
