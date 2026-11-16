//! Growth through configured features that are not saplings: the huge red and brown
//! mushrooms of `MushroomBlock.growMushroom` (bone meal on the mushroom plants).
//!
//! The feature is placed by the sapling bridge (`sapling::place_and_replay`): a copy of the
//! live box, the rine-worldgen feature, then the writes replayed on the live world with
//! `UPDATE_ALL` (`Feature.setBlock`). The compiled decorator has its own `OnceLock`, so a
//! failure here does not disable the saplings.
//!
//! Azalea bushes (`AzaleaBlock`) grow `TreeGrower.AZALEA` (`azalea_tree`) the same way.
//! Not ported: the bone meal of the nylium, fungus and moss targets (their features are not in the bundle yet).

use std::sync::OnceLock;

use rine_protocol::BlockPos;

use super::props::{geti, getb, seti, Pr};
use super::sapling::{self, Growth};
use super::{block_id, block_of, Rng};
use crate::game::redstone::engine::{self, fl, Lvl, UPDATE_LIMIT};
use crate::game::redstone::Rs;

/// Overworld build height (`min_y`, `height`), as in `sapling.rs`.
const MIN_Y: i32 = -64;
const WORLD_HEIGHT: i32 = 384;

const HUGE_RED: &str = "minecraft:huge_red_mushroom";
const HUGE_BROWN: &str = "minecraft:huge_brown_mushroom";

/// `SaplingBlock.isBonemealSuccess`: `nextFloat() < 0.45` (the standing mangrove propagule uses it too).
const SAPLING_BONEMEAL_SUCCESS: f32 = 0.45;

/// `MushroomBlock.isBonemealSuccess`: `nextFloat() < 0.4`.
const MUSHROOM_SUCCESS: f32 = 0.4;

/// `AzaleaBlock.isBonemealSuccess`: `level.getRandom().nextFloat() < 0.45`.
const AZALEA_BONEMEAL_SUCCESS: f32 = 0.45;

struct Mushroom {
    /// Block id of the plant and the index of its feature in the growth decorator.
    block: u16,
    index: usize,
    /// `foliageRadius()`, for the build height check of `isValidBonemealTarget`.
    radius: i32,
}

struct T {
    growth: Option<Growth>,
    /// `MushroomBlock` plants: red, then brown.
    mushrooms: Vec<Mushroom>,
}

fn t() -> &'static T {
    static T_: OnceLock<T> = OnceLock::new();
    T_.get_or_init(|| {
        let Some(growth) = sapling::build(&[HUGE_RED, HUGE_BROWN]) else {
            return T { growth: None, mushrooms: Vec::new() };
        };
        let mut mushrooms = Vec::new();
        for (block, feature) in [("red_mushroom", HUGE_RED), ("brown_mushroom", HUGE_BROWN)] {
            let (Some(index), Some(radius)) = (growth.dec.grower_index(feature), growth.dec.grower_mushroom_radius(feature)) else {
                tracing::error!("block growth: {feature} is not compiled");
                return T { growth: None, mushrooms: Vec::new() };
            };
            mushrooms.push(Mushroom { block: block_id(block), index, radius });
        }
        T { growth: Some(growth), mushrooms }
    })
}

/// The mangrove growth: `TreeGrower.MANGROVE` over the mangrove configured features.
fn mangrove() -> Option<&'static Growth> {
    static M: OnceLock<Option<Growth>> = OnceLock::new();
    M.get_or_init(|| sapling::build(&["minecraft:mangrove", "minecraft:tall_mangrove"])).as_ref()
}

const AZALEA_TREE: &str = "minecraft:azalea_tree";

/// Azalea growth: `TreeGrower.AZALEA` over `azalea_tree`, and the blocks and minimum height it reads.
struct Azalea {
    growth: Growth,
    /// `azalea` and `flowering_azalea` (`AzaleaBlock` subclasses).
    blocks: [u16; 2],
    /// `TreeGrower.getMinimumHeight`: the trunk base height of `azalea_tree`.
    min_height: i32,
}

fn azalea() -> Option<&'static Azalea> {
    static A: OnceLock<Option<Azalea>> = OnceLock::new();
    A.get_or_init(|| {
        let growth = sapling::build(&[AZALEA_TREE])?;
        let min_height = growth.dec.grower_base_height(AZALEA_TREE)?;
        Some(Azalea { growth, blocks: [block_id("azalea"), block_id("flowering_azalea")], min_height })
    })
    .as_ref()
}

/// `AzaleaBlock.isValidBonemealTarget` and `performBonemeal`. Returns false when the bush is not a valid target.
/// `isBonemealSuccess` is `level.getRandom().nextFloat() < 0.45`, the same level random as the growth.
pub fn azalea_bonemeal<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) -> bool {
    let Some(a) = azalea() else { return false };
    if !a.blocks.contains(&block_of(st)) {
        return false;
    }
    // `isInsideBuildHeight(pos.above(minHeight + 2))` and `getFluidState(pos.above()).isEmpty()`.
    let top = pos.y + a.min_height + 2;
    if top < MIN_Y || top >= MIN_Y + WORLD_HEIGHT {
        return false;
    }
    let above = l.get(pos.offset(0, 1, 0));
    if getb(above, Pr::Waterlogged) || is_fluid_block(block_of(above)) {
        return false;
    }
    if rng.next_float() < AZALEA_BONEMEAL_SUCCESS {
        sapling::grow_tree(rs, l, &a.growth, pos, st, &sapling::AZALEA, rng);
    }
    true
}

/// A block whose fluid state is not empty: water or lava (`Block.getFluidState`).
fn is_fluid_block(blk: u16) -> bool {
    static F: OnceLock<[u16; 2]> = OnceLock::new();
    F.get_or_init(|| [block_id("water"), block_id("lava")]).contains(&blk)
}

/// `SaplingBlock.advanceTree` for the standing mangrove propagule: stage 0 becomes stage 1
/// (`UPDATE_NONE`), stage 1 grows `TreeGrower.MANGROVE`.
pub fn propagule_advance<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    if geti(st, Pr::Stage) == 0 {
        engine::set_block(rs, l, pos, seti(st, Pr::Stage, 1), fl::NONE, UPDATE_LIMIT);
        return;
    }
    let Some(gr) = mangrove() else { return };
    sapling::grow_tree(rs, l, gr, pos, st, &sapling::MANGROVE, rng);
}

/// Bone meal on a standing mangrove propagule: `SaplingBlock.isBonemealSuccess`, then `advanceTree`.
/// The hanging stage is in `bonemeal.rs`.
pub fn propagule_bonemeal<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    if rng.next_float() < SAPLING_BONEMEAL_SUCCESS {
        propagule_advance(rs, l, st, pos, rng);
    }
}

/// Bone meal on a mushroom plant (`MushroomBlock.isValidBonemealTarget`, `isBonemealSuccess`,
/// `performBonemeal`). Returns false when it is not a valid target.
pub fn bonemeal<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) -> bool {
    if azalea_bonemeal(rs, l, st, pos, rng) {
        return true;
    }
    let t = t();
    let blk = block_of(st);
    let Some(m) = t.mushrooms.iter().find(|m| m.block == blk) else { return false };
    let Some(gr) = t.growth.as_ref() else { return false };
    // `isInsideBuildHeight(pos.above(4 + foliageRadius))`
    let top = pos.y + 4 + m.radius;
    if top < MIN_Y || top >= MIN_Y + WORLD_HEIGHT {
        return false;
    }
    if rng.next_float() < MUSHROOM_SUCCESS {
        grow_mushroom(rs, l, gr, st, pos, m.index, rng);
    }
    true
}

/// `MushroomBlock.growMushroom`: the plant is removed first, so the feature sees air at `pos`;
/// when the placement fails the plant is put back (`setBlockAndUpdate`).
fn grow_mushroom<L: Lvl>(rs: &mut Rs, l: &mut L, gr: &Growth, st: u16, pos: BlockPos, index: usize, rng: &mut Rng) {
    // `level.removeBlock(pos, false)`
    engine::remove_block(rs, l, pos, false);
    if !sapling::place_and_replay(rs, l, gr, index, pos, rng, fl::ALL) {
        engine::set_block(rs, l, pos, st, fl::ALL, UPDATE_LIMIT);
    }
}

#[cfg(test)]
mod tests {
    use super::super::props::{geti, seti, Pr};
    use super::super::testw::World;
    use super::super::{block_id, default_state};
    use super::*;

    fn count(w: &World, name: &str) -> usize {
        let b = block_id(name);
        w.cells.values().filter(|&&s| block_of(s) == b).count()
    }

    /// Dirt floor under a 9x9 area, so the mushroom has a `canPlaceOn` block everywhere.
    fn floor(w: &mut World) {
        for x in -8..=8 {
            for z in -8..=8 {
                w.put(BlockPos { x, y: 63, z }, default_state("dirt"));
            }
        }
    }

    #[test]
    fn red_mushroom_grows_a_huge_mushroom_on_a_dirt_floor() {
        rine_data::init();
        let p = BlockPos { x: 0, y: 64, z: 0 };
        let red = default_state("red_mushroom");
        let mut grew = 0;
        for seed in 0..24u64 {
            let mut rs = Rs::default();
            let mut w = World::new();
            floor(&mut w);
            w.put(p, red);
            let mut rng = Rng::new(seed);
            assert!(bonemeal(&mut rs, &mut w, red, p, &mut rng));
            if count(&w, "red_mushroom_block") > 0 {
                grew += 1;
                assert!(count(&w, "mushroom_stem") >= 4, "seed {seed}: trunk of the mushroom");
                assert!(count(&w, "red_mushroom_block") >= 9, "seed {seed}: cap");
                // the plant itself was removed and the stem stands on its place
                assert_eq!(block_of(w.get(p)), block_id("mushroom_stem"), "seed {seed}");
            }
        }
        assert!(grew > 0, "some growths succeed in 24 seeds");
    }

    #[test]
    fn brown_mushroom_has_a_flat_cap_and_the_stem_under_it() {
        rine_data::init();
        let p = BlockPos { x: 0, y: 64, z: 0 };
        let brown = default_state("brown_mushroom");
        let mut grew = 0;
        for seed in 0..24u64 {
            let mut rs = Rs::default();
            let mut w = World::new();
            floor(&mut w);
            w.put(p, brown);
            let mut rng = Rng::new(seed);
            assert!(bonemeal(&mut rs, &mut w, brown, p, &mut rng));
            if count(&w, "brown_mushroom_block") > 0 {
                grew += 1;
                assert!(count(&w, "brown_mushroom_block") >= 5, "seed {seed}: cap");
                assert_eq!(block_of(w.get(p)), block_id("mushroom_stem"), "seed {seed}");
            }
        }
        assert!(grew > 0, "some growths succeed in 24 seeds");
    }

    #[test]
    fn standing_propagule_takes_two_random_ticks_to_start_and_then_grows_mangrove() {
        rine_data::init();
        let p = BlockPos { x: 0, y: 64, z: 0 };
        let stage1 = seti(default_state("mangrove_propagule"), Pr::Stage, 1);
        // stage 0 -> 1 with no tree
        let mut w = World::new();
        let mut rs = Rs::default();
        let mut rng = Rng::new(1);
        w.put(p, default_state("mangrove_propagule"));
        propagule_advance(&mut rs, &mut w, default_state("mangrove_propagule"), p, &mut rng);
        assert_eq!(geti(w.get(p), Pr::Stage), 1);
        assert_eq!(count(&w, "mangrove_log"), 0);

        // stage 1 grows the mangrove tree on a mud floor
        let mut logs = 0;
        for seed in 0..16u64 {
            let mut w = World::new();
            let mut rs = Rs::default();
            let mut rng = Rng::new(seed);
            // mud, with stone under it: the roots stop at the stone (`MangroveRootPlacer.simulateRoots`)
            for x in -8..=8 {
                for z in -8..=8 {
                    w.put(BlockPos { x, y: 63, z }, default_state("mud"));
                    w.put(BlockPos { x, y: 62, z }, default_state("stone"));
                }
            }
            w.put(p, stage1);
            propagule_advance(&mut rs, &mut w, stage1, p, &mut rng);
            logs += count(&w, "mangrove_log");
        }
        assert!(logs > 0, "some mangrove trees grow in 16 seeds");
    }

    #[test]
    fn azalea_bone_meal_grows_a_bending_tree_on_dirt() {
        rine_data::init();
        let p = BlockPos { x: 0, y: 64, z: 0 };
        let az = default_state("azalea");
        let mut grew = 0;
        for seed in 0..32u64 {
            let mut rs = Rs::default();
            let mut w = World::new();
            floor(&mut w);
            w.put(p, az);
            let mut rng = Rng::new(seed);
            assert!(bonemeal(&mut rs, &mut w, az, p, &mut rng), "seed {seed}: valid target");
            if count(&w, "oak_log") > 0 {
                grew += 1;
                assert!(count(&w, "azalea_leaves") + count(&w, "flowering_azalea_leaves") > 0, "seed {seed}: leaves");
            } else {
                // a failed growth puts the bush back (`resetSaplings`)
                assert_eq!(block_of(w.get(p)), block_id("azalea"), "seed {seed}");
            }
        }
        assert!(grew > 0, "some azalea trees grow in 32 seeds");
    }

    #[test]
    fn azalea_with_water_above_or_no_height_is_not_a_target() {
        rine_data::init();
        let p = BlockPos { x: 0, y: 64, z: 0 };
        let az = default_state("azalea");
        let mut rs = Rs::default();
        let mut rng = Rng::new(1);

        // `getFluidState(pos.above()).isEmpty()` fails with water above
        let mut w = World::new();
        floor(&mut w);
        w.put(p, az);
        w.put(p.offset(0, 1, 0), default_state("water"));
        assert!(!bonemeal(&mut rs, &mut w, az, p, &mut rng));
        assert_eq!(block_of(w.get(p)), block_id("azalea"));

        // `isInsideBuildHeight(pos.above(minHeight + 2))`: base height 4, so y + 6 must stay below 320
        let top_ok = BlockPos { x: 0, y: 313, z: 0 };
        let top_out = BlockPos { x: 0, y: 314, z: 0 };
        let mut w = World::new();
        w.put(top_ok, az);
        assert!(bonemeal(&mut rs, &mut w, az, top_ok, &mut rng));
        let mut w = World::new();
        w.put(top_out, az);
        assert!(!bonemeal(&mut rs, &mut w, az, top_out, &mut rng));
    }

    #[test]
    fn no_build_height_no_target() {
        rine_data::init();
        let p = BlockPos { x: 0, y: 320, z: 0 };
        let red = default_state("red_mushroom");
        let mut rs = Rs::default();
        let mut w = World::new();
        w.put(p, red);
        let mut rng = Rng::new(1);
        assert!(!bonemeal(&mut rs, &mut w, red, p, &mut rng));
        assert_eq!(w.get(p), red);
    }

    #[test]
    fn stone_under_the_plant_keeps_the_plant_and_the_bone_meal_is_still_used() {
        rine_data::init();
        let p = BlockPos { x: 0, y: 64, z: 0 };
        let red = default_state("red_mushroom");
        // no dirt floor: `canPlaceOn` fails, the feature does nothing, the plant comes back
        let mut any_grow = false;
        for seed in 0..24u64 {
            let mut rs = Rs::default();
            let mut w = World::new();
            w.put(p, red);
            w.put(p.offset(0, -1, 0), default_state("stone"));
            let mut rng = Rng::new(seed);
            // `bonemeal` is a valid target, the success draw decides the growth
            assert!(bonemeal(&mut rs, &mut w, red, p, &mut rng));
            if count(&w, "mushroom_stem") > 0 {
                any_grow = true;
            }
            assert_eq!(w.get(p), red, "seed {seed}: the plant is back");
        }
        assert!(!any_grow, "a stone floor grows nothing");
    }
}
