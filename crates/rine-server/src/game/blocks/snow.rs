//! Ice, frosted ice, snow layers and precipitation: `IceBlock.randomTick`
//! and `melt`, `FrostedIceBlock` (scheduled aging, neighbour melting),
//! `SnowLayerBlock` (survival, light melting, `updateShape`), and
//! `ServerLevel.tickPrecipitation` with `Biome.shouldFreeze` / `shouldSnow`
//! and the cauldron `handlePrecipitation` methods.
//!
//! Not ported: `IceBlock.afterDestroy` (player breaking, not a tick),
//! `Block.pushEntitiesUp` (entities inside a growing snow layer), the
//! ultra-warm `WATER_EVAPORATES` branch (overworld only: ice always melts
//! into water), and the frosted ice config of Paper (defaults used).

use super::props::{geti, seti, Pr};
use super::{block_of, block_tag, default_state, is_air, local_brightness, rule_int, Beh, Rng};
use crate::game::redstone::data::{self, step, DOWN, EAST, NORTH, SOUTH, UP, WEST};
use crate::game::redstone::engine::{self, fl, prio, Lvl, UPDATE_LIMIT};
use crate::game::redstone::Rs;
use rine_data::blocks::flag;
use rine_protocol::BlockPos;
use rine_world::{MAX_Y, MIN_Y};
use std::sync::OnceLock;

struct T {
    air: u16,
    water: u16,
    water_state: u16,
    ice_state: u16,
    frosted: u16,
    snow: u16,
    snow_state: u16,
    cauldron: u16,
    water_cauldron: u16,
    water_cauldron_state: u16,
    powder_snow_cauldron: u16,
    powder_cauldron_state: u16,
    cannot_support_snow: Vec<bool>,
    support_override_snow: Vec<bool>,
}

fn t() -> &'static T {
    static T_: OnceLock<T> = OnceLock::new();
    T_.get_or_init(|| T {
        air: default_state("air"),
        water: super::block_id("water"),
        water_state: default_state("water"),
        ice_state: default_state("ice"),
        frosted: super::block_id("frosted_ice"),
        snow: super::block_id("snow"),
        snow_state: default_state("snow"),
        cauldron: super::block_id("cauldron"),
        water_cauldron: super::block_id("water_cauldron"),
        water_cauldron_state: default_state("water_cauldron"),
        powder_snow_cauldron: super::block_id("powder_snow_cauldron"),
        powder_cauldron_state: default_state("powder_snow_cauldron"),
        cannot_support_snow: block_tag("cannot_support_snow_layer"),
        support_override_snow: block_tag("support_override_snow_layer"),
    })
}

pub fn classify(n: &str) -> Option<Beh> {
    Some(match n {
        "ice" => Beh::Ice,
        "frosted_ice" => Beh::FrostedIce,
        "snow" => Beh::Snow,
        _ => return None,
    })
}

/// `BlockState.getLightDampening` (rine_data `light_block`).
#[inline]
fn light_block(st: u16) -> i32 {
    rine_data::get().blocks.state(st).light_block as i32
}

#[inline]
fn in_build_height(y: i32) -> bool {
    (MIN_Y..=MAX_Y).contains(&y)
}

/// `IceBlock.melt` in the overworld: the block becomes water, then its
/// neighbours are told (`level.neighborChanged(pos, water, null)`).
fn melt<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos) {
    let t = t();
    engine::set_block(rs, l, pos, t.water_state, fl::ALL, UPDATE_LIMIT);
    engine::neighbor_changed_at(rs, l, pos, t.water);
}

/// `IceBlock.randomTick` (also inherited by `FrostedIceBlock`).
pub fn random_tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    match super::beh(st) {
        Beh::Ice | Beh::FrostedIce => {
            // melts when block light > 11 - lightDampening
            if l.block_light(pos) as i32 > 11 - light_block(st) {
                melt(rs, l, pos);
            }
        }
        Beh::Snow => {
            // `SnowLayerBlock.randomTick`: bright block light clears the layer
            if l.block_light(pos) > 11 {
                engine::drop_resources(l, st, pos);
                engine::remove_block(rs, l, pos, false);
            }
        }
        _ => {}
    }
}

/// `SnowLayerBlock.canSurvive`: not on the cannot-support tag, and on a
/// support-override block, a full top face, or a layer of 8 snow.
fn snow_can_survive<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    let t = t();
    let b = l.get(pos.offset(0, -1, 0));
    let blk = block_of(b) as usize;
    if t.cannot_support_snow[blk] {
        return false;
    }
    t.support_override_snow[blk] || data::sturdy(b, UP) || (blk == t.snow as usize && geti(b, Pr::Layers) == 8)
}

/// `SnowLayerBlock.updateShape`: air when the layer cannot stay.
pub fn update_shape<L: Lvl>(l: &L, st: u16, pos: BlockPos) -> u16 {
    if snow_can_survive(l, pos) {
        st
    } else {
        t().air
    }
}

/// Precipitation at a position for the biome (`Biome.Precipitation`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Precip {
    None,
    Rain,
    Snow,
}

fn precipitation_at<L: Lvl>(l: &L, biome: u8, pos: BlockPos) -> Precip {
    if !l.has_precipitation(biome) {
        Precip::None
    } else if l.temperature(biome, pos) < 0.15 {
        // `coldEnoughToSnow`: not `warmEnoughToRain`
        Precip::Snow
    } else {
        Precip::Rain
    }
}

/// `Biome.shouldFreeze(level, pos)` with `checkNeighbors = true`. Only
/// still (source) water freezes, and only where a side is not water.
fn should_freeze<L: Lvl>(l: &L, biome: u8, pos: BlockPos) -> bool {
    let t = t();
    if l.temperature(biome, pos) >= 0.15 {
        return false;
    }
    if !in_build_height(pos.y) || l.block_light(pos) >= 10 {
        return false;
    }
    let st = l.get(pos);
    // `fluidState.is(Fluids.WATER)` is the source fluid, and LiquidBlock is the water block
    if block_of(st) != t.water || !rine_data::get().blocks.has(st, flag::FLUID_SOURCE) {
        return false;
    }
    let wet = |p: BlockPos| rine_data::get().blocks.has(l.get(p), flag::WATER);
    let surrounded = wet(pos.offset(-1, 0, 0)) && wet(pos.offset(1, 0, 0)) && wet(pos.offset(0, 0, -1)) && wet(pos.offset(0, 0, 1));
    !surrounded
}

/// `Biome.shouldSnow(level, pos)`.
fn should_snow<L: Lvl>(l: &L, biome: u8, pos: BlockPos) -> bool {
    if precipitation_at(l, biome, pos) != Precip::Snow {
        return false;
    }
    if !in_build_height(pos.y) || l.block_light(pos) >= 10 {
        return false;
    }
    let st = l.get(pos);
    (is_air(st) || block_of(st) == t().snow) && snow_can_survive(l, pos)
}

/// `Biome.shouldHandlePrecipitation` (the cauldron draw). Draws only for
/// rain and snow, like the vanilla ternary.
fn should_handle_precipitation(rng: &mut Rng, p: Precip) -> bool {
    match p {
        Precip::Rain => rng.next_float() < 0.05,
        Precip::Snow => rng.next_float() < 0.1,
        Precip::None => false,
    }
}

/// `Block.handlePrecipitation` of the cauldrons. Other blocks do nothing.
fn handle_precipitation<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, p: Precip, rng: &mut Rng) {
    let t = t();
    let st = l.get(pos);
    let blk = block_of(st);
    if blk == t.cauldron {
        // CauldronBlock: an empty cauldron fills
        if !should_handle_precipitation(rng, p) {
            return;
        }
        let to = match p {
            Precip::Rain => t.water_cauldron_state,
            Precip::Snow => t.powder_cauldron_state,
            Precip::None => return,
        };
        engine::set_block(rs, l, pos, to, fl::ALL, UPDATE_LIMIT);
    } else if blk == t.water_cauldron || blk == t.powder_snow_cauldron {
        // LayeredCauldronBlock: one more level, for its own precipitation type
        let kind = if blk == t.water_cauldron { Precip::Rain } else { Precip::Snow };
        let level = geti(st, Pr::Level);
        if should_handle_precipitation(rng, p) && level != 3 && p == kind {
            engine::set_block(rs, l, pos, seti(st, Pr::Level, level + 1), fl::ALL, UPDATE_LIMIT);
        }
    }
}

/// `ServerLevel.tickPrecipitation` for the column (x, z), with the game rule
/// `max_snow_accumulation_height`.
pub fn precipitation<L: Lvl>(rs: &mut Rs, l: &mut L, x: i32, z: i32, rng: &mut Rng) {
    static MAX_SNOW: OnceLock<Option<usize>> = OnceLock::new();
    let max = rule_int(l, &MAX_SNOW, "max_snow_accumulation_height", 1);
    precipitation_with(rs, l, x, z, rng, max);
}

/// `tickPrecipitation` with the snow height limit given (tests use it).
fn precipitation_with<L: Lvl>(rs: &mut Rs, l: &mut L, x: i32, z: i32, rng: &mut Rng, max_snow: i32) {
    let t = t();
    // `getHeightmapPos(MOTION_BLOCKING, pos)` and the biome there
    let top = BlockPos::new(x, l.height_motion_blocking(x, z), z);
    let below = top.offset(0, -1, 0);
    let biome = l.biome(top);
    if should_freeze(l, biome, below) {
        engine::set_block(rs, l, below, t.ice_state, fl::ALL, UPDATE_LIMIT);
    }
    if !l.raining() {
        return;
    }
    if max_snow > 0 && should_snow(l, biome, top) {
        let st = l.get(top);
        if block_of(st) == t.snow {
            let layers = geti(st, Pr::Layers);
            if layers < max_snow.min(8) {
                engine::set_block(rs, l, top, seti(st, Pr::Layers, layers + 1), fl::ALL, UPDATE_LIMIT);
            }
        } else {
            engine::set_block(rs, l, top, t.snow_state, fl::ALL, UPDATE_LIMIT);
        }
    }
    // read the block below again: it may have just frozen
    let p = precipitation_at(l, biome, below);
    if p != Precip::None {
        handle_precipitation(rs, l, below, p, rng);
    }
}

// ------------------------------------------------------------------ frosted ice

/// `FrostedIceBlock` neighbour order: `Direction.values()`.
const FROSTED_ORDER: [u8; 6] = [DOWN, UP, NORTH, SOUTH, WEST, EAST];

/// `FrostedIceBlock.fewerNeigboursThan`: fewer than `limit` frosted ice neighbours.
fn fewer_frosted_than<L: Lvl>(l: &L, pos: BlockPos, limit: usize) -> bool {
    let f = t().frosted;
    let mut n = 0;
    for d in FROSTED_ORDER {
        let (dx, dy, dz) = step(d);
        if block_of(l.get(pos.offset(dx, dy, dz))) == f {
            n += 1;
            if n >= limit {
                return false;
            }
        }
    }
    true
}

/// `FrostedIceBlock.slightlyMelt`: age by one, or melt at the last age.
/// Returns true when it melted.
fn slightly_melt<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) -> bool {
    let age = geti(st, Pr::Age);
    if age < 3 {
        engine::set_block(rs, l, pos, seti(st, Pr::Age, age + 1), fl::CLIENTS, UPDATE_LIMIT);
        false
    } else {
        melt(rs, l, pos);
        true
    }
}

/// `FrostedIceBlock.onPlace`: the first tick comes 60 to 120 ticks later.
pub fn on_place<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos) {
    let delay = rs.rng.range(60, 120) as u32;
    engine::schedule(rs, l, pos, t().frosted, delay, prio::NORMAL);
}

/// `FrostedIceBlock.tick` (scheduled).
pub fn tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    let t = t();
    if rs.rng.next_int(3) == 0 || fewer_frosted_than(l, pos, 4) {
        // `getMaxLocalRawBrightness` in the overworld
        let brightness = local_brightness(l, pos) as i32;
        let age = geti(st, Pr::Age);
        if brightness > 11 - age - light_block(st) && slightly_melt(rs, l, st, pos) {
            for d in FROSTED_ORDER {
                let (dx, dy, dz) = step(d);
                let np = pos.offset(dx, dy, dz);
                let ns = l.get(np);
                if block_of(ns) == t.frosted && !slightly_melt(rs, l, ns, np) {
                    let delay = rs.rng.range(20, 40) as u32;
                    engine::schedule(rs, l, np, t.frosted, delay, prio::NORMAL);
                }
            }
            return;
        }
    }
    let delay = rs.rng.range(20, 40) as u32;
    engine::schedule(rs, l, pos, t.frosted, delay, prio::NORMAL);
}

/// `FrostedIceBlock.neighborChanged`: a frosted neighbour and fewer than two
/// frosted sides melt the block.
pub fn neighbor_changed<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, from: u16) {
    if from == t().frosted && fewer_frosted_than(l, pos, 2) {
        melt(rs, l, pos);
    }
}

#[cfg(test)]
mod tests {
    use super::super::testw::World as TW;
    use super::*;
    use crate::game::redstone::engine::UPDATE_LIMIT as LIMIT;

    fn s(n: &str) -> u16 {
        default_state(n)
    }

    fn p(x: i32, y: i32, z: i32) -> BlockPos {
        BlockPos::new(x, y, z)
    }

    #[test]
    fn water_freezes_only_cold_dark_still_and_at_an_edge() {
        let below = p(0, 63, 0);
        let mut rs = Rs::default();
        let mut rng = Rng::new(1);
        // lone source water in a cold biome, dark: freezes
        let mut w = TW::new();
        w.temp = -0.5;
        w.put(below, s("water"));
        precipitation(&mut rs, &mut w, 0, 0, &mut rng);
        assert_eq!(w.get(below), t().ice_state);

        // block light 10 is enough to stop it
        let mut w = TW::new();
        w.temp = -0.5;
        w.put(below, s("water"));
        w.block_light.insert(below, 10);
        precipitation(&mut rs, &mut w, 0, 0, &mut rng);
        assert_eq!(w.get(below), s("water"));

        // flowing water never freezes
        let mut w = TW::new();
        w.temp = -0.5;
        w.put(below, seti(s("water"), Pr::Level, 1));
        precipitation(&mut rs, &mut w, 0, 0, &mut rng);
        assert_eq!(block_of(w.get(below)), t().water);

        // surrounded by water on all four sides: stays
        let mut w = TW::new();
        w.temp = -0.5;
        w.put(below, s("water"));
        for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            w.put(p(dx, 63, dz), s("water"));
        }
        precipitation(&mut rs, &mut w, 0, 0, &mut rng);
        assert_eq!(w.get(below), s("water"));

        // warm biome: stays
        let mut w = TW::new();
        w.temp = 0.8;
        w.put(below, s("water"));
        precipitation(&mut rs, &mut w, 0, 0, &mut rng);
        assert_eq!(w.get(below), s("water"));
    }

    #[test]
    fn snow_accumulates_while_raining_up_to_the_rule() {
        let mut rs = Rs::default();
        let mut rng = Rng::new(2);
        let top = p(0, 64, 0);
        let mut w = TW::new();
        w.temp = -0.5;
        w.raining = true;
        w.put(p(0, 63, 0), s("stone"));
        for _ in 0..10 {
            precipitation_with(&mut rs, &mut w, 0, 0, &mut rng, 3);
        }
        assert_eq!(geti(w.get(top), Pr::Layers), 3);
        // the rule caps the layers
        for _ in 0..20 {
            precipitation_with(&mut rs, &mut w, 0, 0, &mut rng, 8);
        }
        assert_eq!(geti(w.get(top), Pr::Layers), 8);

        // rule 0: no snow at all
        let mut w = TW::new();
        w.temp = -0.5;
        w.raining = true;
        w.put(p(0, 63, 0), s("stone"));
        precipitation_with(&mut rs, &mut w, 0, 0, &mut rng, 0);
        assert!(is_air(w.get(top)));

        // no rain: no snow
        let mut w = TW::new();
        w.temp = -0.5;
        w.put(p(0, 63, 0), s("stone"));
        for _ in 0..5 {
            precipitation_with(&mut rs, &mut w, 0, 0, &mut rng, 8);
        }
        assert!(is_air(w.get(top)));
    }

    #[test]
    fn snow_survival_follows_the_support_block() {
        let pos = p(0, 64, 0);
        let mut w = TW::new();
        let check = |w: &mut TW, below: &str, want: bool| {
            w.put(p(0, 63, 0), s(below));
            assert_eq!(snow_can_survive(w, pos), want, "snow on {below}");
        };
        check(&mut w, "stone", true);
        check(&mut w, "honey_block", true);
        check(&mut w, "soul_sand", true);
        check(&mut w, "ice", false);
        check(&mut w, "packed_ice", false);
        check(&mut w, "barrier", false);
        check(&mut w, "farmland", false);
        check(&mut w, "dirt_path", false);
        check(&mut w, "air", false);
        check(&mut w, "snow", false);
        w.put(p(0, 63, 0), seti(s("snow"), Pr::Layers, 7));
        assert!(!snow_can_survive(&w, pos));
        w.put(p(0, 63, 0), seti(s("snow"), Pr::Layers, 8));
        assert!(snow_can_survive(&w, pos));
    }

    #[test]
    fn ice_melts_by_block_light_and_a_torch_melts_it() {
        rine_data::init();
        let pos = p(0, 64, 0);
        let mut rs = Rs::default();
        let mut rng = Rng::new(3);
        let lb = light_block(s("ice"));
        for bl in 0..=15u8 {
            let mut w = TW::new();
            w.put(pos, s("ice"));
            w.block_light.insert(pos, bl);
            let st = w.get(pos);
            super::super::random_tick(&mut rs, &mut w, st, pos, &mut rng);
            let melted = block_of(w.get(pos)) == block_of(s("water"));
            assert_eq!(melted, bl as i32 > 11 - lb, "block light {bl}, light block {lb}");
        }
        // a torch (light 14) next to the ice
        let mut w = TW::new();
        w.put(pos, s("ice"));
        w.block_light.insert(pos, 14);
        let st = w.get(pos);
        super::super::random_tick(&mut rs, &mut w, st, pos, &mut rng);
        assert_eq!(w.get(pos), s("water"));
    }

    #[test]
    fn snow_layer_clears_under_block_light_above_11() {
        let pos = p(0, 64, 0);
        let mut rs = Rs::default();
        let mut rng = Rng::new(4);
        let mut w = TW::new();
        w.put(pos, s("snow"));
        w.block_light.insert(pos, 11);
        let st = w.get(pos);
        super::super::random_tick(&mut rs, &mut w, st, pos, &mut rng);
        assert_eq!(w.get(pos), s("snow"));
        w.block_light.insert(pos, 12);
        let st = w.get(pos);
        super::super::random_tick(&mut rs, &mut w, st, pos, &mut rng);
        assert!(is_air(w.get(pos)));
        assert_eq!(w.dropped.len(), 1);
    }

    #[test]
    fn cauldrons_fill_with_rain_and_snow() {
        let mut rs = Rs::default();
        let mut rng = Rng::new(5);
        let pos = p(0, 63, 0);
        // rain: an empty cauldron becomes water, then gains levels up to 3
        let mut w = TW::new();
        w.raining = true;
        w.put(p(0, 62, 0), s("stone"));
        w.put(pos, s("cauldron"));
        for _ in 0..3000 {
            precipitation(&mut rs, &mut w, 0, 0, &mut rng);
        }
        assert_eq!(block_of(w.get(pos)), t().water_cauldron);
        assert_eq!(geti(w.get(pos), Pr::Level), 3);

        // cold rain: powder snow cauldron
        let mut w = TW::new();
        w.raining = true;
        w.temp = -0.5;
        w.put(p(0, 62, 0), s("stone"));
        w.put(pos, s("cauldron"));
        for _ in 0..3000 {
            precipitation(&mut rs, &mut w, 0, 0, &mut rng);
        }
        assert_eq!(block_of(w.get(pos)), t().powder_snow_cauldron);
        assert_eq!(geti(w.get(pos), Pr::Level), 3);

        // a desert (no precipitation) never fills
        let mut w = TW::new();
        w.raining = true;
        w.precip = false;
        w.put(p(0, 62, 0), s("stone"));
        w.put(pos, s("cauldron"));
        for _ in 0..3000 {
            precipitation(&mut rs, &mut w, 0, 0, &mut rng);
        }
        assert_eq!(block_of(w.get(pos)), block_of(s("cauldron")));

        // rain never fills a powder snow cauldron
        let mut w = TW::new();
        w.raining = true;
        w.put(p(0, 62, 0), s("stone"));
        w.put(pos, s("powder_snow_cauldron"));
        for _ in 0..3000 {
            precipitation(&mut rs, &mut w, 0, 0, &mut rng);
        }
        assert_eq!(geti(w.get(pos), Pr::Level), 1);
    }

    #[test]
    fn lone_frosted_ice_melts_after_aging() {
        let pos = p(0, 64, 0);
        let mut rs = Rs::default();
        let mut w = TW::new();
        engine::set_block(&mut rs, &mut w, pos, s("frosted_ice"), fl::ALL, LIMIT);
        for _ in 0..2000 {
            w.tick(&mut rs);
            if block_of(w.get(pos)) != t().frosted {
                break;
            }
        }
        assert_eq!(w.get(pos), s("water"));
    }

    #[test]
    fn frosted_ice_loses_its_neighbour_and_melts_when_it_has_fewer_than_two() {
        let pos = p(0, 64, 0);
        let other = p(1, 64, 0);
        let mut rs = Rs::default();
        let mut w = TW::new();
        engine::set_block(&mut rs, &mut w, pos, s("frosted_ice"), fl::ALL, LIMIT);
        engine::set_block(&mut rs, &mut w, other, s("frosted_ice"), fl::ALL, LIMIT);
        // placing a neighbour sends `updateNeighborsAt` with the old block (air): no melt
        assert_eq!(block_of(w.get(pos)), t().frosted);
        // removing it sends the frosted ice as the source: one side left, it melts
        engine::set_block(&mut rs, &mut w, other, 0, fl::ALL, LIMIT);
        assert_eq!(w.get(pos), s("water"));
    }
}
