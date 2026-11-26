//! Port of `world.level.levelgen.synth`: gradient base, PerlinNoise, SmearedPerlinNoise,
//! SimplexNoise, NoiseStack, NormalNoise (parity builder + create), BlendedNoise.
//!
//! Vanilla code paths are ported (not Paper's moonrise/NoiseGradients variants); the golden
//! harness runs against the vanilla server jar.

use crate::mth::{self, clamp_f32, floor, lerp, lerp3, smoothstep};
use crate::random::{AnyRng, Positional, Rng};

/// `GradientNoise.GRADIENT`.
const GRADIENT: [(i32, i32, i32); 16] = [
    (1, 1, 0),
    (-1, 1, 0),
    (1, -1, 0),
    (-1, -1, 0),
    (1, 0, 1),
    (-1, 0, 1),
    (1, 0, -1),
    (-1, 0, -1),
    (0, 1, 1),
    (0, -1, 1),
    (0, 1, -1),
    (0, -1, -1),
    (1, 1, 0),
    (0, -1, 1),
    (-1, 1, 0),
    (0, -1, -1),
];

/// `Math.nextDown(1.6777216E7)`.
const HALF_ROUND_OFF: f64 = 16_777_215.999_999_998;
/// `GradientNoise.wrap` uses `3.3554432E7`.
const WRAP_PERIOD: f64 = 33_554_432.0;
/// `SmearedPerlinNoise` epsilon `1.0E-7F`, widened to double as in bytecode.
const FUDGE_EPSILON: f64 = 1.0E-7_f32 as f64;
/// `NormalNoise.INPUT_FACTOR`.
const NORMAL_INPUT_FACTOR: f64 = 1.018_126_888_217_522_7;
/// `BlendedNoise.LIMIT_FACTOR = 0.99998474F`, widened to double.
const BLENDED_LIMIT_FACTOR: f64 = 0.99998474_f32 as f64;
const BLENDED_MAIN_FACTOR: f64 = 12.75;
const BLENDED_BASE_SCALE: f64 = 684.412;

/// `GradientNoise.wrap(double)`.
#[inline]
pub fn wrap(x: f64) -> f64 {
    if x >= -HALF_ROUND_OFF && x < HALF_ROUND_OFF {
        x
    } else {
        x - (x / WRAP_PERIOD + 0.5).floor() * WRAP_PERIOD
    }
}

/// `GradientNoise.Gradient.dot(float, float, float)`: `gx*x + gy*y + gz*z` in float, left to right.
#[inline]
fn grad_dot(hash: i32, x: f32, y: f32, z: f32) -> f32 {
    let (gx, gy, gz) = GRADIENT[(hash & 15) as usize];
    (gx as f32) * x + (gy as f32) * y + (gz as f32) * z
}

/// Permutation table and random offsets shared by the gradient-noise family
/// (`GradientNoise(RandomSource, double noiseOffsetScale)`).
#[derive(Clone, Debug)]
pub struct GradientBase {
    offset_x: f64,
    offset_y: f64,
    offset_z: f64,
    perms: [u8; 256],
}

impl GradientBase {
    pub fn new<R: Rng>(rng: &mut R, noise_offset_scale: f64) -> Self {
        let offset_x = rng.next_double() * noise_offset_scale;
        let offset_y = rng.next_double() * noise_offset_scale;
        let offset_z = rng.next_double() * noise_offset_scale;
        let mut perms = [0u8; 256];
        for (i, p) in perms.iter_mut().enumerate() {
            *p = i as u8;
        }
        for i in 0..256usize {
            let offset = rng.next_int_bound(256 - i as i32) as usize;
            perms.swap(i, offset + i);
        }
        Self {
            offset_x,
            offset_y,
            offset_z,
            perms,
        }
    }

    #[inline]
    fn permute(&self, x: i32) -> i32 {
        (self.perms[(x & 0xFF) as usize] & 0xFF) as i32
    }
}

/// `PerlinNoise` (single octave, vanilla `sampleAndLerpVanilla` path).
#[derive(Clone, Debug)]
pub struct PerlinNoise {
    base: GradientBase,
}

impl PerlinNoise {
    /// `new PerlinNoise(RandomSource)`.
    pub fn new<R: Rng>(rng: &mut R) -> Self {
        Self {
            base: GradientBase::new(rng, 256.0),
        }
    }

    /// `PerlinNoise.get(x, y)`: `get(wrap(x), 0.0, wrap(y))`.
    pub fn get_value_2d(&self, x: f64, y: f64) -> f32 {
        self.get_value(wrap(x), 0.0, wrap(y))
    }

    /// `PerlinNoise.get(x, y, z)`.
    pub fn get_value(&self, x: f64, y: f64, z: f64) -> f32 {
        let x = wrap(x) + self.base.offset_x;
        let y = wrap(y) + self.base.offset_y;
        let z = wrap(z) + self.base.offset_z;
        let fx = floor(x);
        let fy = floor(y);
        let fz = floor(z);
        let rx = (x - fx as f64) as f32;
        let ry = (y - fy as f64) as f32;
        let rz = (z - fz as f64) as f32;
        sample_and_lerp(&self.base, fx, fy, fz, rx, ry, rz, ry)
    }
}

/// `PerlinNoise.sampleAndLerpVanilla`. `original_ry` only drives the Y smoothstep alpha.
#[allow(clippy::too_many_arguments)]
fn sample_and_lerp(
    base: &GradientBase,
    x: i32,
    y: i32,
    z: i32,
    rx: f32,
    ry: f32,
    rz: f32,
    original_ry: f32,
) -> f32 {
    let p = |v: i32| base.permute(v);
    let x0 = p(x);
    let x1 = p(x.wrapping_add(1));
    let xy00 = p(x0.wrapping_add(y));
    let xy01 = p(x0.wrapping_add(y).wrapping_add(1));
    let xy10 = p(x1.wrapping_add(y));
    let xy11 = p(x1.wrapping_add(y).wrapping_add(1));

    let d000 = grad_dot(p(xy00.wrapping_add(z)), rx, ry, rz);
    let d100 = grad_dot(p(xy10.wrapping_add(z)), rx - 1.0, ry, rz);
    let d010 = grad_dot(p(xy01.wrapping_add(z)), rx, ry - 1.0, rz);
    let d110 = grad_dot(p(xy11.wrapping_add(z)), rx - 1.0, ry - 1.0, rz);
    let d001 = grad_dot(p(xy00.wrapping_add(z).wrapping_add(1)), rx, ry, rz - 1.0);
    let d101 = grad_dot(p(xy10.wrapping_add(z).wrapping_add(1)), rx - 1.0, ry, rz - 1.0);
    let d011 = grad_dot(p(xy01.wrapping_add(z).wrapping_add(1)), rx, ry - 1.0, rz - 1.0);
    let d111 = grad_dot(
        p(xy11.wrapping_add(z).wrapping_add(1)),
        rx - 1.0,
        ry - 1.0,
        rz - 1.0,
    );

    let xa = smoothstep(rx);
    let ya = smoothstep(original_ry);
    let za = smoothstep(rz);
    lerp3(xa, ya, za, d000, d100, d010, d110, d001, d101, d011, d111)
}

/// `SmearedPerlinNoise` (deprecated in Java, used by BlendedNoise).
#[derive(Clone, Debug)]
pub struct SmearedPerlinNoise {
    base: GradientBase,
    fudge_y_scale: f64,
}

impl SmearedPerlinNoise {
    pub fn new<R: Rng>(rng: &mut R, fudge_y_scale: f64) -> Self {
        Self {
            base: GradientBase::new(rng, 256.0),
            fudge_y_scale,
        }
    }

    fn compute_fudge_y(&self, original_y: f64, relative_y: f64) -> f64 {
        let fudge_limit = if original_y >= 0.0 && original_y < relative_y {
            original_y
        } else {
            relative_y
        };
        floor(fudge_limit / self.fudge_y_scale + FUDGE_EPSILON) as f64 * self.fudge_y_scale
    }

    pub fn get_value(&self, orig_x: f64, orig_y: f64, orig_z: f64) -> f32 {
        let x = wrap(orig_x) + self.base.offset_x;
        let y = wrap(orig_y) + self.base.offset_y;
        let z = wrap(orig_z) + self.base.offset_z;
        let fx = floor(x);
        let fy = floor(y);
        let fz = floor(z);
        let rx = (x - fx as f64) as f32;
        let rel_y = y - fy as f64;
        let rz = (z - fz as f64) as f32;
        let fudged_ry = (rel_y - self.compute_fudge_y(orig_y, rel_y)) as f32;
        sample_and_lerp(&self.base, fx, fy, fz, rx, fudged_ry, rz, rel_y as f32)
    }
}

/// `SimplexNoise`.
#[derive(Clone, Debug)]
pub struct SimplexNoise {
    base: GradientBase,
}

impl SimplexNoise {
    pub fn new<R: Rng>(rng: &mut R) -> Self {
        Self {
            base: GradientBase::new(rng, 256.0),
        }
    }

    /// `new SimplexNoise(random, true)`: the noise offset is discarded (scale 0.0).
    pub fn new_discard_offset<R: Rng>(rng: &mut R) -> Self {
        Self {
            base: GradientBase::new(rng, 0.0),
        }
    }

    fn corner_noise_3d(index: i32, x: f64, y: f64, z: f64, base: f64) -> f64 {
        let mut t0 = base - x * x - y * y - z * z;
        if t0 < 0.0 {
            0.0
        } else {
            t0 *= t0;
            let (gx, gy, gz) = GRADIENT[index as usize];
            t0 * t0 * (gx as f64 * x + gy as f64 * y + gz as f64 * z)
        }
    }

    pub fn get_value_2d(&self, x_in: f64, y_in: f64) -> f32 {
        let sqrt3 = 3.0_f64.sqrt();
        let f2 = 0.5 * (sqrt3 - 1.0);
        let g2 = (3.0 - sqrt3) / 6.0;

        let xin = x_in + self.base.offset_x;
        let yin = y_in + self.base.offset_y;
        let s = (xin + yin) * f2;
        let i = floor(xin + s);
        let j = floor(yin + s);
        let t = (i.wrapping_add(j)) as f64 * g2;
        let x0_big = i as f64 - t;
        let y0_big = j as f64 - t;
        let x0 = xin - x0_big;
        let y0 = yin - y0_big;

        let (i1, j1) = if x0 > y0 { (1, 0) } else { (0, 1) };

        let x1 = x0 - i1 as f64 + g2;
        let y1 = y0 - j1 as f64 + g2;
        let x2 = x0 - 1.0 + 2.0 * g2;
        let y2 = y0 - 1.0 + 2.0 * g2;
        let ii = i & 0xFF;
        let jj = j & 0xFF;
        let p = |v: i32| self.base.permute(v);
        let gi0 = p(ii + p(jj)) % 12;
        let gi1 = p(ii + i1 + p(jj + j1)) % 12;
        let gi2 = p(ii + 1 + p(jj + 1)) % 12;
        let n0 = Self::corner_noise_3d(gi0, x0, y0, 0.0, 0.5);
        let n1 = Self::corner_noise_3d(gi1, x1, y1, 0.0, 0.5);
        let n2 = Self::corner_noise_3d(gi2, x2, y2, 0.0, 0.5);
        (70.0 * (n0 + n1 + n2)) as f32
    }

    pub fn get_value_3d(&self, x_in: f64, y_in: f64, z_in: f64) -> f32 {
        let f3 = 0.333_333_333_333_333_3;
        let g3 = 0.166_666_666_666_666_66;

        let xin = x_in + self.base.offset_x;
        let yin = y_in + self.base.offset_y;
        let zin = z_in + self.base.offset_z;
        let s = (xin + yin + zin) * f3;
        let i = floor(xin + s);
        let j = floor(yin + s);
        let k = floor(zin + s);
        let t = (i.wrapping_add(j).wrapping_add(k)) as f64 * g3;
        let x0 = xin - (i as f64 - t);
        let y0 = yin - (j as f64 - t);
        let z0 = zin - (k as f64 - t);

        let (i1, j1, k1, i2, j2, k2) = if x0 >= y0 {
            if y0 >= z0 {
                (1, 0, 0, 1, 1, 0)
            } else if x0 >= z0 {
                (1, 0, 0, 1, 0, 1)
            } else {
                (0, 0, 1, 1, 0, 1)
            }
        } else if y0 < z0 {
            (0, 0, 1, 0, 1, 1)
        } else if x0 < z0 {
            (0, 1, 0, 0, 1, 1)
        } else {
            (0, 1, 0, 1, 1, 0)
        };

        let x1 = x0 - i1 as f64 + g3;
        let y1 = y0 - j1 as f64 + g3;
        let z1 = z0 - k1 as f64 + g3;
        let x2 = x0 - i2 as f64 + f3;
        let y2 = y0 - j2 as f64 + f3;
        let z2 = z0 - k2 as f64 + f3;
        let x3 = x0 - 1.0 + 0.5;
        let y3 = y0 - 1.0 + 0.5;
        let z3 = z0 - 1.0 + 0.5;

        let ii = i & 0xFF;
        let jj = j & 0xFF;
        let kk = k & 0xFF;
        let p = |v: i32| self.base.permute(v);
        let gi0 = p(ii + p(jj + p(kk))) % 12;
        let gi1 = p(ii + i1 + p(jj + j1 + p(kk + k1))) % 12;
        let gi2 = p(ii + i2 + p(jj + j2 + p(kk + k2))) % 12;
        let gi3 = p(ii + 1 + p(jj + 1 + p(kk + 1))) % 12;
        let n0 = Self::corner_noise_3d(gi0, x0, y0, z0, 0.6);
        let n1 = Self::corner_noise_3d(gi1, x1, y1, z1, 0.6);
        let n2 = Self::corner_noise_3d(gi2, x2, y2, z2, 0.6);
        let n3 = Self::corner_noise_3d(gi3, x3, y3, z3, 0.6);
        (32.0 * (n0 + n1 + n2 + n3)) as f32
    }
}

/// One octave of a stack: either a plain PerlinNoise or a SmearedPerlinNoise.
#[derive(Clone, Debug)]
pub enum Octave {
    Perlin(PerlinNoise),
    Smeared(SmearedPerlinNoise),
}

impl Octave {
    fn get_value(&self, x: f64, y: f64, z: f64) -> f32 {
        match self {
            Octave::Perlin(p) => p.get_value(x, y, z),
            Octave::Smeared(s) => s.get_value(x, y, z),
        }
    }

    /// Inherited `get(x, y)` = `get(wrap(x), 0.0, wrap(y))` through the (possibly overridden) 3D get.
    fn get_value_2d(&self, x: f64, y: f64) -> f32 {
        self.get_value(wrap(x), 0.0, wrap(y))
    }
}

/// `NoiseStack`: layers summed in insertion order as `value += amplitude * noise(...)`.
#[derive(Clone, Debug)]
pub struct NoiseStack {
    layers: Vec<Layer>,
}

#[derive(Clone, Debug)]
pub struct Layer {
    pub noise: Octave,
    pub frequency: f64,
    pub amplitude: f32,
}

impl NoiseStack {
    pub fn new(layers: Vec<Layer>) -> Self {
        Self { layers }
    }

    pub fn get_value(&self, x: f64, y: f64, z: f64) -> f32 {
        let mut value = 0.0_f32;
        for layer in &self.layers {
            let f = layer.frequency;
            value += layer.amplitude * layer.noise.get_value(x * f, y * f, z * f);
        }
        value
    }

    pub fn get_value_2d(&self, x: f64, y: f64) -> f32 {
        let mut value = 0.0_f32;
        for layer in &self.layers {
            let f = layer.frequency;
            value += layer.amplitude * layer.noise.get_value_2d(x * f, y * f);
        }
        value
    }
}

/// Java `DoubleStream.sum()`: compensated (Kahan-style) summation, as in `Collectors.sumWithCompensation`.
fn java_double_sum(values: impl Iterator<Item = f64>) -> f64 {
    let mut sum = 0.0_f64;
    let mut compensation = 0.0_f64;
    let mut simple = 0.0_f64;
    for v in values {
        let tmp = v - compensation;
        let velvel = sum + tmp;
        compensation = (velvel - sum) - tmp;
        sum = velvel;
        simple += v;
    }
    let tmp = sum - compensation;
    if tmp.is_nan() && simple.is_infinite() {
        simple
    } else {
        tmp
    }
}

#[derive(Clone, Copy, Debug)]
struct OctaveInfo {
    octave_index: i32,
    frequency: f64,
    amplitude: f64,
}

fn amplitude_modifier(modifiers: &[f64], index: usize) -> f64 {
    if modifiers.is_empty() {
        1.0
    } else {
        modifiers[index]
    }
}

/// `NormalNoise.buildOctaves`.
fn build_octaves(
    base_octave: i32,
    base_amplitude: f64,
    octave_count: usize,
    normalize: bool,
    modifiers: &[f64],
) -> Vec<OctaveInfo> {
    let mut frequency = 2f64.powi(base_octave);
    let mut amplitude = base_amplitude;
    if normalize {
        amplitude *= 0.5f64.powi(-(octave_count as i32 - 1))
            / (0.5f64.powi(-(octave_count as i32)) - 1.0);
    }
    let mut octaves = Vec::with_capacity(octave_count);
    for i in 0..octave_count {
        let modifier = amplitude_modifier(modifiers, i);
        if modifier != 0.0 {
            octaves.push(OctaveInfo {
                octave_index: base_octave + i as i32,
                frequency,
                amplitude: amplitude * modifier,
            });
        }
        frequency *= 2.0;
        amplitude *= 0.5;
    }
    octaves
}

/// `NormalNoise.estimateDeviation` (plain loop, not compensated).
fn estimate_deviation(octaves: &[OctaveInfo]) -> f64 {
    let mut variance = 0.0;
    for o in octaves {
        let layer_deviation = 0.270_224_783_124_521_1 * o.amplitude.abs();
        variance += mth::square(layer_deviation);
    }
    variance.sqrt()
}

fn compute_normalization_factor(target_amplitude: f64, octaves: &[OctaveInfo]) -> f64 {
    let input_deviation = estimate_deviation(octaves);
    if input_deviation == 0.0 {
        return 0.0;
    }
    let input_sum_deviation = input_deviation * 2.0_f64.sqrt();
    let target_deviation = target_amplitude * 0.333_333_333_333_333_3;
    target_deviation / input_sum_deviation
}

fn parity_expected_deviation(octave_span: i32) -> f64 {
    0.1 * (1.0 + 1.0 / (octave_span + 1) as f64)
}

fn compute_parity_normalization_factor(
    base_amplitude: f64,
    octave_count: usize,
    modifiers: &[f64],
) -> f64 {
    let mut min_octave = i32::MAX;
    let mut max_octave = i32::MIN;
    for i in 0..octave_count {
        if amplitude_modifier(modifiers, i) != 0.0 {
            min_octave = min_octave.min(i as i32);
            max_octave = max_octave.max(i as i32);
        }
    }
    base_amplitude * 0.5 * 0.333_333_333_333_333_3
        / parity_expected_deviation(max_octave.wrapping_sub(min_octave))
}

fn compute_parity_base_amplitude(base_octave: i32, amplitudes: &[f64]) -> f64 {
    let octaves = build_octaves(base_octave, 1.0, amplitudes.len(), true, amplitudes);
    let target = java_double_sum(octaves.iter().map(|o| o.amplitude.abs()));
    let new_normalization = compute_normalization_factor(target, &octaves);
    if new_normalization == 0.0 {
        return 1.0;
    }
    let old_normalization = compute_parity_normalization_factor(1.0, amplitudes.len(), amplitudes);
    old_normalization / new_normalization
}

/// `NormalNoise.Normalization`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Normalization {
    Disabled,
    Enabled,
    /// Deprecated `setLegacyNormalization()`.
    Legacy,
}

/// `NormalNoise` (general constructor: `NormalNoise(Parameters)`).
#[derive(Clone, Debug)]
pub struct NormalNoise {
    base_octave: i32,
    octaves: Vec<OctaveInfo>,
    normalization_factor: f64,
    target_amplitude: f64,
}

impl NormalNoise {
    /// `new NormalNoise(parameters)`. `modifiers` is empty (all 1.0) or has one entry per octave.
    pub fn new(
        base_amplitude: f64,
        base_octave: i32,
        octave_count: usize,
        normalize: Normalization,
        modifiers: &[f64],
    ) -> Self {
        let octaves = build_octaves(
            base_octave,
            base_amplitude,
            octave_count,
            normalize != Normalization::Disabled,
            modifiers,
        );
        let mut target_amplitude = java_double_sum(octaves.iter().map(|o| o.amplitude.abs()));
        let mut normalization_factor = compute_normalization_factor(target_amplitude, &octaves);
        if normalize == Normalization::Legacy && normalization_factor != 0.0 {
            let parity =
                compute_parity_normalization_factor(base_amplitude, octave_count, modifiers);
            target_amplitude *= parity / normalization_factor;
            normalization_factor = parity;
        }
        Self {
            base_octave,
            octaves,
            normalization_factor,
            target_amplitude,
        }
    }

    /// `NormalNoise.createParity(firstOctave, amplitudes)`: parity base amplitude, `ENABLED` normalization.
    pub fn create_parity(first_octave: i32, amplitudes: &[f64]) -> Self {
        assert!(!amplitudes.is_empty(), "Need at least 1 amplitude");
        let base_amplitude = compute_parity_base_amplitude(first_octave, amplitudes);
        Self::new(
            base_amplitude,
            first_octave,
            amplitudes.len(),
            Normalization::Enabled,
            amplitudes,
        )
    }

    /// `NormalNoise.create(RandomSource)`: two positional forks, one PerlinNoise per octave and per
    /// frequency-shifted copy, seeded from `fromHashOf("octave_<index>")`.
    pub fn create(&self, rng: &mut AnyRng) -> NoiseStack {
        let first: Positional = rng.fork_positional();
        let second: Positional = rng.fork_positional();
        let mut layers = Vec::with_capacity(self.octaves.len() * 2);
        for octave in &self.octaves {
            let name = format!("octave_{}", octave.octave_index);
            let first_noise = PerlinNoise::new(&mut first.from_hash_of(&name));
            let second_noise = PerlinNoise::new(&mut second.from_hash_of(&name));
            let value_factor = (self.normalization_factor * octave.amplitude) as f32;
            layers.push(Layer {
                noise: Octave::Perlin(first_noise),
                frequency: octave.frequency,
                amplitude: value_factor,
            });
            layers.push(Layer {
                noise: Octave::Perlin(second_noise),
                frequency: octave.frequency * NORMAL_INPUT_FACTOR,
                amplitude: value_factor,
            });
        }
        NoiseStack::new(layers)
    }

    /// `NormalNoise.range()`: symmetric interval `[-h, h]` with `h = (float)(target * 1/3 * 6)`.
    pub fn range(&self) -> (f32, f32) {
        let half = (self.target_amplitude * 0.333_333_333_333_333_3 * 6.0) as f32;
        (-half, half)
    }

    /// First octave index (`base_octave`).
    pub fn base_octave(&self) -> i32 {
        self.base_octave
    }
}

/// `BlendedNoise` (overworld "old_blended_noise"), built the way `compileSampler(RandomSource)` builds it.
#[derive(Clone, Debug)]
pub struct BlendedNoise {
    min_limit: NoiseStack,
    max_limit: NoiseStack,
    main: NoiseStack,
    xz_multiplier: f64,
    y_multiplier: f64,
    main_xz_scale: f64,
    main_y_scale: f64,
}

/// `BlendedNoise.createFbm(random, firstOctave, smearScaleY, valueFactor)`.
fn create_fbm(rng: &mut AnyRng, first_octave: i32, smear_scale_y: f64, value_factor: f64) -> NoiseStack {
    assert!(first_octave <= 0, "firstOctave>0");
    let octaves = -first_octave + 1;
    let mut factor = 1.0_f64;
    let mut value_factor = value_factor;
    value_factor /= 2f64.powi(octaves) - 1.0;
    let mut layers = Vec::with_capacity(octaves as usize);
    for _ in (0..octaves).rev() {
        let noise = SmearedPerlinNoise::new(rng, smear_scale_y * factor);
        layers.push(Layer {
            noise: Octave::Smeared(noise),
            frequency: factor,
            amplitude: value_factor as f32,
        });
        factor /= 2.0;
        value_factor *= 2.0;
    }
    NoiseStack::new(layers)
}

impl BlendedNoise {
    /// Consumes `rng` exactly as `BlendedNoise.compileSampler(RandomSource)` does:
    /// min limit, max limit, then main.
    pub fn new(
        rng: &mut AnyRng,
        xz_scale: f64,
        y_scale: f64,
        xz_factor: f64,
        y_factor: f64,
        smear_scale_multiplier: f64,
    ) -> Self {
        let xz_multiplier = BLENDED_BASE_SCALE * xz_scale;
        let y_multiplier = BLENDED_BASE_SCALE * y_scale;
        let limit_smear_scale_y = y_multiplier * smear_scale_multiplier;
        let main_smear_scale_y = limit_smear_scale_y / y_factor;
        let min_limit = create_fbm(rng, -15, limit_smear_scale_y, BLENDED_LIMIT_FACTOR);
        let max_limit = create_fbm(rng, -15, limit_smear_scale_y, BLENDED_LIMIT_FACTOR);
        let main = create_fbm(rng, -7, main_smear_scale_y, BLENDED_MAIN_FACTOR);
        Self {
            min_limit,
            max_limit,
            main,
            xz_multiplier,
            y_multiplier,
            main_xz_scale: xz_multiplier / xz_factor,
            main_y_scale: y_multiplier / y_factor,
        }
    }

    /// `DensitySampler.sampleValue` of the compiled overworld blended noise at block coordinates.
    /// `NoiseFunction.Sampler` scales by `blockX * xzScale`, `blockY * yScale`, `blockZ * xzScale`.
    pub fn compute(&self, block_x: i32, block_y: i32, block_z: i32) -> f32 {
        let main_value = self.main.get_value(
            block_x as f64 * self.main_xz_scale,
            block_y as f64 * self.main_y_scale,
            block_z as f64 * self.main_xz_scale,
        );
        let alpha = clamp_f32(main_value + 0.5_f32, 0.0, 1.0);
        let first = || {
            self.min_limit.get_value(
                block_x as f64 * self.xz_multiplier,
                block_y as f64 * self.y_multiplier,
                block_z as f64 * self.xz_multiplier,
            )
        };
        let second = || {
            self.max_limit.get_value(
                block_x as f64 * self.xz_multiplier,
                block_y as f64 * self.y_multiplier,
                block_z as f64 * self.xz_multiplier,
            )
        };
        if alpha == 0.0 {
            first()
        } else if alpha == 1.0 {
            second()
        } else {
            lerp(alpha, first(), second())
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Volume path. `Noise.addToVolume` in vanilla 26.3 is NOT the point path: PerlinNoise and
// SmearedPerlinNoise use `addToVolumeVanilla`, which sums gradient terms in a different float
// order (`dotXz` plus `g.y * relY`). Keep these bodies literal.
// ---------------------------------------------------------------------------------------------

use crate::volume::Vol;

/// Gradient terms for the 8 cell corners of one (floorX, floorY, floorZ) column. Index order:
/// 000, 100, 010, 110, 001, 101, 011, 111 (the Java variable suffixes).
#[inline]
fn corner_terms(
    base: &GradientBase,
    x0: i32,
    x1: i32,
    floor_y: i32,
    floor_z: i32,
    rel_x: f32,
    rel_z: f32,
) -> ([f32; 8], [f32; 8]) {
    let grad = |v: i32| GRADIENT[(base.permute(v) & 15) as usize];
    let xy00 = base.permute(x0.wrapping_add(floor_y));
    let xy01 = base.permute(x0.wrapping_add(floor_y).wrapping_add(1));
    let xy10 = base.permute(x1.wrapping_add(floor_y));
    let xy11 = base.permute(x1.wrapping_add(floor_y).wrapping_add(1));
    let fz = floor_z;
    let rx1 = rel_x - 1.0;
    let rz1 = rel_z - 1.0;
    // (gx, gy, gz) -> dotXz(x, z) = gx*x + gz*z, and y = gy as float.
    let dot_xz = |g: (i32, i32, i32), x: f32, z: f32| (g.0 as f32) * x + (g.2 as f32) * z;
    let g000 = grad(xy00.wrapping_add(fz));
    let g100 = grad(xy10.wrapping_add(fz));
    let g010 = grad(xy01.wrapping_add(fz));
    let g110 = grad(xy11.wrapping_add(fz));
    let g001 = grad(xy00.wrapping_add(fz).wrapping_add(1));
    let g101 = grad(xy10.wrapping_add(fz).wrapping_add(1));
    let g011 = grad(xy01.wrapping_add(fz).wrapping_add(1));
    let g111 = grad(xy11.wrapping_add(fz).wrapping_add(1));
    let d = [
        dot_xz(g000, rel_x, rel_z),
        dot_xz(g100, rx1, rel_z),
        dot_xz(g010, rel_x, rel_z),
        dot_xz(g110, rx1, rel_z),
        dot_xz(g001, rel_x, rz1),
        dot_xz(g101, rx1, rz1),
        dot_xz(g011, rel_x, rz1),
        dot_xz(g111, rx1, rz1),
    ];
    let gy = [
        g000.1 as f32,
        g100.1 as f32,
        g010.1 as f32,
        g110.1 as f32,
        g001.1 as f32,
        g101.1 as f32,
        g011.1 as f32,
        g111.1 as f32,
    ];
    (d, gy)
}

/// Shared body of `addToVolumeVanilla`. `y_terms(indexY)` yields `(floorY, alphaY, fudgedRelY)`
/// for the Y coordinate of a block row, given the block's `y` (already scaled).
#[inline]
fn add_volume_body(
    base: &GradientBase,
    out: &mut [f32],
    vol: &Vol,
    xz_scale: f64,
    amplitude: f32,
    y_terms: &dyn Fn(i32) -> (i32, f32, f32),
) {
    let mut index = 0usize;
    for iz in 0..vol.sz {
        let z = wrap(vol.block_z(iz) as f64 * xz_scale) + base.offset_z;
        let floor_z = floor(z);
        let rel_z = (z - floor_z as f64) as f32;
        let alpha_z = smoothstep(rel_z);
        for ix in 0..vol.sx {
            let x = wrap(vol.block_x(ix) as f64 * xz_scale) + base.offset_x;
            let floor_x = floor(x);
            let rel_x = (x - floor_x as f64) as f32;
            let x0 = base.permute(floor_x);
            let x1 = base.permute(floor_x.wrapping_add(1));
            let alpha_x = smoothstep(rel_x);
            let mut last_floor_y = i32::MIN;
            let mut d = [0f32; 8];
            let mut gy = [0f32; 8];
            for iy in 0..vol.sy {
                let (floor_y, alpha_y, rel_y) = y_terms(iy);
                if last_floor_y != floor_y {
                    let (nd, ng) = corner_terms(base, x0, x1, floor_y, floor_z, rel_x, rel_z);
                    d = nd;
                    gy = ng;
                    last_floor_y = floor_y;
                }
                let one = rel_y - 1.0;
                let value = lerp3(
                    alpha_x,
                    alpha_y,
                    alpha_z,
                    d[0] + gy[0] * rel_y,
                    d[1] + gy[1] * rel_y,
                    d[2] + gy[2] * one,
                    d[3] + gy[3] * one,
                    d[4] + gy[4] * rel_y,
                    d[5] + gy[5] * rel_y,
                    d[6] + gy[6] * one,
                    d[7] + gy[7] * one,
                );
                out[index] = out[index] + amplitude * value;
                index += 1;
            }
        }
    }
}

impl PerlinNoise {
    /// `PerlinNoise.addToVolume` (vanilla body). Adds `amplitude * noise` into `out`.
    pub fn add_to_volume(
        &self,
        out: &mut [f32],
        vol: &Vol,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
    ) {
        let base = &self.base;
        add_volume_body(base, out, vol, xz_scale, amplitude, &|iy| {
            let y = wrap(vol.block_y(iy) as f64 * y_scale) + base.offset_y;
            let floor_y = floor(y);
            let rel_y = (y - floor_y as f64) as f32;
            (floor_y, smoothstep(rel_y), rel_y)
        });
    }
}

impl SmearedPerlinNoise {
    /// `SmearedPerlinNoise.addToVolume` (vanilla body).
    pub fn add_to_volume(
        &self,
        out: &mut [f32],
        vol: &Vol,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
    ) {
        let base = &self.base;
        add_volume_body(base, out, vol, xz_scale, amplitude, &|iy| {
            let original_y = vol.block_y(iy) as f64 * y_scale;
            let y = wrap(original_y) + base.offset_y;
            let floor_y = floor(y);
            let rel_y = y - floor_y as f64;
            let alpha_y = smoothstep(rel_y as f32);
            let fudged = (rel_y - self.compute_fudge_y(original_y, rel_y)) as f32;
            (floor_y, alpha_y, fudged)
        });
    }
}

impl Octave {
    pub fn add_to_volume(&self, out: &mut [f32], vol: &Vol, xz: f64, y: f64, amp: f32) {
        match self {
            Octave::Perlin(p) => p.add_to_volume(out, vol, xz, y, amp),
            Octave::Smeared(s) => s.add_to_volume(out, vol, xz, y, amp),
        }
    }
}

impl NoiseStack {
    /// `NoiseStack.addToVolume`: each layer adds with `xz * frequency`, `y * frequency`,
    /// `amplitude * layer.amplitude`.
    pub fn add_to_volume(&self, out: &mut [f32], vol: &Vol, xz: f64, y: f64, amplitude: f32) {
        for layer in &self.layers {
            let f = layer.frequency;
            layer
                .noise
                .add_to_volume(out, vol, xz * f, y * f, amplitude * layer.amplitude);
        }
    }
}

/// The three stacks and multipliers of `BlendedNoise.compileSampler` (overworld `old_blended_noise`).
pub struct BlendedParts {
    pub min_limit: NoiseStack,
    pub max_limit: NoiseStack,
    pub main: NoiseStack,
    pub xz_multiplier: f64,
    pub y_multiplier: f64,
    pub main_xz_scale: f64,
    pub main_y_scale: f64,
}

impl BlendedNoise {
    /// Clones out the parts needed to compile `BlendedNoise` as Lerp(Clamp(main + 0.5), min, max).
    pub fn parts(&self) -> BlendedParts {
        BlendedParts {
            min_limit: self.min_limit.clone(),
            max_limit: self.max_limit.clone(),
            main: self.main.clone(),
            xz_multiplier: self.xz_multiplier,
            y_multiplier: self.y_multiplier,
            main_xz_scale: self.main_xz_scale,
            main_y_scale: self.main_y_scale,
        }
    }
}
