//! `SimpleBlockFeature` (`minecraft:simple_block`): one block state from a provider at the origin, when the state
//! can survive there. Double plants place both halves (`DoublePlantBlock.placeAt`).

use super::region::Region;
use super::rng::WorldgenRng;
use super::state_provider::StateProvider;
use super::vegetation::{self, Survival};

pub struct SimpleBlock {
    pub to_place: StateProvider,
    pub schedule_tick: bool,
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
        if vegetation::is_double_plant(tags, s) {
            // DoublePlantBlock placement: the block above must be air, or the same fluid and replaceable.
            let above = region.get_block(x, y + 1, z);
            if !tags.is_air(above)
                && (tags.fluid(above) != tags.fluid(s) || !tags.replaceable(above))
            {
                return false;
            }
            let lower = vegetation::copy_waterlogged(tags, vegetation::lower_state(tags, s), region.get_block(x, y, z));
            region.write(pos, lower, 2);
            let upper = vegetation::copy_waterlogged(tags, vegetation::upper_state(tags, s), region.get_block(x, y + 1, z));
            region.write((x, y + 1, z), upper, 2);
        } else if tags.veg.kind_of_block(tags.block_of(s)) == Survival::MossyCarpet {
            if let Some(carpet) = &tags.veg.moss_carpet {
                carpet.place(region, rng, pos);
            }
        } else {
            region.write(pos, s, 2);
        }
        if self.schedule_tick {
            region.schedule(pos, region.get_block(x, y, z));
        }
        true
    }
}
