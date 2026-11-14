//! `SimpleBlockFeature` (`minecraft:simple_block`): one block state from a provider at the origin, when the state
//! can survive there. Double plants place both halves (`DoublePlantBlock.placeAt`).
//!
//! `schedule_tick` is accepted and ignored: `level.scheduleTick` only queues a tick and changes no block and draws
//! no random numbers, so the generated blocks are the same. `MossyCarpetBlock` (`placeAt` draws from the level
//! random) is not ported: its blocks are not in `ported_features.json`, so no compiled feature places one.

use super::region::Region;
use super::rng::WorldgenRng;
use super::state_provider::StateProvider;
use super::vegetation::{self, Survival};

pub struct SimpleBlock {
    pub to_place: StateProvider,
}

impl SimpleBlock {
    /// `SimpleBlockFeature.place`. Draws the provider's random numbers first, then tests survival.
    pub fn place(&self, region: &mut Region, rng: &mut WorldgenRng, pos: (i32, i32, i32)) -> bool {
        let (x, y, z) = pos;
        let Some(s) = self.to_place.get_optional(region, rng, x, y, z) else {
            return false;
        };
        if !vegetation::can_survive(region, s, x, y, z) {
            return false;
        }
        let tags = region.tags;
        if tags.veg.kind_of_block(tags.block_of(s)) == Survival::DoublePlant {
            // DoublePlantBlock placement: the block above must be air, or the same fluid and replaceable.
            let above = region.get_block(x, y + 1, z);
            if !tags.is_air(above)
                && (tags.fluid(above) != tags.fluid(s) || !vegetation::above_replaceable(tags, above))
            {
                return false;
            }
            let lower = vegetation::lower_state(tags, s);
            let upper = vegetation::upper_state(tags, s);
            region.set_block(x, y, z, lower);
            region.set_block(x, y + 1, z, upper);
        } else {
            region.set_block(x, y, z, s);
        }
        true
    }
}
