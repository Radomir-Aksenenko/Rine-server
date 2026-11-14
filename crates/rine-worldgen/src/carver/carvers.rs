//! The two overworld carvers of vanilla 26.3: `CaveWorldCarver` (`cave`, `cave_extra_underground`) and
//! `CanyonWorldCarver` (`canyon`). They only set bits of the `CarvingMask`; the blocks change later in
//! `apply_mask`. The configurations are the bootstrap values of `data/worldgen/Carvers.java`, which are
//! the same numbers as `worldgen/carver/*.json` in the vanilla 26.3 data.
//!
//! Every expression keeps the operand types and evaluation order of the Java source: float arithmetic
//! stays `f32` (`Mth` helpers return floats), widening to `f64` happens where Java widens. Random draws
//! are bound to `let`s in source order.

use crate::carver::mask::CarvingMask;
use crate::carver::trig::{cos, sin};
use crate::random::{Legacy, Rng};

/// `Mth.PI` = `(float) Math.PI`.
const PI: f32 = std::f32::consts::PI;
/// `(float) (Math.PI / 2)`; halving is exact so this equals `Mth.PI / 2.0F`.
const HALF_PI: f32 = PI / 2.0;
/// `(float) (Math.PI * 2)`.
const TWO_PI: f32 = PI * 2.0;

/// `VerticalAnchor` resolved against `WorldGenerationContext`.
#[derive(Clone, Copy, Debug)]
pub enum Anchor {
    /// `VerticalAnchor.absolute(y)`.
    Absolute(i32),
    /// `VerticalAnchor.aboveBottom(offset)`: `getMinGenY() + offset`.
    AboveBottom(i32),
}

impl Anchor {
    fn resolve(self, min_gen_y: i32) -> i32 {
        match self {
            Anchor::Absolute(y) => y,
            Anchor::AboveBottom(o) => min_gen_y + o,
        }
    }
}

/// `UniformHeight(min, max)`: `Mth.randomBetweenInclusive` unless the range is empty.
#[derive(Clone, Copy, Debug)]
pub struct UniformHeight {
    pub min: Anchor,
    pub max: Anchor,
}

impl UniformHeight {
    fn sample(&self, r: &mut Legacy, min_gen_y: i32) -> i32 {
        let min = self.min.resolve(min_gen_y);
        let max = self.max.resolve(min_gen_y);
        if min > max {
            return min;
        }
        r.next_int_bound(max - min + 1) + min
    }
}

/// `UniformFloat(min, max)`: `Mth.randomBetween` (`nextFloat() * (max - min) + min`).
#[derive(Clone, Copy, Debug)]
pub struct Uniform {
    pub min: f32,
    pub max: f32,
}

impl Uniform {
    fn sample(&self, r: &mut Legacy) -> f32 {
        r.next_float() * (self.max - self.min) + self.min
    }
}

/// `TrapezoidFloat(min, max, plateau)`.
#[derive(Clone, Copy, Debug)]
pub struct Trapezoid {
    pub min: f32,
    pub max: f32,
    pub plateau: f32,
}

impl Trapezoid {
    fn sample(&self, r: &mut Legacy) -> f32 {
        let range = self.max - self.min;
        let plateau_start = (range - self.plateau) / 2.0;
        let plateau_end = range - plateau_start;
        let a = r.next_float();
        let b = r.next_float();
        self.min + a * plateau_end + b * plateau_start
    }
}

/// `CaveWorldCarver` record.
#[derive(Clone, Copy, Debug)]
pub struct CaveCfg {
    pub probability: f32,
    pub y: UniformHeight,
    /// `VeryBiasedToBottomInt(0, 14)` count.
    pub count_max_inclusive: i32,
    pub thickness: Trapezoid,
    pub weird_thickness_bias: bool,
    pub room_vertical_radius_multiplier: Uniform,
    pub horizontal_radius_multiplier: Uniform,
    pub vertical_radius_multiplier: Uniform,
    /// `ConstantFloat`: the start multiplier, no draw.
    pub start_vertical_radius_multiplier: f32,
    pub floor_level: Uniform,
}

/// `CanyonWorldCarver` record and its `Shape`.
#[derive(Clone, Copy, Debug)]
pub struct CanyonCfg {
    pub probability: f32,
    pub y: UniformHeight,
    pub vertical_rotation: Uniform,
    pub distance_factor: Uniform,
    pub thickness: Trapezoid,
    pub width_smoothness: i32,
    pub horizontal_radius_factor: Uniform,
    pub vertical_radius_default_factor: f32,
    pub vertical_radius_center_factor: f32,
    /// `ConstantFloat`.
    pub y_scale: f32,
}

pub const CAVE: CaveCfg = CaveCfg {
    probability: 0.15,
    y: UniformHeight { min: Anchor::AboveBottom(8), max: Anchor::Absolute(180) },
    count_max_inclusive: 14,
    thickness: Trapezoid { min: 0.0, max: 3.0, plateau: 1.0 },
    weird_thickness_bias: true,
    room_vertical_radius_multiplier: Uniform { min: 0.1, max: 0.9 },
    horizontal_radius_multiplier: Uniform { min: 0.7, max: 1.4 },
    vertical_radius_multiplier: Uniform { min: 0.8, max: 1.3 },
    start_vertical_radius_multiplier: 1.0,
    floor_level: Uniform { min: -1.0, max: -0.4 },
};

pub const CAVE_EXTRA_UNDERGROUND: CaveCfg = CaveCfg {
    probability: 0.07,
    y: UniformHeight { min: Anchor::AboveBottom(8), max: Anchor::Absolute(47) },
    ..CAVE
};

pub const CANYON: CanyonCfg = CanyonCfg {
    probability: 0.01,
    y: UniformHeight { min: Anchor::Absolute(10), max: Anchor::Absolute(67) },
    vertical_rotation: Uniform { min: -0.125, max: 0.125 },
    distance_factor: Uniform { min: 0.75, max: 1.0 },
    thickness: Trapezoid { min: 0.0, max: 6.0, plateau: 2.0 },
    width_smoothness: 3,
    horizontal_radius_factor: Uniform { min: 0.75, max: 1.0 },
    vertical_radius_default_factor: 1.0,
    vertical_radius_center_factor: 0.0,
    y_scale: 3.0,
};

/// `WorldCarver.getRange()` = 4 for both carvers.
const RANGE: i32 = 4;
/// `SectionPos.sectionToBlockCoord(range * 2 - 1)` for the cave, `(range * 2 - 1) * 16` for the canyon.
const MAX_DISTANCE: i32 = (RANGE * 2 - 1) * 16;

/// What the carvers need to know about the chunk being carved.
pub struct Env {
    /// Chunk being carved (`chunkPos`).
    pub cx: i32,
    pub cz: i32,
    /// `WorldGenerationContext.getMinGenY()` and `getGenDepth()`.
    pub min_gen_y: i32,
    pub gen_depth: i32,
}

/// `Mth.floor(double)`.
#[inline]
fn floor(v: f64) -> i32 {
    v.floor() as i32
}

/// `WorldCarver.carveEllipsoid`. Calls `skip(xd, yd, zd, worldY)` for each candidate block; the mask keeps
/// the blocks that are not skipped.
#[allow(clippy::too_many_arguments)]
fn carve_ellipsoid<F: Fn(f64, f64, f64, i32) -> bool>(
    env: &Env,
    x: f64,
    y: f64,
    z: f64,
    horizontal_radius: f64,
    vertical_radius: f64,
    mask: &mut CarvingMask,
    skip: F,
) {
    let center_x = (env.cx * 16 + 8) as f64;
    let center_z = (env.cz * 16 + 8) as f64;
    let max_delta = 16.0 + horizontal_radius * 2.0;
    if !((x - center_x).abs() > max_delta) && !((z - center_z).abs() > max_delta) {
        let chunk_min_x = env.cx * 16;
        let chunk_min_z = env.cz * 16;
        let h = horizontal_radius;
        let v = vertical_radius;
        let min_x_index = (floor(x - h) - chunk_min_x - 1).max(0);
        let max_x_index = (floor(x + h) - chunk_min_x).min(15);
        let min_y = (floor(y - v) - 1).max(mask.min_y());
        let max_y = (floor(y + v) + 1).min(mask.max_y());
        let min_z_index = (floor(z - h) - chunk_min_z - 1).max(0);
        let max_z_index = (floor(z + h) - chunk_min_z).min(15);

        for x_index in min_x_index..=max_x_index {
            let world_x = chunk_min_x + x_index;
            let xd = (world_x as f64 + 0.5 - x) / h;
            for z_index in min_z_index..=max_z_index {
                let world_z = chunk_min_z + z_index;
                let zd = (world_z as f64 + 0.5 - z) / h;
                if !(xd * xd + zd * zd >= 1.0) {
                    let mut world_y = max_y;
                    while world_y > min_y {
                        let yd = (world_y as f64 - 0.5 - y) / v;
                        if !skip(xd, yd, zd, world_y) {
                            mask.carve(x_index, world_y, z_index);
                        }
                        world_y -= 1;
                    }
                }
            }
        }
    }
}

/// `WorldCarver.canReach`.
fn can_reach(env: &Env, x: f64, z: f64, current_step: i32, total_steps: i32, thickness: f32) -> bool {
    let x_mid = (env.cx * 16 + 8) as f64;
    let z_mid = (env.cz * 16 + 8) as f64;
    let xd = x - x_mid;
    let zd = z - z_mid;
    let remaining = (total_steps - current_step) as f64;
    // `double rr = thickness + 2.0F + 16.0F;` is a float expression widened afterwards.
    let rr_f: f32 = thickness + 2.0 + 16.0;
    let rr = rr_f as f64;
    xd * xd + zd * zd - remaining * remaining <= rr * rr
}

/// Reads and returns the cave shape parameters that are sampled once per cave.
impl CaveCfg {
    /// `CaveWorldCarver.getThickness`.
    fn thickness_of(&self, r: &mut Legacy) -> f32 {
        let mut thickness = self.thickness.sample(r);
        if self.weird_thickness_bias && r.next_int_bound(10) == 0 {
            let a = r.next_float();
            let b = r.next_float();
            thickness *= a * b * 3.0 + 1.0;
        }
        thickness
    }

    /// `CaveWorldCarver.carve`.
    fn carve(&self, env: &Env, r: &mut Legacy, src: (i32, i32), mask: &mut CarvingMask) {
        let (sx, sz) = src;
        let cave_count = {
            let a = r.next_int_bound(self.count_max_inclusive + 1);
            let b = r.next_int_bound(a + 1);
            let c = r.next_int_bound(b + 1);
            c
        };
        for _ in 0..cave_count {
            let rx = r.next_int_bound(16);
            let x = (sx * 16 + rx) as f64;
            let y = self.y.sample(r, env.min_gen_y) as f64;
            let rz = r.next_int_bound(16);
            let z = (sz * 16 + rz) as f64;
            let horizontal_radius_multiplier = self.horizontal_radius_multiplier.sample(r) as f64;
            let vertical_radius_multiplier = self.vertical_radius_multiplier.sample(r) as f64;
            let start_vertical_radius_multiplier = self.start_vertical_radius_multiplier as f64;
            let floor_level = self.floor_level.sample(r) as f64;
            let mut tunnels = 1;
            if r.next_int_bound(4) == 0 {
                let y_scale = self.room_vertical_radius_multiplier.sample(r) as f64;
                let a = r.next_float();
                let thickness: f32 = 1.0 + a * 6.0;
                // createRoom
                let horizontal_radius = 1.5 + (sin(HALF_PI as f64) * thickness) as f64;
                let vertical_radius = horizontal_radius * y_scale;
                carve_ellipsoid(env, x + 1.0, y, z, horizontal_radius, vertical_radius, mask, |xd, yd, zd, _| {
                    cave_skip(xd, yd, zd, floor_level)
                });
                tunnels += r.next_int_bound(4);
            }
            for _ in 0..tunnels {
                let a = r.next_float();
                let horizontal_rotation: f32 = a * TWO_PI;
                let b = r.next_float();
                let vertical_rotation: f32 = (b - 0.5) / 4.0;
                let thickness = self.thickness_of(r);
                let distance = MAX_DISTANCE - r.next_int_bound(MAX_DISTANCE / 4);
                let tunnel_seed = r.next_long();
                tunnel(
                    env,
                    mask,
                    tunnel_seed,
                    Tunnel {
                        x,
                        y,
                        z,
                        horizontal_radius_multiplier,
                        vertical_radius_multiplier,
                        thickness,
                        horizontal_rotation,
                        vertical_rotation,
                        step: 0,
                        dist: distance,
                        y_scale: start_vertical_radius_multiplier,
                        floor_level,
                    },
                );
            }
        }
    }
}

/// `CaveWorldCarver.shouldSkip`.
#[inline]
fn cave_skip(xd: f64, yd: f64, zd: f64, floor_level: f64) -> bool {
    yd <= floor_level || xd * xd + yd * yd + zd * zd >= 1.0
}

#[derive(Clone, Copy)]
struct Tunnel {
    x: f64,
    y: f64,
    z: f64,
    horizontal_radius_multiplier: f64,
    vertical_radius_multiplier: f64,
    thickness: f32,
    horizontal_rotation: f32,
    vertical_rotation: f32,
    step: i32,
    dist: i32,
    y_scale: f64,
    floor_level: f64,
}

/// `CaveWorldCarver.createTunnel`. The tunnel random is `RandomSource.createThreadLocalInstance(seed)`,
/// which is the same 48-bit LCG as `Legacy`.
fn tunnel(env: &Env, mask: &mut CarvingMask, tunnel_seed: i64, t: Tunnel) {
    let mut r = Legacy::new(tunnel_seed);
    let Tunnel {
        mut x,
        mut y,
        mut z,
        horizontal_radius_multiplier,
        vertical_radius_multiplier,
        thickness,
        mut horizontal_rotation,
        mut vertical_rotation,
        step,
        dist,
        y_scale,
        floor_level,
    } = t;
    let split_point = r.next_int_bound(dist / 2) + dist / 4;
    let steep = r.next_int_bound(6) == 0;
    let mut y_rota: f32 = 0.0;
    let mut x_rota: f32 = 0.0;

    let mut current_step = step;
    while current_step < dist {
        // Mth.sin(Mth.PI * currentStep / dist): float expression, widened to double by the call.
        let arg = PI * current_step as f32 / dist as f32;
        let horizontal_radius = 1.5 + (sin(arg as f64) * thickness) as f64;
        let vertical_radius = horizontal_radius * y_scale;
        let cos_x = cos(vertical_rotation as f64);
        x += (cos(horizontal_rotation as f64) * cos_x) as f64;
        y += sin(vertical_rotation as f64) as f64;
        z += (sin(horizontal_rotation as f64) * cos_x) as f64;
        vertical_rotation *= if steep { 0.92 } else { 0.7 };
        vertical_rotation += x_rota * 0.1;
        horizontal_rotation += y_rota * 0.1;
        x_rota *= 0.9;
        y_rota *= 0.75;
        let a = r.next_float();
        let b = r.next_float();
        let c = r.next_float();
        x_rota += (a - b) * c * 2.0;
        let d = r.next_float();
        let e = r.next_float();
        let f = r.next_float();
        y_rota += (d - e) * f * 4.0;

        if current_step == split_point && thickness > 1.0 {
            let seed_a = r.next_long();
            let g = r.next_float();
            let thickness_a = g * 0.5 + 0.5;
            tunnel(
                env,
                mask,
                seed_a,
                Tunnel {
                    x,
                    y,
                    z,
                    horizontal_radius_multiplier,
                    vertical_radius_multiplier,
                    thickness: thickness_a,
                    horizontal_rotation: horizontal_rotation - HALF_PI,
                    vertical_rotation: vertical_rotation / 3.0,
                    step: current_step,
                    dist,
                    y_scale: 1.0,
                    floor_level,
                },
            );
            let seed_b = r.next_long();
            let h = r.next_float();
            let thickness_b = h * 0.5 + 0.5;
            tunnel(
                env,
                mask,
                seed_b,
                Tunnel {
                    x,
                    y,
                    z,
                    horizontal_radius_multiplier,
                    vertical_radius_multiplier,
                    thickness: thickness_b,
                    horizontal_rotation: horizontal_rotation + HALF_PI,
                    vertical_rotation: vertical_rotation / 3.0,
                    step: current_step,
                    dist,
                    y_scale: 1.0,
                    floor_level,
                },
            );
            return;
        }

        if r.next_int_bound(4) != 0 {
            if !can_reach(env, x, z, current_step, dist, thickness) {
                return;
            }
            carve_ellipsoid(
                env,
                x,
                y,
                z,
                horizontal_radius * horizontal_radius_multiplier,
                vertical_radius * vertical_radius_multiplier,
                mask,
                |xd, yd, zd, _| cave_skip(xd, yd, zd, floor_level),
            );
        }
        current_step += 1;
    }
}

impl CanyonCfg {
    /// `CanyonWorldCarver.carve`.
    fn carve(&self, env: &Env, r: &mut Legacy, src: (i32, i32), mask: &mut CarvingMask) {
        let (sx, sz) = src;
        let rx = r.next_int_bound(16);
        let x = (sx * 16 + rx) as f64;
        let y = self.y.sample(r, env.min_gen_y);
        let rz = r.next_int_bound(16);
        let z = (sz * 16 + rz) as f64;
        let a = r.next_float();
        let horizontal_rotation: f32 = a * TWO_PI;
        let vertical_rotation = self.vertical_rotation.sample(r);
        let y_scale = self.y_scale as f64;
        let thickness = self.thickness.sample(r);
        let distance_factor = self.distance_factor.sample(r);
        let distance = (MAX_DISTANCE as f32 * distance_factor) as i32;
        let tunnel_seed = r.next_long();
        self.do_carve(env, tunnel_seed, x, y as f64, z, thickness, horizontal_rotation, vertical_rotation, 0, distance, y_scale, mask);
    }

    #[allow(clippy::too_many_arguments)]
    fn do_carve(
        &self,
        env: &Env,
        tunnel_seed: i64,
        mut x: f64,
        mut y: f64,
        mut z: f64,
        thickness: f32,
        mut horizontal_rotation: f32,
        mut vertical_rotation: f32,
        step: i32,
        distance: i32,
        y_scale: f64,
        mask: &mut CarvingMask,
    ) {
        let mut r = Legacy::new(tunnel_seed);
        let width_factor_per_height = self.init_width_factors(env, &mut r);
        let mut y_rota: f32 = 0.0;
        let mut x_rota: f32 = 0.0;

        let mut current_step = step;
        while current_step < distance {
            let arg = current_step as f32 * PI / distance as f32;
            let mut horizontal_radius = 1.5 + (sin(arg as f64) * thickness) as f64;
            let mut vertical_radius = horizontal_radius * y_scale;
            horizontal_radius *= self.horizontal_radius_factor.sample(&mut r) as f64;
            vertical_radius = self.update_vertical_radius(&mut r, vertical_radius, distance as f32, current_step as f32);
            let xc = cos(vertical_rotation as f64);
            let xs = sin(vertical_rotation as f64);
            x += (cos(horizontal_rotation as f64) * xc) as f64;
            y += xs as f64;
            z += (sin(horizontal_rotation as f64) * xc) as f64;
            vertical_rotation *= 0.7;
            vertical_rotation += x_rota * 0.05;
            horizontal_rotation += y_rota * 0.05;
            x_rota *= 0.8;
            y_rota *= 0.5;
            let a = r.next_float();
            let b = r.next_float();
            let c = r.next_float();
            x_rota += (a - b) * c * 2.0;
            let d = r.next_float();
            let e = r.next_float();
            let f = r.next_float();
            y_rota += (d - e) * f * 4.0;

            if r.next_int_bound(4) != 0 {
                if !can_reach(env, x, z, current_step, distance, thickness) {
                    return;
                }
                let wf = &width_factor_per_height;
                carve_ellipsoid(env, x, y, z, horizontal_radius, vertical_radius, mask, |xd, yd, zd, wy| {
                    // yIndex = y - context.getMinGenY(); widthFactorPerHeight[yIndex - 1]
                    let y_index = wy - env.min_gen_y;
                    (xd * xd + zd * zd) * wf[(y_index - 1) as usize] as f64 + yd * yd / 6.0 >= 1.0
                });
            }
            current_step += 1;
        }
    }

    /// `CanyonWorldCarver.initWidthFactors`: one squared factor per generated height.
    fn init_width_factors(&self, env: &Env, r: &mut Legacy) -> Vec<f32> {
        let depth = env.gen_depth as usize;
        let mut out = vec![0f32; depth];
        let mut width_factor: f32 = 1.0;
        for (y_index, slot) in out.iter_mut().enumerate() {
            if y_index == 0 || r.next_int_bound(self.width_smoothness) == 0 {
                let a = r.next_float();
                let b = r.next_float();
                width_factor = 1.0 + a * b;
            }
            *slot = width_factor * width_factor;
        }
        out
    }

    /// `CanyonWorldCarver.updateVerticalRadius`.
    fn update_vertical_radius(&self, r: &mut Legacy, vertical_radius: f64, distance: f32, current_step: f32) -> f64 {
        let vertical_multiplier = 1.0 - (0.5 - current_step / distance).abs() * 2.0;
        let factor = self.vertical_radius_default_factor + self.vertical_radius_center_factor * vertical_multiplier;
        let rb = r.next_float() * (1.0 - 0.75) + 0.75;
        (factor as f64 * vertical_radius) * rb as f64
    }
}

/// One configured carver of a biome's carver list.
#[derive(Clone, Copy, Debug)]
pub enum Carver {
    Cave(&'static CaveCfg),
    Canyon(&'static CanyonCfg),
}

impl Carver {
    /// `WorldCarver.isStartChunk`: `random.nextFloat() <= probability`.
    pub fn is_start_chunk(&self, r: &mut Legacy) -> bool {
        let p = match self {
            Carver::Cave(c) => c.probability,
            Carver::Canyon(c) => c.probability,
        };
        r.next_float() <= p
    }

    /// `WorldCarver.carve` for the source chunk `src`, writing into `mask`.
    pub fn carve(&self, env: &Env, r: &mut Legacy, src: (i32, i32), mask: &mut CarvingMask) {
        match self {
            Carver::Cave(c) => c.carve(env, r, src, mask),
            Carver::Canyon(c) => c.carve(env, r, src, mask),
        }
    }
}
