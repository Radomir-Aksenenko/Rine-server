//! Column and growing plants: `CactusBlock`, `SugarCaneBlock`,
//! `BambooStalkBlock` / `BambooSaplingBlock`, `GrowingPlantHeadBlock` and
//! `GrowingPlantBodyBlock` (kelp, weeping and twisting vines, cave vines),
//! `MushroomBlock` (spread only), `BuddingAmethystBlock`,
//! `AmethystClusterBlock`, `EyeblossomBlock` (open and close by time) and
//! `VineBlock` (random spread).
//!
//! Vanilla chances are written in their unmodified form (`nextInt(n) == 0`,
//! `nextDouble() < p`). Spigot modifiers at 100 reduce to those forms. Paper
//! config limits are replaced by the vanilla constants (cactus and reed height
//! 3, bamboo 16, bamboo stage 12 and 15).
//!
//! Not here: cactus damage to entities, water fluid ticks of waterlogged and
//! kelp blocks (fluid system), particles and game events, bone meal for huge
//! mushrooms, cave vine harvesting.

use super::props::{geti, getb, getv, seti, setb, setv, vid, Pr};
use super::{beh, block_id, block_of, block_tag, default_state, is_air, raw_brightness, rule_bool, Beh, PlaceCtx, Rng};
use crate::game::redstone::data::{self, clockwise, counter_clockwise, opposite, step, ALL, DOWN, EAST, HORIZONTAL, NORTH, SOUTH, UP, WEST};
use crate::game::redstone::engine::{self, fl, prio, Lvl, CAT_BLOCKS, UPDATE_LIMIT};
use crate::game::redstone::Rs;
use rine_data::blocks::flag;
use rine_protocol::BlockPos;
use rine_world::{MAX_Y, MIN_Y};
use std::sync::OnceLock;

/// Air state id (`Blocks.AIR.defaultBlockState()`).
const AIR: u16 = 0;

#[derive(Clone, Copy)]
enum Kind {
    /// `KelpBlock`: grows into water, attaches to anything not in `#cannot_support_kelp`.
    Kelp,
    /// `NetherVines`: weeping and twisting vines, grow into air, bone meal count.
    Nether,
    /// `CaveVinesBlock`: grows into air and rolls berries.
    Cave,
}

/// One growing plant family: head and body blocks with their defaults.
struct Plant {
    head: u16,
    body: u16,
    head_def: u16,
    body_def: u16,
    /// `growthDirection` as a direction id.
    dir: u8,
    /// `growPerTickProbability`.
    chance: f64,
    kind: Kind,
}

struct T {
    water: u16,
    lava: u16,
    cactus: u16,
    cactus_def: u16,
    cactus_flower_def: u16,
    sugar: u16,
    sugar_def: u16,
    bamboo: u16,
    bamboo_def: u16,
    sapling: u16,
    sapling_def: u16,
    /// kelp, weeping vines, twisting vines, cave vines
    plants: [Plant; 4],
    /// small, medium, large amethyst bud, amethyst cluster: (block, default state)
    buds: [(u16, u16); 4],
    open_eye: u16,
    open_eye_def: u16,
    closed_eye: u16,
    closed_eye_def: u16,
    vine: u16,
    vine_def: u16,
    supports_cactus: Vec<bool>,
    supports_sugar: Vec<bool>,
    sugar_adj: Vec<bool>,
    supports_bamboo: Vec<bool>,
    cannot_kelp: Vec<bool>,
    mush_override: Vec<bool>,
    leaves_none: u8,
    leaves_small: u8,
    leaves_large: u8,
}

fn t() -> &'static T {
    static T_: OnceLock<T> = OnceLock::new();
    T_.get_or_init(|| {
        let plant = |head: &str, body: &str, dir: u8, chance: f64, kind: Kind| Plant {
            head: block_id(head),
            body: block_id(body),
            head_def: default_state(head),
            body_def: default_state(body),
            dir,
            chance,
            kind,
        };
        let bud = |n: &str| (block_id(n), default_state(n));
        T {
            water: block_id("water"),
            lava: block_id("lava"),
            cactus: block_id("cactus"),
            cactus_def: default_state("cactus"),
            cactus_flower_def: default_state("cactus_flower"),
            sugar: block_id("sugar_cane"),
            sugar_def: default_state("sugar_cane"),
            bamboo: block_id("bamboo"),
            bamboo_def: default_state("bamboo"),
            sapling: block_id("bamboo_sapling"),
            sapling_def: default_state("bamboo_sapling"),
            plants: [
                plant("kelp", "kelp_plant", UP, 0.14, Kind::Kelp),
                plant("weeping_vines", "weeping_vines_plant", DOWN, 0.1, Kind::Nether),
                plant("twisting_vines", "twisting_vines_plant", UP, 0.1, Kind::Nether),
                plant("cave_vines", "cave_vines_plant", DOWN, 0.1, Kind::Cave),
            ],
            buds: [bud("small_amethyst_bud"), bud("medium_amethyst_bud"), bud("large_amethyst_bud"), bud("amethyst_cluster")],
            open_eye: block_id("open_eyeblossom"),
            open_eye_def: default_state("open_eyeblossom"),
            closed_eye: block_id("closed_eyeblossom"),
            closed_eye_def: default_state("closed_eyeblossom"),
            vine: block_id("vine"),
            vine_def: default_state("vine"),
            supports_cactus: block_tag("supports_cactus"),
            supports_sugar: block_tag("supports_sugar_cane"),
            sugar_adj: block_tag("supports_sugar_cane_adjacently"),
            supports_bamboo: block_tag("supports_bamboo"),
            cannot_kelp: block_tag("cannot_support_kelp"),
            mush_override: block_tag("overrides_mushroom_light_requirement"),
            leaves_none: vid("none"),
            leaves_small: vid("small"),
            leaves_large: vid("large"),
        }
    })
}

static RULE_SPREAD_VINES: OnceLock<Option<usize>> = OnceLock::new();

pub fn classify(n: &str) -> Option<Beh> {
    Some(match n {
        "cactus" => Beh::Cactus,
        "sugar_cane" => Beh::SugarCane,
        "bamboo" => Beh::Bamboo,
        "bamboo_sapling" => Beh::BambooSapling,
        "kelp" | "weeping_vines" | "twisting_vines" | "cave_vines" => Beh::GrowHead,
        "kelp_plant" | "weeping_vines_plant" | "twisting_vines_plant" | "cave_vines_plant" => Beh::GrowBody,
        "brown_mushroom" | "red_mushroom" => Beh::Mushroom,
        "budding_amethyst" => Beh::BuddingAmethyst,
        "amethyst_cluster" | "small_amethyst_bud" | "medium_amethyst_bud" | "large_amethyst_bud" => Beh::AmethystCluster,
        "open_eyeblossom" | "closed_eyeblossom" => Beh::Eyeblossom,
        "vine" => Beh::Vine,
        _ => return None,
    })
}

// ------------------------------------------------------------------ helpers

#[inline]
fn at(p: BlockPos, d: u8) -> BlockPos {
    let (x, y, z) = step(d);
    p.offset(x, y, z)
}

#[inline]
fn down(p: BlockPos, n: i32) -> BlockPos {
    p.offset(0, -n, 0)
}

#[inline]
fn up(p: BlockPos, n: i32) -> BlockPos {
    p.offset(0, n, 0)
}

#[inline]
fn has(st: u16, f: u16) -> bool {
    rine_data::get().blocks.has(st, f)
}

/// `Level.isInsideBuildHeight`.
#[inline]
fn inside(p: BlockPos) -> bool {
    (MIN_Y..=MAX_Y).contains(&p.y)
}

/// `FluidState.isFull()`: amount 8.
#[inline]
fn fluid_full(st: u16) -> bool {
    rine_data::get().blocks.state(st).fluid_level == 8
}

/// `state.is(Blocks.WATER) && isFull`, the test of `canClusterGrowAtState`.
#[inline]
fn full_water_block(st: u16) -> bool {
    block_of(st) == t().water && fluid_full(st)
}

/// `BlockState.liquid()` for the water and lava blocks (not waterlogged).
#[inline]
fn liquid_block(st: u16) -> bool {
    let b = block_of(st);
    b == t().water || b == t().lava
}

/// `Level.getRawBrightness(pos, 0) >= 9` style checks use `raw_brightness`.
#[inline]
fn fluid_water(st: u16) -> bool {
    has(st, flag::WATER)
}

/// Growing plant lookup: (plant, is_head). None for other blocks.
fn plant_of(blk: u16) -> Option<(&'static Plant, bool)> {
    for p in t().plants.iter() {
        if p.head == blk {
            return Some((p, true));
        }
        if p.body == blk {
            return Some((p, false));
        }
    }
    None
}

fn set_clients<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16) {
    engine::set_block(rs, l, pos, st, fl::CLIENTS, UPDATE_LIMIT);
}

fn set_all<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16) {
    engine::set_block(rs, l, pos, st, fl::ALL, UPDATE_LIMIT);
}

/// `setBlock(pos, st, UPDATE_NONE)`.
fn set_none<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16) {
    engine::set_block(rs, l, pos, st, 0, UPDATE_LIMIT);
}

/// Schedule the block's own tick in one tick when it cannot survive
/// (`ticks.scheduleTick(pos, this, 1)`).
fn schedule_if_dead<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, blk: u16, alive: bool) {
    if !alive {
        engine::schedule(rs, l, pos, blk, 1, prio::NORMAL);
    }
}

/// `level.destroyBlock(pos, true)` from a scheduled tick that found the block
/// unsupported.
fn break_if_dead<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, alive: bool) {
    if !alive {
        engine::destroy_block(rs, l, pos, true, UPDATE_LIMIT);
    }
}

/// Count of `blk` in the 9x3x9 box around `pos` is at most 4
/// (`findBlocksIn(...).atMostMatched(4)`).
fn spread_room<L: Lvl>(l: &L, pos: BlockPos, blk: u16) -> bool {
    let mut n = 0;
    for dx in -4..=4 {
        for dy in -1..=1 {
            for dz in -4..=4 {
                if block_of(l.get(pos.offset(dx, dy, dz))) == blk {
                    n += 1;
                    if n > 4 {
                        return false;
                    }
                }
            }
        }
    }
    true
}

// ------------------------------------------------------------------ cactus

/// `CactusBlock.canSurvive`.
fn cactus_can_survive<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    let t = t();
    for d in HORIZONTAL {
        let n = l.get(at(pos, d));
        if has(n, flag::SOLID) || has(n, flag::LAVA) {
            return false;
        }
    }
    let below = block_of(l.get(down(pos, 1)));
    (below == t.cactus || t.supports_cactus[below as usize]) && !liquid_block(l.get(up(pos, 1)))
}

/// `CactusBlock.randomTick`.
fn cactus_random<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    let t = t();
    let above = up(pos, 1);
    if !is_air(l.get(above)) {
        return;
    }
    let age = geti(st, Pr::Age);
    let mut height = 1;
    while block_of(l.get(down(pos, height))) == t.cactus {
        height += 1;
        if height == 3 && age == 15 {
            return;
        }
    }
    if age == 8 && cactus_can_survive(l, above) {
        let chance = if height >= 3 { 0.25 } else { 0.1 };
        if rng.next_double() <= chance {
            set_all(rs, l, above, t.cactus_flower_def);
        }
    } else if age == 15 && height < 3 {
        set_all(rs, l, above, t.cactus_def);
        set_none(rs, l, pos, seti(st, Pr::Age, 0));
    }
    if age < 15 {
        set_none(rs, l, pos, seti(st, Pr::Age, age + 1));
    }
}

fn cactus_update<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) -> u16 {
    schedule_if_dead(rs, l, pos, block_of(st), cactus_can_survive(l, pos));
    st
}

// ------------------------------------------------------------------ sugar cane

/// `SugarCaneBlock.canSurvive`.
fn sugar_can_survive<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    let t = t();
    let below = down(pos, 1);
    let bb = block_of(l.get(below));
    if bb == t.sugar {
        return true;
    }
    if t.supports_sugar[bb as usize] {
        for d in HORIZONTAL {
            let n = l.get(at(below, d));
            if fluid_water(n) || t.sugar_adj[block_of(n) as usize] {
                return true;
            }
        }
    }
    false
}

/// `SugarCaneBlock.randomTick` (modifier 100: no random calls).
fn sugar_random<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    let t = t();
    if !is_air(l.get(up(pos, 1))) {
        return;
    }
    let mut height = 1;
    while block_of(l.get(down(pos, height))) == t.sugar {
        height += 1;
    }
    if height < 3 {
        let age = geti(st, Pr::Age);
        if age >= 15 {
            set_all(rs, l, up(pos, 1), t.sugar_def);
            set_none(rs, l, pos, seti(st, Pr::Age, 0));
        } else {
            set_none(rs, l, pos, seti(st, Pr::Age, age + 1));
        }
    }
}

// ------------------------------------------------------------------ bamboo

/// `BambooStalkBlock.canSurvive`.
fn bamboo_can_survive<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    t().supports_bamboo[block_of(l.get(down(pos, 1))) as usize]
}

/// `getHeightAboveUpToMax` with the vanilla max of 16.
fn bamboo_above<L: Lvl>(l: &L, pos: BlockPos) -> i32 {
    let mut h = 0;
    while h < 16 && block_of(l.get(up(pos, h + 1))) == t().bamboo {
        h += 1;
    }
    h
}

/// `getHeightBelowUpToMax` with the vanilla max of 16.
fn bamboo_below<L: Lvl>(l: &L, pos: BlockPos) -> i32 {
    let mut h = 0;
    while h < 16 && block_of(l.get(down(pos, h + 1))) == t().bamboo {
        h += 1;
    }
    h
}

/// `BambooStalkBlock.growBamboo`: the new top at `pos + 1`, then the leaves
/// of the stalks below when a large leaf moves down.
fn bamboo_grow<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng, height: i32) {
    let t = t();
    let below_st = l.get(down(pos, 1));
    let two_pos = down(pos, 2);
    let two_st = l.get(two_pos);
    let mut leaves = t.leaves_none;
    let mut others = false;
    if height >= 1 {
        if block_of(below_st) != t.bamboo || getv(below_st, Pr::Leaves) == t.leaves_none {
            leaves = t.leaves_small;
        } else {
            leaves = t.leaves_large;
            if block_of(two_st) == t.bamboo {
                others = true;
            }
        }
    }
    let age = if geti(st, Pr::Age) != 1 && block_of(two_st) != t.bamboo { 0 } else { 1 };
    // `height < 12 || !(nextFloat < 0.25)`, then `height != 15`
    let stage = if (height < 12 || !(rng.next_float() < 0.25)) && height != 15 { 0 } else { 1 };
    let new = seti(seti(setv(t.bamboo_def, Pr::Leaves, leaves), Pr::Age, age), Pr::Stage, stage);
    set_all(rs, l, up(pos, 1), new);
    if others {
        set_all(rs, l, down(pos, 1), setv(below_st, Pr::Leaves, t.leaves_small));
        set_all(rs, l, two_pos, setv(two_st, Pr::Leaves, t.leaves_none));
    }
}

/// `BambooStalkBlock.randomTick`.
fn bamboo_random<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    if geti(st, Pr::Stage) != 0 {
        return;
    }
    if rng.next_int(3) == 0 && is_air(l.get(up(pos, 1))) && raw_brightness(l, up(pos, 1)) >= 9 {
        let height = bamboo_below(l, pos) + 1;
        if height < 16 {
            bamboo_grow(rs, l, st, pos, rng, height);
        }
    }
}

/// `BambooStalkBlock.updateShape`: the age follows a taller stalk above.
fn bamboo_update<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, dir: u8, nst: u16) -> u16 {
    schedule_if_dead(rs, l, pos, block_of(st), bamboo_can_survive(l, pos));
    if dir == UP && block_of(nst) == t().bamboo && geti(nst, Pr::Age) > geti(st, Pr::Age) {
        seti(st, Pr::Age, (geti(st, Pr::Age) + 1) % 2)
    } else {
        st
    }
}

/// `BambooStalkBlock.getStateForPlacement`.
fn bamboo_placement<L: Lvl>(l: &L, pos: BlockPos) -> Option<u16> {
    let t = t();
    let here = l.get(pos);
    if has(here, flag::WATER) || has(here, flag::LAVA) {
        return None;
    }
    let below = l.get(down(pos, 1));
    let bb = block_of(below);
    if !t.supports_bamboo[bb as usize] {
        return None;
    }
    if bb == t.sapling {
        return Some(seti(t.bamboo_def, Pr::Age, 0));
    }
    if bb == t.bamboo {
        let age = if geti(below, Pr::Age) > 0 { 1 } else { 0 };
        return Some(seti(t.bamboo_def, Pr::Age, age));
    }
    let above = l.get(up(pos, 1));
    if block_of(above) == t.bamboo {
        return Some(seti(t.bamboo_def, Pr::Age, geti(above, Pr::Age)));
    }
    Some(t.sapling_def)
}

/// `BambooSaplingBlock.growBamboo`.
fn sapling_grow<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos) {
    let t = t();
    set_all(rs, l, up(pos, 1), setv(t.bamboo_def, Pr::Leaves, t.leaves_small));
}

/// `BambooSaplingBlock.randomTick`.
fn sapling_random<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, rng: &mut Rng) {
    if rng.next_int(3) == 0 && is_air(l.get(up(pos, 1))) && raw_brightness(l, up(pos, 1)) >= 9 {
        sapling_grow(rs, l, pos);
    }
}

/// `BambooSaplingBlock.updateShape`: air when unsupported (no tick), bamboo under a stalk.
fn sapling_update<L: Lvl>(l: &L, st: u16, pos: BlockPos, dir: u8, nst: u16) -> u16 {
    if !t().supports_bamboo[block_of(l.get(down(pos, 1))) as usize] {
        return AIR;
    }
    if dir == UP && block_of(nst) == t().bamboo {
        t().bamboo_def
    } else {
        st
    }
}

// ------------------------------------------------------------------ mushroom

/// `MushroomBlock.canSurvive`. `mayPlaceOn` is `isSolidRender`, approximated
/// by an opaque block with a sturdy top face.
fn mush_can_survive<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    let below = l.get(down(pos, 1));
    if t().mush_override[block_of(below) as usize] {
        return true;
    }
    raw_brightness(l, pos) < 13 && has(below, flag::OPAQUE) && data::sturdy(below, UP)
}

/// Offset of `MushroomBlock.randomTick`: x, then the two y draws, then z.
fn mush_offset(p: BlockPos, rng: &mut Rng) -> BlockPos {
    let x = rng.next_int(3) - 1;
    let y = rng.next_int(2) - rng.next_int(2);
    let z = rng.next_int(3) - 1;
    p.offset(x, y, z)
}

/// `MushroomBlock.randomTick`.
fn mush_random<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    if rng.next_int(25) != 0 {
        return;
    }
    if !spread_room(l, pos, block_of(st)) {
        return;
    }
    let mut p = pos;
    let mut off = mush_offset(p, rng);
    for _ in 0..4 {
        if is_air(l.get(off)) && mush_can_survive(l, off) {
            p = off;
        }
        off = mush_offset(p, rng);
    }
    if is_air(l.get(off)) && mush_can_survive(l, off) {
        set_clients(rs, l, off, st);
    }
}

// ------------------------------------------------------------------ amethyst

/// Index of a bud or cluster block in `T::buds`.
fn bud_index(blk: u16) -> Option<usize> {
    t().buds.iter().position(|b| b.0 == blk)
}

/// `BuddingAmethystBlock.canClusterGrowAtState`.
fn can_cluster_grow(st: u16) -> bool {
    is_air(st) || full_water_block(st)
}

/// `BuddingAmethystBlock.randomTick`.
fn budding_random<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, rng: &mut Rng) {
    if rng.next_int(5) != 0 {
        return;
    }
    let dir = ALL[rng.next_int(6) as usize];
    let gp = at(pos, dir);
    let rst = l.get(gp);
    let next = if can_cluster_grow(rst) {
        Some(0)
    } else {
        match bud_index(block_of(rst)) {
            Some(k) if k < 3 && data::get(rst, data::P::Facing) == dir => Some(k + 1),
            _ => None,
        }
    };
    if let Some(k) = next {
        let s = data::set(t().buds[k].1, data::P::Facing, dir);
        let s = setb(s, Pr::Waterlogged, fluid_water(rst));
        set_all(rs, l, gp, s);
    }
}

/// `AmethystClusterBlock.canSurvive`.
fn cluster_can_survive<L: Lvl>(l: &L, st: u16, pos: BlockPos) -> bool {
    let f = data::get(st, data::P::Facing);
    data::sturdy(l.get(at(pos, opposite(f))), f)
}

/// `AmethystClusterBlock.updateShape` (the waterlogged fluid tick is not here).
fn cluster_update<L: Lvl>(l: &L, st: u16, pos: BlockPos, dir: u8) -> u16 {
    if dir == opposite(data::get(st, data::P::Facing)) && !cluster_can_survive(l, st, pos) {
        AIR
    } else {
        st
    }
}

// ------------------------------------------------------------------ eyeblossom

/// `EyeblossomBlock.tryChangingState`. The open time is the `minecraft:day`
/// timeline track `gameplay/eyeblossom_open`: TRUE from 12600 up to 23401.
/// Returns the new open flag when the blossom changed.
fn eye_try<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) -> Option<bool> {
    let t = t();
    let clock = l.day_time().rem_euclid(24000);
    let should_open = (12600..23401).contains(&clock);
    let is_open = block_of(st) == t.open_eye;
    if should_open == is_open {
        return None;
    }
    let new_def = if should_open { t.open_eye_def } else { t.closed_eye_def };
    set_all(rs, l, pos, new_def);
    // nearby blossoms of the old state, scanned x, y, z
    let blk = block_of(st);
    for x in -3..=3 {
        for y in -2..=2 {
            for z in -3..=3 {
                let np = pos.offset(x, y, z);
                if l.get(np) != st {
                    continue;
                }
                let dist = ((x * x + y * y + z * z) as f64).sqrt();
                let lo = (dist * 5.0) as i32;
                let hi = (dist * 10.0) as i32;
                let delay = lo + rng.next_int(hi - lo + 1);
                engine::schedule(rs, l, np, blk, delay as u32, prio::NORMAL);
            }
        }
    }
    Some(should_open)
}

/// `FlowerPotBlock.randomTick` for potted eyeblossoms: swap open/closed by the
/// same `eyeblossom_open` attribute, long switch sound (transform particles
/// are not sent, as for the planted blossom).
pub fn potted_eye_random_tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    let (open, closed) = potted_eyes();
    let blk = block_of(st);
    if blk != open && blk != closed {
        return;
    }
    let is_open = blk == open;
    let clock = l.day_time().rem_euclid(24000);
    let should_open = (12600..23401).contains(&clock);
    if is_open == should_open {
        return;
    }
    let to = if is_open { super::default_state("potted_closed_eyeblossom") } else { super::default_state("potted_open_eyeblossom") };
    set_all(rs, l, pos, to);
    eye_sound(l, pos, !is_open, true);
}

fn potted_eyes() -> (u16, u16) {
    static P: OnceLock<(u16, u16)> = OnceLock::new();
    *P.get_or_init(|| (super::block_id("potted_open_eyeblossom"), super::block_id("potted_closed_eyeblossom")))
}

fn eye_sound<L: Lvl>(l: &mut L, pos: BlockPos, open: bool, long: bool) {
    let name = match (open, long) {
        (true, true) => "block.eyeblossom.open_long",
        (true, false) => "block.eyeblossom.open",
        (false, true) => "block.eyeblossom.close_long",
        (false, false) => "block.eyeblossom.close",
    };
    l.sound(name, pos, 1.0, 1.0, CAT_BLOCKS, false);
}

// ------------------------------------------------------------------ vines

fn face_pr(d: u8) -> Pr {
    match d {
        UP => Pr::Up,
        NORTH => Pr::North,
        EAST => Pr::East,
        SOUTH => Pr::South,
        _ => Pr::West,
    }
}

fn faces_count(st: u16) -> u32 {
    [Pr::Up, Pr::North, Pr::East, Pr::South, Pr::West].iter().filter(|&&p| getb(st, p)).count() as u32
}

fn horiz_any(st: u16) -> bool {
    getb(st, Pr::North) || getb(st, Pr::East) || getb(st, Pr::South) || getb(st, Pr::West)
}

/// `MultifaceBlock.canAttachTo`: the neighbour's face toward us is sturdy.
fn vine_acceptable<L: Lvl>(l: &L, npos: BlockPos, dir: u8) -> bool {
    data::sturdy(l.get(npos), opposite(dir))
}

/// `VineBlock.canSupportAtFace`.
fn vine_support_face<L: Lvl>(l: &L, pos: BlockPos, dir: u8) -> bool {
    if dir == DOWN {
        return false;
    }
    if vine_acceptable(l, at(pos, dir), dir) {
        return true;
    }
    if dir == UP {
        return false;
    }
    let above = l.get(up(pos, 1));
    block_of(above) == t().vine && getb(above, face_pr(dir))
}

/// `VineBlock.getUpdatedState`.
fn vine_updated<L: Lvl>(l: &L, st: u16, pos: BlockPos) -> u16 {
    let above = up(pos, 1);
    let mut s = st;
    if getb(s, Pr::Up) {
        s = setb(s, Pr::Up, vine_acceptable(l, above, DOWN));
    }
    let mut above_st: Option<u16> = None;
    for d in HORIZONTAL {
        let f = face_pr(d);
        if getb(s, f) {
            let mut can = vine_support_face(l, pos, d);
            if !can {
                let a = match above_st {
                    Some(a) => a,
                    None => {
                        let a = l.get(above);
                        above_st = Some(a);
                        a
                    }
                };
                can = block_of(a) == t().vine && getb(a, f);
            }
            s = setb(s, f, can);
        }
    }
    s
}

/// `VineBlock.randomTick`, branch by branch in vanilla order.
fn vine_random<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    if !rule_bool(l, &RULE_SPREAD_VINES, "spread_vines", true) {
        return;
    }
    if rng.next_int(4) != 0 {
        return;
    }
    let test = ALL[rng.next_int(6) as usize];
    let above = up(pos, 1);
    if HORIZONTAL.contains(&test) && !getb(st, face_pr(test)) {
        if !spread_room(l, pos, t().vine) {
            return;
        }
        let test_pos = at(pos, test);
        if is_air(l.get(test_pos)) {
            let cw = clockwise(test);
            let ccw = counter_clockwise(test);
            let cw_has = getb(st, face_pr(cw));
            let ccw_has = getb(st, face_pr(ccw));
            let cw_test = at(test_pos, cw);
            let ccw_test = at(test_pos, ccw);
            if cw_has && vine_acceptable(l, cw_test, cw) {
                set_clients(rs, l, test_pos, setb(t().vine_def, face_pr(cw), true));
            } else if ccw_has && vine_acceptable(l, ccw_test, ccw) {
                set_clients(rs, l, test_pos, setb(t().vine_def, face_pr(ccw), true));
            } else {
                let opp = opposite(test);
                if cw_has && is_air(l.get(cw_test)) && vine_acceptable(l, at(pos, cw), opp) {
                    set_clients(rs, l, cw_test, setb(t().vine_def, face_pr(opp), true));
                } else if ccw_has && is_air(l.get(ccw_test)) && vine_acceptable(l, at(pos, ccw), opp) {
                    set_clients(rs, l, ccw_test, setb(t().vine_def, face_pr(opp), true));
                } else if rng.next_float() < 0.05 && vine_acceptable(l, up(test_pos, 1), UP) {
                    set_clients(rs, l, test_pos, setb(t().vine_def, Pr::Up, true));
                }
            }
        } else if vine_acceptable(l, test_pos, test) {
            set_clients(rs, l, pos, setb(st, face_pr(test), true));
        }
    } else {
        if test == UP && pos.y < MAX_Y {
            if vine_support_face(l, pos, UP) {
                set_clients(rs, l, pos, setb(st, Pr::Up, true));
                return;
            }
            if is_air(l.get(above)) {
                if !spread_room(l, pos, t().vine) {
                    return;
                }
                let mut a = st;
                for d in HORIZONTAL {
                    if rng.next_bool() || !vine_acceptable(l, at(above, d), d) {
                        a = setb(a, face_pr(d), false);
                    }
                }
                if horiz_any(a) {
                    set_clients(rs, l, above, a);
                }
                return;
            }
        }
        if pos.y > MIN_Y {
            let below = down(pos, 1);
            let bst = l.get(below);
            if is_air(bst) || block_of(bst) == t().vine {
                let before = if is_air(bst) { t().vine_def } else { bst };
                let mut after = before;
                for d in HORIZONTAL {
                    if rng.next_bool() && getb(st, face_pr(d)) {
                        after = setb(after, face_pr(d), true);
                    }
                }
                if before != after && horiz_any(after) {
                    set_clients(rs, l, below, after);
                }
            }
        }
    }
}

/// `Direction.orderedByNearest(entity)` for the placing player's view.
fn ordered_by_nearest(yaw: f32, pitch: f32) -> [u8; 6] {
    let deg = std::f32::consts::PI / 180.0;
    let pitch = pitch * deg;
    let yaw = -yaw * deg;
    let (ps, pc) = (pitch.sin(), pitch.cos());
    let (ys, yc) = (yaw.sin(), yaw.cos());
    let x_pos = ys > 0.0;
    let y_pos = ps < 0.0;
    let z_pos = yc > 0.0;
    let x_yaw = if x_pos { ys } else { -ys };
    let y_mag = if y_pos { -ps } else { ps };
    let z_yaw = if z_pos { yc } else { -yc };
    let x_mag = x_yaw * pc;
    let z_mag = z_yaw * pc;
    let ax = if x_pos { EAST } else { WEST };
    let ay = if y_pos { UP } else { DOWN };
    let az = if z_pos { SOUTH } else { NORTH };
    let mk = |a: u8, b: u8, c: u8| [a, b, c, opposite(c), opposite(b), opposite(a)];
    if x_yaw > z_yaw {
        if y_mag > x_mag {
            mk(ay, ax, az)
        } else if z_mag > y_mag {
            mk(ax, az, ay)
        } else {
            mk(ax, ay, az)
        }
    } else if y_mag > z_mag {
        mk(ay, az, ax)
    } else if x_mag > y_mag {
        mk(az, ax, ay)
    } else {
        mk(az, ay, ax)
    }
}

/// `VineBlock.getStateForPlacement`, with `getNearestLookingDirections`
/// (the clicked face's opposite first, unless the clicked block is replaced).
fn vine_placement<L: Lvl>(l: &L, pos: BlockPos, ctx: &PlaceCtx) -> Option<u16> {
    let t = t();
    let clicked = l.get(pos);
    let clicked_vine = block_of(clicked) == t.vine;
    let result = if clicked_vine { clicked } else { t.vine_def };
    let mut dirs = ordered_by_nearest(ctx.yaw, ctx.pitch);
    if ctx.clicked != pos {
        let want = opposite(ctx.face);
        if let Some(i) = dirs.iter().position(|&d| d == want) {
            dirs.copy_within(0..i, 1);
            dirs[0] = want;
        }
    }
    for d in dirs {
        if d == DOWN {
            continue;
        }
        let f = face_pr(d);
        let occupied = clicked_vine && getb(clicked, f);
        if !occupied && vine_support_face(l, pos, d) {
            return Some(setb(result, f, true));
        }
    }
    if clicked_vine {
        Some(result)
    } else {
        None
    }
}

// ------------------------------------------------------------------ growing plants

/// `GrowingPlantBlock.canAttachTo`.
fn gp_can_attach(p: &Plant, att: u16) -> bool {
    match p.kind {
        Kind::Kelp => !t().cannot_kelp[block_of(att) as usize],
        _ => true,
    }
}

/// `canGrowInto`: kelp into water, vines and cave vines into air.
fn gp_can_grow_into(p: &Plant, st: u16) -> bool {
    match p.kind {
        Kind::Kelp => block_of(st) == t().water,
        _ => is_air(st),
    }
}

/// `GrowingPlantBlock.canSurvive`.
fn gp_can_survive<L: Lvl>(l: &L, p: &Plant, pos: BlockPos) -> bool {
    let att = l.get(at(pos, opposite(p.dir)));
    let b = block_of(att);
    gp_can_attach(p, att) && (b == p.head || b == p.body || data::sturdy(att, p.dir))
}

/// `CaveVinesBlock.updateBodyAfterConvertedFromHead` and
/// `CaveVinesPlantBlock.updateHeadAfterConvertedFromBody` copy the berries.
fn body_from_head(head_st: u16, p: &Plant) -> u16 {
    match p.kind {
        Kind::Cave => setb(p.body_def, Pr::Berries, getb(head_st, Pr::Berries)),
        _ => p.body_def,
    }
}

fn head_from_body(body_st: u16, p: &Plant, age: i32) -> u16 {
    let h = seti(p.head_def, Pr::Age, age);
    match p.kind {
        Kind::Cave => setb(h, Pr::Berries, getb(body_st, Pr::Berries)),
        _ => h,
    }
}

/// `GrowingPlantHeadBlock.getGrowIntoState`: age plus one, and for cave vines
/// a new berries roll (`nextFloat() < 0.11`).
fn grow_into(p: &Plant, st: u16, rng: &mut Rng) -> u16 {
    let s = seti(st, Pr::Age, geti(st, Pr::Age) + 1);
    match p.kind {
        Kind::Cave => setb(s, Pr::Berries, rng.next_float() < 0.11),
        _ => s,
    }
}

/// `GrowingPlantHeadBlock.randomTick`.
fn grow_head_random<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    let Some((p, _)) = plant_of(block_of(st)) else { return };
    if geti(st, Pr::Age) < 25 && rng.next_double() < p.chance {
        let gp = at(pos, p.dir);
        if gp_can_grow_into(p, l.get(gp)) {
            let ns = grow_into(p, st, rng);
            set_all(rs, l, gp, ns);
        }
    }
}

/// `GrowingPlantHeadBlock.updateShape` and `GrowingPlantBodyBlock.updateShape`
/// share the structure, with the body and head branches split here.
fn grow_head_update<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, dir: u8, nst: u16) -> u16 {
    let Some((p, _)) = plant_of(block_of(st)) else { return st };
    if dir == opposite(p.dir) {
        let alive = gp_can_survive(l, p, pos);
        schedule_if_dead(rs, l, pos, block_of(st), alive);
        if alive {
            let nb = block_of(l.get(at(pos, p.dir)));
            if nb == p.head || nb == p.body {
                return body_from_head(st, p);
            }
        }
    }
    let nb = block_of(nst);
    if dir != p.dir || !(nb == p.head || nb == p.body) {
        return st;
    }
    body_from_head(st, p)
}

fn grow_body_update<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, dir: u8, nst: u16) -> u16 {
    let Some((p, _)) = plant_of(block_of(st)) else { return st };
    if dir == opposite(p.dir) {
        let alive = gp_can_survive(l, p, pos);
        schedule_if_dead(rs, l, pos, block_of(st), alive);
    }
    let nb = block_of(nst);
    if dir == p.dir && nb != p.body && nb != p.head {
        let age = rs.rng.next_int(25);
        return head_from_body(st, p, age);
    }
    st
}

/// `GrowingPlantBlock.getStateForPlacement`, with the kelp water check.
fn grow_placement<L: Lvl>(l: &L, st: u16, pos: BlockPos) -> Option<u16> {
    let (p, is_head) = plant_of(block_of(st))?;
    if is_head && matches!(p.kind, Kind::Kelp) {
        let here = l.get(pos);
        if !(fluid_water(here) && fluid_full(here)) {
            return None;
        }
    }
    let gs = block_of(l.get(at(pos, p.dir)));
    if (gs == p.head || gs == p.body) || !is_head {
        return Some(p.body_def);
    }
    let age = Rng::derive(l.time(), pos.x as i64, ((pos.y as i64) << 32) ^ pos.z as i64).next_int(25);
    Some(seti(p.head_def, Pr::Age, age))
}

/// `GrowingPlantHeadBlock.getBlocksToGrowWhenBonemealed`.
fn blocks_to_grow(p: &Plant, rng: &mut Rng) -> i32 {
    match p.kind {
        Kind::Kelp | Kind::Cave => 1,
        Kind::Nether => {
            // `NetherVines.getBlocksToGrowWhenBonemealed`
            let mut prob = 1.0f64;
            let mut count = 0;
            while rng.next_double() < prob {
                count += 1;
                prob *= 0.826;
            }
            count
        }
    }
}

/// `GrowingPlantHeadBlock.performBonemeal`.
fn head_bonemeal<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, p: &Plant, rng: &mut Rng) {
    let mut fwd = at(pos, p.dir);
    let mut next_age = (geti(st, Pr::Age) + 1).min(25);
    let n = blocks_to_grow(p, rng);
    let mut i = 0;
    while i < n && gp_can_grow_into(p, l.get(fwd)) && inside(fwd) {
        set_all(rs, l, fwd, seti(st, Pr::Age, next_age));
        fwd = at(fwd, p.dir);
        next_age = (next_age + 1).min(25);
        i += 1;
    }
}

/// `BlockUtil.getTopConnectedBlock`: the first non-body block along the growth
/// direction, if it is the head.
fn top_connected<L: Lvl>(l: &L, pos: BlockPos, p: &Plant) -> Option<BlockPos> {
    let mut fp = at(pos, p.dir);
    while block_of(l.get(fp)) == p.body {
        fp = at(fp, p.dir);
    }
    if block_of(l.get(fp)) == p.head {
        Some(fp)
    } else {
        None
    }
}

// ------------------------------------------------------------------ dispatch

/// `Block.getStateForPlacement` corrections of this family.
pub fn placement<L: Lvl>(l: &L, st: u16, pos: BlockPos, ctx: &PlaceCtx) -> Option<u16> {
    match beh(st) {
        Beh::Bamboo => bamboo_placement(l, pos),
        Beh::GrowHead | Beh::GrowBody => grow_placement(l, st, pos),
        Beh::Vine => vine_placement(l, pos, ctx),
        Beh::AmethystCluster => Some(setb(data::set(st, data::P::Facing, ctx.face), Pr::Waterlogged, fluid_water(l.get(pos)))),
        _ => Some(st),
    }
}

/// `randomTick` of this family.
pub fn random_tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    match beh(st) {
        Beh::Cactus => cactus_random(rs, l, st, pos, rng),
        Beh::SugarCane => sugar_random(rs, l, st, pos),
        Beh::Bamboo => bamboo_random(rs, l, st, pos, rng),
        Beh::BambooSapling => sapling_random(rs, l, pos, rng),
        Beh::GrowHead => grow_head_random(rs, l, st, pos, rng),
        Beh::Mushroom => mush_random(rs, l, st, pos, rng),
        Beh::BuddingAmethyst => budding_random(rs, l, pos, rng),
        Beh::Eyeblossom => {
            if let Some(open) = eye_try(rs, l, st, pos, rng) {
                eye_sound(l, pos, open, true);
            }
        }
        Beh::Vine => vine_random(rs, l, st, pos, rng),
        _ => {}
    }
}

/// `updateShape` of this family.
pub fn update_shape<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, dir: u8, _npos: BlockPos, nst: u16) -> u16 {
    match beh(st) {
        Beh::Cactus => cactus_update(rs, l, st, pos),
        Beh::SugarCane => {
            schedule_if_dead(rs, l, pos, block_of(st), sugar_can_survive(l, pos));
            st
        }
        Beh::Bamboo => bamboo_update(rs, l, st, pos, dir, nst),
        Beh::BambooSapling => sapling_update(l, st, pos, dir, nst),
        Beh::GrowHead => grow_head_update(rs, l, st, pos, dir, nst),
        Beh::GrowBody => grow_body_update(rs, l, st, pos, dir, nst),
        Beh::Mushroom => {
            if mush_can_survive(l, pos) {
                st
            } else {
                AIR
            }
        }
        Beh::AmethystCluster => cluster_update(l, st, pos, dir),
        Beh::Vine => {
            if dir == DOWN {
                return st;
            }
            let b = vine_updated(l, st, pos);
            if faces_count(b) > 0 {
                b
            } else {
                AIR
            }
        }
        _ => st,
    }
}

/// Scheduled tick of this family: break the unsupported plants.
pub fn tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    match beh(st) {
        Beh::Cactus => break_if_dead(rs, l, pos, cactus_can_survive(l, pos)),
        Beh::SugarCane => break_if_dead(rs, l, pos, sugar_can_survive(l, pos)),
        Beh::Bamboo => break_if_dead(rs, l, pos, bamboo_can_survive(l, pos)),
        Beh::GrowHead | Beh::GrowBody => {
            if let Some((p, _)) = plant_of(block_of(st)) {
                break_if_dead(rs, l, pos, gp_can_survive(l, p, pos));
            }
        }
        Beh::Eyeblossom => {
            let mut rng = rs.rng;
            let r = eye_try(rs, l, st, pos, &mut rng);
            rs.rng = rng;
            if let Some(open) = r {
                eye_sound(l, pos, open, false);
            }
        }
        _ => {}
    }
}

/// Bone meal on a block of this family (`isValidBonemealTarget` +
/// `performBonemeal`). Returns false when it is not a valid target.
pub fn bonemeal<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) -> bool {
    match beh(st) {
        Beh::GrowHead | Beh::GrowBody => {
            let Some((p, is_head)) = plant_of(block_of(st)) else { return false };
            if matches!(p.kind, Kind::Cave) {
                // `CaveVines.isValidBonemealTarget`: no berries yet
                if has_berries(st) {
                    return false;
                }
                set_clients(rs, l, pos, setb(st, Pr::Berries, true));
                return true;
            }
            if is_head {
                let gp = at(pos, p.dir);
                if !(gp_can_grow_into(p, l.get(gp)) && inside(gp)) {
                    return false;
                }
                head_bonemeal(rs, l, st, pos, p, rng);
                return true;
            }
            let Some(hp) = top_connected(l, pos, p) else { return false };
            let hs = l.get(hp);
            let gp = at(hp, p.dir);
            if !(gp_can_grow_into(p, l.get(gp)) && inside(gp)) {
                return false;
            }
            head_bonemeal(rs, l, hs, hp, p, rng);
            true
        }
        Beh::Bamboo => bamboo_bonemeal(rs, l, pos, rng),
        Beh::BambooSapling => {
            let above = up(pos, 1);
            if !(is_air(l.get(above)) && inside(above)) {
                return false;
            }
            sapling_grow(rs, l, pos);
            true
        }
        _ => false,
    }
}

fn has_berries(st: u16) -> bool {
    getb(st, Pr::Berries)
}

/// `BambooStalkBlock.isValidBonemealTarget` and `performBonemeal`.
fn bamboo_bonemeal<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, rng: &mut Rng) -> bool {
    let ha = bamboo_above(l, pos);
    let hb = bamboo_below(l, pos);
    let gp = up(pos, ha + 1);
    let top_stage = geti(l.get(up(pos, ha)), Pr::Stage);
    if !(ha + hb + 1 < 16 && top_stage != 1 && inside(gp) && is_air(l.get(gp))) {
        return false;
    }
    let mut above = ha;
    let mut total = ha + hb + 1;
    let new_count = 1 + rng.next_int(2);
    for _ in 0..new_count {
        let top = up(pos, above);
        let ts = l.get(top);
        let g = up(top, 1);
        if total >= 16 || block_of(ts) != t().bamboo || geti(ts, Pr::Stage) == 1 || !is_air(l.get(g)) || !inside(g) {
            return true;
        }
        bamboo_grow(rs, l, ts, top, rng, total);
        above += 1;
        total += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::super::testw::World as TW;
    use super::*;

    fn s(n: &str) -> u16 {
        default_state(n)
    }

    fn cell(w: &TW, p: BlockPos) -> u16 {
        w.get(p)
    }

    /// Count of `name` blocks in the box x -6..=6, y 63..=65, z -6..=6.
    fn count(w: &TW, name: &str) -> usize {
        let b = block_id(name);
        let mut n = 0;
        for x in -6..=6 {
            for y in 63..=65 {
                for z in -6..=6 {
                    if block_of(w.get(BlockPos::new(x, y, z))) == b {
                        n += 1;
                    }
                }
            }
        }
        n
    }

    #[test]
    fn sugar_cane_grows_to_three_and_stops() {
        let mut w = TW::new();
        w.put(BlockPos::new(0, 63, 0), s("sand"));
        w.put(BlockPos::new(0, 64, 0), s("sugar_cane"));
        let mut rs = Rs::default();
        let mut rng = Rng::new(1);
        for _ in 0..20000 {
            for y in 64..=67 {
                let p = BlockPos::new(0, y, 0);
                let st = w.get(p);
                if beh(st) == Beh::SugarCane {
                    random_tick(&mut rs, &mut w, st, p, &mut rng);
                }
            }
        }
        assert_eq!(block_of(w.get(BlockPos::new(0, 66, 0))), block_id("sugar_cane"));
        assert_eq!(w.get(BlockPos::new(0, 67, 0)), 0);
    }

    #[test]
    fn cactus_breaks_next_to_a_solid_block() {
        let mut w = TW::new();
        let p = BlockPos::new(0, 64, 0);
        w.put(BlockPos::new(0, 63, 0), s("sand"));
        w.put(p, s("cactus"));
        let mut rs = Rs::default();
        // a solid block beside the cactus: the cactus schedules a tick
        engine::set_block(&mut rs, &mut w, BlockPos::new(1, 64, 0), s("stone"), fl::ALL, UPDATE_LIMIT);
        w.tick(&mut rs);
        assert_eq!(cell(&w, p), 0);
        assert_eq!(w.dropped.len(), 1);
    }

    #[test]
    fn weeping_vines_grow_down_with_tenth_chance_and_cap_at_25() {
        let p = BlockPos::new(0, 70, 0);
        let below = BlockPos::new(0, 69, 0);
        let mut grew = 0;
        for seed in 0..2000u64 {
            let mut w = TW::new();
            w.put(p, s("weeping_vines"));
            let mut rs = Rs::default();
            let mut rng = Rng::new(seed + 1);
            let st = w.get(p);
            random_tick(&mut rs, &mut w, st, p, &mut rng);
            if block_of(w.get(below)) == block_id("weeping_vines") {
                assert_eq!(geti(w.get(below), Pr::Age), 1);
                assert_eq!(block_of(w.get(p)), block_id("weeping_vines_plant"));
                grew += 1;
            }
        }
        assert!((150..250).contains(&grew), "{grew}");
        // age 25 never grows
        let mut w = TW::new();
        w.put(p, seti(s("weeping_vines"), Pr::Age, 25));
        let mut rs = Rs::default();
        let mut rng = Rng::new(9);
        for _ in 0..5000 {
            let st = w.get(p);
            random_tick(&mut rs, &mut w, st, p, &mut rng);
        }
        assert_eq!(w.get(below), 0);
    }

    #[test]
    fn bonemeal_extends_weeping_vines() {
        let p = BlockPos::new(0, 70, 0);
        let mut w = TW::new();
        w.put(p, s("weeping_vines"));
        let mut rs = Rs::default();
        let mut rng = Rng::new(4);
        let st = w.get(p);
        assert!(bonemeal(&mut rs, &mut w, st, p, &mut rng));
        // each new head turns the one above into a body (performBonemeal order):
        // the lowest head is the last one grown, its age is the number of steps
        assert_eq!(block_of(w.get(p)), block_id("weeping_vines_plant"));
        let mut y = 69;
        while y > 60 && block_of(w.get(BlockPos::new(0, y, 0))) == block_id("weeping_vines_plant") {
            y -= 1;
        }
        let lowest = BlockPos::new(0, y, 0);
        assert_eq!(block_of(w.get(lowest)), block_id("weeping_vines"));
        assert_eq!(geti(w.get(lowest), Pr::Age), 70 - y);
    }

    #[test]
    fn bamboo_grows_and_sets_leaves() {
        let mut w = TW::new();
        w.light = 15;
        w.put(BlockPos::new(0, 62, 0), s("sand"));
        w.put(BlockPos::new(0, 63, 0), s("bamboo"));
        let mut rs = Rs::default();
        let mut rng = Rng::new(21);
        let grow = |w: &mut TW, rs: &mut Rs, rng: &mut Rng, pos: BlockPos, target: BlockPos| {
            for _ in 0..20000 {
                if block_of(w.get(target)) == block_id("bamboo") {
                    return;
                }
                let st = w.get(pos);
                random_tick(rs, w, st, pos, rng);
            }
        };
        // the new stalk takes its leaves from the stalk two below its base
        // (`growBamboo`: below is the stalk being grown)
        grow(&mut w, &mut rs, &mut rng, BlockPos::new(0, 63, 0), BlockPos::new(0, 64, 0));
        assert_eq!(getv(w.get(BlockPos::new(0, 64, 0)), Pr::Leaves), vid("small"));
        grow(&mut w, &mut rs, &mut rng, BlockPos::new(0, 64, 0), BlockPos::new(0, 65, 0));
        assert_eq!(getv(w.get(BlockPos::new(0, 65, 0)), Pr::Leaves), vid("small"));
        // stalk 64 now has leaves: stalk 66 is large and moves the leaves of 63 and 64
        grow(&mut w, &mut rs, &mut rng, BlockPos::new(0, 65, 0), BlockPos::new(0, 66, 0));
        assert_eq!(getv(w.get(BlockPos::new(0, 66, 0)), Pr::Leaves), vid("large"));
        assert_eq!(getv(w.get(BlockPos::new(0, 64, 0)), Pr::Leaves), vid("small"));
        assert_eq!(getv(w.get(BlockPos::new(0, 63, 0)), Pr::Leaves), vid("none"));
    }

    #[test]
    fn budding_amethyst_grows_a_bud_facing_the_free_side() {
        let mut w = TW::new();
        w.light = 15;
        let p = BlockPos::new(0, 64, 0);
        w.put(p, s("budding_amethyst"));
        let mut rs = Rs::default();
        let mut rng = Rng::new(31);
        for _ in 0..2000 {
            let st = w.get(p);
            random_tick(&mut rs, &mut w, st, p, &mut rng);
        }
        // buds grow through small, medium, large and the cluster: all keep their facing
        let stages = [block_id("small_amethyst_bud"), block_id("medium_amethyst_bud"), block_id("large_amethyst_bud"), block_id("amethyst_cluster")];
        let mut buds = 0;
        for d in ALL {
            let q = at(p, d);
            let st = w.get(q);
            if stages.contains(&block_of(st)) {
                buds += 1;
                assert_eq!(data::get(st, data::P::Facing), d);
            }
        }
        assert!(buds >= 1);


    }

    #[test]
    fn mushroom_does_not_spread_in_bright_light() {
        let mut w = TW::new();
        w.light = 15;
        for x in -6..=6 {
            for z in -6..=6 {
                w.put(BlockPos::new(x, 63, z), s("stone"));
            }
        }
        let p = BlockPos::new(0, 64, 0);
        w.put(p, s("brown_mushroom"));
        let before = count(&w, "brown_mushroom");
        let mut rs = Rs::default();
        let mut rng = Rng::new(41);
        for _ in 0..20000 {
            let st = w.get(p);
            random_tick(&mut rs, &mut w, st, p, &mut rng);
        }
        assert_eq!(count(&w, "brown_mushroom"), before);
    }

    #[test]
    fn mushroom_spreads_in_the_dark_and_stops_at_five_per_box() {
        let mut w = TW::new();
        w.light = 0;
        for x in -6..=6 {
            for z in -6..=6 {
                w.put(BlockPos::new(x, 63, z), s("stone"));
            }
        }
        let p = BlockPos::new(0, 64, 0);
        w.put(p, s("brown_mushroom"));
        let mut rs = Rs::default();
        let mut rng = Rng::new(42);
        for _ in 0..20000 {
            let st = w.get(p);
            random_tick(&mut rs, &mut w, st, p, &mut rng);
        }
        assert!(count(&w, "brown_mushroom") > 1);

        // five mushrooms in the 9x3x9 box: no spread from any of them
        let mut w = TW::new();
        w.light = 0;
        for x in -6..=6 {
            for z in -6..=6 {
                w.put(BlockPos::new(x, 63, z), s("stone"));
            }
        }
        for q in [p, BlockPos::new(1, 64, 0), BlockPos::new(2, 64, 0), BlockPos::new(0, 64, 1), BlockPos::new(0, 64, 2)] {
            w.put(q, s("brown_mushroom"));
        }
        let before = count(&w, "brown_mushroom");
        for _ in 0..20000 {
            for q in [p, BlockPos::new(1, 64, 0), BlockPos::new(2, 64, 0), BlockPos::new(0, 64, 1), BlockPos::new(0, 64, 2)] {
                let st = w.get(q);
                random_tick(&mut rs, &mut w, st, q, &mut rng);
            }
        }
        assert_eq!(count(&w, "brown_mushroom"), before);
    }

    #[test]
    fn vine_spreads_to_a_free_side() {
        let mut w = TW::new();
        let p = BlockPos::new(0, 64, 0);
        w.put(BlockPos::new(0, 64, -1), s("stone"));
        w.put(p, setb(s("vine"), Pr::North, true));
        let mut rs = Rs::default();
        let mut rng = Rng::new(51);
        for _ in 0..4000 {
            let st = w.get(p);
            random_tick(&mut rs, &mut w, st, p, &mut rng);
        }
        let mut vines = 0;
        for x in -3..=3 {
            for z in -3..=3 {
                if block_of(w.get(BlockPos::new(x, 64, z))) == block_id("vine") {
                    vines += 1;
                }
            }
        }
        assert!(vines > 1, "{vines}");
    }

    #[test]
    fn blossom_closes_at_day_and_wakes_its_neighbour() {
        let mut w = TW::new();
        w.day_time = 0;
        let p = BlockPos::new(0, 64, 0);
        let q = BlockPos::new(2, 64, 0);
        w.put(p, s("open_eyeblossom"));
        w.put(q, s("open_eyeblossom"));
        let mut rs = Rs::default();
        let mut rng = Rng::new(61);
        let st = w.get(p);
        random_tick(&mut rs, &mut w, st, p, &mut rng);
        assert_eq!(w.get(p), s("closed_eyeblossom"));
        assert_eq!(w.get(q), s("open_eyeblossom"));
        for _ in 0..25 {
            w.tick(&mut rs);
        }
        assert_eq!(w.get(q), s("closed_eyeblossom"));
    }

    #[test]
    fn potted_eyeblossom_follows_the_clock() {
        let mut rs = Rs::default();
        let p = BlockPos::new(0, 64, 0);
        let mut w = TW::new();
        let closed = super::super::default_state("potted_closed_eyeblossom");
        w.put(p, closed);
        assert!(rine_data::get().blocks.has(closed, rine_data::blocks::flag::RANDOM_TICK), "sampled by random ticks");
        w.day_time = 6000;
        potted_eye_random_tick(&mut rs, &mut w, closed, p);
        assert_eq!(w.get(p), closed, "day: stays closed");
        w.day_time = 13000;
        potted_eye_random_tick(&mut rs, &mut w, closed, p);
        assert_eq!(w.get(p), super::super::default_state("potted_open_eyeblossom"));
    }
}