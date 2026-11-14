//! Own hashing RNG and gradient noise (Perlin 2D/3D with fBm). No dependencies.

pub const SALT_BIOME: u64 = 0x0B10;
pub const SALT_SURF: u64 = 0x5F00;
pub const SALT_BED: u64 = 0xBED0;
pub const SALT_DEEP: u64 = 0xDEE9;
pub const SALT_TREE: u64 = 0x7EE0;
pub const SALT_DECOR: u64 = 0xDEC0;
pub const SALT_BLOB: u64 = 0xB10B;
pub const SALT_ORE: u64 = 0x0E0E;

const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

#[inline]
fn finalize(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// splitmix64 finalizer applied to `x + golden`.
#[inline]
pub fn mix(x: u64) -> u64 {
    finalize(x.wrapping_add(GOLDEN))
}

/// Hash of a seed, three integer coordinates and a salt.
#[inline]
pub fn hash(seed: u64, a: i64, b: i64, c: i64, salt: u64) -> u64 {
    let h = mix(seed ^ salt.wrapping_mul(0xD6E8_FEB8_6659_FD93));
    let h = mix(h ^ a as u64);
    let h = mix(h ^ (b as u64).rotate_left(23));
    mix(h ^ (c as u64).rotate_left(41))
}

/// Hash value mapped to [0, 1).
#[inline]
pub fn unit(h: u64) -> f32 {
    (h >> 40) as f32 * (1.0 / 16_777_216.0)
}

/// Small splitmix-style sequence RNG, seeded from a hash.
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    #[inline]
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(GOLDEN);
        finalize(self.0)
    }

    #[inline]
    pub fn next_f32(&mut self) -> f32 {
        unit(self.next_u64())
    }

    /// Uniform in [0, n). `n` must be > 0.
    #[inline]
    pub fn below(&mut self, n: u32) -> u32 {
        (((self.next_u64() >> 32) * n as u64) >> 32) as u32
    }

    /// Uniform in [lo, hi). `hi` must be > `lo`.
    #[inline]
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + self.below((hi - lo) as u32) as i32
    }

    #[inline]
    pub fn chance(&mut self, p: f32) -> bool {
        self.next_f32() < p
    }
}

#[inline]
pub fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

const GRAD2: [(f32, f32); 8] = [
    (1.0, 0.0),
    (-1.0, 0.0),
    (0.0, 1.0),
    (0.0, -1.0),
    (0.707, 0.707),
    (-0.707, 0.707),
    (0.707, -0.707),
    (-0.707, -0.707),
];

const GRAD3: [(f32, f32, f32); 12] = [
    (1.0, 1.0, 0.0),
    (-1.0, 1.0, 0.0),
    (1.0, -1.0, 0.0),
    (-1.0, -1.0, 0.0),
    (1.0, 0.0, 1.0),
    (-1.0, 0.0, 1.0),
    (1.0, 0.0, -1.0),
    (-1.0, 0.0, -1.0),
    (0.0, 1.0, 1.0),
    (0.0, -1.0, 1.0),
    (0.0, 1.0, -1.0),
    (0.0, -1.0, -1.0),
];

/// Gradient noise, hash-based lattice (no permutation table).
#[derive(Clone, Copy)]
pub struct Perlin {
    seed: u64,
}

impl Perlin {
    #[inline]
    pub fn new(seed: u64) -> Self {
        Self { seed }
    }

    /// Roughly in [-1, 1].
    pub fn d2(&self, x: f32, y: f32) -> f32 {
        let (x0, y0) = (x.floor(), y.floor());
        let (ix, iy) = (x0 as i64, y0 as i64);
        let (fx, fy) = (x - x0, y - y0);
        let dot = |dx: i64, dy: i64| -> f32 {
            let g = GRAD2[(hash(self.seed, ix + dx, iy + dy, 0, 2) & 7) as usize];
            g.0 * (fx - dx as f32) + g.1 * (fy - dy as f32)
        };
        let (u, v) = (fade(fx), fade(fy));
        lerp(lerp(dot(0, 0), dot(1, 0), u), lerp(dot(0, 1), dot(1, 1), u), v) * 1.4
    }

    /// Roughly in [-1, 1].
    pub fn d3(&self, x: f32, y: f32, z: f32) -> f32 {
        let (x0, y0, z0) = (x.floor(), y.floor(), z.floor());
        let (ix, iy, iz) = (x0 as i64, y0 as i64, z0 as i64);
        let (fx, fy, fz) = (x - x0, y - y0, z - z0);
        let dot = |dx: i64, dy: i64, dz: i64| -> f32 {
            let g = GRAD3[(hash(self.seed, ix + dx, iy + dy, iz + dz, 3) % 12) as usize];
            g.0 * (fx - dx as f32) + g.1 * (fy - dy as f32) + g.2 * (fz - dz as f32)
        };
        let (u, v, w) = (fade(fx), fade(fy), fade(fz));
        let x00 = lerp(dot(0, 0, 0), dot(1, 0, 0), u);
        let x10 = lerp(dot(0, 1, 0), dot(1, 1, 0), u);
        let x01 = lerp(dot(0, 0, 1), dot(1, 0, 1), u);
        let x11 = lerp(dot(0, 1, 1), dot(1, 1, 1), u);
        lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)
    }
}

fn octave_seed(seed: u64, i: u32) -> u64 {
    mix(seed ^ (i as u64 + 1).wrapping_mul(GOLDEN))
}

fn octave_norm(oct: u32) -> f32 {
    (0..oct).map(|i| 0.25f32.powi(i as i32)).sum::<f32>().sqrt()
}

/// 2D fBm, output roughly in [-1, 1].
#[derive(Clone, Copy)]
pub struct Fbm2 {
    seed: u64,
    freq: f32,
    oct: u32,
    norm: f32,
}

impl Fbm2 {
    pub fn new(seed: u64, freq: f32, oct: u32) -> Self {
        Self { seed, freq, oct, norm: octave_norm(oct) }
    }

    pub fn get(&self, x: f32, z: f32) -> f32 {
        let (mut sum, mut amp, mut f) = (0.0f32, 1.0f32, self.freq);
        for i in 0..self.oct {
            sum += amp * Perlin::new(octave_seed(self.seed, i)).d2(x * f, z * f);
            amp *= 0.5;
            f *= 2.0;
        }
        sum / self.norm
    }
}

/// 3D fBm, output roughly in [-1, 1].
#[derive(Clone, Copy)]
pub struct Fbm3 {
    seed: u64,
    freq: f32,
    oct: u32,
    norm: f32,
}

impl Fbm3 {
    pub fn new(seed: u64, freq: f32, oct: u32) -> Self {
        Self { seed, freq, oct, norm: octave_norm(oct) }
    }

    pub fn get(&self, x: f32, y: f32, z: f32) -> f32 {
        let (mut sum, mut amp, mut f) = (0.0f32, 1.0f32, self.freq);
        for i in 0..self.oct {
            sum += amp * Perlin::new(octave_seed(self.seed, i)).d3(x * f, y * f, z * f);
            amp *= 0.5;
            f *= 2.0;
        }
        sum / self.norm
    }
}
