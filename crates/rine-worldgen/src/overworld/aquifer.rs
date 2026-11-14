//! `Aquifer.NoiseBasedAquifer` (vanilla 26.3) and the global fluid picker of `NoiseBasedChunkGenerator`.
//! Only the block result of `computeSubstance` is ported; the scheduled-fluid-update flags do not
//! change any block state.

use std::collections::HashMap;

use crate::density::volume::Vol;
use crate::overworld::blocks::{Blk, AIR};
use crate::overworld::Samplers;
use crate::random::{Positional, Rng};

/// `Aquifer.FluidStatus(fluidLevel, fluidType)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FluidStatus {
    pub level: i32,
    pub state: Blk,
}

impl FluidStatus {
    /// `FluidStatus.at(blockY)`.
    pub fn at(&self, y: i32) -> Blk {
        if y < self.level {
            self.state
        } else {
            AIR
        }
    }
}

/// `NoiseBasedChunkGenerator.createFluidPicker` for the overworld (lava below -54, sea level water).
#[derive(Clone, Copy, Debug)]
pub struct GlobalFluid {
    pub lava: FluidStatus,
    pub sea: FluidStatus,
    pub sea_level: i32,
}

impl GlobalFluid {
    pub fn compute(&self, y: i32) -> FluidStatus {
        if y < (-54).min(self.sea_level) {
            self.lava
        } else {
            self.sea
        }
    }
}

/// Root density functions of the aquifer config (`noise_settings.aquifers`).
#[derive(Clone, Copy, Debug)]
pub struct AquiferRoots {
    pub barrier: u32,
    pub floodedness: u32,
    pub spread: u32,
    pub lava: u32,
    pub exclusion: u32,
    pub surface_level: u32,
}

/// `Aquifer.Config.create` result for one chunk.
pub struct Aquifer<'a> {
    roots: &'a AquiferRoots,
    global: &'a GlobalFluid,
    rng: Positional,
    lava_state: Blk,
    min_grid_x: i32,
    min_grid_y: i32,
    min_grid_z: i32,
    grid_size_x: i32,
    grid_size_z: i32,
    aquifer_cache: Vec<Option<FluidStatus>>,
    location_cache: Vec<Option<(i32, i32, i32)>>,
    surface_cache: HashMap<(i32, i32), i32>,
    skip_sampling_above_y: i32,
}

const SURFACE_SAMPLING_OFFSETS_IN_CHUNKS: [(i32, i32); 13] = [
    (0, 0),
    (-2, -1),
    (-1, -1),
    (0, -1),
    (1, -1),
    (-3, 0),
    (-2, 0),
    (-1, 0),
    (1, 0),
    (-2, 1),
    (-1, 1),
    (0, 1),
    (1, 1),
];

const WAY_BELOW_MIN_Y: i32 = -32512;

#[inline]
fn grid_x(block: i32) -> i32 {
    block >> 4
}
#[inline]
fn from_grid_x(grid: i32, offset: i32) -> i32 {
    (grid << 4) + offset
}
#[inline]
fn grid_y(block: i32) -> i32 {
    block.div_euclid(12)
}
#[inline]
fn from_grid_y(grid: i32, offset: i32) -> i32 {
    grid * 12 + offset
}
#[inline]
fn grid_z(block: i32) -> i32 {
    block >> 4
}
#[inline]
fn from_grid_z(grid: i32, offset: i32) -> i32 {
    (grid << 4) + offset
}
#[inline]
fn adjust_surface_level(preliminary: i32) -> i32 {
    preliminary + 8
}

fn similarity(d1: i32, d2: i32) -> f64 {
    1.0 - (d2.wrapping_sub(d1)) as f64 / 25.0
}

fn clamped_lerp(factor: f64, min: f64, max: f64) -> f64 {
    if factor < 0.0 {
        min
    } else if factor > 1.0 {
        max
    } else {
        min + factor * (max - min)
    }
}

fn clamped_map(value: f64, from_min: f64, from_max: f64, to_min: f64, to_max: f64) -> f64 {
    clamped_lerp((value - from_min) / (from_max - from_min), to_min, to_max)
}

fn map(value: f64, from_min: f64, from_max: f64, to_min: f64, to_max: f64) -> f64 {
    let alpha = (value - from_min) / (from_max - from_min);
    to_min + alpha * (to_max - to_min)
}

impl<'a> Aquifer<'a> {
    /// `NoiseBasedAquifer` constructor. `volume` is the chunk's `NoiseChunk` volume; `rng` is
    /// `randomState.getOrCreateRandomFactory("minecraft:aquifer")`.
    pub(crate) fn new(
        s: &mut Samplers,
        roots: &'a AquiferRoots,
        global: &'a GlobalFluid,
        rng: Positional,
        volume: &Vol,
    ) -> Self {
        let min_grid_x = grid_x(volume.mx - 5);
        let max_grid_x = grid_x(volume.max_block_x() - 5) + 1;
        let grid_size_x = max_grid_x - min_grid_x + 1;
        let min_grid_y = grid_y(volume.my + 1) - 1;
        let max_grid_y = grid_y(volume.max_block_y() + 1) + 1;
        let grid_size_y = max_grid_y - min_grid_y + 1;
        let min_grid_z = grid_z(volume.mz - 5);
        let max_grid_z = grid_z(volume.max_block_z() - 5) + 1;
        let grid_size_z = max_grid_z - min_grid_z + 1;
        let total = (grid_size_x * grid_size_y * grid_size_z) as usize;

        let mut aq = Aquifer {
            roots,
            global,
            rng,
            lava_state: global.lava.state,
            min_grid_x,
            min_grid_y,
            min_grid_z,
            grid_size_x,
            grid_size_z,
            aquifer_cache: vec![None; total],
            location_cache: vec![None; total],
            surface_cache: HashMap::new(),
            skip_sampling_above_y: 0,
        };
        let max_adjusted = adjust_surface_level(aq.max_surface_level(
            s,
            from_grid_x(min_grid_x, 0),
            from_grid_z(min_grid_z, 0),
            from_grid_x(max_grid_x, 9),
            from_grid_z(max_grid_z, 9),
        ));
        let skip_grid_y = grid_y(max_adjusted + 12) + 1;
        aq.skip_sampling_above_y = from_grid_y(skip_grid_y, 11) - 1;
        aq
    }

    fn surface_level(&mut self, s: &mut Samplers, block_x: i32, block_z: i32) -> i32 {
        let qx = (block_x >> 2) << 2;
        let qz = (block_z >> 2) << 2;
        if let Some(&v) = self.surface_cache.get(&(qx, qz)) {
            return v;
        }
        let v = s.value(self.roots.surface_level, qx, 0, qz).floor() as i32;
        self.surface_cache.insert((qx, qz), v);
        v
    }

    fn max_surface_level(&mut self, s: &mut Samplers, min_bx: i32, min_bz: i32, max_bx: i32, max_bz: i32) -> i32 {
        let min_qx = min_bx >> 2;
        let max_qx = max_bx >> 2;
        let min_qz = min_bz >> 2;
        let max_qz = max_bz >> 2;
        let vol = Vol::with_step(
            max_qx - min_qx + 1,
            1,
            max_qz - min_qz + 1,
            min_qx << 2,
            0,
            min_qz << 2,
            4,
            1,
            4,
        );
        let mut buffer = vec![0f32; vol.size()];
        s.volume(self.roots.surface_level, &vol, &mut buffer);
        let mut max_y = i32::MIN;
        for z in 0..vol.sz {
            for x in 0..vol.sx {
                let level = buffer[vol.index(x, 0, z)].floor() as i32;
                self.surface_cache.insert((vol.block_x(x), vol.block_z(z)), level);
                if level > max_y {
                    max_y = level;
                }
            }
        }
        max_y
    }

    fn index(&self, gx: i32, gy: i32, gz: i32) -> usize {
        let x = gx - self.min_grid_x;
        let y = gy - self.min_grid_y;
        let z = gz - self.min_grid_z;
        ((y * self.grid_size_z + z) * self.grid_size_x + x) as usize
    }

    /// `Aquifer.computeSubstance`: `None` means "use the default block".
    pub(crate) fn compute_substance(&mut self, s: &mut Samplers, x: i32, y: i32, z: i32, density: f64) -> Option<Blk> {
        if density > 0.0 {
            return None;
        }
        let global_fluid = self.global.compute(y);
        if y > self.skip_sampling_above_y {
            return Some(global_fluid.at(y));
        }
        if global_fluid.at(y) == self.lava_state {
            return Some(self.lava_state);
        }

        let x_anchor = grid_x(x - 5);
        let y_anchor = grid_y(y + 1);
        let z_anchor = grid_z(z - 5);
        let mut d1 = i32::MAX;
        let mut d2 = i32::MAX;
        let mut d3 = i32::MAX;
        let mut d4 = i32::MAX;
        let mut c1 = 0usize;
        let mut c2 = 0usize;
        let mut c3 = 0usize;
        let mut c4 = 0usize;

        for x1 in 0..=1 {
            for y1 in -1..=1 {
                for z1 in 0..=1 {
                    let gx = x_anchor + x1;
                    let gy = y_anchor + y1;
                    let gz = z_anchor + z1;
                    let index = self.index(gx, gy, gz);
                    let loc = match self.location_cache[index] {
                        Some(l) => l,
                        None => {
                            let mut random = self.rng.at(gx, gy, gz);
                            let lx = from_grid_x(gx, random.next_int_bound(10));
                            let ly = from_grid_y(gy, random.next_int_bound(9));
                            let lz = from_grid_z(gz, random.next_int_bound(10));
                            let l = (lx, ly, lz);
                            self.location_cache[index] = Some(l);
                            l
                        }
                    };
                    let dx = loc.0 - x;
                    let dy = loc.1 - y;
                    let dz = loc.2 - z;
                    let new_distance = dx
                        .wrapping_mul(dx)
                        .wrapping_add(dy.wrapping_mul(dy))
                        .wrapping_add(dz.wrapping_mul(dz));
                    if d1 >= new_distance {
                        c4 = c3;
                        c3 = c2;
                        c2 = c1;
                        c1 = index;
                        d4 = d3;
                        d3 = d2;
                        d2 = d1;
                        d1 = new_distance;
                    } else if d2 >= new_distance {
                        c4 = c3;
                        c3 = c2;
                        c2 = index;
                        d4 = d3;
                        d3 = d2;
                        d2 = new_distance;
                    } else if d3 >= new_distance {
                        c4 = c3;
                        c3 = index;
                        d4 = d3;
                        d3 = new_distance;
                    } else if d4 >= new_distance {
                        c4 = index;
                        d4 = new_distance;
                    }
                }
            }
        }
        let _ = (c4, d4);

        let status1 = self.aquifer_status(s, c1);
        let similarity12 = similarity(d1, d2);
        let fluid_state = status1.at(y);
        if similarity12 <= 0.0 {
            return Some(fluid_state);
        }
        if fluid_state == self.global.sea.state && self.global.compute(y - 1).at(y - 1) == self.lava_state {
            return Some(fluid_state);
        }

        let mut barrier_noise: Option<f64> = None;
        let status2 = self.aquifer_status(s, c2);
        let barrier12 = similarity12 * self.pressure(s, x, y, z, &mut barrier_noise, status1, status2);
        if density + barrier12 > 0.0 {
            return None;
        }

        let status3 = self.aquifer_status(s, c3);
        let similarity13 = similarity(d1, d3);
        if similarity13 > 0.0 {
            let barrier13 = similarity12 * similarity13 * self.pressure(s, x, y, z, &mut barrier_noise, status1, status3);
            if density + barrier13 > 0.0 {
                return None;
            }
        }

        let similarity23 = similarity(d2, d3);
        if similarity23 > 0.0 {
            let barrier23 = similarity12 * similarity23 * self.pressure(s, x, y, z, &mut barrier_noise, status2, status3);
            if density + barrier23 > 0.0 {
                return None;
            }
        }
        Some(fluid_state)
    }

    #[allow(clippy::too_many_arguments)]
    fn pressure(
        &self,
        s: &mut Samplers,
        x: i32,
        y: i32,
        z: i32,
        barrier_noise: &mut Option<f64>,
        s1: FluidStatus,
        s2: FluidStatus,
    ) -> f64 {
        let type1 = s1.at(y);
        let type2 = s2.at(y);
        let water = self.global.sea.state;
        let lava = self.lava_state;
        if (type1 != lava || type2 != water) && (type1 != water || type2 != lava) {
            let fluid_y_diff = (s1.level - s2.level).abs();
            if fluid_y_diff == 0 {
                return 0.0;
            }
            let average_fluid_y = 0.5 * (s1.level + s2.level) as f64;
            let how_far_above_average = y as f64 + 0.5 - average_fluid_y;
            let base_value = fluid_y_diff as f64 / 2.0;
            let distance_from_edge_towards_middle = base_value - how_far_above_average.abs();
            let gradient = if how_far_above_average > 0.0 {
                let center_point = 0.0 + distance_from_edge_towards_middle;
                if center_point > 0.0 {
                    center_point / 1.5
                } else {
                    center_point / 2.5
                }
            } else {
                let center_point = 3.0 + distance_from_edge_towards_middle;
                if center_point > 0.0 {
                    center_point / 3.0
                } else {
                    center_point / 10.0
                }
            };
            let noise_value = if !(gradient < -2.0) && !(gradient > 2.0) {
                match *barrier_noise {
                    Some(v) => v,
                    None => {
                        let v = s.value(self.roots.barrier, x, y, z) as f64;
                        *barrier_noise = Some(v);
                        v
                    }
                }
            } else {
                0.0
            };
            2.0 * (noise_value + gradient)
        } else {
            2.0
        }
    }

    fn aquifer_status(&mut self, s: &mut Samplers, index: usize) -> FluidStatus {
        if let Some(status) = self.aquifer_cache[index] {
            return status;
        }
        let (x, y, z) = self.location_cache[index].expect("location computed before use");
        let status = self.compute_fluid(s, x, y, z);
        self.aquifer_cache[index] = Some(status);
        status
    }

    fn compute_fluid(&mut self, s: &mut Samplers, x: i32, y: i32, z: i32) -> FluidStatus {
        let global_fluid = self.global.compute(y);
        let mut lowest_preliminary_surface = i32::MAX;
        let top_of_cell = y + 12;
        let bottom_of_cell = y - 12;
        let mut surface_at_center_under_global = false;

        for &(ox, oz) in SURFACE_SAMPLING_OFFSETS_IN_CHUNKS.iter() {
            let sample_x = x + (ox << 4);
            let sample_z = z + (oz << 4);
            let surface_level = self.surface_level(s, sample_x, sample_z);
            let adjusted = adjust_surface_level(surface_level);
            let start = ox == 0 && oz == 0;
            if start && bottom_of_cell > adjusted {
                return global_fluid;
            }
            let top_pokes = top_of_cell > adjusted;
            if top_pokes || start {
                let global_at_surface = self.global.compute(adjusted);
                if global_at_surface.at(adjusted) != AIR {
                    if start {
                        surface_at_center_under_global = true;
                    }
                    if top_pokes {
                        return global_at_surface;
                    }
                }
            }
            lowest_preliminary_surface = lowest_preliminary_surface.min(surface_level);
        }

        let fluid_surface_level =
            self.compute_surface_level(s, x, y, z, global_fluid, lowest_preliminary_surface, surface_at_center_under_global);
        let state = self.compute_fluid_type(s, x, y, z, global_fluid, fluid_surface_level);
        FluidStatus {
            level: fluid_surface_level,
            state,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn compute_surface_level(
        &mut self,
        s: &mut Samplers,
        x: i32,
        y: i32,
        z: i32,
        global_fluid: FluidStatus,
        lowest_surface_level: i32,
        surface_at_center_under_global: bool,
    ) -> i32 {
        let partially;
        let fully;
        if s.value(self.roots.exclusion, x, y, z) as f64 > 0.0 {
            partially = -1.0;
            fully = -1.0;
        } else {
            let distance_below_surface = adjust_surface_level(lowest_surface_level) - y;
            let floodedness_factor = if surface_at_center_under_global {
                clamped_map(distance_below_surface as f64, 0.0, 64.0, 1.0, 0.0)
            } else {
                0.0
            };
            let floodedness_noise = (s.value(self.roots.floodedness, x, y, z) as f64).clamp(-1.0, 1.0);
            let fully_flooded_threshold = map(floodedness_factor, 1.0, 0.0, -0.3, 0.8);
            let partially_flooded_threshold = map(floodedness_factor, 1.0, 0.0, -0.8, 0.4);
            partially = floodedness_noise - partially_flooded_threshold;
            fully = floodedness_noise - fully_flooded_threshold;
        }

        if fully > 0.0 {
            global_fluid.level
        } else if partially > 0.0 {
            self.randomized_fluid_surface_level(s, x, y, z, lowest_surface_level)
        } else {
            WAY_BELOW_MIN_Y
        }
    }

    fn randomized_fluid_surface_level(&mut self, s: &mut Samplers, x: i32, y: i32, z: i32, lowest: i32) -> i32 {
        let cell_x = x.div_euclid(16);
        let cell_y = y.div_euclid(40);
        let cell_z = z.div_euclid(16);
        let middle_y = cell_y * 40 + 20;
        let spread = (s.value(self.roots.spread, cell_x, cell_y, cell_z) * 10.0f32) as f64;
        let quantized = ((spread / 3.0).floor() as i32) * 3;
        let target = middle_y + quantized;
        lowest.min(target)
    }

    fn compute_fluid_type(
        &mut self,
        s: &mut Samplers,
        x: i32,
        y: i32,
        z: i32,
        global_fluid: FluidStatus,
        fluid_surface_level: i32,
    ) -> Blk {
        let fluid_type = global_fluid.state;
        if fluid_surface_level <= -10 && fluid_surface_level != WAY_BELOW_MIN_Y && fluid_type != self.lava_state {
            let cell_x = x.div_euclid(64);
            let cell_y = y.div_euclid(40);
            let cell_z = z.div_euclid(64);
            let lava_value = s.value(self.roots.lava, cell_x, cell_y, cell_z) as f64;
            if lava_value.abs() > 0.3 {
                return self.lava_state;
            }
        }
        fluid_type
    }
}
