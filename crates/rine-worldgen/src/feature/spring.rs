//! `SpringFeature` (vanilla 26.3, `levelgen/feature/SpringFeature`): a fluid source in a rock pocket with
//! exactly `rock_count` solid neighbours (of the six faces) and `hole_count` empty ones. No random draws.
//!
//! Not modelled: `level.scheduleTick(origin, fluid, 0)`. The scheduled fluid tick is server state, not
//! block data, so a decoration golden with frozen ticking cannot see it; a live server must schedule it.

use super::predicate::BlockPredicate;
use super::region::Region;
use super::state::Palette;

pub struct Spring {
    /// The legacy block of the fluid state (`FluidState.createLegacyBlock()`).
    pub state: u16,
    pub requires_block_below: bool,
    pub rock_count: i32,
    pub hole_count: i32,
    /// `valid_blocks` as a block set: a predicate over the block at the position, using `matching_blocks`.
    pub valid: BlockPredicate,
}

const FACES: [(i32, i32, i32); 6] = [(-1, 0, 0), (1, 0, 0), (0, 0, -1), (0, 0, 1), (0, -1, 0), (0, 1, 0)];

impl Spring {
    /// Returns whether the source was placed.
    pub fn place(&self, region: &mut Region, origin: (i32, i32, i32)) -> bool {
        let (x, y, z) = origin;
        let valid = |region: &Region, dx: i32, dy: i32, dz: i32| self.valid.test(region, x + dx, y + dy, z + dz);
        if !valid(region, 0, 1, 0) {
            return false;
        }
        if self.requires_block_below && !valid(region, 0, -1, 0) {
            return false;
        }
        if !region.is_air(x, y, z) && !valid(region, 0, 0, 0) {
            return false;
        }
        let mut rock = 0;
        let mut hole = 0;
        // West, east, north, south, below: the same order as the Java checks (only counts are used).
        for &(dx, dy, dz) in &FACES[..4] {
            if valid(region, dx, dy, dz) {
                rock += 1;
            }
        }
        if valid(region, 0, -1, 0) {
            rock += 1;
        }
        for &(dx, dy, dz) in &FACES[..4] {
            if region.is_air(x + dx, y + dy, z + dz) {
                hole += 1;
            }
        }
        if region.is_air(x, y - 1, z) {
            hole += 1;
        }
        if rock == self.rock_count && hole == self.hole_count {
            region.set_block(x, y, z, self.state);
            return true;
        }
        false
    }
}

/// Legacy block state of a fluid state (`FluidState.createLegacyBlock()`). The configs use the source
/// fluids (`Fluids.WATER`, `Fluids.LAVA`) with `falling=true`; `FlowingFluid.getLegacyLevel` returns 0 for
/// any `isSource()` state, and the source classes return true, so the legacy block is the default state.
pub fn legacy_state(fluid_id: &str, pal: &mut Palette) -> u16 {
    let block = match fluid_id {
        "minecraft:water" => "minecraft:water",
        "minecraft:lava" => "minecraft:lava",
        other => panic!("spring fluid {other} is not ported"),
    };
    pal.default_state(block)
}
