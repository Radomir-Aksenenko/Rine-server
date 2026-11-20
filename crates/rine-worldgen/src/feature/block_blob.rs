use super::predicate::BlockPredicate;
use super::region::{Pos, Region};
use super::rng::WorldgenRng;

pub struct BlockBlob {
    pub state: u16,
    pub can_place_on: BlockPredicate,
}

impl BlockBlob {
    pub fn place(&self, region: &mut Region, rng: &mut WorldgenRng, (mut x, mut y, mut z): Pos) -> bool {
        let floor = region.min_y() + 3;
        while y > floor && !self.can_place_on.test(region, x, y - 1, z) {
            y -= 1;
        }
        if y <= floor {
            return false;
        }
        for _ in 0..3 {
            let xr = rng.next_int_bound(2);
            let yr = rng.next_int_bound(2);
            let zr = rng.next_int_bound(2);
            // Integer cutoffs for the four possible squared radii.
            let max_distance = [0, 0, 1, 2][(xr + yr + zr) as usize];
            for dz in -zr..=zr {
                for dy in -yr..=yr {
                    for dx in -xr..=xr {
                        if dx * dx + dy * dy + dz * dz <= max_distance {
                            region.write((x + dx, y + dy, z + dz), self.state, 3);
                        }
                    }
                }
            }
            x += rng.next_int_bound(2) - 1;
            y -= rng.next_int_bound(2);
            z += rng.next_int_bound(2) - 1;
        }
        true
    }
}
