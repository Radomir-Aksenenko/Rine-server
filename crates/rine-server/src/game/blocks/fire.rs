//! Fire and soul fire: `BaseFireBlock`, `FireBlock`, `SoulFireBlock`, and the
//! fire that lava lights in `LavaFluid.randomTick`.
//!
//! A fire ticks every `30 + nextInt(10)` game ticks: spread rule check, rain
//! extinguish, aging, burnout of the six neighbours, and spreading into the
//! 3x6x3 box around it. Odds come from `FireBlock.bootStrap`, resolved once
//! into per-block tables.
//!
//! Not ported:
//! - portal lighting in `BaseFireBlock.onPlace` / `canBePlacedAt` (Rine has
//!   no nether portal shape yet); `can_be_lit` ignores the portal branch;
//! - entity damage (`entityInside`), particles and sounds (client side);
//! - the end and nether infiniburn tags (Rine has the overworld only);
//! - the temperature noise of `Biome.getHeightAdjustedTemperature` and the
//!   frozen temperature modifier: `warm_enough` uses the base temperature
//!   with the height term and no noise.

use super::props::{getb, geti, seti, setb, Pr};
use super::{beh, block_id, block_of, block_tag, default_state, is_air, rule_int, Beh, Rng};
use crate::game::redstone::data::{self, step, DOWN, EAST, NORTH, SOUTH, UP, WEST};
use crate::game::redstone::engine::{self, fl, Lvl, UPDATE_LIMIT};
use crate::game::redstone::Rs;
use rine_protocol::BlockPos;
use std::sync::OnceLock;

// ------------------------------------------------------------------ data

/// `FireBlock.bootStrap`: (igniteOdds, burnOdds, registry names).
const FLAMMABLE: &[(u8, u8, &[&str])] = &[
    (5, 20, &[
        "oak_planks", "spruce_planks", "birch_planks", "jungle_planks", "acacia_planks",
        "cherry_planks", "dark_oak_planks", "pale_oak_planks", "poplar_planks", "mangrove_planks",
        "bamboo_planks", "bamboo_mosaic", "oak_slab", "spruce_slab", "birch_slab", "jungle_slab",
        "acacia_slab", "cherry_slab", "dark_oak_slab", "pale_oak_slab", "poplar_slab",
        "mangrove_slab", "bamboo_slab", "bamboo_mosaic_slab", "oak_fence_gate",
        "spruce_fence_gate", "birch_fence_gate", "jungle_fence_gate", "acacia_fence_gate",
        "cherry_fence_gate", "dark_oak_fence_gate", "pale_oak_fence_gate", "poplar_fence_gate",
        "mangrove_fence_gate", "bamboo_fence_gate", "oak_fence", "spruce_fence", "birch_fence",
        "jungle_fence", "acacia_fence", "cherry_fence", "dark_oak_fence", "pale_oak_fence",
        "poplar_fence", "mangrove_fence", "bamboo_fence", "oak_stairs", "birch_stairs",
        "spruce_stairs", "jungle_stairs", "acacia_stairs", "cherry_stairs", "dark_oak_stairs",
        "pale_oak_stairs", "poplar_stairs", "mangrove_stairs", "bamboo_stairs",
        "bamboo_mosaic_stairs", "mangrove_roots", "composter", "beehive",
    ]),
    (5, 5, &[
        "oak_log", "spruce_log", "birch_log", "jungle_log", "acacia_log", "cherry_log",
        "pale_oak_log", "poplar_log", "dark_oak_log", "mangrove_log", "bamboo_block",
        "stripped_oak_log", "stripped_spruce_log", "stripped_birch_log", "stripped_jungle_log",
        "stripped_acacia_log", "stripped_cherry_log", "stripped_dark_oak_log",
        "stripped_pale_oak_log", "stripped_poplar_log", "stripped_mangrove_log",
        "stripped_bamboo_block", "stripped_oak_wood", "stripped_spruce_wood",
        "stripped_birch_wood", "stripped_jungle_wood", "stripped_acacia_wood",
        "stripped_cherry_wood", "stripped_dark_oak_wood", "stripped_pale_oak_wood",
        "stripped_poplar_wood", "stripped_mangrove_wood", "oak_wood", "spruce_wood", "birch_wood",
        "jungle_wood", "acacia_wood", "cherry_wood", "pale_oak_wood", "poplar_wood",
        "dark_oak_wood", "mangrove_wood", "coal_block",
    ]),
    (30, 60, &[
        "oak_leaves", "spruce_leaves", "birch_leaves", "jungle_leaves", "acacia_leaves",
        "cherry_leaves", "dark_oak_leaves", "pale_oak_leaves", "mangrove_leaves",
        "red_poplar_leaves", "orange_poplar_leaves", "yellow_poplar_leaves", "dried_kelp_block",
        "azalea_leaves", "flowering_azalea_leaves", "azalea", "flowering_azalea", "hanging_roots",
        "straw_bed",
    ]),
    (30, 20, &[
        "bookshelf", "lectern", "bee_nest", "acacia_shelf", "bamboo_shelf", "birch_shelf",
        "cherry_shelf", "dark_oak_shelf", "jungle_shelf", "mangrove_shelf", "poplar_shelf",
        "oak_shelf", "pale_oak_shelf", "spruce_shelf",
    ]),
    (15, 100, &[
        "tnt", "vine", "glow_lichen",
    ]),
    (60, 100, &[
        "short_grass", "fern", "dead_bush", "short_dry_grass", "tall_dry_grass", "sunflower",
        "lilac", "rose_bush", "peony", "red_shrub", "tall_grass", "large_fern", "dandelion",
        "golden_dandelion", "poppy", "open_eyeblossom", "closed_eyeblossom", "blue_orchid",
        "allium", "azure_bluet", "red_tulip", "orange_tulip", "white_tulip", "pink_tulip",
        "oxeye_daisy", "cornflower", "lily_of_the_valley", "torchflower", "pitcher_plant",
        "wither_rose", "pink_petals", "wildflowers", "leaf_litter", "cactus_flower",
        "sweet_berry_bush", "spore_blossom", "big_dripleaf", "big_dripleaf_stem", "small_dripleaf",
        "firefly_bush", "bush",
    ]),
    (60, 20, &[
        "hay_block",
    ]),
    (15, 20, &[
        "target",
    ]),
    (5, 100, &[
        "pale_moss_block", "pale_moss_carpet", "pale_hanging_moss",
    ]),
    (60, 60, &[
        "bamboo", "scaffolding",
    ]),
    (15, 60, &[
        "cave_vines", "cave_vines_plant",
    ]),
];

/// Dye colours, the order of the colour groups below.
const COLOURS: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray",
    "cyan", "purple", "blue", "brown", "green", "red", "black",
];

/// `FireBlock.bootStrap` colour groups: suffix after the colour name.
const FLAMMABLE_COLOURED: &[(u8, u8, &[&str])] = &[
    (30, 60, &["wool", "wool_stairs", "wool_slab"]),
    (60, 20, &["carpet"]),
];

/// `Block.ignitedByLava` (`.ignitedByLava()` in `Blocks.java`), without the colour groups.
const IGNITED_BY_LAVA: &[&str] = &[
    "oak_planks", "spruce_planks", "birch_planks", "jungle_planks", "acacia_planks",
    "cherry_planks", "dark_oak_planks", "pale_oak_wood", "pale_oak_planks", "mangrove_planks",
    "poplar_planks", "bamboo_planks", "bamboo_mosaic", "mangrove_roots", "oak_wood", "spruce_wood",
    "birch_wood", "jungle_wood", "acacia_wood", "cherry_wood", "dark_oak_wood", "mangrove_wood",
    "poplar_wood", "stripped_oak_wood", "stripped_spruce_wood", "stripped_birch_wood",
    "stripped_jungle_wood", "stripped_acacia_wood", "stripped_cherry_wood",
    "stripped_dark_oak_wood", "stripped_pale_oak_wood", "stripped_poplar_wood", "cherry_leaves",
    "pale_oak_leaves", "note_block", "straw_bed", "short_grass", "fern", "dead_bush", "bush",
    "red_shrub", "short_dry_grass", "tall_dry_grass", "tnt", "bookshelf", "chiseled_bookshelf",
    "acacia_shelf", "bamboo_shelf", "birch_shelf", "cherry_shelf", "crimson_shelf",
    "dark_oak_shelf", "jungle_shelf", "mangrove_shelf", "oak_shelf", "pale_oak_shelf",
    "poplar_shelf", "spruce_shelf", "warped_shelf", "chest", "crafting_table", "oak_sign",
    "spruce_sign", "birch_sign", "acacia_sign", "cherry_sign", "jungle_sign", "dark_oak_sign",
    "pale_oak_sign", "mangrove_sign", "poplar_sign", "bamboo_sign", "oak_door", "oak_wall_sign",
    "spruce_wall_sign", "birch_wall_sign", "acacia_wall_sign", "cherry_wall_sign",
    "jungle_wall_sign", "dark_oak_wall_sign", "pale_oak_wall_sign", "mangrove_wall_sign",
    "poplar_wall_sign", "bamboo_wall_sign", "oak_hanging_sign", "spruce_hanging_sign",
    "birch_hanging_sign", "acacia_hanging_sign", "cherry_hanging_sign", "jungle_hanging_sign",
    "dark_oak_hanging_sign", "pale_oak_hanging_sign", "mangrove_hanging_sign",
    "poplar_hanging_sign", "bamboo_hanging_sign", "oak_wall_hanging_sign",
    "spruce_wall_hanging_sign", "birch_wall_hanging_sign", "acacia_wall_hanging_sign",
    "cherry_wall_hanging_sign", "jungle_wall_hanging_sign", "dark_oak_wall_hanging_sign",
    "pale_oak_wall_hanging_sign", "mangrove_wall_hanging_sign", "poplar_wall_hanging_sign",
    "bamboo_wall_hanging_sign", "oak_pressure_plate", "spruce_pressure_plate",
    "birch_pressure_plate", "jungle_pressure_plate", "acacia_pressure_plate",
    "cherry_pressure_plate", "dark_oak_pressure_plate", "pale_oak_pressure_plate",
    "mangrove_pressure_plate", "poplar_pressure_plate", "bamboo_pressure_plate", "cactus_flower",
    "jukebox", "oak_fence", "oak_trapdoor", "spruce_trapdoor", "birch_trapdoor", "jungle_trapdoor",
    "acacia_trapdoor", "cherry_trapdoor", "dark_oak_trapdoor", "pale_oak_trapdoor",
    "mangrove_trapdoor", "poplar_trapdoor", "bamboo_trapdoor", "brown_mushroom_block",
    "red_mushroom_block", "mushroom_stem", "vine", "glow_lichen", "resin_clump", "oak_fence_gate",
    "trapped_chest", "daylight_detector", "sunflower", "lilac", "rose_bush", "peony", "tall_grass",
    "large_fern", "spruce_fence_gate", "birch_fence_gate", "jungle_fence_gate",
    "acacia_fence_gate", "cherry_fence_gate", "dark_oak_fence_gate", "pale_oak_fence_gate",
    "mangrove_fence_gate", "poplar_fence_gate", "bamboo_fence_gate", "spruce_fence", "birch_fence",
    "jungle_fence", "acacia_fence", "cherry_fence", "dark_oak_fence", "pale_oak_fence",
    "mangrove_fence", "poplar_fence", "bamboo_fence", "spruce_door", "birch_door", "jungle_door",
    "acacia_door", "cherry_door", "dark_oak_door", "pale_oak_door", "mangrove_door", "poplar_door",
    "bamboo_door", "pitcher_plant", "bamboo_sapling", "bamboo", "loom", "barrel",
    "cartography_table", "fletching_table", "lectern", "smithing_table", "campfire",
    "soul_campfire", "composter", "bee_nest", "beehive", "hanging_roots", "pale_moss_block",
    "pale_moss_carpet", "pale_hanging_moss", "firefly_bush",
];

/// `ignitedByLava` colour groups: bed, wool, carpet, banner, wall banner.
const IGNITED_COLOURED: &[&str] = &["bed", "wool", "carpet", "banner", "wall_banner"];

/// Overworld biomes from the vanilla data: (name, has_precipitation,
/// temperature, increased_fire_burnout attribute).
const BIOMES: &[(&str, bool, f32, bool)] = &[
    ("badlands", false, 2.0, false),
    ("bamboo_jungle", true, 0.95, true),
    ("basalt_deltas", false, 2.0, false),
    ("beach", true, 0.8, false),
    ("birch_forest", true, 0.6, false),
    ("cherry_grove", true, 0.5, false),
    ("cold_ocean", true, 0.5, false),
    ("crimson_forest", false, 2.0, false),
    ("dappled_forest", true, 0.6, false),
    ("dark_forest", true, 0.7, false),
    ("deep_cold_ocean", true, 0.5, false),
    ("deep_dark", true, 0.8, false),
    ("deep_frozen_ocean", true, 0.5, false),
    ("deep_lukewarm_ocean", true, 0.5, false),
    ("deep_ocean", true, 0.5, false),
    ("desert", false, 2.0, false),
    ("dripstone_caves", true, 0.8, false),
    ("end_barrens", false, 0.5, false),
    ("end_highlands", false, 0.5, false),
    ("end_midlands", false, 0.5, false),
    ("eroded_badlands", false, 2.0, false),
    ("flower_forest", true, 0.7, false),
    ("forest", true, 0.7, false),
    ("frozen_ocean", true, 0.0, false),
    ("frozen_peaks", true, -0.7, true),
    ("frozen_river", true, 0.0, false),
    ("grove", true, -0.2, false),
    ("ice_spikes", true, 0.0, false),
    ("jagged_peaks", true, -0.7, true),
    ("jungle", true, 0.95, true),
    ("lukewarm_ocean", true, 0.5, false),
    ("lush_caves", true, 0.5, false),
    ("mangrove_swamp", true, 0.8, true),
    ("meadow", true, 0.5, false),
    ("mushroom_fields", true, 0.9, true),
    ("nether_wastes", false, 2.0, false),
    ("ocean", true, 0.5, false),
    ("old_growth_birch_forest", true, 0.6, false),
    ("old_growth_pine_taiga", true, 0.3, false),
    ("old_growth_spruce_taiga", true, 0.25, false),
    ("pale_garden", true, 0.7, false),
    ("plains", true, 0.8, false),
    ("river", true, 0.5, false),
    ("savanna", false, 2.0, false),
    ("savanna_plateau", false, 2.0, false),
    ("small_end_islands", false, 0.5, false),
    ("snowy_beach", true, 0.05, false),
    ("snowy_plains", true, 0.0, false),
    ("snowy_slopes", true, -0.3, true),
    ("snowy_taiga", true, -0.5, false),
    ("soul_sand_valley", false, 2.0, false),
    ("sparse_jungle", true, 0.95, false),
    ("stony_peaks", true, 1.0, false),
    ("stony_shore", true, 0.2, false),
    ("sulfur_caves", true, 0.8, false),
    ("sunflower_plains", true, 0.8, false),
    ("swamp", true, 0.8, true),
    ("taiga", true, 0.25, false),
    ("the_end", false, 0.5, false),
    ("the_void", false, 0.5, false),
    ("warm_ocean", true, 0.5, false),
    ("warped_forest", false, 2.0, false),
    ("windswept_forest", true, 0.2, false),
    ("windswept_gravelly_hills", true, 0.2, false),
    ("windswept_hills", true, 0.2, false),
    ("windswept_savanna", false, 2.0, false),
    ("wooded_badlands", false, 2.0, false),
];

/// `Level.getSeaLevel` in the overworld.
const SEA_LEVEL: i32 = 63;
/// `Biome.getHeightAdjustedTemperature`: cooling starts 17 blocks above sea level.
const SNOW_LEVEL: i32 = SEA_LEVEL + 17;

#[derive(Clone, Copy)]
struct BiomeInfo {
    precip: bool,
    temp: f32,
    incr_burnout: bool,
}

/// Biome ids outside the table (should not happen): rain, plains-like warmth.
const NO_BIOME: BiomeInfo = BiomeInfo { precip: true, temp: 0.5, incr_burnout: false };

struct T {
    /// `FireBlock.igniteOdds` by block id (0: not flammable).
    ignite: Vec<u8>,
    /// `FireBlock.burnOdds` by block id.
    burn: Vec<u8>,
    ignited_by_lava: Vec<bool>,
    /// `BlockTags.BLOCKS_LAVA_FIRE_SPREAD`.
    lava_spread: Vec<bool>,
    /// `BlockTags.INFINIBURN_OVERWORLD`.
    infiniburn: Vec<bool>,
    /// `BlockTags.SOUL_FIRE_BASE_BLOCKS`.
    soul_base: Vec<bool>,
    /// By synced biome registry id.
    biomes: Vec<BiomeInfo>,
    fire: u16,
    tnt: u16,
    fire_default: u16,
    soul_default: u16,
    air: u16,
}

fn t() -> &'static T {
    static T_: OnceLock<T> = OnceLock::new();
    T_.get_or_init(build)
}

fn build() -> T {
    let n = rine_data::get().blocks.blocks.len();
    let id = |name: &str| -> Option<usize> {
        let i = block_id(name) as usize;
        (i < n).then_some(i)
    };
    let mut ignite = vec![0u8; n];
    let mut burn = vec![0u8; n];
    for &(a, b, names) in FLAMMABLE {
        for name in names {
            if let Some(i) = id(name) {
                ignite[i] = a;
                burn[i] = b;
            }
        }
    }
    for &(a, b, suffixes) in FLAMMABLE_COLOURED {
        for c in COLOURS {
            for s in suffixes {
                if let Some(i) = id(&format!("{c}_{s}")) {
                    ignite[i] = a;
                    burn[i] = b;
                }
            }
        }
    }
    let mut ignited_by_lava = vec![false; n];
    for name in IGNITED_BY_LAVA {
        if let Some(i) = id(name) {
            ignited_by_lava[i] = true;
        }
    }
    for s in IGNITED_COLOURED {
        for c in COLOURS {
            if let Some(i) = id(&format!("{c}_{s}")) {
                ignited_by_lava[i] = true;
            }
        }
    }
    let reg = &rine_data::get().registries;
    let mut biomes: Vec<BiomeInfo> = Vec::new();
    for &(name, precip, temp, incr) in BIOMES {
        if let Some(i) = reg.id("minecraft:worldgen/biome", &format!("minecraft:{name}")) {
            let i = i as usize;
            if biomes.len() <= i {
                biomes.resize(i + 1, NO_BIOME);
            }
            biomes[i] = BiomeInfo { precip, temp, incr_burnout: incr };
        }
    }
    T {
        ignite,
        burn,
        ignited_by_lava,
        lava_spread: block_tag("blocks_lava_fire_spread"),
        infiniburn: block_tag("infiniburn_overworld"),
        soul_base: block_tag("soul_fire_base_blocks"),
        biomes,
        fire: block_id("fire"),
        tnt: block_id("tnt"),
        fire_default: default_state("fire"),
        soul_default: default_state("soul_fire"),
        air: default_state("air"),
    }
}

// ------------------------------------------------------------------ helpers

/// The six neighbour directions (`Direction.values()` order is irrelevant here:
/// nothing in the loops draws random numbers per side).
const ALL6: [u8; 6] = [DOWN, UP, NORTH, SOUTH, WEST, EAST];

/// Connection properties of `FireBlock` (`PROPERTY_BY_DIRECTION` without DOWN).
const SIDES: [(u8, Pr); 5] = [(UP, Pr::Up), (NORTH, Pr::North), (SOUTH, Pr::South), (WEST, Pr::West), (EAST, Pr::East)];

#[inline]
fn rel(p: BlockPos, d: u8) -> BlockPos {
    let (x, y, z) = step(d);
    p.offset(x, y, z)
}

#[inline]
fn below(p: BlockPos) -> BlockPos {
    p.offset(0, -1, 0)
}

/// `FireBlock.igniteOdds` of a state (waterlogged blocks never catch).
#[inline]
fn ignite_of(st: u16) -> u8 {
    if getb(st, Pr::Waterlogged) {
        0
    } else {
        t().ignite[block_of(st) as usize]
    }
}

#[inline]
fn burn_of(st: u16) -> u8 {
    if getb(st, Pr::Waterlogged) {
        0
    } else {
        t().burn[block_of(st) as usize]
    }
}

/// `FireBlock.canBurn`.
#[inline]
fn can_burn(st: u16) -> bool {
    ignite_of(st) > 0
}

/// `FireBlock.isValidFireLocation`: a neighbour can burn.
fn valid_fire_location<L: Lvl>(l: &L, p: BlockPos) -> bool {
    ALL6.iter().any(|&d| can_burn(l.get(rel(p, d))))
}

/// `FireBlock.getIgniteOdds(level, pos)`: the best neighbour odds of an empty spot.
fn ignite_odds_at<L: Lvl>(l: &L, p: BlockPos) -> u8 {
    if !is_air(l.get(p)) {
        return 0;
    }
    ALL6.iter().map(|&d| ignite_of(l.get(rel(p, d)))).max().unwrap_or(0)
}

/// `canSurvive` of fire (`FireBlock`) or soul fire (`SoulFireBlock`).
fn can_survive<L: Lvl>(l: &L, st: u16, p: BlockPos) -> bool {
    let b = l.get(below(p));
    if beh(st) == Beh::SoulFire {
        return t().soul_base[block_of(b) as usize];
    }
    data::sturdy(b, UP) || valid_fire_location(l, p)
}

/// `BaseFireBlock.getState`: soul fire on soul base blocks, else fire.
pub fn fire_state_at<L: Lvl>(l: &L, p: BlockPos) -> u16 {
    let t = t();
    if t.soul_base[block_of(l.get(below(p))) as usize] {
        return t.soul_default;
    }
    // `FireBlock.getStateForPlacement`
    let b = l.get(below(p));
    if can_burn(b) || data::sturdy(b, UP) {
        return t.fire_default;
    }
    let mut st = t.fire_default;
    for (d, pr) in SIDES {
        st = setb(st, pr, can_burn(l.get(rel(p, d))));
    }
    st
}

/// `FireBlock.getStateWithAge`.
fn state_with_age<L: Lvl>(l: &L, p: BlockPos, age: i32) -> u16 {
    let st = fire_state_at(l, p);
    if block_of(st) == t().fire {
        seti(st, Pr::Age, age)
    } else {
        st
    }
}

/// `BaseFireBlock.canBePlacedAt` for an air position (flint and steel, fire
/// charge). The portal branch is not ported.
pub fn can_be_lit<L: Lvl>(l: &L, p: BlockPos, _face: u8) -> bool {
    is_air(l.get(p)) && {
        let st = fire_state_at(l, p);
        can_survive(l, st, p)
    }
}

/// Biome of a position, by synced registry id.
#[inline]
fn biome_info(id: u8) -> BiomeInfo {
    t().biomes.get(id as usize).copied().unwrap_or(NO_BIOME)
}

/// `Biome.warmEnoughToRain` at `y` (noise term left out, see module docs).
fn warm_enough(base: f32, y: i32) -> bool {
    let mut temp = base;
    if y > SNOW_LEVEL {
        temp -= (y - SNOW_LEVEL) as f32 * 0.05 / 40.0;
    }
    temp >= 0.15
}

/// `Level.isRainingAt`: rain falls here as rain, not snow.
pub fn raining_at<L: Lvl>(l: &L, p: BlockPos) -> bool {
    if !l.raining() || l.sky_light(p) < 15 || l.height_motion_blocking(p.x, p.z) > p.y {
        return false;
    }
    let b = biome_info(l.biome(p));
    b.precip && warm_enough(b.temp, p.y)
}

/// `FireBlock.isNearRain`.
fn is_near_rain<L: Lvl>(l: &L, p: BlockPos) -> bool {
    raining_at(l, p) || raining_at(l, rel(p, WEST)) || raining_at(l, rel(p, EAST)) || raining_at(l, rel(p, NORTH)) || raining_at(l, rel(p, SOUTH))
}

fn increased_burnout<L: Lvl>(l: &L, p: BlockPos) -> bool {
    biome_info(l.biome(p)).incr_burnout
}

static FIRE_RADIUS: OnceLock<Option<usize>> = OnceLock::new();

/// `ServerLevel.canSpreadFireAround`: with `fire_spread_radius_around_player`
/// -1 always true, else some non-spectator player closer than the radius
/// (`distanceTo` from the block corner, strictly less). No game (tests): true.
pub fn can_spread<L: Lvl>(l: &mut L, p: BlockPos) -> bool {
    let radius = rule_int(&*l, &FIRE_RADIUS, "fire_spread_radius_around_player", 128);
    if radius == -1 {
        return true;
    }
    let Some(g) = l.game() else { return true };
    if radius <= 0 {
        // distance is never < 0, and < 0 never holds for distance 0 either
        return false;
    }
    // per-tick player grid instead of a scan of every player (`blocks::near`)
    g.blocks.near.ensure(g.tick, radius, g.players.iter().flatten().filter(|pl| pl.gamemode != 3).map(|pl| &pl.pos));
    g.blocks.near.any_within([p.x as f64, p.y as f64, p.z as f64], radius)
}

/// `Level.getDifficulty().getId()`: peaceful 0 .. hard 3. No game (tests): normal.
fn difficulty<L: Lvl>(l: &mut L) -> i32 {
    match l.game() {
        Some(g) => crate::game::cmd::commands::world::difficulty(g) as i32,
        None => 2,
    }
}

#[inline]
fn fire_tick_delay(rs: &mut Rs) -> u32 {
    // `FireBlock.getFireTickDelay`: Paper's fire-tick-delay (30) + nextInt(10)
    30 + rs.rng.next_int(10) as u32
}

/// `BaseFireBlock.fireExtinguished`: `removeBlock(pos, false)`.
fn extinguish<L: Lvl>(rs: &mut Rs, l: &mut L, p: BlockPos) {
    engine::remove_block(rs, l, p, false);
}

// ------------------------------------------------------------------ hooks

/// Block id to behaviour for the two fire blocks.
pub fn classify(n: &str) -> Option<Beh> {
    match n {
        "fire" => Some(Beh::Fire),
        "soul_fire" => Some(Beh::SoulFire),
        _ => None,
    }
}

/// `BlockState.updateShape`: a fire that cannot survive becomes air; a fire
/// that can keeps its age and gets new connections.
pub fn update_shape<L: Lvl>(_rs: &mut Rs, l: &L, st: u16, p: BlockPos, _dir: u8, _npos: BlockPos, _nst: u16) -> u16 {
    match beh(st) {
        Beh::Fire => {
            if can_survive(l, st, p) {
                state_with_age(l, p, geti(st, Pr::Age))
            } else {
                t().air
            }
        }
        Beh::SoulFire => {
            if can_survive(l, st, p) {
                t().soul_default
            } else {
                t().air
            }
        }
        _ => st,
    }
}

/// `BaseFireBlock.onPlace` then, for fire, `FireBlock.onPlace` (schedule).
pub fn on_place<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, p: BlockPos, old: u16) {
    if block_of(old) != block_of(st) && !can_survive(l, st, p) {
        extinguish(rs, l, p);
    }
    if beh(st) == Beh::Fire {
        let delay = fire_tick_delay(rs);
        engine::schedule(rs, l, p, block_of(st), delay, engine::prio::NORMAL);
    }
}

/// `FireBlock.tick` (scheduled). Soul fire has no tick.
pub fn tick<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, p: BlockPos) {
    if beh(st) != Beh::Fire {
        return;
    }
    let delay = fire_tick_delay(rs);
    engine::schedule(rs, l, p, block_of(st), delay, engine::prio::NORMAL);
    if !can_spread(l, p) {
        return;
    }
    if !can_survive(l, st, p) {
        extinguish(rs, l, p);
    }
    let below_p = below(p);
    let infini = t().infiniburn[block_of(l.get(below_p)) as usize];
    let age = geti(st, Pr::Age);
    if !infini && l.raining() && is_near_rain(l, p) && rs.rng.next_float() < 0.2 + age as f32 * 0.03 {
        extinguish(rs, l, p);
        return;
    }
    let new_age = (age + rs.rng.next_int(3) / 2).min(15);
    if age != new_age {
        engine::set_block(rs, l, p, seti(st, Pr::Age, new_age), 0, UPDATE_LIMIT);
    }
    if !infini {
        if !valid_fire_location(l, p) {
            if !data::sturdy(l.get(below_p), UP) || age > 3 {
                extinguish(rs, l, p);
            }
            return;
        }
        if age == 15 && rs.rng.next_int(4) == 0 && !can_burn(l.get(below_p)) {
            extinguish(rs, l, p);
            return;
        }
    }
    spread(rs, l, p, age);
}

/// The burnout checks and the spread of `FireBlock.tick`.
fn spread<L: Lvl>(rs: &mut Rs, l: &mut L, p: BlockPos, age: i32) {
    let incr = increased_burnout(l, p);
    let extra = if incr { -50 } else { 0 };
    check_burn_out(rs, l, p.offset(1, 0, 0), 300 + extra, age);
    check_burn_out(rs, l, p.offset(-1, 0, 0), 300 + extra, age);
    check_burn_out(rs, l, p.offset(0, -1, 0), 250 + extra, age);
    check_burn_out(rs, l, p.offset(0, 1, 0), 250 + extra, age);
    check_burn_out(rs, l, p.offset(0, 0, -1), 300 + extra, age);
    check_burn_out(rs, l, p.offset(0, 0, 1), 300 + extra, age);

    let diff = difficulty(l);
    for xx in -1..=1 {
        for zz in -1..=1 {
            for yy in -1..=4 {
                if xx == 0 && yy == 0 && zz == 0 {
                    continue;
                }
                let mut rate = 100;
                if yy > 1 {
                    rate += (yy - 1) * 100;
                }
                let tp = p.offset(xx, yy, zz);
                let ig = ignite_odds_at(l, tp) as i32;
                if ig <= 0 {
                    continue;
                }
                let mut odds = (ig + 40 + diff * 7) / (age + 30);
                if incr {
                    odds /= 2;
                }
                if odds > 0 && rs.rng.next_int(rate) <= odds && (!l.raining() || !is_near_rain(l, tp)) {
                    let spread_age = (age + rs.rng.next_int(5) / 4).min(15);
                    let st = state_with_age(l, tp, spread_age);
                    engine::set_block(rs, l, tp, st, fl::ALL, UPDATE_LIMIT);
                }
            }
        }
    }
}

/// `FireBlock.checkBurnOut`: maybe burn the block at `p`. TNT is primed and
/// removed in every case where it burns.
fn check_burn_out<L: Lvl>(rs: &mut Rs, l: &mut L, p: BlockPos, chance: i32, age: i32) {
    let old = l.get(p);
    let odds = burn_of(old) as i32;
    if rs.rng.next_int(chance) < odds {
        let tnt = block_of(old) == t().tnt;
        if rs.rng.next_int(age + 10) < 5 && !raining_at(l, p) {
            let new_age = (age + rs.rng.next_int(5) / 4).min(15);
            let st = state_with_age(l, p, new_age);
            engine::set_block(rs, l, p, st, fl::ALL, UPDATE_LIMIT);
        } else if !tnt {
            extinguish(rs, l, p);
        }
        if tnt {
            extinguish(rs, l, p);
            l.prime_tnt(p);
        }
    }
}

/// `LavaFluid.randomTick`: lava lights a flammable block nearby.
pub fn lava_random_tick<L: Lvl>(rs: &mut Rs, l: &mut L, p: BlockPos, rng: &mut Rng) {
    if !can_spread(l, p) {
        return;
    }
    let passes = rng.next_int(3);
    if passes > 0 {
        let mut tp = p;
        for _ in 0..passes {
            let dx = rng.next_int(3) - 1;
            let dz = rng.next_int(3) - 1;
            tp = tp.offset(dx, 1, dz);
            if !l.loaded(tp) {
                return;
            }
            let s = l.get(tp);
            if is_air(s) {
                if has_flammable_neighbours(l, tp) {
                    let f = fire_state_at(l, tp);
                    engine::set_block(rs, l, tp, f, fl::ALL, UPDATE_LIMIT);
                    return;
                }
            } else if t().lava_spread[block_of(s) as usize] {
                return;
            }
        }
    } else {
        for _ in 0..3 {
            let dx = rng.next_int(3) - 1;
            let dz = rng.next_int(3) - 1;
            let tp = p.offset(dx, 0, dz);
            if !l.loaded(tp) {
                return;
            }
            let up = tp.offset(0, 1, 0);
            if is_air(l.get(up)) && is_flammable(l, tp) {
                // vanilla computes the state at testPos, not at the block above
                let f = fire_state_at(l, tp);
                engine::set_block(rs, l, up, f, fl::ALL, UPDATE_LIMIT);
            }
        }
    }
}

fn has_flammable_neighbours<L: Lvl>(l: &L, p: BlockPos) -> bool {
    ALL6.iter().any(|&d| is_flammable(l, rel(p, d)))
}

/// `LavaFluid.isFlammable`: loaded and `ignitedByLava`.
fn is_flammable<L: Lvl>(l: &L, p: BlockPos) -> bool {
    l.loaded(p) && t().ignited_by_lava[block_of(l.get(p)) as usize]
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::super::testw::World as TW;
    use super::*;

    /// Test level that also records the TNT primed by fire.
    struct Rec {
        w: TW,
        primed: Vec<BlockPos>,
    }

    impl Rec {
        fn new() -> Self {
            Self { w: TW::new(), primed: Vec::new() }
        }
    }

    impl Lvl for Rec {
        fn get(&self, p: BlockPos) -> u16 {
            self.w.get(p)
        }
        fn loaded(&self, p: BlockPos) -> bool {
            self.w.loaded(p)
        }
        fn raw_set(&mut self, p: BlockPos, st: u16) -> u16 {
            self.w.raw_set(p, st)
        }
        fn time(&self) -> u64 {
            self.w.time()
        }
        fn day_time(&self) -> i64 {
            self.w.day_time()
        }
        fn sky_light(&self, p: BlockPos) -> u8 {
            self.w.sky_light(p)
        }
        fn level_event(&mut self, ev: i32, p: BlockPos, data: i32) {
            self.w.level_event(ev, p, data)
        }
        fn sound(&mut self, name: &str, p: BlockPos, vol: f32, pitch: f32, cat: i32, skip: bool) {
            self.w.sound(name, p, vol, pitch, cat, skip)
        }
        fn block_event_packet(&mut self, p: BlockPos, block: u16, a: u8, b: u8) {
            self.w.block_event_packet(p, block, a, b)
        }
        fn drop_block(&mut self, p: BlockPos, st: u16) {
            self.w.drop_block(p, st)
        }
        fn prime_tnt(&mut self, p: BlockPos) {
            self.primed.push(p);
        }
        fn count_entities(&self, min: [f64; 3], max: [f64; 3], living: bool) -> usize {
            self.w.count_entities(min, max, living)
        }
        fn container_signal(&self, p: BlockPos, st: u16) -> Option<u8> {
            self.w.container_signal(p, st)
        }
        fn chest_viewers(&self, p: BlockPos) -> u8 {
            self.w.chest_viewers(p)
        }
        fn rand(&mut self) -> f32 {
            self.w.rand()
        }
        fn block_light(&self, p: BlockPos) -> u8 {
            self.w.block_light(p)
        }
        fn sky_darken(&self) -> u8 {
            self.w.sky_darken()
        }
        fn raining(&self) -> bool {
            self.w.raining()
        }
        fn thundering(&self) -> bool {
            self.w.thundering()
        }
        fn biome(&self, p: BlockPos) -> u8 {
            self.w.biome(p)
        }
        fn height_motion_blocking(&self, x: i32, z: i32) -> i32 {
            self.w.height_motion_blocking(x, z)
        }
        fn pop_resource(&mut self, p: BlockPos, item: u32, count: i32) {
            self.w.pop_resource(p, item, count)
        }
    }

    fn s(n: &str) -> u16 {
        default_state(n)
    }

    fn biome_id(n: &str) -> u8 {
        rine_data::get().registries.id("minecraft:worldgen/biome", &format!("minecraft:{n}")).expect("biome") as u8
    }

    /// One game tick: advance the clock and run the due block ticks.
    fn step(rs: &mut Rs, w: &mut Rec) {
        w.w.time += 1;
        engine::run_block_ticks(rs, w);
    }

    fn is_fire(st: u16) -> bool {
        block_of(st) == block_of(s("fire"))
    }

    #[test]
    fn every_table_name_resolves() {
        rine_data::init();
        for &(_, _, names) in FLAMMABLE {
            for n in names {
                assert_ne!(block_id(n), u16::MAX, "{n}");
            }
        }
        for &(_, _, suffixes) in FLAMMABLE_COLOURED {
            for c in COLOURS {
                for sfx in suffixes {
                    assert_ne!(block_id(&format!("{c}_{sfx}")), u16::MAX, "{c} {sfx}");
                }
            }
        }
        for n in IGNITED_BY_LAVA {
            assert_ne!(block_id(n), u16::MAX, "{n}");
        }
        for c in COLOURS {
            for sfx in IGNITED_COLOURED {
                assert_ne!(block_id(&format!("{c}_{sfx}")), u16::MAX, "{c} {sfx}");
            }
        }
        assert_eq!(t().ignite[block_of(s("oak_planks")) as usize], 5);
        assert_eq!(t().burn[block_of(s("oak_planks")) as usize], 20);
        assert_eq!(t().ignite[block_of(s("netherrack")) as usize], 0);
        assert_eq!(t().burn[block_of(s("tnt")) as usize], 100);
    }

    #[test]
    fn fire_on_netherrack_never_burns_out() {
        let mut w = Rec::new();
        let mut rs = Rs::default();
        let p = BlockPos::new(0, 64, 0);
        w.w.put(below(p), s("netherrack"));
        engine::set_block(&mut rs, &mut w, p, s("fire"), fl::ALL, UPDATE_LIMIT);
        for _ in 0..3000 {
            step(&mut rs, &mut w);
        }
        let st = w.get(p);
        assert!(is_fire(st));
        assert_eq!(geti(st, Pr::Age), 15);
    }

    #[test]
    fn fire_without_fuel_goes_out() {
        let mut w = Rec::new();
        let mut rs = Rs::default();
        let p = BlockPos::new(0, 64, 0);
        w.w.put(below(p), s("stone"));
        engine::set_block(&mut rs, &mut w, p, s("fire"), fl::ALL, UPDATE_LIMIT);
        for _ in 0..2000 {
            step(&mut rs, &mut w);
        }
        assert!(is_air(w.get(p)));
    }

    #[test]
    fn planks_next_to_fire_burn_and_fire_spreads() {
        let mut w = Rec::new();
        let mut rs = Rs::default();
        let p = BlockPos::new(0, 64, 0);
        let q = BlockPos::new(1, 64, 0);
        w.w.put(below(p), s("netherrack"));
        w.w.put(below(q), s("stone"));
        w.w.put(q, s("oak_planks"));
        engine::set_block(&mut rs, &mut w, p, s("fire"), fl::ALL, UPDATE_LIMIT);
        let mut spread = false;
        for _ in 0..3000 {
            step(&mut rs, &mut w);
            for dx in -2..=2 {
                for dz in -2..=2 {
                    for dy in -1..=2 {
                        let at = p.offset(dx, dy, dz);
                        if at != p && is_fire(w.get(at)) {
                            spread = true;
                        }
                    }
                }
            }
            if block_of(w.get(q)) != block_of(s("oak_planks")) {
                break;
            }
        }
        assert_ne!(block_of(w.get(q)), block_of(s("oak_planks")), "planks did not burn");
        assert!(spread, "fire never reached a block other than its origin");
    }

    #[test]
    fn rain_puts_out_exposed_fire() {
        let mut w = Rec::new();
        let mut rs = Rs::default();
        let p = BlockPos::new(0, 64, 0);
        w.w.put(below(p), s("stone"));
        w.w.raining = true;
        w.w.biome = biome_id("plains");
        engine::set_block(&mut rs, &mut w, p, s("fire"), fl::ALL, UPDATE_LIMIT);
        for _ in 0..2000 {
            step(&mut rs, &mut w);
            if is_air(w.get(p)) {
                break;
            }
        }
        assert!(is_air(w.get(p)));
    }

    #[test]
    fn rain_falls_as_rain_only_where_it_is_warm() {
        let mut w = Rec::new();
        let p = BlockPos::new(0, 64, 0);
        w.w.raining = true;
        w.w.biome = biome_id("plains");
        assert!(raining_at(&w, p));
        w.w.biome = biome_id("snowy_plains");
        assert!(!raining_at(&w, p), "snow, not rain");
        w.w.biome = biome_id("desert");
        assert!(!raining_at(&w, p), "no precipitation");
        w.w.biome = biome_id("plains");
        w.w.light = 14;
        assert!(!raining_at(&w, p), "no sky");
        w.w.light = 15;
        w.w.put(p.offset(0, 5, 0), s("stone"));
        assert!(!raining_at(&w, p), "covered");
        w.w.raining = false;
        w.w.cells.clear();
        assert!(!raining_at(&w, p), "not raining");
    }

    #[test]
    fn tnt_next_to_fire_is_primed() {
        let mut w = Rec::new();
        let mut rs = Rs::default();
        let p = BlockPos::new(0, 64, 0);
        let q = BlockPos::new(1, 64, 0);
        w.w.put(below(p), s("netherrack"));
        w.w.put(below(q), s("stone"));
        w.w.put(q, s("tnt"));
        engine::set_block(&mut rs, &mut w, p, s("fire"), fl::ALL, UPDATE_LIMIT);
        for _ in 0..3000 {
            step(&mut rs, &mut w);
            if !w.primed.is_empty() {
                break;
            }
        }
        assert_eq!(w.primed, vec![q]);
        assert!(is_air(w.get(q)), "the tnt is removed when primed");
    }

    #[test]
    fn soul_fire_breaks_when_its_base_is_removed() {
        let mut w = Rec::new();
        let mut rs = Rs::default();
        let p = BlockPos::new(0, 64, 0);
        let base = below(p);
        w.w.put(base, s("soul_sand"));
        engine::set_block(&mut rs, &mut w, p, s("soul_fire"), fl::ALL, UPDATE_LIMIT);
        assert_eq!(w.get(p), s("soul_fire"));
        engine::set_block(&mut rs, &mut w, base, s("air"), fl::ALL, UPDATE_LIMIT);
        assert!(is_air(w.get(p)));
    }

    #[test]
    fn lava_ignites_flammable_block_above_its_neighbour() {
        let mut w = Rec::new();
        let mut rs = Rs::default();
        let mut rng = Rng::new(7);
        let p = BlockPos::new(0, 64, 0);
        let q = BlockPos::new(1, 64, 0);
        let above_q = q.offset(0, 1, 0);
        w.w.put(below(p), s("stone"));
        w.w.put(p, s("lava"));
        w.w.put(below(q), s("stone"));
        w.w.put(q, s("oak_planks"));
        let lava = w.get(p);
        for _ in 0..500 {
            super::super::fluid_random_tick(&mut rs, &mut w, lava, p, &mut rng);
            if is_fire(w.get(above_q)) {
                break;
            }
        }
        assert!(is_fire(w.get(above_q)));
    }

    #[test]
    fn fire_state_connects_to_flammable_sides() {
        let mut w = Rec::new();
        let p = BlockPos::new(0, 64, 0);
        // nothing under the fire: its sides carry the connections
        w.w.put(p.offset(0, 0, -1), s("oak_planks"));
        let st = fire_state_at(&w, p);
        assert!(is_fire(st));
        assert!(getb(st, Pr::North));
        assert!(!getb(st, Pr::South));
        assert!(!getb(st, Pr::Up));
        // a sturdy floor gives a plain fire
        w.w.put(below(p), s("stone"));
        assert!(!getb(fire_state_at(&w, p), Pr::North));
        w.w.put(below(p), s("soul_sand"));
        assert_eq!(fire_state_at(&w, p), s("soul_fire"));
    }

    #[test]
    fn can_be_lit_needs_something_to_stand_on() {
        let mut w = Rec::new();
        let p = BlockPos::new(0, 64, 0);
        assert!(!can_be_lit(&w, p, UP));
        w.w.put(below(p), s("stone"));
        assert!(can_be_lit(&w, p, UP));
        w.w.put(p, s("stone"));
        assert!(!can_be_lit(&w, p, UP), "not air");
    }
}
