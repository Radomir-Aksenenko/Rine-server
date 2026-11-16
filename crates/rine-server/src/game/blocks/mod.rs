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

pub mod bed;
pub mod cauldron;
pub mod copper;
pub mod crops;
pub mod falling;
pub mod grass;
pub mod interact;
pub mod leaves;
pub mod misc;
pub mod near;
pub mod fire;
pub mod plants;
pub mod props;
pub mod random;
pub mod rng;
pub mod shape;
pub mod snow;
pub mod tools;
pub mod uses;
pub mod sponge;
pub mod sapling;
pub mod bonemeal;
pub mod growth;
#[cfg(test)]
pub mod testw;

#[cfg(test)]
mod tests {
    #[test]
    fn sky_darken_follows_day_and_weather() {
        use super::sky_darken;
        assert_eq!(sky_darken(6000, 0.0, 0.0), 0);
        // night: 15 * 0.26666668 = 4 -> darken 11
        assert_eq!(sky_darken(18000, 0.0, 0.0), 11);
        // midway down the dusk ramp
        assert_eq!(sky_darken(12768, 0.0, 0.0), 5);
        // dawn wraps over the period
        assert_eq!(sky_darken(24000 + 133, 0.0, 0.0), 0);
        assert_eq!(sky_darken(-1670, 0.0, 0.0), 11);
        // full rain at noon: 15 -> lerp(0.3125, 15, 4) = 11.5625 -> darken 3
        assert_eq!(sky_darken(6000, 1.0, 0.0), 3);
        // thunder: 15 -> lerp(0.52734375, 15, 4) = 9.199 -> darken 5
        assert_eq!(sky_darken(6000, 1.0, 1.0), 5);
    }
}

use super::redstone::engine::Lvl;
use super::redstone::Rs;
use rine_protocol::BlockPos;
pub use rng::Rng;
use std::sync::OnceLock;

/// Block behaviour state kept on the game.
pub struct State {
    pub random: random::State,
    /// Players by grid cell for fire's player-distance check.
    pub near: near::Grid,
    /// Base seed of the random streams (clock-seeded, as vanilla's level random).
    pub seed: u64,
}

impl Default for State {
    fn default() -> Self {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
        Self { random: Default::default(), near: Default::default(), seed: t ^ 0x5DEE_CE66_D1CE_4E5B }
    }
}

/// What a block does. One value per block id.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Beh {
    None,
    /// `RedStoneOreBlock` (lit ore, random tick off).
    RedstoneOre,
    /// `FarmlandBlock` (moisture, trampling, shape).
    Farmland,
    /// `TurtleEggBlock` (hatch on sand at night).
    TurtleEgg,
    /// `SnifferEggBlock` (scheduled hatch stages).
    SnifferEgg,
    /// `DriedGhastBlock` (hydration, scheduled).
    DriedGhast,
    /// `ChorusFlowerBlock` (growth, survival).
    ChorusFlower,
    /// `ChorusPlantBlock` as claimed by misc (shape.rs claims it first: `Chorus`).
    ChorusPlant,
    /// `SpeleothemBlock` (pointed dripstone, sulfur spike).
    Speleothem,
    /// `MangrovePropaguleBlock` (hanging stage).
    Propagule,
    /// `SporeBlossomBlock` (ceiling support).
    SporeBlossom,
    /// Cauldrons: bucket interactions (`cauldron.rs`), stalactite drips
    /// (`AbstractCauldronBlock.tick`) and rain (`snow.rs`).
    Cauldron,
    /// `SpongeBlock` (absorbs water on place and neighbour change).
    Sponge,
    /// `WetSpongeBlock` (no effect in the overworld).
    WetSponge,
    /// `FallingBlock` subclasses without their own rules: sand, red sand,
    /// gravel, anvils.
    Falling,
    /// `ConcretePowderBlock` (turns to concrete next to water).
    ConcretePowder,
    /// `DragonEggBlock` (falls, teleports on use).
    DragonEgg,
    /// `BrushableBlock` (suspicious sand and gravel: falls, no brushing here).
    Brushable,
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
    /// `SpreadingSnowyBlock` (grass_block, mycelium).
    Spreading,
    /// `SnowyBlock` without spreading (podzol): `snowy` only.
    Snowy,
    /// `NyliumBlock` (crimson, warped).
    Nylium,
    /// `ChangeOverTimeBlock` copper classes (`WeatheringCopper`).
    Copper,
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
    /// `LeavesBlock` and subclasses (distance, decay).
    Leaves,
    /// `FireBlock`.
    Fire,
    /// `SoulFireBlock`.
    SoulFire,
    /// `IceBlock` (melts in light).
    Ice,
    /// `FrostedIceBlock` (ages, then melts).
    FrostedIce,
    /// `SnowLayerBlock`.
    Snow,
    /// `CactusBlock`.
    Cactus,
    /// `SugarCaneBlock`.
    SugarCane,
    /// `BambooStalkBlock`.
    Bamboo,
    /// `BambooSaplingBlock`.
    BambooSapling,
    /// `GrowingPlantHeadBlock` (kelp, weeping, twisting, cave vines).
    GrowHead,
    /// `GrowingPlantBodyBlock` (the body blocks of the same plants).
    GrowBody,
    /// `MushroomBlock` (spread).
    Mushroom,
    /// `BuddingAmethystBlock`.
    BuddingAmethyst,
    /// `AmethystClusterBlock` (cluster and buds).
    AmethystCluster,
    /// `EyeblossomBlock`.
    Eyeblossom,
    /// `VineBlock`.
    Vine,
    /// `BellBlock` (ring on use and on a rising redstone edge).
    Bell,
    /// `ComposterBlock` (layers, ripening, bone meal).
    Composter,
    /// `FlowerPotBlock` and the potted plants.
    FlowerPot,
    /// `CakeBlock`.
    Cake,
    /// `CandleCakeBlock`.
    CandleCake,
    /// `CandleBlock`.
    Candle,
    /// `RespawnAnchorBlock`.
    RespawnAnchor,
    /// `SaplingBlock` (stage, random tick growth, bone meal).
    Sapling,
    /// `BedBlock` and `AbstractBedBlock` of the 16 colours (`bed.rs`).
    Bed,
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
    if let Some(b) = cauldron::classify(n) {
        return b;
    }
    if let Some(b) = sponge::classify(n) {
        return b;
    }
    if let Some(b) = falling::classify(n) {
        return b;
    }
    if let Some(b) = grass::classify(n) {
        return b;
    }
    if let Some(b) = copper::classify(n) {
        return b;
    }
    if let Some(b) = shape::classify(n) {
        return b;
    }
    if let Some(b) = leaves::classify(n) {
        return b;
    }
    if let Some(b) = fire::classify(n) {
        return b;
    }
    if let Some(b) = snow::classify(n) {
        return b;
    }
    if let Some(b) = plants::classify(n) {
        return b;
    }
    if let Some(b) = sapling::classify(n) {
        return b;
    }
    if let Some(b) = misc::classify(n) {
        return b;
    }
    if let Some(b) = bed::classify(n) {
        return b;
    }
    if let Some(b) = uses::classify(n) {
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

/// `Block.isFaceFull(state.getCollisionShape(..), Direction.UP)`: the top
/// face of the collision shape covers the whole block. Precomputed per
/// collision shape; exact for boxes on the 1/16 grid.
#[inline]
pub fn collision_top_full(st: u16) -> bool {
    static T: OnceLock<Vec<bool>> = OnceLock::new();
    let t = T.get_or_init(|| {
        rine_data::get()
            .blocks
            .shapes
            .iter()
            .map(|boxes| {
                (0..16).all(|i| {
                    (0..16).all(|j| {
                        let (x, z) = ((i as f32 + 0.5) / 16.0, (j as f32 + 0.5) / 16.0);
                        boxes.iter().any(|b| b[4] >= 1.0 - 1e-6 && b[0] <= x && x <= b[3] && b[2] <= z && z <= b[5])
                    })
                })
            })
            .collect()
    });
    t[rine_data::get().blocks.state(st).shape as usize]
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

/// `Level.skyDarken` in the overworld: `15 - SKY_LIGHT_LEVEL`, where the
/// attribute is 15 multiplied by the `minecraft:day` timeline track
/// `gameplay/sky_light_level` (linear keyframes, period 24000 on the
/// overworld clock), then the weather layer (`WeatherAttributes`: alpha
/// blend toward 4.0 with alpha 0.3125 for rain, 0.52734375 for thunder,
/// each lerped by its level). `thunder` is the raw thunder level; vanilla
/// `getThunderLevel` multiplies it by the rain level.
pub fn sky_darken(clock: i64, rain: f32, thunder_raw: f32) -> u8 {
    // (ticks, value) keyframes of timeline/day.json
    const K: [(i64, f32); 4] = [(133, 1.0), (11867, 1.0), (13670, 0.266_666_68), (22330, 0.266_666_68)];
    let t = clock.rem_euclid(24000);
    // baked segments: wrap-in, inner, wrap-out (`KeyframeTrackSampler`)
    let segs = [(K[3].0 - 24000, K[3].1, K[0].0, K[0].1), (K[0].0, K[0].1, K[1].0, K[1].1), (K[1].0, K[1].1, K[2].0, K[2].1), (K[2].0, K[2].1, K[3].0, K[3].1), (K[3].0, K[3].1, K[0].0 + 24000, K[0].1)];
    let seg = segs.iter().find(|s| t < s.2).copied().unwrap_or(segs[4]);
    let f = if t <= seg.0 {
        seg.1
    } else if t >= seg.2 {
        seg.3
    } else {
        let a = (t - seg.0) as f32 / (seg.2 - seg.0) as f32;
        seg.1 + a * (seg.3 - seg.1)
    };
    let lerp = |d: f32, a: f32, b: f32| a + d * (b - a);
    let mut v = 15.0f32 * f;
    let thunder = thunder_raw * rain;
    let rain_only = rain - thunder;
    if rain_only > 0.0 {
        v = lerp(rain_only, v, lerp(0.3125, v, 4.0));
    }
    if thunder > 0.0 {
        v = lerp(thunder, v, lerp(0.527_343_75, v, 4.0));
    }
    (15.0 - v.clamp(0.0, 15.0)) as u8
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
        Beh::Leaves => Some(leaves::placement(l, st, pos)),
        Beh::ChorusPlant => Some(misc::chorus_placement(l, st, pos)),
        Beh::Bamboo | Beh::GrowHead | Beh::GrowBody | Beh::Vine | Beh::AmethystCluster => plants::placement(l, st, pos, ctx),
        Beh::ConcretePowder => Some(falling::placement(l, st, pos)),
        _ => Some(st),
    }
}

/// `BlockState.randomTick`. Only called for states with the vanilla
/// `isRandomlyTicking` flag.
pub fn random_tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, rng: &mut Rng) {
    match beh(st) {
        Beh::None | Beh::Stairs | Beh::Fence | Beh::Bars | Beh::Wall | Beh::FenceGate | Beh::Door | Beh::Chorus | Beh::HugeMushroom => {}
        Beh::Fire | Beh::SoulFire => {}
        Beh::Crop | Beh::Stem | Beh::NetherWart | Beh::Cocoa | Beh::SweetBerry | Beh::PitcherCrop => crops::random_tick(rs, l, st, pos, rng),
        Beh::Spreading | Beh::Nylium => grass::random_tick(rs, l, st, pos, rng),
        Beh::Snowy => {}
        Beh::Copper => copper::random_tick(rs, l, st, pos, rng),
        Beh::Leaves => leaves::random_tick(rs, l, st, pos),
        Beh::Ice | Beh::FrostedIce | Beh::Snow => snow::random_tick(rs, l, st, pos),
        Beh::Sapling => sapling::random_tick(rs, l, st, pos, rng),
        Beh::RedstoneOre | Beh::Farmland | Beh::TurtleEgg | Beh::DriedGhast | Beh::ChorusFlower | Beh::Speleothem | Beh::Propagule => misc::random_tick(rs, l, st, pos, rng),
        Beh::Cactus | Beh::SugarCane | Beh::Bamboo | Beh::BambooSapling | Beh::GrowHead | Beh::Mushroom | Beh::BuddingAmethyst | Beh::Eyeblossom | Beh::Vine => plants::random_tick(rs, l, st, pos, rng),
        Beh::GrowBody | Beh::AmethystCluster => {}
        Beh::FlowerPot => plants::potted_eye_random_tick(rs, l, st, pos),
        #[allow(unreachable_patterns)]
        _ => {}
    }
}

/// `FluidState.randomTick` (only lava ticks randomly): fire from lava.
pub fn fluid_random_tick<L: Lvl>(rs: &mut Rs, l: &mut L, _st: u16, pos: BlockPos, rng: &mut Rng) {
    fire::lava_random_tick(rs, l, pos, rng);
}

/// `ServerLevel.tickPrecipitation` at column (x, z): ice, snow layers,
/// `handlePrecipitation` of the block below the top.
pub fn precipitation<L: Lvl>(rs: &mut Rs, l: &mut L, x: i32, z: i32, rng: &mut Rng) {
    snow::precipitation(rs, l, x, z, rng);
}

/// `BlockState.updateShape` for blocks the redstone engine does not handle.
/// `dir` points from `pos` to the changed neighbor `npos`, now `nst`.
pub fn update_shape<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, dir: u8, npos: BlockPos, nst: u16) -> u16 {
    match beh(st) {
        Beh::None | Beh::Door => st,
        Beh::Crop | Beh::Stem | Beh::NetherWart | Beh::Cocoa | Beh::SweetBerry | Beh::PitcherCrop => crops::update_shape(rs, l, st, pos, dir, npos, nst),
        Beh::Spreading | Beh::Snowy => grass::update_shape(st, dir, nst),
        Beh::Cactus | Beh::SugarCane | Beh::Bamboo | Beh::BambooSapling | Beh::GrowHead | Beh::GrowBody | Beh::Mushroom | Beh::AmethystCluster | Beh::Vine => plants::update_shape(rs, l, st, pos, dir, npos, nst),
        Beh::Sapling => sapling::update_shape(&*l, st, pos),
        Beh::BuddingAmethyst | Beh::Eyeblossom => st,
        Beh::Nylium => st,
        Beh::Copper => st,
        Beh::Stairs | Beh::Fence | Beh::Bars | Beh::Wall | Beh::FenceGate | Beh::Chorus | Beh::HugeMushroom => shape::update_shape(rs, l, st, pos, dir, npos, nst),
        Beh::Falling | Beh::ConcretePowder | Beh::DragonEgg | Beh::Brushable => falling::update_shape(rs, l, st, pos),
        Beh::Leaves => leaves::update_shape(rs, l, st, pos, nst),
        Beh::Fire | Beh::SoulFire => fire::update_shape(rs, l, st, pos, dir, npos, nst),
        Beh::Snow => snow::update_shape(l, st, pos),
        Beh::Ice | Beh::FrostedIce => st,
        Beh::Farmland | Beh::DriedGhast | Beh::ChorusFlower | Beh::ChorusPlant | Beh::Speleothem | Beh::SporeBlossom => misc::update_shape(rs, l, st, pos, dir, nst),
        Beh::Bed => bed::update_shape(st, dir, nst),
        #[allow(unreachable_patterns)]
        _ => st,
    }
}

/// A scheduled tick came due for a block the engine does not know.
pub fn tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    match beh(st) {
        Beh::Cactus | Beh::SugarCane | Beh::Bamboo | Beh::GrowHead | Beh::GrowBody | Beh::Eyeblossom => plants::tick(rs, l, st, pos),
        Beh::Chorus => shape::tick(rs, l, st, pos),
        Beh::Leaves => leaves::tick(rs, l, st, pos),
        Beh::Fire => fire::tick(rs, l, st, pos),
        Beh::FrostedIce => snow::tick(rs, l, st, pos),
        Beh::Composter => uses::tick(rs, l, st, pos),
        Beh::Falling | Beh::ConcretePowder | Beh::DragonEgg | Beh::Brushable => falling::tick(rs, l, st, pos),
        _ => misc::tick(rs, l, st, pos),
    }
}

/// `BlockState.onPlace` for blocks the engine does not know.
pub fn on_place<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, old: u16) {
    match beh(st) {
        Beh::Fire | Beh::SoulFire => fire::on_place(rs, l, st, pos, old),
        Beh::FrostedIce => snow::on_place(rs, l, pos),
        Beh::SnifferEgg => misc::on_place(rs, l, st, pos),
        Beh::Composter => uses::on_place(rs, l, st, pos),
        Beh::Sponge => sponge::on_place(rs, l, st, pos, old),
        Beh::Falling | Beh::ConcretePowder | Beh::DragonEgg | Beh::Brushable => falling::on_place(rs, l, st, pos),
        _ => {}
    }
}

/// `BlockState.handleNeighborChanged` for blocks the engine does not know.
pub fn neighbor_changed<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, from: u16) {
    match beh(st) {
        Beh::FrostedIce => snow::neighbor_changed(rs, l, pos, from),
        Beh::Bell => uses::neighbor_changed(rs, l, st, pos),
        Beh::Sponge => sponge::neighbor_changed(rs, l, pos),
        _ => {}
    }
}
#[cfg(test)]
mod getbench;
