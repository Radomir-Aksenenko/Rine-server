//! Falling blocks: `FallingBlock` (sand, red sand, gravel, the three anvils,
//! the dragon egg, every concrete powder) and `BrushableBlock` (suspicious
//! sand and gravel). Also `ConcretePowderBlock` (turns to concrete next to
//! water) and `DragonEggBlock.teleport`.
//!
//! `onPlace` and `updateShape` schedule the tick (`getDelayAfterPlace`: 2,
//! 5 for the dragon egg). The tick starts the fall when the block below is
//! free (`FallingBlock.isFree`) and the block is above the world bottom:
//! `FallingBlockEntity.fall` leaves the fluid behind and spawns the entity.
//! The landing is in `game/entity.rs`, which calls `solidify_after_land`.
//!
//! Not modelled: anvil falling damage (`setHurtsEntities`) and the brushable
//! entity's `disableDrop` (a suspicious block that cannot land still drops),
//! block entity data of the falling block, and the stuck-in-water clip.

use super::props::{setb, has as has_prop, Pr};
use super::{block_id, block_of, block_tag, default_state, is_air, Beh, Rng};
use crate::game::redstone::data::{self, step, DOWN};
use crate::game::redstone::engine::{self, fl, prio, Lvl, UPDATE_LIMIT};
use crate::game::redstone::Rs;
use rine_data::blocks::flag;
use rine_protocol::BlockPos;
use rine_world::{MAX_Y, MIN_Y};
use std::sync::OnceLock;

struct T {
    /// `FallingBlock.isFree` on fire (`#minecraft:fire`), by block id.
    fire: Vec<bool>,
    water: u16,
    lava: u16,
    bubble: u16,
    /// Concrete state of each concrete powder, by block id (`u16::MAX` otherwise).
    concrete: Vec<u16>,
    anvils: [u16; 3],
}

fn t() -> &'static T {
    static T_: OnceLock<T> = OnceLock::new();
    T_.get_or_init(|| {
        let d = rine_data::get();
        let mut concrete = vec![u16::MAX; d.blocks.blocks.len()];
        for b in &d.blocks.blocks {
            let n = b.name.strip_prefix("minecraft:").unwrap_or(&b.name);
            if let Some(base) = n.strip_suffix("_concrete_powder") {
                concrete[b.id as usize] = default_state(&format!("{base}_concrete"));
            }
        }
        T {
            fire: block_tag("fire"),
            water: block_id("water"),
            lava: block_id("lava"),
            bubble: block_id("bubble_column"),
            concrete,
            anvils: [block_id("anvil"), block_id("chipped_anvil"), block_id("damaged_anvil")],
        }
    })
}

pub fn classify(n: &str) -> Option<Beh> {
    Some(match n {
        "sand" | "red_sand" | "gravel" | "anvil" | "chipped_anvil" | "damaged_anvil" => Beh::Falling,
        "dragon_egg" => Beh::DragonEgg,
        "suspicious_sand" | "suspicious_gravel" => Beh::Brushable,
        _ if n.ends_with("_concrete_powder") => Beh::ConcretePowder,
        _ => return None,
    })
}

#[inline]
fn has(st: u16, f: u16) -> bool {
    rine_data::get().blocks.has(st, f)
}

/// `FallingBlock.isFree`: air, fire, a liquid block (`BlockStateBase.liquid`,
/// which is water, lava and bubble columns) or a replaceable block.
pub fn is_free(st: u16) -> bool {
    let t = t();
    let blk = block_of(st);
    is_air(st) || t.fire[blk as usize] || blk == t.water || blk == t.lava || blk == t.bubble || has(st, flag::REPLACEABLE)
}

/// Delay of `getDelayAfterPlace`.
fn delay(st: u16) -> u32 {
    if beh_is_egg(st) {
        5
    } else {
        2
    }
}

fn beh_is_egg(st: u16) -> bool {
    super::beh(st) == Beh::DragonEgg
}

fn concrete_of(st: u16) -> u16 {
    t().concrete[block_of(st) as usize]
}

/// `ConcretePowderBlock.touchesLiquid`. Below counts only when the powder's
/// own position is water (vanilla reads the position before the loop); a side
/// or top neighbour counts when it is water and its face toward the powder is
/// not sturdy.
fn touches_liquid(get: impl Fn(BlockPos) -> u16, pos: BlockPos) -> bool {
    for &d in &data::ALL {
        if d == DOWN && !has(get(pos), flag::WATER) {
            continue;
        }
        let (dx, dy, dz) = step(d);
        let s = get(pos.offset(dx, dy, dz));
        if has(s, flag::WATER) && !data::sturdy(s, data::opposite(d)) {
            return true;
        }
    }
    false
}

/// `ConcretePowderBlock.getStateForPlacement`: the replaced block (still in
/// the level when this runs) and its water decide.
pub fn placement<L: Lvl>(l: &L, st: u16, pos: BlockPos) -> u16 {
    if has(l.get(pos), flag::WATER) || touches_liquid(|p| l.get(p), pos) {
        concrete_of(st)
    } else {
        st
    }
}

/// `ConcretePowderBlock.onLand`, after the powder was placed at `pos`: it
/// becomes concrete when the block it replaced was water or it touches water.
/// None for other blocks. `get` must read the level with the powder in place.
pub fn solidify_after_land(st: u16, replaced: u16, get: impl Fn(BlockPos) -> u16, pos: BlockPos) -> Option<u16> {
    if super::beh(st) != Beh::ConcretePowder {
        return None;
    }
    if has(replaced, flag::WATER) || touches_liquid(get, pos) {
        Some(concrete_of(st))
    } else {
        None
    }
}

/// `cancelDrop` of suspicious sand and gravel: the entity breaks on landing
/// (`BrushableBlock.onBrokenAfterFall`), nothing is placed.
pub fn breaks_on_landing(st: u16) -> bool {
    super::beh(st) == Beh::Brushable
}

/// `Fallable.onLand` sound of a placed block: anvils play `SOUND_ANVIL_LAND`.
pub fn land_event(st: u16) -> Option<i32> {
    if t().anvils.contains(&block_of(st)) {
        Some(1031) // LevelEvent.SOUND_ANVIL_LAND
    } else {
        None
    }
}

/// `FallingBlockEntity.isStuckInWater`: concrete powder whose cell is water
/// stops there. `cell` is the state of the entity's block position.
pub fn stuck_in_water(st: u16, cell: u16) -> bool {
    super::beh(st) == Beh::ConcretePowder && has(cell, flag::WATER)
}

/// `FallingBlockEntity.fall`: the block becomes its fluid (water when
/// waterlogged), the entity takes over the block without waterlogging.
fn fall<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    engine::set_block(rs, l, pos, engine::fluid_legacy(st), fl::ALL, UPDATE_LIMIT);
    let body = if has_prop(st, Pr::Waterlogged) { setb(st, Pr::Waterlogged, false) } else { st };
    if let Some(g) = l.game() {
        crate::game::entity::spawn_falling(g, [pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5], body);
    }
}

/// `FallingBlock.tick` (and `BrushableBlock.tick` without the block entity reset).
pub fn tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    if is_free(l.get(pos.offset(0, -1, 0))) && pos.y >= MIN_Y {
        fall(rs, l, st, pos);
    }
}

/// `onPlace`: `scheduleTick(pos, this, getDelayAfterPlace())`.
pub fn on_place<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    engine::schedule(rs, l, pos, block_of(st), delay(st), prio::NORMAL);
}

/// `updateShape`. A powder touching water turns to concrete (no tick);
/// otherwise the fall is scheduled again.
pub fn update_shape<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) -> u16 {
    if super::beh(st) == Beh::ConcretePowder && touches_liquid(|p| l.get(p), pos) {
        return concrete_of(st);
    }
    engine::schedule(rs, l, pos, block_of(st), delay(st), prio::NORMAL);
    st
}

/// `DragonEggBlock.teleport` (use only; attack is not wired): up to 1000
/// tries in the 16 x 8 x 16 box, the first air cell with a solid block below.
pub fn dragon_teleport<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    for _ in 0..1000 {
        // argument order of the Java expressions: x1, x2, y1, y2, z1, z2
        let dx = rng.next_int(16) - rng.next_int(16);
        let dy = rng.next_int(8) - rng.next_int(8);
        let dz = rng.next_int(16) - rng.next_int(16);
        let test = pos.offset(dx, dy, dz);
        // no world border in Lvl; unloaded cells count as air in vanilla but cannot take the egg
        if l.loaded(test) && is_air(l.get(test)) && !is_air(l.get(test.offset(0, -1, 0))) && (MIN_Y..=MAX_Y).contains(&test.y) {
            // BlockUtil.packDifferenceInPosition(pos, test, 16, 8, 16)
            let packed = (((dx + 16) & 0xFF) << 16) | (((dy + 8) & 0xFF) << 8) | ((dz + 16) & 0xFF);
            l.level_event(2015, pos, packed);
            engine::set_block(rs, l, test, st, fl::CLIENTS, UPDATE_LIMIT);
            engine::remove_block(rs, l, pos, false);
            return;
        }
    }
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

    /// Schedule the fall of the block at `pos` from `set_block` (onPlace) and run ticks.
    fn place(rs: &mut Rs, w: &mut TW, pos: BlockPos, name: &str) {
        engine::set_block(rs, w, pos, s(name), fl::ALL, UPDATE_LIMIT);
    }

    #[test]
    fn sand_placed_in_air_waits_two_ticks_then_falls() {
        let mut rs = Rs::default();
        let mut w = TW::new();
        w.put(p(0, 0, 0), s("stone"));
        place(&mut rs, &mut w, p(0, 5, 0), "sand");
        assert!(engine::has_scheduled(&rs, p(0, 5, 0), block_of(s("sand"))));
        w.tick(&mut rs); // time 1
        assert_eq!(block_of(w.get(p(0, 5, 0))), block_of(s("sand")));
        w.tick(&mut rs); // time 2: due
        assert!(is_air(w.get(p(0, 5, 0))));
    }

    #[test]
    fn sand_stays_on_a_torch_and_on_stone() {
        // torch is not free: a sand block on top of it does not fall
        let mut rs = Rs::default();
        let mut w = TW::new();
        w.put(p(0, 4, 0), s("torch"));
        place(&mut rs, &mut w, p(0, 5, 0), "sand");
        w.tick(&mut rs);
        w.tick(&mut rs);
        assert_eq!(block_of(w.get(p(0, 5, 0))), block_of(s("sand")));
    }

    #[test]
    fn sand_falls_through_water_and_replaces_nothing() {
        // water below is replaceable and liquid: the fall starts
        let mut rs = Rs::default();
        let mut w = TW::new();
        w.put(p(0, 4, 0), s("water"));
        w.put(p(0, 5, 0), s("sand"));
        engine::schedule(&mut rs, &w, p(0, 5, 0), block_of(s("sand")), 2, prio::NORMAL);
        w.tick(&mut rs);
        w.tick(&mut rs);
        assert!(is_air(w.get(p(0, 5, 0))));
    }

    #[test]
    fn breaking_the_block_below_starts_the_column_fall_two_ticks_per_block() {
        // sand column at y 3..=5 on stone at y 2; the stone breaks, the column
        // falls one block per 2 ticks (each block's updateShape reschedules)
        let mut rs = Rs::default();
        let mut w = TW::new();
        w.put(p(0, 2, 0), s("stone"));
        for y in 3..=5 {
            w.put(p(0, y, 0), s("sand"));
        }
        // the support is removed as a player break does: a world change with neighbour updates
        // the world already holds air when external_set runs (World::set_block queues it)
        w.put(p(0, 2, 0), s("air"));
        engine::external_set(&mut rs, &mut w, p(0, 2, 0), s("stone"), s("air"));
        // 2 ticks: the bottom sand's tick (scheduled by updateShape) falls
        assert!(engine::has_scheduled(&rs, p(0, 3, 0), block_of(s("sand"))));
        w.tick(&mut rs);
        w.tick(&mut rs);
        assert!(is_air(w.get(p(0, 3, 0))));
        assert_eq!(block_of(w.get(p(0, 4, 0))), block_of(s("sand")));
    }

    #[test]
    fn concrete_powder_turns_to_concrete_next_to_water() {
        let mut rs = Rs::default();
        // a side neighbour that is water and not sturdy: the powder solidifies on update
        let mut w = TW::new();
        w.put(p(1, 1, 0), s("water"));
        w.put(p(0, 1, 0), s("white_concrete_powder"));
        let st = w.get(p(0, 1, 0));
        let out = update_shape(&mut rs, &mut w, st, p(0, 1, 0));
        assert_eq!(out, s("white_concrete"));
        assert!(!engine::has_scheduled(&rs, p(0, 1, 0), block_of(st)));

        // dry surroundings: the powder keeps its state and schedules the fall
        let mut w = TW::new();
        w.put(p(0, 1, 0), s("white_concrete_powder"));
        let out = update_shape(&mut rs, &mut w, st, p(0, 1, 0));
        assert_eq!(out, st);
        assert!(engine::has_scheduled(&rs, p(0, 1, 0), block_of(st)));
    }

    #[test]
    fn powder_with_water_below_only_counts_when_it_sits_in_water() {
        // DOWN counts only when the powder's own cell is water
        let mut w = TW::new();
        w.put(p(0, 0, 0), s("water"));
        assert!(!touches_liquid(|q| w.get(q), p(0, 1, 0)));
        w.put(p(0, 1, 0), s("water"));
        assert!(touches_liquid(|q| w.get(q), p(0, 1, 0)));
    }

    #[test]
    fn concrete_powder_lands_as_concrete_in_water() {
        let mut w = TW::new();
        w.put(p(0, 0, 0), s("water"));
        // the powder is placed after landing into water: the replaced water decides
        assert_eq!(solidify_after_land(s("black_concrete_powder"), s("water"), |q| w.get(q), p(0, 0, 0)), Some(s("black_concrete")));
        assert_eq!(solidify_after_land(s("black_concrete_powder"), s("air"), |q| w.get(q), p(0, 1, 0)), None);
        assert_eq!(solidify_after_land(s("sand"), s("water"), |q| w.get(q), p(0, 0, 0)), None);
    }

    #[test]
    fn placement_turns_powder_to_concrete_on_water() {
        let mut w = TW::new();
        w.put(p(0, 0, 0), s("water"));
        assert_eq!(placement(&w, s("red_concrete_powder"), p(0, 0, 0)), s("red_concrete"));
        assert_eq!(placement(&w, s("red_concrete_powder"), p(0, 3, 0)), s("red_concrete_powder"));
    }

    #[test]
    fn dragon_egg_teleports_into_an_air_cell_on_a_floor() {
        let mut rs = Rs::default();
        let mut rng = Rng::new(9);
        let mut w = TW::new();
        let pos = p(0, 64, 0);
        w.put(pos, s("dragon_egg"));
        // a stone floor under the egg's box, air elsewhere
        for x in -16..=16 {
            for z in -16..=16 {
                w.put(p(x, 63, z), s("stone"));
            }
        }
        dragon_teleport(&mut rs, &mut w, s("dragon_egg"), pos, &mut rng);
        assert!(is_air(w.get(pos)));
        let eggs: Vec<&BlockPos> = w.cells.iter().filter(|(_, &st)| block_of(st) == block_of(s("dragon_egg"))).map(|(p, _)| p).collect();
        assert_eq!(eggs.len(), 1);
        let e = *eggs[0];
        assert!((e.x - pos.x).abs() < 16 && (e.z - pos.z).abs() < 16 && (e.y - pos.y).abs() < 8);
        assert_eq!(w.events.last().map(|e| e.0), Some(2015));
    }

    #[test]
    fn dragon_egg_stays_when_no_cell_fits() {
        // every cell in the box is solid: no air to go to, the egg does not move
        let mut rs = Rs::default();
        let mut rng = Rng::new(3);
        let mut w = TW::new();
        let pos = p(0, 64, 0);
        for x in -16..=16 {
            for y in -8..=8 {
                for z in -16..=16 {
                    w.put(p(x, 64 + y, z), s("stone"));
                }
            }
        }
        w.put(pos, s("dragon_egg"));
        dragon_teleport(&mut rs, &mut w, s("dragon_egg"), pos, &mut rng);
        assert_eq!(block_of(w.get(pos)), block_of(s("dragon_egg")));
    }
}
