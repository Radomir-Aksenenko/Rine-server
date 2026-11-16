//! Items used on blocks (`ItemStack.useOn`) that no block family handles by
//! itself, routed by the held item:
//! - hoe, shovel, axe: `BlockTransformer` (`BlockTransformers.java`): till,
//!   flatten, strip, scrape and wax off copper;
//! - honeycomb: `HoneycombItem.useOn` (wax copper);
//! - bone meal: `BoneMealItem.useOn` (`growCrop`);
//! - flint and steel, fire charge: light campfires, candles and candle cakes,
//!   or place fire next to the clicked face.
//!
//! Not ported:
//! - shield in the offhand (`BlockTransformer.playerHasBlockingItemUseIntent`):
//!   `UseCtx` has no offhand. It also gates shovel, hoe and axe in vanilla.
//! - adventure mode (`ItemStack.useOn` gives PASS when `!mayBuild`): `UseCtx`
//!   has no adventure flag.
//! - honeycomb on signs (`SignApplicator`): the sign family.
//! - bone meal on water (`growWaterPlant`) and on the blocks listed in the
//!   doc of `bone_meal` (grass, moss, mushrooms, nylium, ...).
//! - nether portal ignition in `BaseFireBlock.canBePlacedAt` (`isPortal`).
//! - game events and criteria triggers.
//! - the `popResourceFromFace` drop position (rooted dirt pops from the middle).
//! - level events sent to the acting player (`levelEvent(player, ...)`):
//!   `Lvl::level_event` has no actor to skip.
//!
//! Adding a tool or item means a `Kind` and a row in `build`.

use super::crops;
use super::fire;
use super::bonemeal;
use super::sapling;
use super::interact::{ItemOp, Res, UseCtx, UseOut};
use super::plants;
use super::props::{getb, getv, has, setb, vid, Pr};
use super::{block_id, block_of, block_tag, item_id, Rng};
use crate::game::redstone::data::{self, step, DOWN};
use crate::game::redstone::engine::{self, fl, Lvl, CAT_BLOCKS, UPDATE_LIMIT};
use crate::game::redstone::Rs;
use ahash::AHashMap;
use rine_protocol::BlockPos;
use std::sync::OnceLock;

/// `LevelEvent` ids.
const EV_PLANT_GROWTH: i32 = 1505;
const EV_WAX_ON: i32 = 3003;
const EV_WAX_OFF: i32 = 3004;
const EV_SCRAPE: i32 = 3005;

const NONE16: u16 = u16::MAX;

/// The 15 `WeatheringCopper` families of `WeatheringCopper.NEXT_BY_BLOCK`
/// as (unaffected, exposed, weathered, oxidized). Waxed names are "waxed_"
/// plus these. Same list as `copper.rs` (kept separate: other family).
const COPPER: [[&str; 4]; 15] = [
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

/// Strippable blocks of `axeStrippables`: the stripped block is "stripped_"
/// plus the name, and keeps the properties (axis).
const WOOD_PAIRS: [&str; 25] = [
    "oak_wood",
    "oak_log",
    "dark_oak_wood",
    "dark_oak_log",
    "pale_oak_wood",
    "pale_oak_log",
    "poplar_wood",
    "poplar_log",
    "acacia_wood",
    "acacia_log",
    "cherry_wood",
    "cherry_log",
    "birch_wood",
    "birch_log",
    "jungle_wood",
    "jungle_log",
    "spruce_wood",
    "spruce_log",
    "warped_stem",
    "warped_hyphae",
    "crimson_stem",
    "crimson_hyphae",
    "mangrove_wood",
    "mangrove_log",
    "bamboo_block",
];

/// Which item the held stack is, for `use_on`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Shovel,
    Hoe,
    Axe,
    Honeycomb,
    BoneMeal,
    FlintSteel,
    FireCharge,
}

/// One `BlockTransformData` (or the honeycomb entry).
#[derive(Clone, Copy)]
struct Rule {
    /// `Direction.UP` must match the `air` tag.
    air_above: bool,
    /// `disallowed_faces` holds DOWN.
    no_down: bool,
    /// `updateFromNeighbors`: `Block.updateFromNeighbourShapes` before the set.
    update: bool,
    /// `TransformType.COPPER_CHEST`: the connected half gets the particle.
    chest: bool,
    /// `loot` `till/rooted_dirt`: one hanging roots.
    loot: bool,
    sound: &'static str,
    particle: Particle,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Particle {
    None,
    Scrape,
    WaxOn,
    WaxOff,
}

impl Particle {
    fn event(self) -> Option<i32> {
        match self {
            Particle::None => None,
            Particle::Scrape => Some(EV_SCRAPE),
            Particle::WaxOn => Some(EV_WAX_ON),
            Particle::WaxOff => Some(EV_WAX_OFF),
        }
    }
}

/// A transformer as one table: per state id, the target state and the rule
/// that claims it. The sets of the rules are disjoint (tested), so a state
/// is claimed by one rule at most, which keeps vanilla's "first transform
/// that applies" order.
struct Map {
    rules: Vec<Rule>,
    target: Vec<u16>,
    rule: Vec<u8>,
}

impl Map {
    fn new(rules: Vec<Rule>) -> Self {
        let n = rine_data::get().blocks.states.len();
        Self { rules, target: vec![NONE16; n], rule: vec![0; n] }
    }

    /// Claim every state of block `src` for `dst`. `copy` is
    /// `CopyPropertiesProvider` (`withPropertiesOf`), else the default state.
    fn put(&mut self, src: u16, dst: u16, copy: bool, rule: u8) {
        if src == NONE16 || dst == NONE16 {
            return;
        }
        let b = &rine_data::get().blocks;
        let info = &b.blocks[src as usize];
        let default = b.blocks[dst as usize].default_state as u16;
        for s in info.min_state..info.min_state + info.state_count {
            self.target[s as usize] = if copy { copy_props(s as u16, dst) } else { default };
            self.rule[s as usize] = rule;
        }
    }

    fn named(&mut self, src: &str, dst: &str, copy: bool, rule: u8) {
        self.put(block_id(src), block_id(dst), copy, rule);
    }

    fn tagged(&mut self, tag: &str, dst: &str, rule: u8) {
        let dst = block_id(dst);
        for (blk, yes) in block_tag(tag).into_iter().enumerate() {
            if yes {
                self.put(blk as u16, dst, false, rule);
            }
        }
    }
}

/// `Block.withPropertiesOf`: the default state of `dst`, with every property
/// that `src` and `dst` share set to the value of `src`. Init time only.
fn copy_props(src: u16, dst: u16) -> u16 {
    let b = &rine_data::get().blocks;
    let dst_info = &b.blocks[dst as usize];
    let mut out = dst_info.default_state as u16;
    for p in &b.block_of(src).props {
        if !dst_info.props.iter().any(|q| q.name == p.name) {
            continue;
        }
        if let Some(v) = b.get_prop(src, &p.name) {
            if let Some(n) = b.with_prop(out, &p.name, v) {
                out = n;
            }
        }
    }
    out
}

fn is_chest(name: &str) -> bool {
    name.ends_with("copper_chest")
}

fn is_door(name: &str) -> bool {
    name.ends_with("copper_door")
}

/// `Item.useOn` of the axe with `BlockTransformers.AXE`: strip, then scrape
/// (`WeatheringCopper.PREVIOUS_BY_BLOCK`), then wax off (`WAX_OFF_BY_BLOCK`).
/// Each group is three transforms: normal, copper chest, door. Rules 0..=6.
fn axe_map() -> Map {
    let rule = |sound: &'static str, particle: Particle, update: bool, chest: bool| Rule { air_above: false, no_down: false, update, chest, loot: false, sound, particle };
    let mut m = Map::new(vec![
        rule("item.axe.strip", Particle::None, true, false),
        rule("item.axe.scrape", Particle::Scrape, true, false),
        rule("item.axe.scrape", Particle::Scrape, false, true),
        rule("item.axe.scrape", Particle::Scrape, false, false),
        rule("item.axe.wax_off", Particle::WaxOff, true, false),
        rule("item.axe.wax_off", Particle::WaxOff, false, true),
        rule("item.axe.wax_off", Particle::WaxOff, false, false),
    ]);
    for w in WOOD_PAIRS {
        m.named(w, &format!("stripped_{w}"), true, 0);
    }
    // scrape: exposed -> unaffected, weathered -> exposed, oxidized -> weathered
    for fam in COPPER {
        for i in 1..4 {
            let key = fam[i];
            let r = if is_chest(key) {
                2
            } else if is_door(key) {
                3
            } else {
                1
            };
            m.named(key, fam[i - 1], true, r);
        }
    }
    // wax off: waxed_X -> X for all four stages
    for fam in COPPER {
        for name in fam {
            let key = format!("waxed_{name}");
            let r = if is_chest(name) {
                5
            } else if is_door(name) {
                6
            } else {
                4
            };
            m.named(&key, name, true, r);
        }
    }
    m
}

/// `BlockTransformers.HOE`: farmland or dirt (default state, air above, not
/// from below), then rooted dirt to dirt (any face, no air check, hanging roots).
fn hoe_map() -> Map {
    let mut m = Map::new(vec![
        Rule { air_above: true, no_down: true, update: true, chest: false, loot: false, sound: "item.hoe.till", particle: Particle::None },
        Rule { air_above: false, no_down: false, update: true, chest: false, loot: true, sound: "item.hoe.till", particle: Particle::None },
    ]);
    m.tagged("turns_into_farmland", "farmland", 0);
    m.named("coarse_dirt", "dirt", false, 0);
    m.named("rooted_dirt", "dirt", false, 1);
    m
}

/// `BlockTransformers.SHOVEL`: dirt path, air above, not from below.
fn shovel_map() -> Map {
    let mut m = Map::new(vec![Rule { air_above: true, no_down: true, update: true, chest: false, loot: false, sound: "item.shovel.flatten", particle: Particle::None }]);
    m.tagged("turns_into_dirt_path", "dirt_path", 0);
    m
}

/// `HoneycombItem.WAXABLES`: every copper stage to its waxed block, with
/// the properties kept. Rule 1 is a copper chest (second half, see `wax_on`).
fn wax_map() -> Map {
    let mut m = Map::new(vec![
        Rule { air_above: false, no_down: false, update: false, chest: false, loot: false, sound: "item.honeycomb.wax_on", particle: Particle::WaxOn },
        Rule { air_above: false, no_down: false, update: false, chest: true, loot: false, sound: "item.honeycomb.wax_on", particle: Particle::WaxOn },
    ]);
    for fam in COPPER {
        for name in fam {
            let r = if is_chest(name) { 1 } else { 0 };
            m.named(name, &format!("waxed_{name}"), true, r);
        }
    }
    m
}

struct Ids {
    hanging_roots: u32,
    /// Read by `passes_on_candle_cake`, which the cake family will call.
    #[allow(dead_code)]
    flint_and_steel: u32,
    #[allow(dead_code)]
    fire_charge: u32,
    /// `ChestType.LEFT` / `SINGLE` as interned property values.
    v_left: u8,
    v_single: u8,
}

struct Tables {
    shovel: Map,
    hoe: Map,
    axe: Map,
    wax: Map,
    /// Block tags by block id.
    air: Vec<bool>,
    campfire: Vec<bool>,
    candle: Vec<bool>,
    candle_cake: Vec<bool>,
    items: AHashMap<u32, Kind>,
    shield: u32,
    ids: Ids,
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(build)
}

fn build() -> Tables {
    let mut items = AHashMap::new();
    for mat in ["wooden", "copper", "stone", "golden", "iron", "diamond", "netherite"] {
        for (suffix, k) in [("shovel", Kind::Shovel), ("hoe", Kind::Hoe), ("axe", Kind::Axe)] {
            let id = item_id(&format!("{mat}_{suffix}"));
            if id != 0 {
                items.insert(id, k);
            }
        }
    }
    for (name, k) in [("honeycomb", Kind::Honeycomb), ("bone_meal", Kind::BoneMeal), ("flint_and_steel", Kind::FlintSteel), ("fire_charge", Kind::FireCharge)] {
        let id = item_id(name);
        if id != 0 {
            items.insert(id, k);
        }
    }
    Tables {
        shovel: shovel_map(),
        hoe: hoe_map(),
        axe: axe_map(),
        wax: wax_map(),
        air: block_tag("air"),
        campfire: block_tag("campfires"),
        candle: block_tag("candles"),
        candle_cake: block_tag("candle_cakes"),
        items,
        shield: item_id("shield"),
        ids: Ids {
            hanging_roots: item_id("hanging_roots"),
            flint_and_steel: item_id("flint_and_steel"),
            fire_charge: item_id("fire_charge"),
            v_left: vid("left"),
            v_single: vid("single"),
        },
    }
}

fn is_air_block<L: Lvl>(t: &Tables, l: &L, p: BlockPos) -> bool {
    t.air[block_of(l.get(p)) as usize]
}

/// `ChestBlock.getConnectedBlockPos` of a copper chest half.
fn chest_partner(t: &Tables, pos: BlockPos, st: u16) -> BlockPos {
    let facing = data::get(st, data::P::Facing);
    let d = if getv(st, Pr::Type) == t.ids.v_left { data::clockwise(facing) } else { data::counter_clockwise(facing) };
    let (dx, dy, dz) = step(d);
    pos.offset(dx, dy, dz)
}

/// A double copper chest half (`TYPE != SINGLE`).
fn chest_is_double(t: &Tables, st: u16) -> bool {
    getv(st, Pr::Type) != t.ids.v_single
}

/// `stack.shrink(1)`. Creative keeps the stack: `ServerPlayerGameMode.useItemOn`
/// restores the count after `useOn` for players with infinite materials.
fn shrink_always(_ctx: &UseCtx) -> ItemOp {
    ItemOp::Shrink(1)
}

/// The face's neighbour, `BlockPos.relative(face)`.
fn relative(pos: BlockPos, face: u8) -> BlockPos {
    let (dx, dy, dz) = step(face);
    pos.offset(dx, dy, dz)
}

/// `CampfireBlock.canLight`, `CandleBlock.canLight`, `CandleCakeBlock.canLight`.
fn can_light(t: &Tables, st: u16) -> bool {
    let b = block_of(st) as usize;
    let campfire = t.campfire[b] && has(st, Pr::Waterlogged) && has(st, Pr::Lit) && !getb(st, Pr::Waterlogged) && !getb(st, Pr::Lit);
    let candle = t.candle[b] && has(st, Pr::Lit) && has(st, Pr::Waterlogged) && !getb(st, Pr::Lit) && !getb(st, Pr::Waterlogged);
    let cake = t.candle_cake[b] && has(st, Pr::Lit) && !getb(st, Pr::Lit);
    campfire || candle || cake
}

/// `BlockTransformer.transformBlock` for one transformer.
fn transform<L: Lvl>(rs: &mut Rs, l: &mut L, m: &Map, st: u16, pos: BlockPos, face: u8) -> Res {
    let t = tables();
    let target = m.target[st as usize];
    if target == NONE16 {
        return Res::Pass;
    }
    let rule = m.rules[m.rule[st as usize] as usize];
    if rule.no_down && face == DOWN {
        return Res::Pass;
    }
    if rule.air_above && !is_air_block(t, l, pos.offset(0, 1, 0)) {
        return Res::Pass;
    }
    let new = if rule.update { engine::update_from_neighbour_shapes(rs, l, target, pos) } else { target };
    if rule.loot {
        // `Block.dropFromBlockInteractLootTable`, `till/rooted_dirt`
        l.pop_resource(pos, t.ids.hanging_roots, 1);
    }
    // consume / hurtAndBreak first, then the block, sound, particles
    engine::set_block(rs, l, pos, new, fl::NEIGHBORS | fl::CLIENTS | fl::IMMEDIATE, UPDATE_LIMIT);
    l.sound(rule.sound, pos, 1.0, 1.0, CAT_BLOCKS, true);
    if let Some(ev) = rule.particle.event() {
        l.level_event(ev, pos, 0);
    }
    if rule.chest && chest_is_double(t, st) {
        if let Some(ev) = rule.particle.event() {
            l.level_event(ev, chest_partner(t, pos, st), 0);
        }
    }
    Res::Done(UseOut { item: ItemOp::Damage(1), ..UseOut::default() })
}

/// `HoneycombItem.useOn`.
fn wax_on<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, ctx: &UseCtx) -> Res {
    let t = tables();
    let m = &t.wax;
    let target = m.target[st as usize];
    if target == NONE16 {
        return Res::Pass;
    }
    let rule = m.rules[m.rule[st as usize] as usize];
    engine::set_block(rs, l, pos, target, fl::NEIGHBORS | fl::CLIENTS | fl::IMMEDIATE, UPDATE_LIMIT);
    l.level_event(EV_WAX_ON, pos, 0);
    l.sound(rule.sound, pos, 1.0, 1.0, CAT_BLOCKS, true);
    if rule.chest && chest_is_double(t, st) {
        // vanilla plays the second sound at `pos`, not at the partner
        l.level_event(EV_WAX_ON, chest_partner(t, pos, st), 0);
        l.sound(rule.sound, pos, 1.0, 1.0, CAT_BLOCKS, true);
    }
    Res::Done(UseOut { item: shrink_always(ctx), ..UseOut::default() })
}

/// `BoneMealItem.useOn` through `growCrop`. The water branch is not ported.
///
/// Saplings (`sapling.rs`) are routed here. Not reached here (vanilla
/// `BonemealableBlock` that needs a placed feature or a spread, all TODO):
/// grass_block, short and tall grass, fern, large fern, propagule, azalea,
/// moss blocks and pale moss, huge mushroom blocks, mycelium, crimson and
/// warped nylium, nether fungus.
/// Simple ones not ported yet: tall flowers, flower bed, glow lichen, hanging
/// moss, mossy carpet, dripleaf, sea pickle, seagrass, rooted dirt,
/// netherrack, shelf mushroom, mangrove leaves, dry grass, firefly bush, bush.
fn bone_meal<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, ctx: &UseCtx, rng: &mut Rng) -> Res {
    if !(crops::bonemeal(rs, l, st, pos, rng) || plants::bonemeal(rs, l, st, pos, rng) || sapling::bonemeal(rs, l, st, pos, rng) || bonemeal::bonemeal(rs, l, st, pos)) {
        return Res::Pass;
    }
    l.level_event(EV_PLANT_GROWTH, pos, 15);
    Res::Done(UseOut { item: shrink_always(ctx), ..UseOut::default() })
}

/// `FlintAndSteelItem.useOn`.
fn flint_and_steel<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, face: u8, rng: &mut Rng) -> Res {
    let t = tables();
    if can_light(t, st) {
        let pitch = rng.next_float() * 0.4 + 0.8;
        l.sound("item.flintandsteel.use", pos, 1.0, pitch, CAT_BLOCKS, true);
        engine::set_block(rs, l, pos, setb(st, Pr::Lit, true), fl::NEIGHBORS | fl::CLIENTS | fl::IMMEDIATE, UPDATE_LIMIT);
        return Res::Done(UseOut { item: ItemOp::Damage(1), ..UseOut::default() });
    }
    let rel = relative(pos, face);
    if !fire::can_be_lit(&*l, rel, face) {
        return Res::Fail;
    }
    let pitch = rng.next_float() * 0.4 + 0.8;
    l.sound("item.flintandsteel.use", rel, 1.0, pitch, CAT_BLOCKS, true);
    let fire_st = fire::fire_state_at(&*l, rel);
    engine::set_block(rs, l, rel, fire_st, fl::NEIGHBORS | fl::CLIENTS | fl::IMMEDIATE, UPDATE_LIMIT);
    Res::Done(UseOut { item: ItemOp::Damage(1), ..UseOut::default() })
}

/// `FireChargeItem.useOn`. The sound comes from no actor (`playSound(null)`),
/// the block change is `setBlockAndUpdate`, and the stack shrinks in any mode.
fn fire_charge<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, ctx: &UseCtx, face: u8, rng: &mut Rng) -> Res {
    let t = tables();
    if can_light(t, st) {
        let pitch = (rng.next_float() - rng.next_float()) * 0.2 + 1.0;
        l.sound("item.firecharge.use", pos, 1.0, pitch, CAT_BLOCKS, false);
        engine::set_block(rs, l, pos, setb(st, Pr::Lit, true), fl::ALL, UPDATE_LIMIT);
    } else {
        let rel = relative(pos, face);
        if !fire::can_be_lit(&*l, rel, face) {
            return Res::Fail;
        }
        let pitch = (rng.next_float() - rng.next_float()) * 0.2 + 1.0;
        l.sound("item.firecharge.use", rel, 1.0, pitch, CAT_BLOCKS, false);
        let fire_st = fire::fire_state_at(&*l, rel);
        engine::set_block(rs, l, rel, fire_st, fl::ALL, UPDATE_LIMIT);
    }
    Res::Done(UseOut { item: shrink_always(ctx), ..UseOut::default() })
}

/// Step 2 of a right click (`ItemStack.useOn`) for the items of this file.
/// `Pass` for every other item, which falls through to block placement.
pub fn use_on<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, ctx: &UseCtx, rng: &mut Rng) -> Res {
    let t = tables();
    let Some(kind) = t.items.get(&ctx.item).copied() else { return Res::Pass };
    match kind {
        // `BlockTransformer.playerHasBlockingItemUseIntent`: main hand, a shield
        // (the only `blocks_attacks` item) in the offhand, not sneaking
        Kind::Shovel | Kind::Hoe | Kind::Axe if ctx.main_hand && ctx.other == t.shield && !ctx.sneaking => Res::Pass,
        Kind::Shovel => transform(rs, l, &t.shovel, st, pos, ctx.face),
        Kind::Hoe => transform(rs, l, &t.hoe, st, pos, ctx.face),
        Kind::Axe => transform(rs, l, &t.axe, st, pos, ctx.face),
        Kind::Honeycomb => wax_on(rs, l, st, pos, ctx),
        Kind::BoneMeal => bone_meal(rs, l, st, pos, ctx, rng),
        Kind::FlintSteel => flint_and_steel(rs, l, st, pos, ctx.face, rng),
        Kind::FireCharge => fire_charge(rs, l, st, pos, ctx, ctx.face, rng),
    }
}

/// Whether `item` is one of the items that `CandleCakeBlock.useItemOn` passes
/// on (flint and steel, fire charge). The cake family must answer PASS for
/// them, or a click would eat the cake before it lights the candle.
#[allow(dead_code)] // for the candle cake family (`useItemOn`)
pub fn passes_on_candle_cake(item: u32) -> bool {
    let ids = &tables().ids;
    item != 0 && (item == ids.flint_and_steel || item == ids.fire_charge)
}

#[cfg(test)]
mod tests {
    use super::super::testw::World as TW;
    use super::super::{default_state, Beh};
    use super::*;
    use crate::game::blocks::props::{geti, seti};

    fn s(n: &str) -> u16 {
        default_state(n)
    }

    fn held(name: &str, count: i32, face: u8) -> UseCtx {
        UseCtx { item: item_id(name), count, damage: 0, main_hand: true, creative: false, sneaking: false, face, cursor: (0.5, 0.5, 0.5), yaw: 0.0, pos: [0.0; 3], can_eat: false, other: 0, sign: Default::default() }
    }

    fn run(w: &mut TW, rs: &mut Rs, p: BlockPos, name: &str, face: u8) -> Res {
        let mut rng = Rng::new(7);
        let st = w.get(p);
        let ctx = held(name, 3, face);
        use_on(rs, w, st, p, &ctx, &mut rng)
    }

    fn done_item(r: Res) -> ItemOp {
        match r {
            Res::Done(o) => o.item,
            other => panic!("expected Done, got {other:?}"),
        }
    }

    fn base() -> BlockPos {
        BlockPos::new(0, 64, 0)
    }

    #[test]
    fn every_tool_and_item_name_resolves() {
        rine_data::init();
        for mat in ["wooden", "copper", "stone", "golden", "iron", "diamond", "netherite"] {
            for k in ["shovel", "hoe", "axe"] {
                assert_ne!(item_id(&format!("{mat}_{k}")), 0, "{mat}_{k}");
            }
        }
        for n in ["honeycomb", "bone_meal", "flint_and_steel", "fire_charge", "hanging_roots"] {
            assert_ne!(item_id(n), 0, "{n}");
        }
        // 21 tools and 4 items
        assert_eq!(tables().items.len(), 25);
        for fam in COPPER {
            for n in fam {
                assert_ne!(block_id(n), NONE16, "{n}");
                assert_ne!(block_id(&format!("waxed_{n}")), NONE16, "waxed_{n}");
            }
        }
        for w in WOOD_PAIRS {
            assert_ne!(block_id(w), NONE16, "{w}");
            assert_ne!(block_id(&format!("stripped_{w}")), NONE16, "stripped_{w}");
        }
    }

    #[test]
    fn copy_props_keeps_shared_properties_only() {
        rine_data::init();
        // exposed copper stairs (facing, half, shape, waterlogged) to the waxed stairs
        let stairs = s("exposed_cut_copper_stairs");
        let to = copy_props(stairs, block_id("waxed_exposed_cut_copper_stairs"));
        assert_eq!(block_of(to), block_id("waxed_exposed_cut_copper_stairs"));
        assert_eq!(getv(to, Pr::Facing), getv(stairs, Pr::Facing));
        assert_eq!(getv(to, Pr::Half), getv(stairs, Pr::Half));
        // a log keeps its axis when stripped
        let log_z = b_set(s("oak_log"), "axis", "z");
        let stripped = copy_props(log_z, block_id("stripped_oak_log"));
        assert_eq!(rine_data::get().blocks.get_prop(stripped, "axis"), Some("z"));
    }

    fn b_set(st: u16, prop: &str, v: &str) -> u16 {
        rine_data::get().blocks.with_prop(st, prop, v).expect("prop")
    }

    #[test]
    fn hoe_tills_grass_and_dirt_and_flattens_coarse_dirt() {
        let mut rs = Rs::default();
        let p = base();
        let mut w = TW::new();
        w.put(p, s("grass_block"));
        assert_eq!(done_item(run(&mut w, &mut rs, p, "wooden_hoe", 1)), ItemOp::Damage(1));
        assert_eq!(w.get(p), s("farmland"));
        assert!(w.sounds.iter().any(|(n, _)| n.as_str() == "item.hoe.till"));
        // coarse dirt becomes dirt, not farmland
        let mut w = TW::new();
        w.put(p, s("coarse_dirt"));
        run(&mut w, &mut rs, p, "iron_hoe", 1);
        assert_eq!(w.get(p), s("dirt"));
    }

    #[test]
    fn hoe_vanilla_face_and_air_rules() {
        let mut rs = Rs::default();
        let p = base();
        // from below: the default transform is disallowed
        let mut w = TW::new();
        w.put(p, s("dirt"));
        assert_eq!(run(&mut w, &mut rs, p, "stone_hoe", DOWN), Res::Pass);
        assert_eq!(w.get(p), s("dirt"));
        // a block above (not the air tag): no farmland
        let mut w = TW::new();
        w.put(p, s("dirt"));
        w.put(p.offset(0, 1, 0), s("stone"));
        assert_eq!(run(&mut w, &mut rs, p, "stone_hoe", 1), Res::Pass);
        // a snow layer above is not air: no farmland
        let mut w = TW::new();
        w.put(p, s("dirt"));
        w.put(p.offset(0, 1, 0), s("snow"));
        assert_eq!(run(&mut w, &mut rs, p, "stone_hoe", 1), Res::Pass);
        // a cave air above is still the air tag
        let mut w = TW::new();
        w.put(p, s("dirt"));
        w.put(p.offset(0, 1, 0), s("cave_air"));
        assert_eq!(done_item(run(&mut w, &mut rs, p, "stone_hoe", 1)), ItemOp::Damage(1));
        assert_eq!(w.get(p), s("farmland"));
    }

    #[test]
    fn hoe_rooted_dirt_from_any_face_drops_hanging_roots() {
        let mut rs = Rs::default();
        let p = base();
        for face in [DOWN, 2, 1] {
            let mut w = TW::new();
            w.put(p, s("rooted_dirt"));
            // stone above, which the rooted transform does not check
            w.put(p.offset(0, 1, 0), s("stone"));
            assert_eq!(done_item(run(&mut w, &mut rs, p, "diamond_hoe", face)), ItemOp::Damage(1));
            assert_eq!(w.get(p), s("dirt"));
            assert_eq!(w.popped, vec![(p, item_id("hanging_roots"), 1)]);
        }
    }

    #[test]
    fn shovel_makes_dirt_path_only_on_top_with_air_above() {
        let mut rs = Rs::default();
        let p = base();
        let mut w = TW::new();
        w.put(p, s("dirt"));
        assert_eq!(done_item(run(&mut w, &mut rs, p, "golden_shovel", 1)), ItemOp::Damage(1));
        assert_eq!(w.get(p), s("dirt_path"));
        assert!(w.sounds.iter().any(|(n, _)| n.as_str() == "item.shovel.flatten"));
        // not from below, not with a block above, not on stone
        let mut w = TW::new();
        w.put(p, s("dirt"));
        assert_eq!(run(&mut w, &mut rs, p, "golden_shovel", DOWN), Res::Pass);
        let mut w = TW::new();
        w.put(p, s("dirt"));
        w.put(p.offset(0, 1, 0), s("stone"));
        assert_eq!(run(&mut w, &mut rs, p, "golden_shovel", 1), Res::Pass);
        let mut w = TW::new();
        w.put(p, s("stone"));
        assert_eq!(run(&mut w, &mut rs, p, "golden_shovel", 1), Res::Pass);
    }

    #[test]
    fn axe_strips_logs_keeping_the_axis_from_any_face() {
        let mut rs = Rs::default();
        let p = base();
        for face in [DOWN, 1, 4] {
            let mut w = TW::new();
            let log = b_set(s("oak_log"), "axis", "x");
            w.put(p, log);
            assert_eq!(done_item(run(&mut w, &mut rs, p, "iron_axe", face)), ItemOp::Damage(1));
            let st = w.get(p);
            assert_eq!(block_of(st), block_id("stripped_oak_log"));
            assert_eq!(rine_data::get().blocks.get_prop(st, "axis"), Some("x"));
            assert!(w.sounds.iter().any(|(n, _)| n.as_str() == "item.axe.strip"));
            // no particle for stripping
            assert!(w.events.is_empty());
        }
    }

    #[test]
    fn axe_scrapes_one_stage_and_waxes_off() {
        let mut rs = Rs::default();
        let p = base();
        let mut w = TW::new();
        w.put(p, s("oxidized_copper"));
        run(&mut w, &mut rs, p, "stone_axe", 1);
        assert_eq!(w.get(p), s("weathered_copper"));
        assert_eq!(w.events, vec![(EV_SCRAPE, p, 0)]);
        // the unaffected copper has nothing to scrape
        let mut w = TW::new();
        w.put(p, s("copper_block"));
        assert_eq!(run(&mut w, &mut rs, p, "stone_axe", 1), Res::Pass);
        // waxed: wax off keeps the stage, particle wax off
        let mut w = TW::new();
        w.put(p, s("waxed_oxidized_copper"));
        run(&mut w, &mut rs, p, "stone_axe", 1);
        assert_eq!(w.get(p), s("oxidized_copper"));
        assert_eq!(w.events, vec![(EV_WAX_OFF, p, 0)]);
        // waxed unaffected -> unaffected
        let mut w = TW::new();
        w.put(p, s("waxed_copper_block"));
        run(&mut w, &mut rs, p, "stone_axe", 1);
        assert_eq!(w.get(p), s("copper_block"));
    }

    #[test]
    fn axe_copies_stairs_and_bars_state() {
        let mut rs = Rs::default();
        let p = base();
        let mut w = TW::new();
        let stairs = b_set(b_set(s("weathered_cut_copper_stairs"), "half", "top"), "facing", "east");
        w.put(p, stairs);
        run(&mut w, &mut rs, p, "iron_axe", 1);
        let st = w.get(p);
        assert_eq!(block_of(st), block_id("exposed_cut_copper_stairs"));
        assert_eq!(rine_data::get().blocks.get_prop(st, "half"), Some("top"));
        assert_eq!(rine_data::get().blocks.get_prop(st, "facing"), Some("east"));
    }

    #[test]
    fn axe_copper_door_scrapes_without_neighbour_update() {
        let mut rs = Rs::default();
        let p = base();
        let mut w = TW::new();
        let lower = b_set(b_set(s("exposed_copper_door"), "half", "lower"), "facing", "north");
        w.put(p, lower);
        run(&mut w, &mut rs, p, "iron_axe", 1);
        let st = w.get(p);
        assert_eq!(block_of(st), block_id("copper_door"));
        assert_eq!(rine_data::get().blocks.get_prop(st, "half"), Some("lower"));
        assert_eq!(rine_data::get().blocks.get_prop(st, "facing"), Some("north"));
        assert_eq!(w.events, vec![(EV_SCRAPE, p, 0)]);
    }

    #[test]
    fn axe_double_copper_chest_also_marks_the_other_half() {
        let mut rs = Rs::default();
        let p = base();
        let mut w = TW::new();
        // facing north, left half: the partner is east (clockwise of north)
        let left = b_set(b_set(s("oxidized_copper_chest"), "type", "left"), "facing", "north");
        w.put(p, left);
        run(&mut w, &mut rs, p, "iron_axe", 1);
        assert_eq!(block_of(w.get(p)), block_id("weathered_copper_chest"));
        let partner = p.offset(1, 0, 0);
        assert_eq!(w.events, vec![(EV_SCRAPE, p, 0), (EV_SCRAPE, partner, 0)]);
        // the other half keeps its state (vanilla changes only the clicked half)
        assert_eq!(w.get(partner), 0);
    }

    #[test]
    fn creative_and_worn_tools_change_the_block_the_same_way() {
        let mut rs = Rs::default();
        let p = base();
        let mut w = TW::new();
        w.put(p, s("dirt"));
        let mut ctx = held("iron_shovel", 1, 1);
        ctx.creative = true;
        let st = w.get(p);
        let r = use_on(&mut rs, &mut w, st, p, &ctx, &mut Rng::new(1));
        assert_eq!(done_item(r), ItemOp::Damage(1));
        assert_eq!(w.get(p), s("dirt_path"));
        // a tool with one point of durability left still works
        let mut w = TW::new();
        w.put(p, s("dirt"));
        let mut ctx = held("iron_shovel", 1, 1);
        ctx.damage = 1000;
        let st = w.get(p);
        assert_eq!(done_item(use_on(&mut rs, &mut w, st, p, &ctx, &mut Rng::new(1))), ItemOp::Damage(1));
        assert_eq!(w.get(p), s("dirt_path"));
    }

    #[test]
    fn honeycomb_waxes_copper_and_consumes_one() {
        let mut rs = Rs::default();
        let p = base();
        let mut w = TW::new();
        let st = b_set(s("exposed_cut_copper_stairs"), "half", "bottom");
        w.put(p, st);
        let r = run(&mut w, &mut rs, p, "honeycomb", 1);
        assert_eq!(done_item(r), ItemOp::Shrink(1));
        assert_eq!(block_of(w.get(p)), block_id("waxed_exposed_cut_copper_stairs"));
        assert_eq!(rine_data::get().blocks.get_prop(w.get(p), "half"), Some("bottom"));
        assert_eq!(w.events, vec![(EV_WAX_ON, p, 0)]);
        assert!(w.sounds.iter().any(|(n, _)| n.as_str() == "item.honeycomb.wax_on"));
    }

    #[test]
    fn honeycomb_on_waxed_or_plain_blocks_passes() {
        let mut rs = Rs::default();
        let p = base();
        let mut w = TW::new();
        w.put(p, s("waxed_copper_block"));
        assert_eq!(run(&mut w, &mut rs, p, "honeycomb", 1), Res::Pass);
        let mut w = TW::new();
        w.put(p, s("stone"));
        assert_eq!(run(&mut w, &mut rs, p, "honeycomb", 1), Res::Pass);
        assert!(w.events.is_empty());
    }

    #[test]
    fn honeycomb_on_double_chest_plays_twice_at_the_clicked_pos() {
        let mut rs = Rs::default();
        let p = base();
        let mut w = TW::new();
        w.put(p, b_set(s("copper_chest"), "type", "right"));
        w.put(p.offset(1, 0, 0), b_set(s("copper_chest"), "type", "left"));
        run(&mut w, &mut rs, p, "honeycomb", 1);
        assert_eq!(block_of(w.get(p)), block_id("waxed_copper_chest"));
        assert_eq!(w.events, vec![(EV_WAX_ON, p, 0), (EV_WAX_ON, p.offset(-1, 0, 0), 0)]);
        assert_eq!(w.sounds.iter().filter(|(n, q)| n.as_str() == "item.honeycomb.wax_on" && *q == p).count(), 2);
    }

    #[test]
    fn honeycomb_shrinks_in_creative_as_vanilla_does() {
        let mut rs = Rs::default();
        let p = base();
        let mut w = TW::new();
        w.put(p, s("copper_block"));
        let mut ctx = held("honeycomb", 1, 1);
        ctx.creative = true;
        let st = w.get(p);
        let r = use_on(&mut rs, &mut w, st, p, &ctx, &mut Rng::new(2));
        // creative: the caller keeps the stack (count restored after useOn)
        assert_eq!(done_item(r), ItemOp::Shrink(1));
    }

    #[test]
    fn bone_meal_grows_a_crop_and_plays_growth_particles() {
        let mut rs = Rs::default();
        let p = base();
        let mut w = TW::new();
        w.put(p.offset(0, -1, 0), s("farmland"));
        w.put(p, seti(s("wheat"), Pr::Age, 1));
        w.light = 15;
        let r = run(&mut w, &mut rs, p, "bone_meal", 1);
        assert_eq!(done_item(r), ItemOp::Shrink(1));
        let age = geti(w.get(p), Pr::Age);
        // age 1 + range(2, 5)
        assert!((3..=6).contains(&age), "{age}");
        assert_eq!(w.events, vec![(EV_PLANT_GROWTH, p, 15)]);
    }

    #[test]
    fn bone_meal_on_a_full_grown_crop_or_stone_passes() {
        let mut rs = Rs::default();
        let p = base();
        let mut w = TW::new();
        w.put(p.offset(0, -1, 0), s("farmland"));
        w.put(p, seti(s("wheat"), Pr::Age, 7));
        assert_eq!(run(&mut w, &mut rs, p, "bone_meal", 1), Res::Pass);
        assert!(w.events.is_empty());
        let mut w = TW::new();
        w.put(p, s("stone"));
        assert_eq!(run(&mut w, &mut rs, p, "bone_meal", 1), Res::Pass);
    }

    #[test]
    fn bone_meal_grows_a_sweet_berry_bush() {
        let mut rs = Rs::default();
        let p = base();
        let mut w = TW::new();
        w.put(p, seti(s("sweet_berry_bush"), Pr::Age, 2));
        let r = run(&mut w, &mut rs, p, "bone_meal", 1);
        assert!(matches!(r, Res::Done(_)));
        assert_eq!(geti(w.get(p), Pr::Age), 3);
    }

    #[test]
    fn flint_lights_a_campfire_candle_and_cake() {
        let mut rs = Rs::default();
        let p = base();
        // unlit campfire
        let mut w = TW::new();
        w.put(p, b_set(b_set(s("campfire"), "lit", "false"), "waterlogged", "false"));
        assert_eq!(done_item(run(&mut w, &mut rs, p, "flint_and_steel", 1)), ItemOp::Damage(1));
        assert_eq!(rine_data::get().blocks.get_prop(w.get(p), "lit"), Some("true"));
        assert!(w.sounds.iter().any(|(n, _)| n.as_str() == "item.flintandsteel.use"));
        // a waterlogged campfire does not light: the fire goes next to it, if it can (no)
        let mut w = TW::new();
        w.put(p, b_set(b_set(s("campfire"), "lit", "false"), "waterlogged", "true"));
        assert_eq!(run(&mut w, &mut rs, p, "flint_and_steel", 1), Res::Fail);
        // a candle, unlit
        let mut w = TW::new();
        w.put(p, b_set(b_set(s("candle"), "lit", "false"), "waterlogged", "false"));
        assert!(matches!(run(&mut w, &mut rs, p, "flint_and_steel", 1), Res::Done(_)));
        assert_eq!(rine_data::get().blocks.get_prop(w.get(p), "lit"), Some("true"));
        // a candle cake
        let mut w = TW::new();
        w.put(p, b_set(s("candle_cake"), "lit", "false"));
        assert!(matches!(run(&mut w, &mut rs, p, "flint_and_steel", 1), Res::Done(_)));
        assert_eq!(rine_data::get().blocks.get_prop(w.get(p), "lit"), Some("true"));
    }

    #[test]
    fn lit_campfire_is_not_relit() {
        let mut rs = Rs::default();
        let p = base();
        let mut w = TW::new();
        w.put(p.offset(0, -1, 0), s("stone"));
        let lit = b_set(b_set(s("campfire"), "lit", "true"), "waterlogged", "false");
        w.put(p, lit);
        // not lightable, so the fire goes above; a campfire top does not hold fire
        // (data sturdy face), so this fails and the campfire is untouched
        assert_eq!(run(&mut w, &mut rs, p, "flint_and_steel", 1), Res::Fail);
        assert_eq!(w.get(p), lit);
    }

    #[test]
    fn flint_places_fire_on_the_clicked_face_neighbour() {
        let mut rs = Rs::default();
        let p = base();
        let mut w = TW::new();
        w.put(p, s("stone"));
        w.put(p.offset(0, -1, 0), s("stone"));
        // up: the fire goes at p + (0,1,0), supported by stone at p
        let r = run(&mut w, &mut rs, p, "flint_and_steel", 1);
        assert_eq!(done_item(r), ItemOp::Damage(1));
        assert_eq!(block_of(w.get(p.offset(0, 1, 0))), block_id("fire"));
        assert_eq!(w.get(p), s("stone"));
        assert!(w.sounds.iter().any(|(n, q)| n.as_str() == "item.flintandsteel.use" && *q == p.offset(0, 1, 0)));
    }

    #[test]
    fn flint_on_the_wrong_face_fails() {
        let mut rs = Rs::default();
        let p = base();
        // clicked from below on a stone block: the fire would go under it,
        // with nothing to stand on
        let mut w = TW::new();
        w.put(p, s("stone"));
        assert_eq!(run(&mut w, &mut rs, p, "flint_and_steel", DOWN), Res::Fail);
        assert!(w.cells.get(&p.offset(0, -1, 0)).is_none());
    }

    #[test]
    fn flint_with_a_block_in_the_way_fails() {
        let mut rs = Rs::default();
        let p = base();
        let mut w = TW::new();
        w.put(p, s("stone"));
        w.put(p.offset(0, -1, 0), s("stone"));
        w.put(p.offset(0, 1, 0), s("stone"));
        assert_eq!(run(&mut w, &mut rs, p, "flint_and_steel", 1), Res::Fail);
        assert_eq!(w.get(p.offset(0, 1, 0)), s("stone"));
    }

    #[test]
    fn fire_charge_lights_a_candle_or_places_fire_and_shrinks_one() {
        let mut rs = Rs::default();
        let p = base();
        // candle cake lit by a fire charge, one left in the stack
        let mut w = TW::new();
        w.put(p, b_set(s("candle_cake"), "lit", "false"));
        let r = run(&mut w, &mut rs, p, "fire_charge", 1);
        assert_eq!(done_item(r), ItemOp::Shrink(1));
        assert_eq!(rine_data::get().blocks.get_prop(w.get(p), "lit"), Some("true"));
        assert!(w.sounds.iter().any(|(n, _)| n.as_str() == "item.firecharge.use"));
        // fire on the clicked face
        let mut w = TW::new();
        w.put(p, s("stone"));
        w.put(p.offset(0, -1, 0), s("stone"));
        assert!(matches!(run(&mut w, &mut rs, p, "fire_charge", 1), Res::Done(_)));
        assert_eq!(block_of(w.get(p.offset(0, 1, 0))), block_id("fire"));
    }

    #[test]
    fn fire_charge_fails_where_flint_fails_and_empties_a_single_stack() {
        let mut rs = Rs::default();
        let p = base();
        let mut w = TW::new();
        w.put(p, s("stone"));
        assert_eq!(run(&mut w, &mut rs, p, "fire_charge", DOWN), Res::Fail);
        // count 1 -> empty
        let mut w = TW::new();
        w.put(p, b_set(s("candle_cake"), "lit", "false"));
        let mut rng = Rng::new(3);
        let st = w.get(p);
        let r = use_on(&mut rs, &mut w, st, p, &held("fire_charge", 1, 1), &mut rng);
        assert_eq!(done_item(r), ItemOp::Shrink(1));
    }

    #[test]
    fn candle_cake_passes_flint_and_fire_charge_to_the_item_step() {
        rine_data::init();
        assert!(passes_on_candle_cake(item_id("flint_and_steel")));
        assert!(passes_on_candle_cake(item_id("fire_charge")));
        assert!(!passes_on_candle_cake(item_id("honeycomb")));
    }

    #[test]
    fn unknown_items_pass_and_beh_is_untouched() {
        let mut rs = Rs::default();
        let p = base();
        let mut w = TW::new();
        w.put(p, s("dirt"));
        assert_eq!(run(&mut w, &mut rs, p, "stick", 1), Res::Pass);
        assert_eq!(w.get(p), s("dirt"));
        // the family does not claim blocks
        assert_eq!(crate::game::blocks::beh(s("dirt")), Beh::None);
    }

    #[test]
    fn axe_on_door_lower_half_changes_the_clicked_half() {
        // vanilla: the upper half follows through DoorBlock.updateShape. The
        // door behaviour (shape.rs, Beh::Door) does not do that yet, so only
        // the clicked half is checked here.
        let mut rs = Rs::default();
        let p = base();
        let mut w = TW::new();
        w.put(p, b_set(s("exposed_copper_door"), "half", "lower"));
        w.put(p.offset(0, 1, 0), b_set(s("exposed_copper_door"), "half", "upper"));
        run(&mut w, &mut rs, p, "iron_axe", 1);
        assert_eq!(block_of(w.get(p)), block_id("copper_door"));
    }

    #[test]
    fn shield_in_offhand_blocks_tool_transforms() {
        // `BlockTransformer.playerHasBlockingItemUseIntent`
        let mut rs = Rs::default();
        let p = base();
        let mut w = TW::new();
        w.put(p, s("dirt"));
        let mut ctx = held("iron_hoe", 1, 1);
        ctx.other = item_id("shield");
        assert_eq!(use_on(&mut rs, &mut w, s("dirt"), p, &ctx, &mut Rng::new(1)), Res::Pass);
        assert_eq!(w.get(p), s("dirt"));
        // sneaking (secondary use) lets the hoe work
        ctx.sneaking = true;
        assert!(matches!(use_on(&mut rs, &mut w, s("dirt"), p, &ctx, &mut Rng::new(1)), Res::Done(_)));
        assert_eq!(block_of(w.get(p)), block_id("farmland"));
    }
}