use super::{
    region::{Pos, Region},
    rng::WorldgenRng,
    shape::DIRECTIONS,
};

pub(super) struct UnderwaterMagma {
    pub floor_range: i32,
    pub radius: i32,
    pub probability: f32,
    pub water: u16,
    pub magma: u16,
}

impl UnderwaterMagma {
    pub fn place(&self, level: &mut Region, rng: &mut WorldgenRng, (x, y, z): Pos) -> bool {
        let is_water =
            |level: &Region, py| level.tags.is_block(level.get_block(x, py, z), self.water);
        if !is_water(level, y) {
            return false;
        }
        // The ceiling scan has no effect on the floor or RNG.
        let Some(floor) = (1..self.floor_range)
            .map(|d| y - d)
            .find(|&py| !is_water(level, py))
        else {
            return false;
        };
        let r = self.radius;
        let mut placed = false;
        for pz in z - r..=z + r {
            for py in floor - r..=floor + r {
                for px in x - r..=x + r {
                    if rng.next_float() >= self.probability {
                        continue;
                    }
                    let state = level.get_block(px, py, pz);
                    if level.tags.is_air(state)
                        || level.tags.is_block(state, self.water)
                        || !level
                            .tags
                            .face_occluding(level.get_block(px, py - 1, pz), 1)
                    {
                        continue;
                    }
                    if [2, 5, 3, 4].into_iter().all(|face| {
                        let (dx, _, dz) = DIRECTIONS[face];
                        level
                            .tags
                            .face_occluding(level.get_block(px + dx, py, pz + dz), face ^ 1)
                    }) {
                        level.write((px, py, pz), self.magma, 2);
                        placed = true;
                    }
                }
            }
        }
        placed
    }
}
