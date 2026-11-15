//! Grass, mycelium, podzol and nylium.
//!
//! - `SpreadingSnowyBlock` (`GrassBlock`, `MyceliumBlock`): random tick death
//!   under an opaque or liquid top, spread to dirt below/beside, `snowy`.
//! - `SnowyBlock` (`PodzolBlock` in 26.3 is a plain `SnowyBlock`): `snowy`
//!   in `updateShape`, no random tick.
//! - `NyliumBlock` (crimson, warped): netherrack when the top blocks light.
//!
//! The light test is `LightEngine.getLightDampeningInto(state, above, UP,
//! above.getLightDampening())` (26.3 name; the brief calls it
//! `getLightBlockInto`). It is 16 when the block above has an occlusion
//! shape whose bottom face is full, else its light dampening. Rine has no
//! occlusion shapes: the full-face test uses the collision-based
//! `sturdy(above, DOWN)` and only applies to opaque (`canOcclude`) blocks.
//! Full opaque cubes have dampening 15, so the result matches vanilla there.

use super::props::{geti, setb, Pr};
use super::{block_id, block_of, block_tag, default_state, local_brightness, Beh, Rng};
use crate::game::redstone::data::{self, DOWN, UP};
use crate::game::redstone::engine::{self, fl, Lvl, UPDATE_LIMIT};
use crate::game::redstone::Rs;
use rine_data::blocks::flag;
use rine_protocol::BlockPos;
use std::sync::OnceLock;

struct T {
    dirt: u16,
    dirt_default: u16,
    grass_default: u16,
    mycelium: u16,
    mycelium_default: u16,
    netherrack_default: u16,
    snow_layer: u16,
    /// `BlockTags.SNOW` by block id.
    snow_tag: Vec<bool>,
}

fn t() -> &'static T {
    static T_: OnceLock<T> = OnceLock::new();
    T_.get_or_init(|| T {
        dirt: block_id("dirt"),
        dirt_default: default_state("dirt"),
        grass_default: default_state("grass_block"),
        mycelium: block_id("mycelium"),
        mycelium_default: default_state("mycelium"),
        netherrack_default: default_state("netherrack"),
        snow_layer: block_id("snow"),
        snow_tag: block_tag("snow"),
    })
}

pub fn classify(n: &str) -> Option<Beh> {
    Some(match n {
        "grass_block" | "mycelium" => Beh::Spreading,
        "podzol" => Beh::Snowy,
        "crimson_nylium" | "warped_nylium" => Beh::Nylium,
        _ => return None,
    })
}

/// `SnowyBlock.isSnowySetting`: the block above is in `#snow`.
#[inline]
fn is_snowy_setting(st: u16) -> bool {
    t().snow_tag[block_of(st) as usize]
}

/// `LightEngine.getLightDampeningInto(.., UP, above.getLightDampening())`.
#[inline]
fn dampening_up(above: u16) -> u8 {
    let b = &rine_data::get().blocks;
    if b.has(above, flag::OPAQUE) && data::sturdy(above, DOWN) {
        16
    } else {
        b.state(above).light_block
    }
}

/// `FluidState.isFull()`: amount 8.
#[inline]
fn fluid_full(st: u16) -> bool {
    rine_data::get().blocks.state(st).fluid_level == 8
}

/// `SpreadingSnowyBlock.canStayAlive` for the block at `pos`.
fn can_stay_alive<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    let t = t();
    let above = l.get(pos.offset(0, 1, 0));
    if block_of(above) == t.snow_layer && geti(above, Pr::Layers) == 1 {
        return true;
    }
    if fluid_full(above) {
        return false;
    }
    dampening_up(above) < 15
}

/// `SpreadingSnowyBlock.canPropagate` for a dirt block at `pos`.
fn can_propagate<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    can_stay_alive(l, pos) && !rine_data::get().blocks.has(l.get(pos.offset(0, 1, 0)), flag::WATER)
}

#[inline]
fn set_all<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16) {
    engine::set_block(rs, l, pos, st, fl::ALL, UPDATE_LIMIT);
}

pub fn random_tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    match super::beh(st) {
        Beh::Spreading => spread_tick(rs, l, st, pos, rng),
        Beh::Nylium => {
            // `NyliumBlock.randomTick`
            if dampening_up(l.get(pos.offset(0, 1, 0))) >= 15 {
                set_all(rs, l, pos, t().netherrack_default);
            }
        }
        _ => {}
    }
}

/// `SpreadingSnowyBlock.randomTick` (grass and mycelium).
fn spread_tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    let t = t();
    if !can_stay_alive(l, pos) {
        set_all(rs, l, pos, t.dirt_default);
        return;
    }
    if local_brightness(l, pos.offset(0, 1, 0)) < 9 {
        return;
    }
    let me = if block_of(st) == t.mycelium { t.mycelium_default } else { t.grass_default };
    for _ in 0..4 {
        // argument order as in vanilla: x, y, z
        let dx = rng.next_int(3) - 1;
        let dy = rng.next_int(5) - 3;
        let dz = rng.next_int(3) - 1;
        let tp = pos.offset(dx, dy, dz);
        if tp == pos {
            continue;
        }
        if block_of(l.get(tp)) != t.dirt || !can_propagate(l, tp) {
            continue;
        }
        let snowy = is_snowy_setting(l.get(tp.offset(0, 1, 0)));
        set_all(rs, l, tp, setb(me, Pr::Snowy, snowy));
    }
}

/// `SnowyBlock.updateShape`: `snowy` follows the block above; other
/// directions leave the state alone. `dir` points from the block to the neighbor.
pub fn update_shape(st: u16, dir: u8, nst: u16) -> u16 {
    if dir == UP {
        setb(st, Pr::Snowy, is_snowy_setting(nst))
    } else {
        st
    }
}

#[cfg(test)]
mod tests {
    use super::super::testw::World as TW;
    use super::*;

    fn s(n: &str) -> u16 {
        default_state(n)
    }

    fn up(p: BlockPos) -> BlockPos {
        p.offset(0, 1, 0)
    }

    /// Dirt at y=63 in a 5x5 patch, air above it, sky light `light`.
    fn dirt_field(light: u8) -> TW {
        let mut w = TW::new();
        w.light = light;
        for x in -2..=2 {
            for z in -2..=2 {
                w.put(BlockPos::new(x, 63, z), s("dirt"));
            }
        }
        w
    }

    fn ticks<L: Lvl>(rs: &mut Rs, w: &mut L, p: BlockPos, n: usize, rng: &mut Rng) {
        for _ in 0..n {
            let st = w.get(p);
            random_tick(rs, w, st, p, rng);
        }
    }

    /// The 8 dirt tiles around (0, 63, 0) under the grass at (0, 64, 0).
    fn ring() -> Vec<BlockPos> {
        let mut v = Vec::new();
        for x in -1..=1 {
            for z in -1..=1 {
                if x != 0 || z != 0 {
                    v.push(BlockPos::new(x, 63, z));
                }
            }
        }
        v
    }

    #[test]
    fn grass_spreads_to_dirt_in_light() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = dirt_field(15);
        w.put(p, s("grass_block"));
        let mut rs = Rs::default();
        let mut rng = Rng::new(3);
        ticks(&mut rs, &mut w, p, 400, &mut rng);
        for q in ring() {
            assert_eq!(w.get(q), s("grass_block"), "{q:?}");
        }
        // the dirt under the grass has grass on top: it cannot propagate
        assert_eq!(w.get(BlockPos::new(0, 63, 0)), s("dirt"));
        assert_eq!(w.get(p), s("grass_block"));
    }

    #[test]
    fn grass_does_not_spread_in_darkness() {
        let p = BlockPos::new(0, 64, 0);
        // local brightness above is 8 < 9
        let mut w = dirt_field(8);
        w.put(p, s("grass_block"));
        let mut rs = Rs::default();
        let mut rng = Rng::new(4);
        ticks(&mut rs, &mut w, p, 400, &mut rng);
        for q in ring() {
            assert_eq!(w.get(q), s("dirt"), "{q:?}");
        }
        assert_eq!(w.get(p), s("grass_block"));
    }

    #[test]
    fn grass_under_opaque_block_turns_to_dirt() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = dirt_field(15);
        w.put(p, s("grass_block"));
        w.put(up(p), s("stone"));
        let mut rs = Rs::default();
        let mut rng = Rng::new(5);
        ticks(&mut rs, &mut w, p, 1, &mut rng);
        assert_eq!(w.get(p), s("dirt"));
    }

    #[test]
    fn grass_under_water_source_turns_to_dirt() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = dirt_field(15);
        w.put(p, s("grass_block"));
        w.put(up(p), s("water"));
        let mut rs = Rs::default();
        let mut rng = Rng::new(6);
        ticks(&mut rs, &mut w, p, 1, &mut rng);
        assert_eq!(w.get(p), s("dirt"));
    }

    #[test]
    fn spread_skips_dirt_under_opaque_or_water() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = dirt_field(15);
        w.put(p, s("grass_block"));
        // stone above one tile, water above another
        w.put(up(BlockPos::new(1, 63, 0)), s("stone"));
        w.put(up(BlockPos::new(-1, 63, 0)), s("water"));
        let mut rs = Rs::default();
        let mut rng = Rng::new(8);
        ticks(&mut rs, &mut w, p, 400, &mut rng);
        assert_eq!(w.get(BlockPos::new(1, 63, 0)), s("dirt"));
        assert_eq!(w.get(BlockPos::new(-1, 63, 0)), s("dirt"));
        assert_eq!(w.get(BlockPos::new(0, 63, 1)), s("grass_block"));
    }

    #[test]
    fn spread_takes_snowy_from_snow_above_the_target() {
        let p = BlockPos::new(0, 64, 0);
        let target = BlockPos::new(1, 63, 0);
        let mut w = dirt_field(15);
        w.put(p, s("grass_block"));
        w.put(up(target), seti_layers(1));
        let mut rs = Rs::default();
        let mut rng = Rng::new(9);
        ticks(&mut rs, &mut w, p, 400, &mut rng);
        assert_eq!(w.get(target), setb(s("grass_block"), Pr::Snowy, true));
        // a plain tile keeps snowy off
        assert_eq!(w.get(BlockPos::new(0, 63, 1)), s("grass_block"));
    }

    fn seti_layers(n: i32) -> u16 {
        super::super::props::seti(s("snow"), Pr::Layers, n)
    }

    #[test]
    fn mycelium_spreads() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = dirt_field(15);
        w.put(p, s("mycelium"));
        let mut rs = Rs::default();
        let mut rng = Rng::new(10);
        ticks(&mut rs, &mut w, p, 400, &mut rng);
        for q in ring() {
            assert_eq!(w.get(q), s("mycelium"), "{q:?}");
        }
        // mycelium dies under stone too
        w.put(up(p), s("stone"));
        ticks(&mut rs, &mut w, p, 1, &mut rng);
        assert_eq!(w.get(p), s("dirt"));
    }

    #[test]
    fn snowy_follows_snow_above_in_update_shape() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = TW::new();
        let mut rs = Rs::default();
        for name in ["grass_block", "podzol", "mycelium"] {
            let st = s(name);
            let snow = seti_layers(1);
            let out = super::super::update_shape(&mut rs, &mut w, st, p, UP, up(p), snow);
            assert_eq!(out, setb(st, Pr::Snowy, true), "{name}");
            let back = super::super::update_shape(&mut rs, &mut w, out, p, UP, up(p), 0);
            assert_eq!(back, st, "{name}");
            // a neighbour to the side does not change snowy
            let side = super::super::update_shape(&mut rs, &mut w, st, p, data::EAST, p, snow);
            assert_eq!(side, st, "{name}");
        }
    }

    #[test]
    fn podzol_does_not_spread() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = dirt_field(15);
        w.put(p, s("podzol"));
        let mut rs = Rs::default();
        let mut rng = Rng::new(11);
        ticks(&mut rs, &mut w, p, 400, &mut rng);
        for q in ring() {
            assert_eq!(w.get(q), s("dirt"), "{q:?}");
        }
    }

    #[test]
    fn nylium_turns_to_netherrack_under_light_blocking_top() {
        let p = BlockPos::new(0, 64, 0);
        let mut rs = Rs::default();
        let mut rng = Rng::new(12);
        for name in ["crimson_nylium", "warped_nylium"] {
            // air above: stays
            let mut w = TW::new();
            w.put(p, s(name));
            ticks(&mut rs, &mut w, p, 200, &mut rng);
            assert_eq!(w.get(p), s(name), "{name}");
            // glass above passes light: stays
            w.put(up(p), s("glass"));
            ticks(&mut rs, &mut w, p, 50, &mut rng);
            assert_eq!(w.get(p), s(name), "{name} under glass");
            // stone above: netherrack
            w.put(up(p), s("stone"));
            ticks(&mut rs, &mut w, p, 1, &mut rng);
            assert_eq!(w.get(p), s("netherrack"), "{name} under stone");
        }
    }
}
