//! Vanilla block behaviour outside redstone: random ticks, shape updates,
//! scheduled ticks and placement of ordinary blocks (crops, grass, leaves,
//! ice, copper, stairs, fences, fire...).
//!
//! Layout: one file per family. Each family exposes `classify(name)` that
//! claims its blocks with a `Beh` value, and the hooks it implements
//! (`random_tick`, `update_shape`, `tick`, `on_place`, `neighbor_changed`).
//! The dispatchers below route by a per-block table built once at startup,
//! so no strings are compared at run time.
//!
//! Everything here works on the redstone engine's `Lvl` trait, so it runs
//! inside the engine with vanilla neighbor-update order and is unit-tested
//! on the in-memory world of `redstone/tests.rs`.
//!
//! Hooks from the engine (`redstone/engine.rs`):
//! - `update_shape` from `BlockState.updateShape` (after the redstone kinds);
//! - `tick` from scheduled block ticks of blocks the engine does not know;
//! - `on_place` / `neighbor_changed` likewise.
//! Random ticks come from `random::tick`, once per game tick.

pub mod crops;
pub mod props;
pub mod random;
pub mod rng;
pub mod shape;
#[cfg(test)]
pub mod testw;

use super::redstone::engine::Lvl;
use super::redstone::Rs;
use rine_protocol::BlockPos;
pub use rng::Rng;
use std::sync::OnceLock;

/// Block behaviour state kept on the game.
pub struct State {
    pub random: random::State,
    /// Base seed of the random streams (clock-seeded, as vanilla's level random).
    pub seed: u64,
}

impl Default for State {
    fn default() -> Self {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
        Self { random: Default::default(), seed: t ^ 0x5DEE_CE66_D1CE_4E5B }
    }
}

/// What a block does. One value per block id.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Beh {
    None,
    /// `CropBlock` and subclasses (wheat, carrots, potatoes, beetroots, torchflower crop).
    Crop,
    /// `StemBlock` (pumpkin, melon).
    Stem,
    /// `NetherWartBlock`.
    NetherWart,
    /// `CocoaBlock`.
    Cocoa,
    /// `SweetBerryBushBlock`.
    SweetBerry,
    /// `PitcherCropBlock`.
    PitcherCrop,
    /// `StairBlock` (shape at placement and on neighbour change).
    Stairs,
    /// `FenceBlock` (north/east/south/west connections).
    Fence,
    /// `IronBarsBlock`: iron bars, copper bars and panes.
    Bars,
    /// `WallBlock` (arms and post).
    Wall,
    /// `FenceGateBlock` (`in_wall`).
    FenceGate,
    /// `DoorBlock` (hinge at placement only).
    Door,
    /// `ChorusPlantBlock` (connections, survival tick).
    Chorus,
    /// `HugeMushroomBlock` (side connections).
    HugeMushroom,
}

struct Table {
    beh: Vec<Beh>,
}

fn table() -> &'static Table {
    static T: OnceLock<Table> = OnceLock::new();
    T.get_or_init(|| {
        let b = &rine_data::get().blocks;
        let beh = b
            .blocks
            .iter()
            .map(|blk| {
                let n = blk.name.strip_prefix("minecraft:").unwrap_or(&blk.name);
                classify(n)
            })
            .collect();
        Table { beh }
    })
}

/// Claim a block for a family. Families are asked in order.
fn classify(n: &str) -> Beh {
    if let Some(b) = crops::classify(n) {
        return b;
    }
    if let Some(b) = shape::classify(n) {
        return b;
    }
    Beh::None
}

/// Behaviour of the block of a state.
#[inline]
pub fn beh(st: u16) -> Beh {
    let blk = rine_data::get().blocks.states[st as usize].block as usize;
    table().beh[blk]
}

/// Block id by name, for families' init-time tables ("wheat").
pub fn block_id(name: &str) -> u16 {
    rine_data::get().blocks.block(name).map(|b| b.id as u16).unwrap_or(u16::MAX)
}

/// Default state by name ("wheat").
pub fn default_state(name: &str) -> u16 {
    rine_data::get().blocks.default_state(name)
}

/// Membership table of a block tag, indexed by block id ("supports_crops").
pub fn block_tag(tag: &str) -> Vec<bool> {
    let d = rine_data::get();
    let mut v = vec![false; d.blocks.blocks.len()];
    if let Some(list) = d.registries.tag("minecraft:block", tag) {
        for e in list {
            if let Some(b) = d.blocks.block(e) {
                v[b.id as usize] = true;
            }
        }
    }
    v
}

/// Item id by name ("wheat_seeds"); 0 if unknown.
pub fn item_id(name: &str) -> u32 {
    let n = if name.contains(':') { name.to_string() } else { format!("minecraft:{name}") };
    rine_data::get().registries.id("minecraft:item", &n).unwrap_or(0)
}

#[inline]
pub fn is_air(st: u16) -> bool {
    rine_data::get().blocks.is_air(st)
}

/// Block id of a state.
#[inline]
pub fn block_of(st: u16) -> u16 {
    rine_data::get().blocks.states[st as usize].block
}

/// `Level.getRawBrightness(pos, 0)`: max of block light and raw sky light.
#[inline]
pub fn raw_brightness<L: Lvl>(l: &L, p: BlockPos) -> u8 {
    l.block_light(p).max(l.sky_light(p))
}

/// `Level.getMaxLocalRawBrightness(pos)`: sky light minus the current sky
/// darkening, against block light.
#[inline]
pub fn local_brightness<L: Lvl>(l: &L, p: BlockPos) -> u8 {
    l.block_light(p).max(l.sky_light(p).saturating_sub(l.sky_darken()))
}

/// Game rule value by index, falling back to `default` when the level has no rules.
pub fn rule_int<L: Lvl>(l: &L, idx: &OnceLock<Option<usize>>, name: &str, default: i32) -> i32 {
    let i = *idx.get_or_init(|| super::cmd::rules::GameRules::index(name));
    match (l.rules(), i) {
        (Some(r), Some(i)) => match r.get(i) {
            super::cmd::rules::Value::Int(v) => v,
            super::cmd::rules::Value::Bool(b) => b as i32,
        },
        _ => default,
    }
}

pub fn rule_bool<L: Lvl>(l: &L, idx: &OnceLock<Option<usize>>, name: &str, default: bool) -> bool {
    rule_int(l, idx, name, default as i32) != 0
}

/// What a player's placement looked like (`BlockPlaceContext`).
#[derive(Clone, Copy, Debug)]
pub struct PlaceCtx {
    /// Clicked face of the clicked block (protocol direction id).
    pub face: u8,
    /// Cursor position on the clicked face, 0..1.
    pub cursor: (f32, f32, f32),
    pub yaw: f32,
    pub pitch: f32,
    pub sneaking: bool,
    /// The clicked block (the placed block is there or next to it).
    pub clicked: BlockPos,
}

// ------------------------------------------------------------------ dispatch

/// `Block.getStateForPlacement` corrections for this family's blocks, run
/// after the generic placement in `place.rs` and the redstone engine's.
/// Read-only. None means the block cannot be placed.
pub fn placement<L: Lvl>(l: &L, st: u16, pos: BlockPos, ctx: &PlaceCtx) -> Option<u16> {
    match beh(st) {
        Beh::Stairs | Beh::Fence | Beh::Bars | Beh::Wall | Beh::FenceGate | Beh::Door | Beh::Chorus | Beh::HugeMushroom => shape::placement(l, st, pos, ctx),
        _ => Some(st),
    }
}

/// `BlockState.randomTick`. Only called for states with the vanilla
/// `isRandomlyTicking` flag.
pub fn random_tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    match beh(st) {
        Beh::None | Beh::Stairs | Beh::Fence | Beh::Bars | Beh::Wall | Beh::FenceGate | Beh::Door | Beh::Chorus | Beh::HugeMushroom => {}
        Beh::Crop | Beh::Stem | Beh::NetherWart | Beh::Cocoa | Beh::SweetBerry | Beh::PitcherCrop => crops::random_tick(rs, l, st, pos, rng),
    }
}

/// `FluidState.randomTick` (only lava ticks randomly): fire from lava.
pub fn fluid_random_tick<L: Lvl>(_rs: &mut Rs, _l: &mut L, _st: u16, _pos: BlockPos, _rng: &mut Rng) {}

/// `ServerLevel.tickPrecipitation` at column (x, z): ice, snow layers,
/// `handlePrecipitation` of the block below the top.
pub fn precipitation<L: Lvl>(_rs: &mut Rs, _l: &mut L, _x: i32, _z: i32, _rng: &mut Rng) {}

/// `BlockState.updateShape` for blocks the redstone engine does not handle.
/// `dir` points from `pos` to the changed neighbor `npos`, now `nst`.
pub fn update_shape<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, dir: u8, npos: BlockPos, nst: u16) -> u16 {
    match beh(st) {
        Beh::None | Beh::Door => st,
        Beh::Crop | Beh::Stem | Beh::NetherWart | Beh::Cocoa | Beh::SweetBerry | Beh::PitcherCrop => crops::update_shape(rs, l, st, pos, dir, npos, nst),
        Beh::Stairs | Beh::Fence | Beh::Bars | Beh::Wall | Beh::FenceGate | Beh::Chorus | Beh::HugeMushroom => shape::update_shape(rs, l, st, pos, dir, npos, nst),
    }
}

/// A scheduled tick came due for a block the engine does not know.
pub fn tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    match beh(st) {
        Beh::Chorus => shape::tick(rs, l, st, pos),
        _ => {}
    }
}

/// `BlockState.onPlace` for blocks the engine does not know.
pub fn on_place<L: Lvl>(_rs: &mut Rs, _l: &mut L, st: u16, _pos: BlockPos, _old: u16) {
    match beh(st) {
        _ => {}
    }
}

/// `BlockState.handleNeighborChanged` for blocks the engine does not know.
pub fn neighbor_changed<L: Lvl>(_rs: &mut Rs, _l: &mut L, st: u16, _pos: BlockPos, _from: u16) {
    match beh(st) {
        _ => {}
    }
}
