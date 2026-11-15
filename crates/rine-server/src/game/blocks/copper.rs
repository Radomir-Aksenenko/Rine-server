//! Copper weathering: `ChangeOverTimeBlock.changeOverTime` and
//! `getNextState` for the `WeatheringCopper` blocks of `NEXT_BY_BLOCK`
//! (copper block, cut copper, chiseled copper, cut copper slab and stairs,
//! copper door, trapdoor, bars, grate, bulb, lantern, chest, golem statue,
//! lightning rod, copper chain).
//!
//! Random tick only. Waxed copper is not a `ChangeOverTimeBlock`, so it has
//! no `Beh::Copper` and never changes. Oxidized copper has no next block and
//! only rolls.

use super::props::{getv, vid, Pr};
use super::{block_of, Beh, Rng};
use crate::game::redstone::engine::{self, fl, Lvl, UPDATE_LIMIT};
use crate::game::redstone::Rs;
use rine_data::blocks::BlockInfo;
use rine_protocol::BlockPos;
use std::sync::OnceLock;

/// The 15 `WeatheringCopper` families of vanilla `NEXT_BY_BLOCK`, each as
/// (unaffected, exposed, weathered, oxidized) block names. Names come from
/// `BlockItemIds` (`createSimpleCopper`, and the `copper_block` special case).
const FAMILIES: [[&str; 4]; 15] = [
    ["copper_block", "exposed_copper", "weathered_copper", "oxidized_copper"],
    ["cut_copper", "exposed_cut_copper", "weathered_cut_copper", "oxidized_cut_copper"],
    ["chiseled_copper", "exposed_chiseled_copper", "weathered_chiseled_copper", "oxidized_chiseled_copper"],
    ["cut_copper_slab", "exposed_cut_copper_slab", "weathered_cut_copper_slab", "oxidized_cut_copper_slab"],
    ["cut_copper_stairs", "exposed_cut_copper_stairs", "weathered_cut_copper_stairs", "oxidized_cut_copper_stairs"],
    ["copper_door", "exposed_copper_door", "weathered_copper_door", "oxidized_copper_door"],
    ["copper_trapdoor", "exposed_copper_trapdoor", "weathered_copper_trapdoor", "oxidized_copper_trapdoor"],
    ["copper_bars", "exposed_copper_bars", "weathered_copper_bars", "oxidized_copper_bars"],
    ["copper_grate", "exposed_copper_grate", "weathered_copper_grate", "oxidized_copper_grate"],
    ["copper_bulb", "exposed_copper_bulb", "weathered_copper_bulb", "oxidized_copper_bulb"],
    ["copper_lantern", "exposed_copper_lantern", "weathered_copper_lantern", "oxidized_copper_lantern"],
    ["copper_chest", "exposed_copper_chest", "weathered_copper_chest", "oxidized_copper_chest"],
    ["copper_golem_statue", "exposed_copper_golem_statue", "weathered_copper_golem_statue", "oxidized_copper_golem_statue"],
    ["lightning_rod", "exposed_lightning_rod", "weathered_lightning_rod", "oxidized_lightning_rod"],
    ["copper_chain", "exposed_copper_chain", "weathered_copper_chain", "oxidized_copper_chain"],
];

const NONE16: u16 = u16::MAX;
const NOT_WEATHERING: u8 = u8::MAX;

/// Extra `randomTick` condition of a weathering class.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Gate {
    None,
    /// `WeatheringCopperDoorBlock.randomTick`: only the lower half.
    DoorLower,
    /// `WeatheringCopperChestBlock.randomTick`: not the right half, and no
    /// container open (`getEntitiesWithContainerOpen().isEmpty()`).
    ChestNotRight,
}

struct T {
    /// Per block id: `WeatherState` ordinal (0 unaffected .. 3 oxidized).
    age: Vec<u8>,
    /// Per block id: `getNext(Block)`, NONE16 if none.
    next: Vec<u16>,
    /// Per block id: the `randomTick` condition of the class.
    gate: Vec<Gate>,
    /// Per state id: `getNext(BlockState)` = next block's default state with
    /// `withPropertiesOf(state)`, NONE16 if none.
    next_state: Vec<u16>,
    v_lower: u8,
    v_right: u8,
}

fn t() -> &'static T {
    static T_: OnceLock<T> = OnceLock::new();
    T_.get_or_init(build)
}

fn build() -> T {
    let d = rine_data::get();
    let b = &d.blocks;
    let nb = b.blocks.len();
    let mut t = T {
        age: vec![NOT_WEATHERING; nb],
        next: vec![NONE16; nb],
        gate: vec![Gate::None; nb],
        next_state: vec![NONE16; b.states.len()],
        v_lower: vid("lower"),
        v_right: vid("right"),
    };
    for names in FAMILIES.iter() {
        let infos: Vec<&BlockInfo> = names.iter().filter_map(|n| b.block(n)).collect();
        // A missing name is reported by `families_exist_in_data`.
        if infos.len() != 4 {
            continue;
        }
        let gate = match names[0] {
            "copper_door" => Gate::DoorLower,
            "copper_chest" => Gate::ChestNotRight,
            _ => Gate::None,
        };
        for a in 0..4 {
            let id = infos[a].id as usize;
            t.age[id] = a as u8;
            t.next[id] = if a < 3 { infos[a + 1].id as u16 } else { NONE16 };
            t.gate[id] = gate;
        }
        // `withPropertiesOf`: start from the next default, copy each property
        // of the source that the target also has (`trySetValue`).
        for a in 0..3 {
            let (src, dst) = (infos[a], infos[a + 1]);
            for s in src.min_state..src.min_state + src.state_count {
                let mut n = dst.default_state as u16;
                for p in &src.props {
                    let Some(v) = b.get_prop(s as u16, &p.name) else { continue };
                    if let Some(m) = b.with_prop(n, &p.name, v) {
                        n = m;
                    }
                }
                t.next_state[s as usize] = n;
            }
        }
    }
    t
}

/// Family name check: the 60 weathering blocks, nothing else.
pub fn classify(n: &str) -> Option<Beh> {
    FAMILIES.iter().any(|f| f.contains(&n)).then_some(Beh::Copper)
}

/// `BlockState.randomTick` of the weathering classes: the class gate, then
/// `ChangeOverTimeBlock.changeOverTime`.
pub fn random_tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    let t = t();
    match t.gate[block_of(st) as usize] {
        Gate::None => {}
        Gate::DoorLower => {
            if getv(st, Pr::Half) != t.v_lower {
                return;
            }
        }
        Gate::ChestNotRight => {
            if getv(st, Pr::Type) == t.v_right || l.chest_viewers(pos) > 0 {
                return;
            }
        }
    }
    change_over_time(rs, l, st, pos, rng);
}

/// `ChangeOverTimeBlock.changeOverTime`: a 0.05688889 roll per tick, then
/// `level.setBlockAndUpdate` (`handleBlockFormEvent` with `UPDATE_ALL`).
fn change_over_time<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    if rng.next_float() >= 0.05688889f32 {
        return;
    }
    let t = t();
    if let Some(next) = get_next_state(t, l, st, pos, rng) {
        engine::set_block(rs, l, pos, next, fl::ALL, UPDATE_LIMIT);
    }
}

/// `ChangeOverTimeBlock.getNextState`. A younger neighbour within Manhattan
/// distance 4 aborts (no roll). Otherwise the chance is
/// `(older+1)/(older+same+1)`, squared, times the modifier, and then the
/// neighbour counts are the only input.
fn get_next_state<L: Lvl>(t: &T, l: &L, st: u16, pos: BlockPos, rng: &mut Rng) -> Option<u16> {
    let blk = block_of(st) as usize;
    let own = t.age[blk];
    let (mut same, mut older) = (0i32, 0i32);
    // `level.findBlocksInManhattan(pos, 4)` without pos: |dx|+|dy|+|dz| <= 4.
    for dx in -4..=4i32 {
        for dy in -4..=4i32 {
            let rem = 4 - dx.abs() - dy.abs();
            if rem < 0 {
                continue;
            }
            for dz in -rem..=rem {
                if dx == 0 && dy == 0 && dz == 0 {
                    continue;
                }
                let a = t.age[block_of(l.get(pos.offset(dx, dy, dz))) as usize];
                if a == NOT_WEATHERING {
                    continue;
                }
                if a < own {
                    return None;
                }
                if a > own {
                    older += 1;
                } else {
                    same += 1;
                }
            }
        }
    }
    let actual = roll_chance(own, older, same);
    if rng.next_float() < actual && t.next[blk] != NONE16 {
        Some(t.next_state[st as usize]).filter(|&s| s != NONE16)
    } else {
        None
    }
}

/// `chance * chance * getChanceModifier()` with the vanilla float math.
/// Copper has 0.75 when unaffected and 1.0 otherwise.
#[inline]
fn roll_chance(own: u8, older: i32, same: i32) -> f32 {
    let chance = (older + 1) as f32 / (older + same + 1) as f32;
    let modifier = if own == 0 { 0.75f32 } else { 1.0f32 };
    chance * chance * modifier
}

#[cfg(test)]
mod tests {
    use super::super::props::{getb, setb, setv};
    use super::super::testw::World as TW;
    use super::super::{block_id, default_state};
    use super::*;

    fn s(n: &str) -> u16 {
        default_state(n)
    }

    fn name_of(st: u16) -> String {
        rine_data::get().blocks.block_of(st).name.trim_start_matches("minecraft:").to_string()
    }

    #[test]
    fn families_exist_in_data() {
        rine_data::init();
        let b = &rine_data::get().blocks;
        assert_eq!(FAMILIES.len(), 15);
        for names in FAMILIES.iter() {
            for n in names.iter() {
                assert!(b.block(n).is_some(), "missing {n}");
                assert_eq!(classify(n), Some(Beh::Copper));
            }
        }
    }

    #[test]
    fn classify_claims_only_weathering_copper() {
        assert_eq!(classify("waxed_copper_block"), None);
        assert_eq!(classify("waxed_exposed_copper_door"), None);
        assert_eq!(classify("exposed_copper_block"), None);
        assert_eq!(classify("copper"), None);
        assert_eq!(classify("copper_ore"), None);
        assert_eq!(classify("copper_block"), Some(Beh::Copper));
        assert_eq!(classify("oxidized_lightning_rod"), Some(Beh::Copper));
    }

    #[test]
    fn next_block_and_age_follow_vanilla_progression() {
        rine_data::init();
        let t = t();
        let b = &rine_data::get().blocks;
        for names in FAMILIES.iter() {
            for a in 0..4 {
                let id = b.block(names[a]).unwrap().id as usize;
                assert_eq!(t.age[id], a as u8);
                let want = if a < 3 { block_id(names[a + 1]) } else { NONE16 };
                assert_eq!(t.next[id], want, "{}", names[a]);
            }
        }
    }

    #[test]
    fn state_copy_is_exact_for_every_pair() {
        rine_data::init();
        let t = t();
        let b = &rine_data::get().blocks;
        let mut checked = 0;
        for names in FAMILIES.iter() {
            for a in 0..4 {
                let src = b.block(names[a]).unwrap();
                for st in src.min_state..src.min_state + src.state_count {
                    let st = st as u16;
                    let n = t.next_state[st as usize];
                    if a == 3 {
                        assert_eq!(n, NONE16, "{}", names[a]);
                        continue;
                    }
                    let dst = b.block(names[a + 1]).unwrap();
                    assert_eq!(block_of(n) as u32, dst.id, "{} -> {}", names[a], names[a + 1]);
                    // every property the two blocks share keeps its value
                    for p in &src.props {
                        assert_eq!(b.get_prop(n, &p.name), b.get_prop(st, &p.name), "{} {}", names[a], p.name);
                    }
                    checked += 1;
                }
            }
        }
        assert!(checked > 500, "{checked}");
    }

    #[test]
    fn roll_chance_matches_vanilla_formula() {
        // unaffected alone: 1 * 0.75
        assert_eq!(roll_chance(0, 0, 0), 0.75);
        // exposed alone: 1 * 1
        assert_eq!(roll_chance(1, 0, 0), 1.0);
        // three same-age neighbours: (1/4)^2
        assert_eq!(roll_chance(1, 0, 3), 1.0 / 16.0);
        // older neighbours only: (3+1)/(3+1) = 1
        assert_eq!(roll_chance(1, 3, 0), 1.0);
        // unaffected with one same: (1/2)^2 * 0.75
        assert_eq!(roll_chance(0, 0, 1), 0.1875);
    }

    #[test]
    fn copper_block_weathers_one_stage_at_a_time_to_oxidized() {
        let mut w = TW::new();
        let p = BlockPos::new(0, 64, 0);
        w.put(p, s("copper_block"));
        let mut rs = Rs::default();
        let mut rng = Rng::new(1);
        let mut seq = vec![block_of(w.get(p))];
        for _ in 0..200_000 {
            let before = w.get(p);
            super::super::random_tick(&mut rs, &mut w, before, p, &mut rng);
            let after = w.get(p);
            if after != before {
                seq.push(block_of(after));
            }
        }
        let want: Vec<u16> = ["copper_block", "exposed_copper", "weathered_copper", "oxidized_copper"].iter().map(|n| block_id(n)).collect();
        assert_eq!(seq, want);
        assert_eq!(name_of(w.get(p)), "oxidized_copper");
    }

    #[test]
    fn neighbour_rolls_use_the_scan_rules() {
        rine_data::init();
        let t = t();
        let mut rng = Rng::new(7);
        let p = BlockPos::new(0, 64, 0);
        let n = 10_000;

        // a younger neighbour at Manhattan distance 4 aborts every time
        let mut w = TW::new();
        w.put(p, s("exposed_copper"));
        w.put(p.offset(3, 1, 0), s("copper_block"));
        let st = w.get(p);
        assert!((0..n).all(|_| get_next_state(t, &w, st, p, &mut rng).is_none()));

        // at distance 5 it does not count: the roll is 1.0 and always succeeds
        let mut w = TW::new();
        w.put(p, s("exposed_copper"));
        w.put(p.offset(3, 1, 1), s("copper_block"));
        let st = w.get(p);
        assert!((0..n).all(|_| get_next_state(t, &w, st, p, &mut rng).is_some()));

        // more oxidized neighbours count as older: chance 1, always succeeds
        let mut w = TW::new();
        w.put(p, s("exposed_copper"));
        w.put(p.offset(1, 0, 0), s("oxidized_copper"));
        let st = w.get(p);
        assert!((0..n).all(|_| get_next_state(t, &w, st, p, &mut rng).is_some()));

        // three same-age neighbours: rate (1/4)^2 = 1/16
        let mut w = TW::new();
        w.put(p, s("exposed_copper"));
        for d in [(1, 0, 0), (0, 0, 1), (-1, 0, 0)] {
            w.put(p.offset(d.0, d.1, d.2), s("exposed_copper"));
        }
        let st = w.get(p);
        let hits = (0..200_000).filter(|_| get_next_state(t, &w, st, p, &mut rng).is_some()).count();
        assert!((11_700..13_300).contains(&hits), "{hits}");

        // no neighbours, unaffected: rate 0.75
        let mut w = TW::new();
        w.put(p, s("copper_block"));
        let st = w.get(p);
        let hits = (0..200_000).filter(|_| get_next_state(t, &w, st, p, &mut rng).is_some()).count();
        assert!((148_000..152_000).contains(&hits), "{hits}");
    }

    #[test]
    fn waxed_and_oxidized_copper_never_change() {
        let mut rs = Rs::default();
        let mut rng = Rng::new(3);
        let p = BlockPos::new(0, 64, 0);
        for name in ["waxed_copper_block", "waxed_exposed_copper_door", "oxidized_copper", "oxidized_copper_door"] {
            let mut w = TW::new();
            let st = if name.contains("door") { setv(s(name), Pr::Half, vid("lower")) } else { s(name) };
            w.put(p, st);
            for _ in 0..20_000 {
                let cur = w.get(p);
                super::super::random_tick(&mut rs, &mut w, cur, p, &mut rng);
            }
            assert_eq!(w.get(p), st, "{name}");
        }
    }

    #[test]
    fn door_keeps_facing_half_hinge_and_open_after_weathering() {
        let mut w = TW::new();
        let p = BlockPos::new(0, 64, 0);
        let up = p.offset(0, 1, 0);
        let mut lower = s("copper_door");
        lower = setv(lower, Pr::Half, vid("lower"));
        lower = setv(lower, Pr::Facing, vid("east"));
        lower = setv(lower, Pr::Hinge, vid("right"));
        lower = setb(lower, Pr::Open, true);
        lower = setb(lower, Pr::Powered, false);
        let mut upper = setv(lower, Pr::Half, vid("upper"));
        upper = setb(upper, Pr::Open, true);
        w.put(p, lower);
        w.put(up, upper);
        let mut rs = Rs::default();
        let mut rng = Rng::new(21);
        let mut changed = false;
        for _ in 0..50_000 {
            let cur = w.get(p);
            super::super::random_tick(&mut rs, &mut w, cur, p, &mut rng);
            if block_of(w.get(p)) != block_of(lower) {
                changed = true;
                break;
            }
        }
        assert!(changed, "lower half never weathered");
        let st = w.get(p);
        assert_eq!(name_of(st), "exposed_copper_door");
        assert_eq!(getv(st, Pr::Facing), vid("east"));
        assert_eq!(getv(st, Pr::Half), vid("lower"));
        assert_eq!(getv(st, Pr::Hinge), vid("right"));
        assert!(getb(st, Pr::Open));
        assert!(!getb(st, Pr::Powered));
        // the upper half follows the lower one (`DoorBlock.updateShape`:
        // the neighbour's state with HALF set to this half)
        let synced = setv(st, Pr::Half, vid("upper"));
        assert_eq!(w.get(up), synced);
        // and the upper half never weathers on its own (`randomTick` only
        // for the lower half), so it stays exposed after 20k ticks
        for _ in 0..20_000 {
            let cur = w.get(up);
            super::super::random_tick(&mut rs, &mut w, cur, up, &mut rng);
        }
        assert_eq!(w.get(up), synced);
    }

    #[test]
    fn chest_on_right_half_never_weathers_and_left_does() {
        rine_data::init();
        let mut rs = Rs::default();
        let mut rng = Rng::new(4);
        let p = BlockPos::new(0, 64, 0);
        let right = setv(s("copper_chest"), Pr::Type, vid("right"));
        let mut w = TW::new();
        w.put(p, right);
        for _ in 0..20_000 {
            let cur = w.get(p);
            super::super::random_tick(&mut rs, &mut w, cur, p, &mut rng);
        }
        assert_eq!(w.get(p), right);

        let left = setv(s("copper_chest"), Pr::Type, vid("single"));
        let mut w = TW::new();
        w.put(p, left);
        let mut changed = false;
        for _ in 0..20_000 {
            let cur = w.get(p);
            super::super::random_tick(&mut rs, &mut w, cur, p, &mut rng);
            if block_of(w.get(p)) != block_of(left) {
                changed = true;
                break;
            }
        }
        assert!(changed);
        assert_eq!(name_of(w.get(p)), "exposed_copper_chest");
    }
}
