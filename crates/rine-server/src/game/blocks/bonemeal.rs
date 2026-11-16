//! Bone meal on the simple vanilla `BonemealableBlock` targets that place no
//! feature: `FlowerBedBlock` (pink petals, wildflowers) and `SeagrassBlock`.
//!
//! Not ported here: tall flowers (`TallFlowerBlock` pops the item), sea pickles,
//! small and big dripleaf (`placeWithRandomHeight`, stem placement), glow lichen
//! (`MultifaceSpreader`), and the feature targets in `sapling.rs`.

use std::sync::OnceLock;

use rine_protocol::BlockPos;

use super::{block_of, block_id, default_state};
use crate::game::redstone::engine::{self, fl, Lvl, UPDATE_LIMIT};
use crate::game::redstone::Rs;

struct T {
    /// `FlowerBedBlock` by block id.
    flower_bed: Vec<bool>,
    seagrass: u16,
    water: u16,
    tall_lower: u16,
    tall_upper: u16,
}

fn t() -> &'static T {
    static T_: OnceLock<T> = OnceLock::new();
    T_.get_or_init(|| {
        let n = rine_data::get().blocks.blocks.len();
        let mut flower_bed = vec![false; n];
        for name in ["pink_petals", "wildflowers"] {
            let id = block_id(name) as usize;
            if id < n {
                flower_bed[id] = true;
            }
        }
        let d = &rine_data::get().blocks;
        T {
            flower_bed,
            seagrass: block_id("seagrass"),
            water: block_id("water"),
            tall_lower: d.parse_state("minecraft:tall_seagrass[half=lower]").unwrap_or_else(|| default_state("tall_seagrass")),
            tall_upper: d.parse_state("minecraft:tall_seagrass[half=upper]").unwrap_or_else(|| default_state("tall_seagrass")),
        }
    })
}

/// Bone meal on a flower bed or seagrass. Returns false when the block is not a valid target.
pub fn bonemeal<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) -> bool {
    let t = t();
    let blk = block_of(st) as usize;
    if t.flower_bed.get(blk).copied().unwrap_or(false) {
        flower_bed(rs, l, st, pos);
        return true;
    }
    if block_of(st) == t.seagrass {
        // `SeagrassBlock.isValidBonemealTarget`: water above
        if block_of(l.get(pos.offset(0, 1, 0))) != t.water {
            return false;
        }
        // `performBonemeal`: tall seagrass, lower then upper, `UPDATE_CLIENTS`
        engine::set_block(rs, l, pos, t.tall_lower, fl::CLIENTS, UPDATE_LIMIT);
        engine::set_block(rs, l, pos.offset(0, 1, 0), t.tall_upper, fl::CLIENTS, UPDATE_LIMIT);
        return true;
    }
    false
}

/// `FlowerBedBlock.performBonemeal`: one more flower up to 4, then the item pops.
fn flower_bed<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    let d = &rine_data::get().blocks;
    let amount = d.get_prop(st, "flower_amount").and_then(|v| v.parse::<i32>().ok()).unwrap_or(1);
    if amount < 4 {
        if let Some(next) = d.with_prop(st, "flower_amount", &(amount + 1).to_string()) {
            engine::set_block(rs, l, pos, next, fl::CLIENTS, UPDATE_LIMIT);
        }
    } else {
        // `popResource(level, pos, new ItemStack(this))`
        l.pop_resource(pos, d.state(st).item, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::super::testw::World;
    use super::*;

    fn at() -> BlockPos {
        BlockPos { x: 0, y: 64, z: 0 }
    }

    #[test]
    fn flower_bed_gains_a_flower_then_pops_one() {
        rine_data::init();
        let d = &rine_data::get().blocks;
        let mut rs = Rs::default();
        let mut w = World::default();
        let p = at();
        let one = d.parse_state("minecraft:pink_petals[facing=north,flower_amount=1]").unwrap();
        w.put(p, one);
        assert!(bonemeal(&mut rs, &mut w, one, p));
        assert_eq!(d.get_prop(w.get(p), "flower_amount"), Some("2"));
        assert!(w.popped.is_empty());

        let four = d.parse_state("minecraft:pink_petals[facing=north,flower_amount=4]").unwrap();
        w.put(p, four);
        assert!(bonemeal(&mut rs, &mut w, four, p));
        assert_eq!(w.get(p), four, "a full bed stays as it is");
        assert_eq!(w.popped.len(), 1);
        assert_eq!(w.popped[0].0, p);
    }

    #[test]
    fn seagrass_needs_water_above_and_becomes_tall_seagrass() {
        rine_data::init();
        let mut rs = Rs::default();
        let mut w = World::default();
        let p = at();
        let sg = default_state("seagrass");
        w.put(p, sg);
        assert!(!bonemeal(&mut rs, &mut w, sg, p), "no water above");
        assert_eq!(w.get(p), sg);

        w.put(p.offset(0, 1, 0), default_state("water"));
        assert!(bonemeal(&mut rs, &mut w, sg, p));
        assert_eq!(w.get(p), t().tall_lower);
        assert_eq!(w.get(p.offset(0, 1, 0)), t().tall_upper);
    }

    #[test]
    fn stone_is_not_a_target() {
        rine_data::init();
        let mut rs = Rs::default();
        let mut w = World::default();
        let p = at();
        let stone = default_state("stone");
        w.put(p, stone);
        assert!(!bonemeal(&mut rs, &mut w, stone, p));
    }
}
