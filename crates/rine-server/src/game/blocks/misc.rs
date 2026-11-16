//! Miscellaneous ticking blocks: `RedStoneOreBlock`, `FarmlandBlock`,
//! `TurtleEggBlock`, `SnifferEggBlock`, `DriedGhastBlock`, `ChorusFlowerBlock`
//! and `ChorusPlantBlock`, `SpeleothemBlock` (pointed dripstone, sulfur
//! spike), `AbstractCauldronBlock.tick` for drip filling, `MangrovePropaguleBlock`
//! (hanging stage only) and `SporeBlossomBlock` (survival only).
//!
//! Random ticks, scheduled ticks and `updateShape` follow the vanilla
//! classes. Entities that vanilla spawns (turtles, sniffers, happy ghasts)
//! are marked `TODO(entities)`. Falling stalactites use `entity::spawn_falling`
//! when the level has a game behind it.

use super::props::{geti, getb, getv, seti, setb, setv, vid, Pr};
use super::{block_id, block_of, block_tag, default_state, is_air, Beh, Rng};
use crate::game::redstone::data::{self, step, DOWN, HORIZONTAL, UP};
use crate::game::redstone::engine::{self, fl, prio, Lvl, UPDATE_LIMIT};
use crate::game::redstone::Rs;
use rine_data::blocks::flag;
use rine_protocol::BlockPos;
use rine_world::{MAX_Y, MIN_Y};
use std::sync::OnceLock;

// `LevelEvent` ids
const EV_CHORUS_GROW: i32 = 1033;
const EV_CHORUS_DEATH: i32 = 1034;
const EV_DRIP_LAVA_INTO_CAULDRON: i32 = 1046;
const EV_DRIP_WATER_INTO_CAULDRON: i32 = 1047;
const EV_DRIPSTONE_DRIP: i32 = 1504;
const EV_EGG_CRACK: i32 = 3009;

/// Fluid of a dripping stalactite (`Fluid` as the drip code sees it).
const NO_FLUID: u8 = 0;
const WATER_F: u8 = 1;
const LAVA_F: u8 = 2;

struct T {
    redstone_ore: [u16; 2],
    farmland: u16,
    dirt: u16,
    turtle_egg: u16,
    sniffer_egg: u16,
    dried_ghast: u16,
    chorus_flower: u16,
    chorus_plant: u16,
    flower_def: u16,
    plant_def: u16,
    pointed: u16,
    sulfur_spike: u16,
    pointed_def: u16,
    sulfur_spike_def: u16,
    dripstone_block: u16,
    sulfur: u16,
    propagule: u16,
    spore: u16,
    cauldron: u16,
    water_cauldron: u16,
    lava_cauldron: u16,
    water: u16,
    mud: u16,
    clay: u16,
    sand: Vec<bool>,
    maintains: Vec<bool>,
    supports_flower: Vec<bool>,
    supports_plant: Vec<bool>,
    speleothems: Vec<bool>,
    sniffer_boost: Vec<bool>,
    v_up: u8,
    v_down: u8,
    v_tip: u8,
    v_tip_merge: u8,
    v_frustum: u8,
    v_middle: u8,
    v_base: u8,
}

fn t() -> &'static T {
    static T_: OnceLock<T> = OnceLock::new();
    T_.get_or_init(|| T {
        redstone_ore: [block_id("redstone_ore"), block_id("deepslate_redstone_ore")],
        farmland: block_id("farmland"),
        dirt: block_id("dirt"),
        turtle_egg: block_id("turtle_egg"),
        sniffer_egg: block_id("sniffer_egg"),
        dried_ghast: block_id("dried_ghast"),
        chorus_flower: block_id("chorus_flower"),
        chorus_plant: block_id("chorus_plant"),
        flower_def: default_state("chorus_flower"),
        plant_def: default_state("chorus_plant"),
        pointed: block_id("pointed_dripstone"),
        sulfur_spike: block_id("sulfur_spike"),
        pointed_def: default_state("pointed_dripstone"),
        sulfur_spike_def: default_state("sulfur_spike"),
        dripstone_block: block_id("dripstone_block"),
        sulfur: block_id("sulfur"),
        propagule: block_id("mangrove_propagule"),
        spore: block_id("spore_blossom"),
        cauldron: block_id("cauldron"),
        water_cauldron: block_id("water_cauldron"),
        lava_cauldron: block_id("lava_cauldron"),
        water: block_id("water"),
        mud: block_id("mud"),
        clay: block_id("clay"),
        sand: block_tag("sand"),
        maintains: block_tag("maintains_farmland"),
        supports_flower: block_tag("supports_chorus_flower"),
        supports_plant: block_tag("supports_chorus_plant"),
        speleothems: block_tag("speleothems"),
        sniffer_boost: block_tag("sniffer_egg_hatch_boost"),
        v_up: vid("up"),
        v_down: vid("down"),
        v_tip: vid("tip"),
        v_tip_merge: vid("tip_merge"),
        v_frustum: vid("frustum"),
        v_middle: vid("middle"),
        v_base: vid("base"),
    })
}

pub fn classify(n: &str) -> Option<Beh> {
    Some(match n {
        "redstone_ore" | "deepslate_redstone_ore" => Beh::RedstoneOre,
        "farmland" => Beh::Farmland,
        "turtle_egg" => Beh::TurtleEgg,
        "sniffer_egg" => Beh::SnifferEgg,
        "dried_ghast" => Beh::DriedGhast,
        "chorus_flower" => Beh::ChorusFlower,
        "chorus_plant" => Beh::ChorusPlant,
        "pointed_dripstone" | "sulfur_spike" => Beh::Speleothem,
        "mangrove_propagule" => Beh::Propagule,
        "spore_blossom" => Beh::SporeBlossom,
        "cauldron" | "water_cauldron" => Beh::Cauldron,
        _ => return None,
    })
}

// ------------------------------------------------------------------ helpers

#[inline]
fn has(st: u16, f: u16) -> bool {
    rine_data::get().blocks.has(st, f)
}

#[inline]
fn rel(p: BlockPos, d: u8) -> BlockPos {
    let (dx, dy, dz) = step(d);
    p.offset(dx, dy, dz)
}

#[inline]
fn below(p: BlockPos) -> BlockPos {
    p.offset(0, -1, 0)
}

#[inline]
fn above(p: BlockPos) -> BlockPos {
    p.offset(0, 1, 0)
}

/// Property holding the connection towards a direction (`PROPERTY_BY_DIRECTION`).
fn dir_prop(d: u8) -> Pr {
    match d {
        UP => Pr::Up,
        DOWN => Pr::Down,
        data::NORTH => Pr::North,
        data::SOUTH => Pr::South,
        data::WEST => Pr::West,
        _ => Pr::East,
    }
}

fn set_clients<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16) {
    engine::set_block(rs, l, pos, st, fl::CLIENTS, UPDATE_LIMIT);
}

fn set_all<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16) {
    engine::set_block(rs, l, pos, st, fl::ALL, UPDATE_LIMIT);
}

// ------------------------------------------------------------------ redstone ore

/// `RedStoneOreBlock.interact`: lit ore on touch, attack or use. Particles are
/// client-only, so the server only sets `lit`.
pub fn redstone_ore_touch<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    if !getb(st, Pr::Lit) {
        set_all(rs, l, pos, setb(st, Pr::Lit, true));
    }
}

// ------------------------------------------------------------------ farmland

/// `FarmlandBlock.isNearWater`: water in the 9 x 2 x 9 box, x and z within 4, y in 0..=1.
fn near_water<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    for dz in -4..=4 {
        for dx in -4..=4 {
            for dy in 0..=1 {
                if has(l.get(pos.offset(dx, dy, dz)), flag::WATER) {
                    return true;
                }
            }
        }
    }
    false
}

/// `Level.isRainingAt`, without biome precipitation (Rine's biome data has none):
/// raining and no non-air block from `pos` up to the build limit.
fn raining_at<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    if !l.raining() {
        return false;
    }
    let mut y = pos.y;
    while y <= MAX_Y {
        if !is_air(l.get(BlockPos::new(pos.x, y, pos.z))) {
            return false;
        }
        y += 1;
    }
    true
}

/// `shouldMaintainFarmland`: the block above is in `#maintains_farmland`.
fn maintains<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    t().maintains[block_of(l.get(above(pos))) as usize]
}

/// `FarmlandBlock.canSurvive`.
fn farmland_can_survive<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    let a = l.get(above(pos));
    !has(a, flag::SOLID) || maintains(l, pos)
}

/// `FarmlandBlock.turnToBaseBlock`. Vanilla pushes entities above the block
/// first (`Block.pushEntitiesUp`); Rine has no entity push here yet.
fn turn_to_dirt<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos) {
    set_all(rs, l, pos, default_state("dirt"));
}

/// `FarmlandBlock.fallOn` trample: the caller decides when a fall tramples
/// (entity weight, fall distance, mob griefing). This is only the change.
pub fn trample<L: Lvl>(rs: &mut Rs, l: &mut L, _st: u16, pos: BlockPos) {
    turn_to_dirt(rs, l, pos);
}

fn farmland_random_tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    let moisture = geti(st, Pr::Moisture);
    if !near_water(l, pos) && !raining_at(l, above(pos)) {
        if moisture > 0 {
            set_clients(rs, l, pos, seti(st, Pr::Moisture, moisture - 1));
        } else if !maintains(l, pos) {
            turn_to_dirt(rs, l, pos);
        }
    } else if moisture < 7 {
        set_clients(rs, l, pos, seti(st, Pr::Moisture, 7));
    }
}

// ------------------------------------------------------------------ turtle and sniffer eggs

/// `TurtleEggBlock.shouldUpdateHatchLevel` chance from the
/// `gameplay/turtle_egg_hatch_chance` timeline: constant 1.0 between 21062
/// and 21905, else 0.002 (the `maximum` modifier keeps the default).
pub fn turtle_hatch_chance(day_time: i64) -> f32 {
    let tick = day_time.rem_euclid(24000);
    if (21062..21905).contains(&tick) {
        1.0
    } else {
        0.002
    }
}

fn turtle_random_tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    let chance = turtle_hatch_chance(l.day_time());
    // vanilla: `shouldUpdateHatchLevel(level, pos) && onSand(level, pos)`
    let roll = chance > 0.0 && rng.next_float() < chance;
    if !roll || !t().sand[block_of(l.get(below(pos))) as usize] {
        return;
    }
    let hatch = geti(st, Pr::Hatch);
    if hatch < 2 {
        set_clients(rs, l, pos, seti(st, Pr::Hatch, hatch + 1));
    } else {
        // TODO(entities): one baby turtle per egg (age -24000, home at pos)
        engine::remove_block(rs, l, pos, false);
    }
}

/// `TurtleEggBlock.decreaseEggs`: one egg less, the block goes at the last one.
/// Called by the interaction code when a player or mob breaks or tramples eggs.
pub fn turtle_egg_decrease<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    let n = geti(st, Pr::Eggs);
    if n <= 1 {
        engine::destroy_block(rs, l, pos, false, UPDATE_LIMIT);
    } else {
        set_clients(rs, l, pos, seti(st, Pr::Eggs, n - 1));
    }
}

/// `SnifferEggBlock.onPlace`: schedule the first progression tick. Vanilla runs
/// it on every state change, so each hatch stage reschedules itself.
pub fn on_place<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    let boosted = t().sniffer_boost[block_of(l.get(below(pos))) as usize];
    if boosted {
        l.level_event(EV_EGG_CRACK, pos, 0);
    }
    let hatch_time: u32 = if boosted { 12000 } else { 24000 };
    let delay = hatch_time / 3 + rs.rng.next_int(300) as u32;
    engine::schedule(rs, l, pos, block_of(st), delay, prio::NORMAL);
}

fn sniffer_tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    if geti(st, Pr::Hatch) != 2 {
        set_clients(rs, l, pos, seti(st, Pr::Hatch, geti(st, Pr::Hatch) + 1));
    } else {
        // TODO(entities): baby sniffer at the block centre
        engine::destroy_block(rs, l, pos, false, UPDATE_LIMIT);
    }
}

// ------------------------------------------------------------------ dried ghast

fn ghast_random_tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    if (getb(st, Pr::Waterlogged) || geti(st, Pr::Hydration) > 0) && !engine::has_scheduled(rs, pos, block_of(st)) {
        engine::schedule(rs, l, pos, block_of(st), 5000, prio::NORMAL);
    }
}

fn ghast_tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    let h = geti(st, Pr::Hydration);
    if getb(st, Pr::Waterlogged) {
        if h != 3 {
            set_clients(rs, l, pos, seti(st, Pr::Hydration, h + 1));
        } else {
            // TODO(entities): happy ghast baby (`spawnGhastling`); the block is removed
            engine::remove_block(rs, l, pos, false);
        }
    } else if h > 0 {
        set_clients(rs, l, pos, seti(st, Pr::Hydration, h - 1));
    }
}

// ------------------------------------------------------------------ chorus

fn plant_or_flower(st: u16) -> bool {
    let t = t();
    block_of(st) == t.chorus_plant || block_of(st) == t.chorus_flower
}

/// `ChorusPlantBlock.getStateWithConnections`: connections computed from the
/// six neighbours of `pos`, starting from `base` (the plant's default state).
pub fn chorus_connections<L: Lvl>(l: &L, pos: BlockPos, base: u16) -> u16 {
    let t = t();
    let down = l.get(below(pos));
    let mut s = setb(base, Pr::Down, plant_or_flower(down) || t.supports_plant[block_of(down) as usize]);
    s = setb(s, Pr::Up, plant_or_flower(l.get(above(pos))));
    for d in HORIZONTAL {
        s = setb(s, dir_prop(d), plant_or_flower(l.get(rel(pos, d))));
    }
    s
}

/// `ChorusPlantBlock.getStateForPlacement` (Paper's option off).
pub fn chorus_placement<L: Lvl>(l: &L, st: u16, pos: BlockPos) -> u16 {
    chorus_connections(l, pos, st)
}

/// `ChorusPlantBlock.canSurvive`.
fn plant_can_survive<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    let t = t();
    let me = t.chorus_plant;
    let bl = l.get(below(pos));
    let both = !is_air(l.get(above(pos))) && !is_air(bl);
    for d in HORIZONTAL {
        let np = rel(pos, d);
        if block_of(l.get(np)) == me {
            if both {
                return false;
            }
            let nb = block_of(l.get(below(np)));
            if nb == me || t.supports_plant[nb as usize] {
                return true;
            }
        }
    }
    block_of(bl) == me || t.supports_plant[block_of(bl) as usize]
}

/// `ChorusFlowerBlock.canSurvive`.
fn flower_can_survive<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    let t = t();
    let bl = l.get(below(pos));
    if block_of(bl) == t.chorus_plant || t.supports_flower[block_of(bl) as usize] {
        return true;
    }
    if !is_air(bl) {
        return false;
    }
    let mut one = false;
    for d in HORIZONTAL {
        let n = l.get(rel(pos, d));
        if block_of(n) == t.chorus_plant {
            if one {
                return false;
            }
            one = true;
        } else if !is_air(n) {
            return false;
        }
    }
    one
}

/// `ChorusFlowerBlock.allNeighborsEmpty`.
fn all_neighbors_empty<L: Lvl>(l: &L, pos: BlockPos, ignore: Option<u8>) -> bool {
    for d in HORIZONTAL {
        if Some(d) == ignore {
            continue;
        }
        if !is_air(l.get(rel(pos, d))) {
            return false;
        }
    }
    true
}

/// `ChorusFlowerBlock.randomTick`: grow up, branch sideways, or die.
fn flower_random_tick<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, rng: &mut Rng) {
    let t = t();
    let up = above(pos);
    if !is_air(l.get(up)) || up.y > MAX_Y {
        return;
    }
    let age = geti(l.get(pos), Pr::Age);
    if age >= 5 {
        return;
    }
    let bl = l.get(below(pos));
    let bb = block_of(bl);
    let mut grow_upwards = false;
    let mut pillar_on_support = false;
    if t.supports_flower[bb as usize] {
        grow_upwards = true;
    } else if bb == t.chorus_plant {
        let mut height = 1;
        for _ in 0..4 {
            let test = l.get(pos.offset(0, -(height + 1), 0));
            if block_of(test) != t.chorus_plant {
                if t.supports_flower[block_of(test) as usize] {
                    pillar_on_support = true;
                }
                break;
            }
            height += 1;
        }
        if height < 2 || height <= rng.next_int(if pillar_on_support { 5 } else { 4 }) {
            grow_upwards = true;
        }
    } else if is_air(bl) {
        grow_upwards = true;
    }

    if grow_upwards && all_neighbors_empty(l, up, None) && is_air(l.get(pos.offset(0, 2, 0))) {
        // spread the new flower first, then turn this block into the plant
        set_clients(rs, l, up, seti(t.flower_def, Pr::Age, age));
        l.level_event(EV_CHORUS_GROW, up, 0);
        set_clients(rs, l, pos, chorus_connections(l, pos, t.plant_def));
    } else if age < 4 {
        let mut attempts = rng.next_int(4);
        if pillar_on_support {
            attempts += 1;
        }
        let mut created = false;
        for _ in 0..attempts {
            let d = HORIZONTAL[rng.next_int(4) as usize];
            let target = rel(pos, d);
            if is_air(l.get(target)) && is_air(l.get(below(target))) && all_neighbors_empty(l, target, Some(data::opposite(d))) {
                set_clients(rs, l, target, seti(t.flower_def, Pr::Age, age + 1));
                l.level_event(EV_CHORUS_GROW, target, 0);
                created = true;
            }
        }
        if created {
            set_clients(rs, l, pos, chorus_connections(l, pos, t.plant_def));
        } else {
            set_clients(rs, l, pos, seti(t.flower_def, Pr::Age, 5));
            l.level_event(EV_CHORUS_DEATH, pos, 0);
        }
    } else {
        set_clients(rs, l, pos, seti(t.flower_def, Pr::Age, 5));
        l.level_event(EV_CHORUS_DEATH, pos, 0);
    }
}

// ------------------------------------------------------------------ spore blossom

/// `SporeBlossomBlock.canSurvive`: a sturdy ceiling and no water.
fn spore_can_survive<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    data::sturdy_center(l.get(above(pos)), DOWN) && !has(l.get(pos), flag::WATER)
}

// ------------------------------------------------------------------ mangrove propagule

/// `MangrovePropaguleBlock.randomTick`, hanging stage. Tree growth is not here:
/// `advanceTree` belongs to the sapling family.
const SAPLING_CHANCE: f32 = 100.0 / (100.0 * 7.0);

fn propagule_random_tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    if !getb(st, Pr::Hanging) {
        if rng.next_float() < SAPLING_CHANCE {
            // TODO(sapling): advanceTree (tree growth, another task)
        }
    } else {
        let age = geti(st, Pr::Age);
        if age < 4 {
            set_clients(rs, l, pos, seti(st, Pr::Age, age + 1));
        }
    }
}

// ------------------------------------------------------------------ speleothems

fn is_speleothem(st: u16) -> bool {
    t().speleothems[block_of(st) as usize]
}

/// `BlockStateProperties.VERTICAL_DIRECTION` as a `Direction` (UP or DOWN).
fn tip_dir(st: u16) -> u8 {
    if getv(st, Pr::VerticalDirection) == t().v_up {
        UP
    } else {
        DOWN
    }
}

fn is_sp_dir(st: u16, d: u8) -> bool {
    is_speleothem(st) && tip_dir(st) == d
}

fn is_stalactite(st: u16) -> bool {
    is_sp_dir(st, DOWN)
}

fn is_stalagmite(st: u16) -> bool {
    is_sp_dir(st, UP)
}

fn is_tip(st: u16, merged: bool) -> bool {
    if !is_speleothem(st) {
        return false;
    }
    let th = getv(st, Pr::Thickness);
    th == t().v_tip || merged && th == t().v_tip_merge
}

/// `SpeleothemBlock.isFreeHangingStalactite`.
fn is_free_hanging(st: u16) -> bool {
    is_stalactite(st) && getv(st, Pr::Thickness) == t().v_tip && !getb(st, Pr::Waterlogged)
}

fn is_unmerged_tip(st: u16, d: u8, me: u16) -> bool {
    is_tip(st, false) && tip_dir(st) == d && block_of(st) == me
}

fn dy_of(d: u8) -> i32 {
    if d == UP {
        1
    } else {
        -1
    }
}

/// `SpeleothemBlock.findBlockVertical`: walk `max - 1` steps, the target is
/// checked before the path predicate, and leaving the build height ends the walk.
fn walk<L: Lvl>(l: &L, pos: BlockPos, dy: i32, max: i32, path: impl Fn(u16) -> bool, target: impl Fn(u16) -> bool) -> Option<BlockPos> {
    let mut p = pos;
    for _ in 1..max {
        p = p.offset(0, dy, 0);
        let s = l.get(p);
        if target(s) {
            return Some(p);
        }
        if p.y < MIN_Y || p.y > MAX_Y || !path(s) {
            return None;
        }
    }
    None
}

/// `SpeleothemBlock.findTip`.
fn find_tip<L: Lvl>(l: &L, st: u16, pos: BlockPos, max: i32, merged: bool) -> Option<BlockPos> {
    if is_tip(st, merged) {
        return Some(pos);
    }
    let me = block_of(st);
    let dir = tip_dir(st);
    walk(l, pos, dy_of(dir), max, |s| block_of(s) == me && tip_dir(s) == dir, |s| is_tip(s, merged))
}

/// `PointedDripstoneBlock.findRootBlock`: the first block above the chain.
fn find_root<L: Lvl>(l: &L, pos: BlockPos, st: u16) -> Option<BlockPos> {
    let me = block_of(st);
    let tip = tip_dir(st);
    walk(l, pos, -dy_of(tip), 11, |s| block_of(s) == me && tip_dir(s) == tip, |s| block_of(s) != me)
}

/// `SpeleothemBlock.isValidSpeleothemPlacement`.
fn valid_placement<L: Lvl>(l: &L, pos: BlockPos, tip: u8, me: u16) -> bool {
    let bs = l.get(rel(pos, data::opposite(tip)));
    data::sturdy(bs, tip) || (is_sp_dir(bs, tip) && block_of(bs) == me)
}

fn spel_can_survive<L: Lvl>(l: &L, pos: BlockPos, st: u16) -> bool {
    valid_placement(l, pos, tip_dir(st), block_of(st))
}

/// `SpeleothemBlock.calculateSpeleothemThickness`.
fn calc_thickness<L: Lvl>(l: &L, pos: BlockPos, me: u16, tip: u8, merge: bool) -> u8 {
    let t = t();
    let base = data::opposite(tip);
    let front = l.get(rel(pos, tip));
    if is_sp_dir(front, base) && block_of(front) == me {
        return if !merge && getv(front, Pr::Thickness) != t.v_tip_merge { t.v_tip } else { t.v_tip_merge };
    }
    if !is_sp_dir(front, tip) {
        return t.v_tip;
    }
    let ft = getv(front, Pr::Thickness);
    if ft != t.v_tip && ft != t.v_tip_merge {
        let behind = l.get(rel(pos, base));
        if !is_sp_dir(behind, tip) {
            t.v_base
        } else {
            t.v_middle
        }
    } else {
        t.v_frustum
    }
}

/// `SpeleothemBlock.isStalactiteStartPos`.
fn is_stalactite_start<L: Lvl>(l: &L, pos: BlockPos, st: u16) -> bool {
    is_stalactite(st) && block_of(l.get(above(pos))) != block_of(st)
}

/// `PointedDripstoneBlock.getFluidAboveStalactite`: the fluid above the root
/// block of the chain (mud counts as water: the overworld does not evaporate it).
fn fluid_above<L: Lvl>(l: &L, pos: BlockPos, st: u16) -> Option<(BlockPos, u8, u16)> {
    if !is_stalactite(st) {
        return None;
    }
    let root = find_root(l, pos, st)?;
    let ap = above(root);
    let s = l.get(ap);
    let f = if block_of(s) == t().mud || has(s, flag::WATER) {
        WATER_F
    } else if has(s, flag::LAVA) {
        LAVA_F
    } else {
        NO_FLUID
    };
    Some((ap, f, s))
}

/// `PointedDripstoneBlock.canDripThrough`.
fn can_drip_through(st: u16) -> bool {
    if is_air(st) {
        return true;
    }
    if has(st, flag::OPAQUE) || has(st, flag::WATER) || has(st, flag::LAVA) {
        return false;
    }
    // collision shape against the 4/16 wide column of the drip check
    let b = &rine_data::get().blocks;
    for bx in &b.shapes[b.state(st).shape as usize] {
        if bx[3] > 0.375 && bx[0] < 0.625 && bx[5] > 0.375 && bx[2] < 0.625 && bx[4] > 0.0 && bx[1] < 1.0 {
            return false;
        }
    }
    true
}

/// `PointedDripstoneBlock.findFillableCauldronBelowStalactiteTip`.
fn find_fillable_cauldron<L: Lvl>(l: &L, tip: BlockPos, fluid: u8) -> Option<BlockPos> {
    let t = t();
    walk(l, tip, -1, 11, can_drip_through, |s| {
        let b = block_of(s);
        b == t.cauldron || (b == t.water_cauldron && fluid == WATER_F)
    })
}

/// `PointedDripstoneBlock.findStalactiteTipAboveCauldron`.
fn find_tip_above_cauldron<L: Lvl>(l: &L, cauldron: BlockPos) -> Option<BlockPos> {
    walk(l, cauldron, 1, 11, can_drip_through, is_free_hanging)
}

/// `PointedDripstoneBlock.maybeTransferFluid` with a given random value.
/// Water fills a cauldron below (after `50 + fall` ticks), mud turns to clay.
pub fn pointed_transfer<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rv: f32) {
    if rv > 0.17578125 && rv > 0.05859375 {
        return;
    }
    if !is_stalactite_start(l, pos, st) {
        return;
    }
    let Some((fpos, fluid, src)) = fluid_above(l, pos, st) else { return };
    let prob = match fluid {
        WATER_F => 0.17578125,
        LAVA_F => 0.05859375,
        _ => return,
    };
    if rv >= prob {
        return;
    }
    let Some(tip) = find_tip(l, st, pos, 11, false) else { return };
    if block_of(src) == t().mud && fluid == WATER_F {
        set_all(rs, l, fpos, default_state("clay"));
        l.level_event(EV_DRIPSTONE_DRIP, tip, 0);
    } else if let Some(cpos) = find_fillable_cauldron(l, tip, fluid) {
        l.level_event(EV_DRIPSTONE_DRIP, tip, 0);
        let delay = (50 + tip.y - cpos.y).max(0) as u32;
        let cb = block_of(l.get(cpos));
        engine::schedule(rs, l, cpos, cb, delay, prio::NORMAL);
    }
}

fn can_grow<L: Lvl>(l: &L, pos: BlockPos, st: u16) -> bool {
    let t = t();
    let me = block_of(st);
    let on = if me == t.pointed { t.dripstone_block } else { t.sulfur };
    if block_of(l.get(above(pos))) != on {
        return false;
    }
    if me == t.pointed {
        let f = l.get(pos.offset(0, 2, 0));
        return has(f, flag::WATER) && has(f, flag::FLUID_SOURCE);
    }
    true
}

fn can_tip_grow<L: Lvl>(l: &L, tip_pos: BlockPos, ts: u16) -> bool {
    let gd = tip_dir(ts);
    let s = l.get(rel(tip_pos, gd));
    !has(s, flag::WATER) && !has(s, flag::LAVA) && (is_air(s) || is_unmerged_tip(s, data::opposite(gd), block_of(ts)))
}

fn default_of(me: u16) -> u16 {
    let t = t();
    if me == t.pointed {
        t.pointed_def
    } else {
        t.sulfur_spike_def
    }
}

/// `SpeleothemBlock.createSpeleothem` (the spread event places the block).
fn create_spel<L: Lvl>(rs: &mut Rs, l: &mut L, me: u16, pos: BlockPos, dir: u8, th: u8) {
    let t = t();
    let mut s = default_of(me);
    s = setv(s, Pr::VerticalDirection, if dir == UP { t.v_up } else { t.v_down });
    s = setv(s, Pr::Thickness, th);
    s = setb(s, Pr::Waterlogged, has(l.get(pos), flag::WATER));
    set_all(rs, l, pos, s);
}

/// `SpeleothemBlock.createMergedTips`.
fn create_merged<L: Lvl>(rs: &mut Rs, l: &mut L, me: u16, tip_st: u16, tip_pos: BlockPos) {
    let t = t();
    let (stalactite, stalagmite) = if tip_dir(tip_st) == UP { (above(tip_pos), tip_pos) } else { (tip_pos, below(tip_pos)) };
    create_spel(rs, l, me, stalactite, DOWN, t.v_tip_merge);
    create_spel(rs, l, me, stalagmite, UP, t.v_tip_merge);
}

/// `SpeleothemBlock.grow`.
fn grow<L: Lvl>(rs: &mut Rs, l: &mut L, me: u16, from: BlockPos, to: u8) {
    let t = t();
    let target = rel(from, to);
    let ex = l.get(target);
    if is_unmerged_tip(ex, data::opposite(to), me) {
        create_merged(rs, l, me, ex, target);
    } else if is_air(ex) || block_of(ex) == t.water {
        create_spel(rs, l, me, target, to, t.v_tip);
    }
}

/// `SpeleothemBlock.growStalagmiteBelow`.
fn grow_stalagmite_below<L: Lvl>(rs: &mut Rs, l: &mut L, me: u16, from: BlockPos) {
    let pointed = me == t().pointed;
    let mut p = from;
    for _ in 0..10 {
        p = below(p);
        let s = l.get(p);
        if has(s, flag::WATER) || has(s, flag::LAVA) {
            return;
        }
        if is_unmerged_tip(s, UP, me) && can_tip_grow(l, p, s) {
            grow(rs, l, me, p, UP);
            return;
        }
        if valid_placement(l, p, UP, me) && !has(l.get(below(p)), flag::WATER) {
            grow(rs, l, me, below(p), UP);
            return;
        }
        if pointed && !can_drip_through(s) {
            return;
        }
    }
}

/// `SpeleothemBlock.growStalactiteOrStalagmiteIfPossible`.
fn grow_or_stalagmite<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    if !can_grow(l, pos, st) {
        return;
    }
    let me = block_of(st);
    let max = if me == t().pointed { 7 } else { 2 };
    let Some(tip_pos) = find_tip(l, st, pos, max, false) else { return };
    let ts = l.get(tip_pos);
    if is_free_hanging(ts) && can_tip_grow(l, tip_pos, ts) {
        if rng.next_bool() {
            grow(rs, l, me, tip_pos, DOWN);
        } else {
            grow_stalagmite_below(rs, l, me, tip_pos);
        }
    }
}

fn speleothem_random_tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    if block_of(st) == t().pointed {
        let rv = rng.next_float();
        pointed_transfer(rs, l, st, pos, rv);
    }
    if rng.next_float() < 0.011_377_778 && is_stalactite_start(l, pos, st) {
        grow_or_stalagmite(rs, l, st, pos, rng);
    }
}

/// `SpeleothemBlock.tick`: a stalagmite that lost its base breaks; anything
/// else starts falling as a stalactite chain.
fn speleothem_tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    if is_stalagmite(st) && !spel_can_survive(l, pos, st) {
        engine::destroy_block(rs, l, pos, true, UPDATE_LIMIT);
        return;
    }
    // `spawnFallingStalactite`: each stalactite of the chain falls, the tip
    // hurts entities (setHurtsEntities) which is not modelled yet
    let mut fp = pos;
    let mut fs = st;
    while is_stalactite(fs) {
        let body = setb(fs, Pr::Waterlogged, false);
        engine::set_block(rs, l, fp, engine::fluid_legacy(fs), fl::ALL, UPDATE_LIMIT);
        if let Some(g) = l.game() {
            crate::game::entity::spawn_falling(g, [fp.x as f64 + 0.5, fp.y as f64, fp.z as f64 + 0.5], body);
        }
        if is_tip(fs, true) {
            break;
        }
        fp = below(fp);
        fs = l.get(fp);
    }
}

/// `SpeleothemBlock.updateShape`.
fn speleothem_update_shape<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, dir: u8) -> u16 {
    if dir != UP && dir != DOWN {
        return st;
    }
    let tip = tip_dir(st);
    if tip == DOWN && engine::has_scheduled(rs, pos, block_of(st)) {
        return st;
    }
    if dir == data::opposite(tip) && !spel_can_survive(l, pos, st) {
        engine::schedule(rs, l, pos, block_of(st), if tip == DOWN { 2 } else { 1 }, prio::NORMAL);
        return st;
    }
    let merge = getv(st, Pr::Thickness) == t().v_tip_merge;
    let th = calc_thickness(l, pos, block_of(st), tip, merge);
    setv(st, Pr::Thickness, th)
}

// ------------------------------------------------------------------ cauldrons

/// `AbstractCauldronBlock.tick`: the drip above the stalactite fills the cauldron.
/// Only the empty cauldron and the water cauldron accept drips here; the
/// water cauldron's rain-biome condition is not checked (biome data has no precipitation).
fn cauldron_tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    let t = t();
    let Some(tip) = find_tip_above_cauldron(l, pos) else { return };
    let fluid = match fluid_above(l, tip, l.get(tip)) {
        Some((_, f, _)) => f,
        None => NO_FLUID,
    };
    if fluid == NO_FLUID {
        return;
    }
    if block_of(st) == t.cauldron {
        if fluid == WATER_F {
            set_all(rs, l, pos, default_state("water_cauldron"));
            l.level_event(EV_DRIP_WATER_INTO_CAULDRON, pos, 0);
        } else {
            set_all(rs, l, pos, default_state("lava_cauldron"));
            l.level_event(EV_DRIP_LAVA_INTO_CAULDRON, pos, 0);
        }
    } else if block_of(st) == t.water_cauldron && fluid == WATER_F {
        let level = geti(st, Pr::Level);
        if level < 3 {
            set_all(rs, l, pos, seti(st, Pr::Level, level + 1));
            l.level_event(EV_DRIP_WATER_INTO_CAULDRON, pos, 0);
        }
    }
}

// ------------------------------------------------------------------ dispatch

/// `BlockState.randomTick` for this family.
pub fn random_tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    match super::beh(st) {
        Beh::RedstoneOre => {
            if getb(st, Pr::Lit) {
                set_all(rs, l, pos, setb(st, Pr::Lit, false));
            }
        }
        Beh::Farmland => farmland_random_tick(rs, l, st, pos),
        Beh::TurtleEgg => turtle_random_tick(rs, l, st, pos, rng),
        Beh::DriedGhast => ghast_random_tick(rs, l, st, pos),
        Beh::ChorusFlower => flower_random_tick(rs, l, pos, rng),
        Beh::Speleothem => speleothem_random_tick(rs, l, st, pos, rng),
        Beh::Propagule => propagule_random_tick(rs, l, st, pos, rng),
        _ => {}
    }
}

/// `BlockState.updateShape` for this family. `dir` points from `pos` to the changed neighbour.
pub fn update_shape<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, dir: u8, nst: u16) -> u16 {
    let t = t();
    match super::beh(st) {
        Beh::Farmland => {
            if dir == UP && !farmland_can_survive(l, pos) {
                engine::schedule(rs, l, pos, t.farmland, 1, prio::NORMAL);
            }
            st
        }
        Beh::ChorusFlower => {
            if dir != UP && !flower_can_survive(l, pos) {
                engine::schedule(rs, l, pos, block_of(st), 1, prio::NORMAL);
            }
            st
        }
        Beh::ChorusPlant => {
            if !plant_can_survive(l, pos) {
                engine::schedule(rs, l, pos, block_of(st), 1, prio::NORMAL);
                st
            } else {
                let connect = plant_or_flower(nst) || (dir == DOWN && t.supports_plant[block_of(nst) as usize]);
                setb(st, dir_prop(dir), connect)
            }
        }
        Beh::Speleothem => speleothem_update_shape(rs, l, st, pos, dir),
        Beh::DriedGhast => {
            // TODO(fluid): waterlogged ghast schedules a water fluid tick (not in the engine yet)
            st
        }
        Beh::SporeBlossom => {
            if dir == UP && !spore_can_survive(l, pos) {
                0
            } else {
                st
            }
        }
        _ => st,
    }
}

/// `BlockState.tick` (scheduled) for this family.
pub fn tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    match super::beh(st) {
        Beh::Farmland => {
            if !farmland_can_survive(l, pos) {
                turn_to_dirt(rs, l, pos);
            }
        }
        Beh::SnifferEgg => sniffer_tick(rs, l, st, pos),
        Beh::DriedGhast => ghast_tick(rs, l, st, pos),
        Beh::ChorusFlower => {
            if !flower_can_survive(l, pos) {
                engine::destroy_block(rs, l, pos, true, UPDATE_LIMIT);
            }
        }
        Beh::ChorusPlant => {
            if !plant_can_survive(l, pos) {
                engine::destroy_block(rs, l, pos, true, UPDATE_LIMIT);
            }
        }
        Beh::Speleothem => speleothem_tick(rs, l, st, pos),
        Beh::Cauldron => cauldron_tick(rs, l, st, pos),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::super::testw::World as TW;
    use super::*;

    fn s(n: &str) -> u16 {
        default_state(n)
    }

    fn at(x: i32, y: i32, z: i32) -> BlockPos {
        BlockPos::new(x, y, z)
    }

    #[test]
    fn redstone_ore_lights_on_touch_and_fades_on_random_tick() {
        let mut w = TW::new();
        let p = at(0, 64, 0);
        let mut rs = Rs::default();
        let mut rng = Rng::new(1);
        w.put(p, s("redstone_ore"));
        let st = w.get(p);
        redstone_ore_touch(&mut rs, &mut w, st, p);
        assert_eq!(w.get(p), setb(s("redstone_ore"), Pr::Lit, true));
        // touching a lit ore changes nothing
        let st = w.get(p);
        redstone_ore_touch(&mut rs, &mut w, st, p);
        assert_eq!(w.get(p), setb(s("redstone_ore"), Pr::Lit, true));
        // random tick turns it off
        let st = w.get(p);
        random_tick(&mut rs, &mut w, st, p, &mut rng);
        assert_eq!(w.get(p), s("redstone_ore"));
    }

    #[test]
    fn farmland_wets_within_the_nine_by_two_by_nine_box() {
        let mut rs = Rs::default();
        let mut rng = Rng::new(2);
        let p = at(0, 64, 0);
        // water at dy 1, dx 4: inside the box
        let mut w = TW::new();
        w.put(p, seti(s("farmland"), Pr::Moisture, 0));
        w.put(at(4, 65, 0), s("water"));
        let st = w.get(p);
        random_tick(&mut rs, &mut w, st, p, &mut rng);
        assert_eq!(geti(w.get(p), Pr::Moisture), 7);
        // water at dy 2 is outside (box is y 0..=1)
        let mut w = TW::new();
        w.put(p, seti(s("farmland"), Pr::Moisture, 0));
        w.put(at(0, 66, 0), s("water"));
        let st = w.get(p);
        random_tick(&mut rs, &mut w, st, p, &mut rng);
        assert_eq!(w.get(p), s("dirt"));
        // water at dx 5 is outside
        let mut w = TW::new();
        w.put(p, seti(s("farmland"), Pr::Moisture, 3));
        w.put(at(5, 64, 0), s("water"));
        let st = w.get(p);
        random_tick(&mut rs, &mut w, st, p, &mut rng);
        assert_eq!(geti(w.get(p), Pr::Moisture), 2);
    }

    #[test]
    fn dry_farmland_turns_to_dirt_unless_a_crop_is_above() {
        let mut rs = Rs::default();
        let mut rng = Rng::new(3);
        let p = at(0, 64, 0);
        let mut w = TW::new();
        w.put(p, s("farmland"));
        let st = w.get(p);
        random_tick(&mut rs, &mut w, st, p, &mut rng);
        assert_eq!(w.get(p), s("dirt"));
        let mut w = TW::new();
        w.put(p, s("farmland"));
        w.put(at(0, 65, 0), s("wheat"));
        let st = w.get(p);
        random_tick(&mut rs, &mut w, st, p, &mut rng);
        assert_eq!(w.get(p), s("farmland"));
    }

    #[test]
    fn farmland_under_a_solid_block_breaks_on_scheduled_tick() {
        let mut rs = Rs::default();
        let p = at(0, 64, 0);
        let mut w = TW::new();
        w.put(p, s("farmland"));
        w.put(at(0, 65, 0), s("stone"));
        tick(&mut rs, &mut w, s("farmland"), p);
        assert_eq!(w.get(p), s("dirt"));
    }

    #[test]
    fn turtle_chance_follows_the_night_window() {
        assert_eq!(turtle_hatch_chance(21061), 0.002);
        assert_eq!(turtle_hatch_chance(21062), 1.0);
        assert_eq!(turtle_hatch_chance(21904), 1.0);
        assert_eq!(turtle_hatch_chance(21905), 0.002);
        assert_eq!(turtle_hatch_chance(-2939), 0.002);
    }

    #[test]
    fn turtle_egg_hatches_in_three_steps_on_sand_at_night() {
        let mut rs = Rs::default();
        let mut rng = Rng::new(4);
        let p = at(0, 64, 0);
        let mut w = TW::new();
        w.day_time = 21500;
        w.put(at(0, 63, 0), s("sand"));
        w.put(p, s("turtle_egg"));
        for want in [1, 2] {
            let st = w.get(p);
            random_tick(&mut rs, &mut w, st, p, &mut rng);
            assert_eq!(geti(w.get(p), Pr::Hatch), want);
        }
        let st = w.get(p);
        random_tick(&mut rs, &mut w, st, p, &mut rng);
        assert_eq!(w.get(p), 0);
    }

    #[test]
    fn turtle_egg_on_stone_never_progresses() {
        let mut rs = Rs::default();
        let mut rng = Rng::new(5);
        let p = at(0, 64, 0);
        let mut w = TW::new();
        w.day_time = 21500;
        w.put(at(0, 63, 0), s("stone"));
        w.put(p, s("turtle_egg"));
        for _ in 0..50 {
            let st = w.get(p);
            random_tick(&mut rs, &mut w, st, p, &mut rng);
        }
        assert_eq!(geti(w.get(p), Pr::Hatch), 0);
    }

    #[test]
    fn sniffer_egg_takes_three_stages_of_eight_thousand_ticks() {
        let mut rs = Rs::default();
        let p = at(0, 64, 0);
        let mut w = TW::new();
        engine::set_block(&mut rs, &mut w, p, s("sniffer_egg"), fl::ALL, UPDATE_LIMIT);
        let mut n = 0;
        while w.get(p) != 0 && n < 100_000 {
            w.tick(&mut rs);
            n += 1;
        }
        assert_eq!(w.get(p), 0);
        assert!((24000..=24897).contains(&w.time), "{}", w.time);
    }

    #[test]
    fn sniffer_egg_on_moss_hatches_twice_as_fast() {
        let mut rs = Rs::default();
        let p = at(0, 64, 0);
        let mut w = TW::new();
        w.put(at(0, 63, 0), s("moss_block"));
        engine::set_block(&mut rs, &mut w, p, s("sniffer_egg"), fl::ALL, UPDATE_LIMIT);
        let mut n = 0;
        while w.get(p) != 0 && n < 100_000 {
            w.tick(&mut rs);
            n += 1;
        }
        assert!((12000..=12897).contains(&w.time), "{}", w.time);
    }

    #[test]
    fn dried_ghast_fills_with_water_then_becomes_a_ghastling() {
        let mut rs = Rs::default();
        let p = at(0, 64, 0);
        let mut rng = Rng::new(6);
        let wet = setb(s("dried_ghast"), Pr::Waterlogged, true);
        let mut w = TW::new();
        w.put(p, wet);
        let st = w.get(p);
        random_tick(&mut rs, &mut w, st, p, &mut rng);
        assert!(engine::has_scheduled(&rs, p, block_of(wet)));
        for want in [1, 2, 3] {
            let st = w.get(p);
            tick(&mut rs, &mut w, st, p);
            assert_eq!(geti(w.get(p), Pr::Hydration), want);
        }
        // removeBlock leaves the waterlogged block's fluid behind
        let st = w.get(p);
        tick(&mut rs, &mut w, st, p);
        assert_eq!(block_of(w.get(p)), block_of(s("water")));
    }

    #[test]
    fn dried_ghast_on_land_dries_out_one_step_per_tick() {
        let mut rs = Rs::default();
        let p = at(0, 64, 0);
        let mut w = TW::new();
        w.put(p, seti(s("dried_ghast"), Pr::Hydration, 2));
        let st = w.get(p);
        tick(&mut rs, &mut w, st, p);
        assert_eq!(geti(w.get(p), Pr::Hydration), 1);
        // a dry ghast with no water does not schedule anything
        let mut w = TW::new();
        w.put(p, s("dried_ghast"));
        let st = w.get(p);
        random_tick(&mut rs, &mut w, st, p, &mut Rng::new(7));
        assert!(!engine::has_scheduled(&rs, p, block_of(st)));
    }

    #[test]
    fn chorus_flower_on_end_stone_grows_upwards_into_a_plant() {
        let mut rs = Rs::default();
        let mut rng = Rng::new(8);
        let p = at(0, 64, 0);
        let mut w = TW::new();
        w.put(at(0, 63, 0), s("end_stone"));
        w.put(p, s("chorus_flower"));
        let st = w.get(p);
        random_tick(&mut rs, &mut w, st, p, &mut rng);
        assert_eq!(block_of(w.get(p)), block_of(s("chorus_plant")));
        assert!(getb(w.get(p), Pr::Up));
        assert!(getb(w.get(p), Pr::Down));
        assert!(!getb(w.get(p), Pr::North));
        let up = w.get(at(0, 65, 0));
        assert_eq!(block_of(up), block_of(s("chorus_flower")));
        assert_eq!(geti(up, Pr::Age), 0);
    }

    #[test]
    fn chorus_flower_dies_at_age_four_with_no_room() {
        let mut rs = Rs::default();
        let mut rng = Rng::new(9);
        let p = at(0, 64, 0);
        let mut w = TW::new();
        // a flower on top of a plant, blocked above: no room to grow or branch
        w.put(at(0, 63, 0), s("chorus_plant"));
        w.put(p, seti(s("chorus_flower"), Pr::Age, 4));
        w.put(at(0, 66, 0), s("stone"));
        for d in HORIZONTAL {
            w.put(rel(p, d), s("stone"));
        }
        let st = w.get(p);
        random_tick(&mut rs, &mut w, st, p, &mut rng);
        assert_eq!(geti(w.get(p), Pr::Age), 5);
        // age 5 never ticks
        let st = w.get(p);
        random_tick(&mut rs, &mut w, st, p, &mut rng);
        assert_eq!(geti(w.get(p), Pr::Age), 5);
    }

    #[test]
    fn unsupported_chorus_flower_breaks() {
        let mut rs = Rs::default();
        let p = at(0, 64, 0);
        let mut w = TW::new();
        w.put(p, s("chorus_flower"));
        tick(&mut rs, &mut w, s("chorus_flower"), p);
        assert_eq!(w.get(p), 0);
    }

    #[test]
    fn chorus_plant_connections_match_neighbours() {
        let p = at(0, 64, 0);
        let mut w = TW::new();
        w.put(at(0, 63, 0), s("end_stone"));
        w.put(at(0, 65, 0), s("chorus_plant"));
        w.put(at(1, 64, 0), s("chorus_flower"));
        w.put(at(-1, 64, 0), s("stone"));
        let c = chorus_placement(&w, s("chorus_plant"), p);
        assert!(getb(c, Pr::Down));
        assert!(getb(c, Pr::Up));
        assert!(getb(c, Pr::East));
        assert!(!getb(c, Pr::West));
        assert!(!getb(c, Pr::North));
        assert!(!getb(c, Pr::South));
    }

    #[test]
    fn stalactite_without_its_ceiling_falls() {
        let mut rs = Rs::default();
        let p = at(0, 64, 0);
        let mut w = TW::new();
        w.put(at(0, 65, 0), s("dripstone_block"));
        let tip = setv(setv(s("pointed_dripstone"), Pr::VerticalDirection, vid("down")), Pr::Thickness, vid("tip"));
        w.put(p, tip);
        // the ceiling goes away: the stalactite schedules a fall and then drops
        engine::set_block(&mut rs, &mut w, at(0, 65, 0), 0, fl::ALL, UPDATE_LIMIT);
        for _ in 0..3 {
            w.tick(&mut rs);
        }
        assert_eq!(w.get(p), 0);
    }

    #[test]
    fn stalactite_under_another_tip_becomes_frustum() {
        let p = at(0, 64, 0);
        let mut rs = Rs::default();
        let tip = setv(setv(s("pointed_dripstone"), Pr::VerticalDirection, vid("down")), Pr::Thickness, vid("tip"));
        let mut w = TW::new();
        w.put(at(0, 65, 0), s("dripstone_block"));
        w.put(at(0, 63, 0), tip);
        let out = update_shape(&mut rs, &mut w, tip, p, DOWN, tip);
        assert_eq!(getv(out, Pr::Thickness), vid("frustum"));
        // with nothing below it is a tip
        let mut w = TW::new();
        w.put(at(0, 65, 0), s("dripstone_block"));
        let out = update_shape(&mut rs, &mut w, tip, p, DOWN, 0);
        assert_eq!(getv(out, Pr::Thickness), vid("tip"));
    }

    #[test]
    fn dripping_water_fills_the_cauldron_below_after_fifty_four_ticks() {
        let mut rs = Rs::default();
        let p = at(0, 64, 0);
        let mut w = TW::new();
        w.put(at(0, 65, 0), s("dripstone_block"));
        w.put(at(0, 66, 0), s("water"));
        let tip = setv(setv(s("pointed_dripstone"), Pr::VerticalDirection, vid("down")), Pr::Thickness, vid("tip"));
        w.put(p, tip);
        w.put(at(0, 60, 0), s("cauldron"));
        // roll 0.1 is under the water chance 0.1758
        pointed_transfer(&mut rs, &mut w, tip, p, 0.1);
        assert!(engine::has_scheduled(&rs, at(0, 60, 0), block_of(s("cauldron"))));
        for _ in 0..54 {
            w.tick(&mut rs);
        }
        assert_eq!(w.get(at(0, 60, 0)), s("water_cauldron"));
    }

    #[test]
    fn a_high_roll_drips_nothing() {
        let mut rs = Rs::default();
        let p = at(0, 64, 0);
        let mut w = TW::new();
        w.put(at(0, 65, 0), s("dripstone_block"));
        w.put(at(0, 66, 0), s("water"));
        let tip = setv(setv(s("pointed_dripstone"), Pr::VerticalDirection, vid("down")), Pr::Thickness, vid("tip"));
        w.put(p, tip);
        w.put(at(0, 60, 0), s("cauldron"));
        pointed_transfer(&mut rs, &mut w, tip, p, 0.5);
        assert!(!engine::has_scheduled(&rs, at(0, 60, 0), block_of(s("cauldron"))));
    }

    #[test]
    fn mangrove_propagule_hanging_ages_to_four() {
        let mut rs = Rs::default();
        let mut rng = Rng::new(10);
        let p = at(0, 64, 0);
        let mut w = TW::new();
        w.put(p, setb(s("mangrove_propagule"), Pr::Hanging, true));
        for want in [1, 2, 3, 4, 4] {
            let st = w.get(p);
            random_tick(&mut rs, &mut w, st, p, &mut rng);
            assert_eq!(geti(w.get(p), Pr::Age), want);
        }
    }

    #[test]
    fn spore_blossom_falls_when_its_ceiling_goes() {
        let mut rs = Rs::default();
        let p = at(0, 64, 0);
        let spore = s("spore_blossom");
        let mut w = TW::new();
        // air above: the blossom goes
        let out = update_shape(&mut rs, &mut w, spore, p, UP, 0);
        assert_eq!(out, 0);
        // stone above: it stays
        let mut w = TW::new();
        w.put(at(0, 65, 0), s("stone"));
        let out = update_shape(&mut rs, &mut w, spore, p, UP, s("stone"));
        assert_eq!(out, spore);
    }

    #[test]
    fn hatch_chance_is_a_pure_window() {
        // the window is half-open: 21905 is already the low chance
        assert!(turtle_hatch_chance(21904) > 0.5);
        assert!(turtle_hatch_chance(21905) < 0.5);
    }
}
