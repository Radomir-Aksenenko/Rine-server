use super::placement::{sample_int, IntProvider};
use super::predicate::BlockPredicate;
use super::region::{Pos, Region};
use super::rng::WorldgenRng;
use super::shape::DIRECTIONS;
use super::state_provider::StateProvider;
use super::tree::order::JavaSet;
use super::Decorator;

pub(super) struct VegetationPatch {
    pub ground: StateProvider,
    pub replaceable: BlockPredicate,
    pub vegetation: usize,
    pub inward: i32,
    pub depth: IntProvider,
    pub extra_bottom: f32,
    pub vertical_range: i32,
    pub vegetation_chance: f32,
    pub radius: IntProvider,
    pub extra_edge: f32,
    pub water: Option<u16>,
}

impl VegetationPatch {
    /// VegetationPatchFeature.place and WaterloggedVegetationPatchFeature.placeGroundPatch.
    pub fn place(&self, decorator: &Decorator, region: &mut Region, rng: &mut WorldgenRng, (x, y, z): Pos) -> bool {
        let xr = sample_int(&self.radius, rng) + 1;
        let zr = sample_int(&self.radius, rng) + 1;
        let mut surface = JavaSet::new();
        let inward = self.inward;
        let tags = region.tags;
        for dx in -xr..=xr {
            let x_edge = dx.abs() == xr;
            for dz in -zr..=zr {
                let z_edge = dz.abs() == zr;
                if x_edge && z_edge {
                    continue;
                }
                if (x_edge || z_edge) && (self.extra_edge == 0.0 || rng.next_float() > self.extra_edge) {
                    continue;
                }
                let (px, pz) = (x + dx, z + dz);
                let mut py = y;
                for _ in 0..self.vertical_range {
                    if !region.is_air(px, py, pz) {
                        break;
                    }
                    py += inward;
                }
                for _ in 0..self.vertical_range {
                    if region.is_air(px, py, pz) {
                        break;
                    }
                    py -= inward;
                }
                if !region.is_air(px, py, pz) {
                    continue;
                }
                let ground_y = py + inward;
                let face = usize::from(inward < 0);
                if !tags.sturdy(region.get_block(px, ground_y, pz), face) {
                    continue;
                }
                let depth = sample_int(&self.depth, rng)
                    + i32::from(self.extra_bottom > 0.0 && rng.next_float() < self.extra_bottom);
                if self.place_ground(region, rng, (px, ground_y, pz), depth) {
                    surface.insert((px, ground_y, pz));
                }
            }
        }
        if let Some(water) = self.water {
            let mut enclosed = JavaSet::new();
            for (x, y, z) in surface.iter() {
                if [2, 5, 3, 4, 0].into_iter().all(|d| {
                    let (dx, dy, dz) = DIRECTIONS[d];
                    tags.sturdy(region.get_block(x + dx, y + dy, z + dz), d ^ 1)
                }) {
                    enclosed.insert((x, y, z));
                }
            }
            surface = enclosed;
            for pos in surface.iter() {
                region.write(pos, water, 2);
            }
        }
        for (x, y, z) in surface.iter() {
            if self.vegetation_chance > 0.0 && rng.next_float() < self.vegetation_chance {
                let dy = if self.water.is_some() { -1 } else { 0 };
                if decorator.run_placed(self.vegetation, region, rng, (x, y + dy - inward, z), None)
                    && self.water.is_some()
                {
                    let state = region.get_block(x, y, z);
                    if let Some(wet) = decorator.waterlogged.get(state as usize).copied().flatten() {
                        region.write((x, y, z), wet, 2);
                    }
                }
            }
        }
        !surface.is_empty()
    }

    fn place_ground(&self, region: &mut Region, rng: &mut WorldgenRng, (x, mut y, z): Pos, depth: i32) -> bool {
        let tags = region.tags;
        for i in 0..depth {
            let state = self.ground.get(region, rng, x, y, z);
            if tags.block_of(state) == tags.block_of(region.get_block(x, y, z)) {
                continue;
            }
            if !self.replaceable.test(region, x, y, z) {
                return i != 0;
            }
            region.write((x, y, z), state, 2);
            y += self.inward;
        }
        true
    }
}
