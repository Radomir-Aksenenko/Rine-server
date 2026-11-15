//! Leaves: `LeavesBlock` and its subclasses (`FallingParticlesLeavesBlock`,
//! `TintedParticleLeavesBlock`, `UntintedParticleLeavesBlock`,
//! `MangroveLeavesBlock`). Distance propagation (`updateShape`, scheduled
//! `tick`, `getStateForPlacement`) and decay (`randomTick`).
//!
//! Not ported: `animateTick` (falling and dripping particles, ambient sounds)
//! is client-side only. The Bukkit `LeavesDecayEvent` in `randomTick` is a
//! plugin hook, not vanilla. Mangrove bonemeal is not in this family.

use super::props::{geti, getb, has, seti, setb, Pr};
use super::{block_of, block_tag, Beh};
use crate::game::redstone::data::{DOWN, EAST, NORTH, SOUTH, UP, WEST};
use crate::game::redstone::engine::{self, fl, prio, Lvl, UPDATE_LIMIT};
use crate::game::redstone::Rs;
use rine_protocol::BlockPos;
use std::sync::OnceLock;

/// `Direction.values()` order: DOWN, UP, NORTH, SOUTH, WEST, EAST.
const ALL_DIRS: [u8; 6] = [DOWN, UP, NORTH, SOUTH, WEST, EAST];

struct T {
    /// `BlockTags.PREVENTS_NEARBY_LEAF_DECAY` (logs, stems, hyphae), by block id.
    prevents_decay: Vec<bool>,
}

fn t() -> &'static T {
    static T_: OnceLock<T> = OnceLock::new();
    T_.get_or_init(|| T { prevents_decay: block_tag("prevents_nearby_leaf_decay") })
}

pub fn classify(n: &str) -> Option<Beh> {
    match n {
        "oak_leaves"
        | "spruce_leaves"
        | "birch_leaves"
        | "jungle_leaves"
        | "acacia_leaves"
        | "cherry_leaves"
        | "dark_oak_leaves"
        | "pale_oak_leaves"
        | "mangrove_leaves"
        | "red_poplar_leaves"
        | "orange_poplar_leaves"
        | "yellow_poplar_leaves"
        | "azalea_leaves"
        | "flowering_azalea_leaves" => Some(Beh::Leaves),
        _ => None,
    }
}

/// `LeavesBlock.getDistanceAt`: 0 next to `#prevents_nearby_leaf_decay`, the
/// distance property when the block has one, otherwise 7.
#[inline]
fn distance_at(st: u16) -> i32 {
    if t().prevents_decay[block_of(st) as usize] {
        0
    } else if has(st, Pr::Distance) {
        geti(st, Pr::Distance)
    } else {
        7
    }
}

/// `LeavesBlock.updateDistance`: one more than the nearest support, capped at 7.
/// Stops early at 1, like the vanilla loop.
fn update_distance<L: Lvl>(l: &L, st: u16, pos: BlockPos) -> u16 {
    let mut d = 7;
    for dir in ALL_DIRS {
        d = d.min(distance_at(l.get(pos.relative(dir))) + 1);
        if d == 1 {
            break;
        }
    }
    seti(st, Pr::Distance, d)
}

/// `LeavesBlock.decaying`.
#[inline]
fn decaying(st: u16) -> bool {
    !getb(st, Pr::Persistent) && geti(st, Pr::Distance) == 7
}

/// `LeavesBlock.updateShape`. Waterlogged leaves schedule their water tick in
/// vanilla (`ticks.scheduleTick(pos, Fluids.WATER, ...)`); Rine runs the
/// water of waterlogged blocks in `fluids.rs`, so that part is not repeated here.
pub fn update_shape<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, nst: u16) -> u16 {
    let dist = distance_at(nst) + 1;
    if dist != 1 || geti(st, Pr::Distance) != dist {
        engine::schedule(rs, l, pos, block_of(st), 1, prio::NORMAL);
    }
    st
}

/// `LeavesBlock.tick` (scheduled): `setBlockAndUpdate(pos, updateDistance(...))`.
pub fn tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    let ns = update_distance(&*l, st, pos);
    engine::set_block(rs, l, pos, ns, fl::ALL, UPDATE_LIMIT);
}

/// `LeavesBlock.randomTick`: a decaying leaf drops its loot and is removed.
/// Only called for states with the random tick flag (distance 7, not persistent).
pub fn random_tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    if decaying(st) {
        engine::drop_resources(l, st, pos);
        engine::remove_block(rs, l, pos, false);
    }
}

/// `LeavesBlock.getStateForPlacement`. `pos` is the target and still holds the
/// replaced block, so its fluid is the one vanilla reads. Waterlogged only for
/// source water (`FluidState.is(Fluids.WATER)` compares the fluid type).
pub fn placement<L: Lvl>(l: &L, st: u16, pos: BlockPos) -> u16 {
    let b = &rine_data::get().blocks;
    let replaced = l.get(pos);
    let source_water = b.has(replaced, rine_data::blocks::flag::WATER) && b.has(replaced, rine_data::blocks::flag::FLUID_SOURCE);
    let s = setb(st, Pr::Persistent, true);
    let s = setb(s, Pr::Waterlogged, source_water);
    update_distance(l, s, pos)
}

#[cfg(test)]
mod tests {
    use super::super::testw::World as TW;
    use super::*;

    fn s(n: &str) -> u16 {
        super::super::default_state(n)
    }

    fn leaf(d: i32, persistent: bool) -> u16 {
        setb(seti(s("oak_leaves"), Pr::Distance, d), Pr::Persistent, persistent)
    }

    fn log() -> u16 {
        s("oak_log")
    }

    #[test]
    fn distance_climbs_from_a_log_over_scheduled_ticks() {
        let mut w = TW::new();
        let mut rs = Rs::default();
        let p0 = BlockPos::new(0, 64, 0);
        for x in 1..=3 {
            w.put(p0.offset(x, 0, 0), leaf(7, false));
        }
        // placing the log notifies the leaves; their ticks run one at a time
        engine::set_block(&mut rs, &mut w, p0, log(), fl::ALL, UPDATE_LIMIT);
        for _ in 0..6 {
            w.tick(&mut rs);
        }
        assert_eq!(geti(w.get(p0.offset(1, 0, 0)), Pr::Distance), 1);
        assert_eq!(geti(w.get(p0.offset(2, 0, 0)), Pr::Distance), 2);
        assert_eq!(geti(w.get(p0.offset(3, 0, 0)), Pr::Distance), 3);
        // nothing left to do: no tick scheduled next to a log
        let p1 = p0.offset(1, 0, 0);
        assert!(!engine::has_scheduled(&rs, p1, block_of(leaf(1, false))));
    }

    #[test]
    fn settled_canopy_stops_scheduling_ticks() {
        let mut w = TW::new();
        let mut rs = Rs::default();
        let p0 = BlockPos::new(0, 64, 0);
        for x in 1..=3 {
            w.put(p0.offset(x, 0, 0), leaf(7, false));
        }
        engine::set_block(&mut rs, &mut w, p0, log(), fl::ALL, UPDATE_LIMIT);
        for _ in 0..10 {
            w.tick(&mut rs);
        }
        // `set_block` with an unchanged state does nothing (vanilla setBlock
        // returns early), so a settled chain sends no more shape updates
        assert_eq!(rs.scheduled_count(), 0);
        for x in 1..=3 {
            assert_eq!(geti(w.get(p0.offset(x, 0, 0)), Pr::Distance), x);
        }
    }

    #[test]
    fn update_shape_schedules_only_when_distance_changes() {
        let mut w = TW::new();
        let mut rs = Rs::default();
        let p = BlockPos::new(0, 64, 0);
        let lf = leaf(1, false);
        w.put(p, lf);
        // log next to a leaf that is already at 1: nothing to do
        let out = update_shape(&mut rs, &mut w, lf, p, log());
        assert_eq!(out, lf);
        assert!(!engine::has_scheduled(&rs, p, block_of(lf)));
        // air next to it: distance would be 7, so a tick is scheduled
        let out = update_shape(&mut rs, &mut w, lf, p, 0);
        assert_eq!(out, lf);
        assert!(engine::has_scheduled(&rs, p, block_of(lf)));
    }

    #[test]
    fn unsupported_leaf_decays_after_log_removal() {
        let mut w = TW::new();
        let mut rs = Rs::default();
        let p0 = BlockPos::new(0, 64, 0);
        let p1 = p0.offset(1, 0, 0);
        w.put(p0, log());
        w.put(p1, leaf(1, false));
        engine::set_block(&mut rs, &mut w, p0, 0, fl::ALL, UPDATE_LIMIT);
        for _ in 0..3 {
            w.tick(&mut rs);
        }
        let st = w.get(p1);
        assert_eq!(geti(st, Pr::Distance), 7);
        random_tick(&mut rs, &mut w, st, p1);
        assert_eq!(w.get(p1), 0);
        assert_eq!(w.dropped, vec![(p1, st)]);
    }

    #[test]
    fn decay_chain_removes_every_leaf_and_drops_each() {
        let mut w = TW::new();
        let mut rs = Rs::default();
        let p0 = BlockPos::new(0, 64, 0);
        w.put(p0, log());
        for x in 1..=3 {
            w.put(p0.offset(x, 0, 0), leaf(7, false));
        }
        engine::set_block(&mut rs, &mut w, p0, 0, fl::ALL, UPDATE_LIMIT);
        for _ in 0..12 {
            w.tick(&mut rs);
        }
        for x in 1..=3 {
            let p = p0.offset(x, 0, 0);
            let st = w.get(p);
            assert_eq!(geti(st, Pr::Distance), 7, "leaf {x}");
            random_tick(&mut rs, &mut w, st, p);
            assert_eq!(w.get(p), 0, "leaf {x}");
        }
        assert_eq!(w.dropped.len(), 3);
    }

    #[test]
    fn persistent_leaves_never_decay() {
        let mut w = TW::new();
        let mut rs = Rs::default();
        let p0 = BlockPos::new(0, 64, 0);
        let p1 = p0.offset(1, 0, 0);
        w.put(p0, log());
        w.put(p1, leaf(1, true));
        engine::set_block(&mut rs, &mut w, p0, 0, fl::ALL, UPDATE_LIMIT);
        for _ in 0..10 {
            w.tick(&mut rs);
        }
        let st = w.get(p1);
        assert_eq!(geti(st, Pr::Distance), 7);
        assert!(getb(st, Pr::Persistent));
        for _ in 0..500 {
            random_tick(&mut rs, &mut w, st, p1);
        }
        assert_eq!(w.get(p1), st);
        assert!(w.dropped.is_empty());
    }

    #[test]
    fn placed_leaves_are_persistent_with_distance_from_neighbours() {
        let mut w = TW::new();
        let p = BlockPos::new(0, 64, 0);
        // nothing around: persistent, distance 7
        let st = placement(&w, s("oak_leaves"), p);
        assert!(getb(st, Pr::Persistent));
        assert_eq!(geti(st, Pr::Distance), 7);
        // next to a log: distance 1
        w.put(p.offset(1, 0, 0), log());
        let st = placement(&w, s("oak_leaves"), p);
        assert_eq!(geti(st, Pr::Distance), 1);
        // next to a distance 1 and a distance 3 leaf: 2
        let mut w = TW::new();
        w.put(p.offset(1, 0, 0), leaf(1, false));
        w.put(p.offset(0, 1, 0), leaf(3, false));
        assert_eq!(geti(placement(&w, s("oak_leaves"), p), Pr::Distance), 2);
        // a chain: a leaf at 2 next to the placed spot gives 3
        let mut w = TW::new();
        w.put(p.offset(0, 0, -1), leaf(2, false));
        assert_eq!(geti(placement(&w, s("oak_leaves"), p), Pr::Distance), 3);
    }

    #[test]
    fn placed_leaves_waterlog_only_into_source_water() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = TW::new();
        w.put(p, s("water"));
        assert!(getb(placement(&w, s("oak_leaves"), p), Pr::Waterlogged));
        // flowing water is a different fluid type
        let mut w = TW::new();
        w.put(p, seti(s("water"), Pr::Level, 3));
        assert!(!getb(placement(&w, s("oak_leaves"), p), Pr::Waterlogged));
        // air
        let w = TW::new();
        assert!(!getb(placement(&w, s("oak_leaves"), p), Pr::Waterlogged));
    }
}
