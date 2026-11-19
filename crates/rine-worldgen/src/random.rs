//! Port of Minecraft 26.3 random sources:
//! `Xoroshiro128PlusPlus`, `XoroshiroRandomSource` (+ positional factory),
//! `LegacyRandomSource` (+ positional factory), `BitRandomSource`, `MarsagliaPolarGaussian`,
//! `RandomSupport`, `RandomSource`.
//!
//! Java semantics kept on purpose: wrapping i64 math, `>>>` as a u64 shift,
//! `(int)` casts that truncate, `long * float` promotion, and the exact MD5 seed layout.

use md5::{Digest, Md5};

use crate::mth;

const GOLDEN_RATIO_64: i64 = -7_046_029_254_386_353_131;
const SILVER_RATIO_64: i64 = 7_640_891_576_956_012_809;
const STAFFORD_M1: i64 = -4_658_895_280_553_007_687;
const STAFFORD_M2: i64 = -7_723_592_293_110_705_685;

/// `XoroshiroRandomSource.FLOAT_UNIT` (a `float` constant, used in float arithmetic).
const FLOAT_UNIT: f32 = 5.960_464_5E-8;
/// `XoroshiroRandomSource.DOUBLE_UNIT` / `BitRandomSource.DOUBLE_MULTIPLIER` (2^-53, a `double` in bytecode).
const DOUBLE_UNIT: f64 = 1.110_223_024_625_156_5E-16;

const LEGACY_MULTIPLIER: u64 = 25_214_903_917;
const LEGACY_INCREMENT: u64 = 11;
const LEGACY_MASK: u64 = (1u64 << 48) - 1;

/// Common surface of `net.minecraft.util.RandomSource` as used by worldgen.
pub trait Rng {
    /// `nextInt()`.
    fn next_int(&mut self) -> i32;
    /// `nextInt(bound)`. Panics on `bound <= 0`, like Java's `IllegalArgumentException`.
    fn next_int_bound(&mut self, bound: i32) -> i32;
    /// `nextLong()`.
    fn next_long(&mut self) -> i64;
    /// `nextFloat()`.
    fn next_float(&mut self) -> f32;
    /// `nextDouble()`.
    fn next_double(&mut self) -> f64;
    /// `nextBoolean()`.
    fn next_bool(&mut self) -> bool;
    /// `nextGaussian()` (Marsaglia polar method, with its cached second value).
    fn next_gaussian(&mut self) -> f64;
    /// `consumeCount(n)`.
    fn consume_count(&mut self, n: i32);
}

/// Cached second value of `MarsagliaPolarGaussian`. Kept inside each source; it is reset on `set_seed`.
#[derive(Clone, Copy, Debug, Default)]
struct GaussState {
    next: f64,
    has_next: bool,
}

impl GaussState {
    fn reset(&mut self) {
        self.has_next = false;
    }
}

/// `MarsagliaPolarGaussian.nextGaussian()`, driven by `src.next_double()`.
fn marsaglia_next_gaussian<R: Rng + ?Sized>(src: &mut R, g: &mut GaussState) -> f64 {
    if g.has_next {
        g.has_next = false;
        return g.next;
    }

    let mut x;
    let mut y;
    let mut radius_squared;
    loop {
        x = 2.0 * src.next_double() - 1.0;
        y = 2.0 * src.next_double() - 1.0;
        radius_squared = mth::square(x) + mth::square(y);
        if !(radius_squared >= 1.0 || radius_squared == 0.0) {
            break;
        }
    }

    let multiplier = (-2.0 * radius_squared.ln() / radius_squared).sqrt();
    g.next = y * multiplier;
    g.has_next = true;
    x * multiplier
}

/// `RandomSupport.mixStafford13`.
#[inline]
pub fn mix_stafford13(z: i64) -> i64 {
    let mut z = z as u64;
    z = (z ^ (z >> 30)).wrapping_mul(STAFFORD_M1 as u64);
    z = (z ^ (z >> 27)).wrapping_mul(STAFFORD_M2 as u64);
    (z ^ (z >> 31)) as i64
}

/// `RandomSupport.upgradeSeedTo128bit(legacySeed)`: returns `(seedLo, seedHi)`, mixed.
pub fn upgrade_seed_to_128bit(legacy_seed: i64) -> (i64, i64) {
    let low = legacy_seed ^ SILVER_RATIO_64;
    let high = low.wrapping_add(GOLDEN_RATIO_64);
    (mix_stafford13(low), mix_stafford13(high))
}

/// `RandomSupport.seedFromHashOf(name)`: MD5 of the UTF-8 bytes, read as two big-endian longs.
pub fn seed_from_hash_of(name: &str) -> (i64, i64) {
    let digest = Md5::digest(name.as_bytes());
    let lo = u64::from_be_bytes(digest[0..8].try_into().expect("md5 is 16 bytes"));
    let hi = u64::from_be_bytes(digest[8..16].try_into().expect("md5 is 16 bytes"));
    (lo as i64, hi as i64)
}

/// `String.hashCode()`: UTF-16 code units, wrapping i32.
pub fn java_string_hash(s: &str) -> i32 {
    let mut h: i32 = 0;
    for unit in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(unit as i32);
    }
    h
}

/// `Xoroshiro128PlusPlus`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Xoro128pp {
    lo: u64,
    hi: u64,
}

impl Xoro128pp {
    fn new(lo: i64, hi: i64) -> Self {
        let (mut lo, mut hi) = (lo as u64, hi as u64);
        if lo | hi == 0 {
            lo = GOLDEN_RATIO_64 as u64;
            hi = SILVER_RATIO_64 as u64;
        }
        Self { lo, hi }
    }

    #[inline]
    fn next_u64(&mut self) -> u64 {
        let s0 = self.lo;
        let mut s1 = self.hi;
        let result = s0.wrapping_add(s1).rotate_left(17).wrapping_add(s0);
        s1 ^= s0;
        self.lo = s0.rotate_left(49) ^ s1 ^ (s1 << 21);
        self.hi = s1.rotate_left(28);
        result
    }
}

/// `XoroshiroRandomSource`.
#[derive(Clone, Debug)]
pub struct Xoroshiro {
    gen: Xoro128pp,
    gauss: GaussState,
}

impl Xoroshiro {
    /// `new XoroshiroRandomSource(long seed)`: upgrades the seed to 128 bits with the Stafford mix.
    pub fn new(seed: i64) -> Self {
        let (lo, hi) = upgrade_seed_to_128bit(seed);
        Self::from_longs(lo, hi)
    }

    /// `new XoroshiroRandomSource(long seedLo, long seedHi)`: raw state, no mixing.
    pub fn from_longs(seed_lo: i64, seed_hi: i64) -> Self {
        Self {
            gen: Xoro128pp::new(seed_lo, seed_hi),
            gauss: GaussState::default(),
        }
    }

    pub fn raw_state(&self) -> (u64, u64) {
        (self.gen.lo, self.gen.hi)
    }

    /// `setSeed(long)`.
    pub fn set_seed(&mut self, seed: i64) {
        *self = Self::new(seed);
    }

    /// `forkPositional()`: consumes two longs (seedLo first).
    pub fn fork_positional(&mut self) -> Positional {
        let lo = self.next_long();
        let hi = self.next_long();
        Positional::Xoro { lo, hi }
    }

    /// `fork()`: consumes two longs (first is the new seedLo).
    pub fn fork(&mut self) -> Xoroshiro {
        let lo = self.next_long();
        let hi = self.next_long();
        Xoroshiro::from_longs(lo, hi)
    }

    #[inline]
    fn next_bits(&mut self, bits: u32) -> u64 {
        self.gen.next_u64() >> (64 - bits)
    }
}

impl Rng for Xoroshiro {
    #[inline]
    fn next_int(&mut self) -> i32 {
        self.next_long() as i32
    }

    fn next_int_bound(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "Bound must be positive");
        let bound_u = bound as u64;
        let mut random_bits = self.next_int() as u32 as u64;
        let mut multiplied = random_bits * bound_u;
        let mut fractional = multiplied & 0xFFFF_FFFF;
        if fractional < bound_u {
            // Integer.remainderUnsigned(~bound + 1, bound)
            let start = (!(bound as u32)).wrapping_add(1) % (bound as u32);
            while fractional < start as u64 {
                random_bits = self.next_int() as u32 as u64;
                multiplied = random_bits * bound_u;
                fractional = multiplied & 0xFFFF_FFFF;
            }
        }
        (multiplied >> 32) as i32
    }

    #[inline]
    fn next_long(&mut self) -> i64 {
        self.gen.next_u64() as i64
    }

    #[inline]
    fn next_float(&mut self) -> f32 {
        // (float)nextBits(24) * FLOAT_UNIT, float arithmetic
        (self.next_bits(24) as f32) * FLOAT_UNIT
    }

    #[inline]
    fn next_double(&mut self) -> f64 {
        // (double)nextBits(53) * 2^-53, double arithmetic (verified against bytecode)
        (self.next_bits(53) as f64) * DOUBLE_UNIT
    }

    #[inline]
    fn next_bool(&mut self) -> bool {
        (self.next_long() & 1) != 0
    }

    fn next_gaussian(&mut self) -> f64 {
        let mut g = self.gauss;
        let value = marsaglia_next_gaussian(self, &mut g);
        self.gauss = g;
        value
    }

    fn consume_count(&mut self, n: i32) {
        for _ in 0..n {
            self.next_long();
        }
    }
}

/// `LegacyRandomSource` (java.util.Random LCG, plus `BitRandomSource` defaults).
#[derive(Clone, Debug)]
pub struct Legacy {
    seed: u64,
    gauss: GaussState,
}

impl Legacy {
    /// `new LegacyRandomSource(long seed)` / `setSeed`.
    pub fn new(seed: i64) -> Self {
        Self {
            seed: ((seed as u64) ^ LEGACY_MULTIPLIER) & LEGACY_MASK,
            gauss: GaussState::default(),
        }
    }

    /// `setSeed(long)`.
    pub fn set_seed(&mut self, seed: i64) {
        self.seed = ((seed as u64) ^ LEGACY_MULTIPLIER) & LEGACY_MASK;
        self.gauss.reset();
    }

    /// `next(bits)`: `(int)(newSeed >> (48 - bits))`.
    #[inline]
    fn next_bits(&mut self, bits: u32) -> i32 {
        let new_seed = self
            .seed
            .wrapping_mul(LEGACY_MULTIPLIER)
            .wrapping_add(LEGACY_INCREMENT)
            & LEGACY_MASK;
        self.seed = new_seed;
        (new_seed >> (48 - bits)) as i32
    }

    /// `forkPositional()`: consumes one long.
    pub fn fork_positional(&mut self) -> Positional {
        Positional::Legacy {
            seed: self.next_long(),
        }
    }

    /// `fork()`: `new LegacyRandomSource(nextLong())`.
    pub fn fork(&mut self) -> Legacy {
        Legacy::new(self.next_long())
    }
}

impl Rng for Legacy {
    #[inline]
    fn next_int(&mut self) -> i32 {
        self.next_bits(32)
    }

    /// `BitRandomSource.nextInt(bound)`.
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "Bound must be positive");
        if bound & (bound - 1) == 0 {
            return (((bound as i64) * (self.next_bits(31) as i64)) >> 31) as i32;
        }
        loop {
            let sample = self.next_bits(31);
            let modulo = sample % bound;
            if sample.wrapping_sub(modulo).wrapping_add(bound - 1) >= 0 {
                return modulo;
            }
        }
    }

    /// `BitRandomSource.nextLong()`: `((long)upper << 32) + lower`, lower sign-extended.
    fn next_long(&mut self) -> i64 {
        let upper = self.next_bits(32);
        let lower = self.next_bits(32);
        ((upper as i64) << 32).wrapping_add(lower as i64)
    }

    /// `BitRandomSource.nextFloat()`: `next(24) * FLOAT_MULTIPLIER`.
    fn next_float(&mut self) -> f32 {
        (self.next_bits(24) as f32) * FLOAT_UNIT
    }

    /// `BitRandomSource.nextDouble()`: `((long)next(26) << 27) + next(27)` times 2^-53.
    fn next_double(&mut self) -> f64 {
        let upper = self.next_bits(26);
        let lower = self.next_bits(27);
        let combined = ((upper as i64) << 27).wrapping_add(lower as i64);
        (combined as f64) * DOUBLE_UNIT
    }

    /// `BitRandomSource.nextBoolean()`: `next(1) != 0`.
    fn next_bool(&mut self) -> bool {
        self.next_bits(1) != 0
    }

    fn next_gaussian(&mut self) -> f64 {
        let mut g = self.gauss;
        let value = marsaglia_next_gaussian(self, &mut g);
        self.gauss = g;
        value
    }

    /// `RandomSource.consumeCount` default: calls `nextInt()` n times for the legacy source.
    fn consume_count(&mut self, n: i32) {
        for _ in 0..n {
            self.next_int();
        }
    }
}

/// `PositionalRandomFactory` for both algorithms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Positional {
    /// `XoroshiroPositionalRandomFactory(seedLo, seedHi)`.
    Xoro { lo: i64, hi: i64 },
    /// `LegacyPositionalRandomFactory(seed)`.
    Legacy { seed: i64 },
}

impl Positional {
    /// `at(x, y, z)`.
    pub fn at(&self, x: i32, y: i32, z: i32) -> AnyRng {
        let positional = mth::get_seed(x, y, z);
        match *self {
            Positional::Xoro { lo, hi } => AnyRng::Xoro(Xoroshiro::from_longs(positional ^ lo, hi)),
            Positional::Legacy { seed } => AnyRng::Legacy(Legacy::new(positional ^ seed)),
        }
    }

    /// `fromHashOf(String name)`.
    pub fn from_hash_of(&self, name: &str) -> AnyRng {
        match *self {
            Positional::Xoro { lo, hi } => {
                let (seed_lo, seed_hi) = seed_from_hash_of(name);
                // Seed128bit.xor(lo, hi) then the raw constructor: no mixing.
                AnyRng::Xoro(Xoroshiro::from_longs(seed_lo ^ lo, seed_hi ^ hi))
            }
            Positional::Legacy { seed } => {
                let positional = java_string_hash(name) as i64;
                AnyRng::Legacy(Legacy::new(positional ^ seed))
            }
        }
    }

    /// `fromSeed(long seed)`.
    pub fn from_seed(&self, seed: i64) -> AnyRng {
        match *self {
            Positional::Xoro { lo, hi } => {
                AnyRng::Xoro(Xoroshiro::from_longs(seed ^ lo, seed ^ hi))
            }
            Positional::Legacy { .. } => AnyRng::Legacy(Legacy::new(seed)),
        }
    }
}

/// Either algorithm behind one type, used wherever Java passes a `RandomSource`.
#[derive(Clone, Debug)]
pub enum AnyRng {
    Xoro(Xoroshiro),
    Legacy(Legacy),
}

impl AnyRng {
    /// `new` for the chosen algorithm (`WorldgenRandom.Algorithm.newInstance`).
    pub fn new_xoroshiro(seed: i64) -> Self {
        AnyRng::Xoro(Xoroshiro::new(seed))
    }

    pub fn new_legacy(seed: i64) -> Self {
        AnyRng::Legacy(Legacy::new(seed))
    }

    /// `forkPositional()`.
    pub fn fork_positional(&mut self) -> Positional {
        match self {
            AnyRng::Xoro(r) => r.fork_positional(),
            AnyRng::Legacy(r) => r.fork_positional(),
        }
    }

    /// `fork()`.
    pub fn fork(&mut self) -> AnyRng {
        match self {
            AnyRng::Xoro(r) => AnyRng::Xoro(r.fork()),
            AnyRng::Legacy(r) => AnyRng::Legacy(r.fork()),
        }
    }
}

impl Rng for AnyRng {
    fn next_int(&mut self) -> i32 {
        match self {
            AnyRng::Xoro(r) => r.next_int(),
            AnyRng::Legacy(r) => r.next_int(),
        }
    }

    fn next_int_bound(&mut self, bound: i32) -> i32 {
        match self {
            AnyRng::Xoro(r) => r.next_int_bound(bound),
            AnyRng::Legacy(r) => r.next_int_bound(bound),
        }
    }

    fn next_long(&mut self) -> i64 {
        match self {
            AnyRng::Xoro(r) => r.next_long(),
            AnyRng::Legacy(r) => r.next_long(),
        }
    }

    fn next_float(&mut self) -> f32 {
        match self {
            AnyRng::Xoro(r) => r.next_float(),
            AnyRng::Legacy(r) => r.next_float(),
        }
    }

    fn next_double(&mut self) -> f64 {
        match self {
            AnyRng::Xoro(r) => r.next_double(),
            AnyRng::Legacy(r) => r.next_double(),
        }
    }

    fn next_bool(&mut self) -> bool {
        match self {
            AnyRng::Xoro(r) => r.next_bool(),
            AnyRng::Legacy(r) => r.next_bool(),
        }
    }

    fn next_gaussian(&mut self) -> f64 {
        match self {
            AnyRng::Xoro(r) => r.next_gaussian(),
            AnyRng::Legacy(r) => r.next_gaussian(),
        }
    }

    fn consume_count(&mut self, n: i32) {
        match self {
            AnyRng::Xoro(r) => r.consume_count(n),
            AnyRng::Legacy(r) => r.consume_count(n),
        }
    }
}
