//! Static per-block and per-state tables for the redstone engine, built once
//! from the block data: property handles (get/set a property by arithmetic on
//! the state id), block kinds, redstone conductors, face sturdiness, piston
//! push reactions and block entities.

use rine_data::blocks::flag;
use std::sync::OnceLock;

/// Directions as vanilla 3D data values (also the protocol face ids).
pub const DOWN: u8 = 0;
pub const UP: u8 = 1;
pub const NORTH: u8 = 2;
pub const SOUTH: u8 = 3;
pub const WEST: u8 = 4;
pub const EAST: u8 = 5;
/// `Direction.values()` order.
pub const ALL: [u8; 6] = [DOWN, UP, NORTH, SOUTH, WEST, EAST];
/// `Direction.Plane.HORIZONTAL`: north, east, south, west.
pub const HORIZONTAL: [u8; 4] = [NORTH, EAST, SOUTH, WEST];
/// `NeighborUpdater.UPDATE_ORDER`.
pub const UPDATE_ORDER: [u8; 6] = [WEST, EAST, DOWN, UP, NORTH, SOUTH];
/// `BlockBehaviour.UPDATE_SHAPE_ORDER`.
pub const SHAPE_ORDER: [u8; 6] = [WEST, EAST, NORTH, SOUTH, DOWN, UP];

#[inline]
pub fn opposite(d: u8) -> u8 {
    d ^ 1
}

#[inline]
pub fn axis(d: u8) -> u8 {
    d >> 1
}

/// `Direction.getClockWise()` around the Y axis (horizontal only).
pub fn clockwise(d: u8) -> u8 {
    match d {
        NORTH => EAST,
        EAST => SOUTH,
        SOUTH => WEST,
        _ => NORTH,
    }
}

pub fn counter_clockwise(d: u8) -> u8 {
    match d {
        NORTH => WEST,
        WEST => SOUTH,
        SOUTH => EAST,
        _ => NORTH,
    }
}

pub fn step(d: u8) -> (i32, i32, i32) {
    match d {
        DOWN => (0, -1, 0),
        UP => (0, 1, 0),
        NORTH => (0, 0, -1),
        SOUTH => (0, 0, 1),
        WEST => (-1, 0, 0),
        _ => (1, 0, 0),
    }
}

/// Block state properties the engine reads or writes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum P {
    Facing,
    Powered,
    Power,
    Face,
    Open,
    Half,
    Delay,
    Locked,
    Mode,
    Lit,
    Extended,
    Type,
    Short,
    Triggered,
    Enabled,
    North,
    East,
    South,
    West,
    Shape,
    Attached,
    Disarmed,
    Inverted,
    Hinge,
    HasBook,
    Note,
    Instrument,
    Waterlogged,
    Unstable,
    Attachment,
    InWall,
}

const NP: usize = 31;
const PROP_NAMES: [&str; NP] = [
    "facing",
    "powered",
    "power",
    "face",
    "open",
    "half",
    "delay",
    "locked",
    "mode",
    "lit",
    "extended",
    "type",
    "short",
    "triggered",
    "enabled",
    "north",
    "east",
    "south",
    "west",
    "shape",
    "attached",
    "disarmed",
    "inverted",
    "hinge",
    "has_book",
    "note",
    "instrument",
    "waterlogged",
    "unstable",
    "attachment",
    "in_wall",
];

const ALL_P: [P; NP] = [
    P::Facing,
    P::Powered,
    P::Power,
    P::Face,
    P::Open,
    P::Half,
    P::Delay,
    P::Locked,
    P::Mode,
    P::Lit,
    P::Extended,
    P::Type,
    P::Short,
    P::Triggered,
    P::Enabled,
    P::North,
    P::East,
    P::South,
    P::West,
    P::Shape,
    P::Attached,
    P::Disarmed,
    P::Inverted,
    P::Hinge,
    P::HasBook,
    P::Note,
    P::Instrument,
    P::Waterlogged,
    P::Unstable,
    P::Attachment,
    P::InWall,
];

/// Rail shapes in `RailShape` order.
pub const RAIL_SHAPES: [&str; 10] = [
    "north_south",
    "east_west",
    "ascending_east",
    "ascending_west",
    "ascending_north",
    "ascending_south",
    "south_east",
    "south_west",
    "north_west",
    "north_east",
];

/// Wire side connection codes (`RedstoneSide`).
pub const SIDE_NONE: u8 = 0;
pub const SIDE_SIDE: u8 = 1;
pub const SIDE_UP: u8 = 2;

/// Lever/button `face` codes.
pub const FACE_FLOOR: u8 = 0;
pub const FACE_WALL: u8 = 1;
pub const FACE_CEILING: u8 = 2;

fn dir_code(v: &str) -> Option<u8> {
    Some(match v {
        "down" => DOWN,
        "up" => UP,
        "north" => NORTH,
        "south" => SOUTH,
        "west" => WEST,
        "east" => EAST,
        _ => return None,
    })
}

/// Canonical code of a property value: booleans 0/1, integers as is,
/// directions as 3D data values, enums as listed per property.
fn code_of(p: P, v: &str, idx: usize) -> u8 {
    match v {
        "false" => return 0,
        "true" => return 1,
        _ => {}
    }
    if let Ok(i) = v.parse::<u8>() {
        return i;
    }
    match p {
        P::Facing => dir_code(v).unwrap_or(idx as u8),
        P::Face => match v {
            "floor" => FACE_FLOOR,
            "wall" => FACE_WALL,
            _ => FACE_CEILING,
        },
        P::Half => u8::from(matches!(v, "upper" | "top")),
        P::Mode => u8::from(v == "subtract"),
        P::North | P::East | P::South | P::West => match v {
            "none" => SIDE_NONE,
            "side" => SIDE_SIDE,
            _ => SIDE_UP,
        },
        P::Type => match v {
            "normal" | "single" | "bottom" => 0,
            "sticky" | "left" | "top" => 1,
            _ => 2,
        },
        P::Hinge => u8::from(v == "right"),
        P::Shape => RAIL_SHAPES.iter().position(|s| *s == v).unwrap_or(idx) as u8,
        P::Attachment => match v {
            "floor" => 0,
            "ceiling" => 1,
            "single_wall" => 2,
            _ => 3,
        },
        _ => idx as u8,
    }
}

/// One property of one block: value index = (local / stride) % n.
struct Ph {
    stride: u32,
    n: u32,
    to_code: [u8; 32],
    /// value index by code, 255 when the code is not a value.
    from_code: [u8; 32],
}

/// What a block does in the redstone engine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum K {
    None,
    Wire,
    Lever,
    /// Ticks the button stays pressed (stone 20, wood 30).
    Button(u8),
    /// Binary plate; `mobs` only counts living entities (stone plates).
    Plate {
        mobs: bool,
    },
    /// Weighted plate with its max entity count.
    Weighted(u8),
    Torch,
    WallTorch,
    Repeater,
    Comparator,
    Observer,
    RedstoneBlock,
    Lamp,
    Daylight,
    Piston {
        sticky: bool,
    },
    PistonHead,
    MovingPiston,
    Dispenser,
    Dropper,
    Hopper,
    Door {
        iron: bool,
    },
    Trapdoor {
        iron: bool,
    },
    FenceGate,
    Tnt,
    PoweredRail,
    ActivatorRail,
    DetectorRail,
    Rail,
    Note,
    Target,
    Lectern,
    TrappedChest,
    Bell,
    CopperBulb,
    TripwireHook,
    Tripwire,
}

/// `PushReaction`: Normal is vanilla `PUSH_PULL`, PushOnly is `PUSH`,
/// Destroy is `POPPED`, Block is `IMMOVEABLE`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Push {
    Normal,
    Destroy,
    Block,
    PushOnly,
}

/// Wood/metal family used for sounds (`BlockSetType`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Set {
    Wooden,
    Cherry,
    Bamboo,
    Nether,
    Iron,
    Copper,
    Stone,
    Gold,
}

pub mod sf {
    /// `isRedstoneConductor`.
    pub const CONDUCTOR: u8 = 1;
    /// Full collision box (`isCollisionShapeFullBlock`).
    pub const FULL: u8 = 2;
    pub const AIR: u8 = 4;
    pub const ANALOG: u8 = 8;
}

pub struct Tables {
    pub kind: Vec<K>,
    pub push: Vec<Push>,
    pub set: Vec<Set>,
    pub has_be: Vec<bool>,
    min_state: Vec<u32>,
    ph: Vec<Ph>,
    handles: Vec<[u16; NP]>,
    /// Per state: `sf` flags.
    pub sflags: Vec<u8>,
    /// Per state: face sturdiness, bit `face * 3 + t` for t = 0 full, 1 center, 2 rigid.
    pub sturdy: Vec<u32>,
    pub ids: Ids,
}

/// Block ids the engine compares against.
pub struct Ids {
    pub wire: u16,
    pub redstone_block: u16,
    pub moving_piston: u16,
    pub piston: u16,
    pub sticky_piston: u16,
    pub piston_head: u16,
    pub slime: u16,
    pub honey: u16,
    pub hopper: u16,
    pub comparator: u16,
    pub observer: u16,
    pub obsidian: u16,
    pub water: u16,
    pub tripwire: u16,
    pub tripwire_hook: u16,
    pub air_state: u16,
    pub water_state: u16,
    pub wire_default: u16,
    pub head_default: u16,
    pub moving_default: u16,
}

pub fn t() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(build)
}

/// Block id of a state.
#[inline]
pub fn block(st: u16) -> u16 {
    rine_data::get().blocks.states[st as usize].block
}

#[inline]
pub fn kind(st: u16) -> K {
    let t = t();
    t.kind[block(st) as usize]
}

/// Code of property `p` in `st` (0 when the block lacks it).
#[inline]
pub fn get(st: u16, p: P) -> u8 {
    let t = t();
    let b = block(st) as usize;
    let h = t.handles[b][p as usize];
    if h == u16::MAX {
        return 0;
    }
    let ph = &t.ph[h as usize];
    let idx = ((st as u32 - t.min_state[b]) / ph.stride) % ph.n;
    ph.to_code[idx as usize]
}

#[inline]
pub fn has(st: u16, p: P) -> bool {
    let t = t();
    t.handles[block(st) as usize][p as usize] != u16::MAX
}

#[inline]
pub fn on(st: u16, p: P) -> bool {
    get(st, p) != 0
}

/// `st` with property `p` set to `code`; unchanged when not applicable.
#[inline]
pub fn set(st: u16, p: P, code: u8) -> u16 {
    let t = t();
    let b = block(st) as usize;
    let h = t.handles[b][p as usize];
    if h == u16::MAX || code >= 32 {
        return st;
    }
    let ph = &t.ph[h as usize];
    let new = ph.from_code[code as usize];
    if new == 255 {
        return st;
    }
    let local = st as u32 - t.min_state[b];
    let cur = (local / ph.stride) % ph.n;
    (t.min_state[b] + local - cur * ph.stride + new as u32 * ph.stride) as u16
}

#[inline]
pub fn setb(st: u16, p: P, v: bool) -> u16 {
    set(st, p, v as u8)
}

#[inline]
pub fn is_air(st: u16) -> bool {
    t().sflags[st as usize] & sf::AIR != 0
}

#[inline]
pub fn conductor(st: u16) -> bool {
    t().sflags[st as usize] & sf::CONDUCTOR != 0
}

#[inline]
pub fn full_block(st: u16) -> bool {
    t().sflags[st as usize] & sf::FULL != 0
}

#[inline]
pub fn has_analog(st: u16) -> bool {
    t().sflags[st as usize] & sf::ANALOG != 0
}

/// `isFaceSturdy(face, FULL)`.
#[inline]
pub fn sturdy(st: u16, face: u8) -> bool {
    t().sturdy[st as usize] & (1 << (face * 3)) != 0
}

/// `isFaceSturdy(face, CENTER)` (`canSupportCenter`).
#[inline]
pub fn sturdy_center(st: u16, face: u8) -> bool {
    t().sturdy[st as usize] & (1 << (face * 3 + 1)) != 0
}

/// `isFaceSturdy(face, RIGID)` (`canSupportRigidBlock` for the top face).
#[inline]
pub fn sturdy_rigid(st: u16, face: u8) -> bool {
    t().sturdy[st as usize] & (1 << (face * 3 + 2)) != 0
}

pub fn block_name(st: u16) -> &'static str {
    let n = rine_data::get().blocks.block_of(st).name.as_str();
    n.strip_prefix("minecraft:").unwrap_or(n)
}

/// Blocks whose `isRedstoneConductor` is `never` although their collision is
/// a full cube (Blocks.java: leaves, glass, pistons...).
fn never_conducts(n: &str) -> bool {
    n.ends_with("_leaves")
        || n == "glass"
        || n.ends_with("stained_glass")
        || n == "tinted_glass"
        || n.ends_with("copper_grate")
        || n.ends_with("copper_bulb")
        || matches!(
            n,
            "moving_piston"
                | "tnt"
                | "ice"
                | "frosted_ice"
                | "glowstone"
                | "repeater"
                | "beacon"
                | "redstone_block"
                | "sea_lantern"
                | "chorus_flower"
                | "observer"
                | "bamboo"
                | "scaffolding"
                | "powder_snow"
                | "pointed_dripstone"
                | "sulfur_spike"
                | "piston"
                | "sticky_piston"
                | "piston_head"
        )
}

fn always_conducts(n: &str) -> bool {
    matches!(n, "soul_sand" | "mud")
}

fn kind_of(n: &str) -> K {
    match n {
        "redstone_wire" => return K::Wire,
        "lever" => return K::Lever,
        "redstone_torch" => return K::Torch,
        "redstone_wall_torch" => return K::WallTorch,
        "repeater" => return K::Repeater,
        "comparator" => return K::Comparator,
        "observer" => return K::Observer,
        "redstone_block" => return K::RedstoneBlock,
        "redstone_lamp" => return K::Lamp,
        "daylight_detector" => return K::Daylight,
        "piston" => return K::Piston { sticky: false },
        "sticky_piston" => return K::Piston { sticky: true },
        "piston_head" => return K::PistonHead,
        "moving_piston" => return K::MovingPiston,
        "dispenser" => return K::Dispenser,
        "dropper" => return K::Dropper,
        "hopper" => return K::Hopper,
        "tnt" => return K::Tnt,
        "powered_rail" => return K::PoweredRail,
        "activator_rail" => return K::ActivatorRail,
        "detector_rail" => return K::DetectorRail,
        "rail" => return K::Rail,
        "note_block" => return K::Note,
        "target" => return K::Target,
        "lectern" => return K::Lectern,
        "trapped_chest" => return K::TrappedChest,
        "bell" => return K::Bell,
        "tripwire_hook" => return K::TripwireHook,
        "tripwire" => return K::Tripwire,
        "light_weighted_pressure_plate" => return K::Weighted(15),
        "heavy_weighted_pressure_plate" => return K::Weighted(150),
        _ => {}
    }
    if n.ends_with("_button") {
        return K::Button(if n == "stone_button" || n == "polished_blackstone_button" { 20 } else { 30 });
    }
    if n.ends_with("_pressure_plate") {
        return K::Plate { mobs: n == "stone_pressure_plate" || n == "polished_blackstone_pressure_plate" };
    }
    if n.ends_with("_trapdoor") {
        return K::Trapdoor { iron: n == "iron_trapdoor" };
    }
    if n.ends_with("_door") {
        return K::Door { iron: n == "iron_door" };
    }
    if n.ends_with("_fence_gate") {
        return K::FenceGate;
    }
    if n.ends_with("copper_bulb") {
        return K::CopperBulb;
    }
    K::None
}

fn set_of(n: &str) -> Set {
    if n.starts_with("iron_") {
        Set::Iron
    } else if n.contains("copper") {
        Set::Copper
    } else if n.starts_with("cherry_") {
        Set::Cherry
    } else if n.starts_with("bamboo_") {
        Set::Bamboo
    } else if n.starts_with("crimson_") || n.starts_with("warped_") {
        Set::Nether
    } else if n.starts_with("stone_") || n.starts_with("polished_blackstone_") || n.starts_with("heavy_weighted") {
        Set::Stone
    } else if n.starts_with("light_weighted") {
        Set::Gold
    } else {
        Set::Wooden
    }
}

/// `PushReaction` from Blocks.java (`.pushReaction(...)` and helper properties).
fn push_of(n: &str) -> Push {
    const DESTROY: &[&str] = &[
        "allium",
        "amethyst_cluster",
        "attached_melon_stem",
        "attached_pumpkin_stem",
        "azalea",
        "azure_bluet",
        "bamboo",
        "bamboo_sapling",
        "beetroots",
        "bell",
        "big_dripleaf",
        "big_dripleaf_stem",
        "blue_orchid",
        "brown_mushroom",
        "bubble_column",
        "budding_amethyst",
        "bush",
        "cactus",
        "cactus_flower",
        "cake",
        "carrots",
        "carved_pumpkin",
        "cave_vines",
        "cave_vines_plant",
        "chorus_flower",
        "chorus_plant",
        "closed_eyeblossom",
        "cobweb",
        "cocoa",
        "comparator",
        "copper_golem_statue",
        "copper_lantern",
        "copper_torch",
        "copper_wall_torch",
        "cornflower",
        "crimson_fungus",
        "crimson_roots",
        "dandelion",
        "dead_bush",
        "decorated_pot",
        "dragon_egg",
        "fern",
        "fire",
        "firefly_bush",
        "flowering_azalea",
        "frogspawn",
        "glow_lichen",
        "golden_dandelion",
        "hanging_roots",
        "jack_o_lantern",
        "kelp",
        "kelp_plant",
        "ladder",
        "lantern",
        "large_fern",
        "lava",
        "leaf_litter",
        "lever",
        "lilac",
        "lily_of_the_valley",
        "lily_pad",
        "mangrove_propagule",
        "melon",
        "melon_stem",
        "moss_block",
        "moss_carpet",
        "nether_sprouts",
        "nether_wart",
        "open_eyeblossom",
        "oxeye_daisy",
        "pale_hanging_moss",
        "pale_moss_block",
        "pale_moss_carpet",
        "peony",
        "pink_petals",
        "pitcher_crop",
        "pitcher_plant",
        "pointed_dripstone",
        "poppy",
        "potatoes",
        "pumpkin",
        "pumpkin_stem",
        "redstone_torch",
        "redstone_wall_torch",
        "redstone_wire",
        "red_mushroom",
        "red_shrub",
        "repeater",
        "resin_clump",
        "rose_bush",
        "scaffolding",
        "sculk_vein",
        "seagrass",
        "sea_pickle",
        "shelf_mushroom",
        "short_dry_grass",
        "short_grass",
        "small_dripleaf",
        "snow",
        "soul_fire",
        "soul_lantern",
        "soul_torch",
        "soul_wall_torch",
        "spore_blossom",
        "structure_void",
        "sugar_cane",
        "sulfur_spike",
        "sunflower",
        "suspicious_gravel",
        "suspicious_sand",
        "sweet_berry_bush",
        "tall_dry_grass",
        "tall_grass",
        "tall_seagrass",
        "torch",
        "torchflower",
        "torchflower_crop",
        "tripwire",
        "tripwire_hook",
        "turtle_egg",
        "twisting_vines",
        "twisting_vines_plant",
        "vine",
        "wall_torch",
        "warped_fungus",
        "warped_roots",
        "water",
        "weeping_vines",
        "weeping_vines_plant",
        "wheat",
        "wildflowers",
        "wither_rose",
        "flower_pot",
    ];
    const BLOCK: &[&str] = &[
        "anvil",
        "barrier",
        "chipped_anvil",
        "crying_obsidian",
        "damaged_anvil",
        "end_gateway",
        "end_portal",
        "grindstone",
        "lodestone",
        "moving_piston",
        "nether_portal",
        "obsidian",
        "piston_head",
        "reinforced_deepslate",
        "respawn_anchor",
    ];
    if n.ends_with("glazed_terracotta") {
        return Push::PushOnly;
    }
    if BLOCK.contains(&n) {
        return Push::Block;
    }
    if DESTROY.contains(&n)
        || n.ends_with("_door")
        || n.ends_with("_pressure_plate")
        || n.ends_with("_sapling")
        || n.ends_with("_tulip")
        || n.ends_with("_button")
        || n.ends_with("candle")
        || n.ends_with("candle_cake")
        || n.starts_with("potted_")
        || n.ends_with("_leaves")
        || n.ends_with("shulker_box")
        || n.ends_with("_bed")
        || n.ends_with("_head")
        || n.ends_with("_skull")
        || n.contains("coral")
    {
        return Push::Destroy;
    }
    Push::Normal
}

/// Blocks that carry a block entity (`hasBlockEntity`): pistons cannot move them.
fn has_block_entity(n: &str) -> bool {
    n.ends_with("chest")
        || n.ends_with("shulker_box")
        || n.ends_with("_sign")
        || n.ends_with("_banner")
        || n.ends_with("_bed")
        || n.ends_with("_head")
        || n.ends_with("_skull")
        || n.ends_with("command_block")
        || n.ends_with("furnace")
        || n.ends_with("campfire")
        || n.ends_with("sculk_sensor")
        || n.ends_with("_shelf")
        || n.ends_with("copper_golem_statue")
        || matches!(
            n,
            "barrel"
                | "smoker"
                | "hopper"
                | "dispenser"
                | "dropper"
                | "crafter"
                | "brewing_stand"
                | "enchanting_table"
                | "beacon"
                | "jukebox"
                | "lectern"
                | "spawner"
                | "trial_spawner"
                | "vault"
                | "comparator"
                | "daylight_detector"
                | "bell"
                | "beehive"
                | "bee_nest"
                | "conduit"
                | "end_gateway"
                | "end_portal"
                | "sculk_catalyst"
                | "sculk_shrieker"
                | "chiseled_bookshelf"
                | "decorated_pot"
                | "suspicious_sand"
                | "suspicious_gravel"
                | "structure_block"
                | "jigsaw"
                | "moving_piston"
                | "creaking_heart"
                | "test_block"
                | "test_instance_block"
        )
}

/// `hasAnalogOutputSignal`.
fn analog(n: &str) -> bool {
    n.ends_with("chest")
        || n.ends_with("shulker_box")
        || n.ends_with("furnace")
        || n.ends_with("cauldron")
        || n.ends_with("_shelf")
        || matches!(
            n,
            "barrel"
                | "smoker"
                | "hopper"
                | "dispenser"
                | "dropper"
                | "crafter"
                | "brewing_stand"
                | "jukebox"
                | "composter"
                | "cake"
                | "end_portal_frame"
                | "respawn_anchor"
                | "beehive"
                | "bee_nest"
                | "chiseled_bookshelf"
                | "lectern"
                | "decorated_pot"
                | "sculk_sensor"
                | "calibrated_sculk_sensor"
        ) && n != "ender_chest"
}

/// Face coverage of a collision shape on side `d`, sampled on a 16x16 grid:
/// (full face, center 2x2 column, outer ring of width 2).
fn face_cover(boxes: &[[f32; 6]], d: u8) -> (bool, bool, bool) {
    const E: f32 = 1e-4;
    let ax = match axis(d) {
        0 => 1, // down/up: y
        1 => 2, // north/south: z
        _ => 0, // west/east: x
    };
    let (u, v) = match ax {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    let positive = matches!(d, UP | SOUTH | EAST);
    let touches = |b: &[f32; 6]| if positive { b[ax + 3] >= 1.0 - E } else { b[ax] <= E };
    let mut full = true;
    let mut center = true;
    let mut rigid = true;
    for i in 0..16 {
        for j in 0..16 {
            let (cu, cv) = ((i as f32 + 0.5) / 16.0, (j as f32 + 0.5) / 16.0);
            let covered = boxes.iter().any(|b| touches(b) && b[u] <= cu && cu <= b[u + 3] && b[v] <= cv && cv <= b[v + 3]);
            if !covered {
                full = false;
                if (7..9).contains(&i) && (7..9).contains(&j) {
                    center = false;
                }
                if !((2..14).contains(&i) && (2..14).contains(&j)) {
                    rigid = false;
                }
            }
        }
    }
    (full, center, rigid)
}

fn build() -> Tables {
    let d = rine_data::get();
    let b = &d.blocks;
    let nb = b.blocks.len();
    let mut tb = Tables {
        kind: Vec::with_capacity(nb),
        push: Vec::with_capacity(nb),
        set: Vec::with_capacity(nb),
        has_be: Vec::with_capacity(nb),
        min_state: Vec::with_capacity(nb),
        ph: Vec::new(),
        handles: Vec::with_capacity(nb),
        sflags: vec![0; b.states.len()],
        sturdy: vec![0; b.states.len()],
        ids: Ids {
            wire: 0,
            redstone_block: 0,
            moving_piston: 0,
            piston: 0,
            sticky_piston: 0,
            piston_head: 0,
            slime: 0,
            honey: 0,
            hopper: 0,
            comparator: 0,
            observer: 0,
            obsidian: 0,
            water: 0,
            tripwire: 0,
            tripwire_hook: 0,
            air_state: 0,
            water_state: 0,
            wire_default: 0,
            head_default: 0,
            moving_default: 0,
        },
    };
    let mut never = vec![false; nb];
    let mut always = vec![false; nb];
    let mut analog_b = vec![false; nb];
    let mut leaves = vec![false; nb];
    for bi in &b.blocks {
        let n = bi.name.strip_prefix("minecraft:").unwrap_or(&bi.name);
        tb.kind.push(kind_of(n));
        tb.push.push(push_of(n));
        tb.set.push(set_of(n));
        tb.has_be.push(has_block_entity(n));
        tb.min_state.push(bi.min_state);
        never[bi.id as usize] = never_conducts(n);
        always[bi.id as usize] = always_conducts(n);
        analog_b[bi.id as usize] = analog(n);
        leaves[bi.id as usize] = n.ends_with("_leaves");
        let mut h = [u16::MAX; NP];
        for (pi, pname) in PROP_NAMES.iter().enumerate() {
            let Some(prop) = bi.props.iter().find(|p| p.name == *pname) else { continue };
            let p = ALL_P[pi];
            let mut ph = Ph { stride: prop.stride, n: prop.values.len() as u32, to_code: [0; 32], from_code: [255; 32] };
            for (vi, v) in prop.values.iter().enumerate().take(32) {
                let c = code_of(p, v, vi);
                ph.to_code[vi] = c;
                if (c as usize) < 32 {
                    ph.from_code[c as usize] = vi as u8;
                }
            }
            h[pi] = tb.ph.len() as u16;
            tb.ph.push(ph);
        }
        tb.handles.push(h);
    }
    // face sturdiness per distinct collision shape
    let mut shape_mask = vec![u32::MAX; b.shapes.len()];
    for (s, info) in b.states.iter().enumerate() {
        let bid = info.block as usize;
        let mut f = 0u8;
        if info.flags & flag::AIR != 0 {
            f |= sf::AIR;
        }
        let fullc = info.flags & flag::FULL_CUBE != 0;
        if fullc {
            f |= sf::FULL;
        }
        if (fullc && !never[bid]) || always[bid] {
            f |= sf::CONDUCTOR;
        }
        if analog_b[bid] {
            f |= sf::ANALOG;
        }
        tb.sflags[s] = f;
        if leaves[bid] {
            continue;
        }
        let si = info.shape as usize;
        if shape_mask[si] == u32::MAX {
            let boxes = &b.shapes[si];
            let mut m = 0u32;
            if !boxes.is_empty() {
                for face in ALL {
                    let (full, center, rigid) = face_cover(boxes, face);
                    if full {
                        m |= 1 << (face * 3);
                    }
                    if center {
                        m |= 1 << (face * 3 + 1);
                    }
                    if rigid {
                        m |= 1 << (face * 3 + 2);
                    }
                }
            }
            shape_mask[si] = m;
        }
        tb.sturdy[s] = shape_mask[si];
    }
    let id = |n: &str| b.block(n).map(|x| x.id as u16).unwrap_or(0);
    tb.ids.wire = id("redstone_wire");
    tb.ids.redstone_block = id("redstone_block");
    tb.ids.moving_piston = id("moving_piston");
    tb.ids.piston = id("piston");
    tb.ids.sticky_piston = id("sticky_piston");
    tb.ids.piston_head = id("piston_head");
    tb.ids.slime = id("slime_block");
    tb.ids.honey = id("honey_block");
    tb.ids.hopper = id("hopper");
    tb.ids.comparator = id("comparator");
    tb.ids.observer = id("observer");
    tb.ids.obsidian = id("obsidian");
    tb.ids.water = id("water");
    tb.ids.tripwire = id("tripwire");
    tb.ids.tripwire_hook = id("tripwire_hook");
    tb.ids.air_state = b.default_state("air");
    tb.ids.water_state = b.default_state("water");
    tb.ids.wire_default = b.default_state("redstone_wire");
    tb.ids.head_default = b.default_state("piston_head");
    tb.ids.moving_default = b.default_state("moving_piston");
    tb
}
