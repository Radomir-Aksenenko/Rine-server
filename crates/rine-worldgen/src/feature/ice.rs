//! `BlueIceFeature` (vanilla 26.3, `levelgen/feature/BlueIceFeature`): a blue ice seed at a water column
//! next to packed ice, grown by 200 random probes. Each probe draws its offsets in the Java order.

use super::region::Region;
use super::rng::WorldgenRng;

pub struct BlueIce {
    pub sea_level: i32,
    /// Default state of `minecraft:blue_ice`.
    pub blue_ice: u16,
    /// Block indices for `BlockState.is(Blocks.X)`.
    pub water_block: u16,
    pub packed_block: u16,
    pub ice_block: u16,
    pub blue_block: u16,
}

const HORIZONTAL_AND_UP: [(i32, i32, i32); 5] = [(0, 1, 0), (0, 0, -1), (0, 0, 1), (-1, 0, 0), (1, 0, 0)];
const ALL_SIX: [(i32, i32, i32); 6] = [(0, -1, 0), (0, 1, 0), (0, 0, -1), (0, 0, 1), (-1, 0, 0), (1, 0, 0)];

impl BlueIce {
    /// Returns whether the feature ran to completion (`false` for the early exits).
    pub fn place(&self, region: &mut Region, rng: &mut WorldgenRng, origin: (i32, i32, i32)) -> bool {
        let (ox, oy, oz) = origin;
        if oy > self.sea_level - 1 {
            return false;
        }
        let t = &region.tags;
        let at_origin_is_water = t.is_block(region.get_block(ox, oy, oz), self.water_block);
        let below_is_water = t.is_block(region.get_block(ox, oy - 1, oz), self.water_block);
        if !at_origin_is_water && !below_is_water {
            return false;
        }
        let found_packed = HORIZONTAL_AND_UP.iter().any(|&(dx, dy, dz)| t.is_block(region.get_block(ox + dx, oy + dy, oz + dz), self.packed_block));
        if !found_packed {
            return false;
        }
        region.set_block(ox, oy, oz, self.blue_ice);

        for _ in 0..200 {
            // `random.nextInt(5) - random.nextInt(6)`, then the xz spread from the y offset.
            let y_off = rng.next_int_bound(5) - rng.next_int_bound(6);
            let mut xz_diff = 3;
            if y_off < 2 {
                xz_diff += y_off / 2;
            }
            if xz_diff >= 1 {
                // Java evaluates the x argument (two draws) before z (two draws).
                let dx = rng.next_int_bound(xz_diff) - rng.next_int_bound(xz_diff);
                let dz = rng.next_int_bound(xz_diff) - rng.next_int_bound(xz_diff);
                let (px, py, pz) = (ox + dx, oy + y_off, oz + dz);
                let state = region.get_block(px, py, pz);
                let t = &region.tags;
                let placeable = region.is_air(px, py, pz)
                    || t.is_block(state, self.water_block)
                    || t.is_block(state, self.packed_block)
                    || t.is_block(state, self.ice_block);
                if placeable {
                    let touches_blue = ALL_SIX.iter().any(|&(ddx, ddy, ddz)| region.tags.is_block(region.get_block(px + ddx, py + ddy, pz + ddz), self.blue_block));
                    if touches_blue {
                        region.set_block(px, py, pz, self.blue_ice);
                    }
                }
            }
        }
        true
    }
}
