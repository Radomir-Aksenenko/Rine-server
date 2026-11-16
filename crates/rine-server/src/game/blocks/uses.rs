//! Blocks a player right-clicks: `BellBlock`, `ComposterBlock`,
//! `FlowerPotBlock`, `CakeBlock`, `CandleCakeBlock`, `CandleBlock` and
//! `RespawnAnchorBlock`, plus flint and steel or a fire charge on candles
//! (`FlintAndSteelItem.useOn`, `FireChargeItem.useOn`).
//!
//! Each hook answers one step of `ServerPlayerGameMode.useItemOn` through
//! `interact.rs`. Effects on the player (held stack, items, food) come back in
//! `UseOut`. The composter's scheduled tick and `onPlace`, and the bell's
//! `neighborChanged` ring, are hooks of the engine dispatchers in `mod.rs`.
//!
//! Not here: the jukebox (discs need the `jukebox_playable` item component,
//! which Rine does not model), composter hopper input and output, the bell's
//! raid glow and resonance, arrows hitting bells, particles and game events.

use super::interact::{ItemOp, Res, UseCtx, UseOut};
use super::props::{geti, getb, getv, seti, setb, vid, Pr};
use super::{block_of, Beh, Rng};
use crate::game::player::Stack;
use crate::game::redstone::data::{self, DOWN, NORTH, SOUTH, UP};
use crate::game::redstone::engine::{self, fl, prio, Lvl, CAT_BLOCKS, UPDATE_LIMIT};
use crate::game::redstone::Rs;
use rine_protocol::BlockPos;
use std::sync::OnceLock;

/// `BellBlock.EVENT_BELL_RING`.
const EVENT_BELL_RING: u8 = 1;
/// `LevelEvent.COMPOSTER_FILL`.
const COMPOSTER_FILL: i32 = 1500;
/// `ComposterBlock.READY`: the level after the 20 tick ripening.
const COMPOSTER_READY: i32 = 8;
/// `ComposterBlock.MAX_LEVEL`: a full input layer ripens.
const COMPOSTER_MAX: i32 = 7;
/// Ticks between level 7 and ready.
const RIPEN_TICKS: u32 = 20;
/// `BellBlock.isProperHit`: a click above this y is on the bell's top.
const BELL_TOP: f32 = 0.8124;
/// `CakeBlock.MAX_BITES`: the seventh slice removes the cake.
const MAX_BITES: i32 = 6;
/// `CakeBlock.eat`: `FoodData.eat(2, 0.1F)`.
const CAKE_NUTRITION: i32 = 2;
const CAKE_SATURATION: f32 = 0.1;
/// `RespawnAnchorBlock.explode` power.
const ANCHOR_POWER: f32 = 5.0;

// Composter chances: `ContextIntProviders.compostable(blocks, chance)`.
const LOW: u8 = 30;
const LOW_MEDIUM: u8 = 50;
const MEDIUM: u8 = 65;
const MEDIUM_HIGH: u8 = 85;
/// `compostable/always_add_one`: one layer, no roll.
const ALWAYS: u8 = 100;

/// Compostable items and their chance, from `Items.java` (`compostable(...)`).
const COMPOST: &[(&str, u8)] = &[
    ("oak_sapling", LOW),
    ("spruce_sapling", LOW),
    ("birch_sapling", LOW),
    ("jungle_sapling", LOW),
    ("acacia_sapling", LOW),
    ("cherry_sapling", LOW),
    ("dark_oak_sapling", LOW),
    ("pale_oak_sapling", LOW),
    ("mangrove_propagule", LOW),
    ("poplar_sapling", LOW),
    ("mangrove_roots", LOW),
    ("oak_leaves", LOW),
    ("spruce_leaves", LOW),
    ("birch_leaves", LOW),
    ("jungle_leaves", LOW),
    ("acacia_leaves", LOW),
    ("cherry_leaves", LOW),
    ("dark_oak_leaves", LOW),
    ("pale_oak_leaves", LOW),
    ("mangrove_leaves", LOW),
    ("red_poplar_leaves", LOW),
    ("orange_poplar_leaves", LOW),
    ("yellow_poplar_leaves", LOW),
    ("azalea_leaves", LOW),
    ("flowering_azalea_leaves", LOW_MEDIUM),
    ("short_grass", LOW),
    ("fern", MEDIUM),
    ("bush", LOW),
    ("red_shrub", LOW),
    ("azalea", MEDIUM),
    ("flowering_azalea", MEDIUM_HIGH),
    ("firefly_bush", LOW),
    ("short_dry_grass", LOW),
    ("tall_dry_grass", LOW),
    ("seagrass", LOW),
    ("sea_pickle", MEDIUM),
    ("dandelion", MEDIUM),
    ("open_eyeblossom", MEDIUM),
    ("closed_eyeblossom", MEDIUM),
    ("poppy", MEDIUM),
    ("blue_orchid", MEDIUM),
    ("allium", MEDIUM),
    ("azure_bluet", MEDIUM),
    ("red_tulip", MEDIUM),
    ("orange_tulip", MEDIUM),
    ("white_tulip", MEDIUM),
    ("pink_tulip", MEDIUM),
    ("oxeye_daisy", MEDIUM),
    ("cornflower", MEDIUM),
    ("lily_of_the_valley", MEDIUM),
    ("wither_rose", MEDIUM),
    ("torchflower", MEDIUM_HIGH),
    ("pitcher_plant", MEDIUM_HIGH),
    ("spore_blossom", MEDIUM),
    ("brown_mushroom", MEDIUM),
    ("red_mushroom", MEDIUM),
    ("shelf_mushroom", MEDIUM),
    ("crimson_fungus", MEDIUM),
    ("warped_fungus", MEDIUM),
    ("crimson_roots", MEDIUM),
    ("warped_roots", MEDIUM),
    ("nether_sprouts", LOW_MEDIUM),
    ("weeping_vines", LOW_MEDIUM),
    ("twisting_vines", LOW_MEDIUM),
    ("sugar_cane", LOW_MEDIUM),
    ("kelp", LOW),
    ("pink_petals", LOW),
    ("wildflowers", LOW),
    ("leaf_litter", LOW),
    ("moss_carpet", LOW),
    ("moss_block", MEDIUM),
    ("pale_moss_carpet", LOW),
    ("pale_hanging_moss", LOW),
    ("pale_moss_block", MEDIUM),
    ("hanging_roots", LOW),
    ("big_dripleaf", MEDIUM),
    ("small_dripleaf", LOW),
    ("cactus", LOW_MEDIUM),
    ("cactus_flower", LOW),
    ("pumpkin", MEDIUM),
    ("carved_pumpkin", MEDIUM),
    ("brown_mushroom_block", MEDIUM_HIGH),
    ("red_mushroom_block", MEDIUM_HIGH),
    ("mushroom_stem", MEDIUM),
    ("melon", MEDIUM),
    ("vine", LOW_MEDIUM),
    ("glow_lichen", LOW_MEDIUM),
    ("lily_pad", MEDIUM),
    ("hay_block", MEDIUM_HIGH),
    ("sunflower", MEDIUM),
    ("lilac", MEDIUM),
    ("rose_bush", MEDIUM),
    ("peony", MEDIUM),
    ("tall_grass", LOW_MEDIUM),
    ("large_fern", MEDIUM),
    ("nether_wart_block", MEDIUM_HIGH),
    ("warped_wart_block", MEDIUM_HIGH),
    ("apple", MEDIUM),
    ("wheat_seeds", LOW),
    ("wheat", MEDIUM),
    ("bread", MEDIUM_HIGH),
    ("dried_kelp_block", LOW_MEDIUM),
    ("cocoa_beans", MEDIUM),
    ("cake", ALWAYS),
    ("straw_bed", MEDIUM_HIGH),
    ("cookie", MEDIUM_HIGH),
    ("melon_slice", LOW_MEDIUM),
    ("dried_kelp", LOW),
    ("pumpkin_seeds", LOW),
    ("melon_seeds", LOW),
    ("nether_wart", MEDIUM),
    ("carrot", MEDIUM),
    ("potato", MEDIUM),
    ("baked_potato", MEDIUM_HIGH),
    ("pumpkin_pie", ALWAYS),
    ("torchflower_seeds", LOW),
    ("pitcher_pod", LOW),
    ("beetroot", MEDIUM),
    ("beetroot_seeds", LOW),
    ("sweet_berries", LOW),
    ("glow_berries", LOW),
    ("shroomlight", MEDIUM),
];

/// Flower pot content: (plant item, potted block), `FlowerPotBlock` constructors in `Blocks.java`.
const POTS: &[(&str, &str)] = &[
    ("torchflower", "potted_torchflower"),
    ("oak_sapling", "potted_oak_sapling"),
    ("spruce_sapling", "potted_spruce_sapling"),
    ("birch_sapling", "potted_birch_sapling"),
    ("jungle_sapling", "potted_jungle_sapling"),
    ("acacia_sapling", "potted_acacia_sapling"),
    ("cherry_sapling", "potted_cherry_sapling"),
    ("dark_oak_sapling", "potted_dark_oak_sapling"),
    ("pale_oak_sapling", "potted_pale_oak_sapling"),
    ("poplar_sapling", "potted_poplar_sapling"),
    ("mangrove_propagule", "potted_mangrove_propagule"),
    ("fern", "potted_fern"),
    ("dandelion", "potted_dandelion"),
    ("golden_dandelion", "potted_golden_dandelion"),
    ("poppy", "potted_poppy"),
    ("blue_orchid", "potted_blue_orchid"),
    ("allium", "potted_allium"),
    ("azure_bluet", "potted_azure_bluet"),
    ("red_tulip", "potted_red_tulip"),
    ("orange_tulip", "potted_orange_tulip"),
    ("white_tulip", "potted_white_tulip"),
    ("pink_tulip", "potted_pink_tulip"),
    ("oxeye_daisy", "potted_oxeye_daisy"),
    ("cornflower", "potted_cornflower"),
    ("lily_of_the_valley", "potted_lily_of_the_valley"),
    ("wither_rose", "potted_wither_rose"),
    ("red_mushroom", "potted_red_mushroom"),
    ("brown_mushroom", "potted_brown_mushroom"),
    ("dead_bush", "potted_dead_bush"),
    ("cactus", "potted_cactus"),
    ("bamboo", "potted_bamboo"),
    ("crimson_fungus", "potted_crimson_fungus"),
    ("warped_fungus", "potted_warped_fungus"),
    ("crimson_roots", "potted_crimson_roots"),
    ("warped_roots", "potted_warped_roots"),
    ("azalea", "potted_azalea_bush"),
    ("flowering_azalea", "potted_flowering_azalea_bush"),
    ("open_eyeblossom", "potted_open_eyeblossom"),
    ("closed_eyeblossom", "potted_closed_eyeblossom"),
];

/// Candle items; each has a candle cake of the same colour.
const CANDLES: &[&str] = &[
    "candle",
    "white_candle",
    "orange_candle",
    "magenta_candle",
    "light_blue_candle",
    "yellow_candle",
    "lime_candle",
    "pink_candle",
    "gray_candle",
    "light_gray_candle",
    "cyan_candle",
    "purple_candle",
    "blue_candle",
    "brown_candle",
    "green_candle",
    "red_candle",
    "black_candle",
];

/// Ids and state tables, built once.
struct T {
    /// Composter chance by item id (0: not compostable).
    compost: Vec<u8>,
    /// Potted state by plant item id (0: cannot be potted).
    pot_of_plant: Vec<u16>,
    /// Plant item id by potted block id (0: not a potted plant).
    plant_of_pot: Vec<u32>,
    /// Candle cake state by candle item id (0: not a candle).
    cake_of_candle: Vec<u16>,
    flower_pot: u16,
    flower_pot_state: u16,
    cake_state: u16,
    glowstone: u32,
    flint: u32,
    fire_charge: u32,
    bone_meal: u32,
    floor: u8,
    ceiling: u8,
    single_wall: u8,
    double_wall: u8,
}

fn t() -> &'static T {
    static T_: OnceLock<T> = OnceLock::new();
    T_.get_or_init(build)
}

/// Default state by name; air if the block is unknown.
fn state_of(name: &str) -> u16 {
    rine_data::get().blocks.block(name).map(|b| b.default_state as u16).unwrap_or(0)
}

/// Dense table from (index, value) pairs; index 0 means "unknown" and is dropped.
fn dense<V: Copy + Default>(pairs: Vec<(usize, V)>) -> Vec<V> {
    let n = pairs.iter().map(|p| p.0 + 1).max().unwrap_or(0);
    let mut v = vec![V::default(); n];
    for (i, x) in pairs {
        v[i] = x;
    }
    v
}

fn build() -> T {
    let mut compost = Vec::new();
    for &(name, chance) in COMPOST {
        let id = super::item_id(name) as usize;
        if id != 0 {
            compost.push((id, chance));
        }
    }
    let mut pot_of_plant = Vec::new();
    let mut plant_of_pot = Vec::new();
    for &(plant, potted) in POTS {
        let id = super::item_id(plant) as usize;
        let blk = super::block_id(potted);
        if id == 0 || blk == u16::MAX {
            continue;
        }
        pot_of_plant.push((id, state_of(potted)));
        plant_of_pot.push((blk as usize, id as u32));
    }
    let mut cake_of_candle = Vec::new();
    for &name in CANDLES {
        let id = super::item_id(name) as usize;
        if id != 0 {
            cake_of_candle.push((id, state_of(&name.replacen("candle", "candle_cake", 1))));
        }
    }
    T {
        compost: dense(compost),
        pot_of_plant: dense(pot_of_plant),
        plant_of_pot: dense(plant_of_pot),
        cake_of_candle: dense(cake_of_candle),
        flower_pot: super::block_id("flower_pot"),
        flower_pot_state: state_of("flower_pot"),
        cake_state: state_of("cake"),
        glowstone: super::item_id("glowstone"),
        flint: super::item_id("flint_and_steel"),
        fire_charge: super::item_id("fire_charge"),
        bone_meal: super::item_id("bone_meal"),
        floor: vid("floor"),
        ceiling: vid("ceiling"),
        single_wall: vid("single_wall"),
        double_wall: vid("double_wall"),
    }
}

pub fn classify(n: &str) -> Option<Beh> {
    Some(match n {
        "bell" => Beh::Bell,
        "composter" => Beh::Composter,
        "flower_pot" => Beh::FlowerPot,
        _ if n.starts_with("potted_") => Beh::FlowerPot,
        "cake" => Beh::Cake,
        "candle" => Beh::Candle,
        _ if n.ends_with("_candle") => Beh::Candle,
        "candle_cake" => Beh::CandleCake,
        _ if n.ends_with("_candle_cake") => Beh::CandleCake,
        "respawn_anchor" => Beh::RespawnAnchor,
        _ => return None,
    })
}

/// `UseOut` that only shrinks the held stack (`stack.consume(1, player)`).
fn shrink_one() -> Res {
    Res::Done(UseOut { item: ItemOp::Shrink(1), ..UseOut::default() })
}

/// Composter chance of an item id, if it is compostable.
#[inline]
fn compost_chance(item: u32) -> Option<u8> {
    t().compost.get(item as usize).copied().filter(|&c| c > 0)
}

// ------------------------------------------------------------------ dispatch

/// Step 1a: `BlockState.useItemOn` with the held stack.
pub fn use_item_on<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, ctx: &UseCtx, rng: &mut Rng) -> Res {
    match super::beh(st) {
        Beh::Composter => composter_use_item(rs, l, st, pos, ctx, rng),
        Beh::FlowerPot => pot_use_item(rs, l, st, pos, ctx),
        Beh::Cake => cake_use_item(rs, l, st, pos, ctx),
        Beh::Candle => candle_use_item(rs, l, st, pos, ctx),
        Beh::CandleCake => candle_cake_use_item(rs, l, st, pos, ctx),
        Beh::RespawnAnchor => anchor_use_item(rs, l, st, pos, ctx),
        _ => Res::TryEmptyHand,
    }
}

/// Step 1b: `BlockState.useWithoutItem` (main hand only).
pub fn use_without_item<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, ctx: &UseCtx, rng: &mut Rng) -> Res {
    match super::beh(st) {
        Beh::Bell => bell_use(l, st, pos, ctx),
        Beh::Composter => composter_use_without_item(rs, l, st, pos, rng),
        Beh::FlowerPot => pot_use_without_item(rs, l, st, pos),
        Beh::Cake => cake_eat(rs, l, st, pos, ctx),
        Beh::CandleCake => candle_cake_use_without_item(rs, l, st, pos, ctx),
        Beh::RespawnAnchor => anchor_use_without_item(rs, l, st, pos),
        _ => Res::Pass,
    }
}

/// Scheduled and placed-state hooks of the composter.
pub fn tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    if geti(st, Pr::Level) != COMPOSTER_MAX {
        return;
    }
    // `ComposterBlock.tick`: level 7 cycles to 8
    engine::set_block(rs, l, pos, seti(st, Pr::Level, COMPOSTER_READY), fl::ALL, UPDATE_LIMIT);
    l.sound("block.composter.ready", pos, 1.0, 1.0, CAT_BLOCKS, false);
}

/// `ComposterBlock.onPlace`: a full layer starts ripening.
pub fn on_place<L: Lvl>(rs: &mut Rs, l: &L, st: u16, pos: BlockPos) {
    if geti(st, Pr::Level) == COMPOSTER_MAX {
        engine::schedule(rs, l, pos, block_of(st), RIPEN_TICKS, prio::NORMAL);
    }
}

/// `BellBlock.neighborChanged`: a powered bell rings once on the rising edge.
pub fn neighbor_changed<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    let signal = engine::has_neighbor_signal(rs, &*l, pos);
    if signal == getb(st, Pr::Powered) {
        return;
    }
    if signal {
        ring(l, st, pos, data::get(st, data::P::Facing));
    }
    engine::set_block(rs, l, pos, setb(st, Pr::Powered, signal), fl::ALL, UPDATE_LIMIT);
}

// ------------------------------------------------------------------ bell

/// `BellBlock.isProperHit`: the click is on the bell's side, not its top.
fn bell_proper_hit(st: u16, face: u8, cy: f32) -> bool {
    if face == UP || face == DOWN || cy > BELL_TOP {
        return false;
    }
    let t = t();
    let clicked_z = face == NORTH || face == SOUTH;
    let facing_z = matches!(data::get(st, data::P::Facing), NORTH | SOUTH);
    let attach = getv(st, Pr::Attachment);
    if attach == t.floor {
        facing_z == clicked_z
    } else if attach == t.single_wall || attach == t.double_wall {
        facing_z != clicked_z
    } else {
        attach == t.ceiling
    }
}

/// `BellBlock.useWithoutItem` -> `onHit(..., requireHitFromCorrectSide = true)`.
fn bell_use<L: Lvl>(l: &mut L, st: u16, pos: BlockPos, ctx: &UseCtx) -> Res {
    if !bell_proper_hit(st, ctx.face, ctx.cursor.1) {
        return Res::Pass;
    }
    ring(l, st, pos, ctx.face);
    Res::swing()
}

/// `BellBlock.attemptToRing` as seen from the clients: the ring event, then
/// the sound to everyone. The bell block entity swing is client-only state.
fn ring<L: Lvl>(l: &mut L, st: u16, pos: BlockPos, dir: u8) {
    l.block_event_packet(pos, block_of(st), EVENT_BELL_RING, dir);
    l.sound("block.bell.use", pos, 2.0, 1.0, CAT_BLOCKS, false);
}

// ------------------------------------------------------------------ composter

/// `ComposterBlock.useItemOn`.
fn composter_use_item<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, ctx: &UseCtx, rng: &mut Rng) -> Res {
    let level = geti(st, Pr::Level);
    let Some(chance) = compost_chance(ctx.item).filter(|_| level < COMPOSTER_READY) else {
        return Res::TryEmptyHand;
    };
    if level >= COMPOSTER_MAX {
        // a full input layer takes nothing and is not used up
        return Res::swing();
    }
    let layers = add_layer(rs, l, st, pos, level, chance, rng);
    l.level_event(COMPOSTER_FILL, pos, (layers > 0) as i32);
    shrink_one()
}

/// `ComposterBlock.addLayer`: returns the layers added (0 when the roll fails).
/// Level 0 always takes one layer and a chance of 100 does not roll
/// (the `NumberDispatcher` empty case and `ConstantValue`).
fn add_layer<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, level: i32, chance: u8, rng: &mut Rng) -> i32 {
    let layers = if level == 0 || chance >= ALWAYS { 1 } else { (rng.next_int(100) < chance as i32) as i32 };
    if layers > 0 {
        let new_level = (level + layers).clamp(0, COMPOSTER_MAX);
        engine::set_block(rs, l, pos, seti(st, Pr::Level, new_level), fl::ALL, UPDATE_LIMIT);
        if new_level == COMPOSTER_MAX {
            engine::schedule(rs, &*l, pos, block_of(st), RIPEN_TICKS, prio::NORMAL);
        }
    }
    layers
}

/// `ComposterBlock.useWithoutItem` -> `extractProduce` on a ready composter.
fn composter_use_without_item<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) -> Res {
    if geti(st, Pr::Level) != COMPOSTER_READY {
        return Res::Pass;
    }
    let bone = t().bone_meal;
    // `Vec3.atLowerCornerWithOffset(pos, 0.5, 1.01, 0.5).offsetRandomXZ(random, 0.7F)`
    let dx = (rng.next_float() - 0.5) * 0.7;
    let dz = (rng.next_float() - 0.5) * 0.7;
    let at = [pos.x as f64 + 0.5 + dx as f64, pos.y as f64 + 1.01, pos.z as f64 + 0.5 + dz as f64];
    // `new ItemEntity(...)`: random velocity, then `setDefaultPickUpDelay()`
    let vel = [rng.next_double() * 0.2 - 0.1, 0.2, rng.next_double() * 0.2 - 0.1];
    if let Some(g) = l.game() {
        crate::game::entity::spawn_item(g, at, vel, Stack::new(bone, 1), 10);
    } else {
        l.pop_resource(pos, bone, 1);
    }
    engine::set_block(rs, l, pos, seti(st, Pr::Level, 0), fl::ALL, UPDATE_LIMIT);
    l.sound("block.composter.empty", pos, 1.0, 1.0, CAT_BLOCKS, false);
    Res::swing()
}

// ------------------------------------------------------------------ flower pot

/// `FlowerPotBlock.useItemOn`: a pottable plant into an empty pot.
fn pot_use_item<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, ctx: &UseCtx) -> Res {
    let t = t();
    let potted = t.pot_of_plant.get(ctx.item as usize).copied().unwrap_or(0);
    if potted == 0 {
        return Res::TryEmptyHand;
    }
    if block_of(st) != t.flower_pot {
        // CONSUME: the pot already holds a plant
        return Res::swing();
    }
    engine::set_block(rs, l, pos, potted, fl::ALL, UPDATE_LIMIT);
    shrink_one()
}

/// `FlowerPotBlock.useWithoutItem`: the plant goes back to the player with
/// `player.addItem`, and the pot becomes empty.
fn pot_use_without_item<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) -> Res {
    let t = t();
    let blk = block_of(st);
    if blk == t.flower_pot {
        return Res::swing();
    }
    let plant = t.plant_of_pot.get(blk as usize).copied().unwrap_or(0);
    if plant == 0 {
        return Res::Pass;
    }
    engine::set_block(rs, l, pos, t.flower_pot_state, fl::ALL, UPDATE_LIMIT);
    Res::Done(UseOut { give: Some((plant, 1)), ..UseOut::default() })
}

// ------------------------------------------------------------------ cake

/// `CakeBlock.useItemOn`: a candle onto an uneaten cake.
fn cake_use_item<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, ctx: &UseCtx) -> Res {
    let candle_cake = t().cake_of_candle.get(ctx.item as usize).copied().unwrap_or(0);
    if candle_cake == 0 || geti(st, Pr::Bites) != 0 {
        return Res::TryEmptyHand;
    }
    l.sound("block.cake.add_candle", pos, 1.0, 1.0, CAT_BLOCKS, false);
    engine::set_block(rs, l, pos, candle_cake, fl::ALL, UPDATE_LIMIT);
    shrink_one()
}

/// `CakeBlock.useWithoutItem` -> `CakeBlock.eat`. `cake` is the cake state
/// to bite (a candle cake passes the plain cake, as vanilla does).
fn cake_eat<L: Lvl>(rs: &mut Rs, l: &mut L, cake: u16, pos: BlockPos, ctx: &UseCtx) -> Res {
    if !ctx.can_eat {
        return Res::Pass;
    }
    let bites = geti(cake, Pr::Bites);
    if bites < MAX_BITES {
        engine::set_block(rs, l, pos, seti(cake, Pr::Bites, bites + 1), fl::ALL, UPDATE_LIMIT);
    } else {
        engine::remove_block(rs, l, pos, false);
    }
    Res::Done(UseOut { food: Some((CAKE_NUTRITION, CAKE_SATURATION)), ..UseOut::default() })
}

// ------------------------------------------------------------------ candles

/// `AbstractCandleBlock.extinguish` without particles and game events.
fn extinguish<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    engine::set_block(rs, l, pos, setb(st, Pr::Lit, false), fl::ALL | fl::IMMEDIATE, UPDATE_LIMIT);
    l.sound("block.candle.extinguish", pos, 1.0, 1.0, CAT_BLOCKS, false);
}

/// `CandleBlock.useItemOn`: an empty hand puts out a lit candle (`mayBuild`
/// is assumed; adventure mode is not in `UseCtx`).
fn candle_use_item<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, ctx: &UseCtx) -> Res {
    if ctx.item == 0 && getb(st, Pr::Lit) {
        extinguish(rs, l, st, pos);
        return Res::swing();
    }
    Res::TryEmptyHand
}

/// `CandleCakeBlock.candleHit`: the click is on the candle, above y 0.5.
fn candle_hit(ctx: &UseCtx) -> bool {
    ctx.cursor.1 > 0.5
}

/// `CandleCakeBlock.useItemOn`.
fn candle_cake_use_item<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, ctx: &UseCtx) -> Res {
    let t = t();
    if ctx.item == t.flint || ctx.item == t.fire_charge {
        return Res::Pass;
    }
    if candle_hit(ctx) && ctx.item == 0 && getb(st, Pr::Lit) {
        extinguish(rs, l, st, pos);
        return Res::swing();
    }
    Res::TryEmptyHand
}

/// `CandleCakeBlock.useWithoutItem`: eat a slice (the candle drops when it
/// is eaten, and the cake left behind has one bite).
fn candle_cake_use_without_item<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, ctx: &UseCtx) -> Res {
    let res = cake_eat(rs, l, t().cake_state, pos, ctx);
    if let Res::Done(_) = res {
        engine::drop_resources(l, st, pos);
    }
    res
}

// ------------------------------------------------------------------ respawn anchor

/// `RespawnAnchorBlock.useItemOn` with glowstone. The offhand check of
/// vanilla (PASS when the offhand holds glowstone) is not modelled: `UseCtx`
/// has no offhand item.
fn anchor_use_item<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, ctx: &UseCtx) -> Res {
    let charges = geti(st, Pr::Charges);
    if ctx.item == t().glowstone && charges < 4 {
        // `RespawnAnchorBlock.charge`
        engine::set_block(rs, l, pos, seti(st, Pr::Charges, charges + 1), fl::ALL, UPDATE_LIMIT);
        l.sound("block.respawn_anchor.charge", pos, 1.0, 1.0, CAT_BLOCKS, false);
        return shrink_one();
    }
    Res::TryEmptyHand
}

/// `RespawnAnchorBlock.useWithoutItem`. Rine has only the overworld, where
/// `RESPAWN_ANCHOR_WORKS` is false everywhere, so a charged anchor explodes.
fn anchor_use_without_item<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) -> Res {
    if geti(st, Pr::Charges) == 0 {
        return Res::Pass;
    }
    // `explode`: remove the block first, then the blast at its centre
    engine::remove_block(rs, l, pos, false);
    if let Some(g) = l.game() {
        let c = [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5];
        crate::game::explosion::explode_batch(g, vec![c], ANCHOR_POWER);
    }
    Res::swing()
}

#[cfg(test)]
mod tests {
    use super::super::props::setv;
    use super::super::testw::World as TW;
    use super::*;
    use crate::game::redstone::data::EAST;

    fn s(n: &str) -> u16 {
        rine_data::init();
        super::super::default_state(n)
    }

    fn ctx(item: &str) -> UseCtx {
        rine_data::init();
        UseCtx {
            item: if item.is_empty() { 0 } else { super::super::item_id(item) },
            count: 1,
            damage: 0,
            main_hand: true,
            creative: false,
            sneaking: false,
            face: NORTH,
            cursor: (0.5, 0.5, 0.5),
            yaw: 0.0,
            pos: [0.0; 3],
            can_eat: true,
        }
    }

    fn at() -> BlockPos {
        BlockPos::new(0, 64, 0)
    }

    /// A seed whose first `next_int(100)` is `want`.
    fn seed_drawing(want: i32) -> u64 {
        (1u64..).find(|&k| Rng::new(k).next_int(100) == want).unwrap()
    }

    #[test]
    fn composter_table_is_complete() {
        rine_data::init();
        // one entry per `compostable(...)` in Items.java (122 at 26.3)
        assert_eq!(COMPOST.len(), 122);
        let unknown: Vec<&str> = COMPOST.iter().map(|(n, _)| *n).filter(|n| super::super::item_id(n) == 0).collect();
        assert!(unknown.is_empty(), "unknown items: {unknown:?}");
        assert_eq!(t().compost.iter().filter(|&&c| c > 0).count(), 122);
        assert_eq!(compost_chance(super::super::item_id("cake")), Some(ALWAYS));
        assert_eq!(compost_chance(super::super::item_id("wheat")), Some(MEDIUM));
        assert_eq!(compost_chance(super::super::item_id("stone")), None);
    }

    #[test]
    fn composter_level_zero_always_takes_a_layer_without_a_roll() {
        let p = at();
        for item in ["oak_sapling", "wheat", "cake"] {
            for seed in 1..30u64 {
                let mut w = TW::new();
                w.put(p, s("composter"));
                let mut rs = Rs::default();
                let mut rng = Rng::new(seed);
                let r = use_item_on(&mut rs, &mut w, s("composter"), p, &ctx(item), &mut rng);
                assert_eq!(r, shrink_one());
                assert_eq!(geti(w.get(p), Pr::Level), 1, "{item} seed {seed}");
                assert_eq!(w.events, vec![(COMPOSTER_FILL, p, 1)]);
                // no roll was drawn: the rng is where it started
                assert_eq!(rng.next_int(100), Rng::new(seed).next_int(100));
            }
        }
    }

    #[test]
    fn composter_roll_boundary_is_chance_exactly() {
        // wheat is medium (65): a draw of 64 adds a layer, 65 does not, but
        // the item is used up either way
        let p = at();
        let st = seti(s("composter"), Pr::Level, 3);
        let mut w = TW::new();
        w.put(p, st);
        let mut rs = Rs::default();
        let mut rng = Rng::new(seed_drawing(64));
        assert_eq!(use_item_on(&mut rs, &mut w, st, p, &ctx("wheat"), &mut rng), shrink_one());
        assert_eq!(geti(w.get(p), Pr::Level), 4);
        assert_eq!(w.events, vec![(COMPOSTER_FILL, p, 1)]);

        let mut w = TW::new();
        w.put(p, st);
        let mut rng = Rng::new(seed_drawing(65));
        assert_eq!(use_item_on(&mut rs, &mut w, st, p, &ctx("wheat"), &mut rng), shrink_one());
        assert_eq!(w.get(p), st, "a failed roll changes nothing");
        assert_eq!(w.events, vec![(COMPOSTER_FILL, p, 0)]);
    }

    #[test]
    fn composter_level_seven_is_full_and_ripens_to_eight() {
        let p = at();
        let st = seti(s("composter"), Pr::Level, 6);
        let mut w = TW::new();
        w.put(p, st);
        let mut rs = Rs::default();
        let mut rng = Rng::new(7);
        // cake is always one layer: 6 -> 7 schedules the ripening
        assert_eq!(use_item_on(&mut rs, &mut w, st, p, &ctx("cake"), &mut rng), shrink_one());
        assert_eq!(geti(w.get(p), Pr::Level), 7);
        assert!(engine::has_scheduled(&rs, p, block_of(st)));
        for _ in 0..19 {
            w.tick(&mut rs);
        }
        assert_eq!(geti(w.get(p), Pr::Level), 7, "not ripe after 19 ticks");
        w.tick(&mut rs);
        assert_eq!(geti(w.get(p), Pr::Level), 8);
        assert!(w.sounds.iter().any(|(n, _)| n == "block.composter.ready"));

        // a full composter takes nothing more and is not used up
        let full = seti(s("composter"), Pr::Level, 7);
        let mut w = TW::new();
        w.put(p, full);
        let r = use_item_on(&mut rs, &mut w, full, p, &ctx("wheat"), &mut rng);
        assert_eq!(r, Res::swing());
        assert_eq!(w.get(p), full);
        assert!(w.events.is_empty());
    }

    #[test]
    fn composter_ready_pops_bone_meal_and_empties() {
        let p = at();
        let st = seti(s("composter"), Pr::Level, 8);
        let mut w = TW::new();
        w.put(p, st);
        let mut rs = Rs::default();
        let mut rng = Rng::new(3);
        // a ready composter takes no compostable item: the block use decides
        assert_eq!(use_item_on(&mut rs, &mut w, st, p, &ctx("wheat"), &mut rng), Res::TryEmptyHand);
        assert_eq!(use_without_item(&mut rs, &mut w, st, p, &ctx(""), &mut rng), Res::swing());
        assert_eq!(geti(w.get(p), Pr::Level), 0);
        assert_eq!(w.popped.len(), 1);
        assert_eq!(w.popped[0].1, super::super::item_id("bone_meal"));
        assert!(w.sounds.iter().any(|(n, _)| n == "block.composter.empty"));
        // an empty one does nothing
        let empty = w.get(p);
        assert_eq!(use_without_item(&mut rs, &mut w, empty, p, &ctx(""), &mut rng), Res::Pass);
    }

    #[test]
    fn composter_on_place_schedules_only_when_full() {
        let p = at();
        let mut w = TW::new();
        let mut rs = Rs::default();
        on_place(&mut rs, &w, s("composter"), p);
        assert!(!engine::has_scheduled(&rs, p, block_of(s("composter"))));
        let full = seti(s("composter"), Pr::Level, 7);
        on_place(&mut rs, &w, full, p);
        assert!(engine::has_scheduled(&rs, p, block_of(full)));
        w.put(p, full);
        tick(&mut rs, &mut w, full, p);
        assert_eq!(geti(w.get(p), Pr::Level), 8);
    }

    #[test]
    fn flower_pot_takes_a_plant_and_gives_it_back() {
        let p = at();
        let mut w = TW::new();
        w.put(p, s("flower_pot"));
        let mut rs = Rs::default();
        let mut rng = Rng::new(1);
        assert_eq!(use_item_on(&mut rs, &mut w, s("flower_pot"), p, &ctx("dandelion"), &mut rng), shrink_one());
        assert_eq!(w.get(p), s("potted_dandelion"));
        let potted = w.get(p);
        // a second plant is refused: consume without change
        assert_eq!(use_item_on(&mut rs, &mut w, potted, p, &ctx("poppy"), &mut rng), Res::swing());
        assert_eq!(w.get(p), potted);
        // a stick is not pottable: the pot gives the plant back with addItem
        assert_eq!(use_item_on(&mut rs, &mut w, potted, p, &ctx("stick"), &mut rng), Res::TryEmptyHand);
        let r = use_without_item(&mut rs, &mut w, potted, p, &ctx("stick"), &mut rng);
        assert_eq!(r, Res::Done(UseOut { give: Some((super::super::item_id("dandelion"), 1)), ..UseOut::default() }));
        assert_eq!(w.get(p), s("flower_pot"));
        // an empty pot with a block in hand: CONSUME, the block is not placed
        assert_eq!(use_without_item(&mut rs, &mut w, s("flower_pot"), p, &ctx("stone"), &mut rng), Res::swing());
        assert_eq!(w.get(p), s("flower_pot"));
    }

    #[test]
    fn flower_pot_round_trips_every_potted_plant() {
        rine_data::init();
        let p = at();
        let mut rs = Rs::default();
        let mut rng = Rng::new(2);
        let potted_count = rine_data::get().blocks.blocks.iter().filter(|b| b.name.strip_prefix("minecraft:").unwrap_or(&b.name).starts_with("potted_")).count();
        assert_eq!(potted_count, POTS.len());
        for &(plant, potted) in POTS {
            let mut w = TW::new();
            w.put(p, s("flower_pot"));
            assert_eq!(use_item_on(&mut rs, &mut w, s("flower_pot"), p, &ctx(plant), &mut rng), shrink_one(), "{plant}");
            assert_eq!(w.get(p), s(potted), "{plant}");
            let r = use_without_item(&mut rs, &mut w, s(potted), p, &ctx(""), &mut rng);
            assert_eq!(r, Res::Done(UseOut { give: Some((super::super::item_id(plant), 1)), ..UseOut::default() }), "{plant}");
        }
    }

    #[test]
    fn cake_slices_and_full_bar() {
        let p = at();
        let mut rs = Rs::default();
        let mut rng = Rng::new(4);
        let mut w = TW::new();
        w.put(p, s("cake"));
        let mut c = ctx("");
        c.can_eat = false;
        // full food bar in survival: PASS, nothing eaten
        assert_eq!(use_without_item(&mut rs, &mut w, s("cake"), p, &c, &mut rng), Res::Pass);
        assert_eq!(w.get(p), s("cake"));
        // hungry: one slice, two food points
        let r = use_without_item(&mut rs, &mut w, s("cake"), p, &ctx(""), &mut rng);
        assert_eq!(r, Res::Done(UseOut { food: Some((2, 0.1)), ..UseOut::default() }));
        assert_eq!(geti(w.get(p), Pr::Bites), 1);
        // the seventh slice removes the cake
        let last = seti(s("cake"), Pr::Bites, 6);
        w.put(p, last);
        assert_eq!(use_without_item(&mut rs, &mut w, last, p, &ctx(""), &mut rng), Res::Done(UseOut { food: Some((2, 0.1)), ..UseOut::default() }));
        assert_eq!(w.get(p), 0);
        // creative counts as able to eat even with a full bar
        let mut w = TW::new();
        w.put(p, s("cake"));
        let mut cr = ctx("");
        cr.creative = true;
        assert!(matches!(use_without_item(&mut rs, &mut w, s("cake"), p, &cr, &mut rng), Res::Done(_)));
    }

    #[test]
    fn candle_on_uneaten_cake_only() {
        let p = at();
        let mut rs = Rs::default();
        let mut rng = Rng::new(5);
        let mut w = TW::new();
        w.put(p, s("cake"));
        assert_eq!(use_item_on(&mut rs, &mut w, s("cake"), p, &ctx("candle"), &mut rng), shrink_one());
        assert_eq!(w.get(p), s("candle_cake"));
        assert!(w.sounds.iter().any(|(n, _)| n == "block.cake.add_candle"));
        // a bitten cake takes no candle: the candle branch falls to eating
        let bitten = seti(s("cake"), Pr::Bites, 2);
        w.put(p, bitten);
        assert_eq!(use_item_on(&mut rs, &mut w, bitten, p, &ctx("candle"), &mut rng), Res::TryEmptyHand);
    }

    #[test]
    fn eating_a_candle_cake_drops_the_candle() {
        let p = at();
        let mut rs = Rs::default();
        let mut rng = Rng::new(6);
        let mut w = TW::new();
        w.put(p, s("candle_cake"));
        let mut c = ctx("");
        c.cursor = (0.5, 0.2, 0.5);
        let r = use_without_item(&mut rs, &mut w, s("candle_cake"), p, &c, &mut rng);
        assert_eq!(r, Res::Done(UseOut { food: Some((2, 0.1)), ..UseOut::default() }));
        assert_eq!(w.get(p), seti(s("cake"), Pr::Bites, 1));
        assert_eq!(w.dropped, vec![(p, s("candle_cake"))]);
        // a full bar: PASS, no drop, no change
        let mut w = TW::new();
        w.put(p, s("candle_cake"));
        c.can_eat = false;
        assert_eq!(use_without_item(&mut rs, &mut w, s("candle_cake"), p, &c, &mut rng), Res::Pass);
        assert_eq!(w.get(p), s("candle_cake"));
        assert!(w.dropped.is_empty());
    }

    #[test]
    fn lit_candles_go_out_with_an_empty_hand() {
        let p = at();
        let mut rs = Rs::default();
        let mut rng = Rng::new(8);
        let lit = setb(s("candle"), Pr::Lit, true);
        let mut w = TW::new();
        w.put(p, lit);
        assert_eq!(use_item_on(&mut rs, &mut w, lit, p, &ctx(""), &mut rng), Res::swing());
        let lit_now = w.get(p);
        assert_eq!(lit_now, setb(s("candle"), Pr::Lit, false));
        assert_eq!(getb(w.get(p), Pr::Lit), false);
        assert!(w.sounds.iter().any(|(n, _)| n == "block.candle.extinguish"));
        // holding something: no extinguish
        assert_eq!(use_item_on(&mut rs, &mut w, lit, p, &ctx("stick"), &mut rng), Res::TryEmptyHand);
        // unlit: nothing either
        assert_eq!(use_item_on(&mut rs, &mut w, s("candle"), p, &ctx(""), &mut rng), Res::TryEmptyHand);
    }

    #[test]
    fn candle_cake_extinguish_needs_a_click_on_the_candle() {
        let p = at();
        let mut rs = Rs::default();
        let mut rng = Rng::new(9);
        let lit = setb(s("candle_cake"), Pr::Lit, true);
        let mut w = TW::new();
        w.put(p, lit);
        let mut c = ctx("");
        c.cursor = (0.5, 0.2, 0.5);
        assert_eq!(use_item_on(&mut rs, &mut w, lit, p, &c, &mut rng), Res::TryEmptyHand);
        c.cursor = (0.5, 0.9, 0.5);
        assert_eq!(use_item_on(&mut rs, &mut w, lit, p, &c, &mut rng), Res::swing());
        assert_eq!(w.get(p), s("candle_cake"));
    }

    #[test]
    fn flint_and_steel_and_fire_charge_light_candles() {
        let p = at();
        let mut rs = Rs::default();
        let mut rng = Rng::new(10);
        let mut w = TW::new();
        w.put(p, s("candle"));
        let r = super::super::interact::item_use_on(&mut rs, &mut w, s("candle"), p, &ctx("flint_and_steel"), &mut rng);
        assert_eq!(r, Res::Done(UseOut { item: ItemOp::Damage(1), ..UseOut::default() }));
        assert!(getb(w.get(p), Pr::Lit));
        // waterlogged candles do not light
        let wet = setb(s("candle"), Pr::Waterlogged, true);
        w.put(p, wet);
        // not lightable: FlintAndSteelItem tries fire next to it, here FAIL
        assert!(!matches!(super::super::interact::item_use_on(&mut rs, &mut w, wet, p, &ctx("flint_and_steel"), &mut rng), Res::Done(_)));
        assert_eq!(w.get(p), wet);
        // fire charge on a candle cake
        w.put(p, s("candle_cake"));
        assert_eq!(super::super::interact::item_use_on(&mut rs, &mut w, s("candle_cake"), p, &ctx("fire_charge"), &mut rng), Res::Done(UseOut { item: ItemOp::Shrink(1), ..UseOut::default() }));
        assert!(getb(w.get(p), Pr::Lit));
        // a lit cake: Pass
        let now = w.get(p);
        assert!(!matches!(super::super::interact::item_use_on(&mut rs, &mut w, now, p, &ctx("fire_charge"), &mut rng), Res::Done(_)));
        assert_eq!(w.get(p), now);
    }

    #[test]
    fn bell_rings_only_from_the_correct_side() {
        let p = at();
        let mut rs = Rs::default();
        let mut rng = Rng::new(11);
        // floor bell facing north: rings from north and south, not east or west
        let floor = data::set(s("bell"), data::P::Facing, NORTH);
        let floor = setv(floor, Pr::Attachment, vid("floor"));
        let mut w = TW::new();
        w.put(p, floor);
        let mut c = ctx("");
        c.face = crate::game::redstone::data::EAST;
        assert_eq!(use_without_item(&mut rs, &mut w, floor, p, &c, &mut rng), Res::Pass);
        assert!(w.sounds.is_empty());
        c.face = NORTH;
        assert_eq!(use_without_item(&mut rs, &mut w, floor, p, &c, &mut rng), Res::swing());
        assert_eq!(w.sounds, vec![("block.bell.use".to_string(), p)]);
        // the top part of the bell (y above 0.8124) does not ring from the side
        w.sounds.clear();
        c.cursor = (0.5, 0.9, 0.5);
        assert_eq!(use_without_item(&mut rs, &mut w, floor, p, &c, &mut rng), Res::Pass);
        // a click on the top face never rings a floor bell
        c.face = UP;
        c.cursor = (0.5, 1.0, 0.5);
        assert_eq!(use_without_item(&mut rs, &mut w, floor, p, &c, &mut rng), Res::Pass);
        // a ceiling bell rings from any side
        let ceil = setv(floor, Pr::Attachment, vid("ceiling"));
        w.put(p, ceil);
        c.face = EAST;
        c.cursor = (0.5, 0.5, 0.5);
        assert_eq!(use_without_item(&mut rs, &mut w, ceil, p, &c, &mut rng), Res::swing());
        // a single wall bell facing north rings from east and west only
        let wall = setv(floor, Pr::Attachment, vid("single_wall"));
        w.put(p, wall);
        c.face = NORTH;
        assert_eq!(use_without_item(&mut rs, &mut w, wall, p, &c, &mut rng), Res::Pass);
        c.face = EAST;
        assert_eq!(use_without_item(&mut rs, &mut w, wall, p, &c, &mut rng), Res::swing());
    }

    #[test]
    fn powered_bell_rings_once_on_the_rising_edge() {
        let p = at();
        let mut rs = Rs::default();
        let floor = setv(data::set(s("bell"), data::P::Facing, NORTH), Pr::Attachment, vid("floor"));
        let mut w = TW::new();
        w.put(p, floor);
        // no signal: nothing
        neighbor_changed(&mut rs, &mut w, floor, p);
        assert!(w.sounds.is_empty());
        // a redstone block above powers it: one ring, and the state is powered
        w.put(p.offset(0, 1, 0), s("redstone_block"));
        neighbor_changed(&mut rs, &mut w, floor, p);
        assert_eq!(w.sounds, vec![("block.bell.use".to_string(), p)]);
        assert!(getb(w.get(p), Pr::Powered));
        // still powered: no second ring
        let powered = w.get(p);
        w.sounds.clear();
        neighbor_changed(&mut rs, &mut w, powered, p);
        assert!(w.sounds.is_empty());
    }

    #[test]
    fn respawn_anchor_charges_to_four_then_explodes() {
        let p = at();
        let mut rs = Rs::default();
        let mut rng = Rng::new(12);
        let mut w = TW::new();
        w.put(p, s("respawn_anchor"));
        // empty hand on an uncharged anchor: nothing
        assert_eq!(use_without_item(&mut rs, &mut w, s("respawn_anchor"), p, &ctx(""), &mut rng), Res::Pass);
        let mut st = s("respawn_anchor");
        for charges in 0..4 {
            assert_eq!(geti(st, Pr::Charges), charges);
            w.put(p, st);
            assert_eq!(use_item_on(&mut rs, &mut w, st, p, &ctx("glowstone"), &mut rng), shrink_one());
            st = w.get(p);
        }
        assert_eq!(geti(st, Pr::Charges), 4);
        // full: glowstone falls to the empty-hand step, which blows it up
        assert_eq!(use_item_on(&mut rs, &mut w, st, p, &ctx("glowstone"), &mut rng), Res::TryEmptyHand);
        assert_eq!(use_without_item(&mut rs, &mut w, st, p, &ctx(""), &mut rng), Res::swing());
        // testw has no game, so only the removal happens here (the blast is not run)
        assert_eq!(w.get(p), 0);
    }

    #[test]
    fn creative_does_not_change_the_block_effects() {
        // the caller applies creative (no consume); the block answer is the same
        let p = at();
        let mut rs = Rs::default();
        let mut rng = Rng::new(13);
        let mut w = TW::new();
        w.put(p, s("flower_pot"));
        let mut c = ctx("dandelion");
        c.creative = true;
        assert_eq!(use_item_on(&mut rs, &mut w, s("flower_pot"), p, &c, &mut rng), shrink_one());
        assert_eq!(w.get(p), s("potted_dandelion"));
    }
}
