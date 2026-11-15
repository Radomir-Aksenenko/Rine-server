//! Block placement rules and block interaction.
//!
//! Placement follows vanilla `getStateForPlacement` closely enough for the
//! common blocks. Face ids are the protocol directions: 0 down, 1 up,
//! 2 north(-z), 3 south(+z), 4 west(-x), 5 east(+x).
//! Player yaw: 0 = south, 90 = west, 180 = north, 270 = east.

use super::entity;
use super::items;
use super::player::Stack;
use super::Game;
use rine_data::blocks::{flag, Blocks};
use rine_protocol::BlockPos;

/// Horizontal facing names indexed by `horizontal_dir`.
const H_NAMES: [&str; 4] = ["south", "west", "north", "east"];
/// Horizontal face ids (north, south, west, east).
const HORIZONTAL: [u8; 4] = [2, 3, 4, 5];
/// Two-block plants: `half=lower` at the placed block, `upper` above.
const TALL_PLANTS: [&str; 7] = ["tall_grass", "large_fern", "sunflower", "lilac", "rose_bush", "peony", "pitcher_plant"];
/// Single-block plants that need soil below.
const SMALL_PLANTS: [&str; 19] = [
    "short_grass",
    "fern",
    "dead_bush",
    "dandelion",
    "poppy",
    "blue_orchid",
    "allium",
    "azure_bluet",
    "red_tulip",
    "orange_tulip",
    "white_tulip",
    "pink_tulip",
    "oxeye_daisy",
    "cornflower",
    "lily_of_the_valley",
    "wither_rose",
    "torchflower",
    "open_eyeblossom",
    "closed_eyeblossom",
];

fn blocks() -> &'static Blocks {
    &rine_data::get().blocks
}

fn short(name: &str) -> &str {
    name.strip_prefix("minecraft:").unwrap_or(name)
}

/// Player's horizontal facing index: 0 south, 1 west, 2 north, 3 east.
pub fn horizontal_dir(yaw: f32) -> u8 {
    ((yaw / 90.0).round() as i32 & 3) as u8
}

/// Vanilla `Mth.floor((180 + yaw) * 16 / 360 + 0.5) & 15` for standing signs, banners and heads.
pub fn rotation16(yaw: f32) -> i32 {
    (((yaw + 180.0) * 16.0 / 360.0 + 0.5).floor() as i32).rem_euclid(16)
}

/// Axis of a block placed against `face`.
pub fn axis_for_face(face: u8) -> &'static str {
    match face {
        0 | 1 => "y",
        2 | 3 => "z",
        _ => "x",
    }
}

/// Opposite protocol direction.
pub fn opposite(dir: u8) -> u8 {
    dir ^ 1
}

fn face_name(dir: u8) -> &'static str {
    ["down", "up", "north", "south", "west", "east"][(dir % 6) as usize]
}

fn h_name(h: u8) -> &'static str {
    H_NAMES[(h & 3) as usize]
}

/// Protocol direction (2..5) of a horizontal facing index.
fn h_to_dir(h: u8) -> u8 {
    [3, 4, 2, 5][(h & 3) as usize]
}

fn dir_of_name(name: &str) -> Option<u8> {
    match name {
        "north" => Some(2),
        "south" => Some(3),
        "west" => Some(4),
        "east" => Some(5),
        _ => None,
    }
}

/// Six-way facing for blocks that face the player (pistons, dispensers, observers, barrels).
/// Looking more than 45 degrees up or down picks up or down, otherwise the horizontal
/// direction the player looks at, flipped so the block faces the player.
pub fn look_facing(yaw: f32, pitch: f32) -> u8 {
    if pitch > 45.0 {
        1
    } else if pitch < -45.0 {
        0
    } else {
        opposite(h_to_dir(horizontal_dir(yaw)))
    }
}

/// True when the clicked side puts the new block in the upper half (or the underside was clicked).
pub fn top_half(face: u8, cursor_y: f32) -> bool {
    face == 0 || (face >= 2 && cursor_y > 0.5)
}

fn is_soil(name: &str) -> bool {
    matches!(name, "grass_block" | "dirt" | "coarse_dirt" | "podzol" | "rooted_dirt" | "moss_block" | "farmland" | "mud")
}

fn is_plant(name: &str) -> bool {
    TALL_PLANTS.contains(&name) || SMALL_PLANTS.contains(&name) || name.ends_with("_sapling")
}

/// Wall variant of a standing banner, head or skull (`oak_banner` -> `oak_wall_banner`).
fn wall_variant(name: &str) -> Option<String> {
    for (suffix, wall) in [("_banner", "_wall_banner"), ("_head", "_wall_head"), ("_skull", "_wall_skull")] {
        if let Some(stem) = name.strip_suffix(suffix) {
            return Some(format!("{stem}{wall}"));
        }
    }
    None
}

/// Blocks that face the player (the front points back toward the placer).
fn faces_player(name: &str) -> bool {
    matches!(
        name,
        "chest" | "trapped_chest" | "ender_chest" | "furnace" | "blast_furnace" | "smoker" | "lectern" | "carved_pumpkin" | "jack_o_lantern" | "beehive" | "loom" | "stonecutter" | "crafter"
    ) || name.ends_with("glazed_terracotta")
        || name.ends_with("anvil")
}

/// Short name of the block at `p` (without the `minecraft:` prefix).
fn name_at(g: &Game, p: BlockPos) -> &'static str {
    short(&blocks().block_of(g.world.get_block(p.x, p.y, p.z)).name)
}

fn state_at(g: &Game, p: BlockPos) -> u16 {
    g.world.get_block(p.x, p.y, p.z)
}

/// Solid block that can carry a placed block (motion-blocking or full cube).
fn solid(g: &Game, p: BlockPos) -> bool {
    let st = state_at(g, p);
    let b = blocks();
    b.has(st, flag::SOLID) || b.has(st, flag::FULL_CUBE)
}

/// Air or a replaceable block (grass, snow layer, flowers, water-like fluids).
fn free(g: &Game, p: BlockPos) -> bool {
    let st = state_at(g, p);
    let b = blocks();
    b.is_air(st) || b.has(st, flag::REPLACEABLE)
}

fn water(g: &Game, p: BlockPos) -> bool {
    blocks().has(state_at(g, p), flag::WATER)
}

/// Set a property if the block has it (and the value is valid), otherwise return the state unchanged.
fn set(b: &Blocks, st: u16, prop: &str, val: &str) -> u16 {
    b.with_prop(st, prop, val).unwrap_or(st)
}

fn has_prop(b: &Blocks, st: u16, prop: &str) -> bool {
    b.block_of(st).props.iter().any(|p| p.name == prop)
}

/// Plants and crops need suitable ground; cactus and sugar cane have extra rules.
fn ground_ok(g: &Game, pos: BlockPos, name: &str) -> bool {
    let below_pos = pos.offset(0, -1, 0);
    let below = name_at(g, below_pos);
    match name {
        "cactus" => matches!(below, "cactus" | "sand" | "red_sand") && !HORIZONTAL.iter().any(|&d| solid(g, pos.relative(d))),
        "sugar_cane" => {
            below == "sugar_cane"
                || (matches!(below, "sand" | "red_sand" | "dirt" | "grass_block" | "podzol" | "coarse_dirt" | "mud")
                    && HORIZONTAL.iter().any(|&d| water(g, below_pos.relative(d))))
        }
        "dead_bush" => is_soil(below) || matches!(below, "sand" | "red_sand") || below.ends_with("terracotta"),
        _ => is_soil(below),
    }
}

/// Final state for a block placed at `pos` (the target, already checked for air/replaceable).
/// `face` is the clicked face of the neighbouring block, so the support is `pos.relative(opposite(face))`.
/// Returns None when placement is invalid.
pub fn placement_state(g: &Game, default: u16, pos: BlockPos, face: u8, cursor: (f32, f32, f32), yaw: f32, pitch: f32) -> Option<u16> {
    let st = placed(g, default, pos, face, cursor, yaw, pitch)?;
    Some(set(blocks(), st, "waterlogged", "false"))
}

fn placed(g: &Game, default: u16, pos: BlockPos, face: u8, cursor: (f32, f32, f32), yaw: f32, pitch: f32) -> Option<u16> {
    let b = blocks();
    let default = set(b, default, "axis", axis_for_face(face));
    let name = short(&b.block_of(default).name);
    let h = horizontal_dir(yaw);
    let away = h_name(h); // block faces away from the player (player's look direction)
    let toward = h_name((h + 2) & 3); // block faces the player
    let support = pos.relative(opposite(face));
    let side = face >= 2;
    let top = top_half(face, cursor.1);

    // plants, cactus, sugar cane
    if is_plant(name) {
        if !ground_ok(g, pos, name) {
            return None;
        }
        if TALL_PLANTS.contains(&name) {
            if !free(g, pos.offset(0, 1, 0)) {
                return None;
            }
            return Some(set(b, default, "half", "lower"));
        }
        return Some(default);
    }
    if name == "cactus" || name == "sugar_cane" {
        return ground_ok(g, pos, name).then_some(default);
    }

    // torches: floor on top, wall torch on the side, never on the underside
    if name.ends_with("torch") {
        let prefix = name.strip_suffix("torch")?;
        return match face {
            0 => None,
            1 => solid(g, support).then_some(default),
            _ => {
                let wall = b.block(&format!("{prefix}wall_torch"))?;
                solid(g, support).then(|| set(b, wall.default_state as u16, "facing", face_name(face)))
            }
        };
    }

    if name == "ladder" {
        if !side {
            return None;
        }
        return solid(g, support).then(|| set(b, default, "facing", face_name(face)));
    }

    if name == "lantern" || name == "soul_lantern" {
        let hanging = face == 0;
        let attach = if hanging { pos.offset(0, 1, 0) } else { pos.offset(0, -1, 0) };
        return solid(g, attach).then(|| set(b, default, "hanging", if hanging { "true" } else { "false" }));
    }

    // rails and redstone wire need a block below
    if name.ends_with("rail") || name == "redstone_wire" {
        return solid(g, pos.offset(0, -1, 0)).then_some(default);
    }

    // hanging signs: ceiling under the clicked underside, wall variant on the side
    if let Some(wood) = name.strip_suffix("_hanging_sign") {
        return match face {
            1 => None,
            0 => solid(g, support).then(|| set(b, default, "rotation", &rotation16(yaw).to_string())),
            _ => {
                let w = b.block(&format!("{wood}_wall_hanging_sign"))?;
                solid(g, support).then(|| set(b, w.default_state as u16, "facing", face_name(face)))
            }
        };
    }

    // standing and wall signs
    if let Some(wood) = name.strip_suffix("_sign") {
        return match face {
            0 => None,
            1 => solid(g, support).then(|| set(b, default, "rotation", &rotation16(yaw).to_string())),
            _ => {
                let w = b.block(&format!("{wood}_wall_sign"))?;
                solid(g, support).then(|| set(b, w.default_state as u16, "facing", face_name(face)))
            }
        };
    }

    // banners, heads and skulls: standing (rotation) or wall variant
    if let Some(wall) = wall_variant(name) {
        let grounded = name.ends_with("_banner");
        return match face {
            0 => None,
            1 => (!grounded || solid(g, support)).then(|| set(b, default, "rotation", &rotation16(yaw).to_string())),
            _ => {
                let w = b.block(&wall)?;
                solid(g, support).then(|| set(b, w.default_state as u16, "facing", face_name(face)))
            }
        };
    }

    // buttons, levers, grindstones: attached by "face" (floor / ceiling / wall)
    if has_prop(b, default, "face") {
        let st = match face {
            1 => set(b, set(b, default, "face", "floor"), "facing", away),
            0 => set(b, set(b, default, "face", "ceiling"), "facing", away),
            _ => set(b, set(b, default, "face", "wall"), "facing", face_name(face)),
        };
        return solid(g, support).then_some(st);
    }

    if name.ends_with("_stairs") {
        let st = set(b, default, "facing", away);
        return Some(set(b, st, "half", if top { "top" } else { "bottom" }));
    }

    if name.ends_with("_slab") {
        // A slab of the same kind already at pos: complete it into a double slab.
        // interact.rs currently targets the adjacent air block, so this only applies when the caller passes the slab itself.
        let cur = state_at(g, pos);
        if b.block_of(cur).id == b.block_of(default).id {
            let half = b.get_prop(cur, "type");
            if (face == 1 && half == Some("bottom")) || (face == 0 && half == Some("top")) {
                return Some(set(b, cur, "type", "double"));
            }
            return None;
        }
        return Some(set(b, default, "type", if top { "top" } else { "bottom" }));
    }

    // trapdoors: side clicks attach to that face, top/bottom clicks face the player
    if name.ends_with("trapdoor") {
        let facing = if side { face_name(face) } else { toward };
        let st = set(b, default, "facing", facing);
        return Some(set(b, st, "half", if top { "top" } else { "bottom" }));
    }

    if name.ends_with("_door") {
        if !free(g, pos.offset(0, 1, 0)) {
            return None;
        }
        let st = set(b, default, "facing", away);
        return Some(set(b, st, "half", "lower"));
    }

    if name.ends_with("_bed") {
        // head goes one block in the facing direction
        if !free(g, pos.relative(h_to_dir(h))) {
            return None;
        }
        let st = set(b, default, "facing", away);
        return Some(set(b, st, "part", "foot"));
    }

    if name.ends_with("_fence_gate") {
        return Some(set(b, default, "facing", away));
    }

    // repeaters and comparators: input side toward the placer, output away (vanilla DiodeBlock)
    if name == "repeater" || name == "comparator" {
        return Some(set(b, default, "facing", toward));
    }

    // six-way facing
    if matches!(name, "piston" | "sticky_piston" | "dispenser" | "dropper" | "observer" | "barrel") {
        return Some(set(b, default, "facing", face_name(look_facing(yaw, pitch))));
    }
    if matches!(name, "end_rod" | "lightning_rod") || name.ends_with("amethyst_cluster") || name.ends_with("amethyst_bud") {
        return Some(set(b, default, "facing", face_name(face)));
    }
    if name == "hopper" {
        // hoppers point away from the clicked face, never up
        let dir = if side { opposite(face) } else { 0 };
        return Some(set(b, default, "facing", face_name(dir)));
    }

    if faces_player(name) {
        return Some(set(b, default, "facing", toward));
    }

    Some(default)
}

/// Second halves and other parts placed after the main block.
pub fn after_place(g: &mut Game, pos: BlockPos, state: u16) {
    let b = blocks();
    // doors and tall plants: upper half above
    if b.get_prop(state, "half") == Some("lower") {
        if let Some(upper) = b.with_prop(state, "half", "upper") {
            g.world.set_block(pos.offset(0, 1, 0), upper);
        }
    }
    // beds: head one block in the facing direction
    if b.get_prop(state, "part") == Some("foot") {
        if let Some(dir) = b.get_prop(state, "facing").and_then(dir_of_name) {
            if let Some(head) = b.with_prop(state, "part", "head") {
                g.world.set_block(pos.relative(dir), head);
            }
        }
    }
}

/// Right click on a block. Returns true when the click was handled (no placement follows).
pub fn interact_block(g: &mut Game, _slot: usize, pos: BlockPos, state: u16, held: Stack) -> bool {
    let b = blocks();
    let name = short(&b.block_of(state).name);

    // levers, buttons, diodes, wire, doors, trapdoors, fence gates, note blocks
    if super::redstone::use_block(g, _slot, pos, state) {
        return true;
    }

    if name == "tnt" {
        let igniters = [items::id("flint_and_steel"), items::id("fire_charge")];
        if !igniters.contains(&Some(held.item)) {
            return false;
        }
        g.world.set_block(pos, 0);
        entity::spawn_tnt(g, [pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5], 80);
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_dir_cardinals() {
        assert_eq!(horizontal_dir(0.0), 0); // south
        assert_eq!(horizontal_dir(90.0), 1); // west
        assert_eq!(horizontal_dir(180.0), 2); // north
        assert_eq!(horizontal_dir(-90.0), 3); // east
        assert_eq!(horizontal_dir(270.0), 3); // east
        assert_eq!(horizontal_dir(-180.0), 2); // north
        assert_eq!(horizontal_dir(359.0), 0); // south
    }

    #[test]
    fn rotation16_values() {
        assert_eq!(rotation16(0.0), 8);
        assert_eq!(rotation16(180.0), 0);
        assert_eq!(rotation16(-180.0), 0);
        assert_eq!(rotation16(90.0), 12);
        assert_eq!(rotation16(-90.0), 4);
    }

    #[test]
    fn axis_follows_face() {
        assert_eq!(axis_for_face(0), "y");
        assert_eq!(axis_for_face(1), "y");
        assert_eq!(axis_for_face(2), "z");
        assert_eq!(axis_for_face(3), "z");
        assert_eq!(axis_for_face(4), "x");
        assert_eq!(axis_for_face(5), "x");
    }

    #[test]
    fn opposite_pairs() {
        assert_eq!(opposite(0), 1);
        assert_eq!(opposite(2), 3);
        assert_eq!(opposite(5), 4);
    }

    #[test]
    fn look_facing_faces_player() {
        assert_eq!(look_facing(0.0, 0.0), 2); // looking south, block faces north
        assert_eq!(look_facing(90.0, 0.0), 5); // looking west, block faces east
        assert_eq!(look_facing(0.0, 60.0), 1); // looking down, block faces up
        assert_eq!(look_facing(0.0, -60.0), 0); // looking up, block faces down
    }

    #[test]
    fn top_half_rules() {
        assert!(top_half(0, 0.1));
        assert!(!top_half(1, 0.9));
        assert!(top_half(2, 0.6));
        assert!(!top_half(2, 0.4));
    }

    #[test]
    fn wall_variant_names() {
        assert_eq!(wall_variant("skeleton_skull").as_deref(), Some("skeleton_wall_skull"));
        assert_eq!(wall_variant("oak_banner").as_deref(), Some("oak_wall_banner"));
        assert_eq!(wall_variant("player_head").as_deref(), Some("player_wall_head"));
        assert_eq!(wall_variant("oak_sign"), None);
    }

    #[test]
    fn soil_and_plant_names() {
        assert!(is_soil("dirt"));
        assert!(!is_soil("stone"));
        assert!(is_plant("tall_grass"));
        assert!(is_plant("oak_sapling"));
        assert!(!is_plant("stone"));
    }

    #[test]
    fn set_ignores_missing_property() {
        let d = rine_data::init();
        let b = &d.blocks;
        let log = b.default_state("oak_log");
        let z = set(b, log, "axis", axis_for_face(2));
        assert_eq!(b.get_prop(z, "axis"), Some("z"));
        let stone = b.default_state("stone");
        assert_eq!(set(b, stone, "axis", "y"), stone);
    }

    #[test]
    fn placement_block_names_exist() {
        let d = rine_data::init();
        let b = &d.blocks;
        for n in ["torch", "wall_torch", "soul_wall_torch", "redstone_wall_torch", "oak_wall_sign", "oak_hanging_sign", "oak_wall_hanging_sign", "skeleton_wall_skull", "white_wall_banner", "oak_door", "white_bed", "oak_trapdoor", "oak_fence_gate"] {
            assert!(b.block(n).is_some(), "missing block {n}");
        }
        let door = set(b, b.default_state("oak_door"), "half", "upper");
        assert_eq!(b.get_prop(door, "half"), Some("upper"));
    }
}
