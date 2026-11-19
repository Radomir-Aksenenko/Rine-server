//! Dripleaf family: `BigDripleafBlock`, `BigDripleafStemBlock`,
//! `SmallDripleafBlock` and the `Tilt` property of the big head.
//!
//! - Survival and `updateShape`: stem and head conversions, a stem without
//!   a head breaks one tick later (`tick`), a small plant breaks with its
//!   other half.
//! - Placement (`getStateForPlacement`, `canSurvive` of `BlockItem`):
//!   facing, waterlogging, the small plant's support.
//! - Bone meal: the big head grows one block up, a stem column grows through
//!   its top, the small plant becomes a big column (`placeWithRandomHeight`).
//! - Tilt: `entityInside` for an entity on the head (`tilt_feet`, fed once
//!   per tick by `players_tilt`), the scheduled cycle unstable -> partial ->
//!   full -> none, and a neighbor signal resetting it (`neighbor_changed`).
//!
//! Not ported: `onProjectileHit` (Rine has no projectiles that hit blocks),
//! the fluid ticks of waterlogged parts (water is handled by `fluids.rs`),
//! the `BLOCK_CHANGE` game event of tilt (Rine has no game events yet).

use super::props::{getb, getv, geti, setb, setv, vid, Pr};
use super::{block_id, block_of, block_tag, default_state, is_air, Beh, PlaceCtx, Rng};
use crate::game::redstone::data::{self, DOWN, UP};
use crate::game::redstone::engine::{self, fl, prio, Lvl, CAT_BLOCKS, UPDATE_LIMIT};
use crate::game::redstone::Rs;
use crate::game::Game;
use rine_data::blocks::flag;
use rine_protocol::BlockPos;
use rine_world::{MAX_Y, MIN_Y};
use std::sync::OnceLock;

/// `Direction` protocol ids of the horizontal facings, indexed like `place::horizontal_dir`
/// (south, west, north, east).
const HDIR: [u8; 4] = [3, 4, 2, 5];

/// Delays of `BigDripleafBlock.DELAY_UNTIL_NEXT_TILT_STATE`.
const DELAY_UNSTABLE: u32 = 10;
const DELAY_PARTIAL: u32 = 10;
const DELAY_FULL: u32 = 100;

const SOUND_TILT_DOWN: &str = "block.big_dripleaf.tilt_down";
const SOUND_TILT_UP: &str = "block.big_dripleaf.tilt_up";

struct T {
    big: u16,
    stem: u16,
    small: u16,
    water: u16,
    supports_big: Vec<bool>,
    supports_small: Vec<bool>,
    supports_veg: Vec<bool>,
    v_lower: u8,
    v_upper: u8,
    v_none: u8,
    v_unstable: u8,
    v_partial: u8,
    v_full: u8,
}

fn t() -> &'static T {
    static T_: OnceLock<T> = OnceLock::new();
    T_.get_or_init(|| T {
        big: block_id("big_dripleaf"),
        stem: block_id("big_dripleaf_stem"),
        small: block_id("small_dripleaf"),
        water: block_id("water"),
        supports_big: block_tag("supports_big_dripleaf"),
        supports_small: block_tag("supports_small_dripleaf"),
        supports_veg: block_tag("supports_vegetation"),
        v_lower: vid("lower"),
        v_upper: vid("upper"),
        v_none: vid("none"),
        v_unstable: vid("unstable"),
        v_partial: vid("partial"),
        v_full: vid("full"),
    })
}

pub fn classify(n: &str) -> Option<Beh> {
    Some(match n {
        "big_dripleaf" => Beh::BigDripleaf,
        "big_dripleaf_stem" => Beh::DripleafStem,
        "small_dripleaf" => Beh::SmallDripleaf,
        _ => return None,
    })
}

#[inline]
fn down(p: BlockPos) -> BlockPos {
    p.offset(0, -1, 0)
}

#[inline]
fn up(p: BlockPos) -> BlockPos {
    p.offset(0, 1, 0)
}

// ------------------------------------------------------------ states

fn big_state(wl: bool, facing: u8) -> u16 {
    data::set(setb(default_state("big_dripleaf"), Pr::Waterlogged, wl), data::P::Facing, facing)
}

fn stem_state(wl: bool, facing: u8) -> u16 {
    data::set(setb(default_state("big_dripleaf_stem"), Pr::Waterlogged, wl), data::P::Facing, facing)
}

fn small_state(lower: bool, wl: bool, facing: u8) -> u16 {
    let t = t();
    let st = setv(default_state("small_dripleaf"), Pr::Half, if lower { t.v_lower } else { t.v_upper });
    data::set(setb(st, Pr::Waterlogged, wl), data::P::Facing, facing)
}

// ------------------------------------------------------------ fluids

/// `FluidState.isSourceOfType(WATER)`: a water source block or a waterlogged block.
fn water_source(st: u16) -> bool {
    if block_of(st) == t().water {
        geti(st, Pr::Level) == 0
    } else {
        getb(st, Pr::Waterlogged)
    }
}

/// `FluidState.is(WATER)` at any level, as `Level.isWaterAt`.
fn water_any(st: u16) -> bool {
    block_of(st) == t().water || getb(st, Pr::Waterlogged)
}

/// `getFluidState(pos).createLegacyBlock()`: the water block that stays
/// in a cell when its block is removed (air without water).
fn legacy_fluid(st: u16) -> u16 {
    if block_of(st) == t().water {
        st
    } else if getb(st, Pr::Waterlogged) {
        default_state("water")
    } else {
        0
    }
}

// ------------------------------------------------------------ survival

/// `BigDripleafBlock.canSurvive`: a head, a stem or a supporting block below.
fn big_survives<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    let t = t();
    let b = block_of(l.get(down(pos)));
    b == t.big || b == t.stem || t.supports_big[b as usize]
}

/// `BigDripleafStemBlock.canSurvive`: a stem or support below, a stem or head above.
fn stem_survives<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    let t = t();
    let b = block_of(l.get(down(pos)));
    let a = block_of(l.get(up(pos)));
    (b == t.stem || t.supports_big[b as usize]) && (a == t.stem || a == t.big)
}

/// `SmallDripleafBlock.mayPlaceOn` for the block below a lower half at `cell`.
fn small_may_place_on<L: Lvl>(l: &L, below: BlockPos, cell: BlockPos) -> bool {
    let t = t();
    let b = block_of(l.get(below)) as usize;
    // `level.getFluidState(belowPos.above()).isSourceOfType(WATER)`: the cell is the plant's own
    t.supports_small[b] || (water_source(l.get(cell)) && t.supports_veg[b])
}

fn small_lower_survives<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    small_may_place_on(l, down(pos), pos)
}

/// `SmallDripleafBlock.canSurvive`.
fn small_survives<L: Lvl>(l: &L, st: u16, pos: BlockPos) -> bool {
    let t = t();
    if getv(st, Pr::Half) == t.v_upper {
        // `DoublePlantBlock.canSurvive` of the upper half
        let b = l.get(down(pos));
        block_of(b) == t.small && getv(b, Pr::Half) == t.v_lower
    } else {
        small_lower_survives(l, pos)
    }
}

/// `canGrowInto`: inside the build height and `canReplace` (air, water, small dripleaf).
fn can_grow_into<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    if !(MIN_Y..=MAX_Y).contains(&pos.y) {
        return false;
    }
    let st = l.get(pos);
    let t = t();
    is_air(st) || block_of(st) == t.water || block_of(st) == t.small
}

/// `DoublePlantBlock.getStateForPlacement`: the cell above must be replaceable.
fn free_above(st: u16) -> bool {
    is_air(st) || rine_data::get().blocks.has(st, flag::REPLACEABLE)
}

// ------------------------------------------------------------ placement

/// `getStateForPlacement` of the two dripleaves, and the `canSurvive` check
/// of `BlockItem` that goes with it. None: the block cannot be placed here.
pub fn placement<L: Lvl>(l: &L, st: u16, pos: BlockPos, ctx: &PlaceCtx) -> Option<u16> {
    let t = t();
    // `context.getHorizontalDirection().getOpposite()`
    let away = HDIR[((((ctx.yaw / 90.0).round() as i32 & 3) + 2) & 3) as usize];
    match super::beh(st) {
        Beh::BigDripleaf => {
            if !big_survives(l, pos) {
                return None;
            }
            // a dripleaf part below gives its facing, otherwise the placer's
            let below = l.get(down(pos));
            let b = block_of(below);
            let facing = if b == t.big || b == t.stem { data::get(below, data::P::Facing) } else { away };
            Some(big_state(water_source(l.get(pos)), facing))
        }
        Beh::SmallDripleaf => {
            if !free_above(l.get(up(pos))) || !small_lower_survives(l, pos) {
                return None;
            }
            Some(small_state(true, water_any(l.get(pos)), away))
        }
        _ => Some(st),
    }
}

// ------------------------------------------------------------ shape

/// `BlockState.updateShape` of the three blocks. `dir` points to the changed
/// neighbor, now `nst`. Returns air to break the block.
pub fn update_shape<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, dir: u8, nst: u16) -> u16 {
    let t = t();
    match super::beh(st) {
        Beh::BigDripleaf => {
            if dir == DOWN && !big_survives(&*l, pos) {
                return 0;
            }
            // `BIG_DRIPLEAF_STEM.withPropertiesOf(state)`: a head under a new head is a stem
            if dir == UP && block_of(nst) == t.big {
                return stem_state(getb(st, Pr::Waterlogged), data::get(st, data::P::Facing));
            }
            st
        }
        Beh::DripleafStem => {
            // `scheduleTick(pos, this, 1)`: destroyed by `tick` if still without support
            if (dir == DOWN || dir == UP) && !stem_survives(&*l, pos) {
                engine::schedule(rs, &*l, pos, t.stem, 1, prio::NORMAL);
            }
            st
        }
        Beh::SmallDripleaf => small_update_shape(&*l, st, pos, dir, nst),
        _ => st,
    }
}

/// `DoublePlantBlock.updateShape` followed by `VegetationBlock.updateShape`.
fn small_update_shape<L: Lvl>(l: &L, st: u16, pos: BlockPos, dir: u8, nst: u16) -> u16 {
    let t = t();
    let lower = getv(st, Pr::Half) == t.v_lower;
    let vertical = dir == UP || dir == DOWN;
    let other_half = block_of(nst) == t.small && getv(nst, Pr::Half) != getv(st, Pr::Half);
    if !vertical || lower != (dir == UP) || other_half {
        if small_survives(l, st, pos) {
            st
        } else {
            0
        }
    } else {
        // the neighbor is the other half in the wrong place: this half goes too
        0
    }
}

// ------------------------------------------------------------ scheduled ticks

/// `BigDripleafBlock.tick` and `BigDripleafStemBlock.tick`.
pub fn tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    let t = t();
    if block_of(st) == t.stem {
        if !stem_survives(&*l, pos) {
            engine::destroy_block(rs, l, pos, true, UPDATE_LIMIT);
        }
        return;
    }
    if engine::has_neighbor_signal(&*rs, &*l, pos) {
        reset_tilt(rs, l, st, pos);
        return;
    }
    let tilt = getv(st, Pr::Tilt);
    if tilt == t.v_unstable {
        tilt_and_schedule(rs, l, st, pos, t.v_partial, Some(SOUND_TILT_DOWN));
    } else if tilt == t.v_partial {
        tilt_and_schedule(rs, l, st, pos, t.v_full, Some(SOUND_TILT_DOWN));
    } else if tilt == t.v_full {
        reset_tilt(rs, l, st, pos);
    }
}

// ------------------------------------------------------------ tilt

/// `BigDripleafBlock.setTilt`: `setBlock(pos, state, UPDATE_CLIENTS)`.
fn set_tilt<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, tilt: u8) {
    engine::set_block(rs, l, pos, setv(st, Pr::Tilt, tilt), fl::CLIENTS, UPDATE_LIMIT);
}

/// `playTiltSound`: `Mth.randomBetween(random, 0.8, 1.2)` pitch, all players hear it.
fn play_tilt<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, sound: &str) {
    let pitch = rs.rng.next_float() * 0.4 + 0.8;
    l.sound(sound, pos, 1.0, pitch, CAT_BLOCKS, false);
}

/// `setTiltAndScheduleTick`: set, optional sound, then the delay of the new tilt.
fn tilt_and_schedule<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, tilt: u8, sound: Option<&str>) {
    let t = t();
    set_tilt(rs, l, st, pos, tilt);
    if let Some(s) = sound {
        play_tilt(rs, l, pos, s);
    }
    let delay = if tilt == t.v_unstable {
        DELAY_UNSTABLE
    } else if tilt == t.v_partial {
        DELAY_PARTIAL
    } else if tilt == t.v_full {
        DELAY_FULL
    } else {
        0
    };
    if delay > 0 {
        engine::schedule(rs, &*l, pos, t.big, delay, prio::NORMAL);
    }
}

/// `resetTilt`: back to none, with the tilt-up sound if it was tilted.
fn reset_tilt<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    let t = t();
    set_tilt(rs, l, st, pos, t.v_none);
    if getv(st, Pr::Tilt) != t.v_none {
        play_tilt(rs, l, pos, SOUND_TILT_UP);
    }
}

/// `BigDripleafBlock.neighborChanged`: a neighbor signal resets the tilt.
pub fn neighbor_changed<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    if engine::has_neighbor_signal(&*rs, &*l, pos) {
        reset_tilt(rs, l, st, pos);
    }
}

/// `BigDripleafBlock.entityInside` for entities whose feet are at `feet_y`
/// in the cell `pos` (the caller checked the box overlaps the cell and that
/// the entity is on the ground). Only the head tilts.
pub fn tilt_feet<L: Lvl>(rs: &mut Rs, l: &mut L, hits: &[(BlockPos, f64)]) {
    let t = t();
    for &(pos, feet_y) in hits {
        let st = l.get(pos);
        if block_of(st) != t.big || getv(st, Pr::Tilt) != t.v_none {
            continue;
        }
        // `canEntityTilt`: on the ground and above 0.6875 of the block
        if feet_y <= pos.y as f64 + 0.6875 {
            continue;
        }
        if engine::has_neighbor_signal(&*rs, &*l, pos) {
            continue;
        }
        tilt_and_schedule(rs, l, st, pos, t.v_unstable, None);
    }
}

/// Once per tick: players on the ground with their feet inside a big head
/// cell. Only the cells under the player's box are looked at, no world scan.
pub fn players_tilt(g: &mut Game) {
    let mut hits: Vec<(BlockPos, f64)> = Vec::new();
    for p in g.players.iter().flatten() {
        // spectators pass through blocks
        if !p.on_ground || p.gamemode == 3 {
            continue;
        }
        let feet = p.pos[1];
        let y = feet.floor();
        // the head cell is the one holding the feet; it needs feet above 0.6875
        if feet - y <= 0.6875 {
            continue;
        }
        // box of width 0.6, deflated a little on the high side
        let (x0, x1) = ((p.pos[0] - 0.3).floor() as i32, (p.pos[0] + 0.3 - 1e-5).floor() as i32);
        let (z0, z1) = ((p.pos[2] - 0.3).floor() as i32, (p.pos[2] + 0.3 - 1e-5).floor() as i32);
        for x in x0..=x1 {
            for z in z0..=z1 {
                // only big dripleaf heads matter: no work for players on slabs and the like
                if super::beh(g.world.get_block(x, y as i32, z)) == Beh::BigDripleaf {
                    hits.push((BlockPos::new(x, y as i32, z), feet));
                }
            }
        }
    }
    if !hits.is_empty() {
        crate::game::redstone::dripleaf_feet(g, &hits);
    }
}

// ------------------------------------------------------------ bone meal

/// `BigDripleafStemBlock.getTopConnectedBlock`: the head above a stem column.
fn top_head<L: Lvl>(l: &L, pos: BlockPos) -> Option<BlockPos> {
    let t = t();
    let mut p = pos;
    loop {
        p = up(p);
        let b = block_of(l.get(p));
        if b != t.stem {
            return (b == t.big).then_some(p);
        }
    }
}

/// `BigDripleafBlock.placeWithRandomHeight`: stems up from `bottom`, then a
/// head. Height 2 to 5 when the space is free; the cells are written
/// bottom-up with `setBlockAndUpdate`.
pub fn place_with_random_height<L: Lvl>(rs: &mut Rs, l: &mut L, bottom: BlockPos, facing: u8, rng: &mut Rng) {
    let desired = rng.range(2, 5);
    let mut p = bottom;
    let mut height = 0;
    while height < desired && can_grow_into(&*l, p) {
        height += 1;
        p = up(p);
    }
    let leaf_y = bottom.y + height - 1;
    let mut q = bottom;
    while q.y < leaf_y {
        let wl = water_source(l.get(q));
        engine::set_block(rs, l, q, stem_state(wl, facing), fl::ALL, UPDATE_LIMIT);
        q = up(q);
    }
    let wl = water_source(l.get(q));
    engine::set_block(rs, l, q, big_state(wl, facing), fl::ALL, UPDATE_LIMIT);
}

/// Bone meal on a dripleaf part: `isValidBonemealTarget` and `performBonemeal`
/// of the three blocks. Returns false when the block is not a valid target.
pub fn bonemeal<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) -> bool {
    let t = t();
    match super::beh(st) {
        Beh::BigDripleaf => {
            // `isValidBonemealTarget`: `canGrowInto(pos.above())`
            let above = up(pos);
            if !can_grow_into(&*l, above) {
                return false;
            }
            // `BigDripleafStemBlock.place(pos, state.fluid)`, then the head above
            let facing = data::get(st, data::P::Facing);
            engine::set_block(rs, l, pos, stem_state(water_source(st), facing), fl::ALL, UPDATE_LIMIT);
            engine::set_block(rs, l, above, big_state(water_source(l.get(above)), facing), fl::ALL, UPDATE_LIMIT);
            true
        }
        Beh::DripleafStem => {
            // `getTopConnectedBlock(pos, UP, BIG_DRIPLEAF)` and `canPlaceAt(head.above)`
            let Some(head) = top_head(&*l, pos) else { return false };
            let above = up(head);
            if !can_grow_into(&*l, above) {
                return false;
            }
            let facing = data::get(st, data::P::Facing);
            engine::set_block(rs, l, head, stem_state(water_source(l.get(head)), facing), fl::ALL, UPDATE_LIMIT);
            engine::set_block(rs, l, above, big_state(water_source(l.get(above)), facing), fl::ALL, UPDATE_LIMIT);
            true
        }
        Beh::SmallDripleaf => {
            // `isValidBonemealTarget` is always true
            if getv(st, Pr::Half) == t.v_lower {
                small_lower_bonemeal(rs, l, st, pos, rng);
            } else {
                // the upper half hands over to the lower one below it
                let below = down(pos);
                let bs = l.get(below);
                if block_of(bs) == t.small && getv(bs, Pr::Half) == t.v_lower {
                    small_lower_bonemeal(rs, l, bs, below, rng);
                }
            }
            true
        }
        _ => false,
    }
}

/// `SmallDripleafBlock.performBonemeal` on the lower half: the upper cell
/// becomes its fluid, then `placeWithRandomHeight` from this cell.
fn small_lower_bonemeal<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    let above = up(pos);
    engine::set_block(rs, l, above, legacy_fluid(l.get(above)), fl::CLIENTS | fl::KNOWN_SHAPE, UPDATE_LIMIT);
    place_with_random_height(rs, l, pos, data::get(st, data::P::Facing), rng);
}

#[cfg(test)]
mod tests {
    use super::super::testw::World as TW;
    use super::*;
    use super::super::props::seti;

    fn s(n: &str) -> u16 {
        default_state(n)
    }

    fn small_upper() -> u16 {
        setv(s("small_dripleaf"), Pr::Half, vid("upper"))
    }

    fn tilt_of(w: &TW, p: BlockPos) -> u8 {
        getv(w.get(p), Pr::Tilt)
    }

    fn ctx(yaw: f32) -> PlaceCtx {
        PlaceCtx { face: 1, cursor: (0.5, 0.0, 0.5), yaw, pitch: 0.0, sneaking: false, clicked: BlockPos::new(0, 63, 0) }
    }

    #[test]
    fn big_head_survives_on_support_head_and_stem_only() {
        let p = BlockPos::new(0, 64, 0);
        for (below, ok) in [("clay", true), ("moss_block", true), ("farmland", true), ("big_dripleaf", true), ("big_dripleaf_stem", true), ("stone", false), ("dirt_path", false)] {
            let mut w = TW::new();
            w.put(down(p), s(below));
            w.put(p, s("big_dripleaf"));
            assert_eq!(big_survives(&w, p), ok, "below {below}");
        }
    }

    #[test]
    fn small_plant_support_follows_tag_and_water() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = TW::new();
        w.put(down(p), s("clay"));
        assert!(small_lower_survives(&w, p));
        // dirt is only fine with water in the plant's own cell (SUPPORTS_VEGETATION)
        let mut w = TW::new();
        w.put(down(p), s("dirt"));
        assert!(!small_lower_survives(&w, p));
        w.put(p, s("water"));
        assert!(small_lower_survives(&w, p));
        // stone never supports it, even with water
        let mut w = TW::new();
        w.put(down(p), s("stone"));
        w.put(p, s("water"));
        assert!(!small_lower_survives(&w, p));
    }

    #[test]
    fn floor_removed_takes_both_halves_of_small_plant() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = TW::new();
        let mut rs = Rs::default();
        w.put(down(p), s("clay"));
        w.put(p, s("small_dripleaf"));
        w.put(up(p), small_upper());
        engine::set_block(&mut rs, &mut w, down(p), 0, fl::ALL, UPDATE_LIMIT);
        assert_eq!(w.get(p), 0);
        assert_eq!(w.get(up(p)), 0);
    }

    #[test]
    fn stem_breaks_one_tick_after_its_head_is_gone() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = TW::new();
        let mut rs = Rs::default();
        w.put(down(p), s("clay"));
        engine::set_block(&mut rs, &mut w, p, stem_state(false, 2), fl::ALL, UPDATE_LIMIT);
        engine::set_block(&mut rs, &mut w, up(p), big_state(false, 2), fl::ALL, UPDATE_LIMIT);
        assert_eq!(block_of(w.get(p)), block_of(s("big_dripleaf_stem")));
        // the head is removed: the stem is still there until its tick
        engine::set_block(&mut rs, &mut w, up(p), 0, fl::ALL, UPDATE_LIMIT);
        assert_eq!(block_of(w.get(p)), block_of(s("big_dripleaf_stem")));
        w.tick(&mut rs);
        assert_eq!(w.get(p), 0);
        assert!(w.dropped.iter().any(|(q, _)| *q == p));
    }

    #[test]
    fn head_on_a_new_head_turns_the_lower_one_into_a_stem() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = TW::new();
        let mut rs = Rs::default();
        w.put(down(p), s("clay"));
        engine::set_block(&mut rs, &mut w, p, big_state(false, 5), fl::ALL, UPDATE_LIMIT);
        engine::set_block(&mut rs, &mut w, up(p), big_state(false, 5), fl::ALL, UPDATE_LIMIT);
        assert_eq!(w.get(p), stem_state(false, 5));
        assert_eq!(block_of(w.get(up(p))), block_of(s("big_dripleaf")));
    }

    #[test]
    fn placement_faces_away_and_follows_the_stem_below() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = TW::new();
        w.put(down(p), s("clay"));
        // small plant: facing opposite the placer (yaw 0 looks south, so north)
        let st = placement(&w, s("small_dripleaf"), p, &ctx(0.0)).expect("placeable");
        assert_eq!(data::get(st, data::P::Facing), 2);
        let st = placement(&w, s("small_dripleaf"), p, &ctx(90.0)).expect("placeable");
        assert_eq!(data::get(st, data::P::Facing), 5);
        // big head on a stem keeps the stem's facing
        let mut w = TW::new();
        w.put(down(p), stem_state(false, 4));
        let st = placement(&w, s("big_dripleaf"), p, &ctx(0.0)).expect("placeable");
        assert_eq!(data::get(st, data::P::Facing), 4);
    }

    #[test]
    fn placement_refuses_small_plant_on_bare_dirt_and_under_a_wall() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = TW::new();
        w.put(down(p), s("dirt"));
        assert!(placement(&w, s("small_dripleaf"), p, &ctx(0.0)).is_none());
        w.put(down(p), s("clay"));
        w.put(up(p), s("stone"));
        assert!(placement(&w, s("small_dripleaf"), p, &ctx(0.0)).is_none());
    }

    #[test]
    fn placed_in_source_water_is_waterlogged() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = TW::new();
        w.put(down(p), s("clay"));
        w.put(p, s("water"));
        let st = placement(&w, s("big_dripleaf"), p, &ctx(0.0)).expect("placeable");
        assert!(getb(st, Pr::Waterlogged));
        // flowing water (level 3) is not a source
        w.put(p, seti(s("water"), Pr::Level, 3));
        let st = placement(&w, s("big_dripleaf"), p, &ctx(0.0)).expect("placeable");
        assert!(!getb(st, Pr::Waterlogged));
    }

    #[test]
    fn unstable_partial_full_then_back_with_vanilla_delays() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = TW::new();
        let mut rs = Rs::default();
        w.put(down(p), s("clay"));
        w.put(p, s("big_dripleaf"));
        tilt_feet(&mut rs, &mut w, &[(p, p.y as f64 + 0.9375)]);
        assert_eq!(tilt_of(&w, p), vid("unstable"));
        assert!(w.sounds.is_empty(), "unstable is silent");
        for _ in 0..9 {
            w.tick(&mut rs);
        }
        assert_eq!(tilt_of(&w, p), vid("unstable"));
        w.tick(&mut rs); // 10
        assert_eq!(tilt_of(&w, p), vid("partial"));
        for _ in 0..9 {
            w.tick(&mut rs);
        }
        assert_eq!(tilt_of(&w, p), vid("partial"));
        w.tick(&mut rs); // 20
        assert_eq!(tilt_of(&w, p), vid("full"));
        for _ in 0..99 {
            w.tick(&mut rs);
        }
        assert_eq!(tilt_of(&w, p), vid("full"));
        w.tick(&mut rs); // 120
        assert_eq!(tilt_of(&w, p), vid("none"));
        let names: Vec<&str> = w.sounds.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, [SOUND_TILT_DOWN, SOUND_TILT_DOWN, SOUND_TILT_UP]);
    }

    #[test]
    fn entity_tilts_only_above_the_threshold_and_not_twice() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = TW::new();
        let mut rs = Rs::default();
        w.put(down(p), s("clay"));
        w.put(p, s("big_dripleaf"));
        // exactly 0.6875 is not above it
        tilt_feet(&mut rs, &mut w, &[(p, 64.6875)]);
        assert_eq!(tilt_of(&w, p), vid("none"));
        tilt_feet(&mut rs, &mut w, &[(p, 64.7)]);
        assert_eq!(tilt_of(&w, p), vid("unstable"));
        // a second entity does not restart or change anything
        for _ in 0..5 {
            w.tick(&mut rs);
        }
        tilt_feet(&mut rs, &mut w, &[(p, 64.9)]);
        for _ in 0..5 {
            w.tick(&mut rs);
        }
        assert_eq!(tilt_of(&w, p), vid("partial"));
    }

    #[test]
    fn power_on_the_head_resets_the_tilt() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = TW::new();
        let mut rs = Rs::default();
        w.put(down(p), s("clay"));
        w.put(p, s("big_dripleaf"));
        tilt_feet(&mut rs, &mut w, &[(p, 64.9)]);
        for _ in 0..10 {
            w.tick(&mut rs);
        }
        assert_eq!(tilt_of(&w, p), vid("partial"));
        // a redstone block next to the head: the neighbor update resets it
        engine::set_block(&mut rs, &mut w, p.offset(1, 0, 0), s("redstone_block"), fl::ALL, UPDATE_LIMIT);
        assert_eq!(tilt_of(&w, p), vid("none"));
        assert_eq!(w.sounds.last().map(|(n, _)| n.as_str()), Some(SOUND_TILT_UP));
        // while powered, an entity does not tilt it
        tilt_feet(&mut rs, &mut w, &[(p, 64.9)]);
        assert_eq!(tilt_of(&w, p), vid("none"));
    }

    #[test]
    fn power_stops_the_scheduled_step_too() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = TW::new();
        let mut rs = Rs::default();
        w.put(down(p), s("clay"));
        w.put(p, s("big_dripleaf"));
        tilt_feet(&mut rs, &mut w, &[(p, 64.9)]);
        w.put(p.offset(0, 0, 1), s("redstone_block"));
        w.tick(&mut rs);
        for _ in 0..30 {
            w.tick(&mut rs);
        }
        assert_eq!(tilt_of(&w, p), vid("none"));
    }

    #[test]
    fn bonemeal_big_head_grows_one_block_up() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = TW::new();
        let mut rs = Rs::default();
        let mut rng = Rng::new(4);
        w.put(down(p), s("clay"));
        let head = big_state(false, 3);
        w.put(p, head);
        assert!(bonemeal(&mut rs, &mut w, head, p, &mut rng));
        assert_eq!(w.get(p), stem_state(false, 3));
        assert_eq!(w.get(up(p)), big_state(false, 3));
        // the new head has free air above: it can grow again
        let st = w.get(up(p));
        assert!(bonemeal(&mut rs, &mut w, st, up(p), &mut rng));
        assert_eq!(w.get(up(p)), stem_state(false, 3));
        assert_eq!(w.get(up(p).offset(0, 1, 0)), big_state(false, 3));
    }

    #[test]
    fn bonemeal_stem_grows_the_column_top() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = TW::new();
        let mut rs = Rs::default();
        let mut rng = Rng::new(4);
        w.put(down(p), s("clay"));
        w.put(p, stem_state(false, 2));
        w.put(up(p), big_state(false, 2));
        let stem = w.get(p);
        assert!(bonemeal(&mut rs, &mut w, stem, p, &mut rng));
        assert_eq!(w.get(p), stem_state(false, 2));
        assert_eq!(w.get(up(p)), stem_state(false, 2));
        assert_eq!(block_of(w.get(up(p).offset(0, 1, 0))), block_of(s("big_dripleaf")));
    }

    #[test]
    fn bonemeal_small_plant_makes_a_column_of_two_to_five() {
        let p = BlockPos::new(0, 64, 0);
        let mut lengths = std::collections::BTreeSet::new();
        for seed in 0..400u64 {
            let mut w = TW::new();
            let mut rs = Rs::default();
            let mut rng = Rng::new(seed);
            w.put(down(p), s("clay"));
            w.put(p, s("small_dripleaf"));
            w.put(up(p), small_upper());
            let st = w.get(p);
            assert!(bonemeal(&mut rs, &mut w, st, p, &mut rng));
            // count the stems from the bottom, then the head
            let mut n = 0;
            let mut q = p;
            while block_of(w.get(q)) == block_of(s("big_dripleaf_stem")) {
                assert_eq!(data::get(w.get(q), data::P::Facing), 2);
                n += 1;
                q = up(q);
            }
            assert_eq!(block_of(w.get(q)), block_of(s("big_dripleaf")), "seed {seed}");
            assert_eq!(data::get(w.get(q), data::P::Facing), 2);
            assert_eq!(w.get(up(q)), 0);
            assert_eq!(w.get(down(p)), s("clay"));
            lengths.insert(n + 1);
        }
        assert_eq!(lengths.into_iter().collect::<Vec<_>>(), vec![2, 3, 4, 5]);
    }

    #[test]
    fn bonemeal_small_plant_stops_under_a_ceiling() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = TW::new();
        let mut rs = Rs::default();
        let mut rng = Rng::new(7);
        w.put(down(p), s("clay"));
        w.put(p, s("small_dripleaf"));
        w.put(up(p), small_upper());
        w.put(up(up(p)), s("stone"));
        let st = w.get(p);
        assert!(bonemeal(&mut rs, &mut w, st, p, &mut rng));
        assert_eq!(block_of(w.get(p)), block_of(s("big_dripleaf_stem")));
        assert_eq!(block_of(w.get(up(p))), block_of(s("big_dripleaf")));
        assert_eq!(w.get(up(up(p))), s("stone"));
    }

    #[test]
    fn bonemeal_on_the_upper_half_works_on_the_lower_one() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = TW::new();
        let mut rs = Rs::default();
        let mut rng = Rng::new(11);
        w.put(down(p), s("clay"));
        w.put(p, s("small_dripleaf"));
        w.put(up(p), small_upper());
        let st = w.get(up(p));
        assert!(bonemeal(&mut rs, &mut w, st, up(p), &mut rng));
        assert_eq!(block_of(w.get(p)), block_of(s("big_dripleaf_stem")));
    }

    #[test]
    fn bonemeal_refused_when_head_has_no_room() {
        let p = BlockPos::new(0, 64, 0);
        let mut w = TW::new();
        let mut rs = Rs::default();
        let mut rng = Rng::new(2);
        w.put(down(p), s("clay"));
        w.put(p, big_state(false, 2));
        w.put(up(p), s("stone"));
        let st = w.get(p);
        assert!(!bonemeal(&mut rs, &mut w, st, p, &mut rng));
        assert_eq!(w.get(p), st);
    }

    #[test]
    fn small_fluid_legacy_matches_vanilla() {
        assert_eq!(legacy_fluid(0), 0);
        assert_eq!(legacy_fluid(s("water")), s("water"));
        assert_eq!(legacy_fluid(s("small_dripleaf")), 0);
        assert_eq!(legacy_fluid(stem_state(true, 2)), s("water"));
    }
}
