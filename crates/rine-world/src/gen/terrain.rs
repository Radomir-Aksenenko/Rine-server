//! Ground height, climate sampling, surface rules, bedrock/deepslate and caves.
//!
//! Climate is sampled at corners on a 4-block lattice (world multiples of 4) and
//! bilinearly interpolated. `Corners::column` is the one shared path for chunk
//! columns and for the neighbour lookups (trees, decorations), so the values are
//! bit-identical in every chunk.

use super::biome::{classify, Biome};
use super::grid::Grid;
use super::noise::{hash, lerp, mix, smoothstep, unit, Fbm2, Fbm3, SALT_BED, SALT_DEEP, SALT_SURF};
use super::Ids;
use crate::chunk::{MAX_Y, MIN_Y};

pub const SEA_LEVEL: i32 = 63;
const CAVE_MIN_Y: i32 = -59;
const LAVA_MAX_Y: i32 = -55;
const CHEESE_T: f32 = 0.45;
const TUNNEL_T: f32 = 0.07;

#[derive(Clone, Copy)]
pub struct Column {
    pub surface: i32,
    pub biome: Biome,
}

/// Noise fields. Corner values are `[cont, eros, peaks, river, temp, humid]`.
pub struct Noises {
    cont: Fbm2,
    eros: Fbm2,
    peaks: Fbm2,
    river: Fbm2,
    temp: Fbm2,
    humid: Fbm2,
    cheese: Fbm3,
    tun_a: Fbm3,
    tun_b: Fbm3,
}

impl Noises {
    pub fn new(seed: u64) -> Self {
        let s = |salt: u64| mix(seed ^ salt.wrapping_mul(0xA076_1D64_78BD_642F));
        Self {
            cont: Fbm2::new(s(1), 1.0 / 600.0, 4),
            eros: Fbm2::new(s(2), 1.0 / 400.0, 3),
            peaks: Fbm2::new(s(3), 1.0 / 260.0, 4),
            river: Fbm2::new(s(4), 1.0 / 450.0, 2),
            temp: Fbm2::new(s(5), 1.0 / 900.0, 3),
            humid: Fbm2::new(s(6), 1.0 / 800.0, 3),
            cheese: Fbm3::new(s(7), 1.0 / 36.0, 2),
            tun_a: Fbm3::new(s(8), 1.0 / 40.0, 1),
            tun_b: Fbm3::new(s(9), 1.0 / 40.0, 1),
        }
    }

    fn corner(&self, x: i32, z: i32) -> [f32; 6] {
        let (x, z) = (x as f32, z as f32);
        [
            self.cont.get(x, z),
            self.eros.get(x, z),
            self.peaks.get(x, z),
            self.river.get(x, z),
            // Temperature and humidity are stretched so that all biome bands occur.
            self.temp.get(x, z) * 1.6,
            self.humid.get(x, z) * 1.6,
        ]
    }
}

/// Continentalness to base ground height.
const SPLINE: [(f32, f32); 8] = [
    (-1.0, 25.0),
    (-0.45, 38.0),
    (-0.2, 52.0),
    (-0.08, 61.0),
    (0.02, 66.0),
    (0.2, 74.0),
    (0.5, 84.0),
    (1.0, 100.0),
];

fn spline(c: f32) -> f32 {
    if c <= SPLINE[0].0 {
        return SPLINE[0].1;
    }
    for w in SPLINE.windows(2) {
        let ((c0, h0), (c1, h1)) = (w[0], w[1]);
        if c <= c1 {
            return lerp(h0, h1, (c - c0) / (c1 - c0));
        }
    }
    SPLINE[SPLINE.len() - 1].1
}

/// Turns interpolated noise values into a column (ground height and biome).
fn derive(seed: u64, c: &[f32; 6], x: i32, z: i32) -> Column {
    let [cont, eros, peaks, river, temp, humid] = *c;
    let land = smoothstep(-0.05, 0.12, cont);
    let flat = smoothstep(-0.35, 0.45, eros);
    let mount = smoothstep(0.15, 0.55, cont) * (1.0 - flat);
    let ridge = (1.0 - 2.0 * peaks.abs()).max(0.0);
    // Slight shift so that the coastline sits at roughly a third of the terrain being water.
    let mut h = spline(cont + 0.05)
        + peaks * lerp(26.0, 4.0, flat) * land
        + peaks * 3.0 * (1.0 - land)
        + 120.0 * mount * ridge * ridge.sqrt();
    let rv = 1.0 - smoothstep(0.0, 0.02, river.abs());
    let rstr = rv * smoothstep(0.0, 0.08, cont) * (1.0 - mount);
    h = lerp(h, h.min(61.0), rstr);
    let surface = ((h + 0.5).floor() as i32).clamp(-60, 300);
    let biome = classify(seed, x, z, surface, cont, mount, rstr, temp, humid);
    Column { surface, biome }
}

/// Corner lattice covering `nx` x `nz` corners starting at world (x0, z0), step 4.
pub struct Corners {
    x0: i32,
    z0: i32,
    nx: usize,
    v: Vec<[f32; 6]>,
}

impl Corners {
    pub fn new(n: &Noises, x0: i32, z0: i32, nx: usize, nz: usize) -> Self {
        let mut v = Vec::with_capacity(nx * nz);
        for j in 0..nz {
            for i in 0..nx {
                v.push(n.corner(x0 + 4 * i as i32, z0 + 4 * j as i32));
            }
        }
        Self { x0, z0, nx, v }
    }

    /// Column at world (x, z). The four surrounding corners must lie inside the lattice.
    pub fn column(&self, seed: u64, x: i32, z: i32) -> Column {
        let i = ((x.div_euclid(4) * 4 - self.x0) / 4) as usize;
        let j = ((z.div_euclid(4) * 4 - self.z0) / 4) as usize;
        let fx = x.rem_euclid(4) as f32 * 0.25;
        let fz = z.rem_euclid(4) as f32 * 0.25;
        let w = self.nx;
        let c00 = self.v[j * w + i];
        let c10 = self.v[j * w + i + 1];
        let c01 = self.v[(j + 1) * w + i];
        let c11 = self.v[(j + 1) * w + i + 1];
        let mut c = [0f32; 6];
        for k in 0..6 {
            c[k] = lerp(lerp(c00[k], c10[k], fx), lerp(c01[k], c11[k], fx), fz);
        }
        derive(seed, &c, x, z)
    }
}

/// Column for a single world position (used by spawn search).
pub fn column_at(n: &Noises, seed: u64, x: i32, z: i32) -> Column {
    Corners::new(n, x.div_euclid(4) * 4, z.div_euclid(4) * 4, 2, 2).column(seed, x, z)
}

/// (top state, filler state, filler count) for a column.
fn surface_rule(ids: &Ids, seed: u64, col: Column, wx: i32, wz: i32) -> (u16, u16, i32) {
    use Biome::*;
    let r = unit(hash(seed, wx as i64, wz as i64, 0, SALT_SURF));
    let s = col.surface;
    let dirt_depth = 3 + (r > 0.5) as i32;
    match col.biome {
        Plains | SunflowerPlains | Forest | BirchForest | DarkForest | Taiga | Jungle | Savanna | Swamp => {
            (ids.grass, ids.dirt, dirt_depth)
        }
        WindsweptHills => {
            if s >= 118 {
                (ids.stone, ids.stone, 0)
            } else {
                (ids.grass, ids.dirt, dirt_depth)
            }
        }
        SnowyPlains | SnowyTaiga => (ids.grass_snowy, ids.dirt, dirt_depth),
        Desert => (ids.sand, ids.sandstone, 4),
        Beach | SnowyBeach => (ids.sand, ids.sandstone, 3),
        Ocean | DeepOcean | ColdOcean | FrozenOcean | River | FrozenRiver => {
            if r < 0.33 {
                (ids.gravel, ids.gravel, 3)
            } else {
                (ids.sand, ids.sand, 3)
            }
        }
        StonyPeaks => {
            if s >= 185 {
                (ids.snow_block, ids.stone, 0)
            } else {
                (ids.stone, ids.stone, 0)
            }
        }
        SnowySlopes => (ids.snow_block, ids.stone, 2),
    }
}

/// Writes bedrock, rock, surface layers, water/ice and the snow layer of one column.
#[allow(clippy::too_many_arguments)]
pub fn fill_column(grid: &mut Grid, ids: &Ids, seed: u64, lx: i32, lz: i32, wx: i32, wz: i32, col: Column) {
    let s = col.surface;
    let (top, filler, depth) = surface_rule(ids, seed, col, wx, wz);
    let top_y = s.min(MAX_Y);
    for y in MIN_Y..=top_y {
        let st = if y == MIN_Y {
            ids.bedrock
        } else if y <= -60
            && unit(hash(seed, wx as i64, y as i64, wz as i64, SALT_BED)) < 0.2 + 0.2 * (-60 - y) as f32
        {
            ids.bedrock
        } else if y == s {
            top
        } else if y >= s - depth {
            filler
        } else if y >= 8 {
            ids.stone
        } else if (y as f32) < 8.0 * unit(hash(seed, wx as i64, y as i64, wz as i64, SALT_DEEP)) {
            ids.deepslate
        } else {
            ids.stone
        };
        grid.set(lx, y, lz, st);
    }
    if s < SEA_LEVEL {
        let frozen = col.biome.is_frozen_water();
        for y in (s + 1)..=SEA_LEVEL {
            let st = if y == SEA_LEVEL && frozen { ids.ice } else { ids.water };
            grid.set(lx, y, lz, st);
        }
    } else if col.biome.is_snowy() && s < MAX_Y {
        grid.set(lx, s + 1, lz, ids.snow_layer);
    }
}

#[inline]
fn tri(nodes: &[[f32; 3]], i: usize, j: usize, k: usize, fx: f32, fy: f32, fz: f32) -> [f32; 3] {
    let at = |di: usize, dj: usize, dk: usize| nodes[((k + dk) * 5 + (j + dj)) * 5 + (i + di)];
    let l = |a: [f32; 3], b: [f32; 3], t: f32| [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)];
    let p00 = l(at(0, 0, 0), at(1, 0, 0), fx);
    let p10 = l(at(0, 1, 0), at(1, 1, 0), fx);
    let p01 = l(at(0, 0, 1), at(1, 0, 1), fx);
    let p11 = l(at(0, 1, 1), at(1, 1, 1), fx);
    let q0 = l(p00, p10, fz);
    let q1 = l(p01, p11, fz);
    l(q0, q1, fy)
}

/// Carves cheese and tunnel caves. `ext` holds the surfaces of the chunk columns
/// extended by one column on every side (18 x 18, index `(lz + 1) * 18 + (lx + 1)`).
/// A cave cell must lie at least 5 blocks below the lowest surface around it, so
/// caves never open into the air or the sea next to them.
pub fn carve(n: &Noises, grid: &mut Grid, ids: &Ids, ox: i32, oz: i32, ext: &[i32]) {
    let mut ceil = [i32::MIN; 256];
    let mut top_y = i32::MIN;
    for lz in 0..16usize {
        for lx in 0..16usize {
            let mut lo = i32::MAX;
            for dz in 0..3usize {
                for dx in 0..3usize {
                    lo = lo.min(ext[(lz + dz) * 18 + lx + dx]);
                }
            }
            let c = lo - 5;
            ceil[lz * 16 + lx] = c;
            top_y = top_y.max(c);
        }
    }
    if top_y < CAVE_MIN_Y {
        return;
    }
    let ky0 = CAVE_MIN_Y.div_euclid(8);
    let ky1 = top_y.div_euclid(8) + 1;
    let nyk = (ky1 - ky0 + 1) as usize;
    let mut nodes = vec![[0f32; 3]; 25 * nyk];
    for k in 0..nyk {
        let wy = ((ky0 + k as i32) * 8) as f32;
        for j in 0..5 {
            for i in 0..5 {
                let wx = (ox + 4 * i as i32) as f32;
                let wz = (oz + 4 * j as i32) as f32;
                nodes[(k * 5 + j) * 5 + i] = [n.cheese.get(wx, wy, wz), n.tun_a.get(wx, wy, wz), n.tun_b.get(wx, wy, wz)];
            }
        }
    }
    for lz in 0..16i32 {
        for lx in 0..16i32 {
            let cmax = ceil[(lz * 16 + lx) as usize];
            if cmax < CAVE_MIN_Y {
                continue;
            }
            let (i, fx) = ((lx >> 2) as usize, (lx & 3) as f32 * 0.25);
            let (j, fz) = ((lz >> 2) as usize, (lz & 3) as f32 * 0.25);
            for y in CAVE_MIN_Y..=cmax {
                let kk = y.div_euclid(8);
                let k = (kk - ky0) as usize;
                let fy = (y - kk * 8) as f32 * 0.125;
                let v = tri(&nodes, i, j, k, fx, fy, fz);
                if v[0] > CHEESE_T || (v[1].abs() < TUNNEL_T && v[2].abs() < TUNNEL_T) {
                    let st = if y <= LAVA_MAX_Y { ids.lava } else { ids.air };
                    grid.set(lx, y, lz, st);
                }
            }
        }
    }
}
