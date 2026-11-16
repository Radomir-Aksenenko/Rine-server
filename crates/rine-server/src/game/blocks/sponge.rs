//! `SpongeBlock` (absorbs water on place and on every neighbour change) and
//! `WetSpongeBlock`.
//!
//! `removeWaterBreadthFirstSearch` is `BlockPos.breadthFirstTraversal(start,
//! 6, 65, ...)`: water is taken through water, up to 6 steps from the sponge,
//! and the search stops at 65 accepted nodes (the sponge counts). The water is
//! removed while the search visits it, so each removal runs its neighbour
//! updates before the next node is read, as in vanilla.
//!
//! `WetSpongeBlock.onPlace` dries the sponge only where `WATER_EVAPORATES` is
//! set (the ultrawarm dimensions). Rine has only the overworld, so a wet sponge
//! never dries here.

use super::props::{getb, setb, has as has_prop, Pr};
use super::{block_id, block_of, default_state, is_air, Beh};
use crate::game::redstone::data::{step, ALL};
use crate::game::redstone::engine::{self, fl, Lvl, CAT_BLOCKS, UPDATE_LIMIT};
use crate::game::redstone::Rs;
use ahash::AHashSet;
use rine_data::blocks::flag;
use rine_protocol::BlockPos;
use std::collections::VecDeque;
use std::sync::OnceLock;

/// `SpongeBlock.MAX_DEPTH` and `MAX_COUNT`.
const MAX_DEPTH: u32 = 6;
const MAX_COUNT: usize = 65;

struct T {
    wet: u16,
    water: u16,
    bubble: u16,
    air: u16,
    /// Kelp and seagrass: dropped and removed (not `BucketPickup`).
    kelp: [u16; 4],
}

fn t() -> &'static T {
    static T_: OnceLock<T> = OnceLock::new();
    T_.get_or_init(|| T {
        wet: default_state("wet_sponge"),
        water: block_id("water"),
        bubble: block_id("bubble_column"),
        air: default_state("air"),
        kelp: [block_id("kelp"), block_id("kelp_plant"), block_id("seagrass"), block_id("tall_seagrass")],
    })
}

pub fn classify(n: &str) -> Option<Beh> {
    Some(match n {
        "sponge" => Beh::Sponge,
        "wet_sponge" => Beh::WetSponge,
        _ => return None,
    })
}

#[inline]
fn has(st: u16, f: u16) -> bool {
    rine_data::get().blocks.has(st, f)
}

/// One node of `removeWaterBreadthFirstSearch` (not the start). Returns
/// whether the node is accepted, i.e. its water was taken.
fn take_water<L: Lvl>(rs: &mut Rs, l: &mut L, p: BlockPos) -> bool {
    let st = l.get(p);
    if !has(st, flag::WATER) {
        return false; // not water: skipped, its neighbours are not visited
    }
    let t = t();
    let blk = block_of(st);
    if blk == t.bubble {
        // BubbleColumnBlock.pickupBlock: air, UPDATE_ALL_IMMEDIATE
        engine::set_block(rs, l, p, t.air, fl::ALL | fl::IMMEDIATE, UPDATE_LIMIT);
        return true;
    }
    if blk == t.water {
        // LiquidBlock.pickupBlock takes a source with UPDATE_ALL_IMMEDIATE;
        // flowing water falls through to the LiquidBlock branch (UPDATE_ALL)
        let flags = if has(st, flag::FLUID_SOURCE) { fl::ALL | fl::IMMEDIATE } else { fl::ALL };
        engine::set_block(rs, l, p, t.air, flags, UPDATE_LIMIT);
        return true;
    }
    if has_prop(st, Pr::Waterlogged) && getb(st, Pr::Waterlogged) {
        // SimpleWaterloggedBlock.pickupBlock (the can't-survive destroy is not modelled)
        engine::set_block(rs, l, p, setb(st, Pr::Waterlogged, false), fl::ALL, UPDATE_LIMIT);
        return true;
    }
    if t.kelp.contains(&blk) {
        engine::drop_resources(l, st, p);
        engine::set_block(rs, l, p, t.air, fl::ALL, UPDATE_LIMIT);
        return true;
    }
    false
}

/// `removeWaterBreadthFirstSearch`: true when more than the sponge was visited.
fn remove_water<L: Lvl>(rs: &mut Rs, l: &mut L, start: BlockPos) -> bool {
    // no wet neighbour: the search stops at the sponge (count 1)
    let wet_next = ALL.iter().any(|&d| {
        let (dx, dy, dz) = step(d);
        has(l.get(start.offset(dx, dy, dz)), flag::WATER)
    });
    if !wet_next {
        return false;
    }
    // Local buffers: the changes below run neighbour updates, which can reach
    // this sponge again (a nested search), so nothing is shared.
    let mut queue: VecDeque<(BlockPos, u32)> = VecDeque::new();
    let mut seen: AHashSet<BlockPos> = AHashSet::new();
    queue.push_back((start, 0));
    let mut count = 0usize;
    while let Some((p, depth)) = queue.pop_front() {
        if !seen.insert(p) {
            continue;
        }
        let accepted = p == start || take_water(rs, l, p);
        if !accepted {
            continue;
        }
        count += 1;
        if count >= MAX_COUNT {
            break;
        }
        if depth < MAX_DEPTH {
            for &d in &ALL {
                let (dx, dy, dz) = step(d);
                queue.push_back((p.offset(dx, dy, dz), depth + 1));
            }
        }
    }
    count > 1
}

/// `SpongeBlock.tryAbsorbWater`.
pub fn try_absorb<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos) {
    if remove_water(rs, l, pos) {
        engine::set_block(rs, l, pos, t().wet, fl::CLIENTS, UPDATE_LIMIT);
        l.sound("block.sponge.absorb", pos, 1.0, 1.0, CAT_BLOCKS, false);
    }
}

/// `SpongeBlock.onPlace`: only when the old block was not a sponge.
pub fn on_place<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, old: u16) {
    if block_of(old) != block_of(st) {
        try_absorb(rs, l, pos);
    }
}

/// `SpongeBlock.neighborChanged`.
pub fn neighbor_changed<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos) {
    try_absorb(rs, l, pos);
}

#[cfg(test)]
mod tests {
    use super::super::testw::World as TW;
    use super::*;

    fn s(n: &str) -> u16 {
        default_state(n)
    }

    fn p(x: i32, y: i32, z: i32) -> BlockPos {
        BlockPos::new(x, y, z)
    }

    fn wet(w: &TW, pos: BlockPos) -> bool {
        block_of(w.get(pos)) == block_of(t().wet)
    }

    #[test]
    fn distance_limit_is_six_steps_through_water() {
        // sponge above a water row: the row cell k is k + 1 steps away
        let mut rs = Rs::default();
        let mut w = TW::new();
        for x in 0..=10 {
            w.put(p(x, 0, 0), s("water"));
        }
        w.put(p(0, 1, 0), s("sponge"));
        try_absorb(&mut rs, &mut w, p(0, 1, 0));
        for x in 0..=5 {
            assert!(is_air(w.get(p(x, 0, 0))), "x={x} should be taken");
        }
        assert_eq!(w.get(p(6, 0, 0)), s("water"), "7 steps away is out of reach");
        assert!(wet(&w, p(0, 1, 0)));
        assert_eq!(w.sounds.iter().filter(|x| x.0 == "block.sponge.absorb").count(), 1);
    }

    #[test]
    fn small_pool_is_fully_absorbed_when_placed() {
        // placing the sponge into a 5x5 pool (on_place through set_block)
        let mut rs = Rs::default();
        let mut w = TW::new();
        for x in -2..=2 {
            for z in -2..=2 {
                w.put(p(x, 0, z), s("water"));
            }
        }
        engine::set_block(&mut rs, &mut w, p(0, 1, 0), s("sponge"), fl::ALL, UPDATE_LIMIT);
        for x in -2..=2 {
            for z in -2..=2 {
                assert!(is_air(w.get(p(x, 0, z))), "({x},{z})");
            }
        }
        assert!(wet(&w, p(0, 1, 0)));
    }

    #[test]
    fn dry_neighbours_do_nothing() {
        let mut rs = Rs::default();
        let mut w = TW::new();
        w.put(p(0, 1, 0), s("sponge"));
        try_absorb(&mut rs, &mut w, p(0, 1, 0));
        assert_eq!(block_of(w.get(p(0, 1, 0))), block_id("sponge"));
        assert!(w.sounds.is_empty());
    }

    #[test]
    fn count_limit_stops_the_search() {
        // a 7x7x7 water cube around the sponge. One pass takes 64 water nodes
        // (65 counting the sponge). The first removal updates its neighbours
        // immediately, the sponge is still dry and starts a nested pass, so the
        // total is more than one pass (measured 99 with this engine).
        let mut rs = Rs::default();
        let mut w = TW::new();
        let mut before = 0usize;
        for x in -3..=3 {
            for y in -3..=3 {
                for z in -3..=3 {
                    if (x, y, z) != (0, 0, 0) {
                        w.put(p(x, y, z), s("water"));
                        before += 1;
                    }
                }
            }
        }
        w.put(p(0, 0, 0), s("sponge"));
        try_absorb(&mut rs, &mut w, p(0, 0, 0));
        let taken = before - w.cells.values().filter(|&&st| block_of(st) == t().water).count();
        assert!(taken > MAX_COUNT - 1, "one pass is not enough: taken {taken}");
        assert!(taken <= 2 * (MAX_COUNT - 1), "taken {taken}");
        assert!(wet(&w, p(0, 0, 0)));
    }

    #[test]
    fn wet_sponge_does_not_dry_in_the_overworld() {
        let mut rs = Rs::default();
        let mut w = TW::new();
        w.put(p(0, 0, 0), s("water"));
        engine::set_block(&mut rs, &mut w, p(0, 1, 0), s("wet_sponge"), fl::ALL, UPDATE_LIMIT);
        assert!(wet(&w, p(0, 1, 0)));
        assert!(w.sounds.is_empty());
    }
}
