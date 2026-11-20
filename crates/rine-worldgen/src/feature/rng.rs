//! `WorldgenRandom` as used by `ChunkGenerator.addVanillaDecorations`: a `LegacyRandomSource` (which
//! is a `BitRandomSource`) whose `next(bits)` is `(int)(xoroshiro.nextLong() >>> (64 - bits))`.
//! `nextInt`, `nextLong`, `nextFloat` and `nextDouble` are the `BitRandomSource` defaults on top of it.

use crate::random::{Rng, Xoroshiro};

/// `BitRandomSource.FLOAT_MULTIPLIER` (a float literal).
const FLOAT_MULTIPLIER: f32 = 5.9604645E-8_f32;
/// `BitRandomSource.DOUBLE_MULTIPLIER`: declared as `1.110223E-16F`, so it is a float value.
const DOUBLE_MULTIPLIER: f32 = 1.110223E-16_f32;

pub struct WorldgenRng {
    src: Xoroshiro,
    direct: bool,
    gaussian: Option<f64>,
}

impl WorldgenRng {
    /// `new WorldgenRandom(new XoroshiroRandomSource(seed))`.
    pub fn new(seed: i64) -> Self {
        WorldgenRng { src: Xoroshiro::new(seed), direct: false, gaussian: None }
    }

    pub fn direct(src: Xoroshiro) -> Self {
        Self { src, direct: true, gaussian: None }
    }

    pub fn source(self) -> Xoroshiro {
        self.src
    }

    /// `WorldgenRandom.setSeed`: forwards to the wrapped source.
    pub fn set_seed(&mut self, seed: i64) {
        self.src.set_seed(seed);
    }

    /// `WorldgenRandom.next(bits)`.
    #[inline]
    pub fn next(&mut self, bits: u32) -> i32 {
        ((self.src.next_long() as u64) >> (64 - bits)) as i32
    }

    /// `nextInt()`.
    #[inline]
    pub fn next_int(&mut self) -> i32 {
        if self.direct { self.src.next_int() } else { self.next(32) }
    }

    /// `nextInt(bound)` (`BitRandomSource`).
    pub fn next_int_bound(&mut self, bound: i32) -> i32 {
        if self.direct {
            return self.src.next_int_bound(bound);
        }
        assert!(bound > 0, "Bound must be positive");
        if bound & (bound - 1) == 0 {
            return ((bound as i64 * self.next(31) as i64) >> 31) as i32;
        }
        loop {
            let sample = self.next(31);
            let modulo = sample % bound;
            if sample.wrapping_sub(modulo).wrapping_add(bound - 1) >= 0 {
                return modulo;
            }
        }
    }

    /// `nextIntBetweenInclusive(min, max)`.
    #[inline]
    pub fn next_int_between(&mut self, min: i32, max_inclusive: i32) -> i32 {
        self.next_int_bound(max_inclusive - min + 1) + min
    }

    /// `nextLong()`.
    pub fn next_long(&mut self) -> i64 {
        if self.direct {
            return self.src.next_long();
        }
        let upper = self.next(32);
        let lower = self.next(32);
        ((upper as i64) << 32).wrapping_add(lower as i64)
    }

    /// `nextBoolean()`.
    pub fn next_bool(&mut self) -> bool {
        if self.direct { self.src.next_long() & 1 != 0 } else { self.next(1) != 0 }
    }

    /// `nextFloat()`: `next(24) * FLOAT_MULTIPLIER` in float arithmetic.
    #[inline]
    pub fn next_float(&mut self) -> f32 {
        self.next(24) as f32 * FLOAT_MULTIPLIER
    }

    /// `nextDouble()`: `(long)combined * DOUBLE_MULTIPLIER`, the product is a float widened to double.
    pub fn next_double(&mut self) -> f64 {
        if self.direct {
            return self.src.next_double();
        }
        let upper = self.next(26);
        let lower = self.next(27);
        let combined = ((upper as i64) << 27) + lower as i64;
        ((combined as f32) * DOUBLE_MULTIPLIER) as f64
    }

    pub fn next_gaussian(&mut self) -> f64 {
        if self.direct {
            return self.src.next_gaussian();
        }
        // WorldgenRandom reseeds the wrapped source, leaving the legacy cache intact.
        if let Some(value) = self.gaussian.take() {
            return value;
        }
        loop {
            let x = 2.0 * self.next_double() - 1.0;
            let y = 2.0 * self.next_double() - 1.0;
            let squared = x * x + y * y;
            if squared > 0.0 && squared < 1.0 {
                let scale = (-2.0 * squared.ln() / squared).sqrt();
                self.gaussian = Some(y * scale);
                return x * scale;
            }
        }
    }

    /// `WorldgenRandom.setDecorationSeed(seed, blockX, blockZ)`. Takes block coordinates (the chunk origin).
    pub fn set_decoration_seed(&mut self, seed: i64, block_x: i32, block_z: i32) -> i64 {
        self.set_seed(seed);
        let x_scale = self.next_long() | 1;
        let z_scale = self.next_long() | 1;
        let result = (block_x as i64)
            .wrapping_mul(x_scale)
            .wrapping_add((block_z as i64).wrapping_mul(z_scale))
            ^ seed;
        self.set_seed(result);
        result
    }

    /// `WorldgenRandom.setFeatureSeed(seed, index, step)`.
    pub fn set_feature_seed(&mut self, seed: i64, index: i32, step: i32) {
        let result = seed.wrapping_add(index as i64).wrapping_add((10000 * step) as i64);
        self.set_seed(result);
    }
}
