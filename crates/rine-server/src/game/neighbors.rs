//! Neighbor updates after block changes (vanilla `updateShape` / `canSurvive`
//! for the common cases): plants, torches,
//! carpets, rails, snow layers and other blocks that need support.
//!
//! Every changed position of the tick is collected by `World::set_block`;
//! `settle` runs once per tick before the block changes are encoded, so a
//! whole explosion is settled in one batched pass and its fixes ride in the
//! same per-section packets.

use super::{entity, Game};
use crate::out;
use ahash::AHashSet;
use rine_data::blocks::flag;
use rine_protocol::BlockPos;

/// Upper bound of positions settled per tick; the rest carries over.
const MAX_WORK: usize = 400_000;

#[derive(Clone, Copy, PartialEq)]
enum Support {
    None,
    /// Needs a block with collision below (or the same block, for stacks).
    Floor,
    /// Upper half of a two-block plant/door: needs its lower half below.
    UpperHalf,
    /// Attached to the block on the given side (dx, dy, dz).
    Side(i32, i32, i32),
    /// The block above has a sturdy center on its bottom face
    /// (`CeilingHangingSignBlock.canSurvive`).
    CeilingCenter,
}

pub fn settle(g: &mut Game) {
    let mut work = std::mem::take(&mut g.world.touched);
    if work.is_empty() {
        return;
    }
    let mut queued: AHashSet<BlockPos> = work.iter().copied().collect();
    let mut i = 0;
    while i < work.len() && i < MAX_WORK {
        let p = work[i];
        i += 1;
        for n in [p, p.offset(0, 1, 0), p.offset(0, -1, 0), p.offset(1, 0, 0), p.offset(-1, 0, 0), p.offset(0, 0, 1), p.offset(0, 0, -1)] {
            if check(g, n) && queued.insert(n) {
                work.push(n);
            }
        }
        // positions changed by `check` were recorded in `touched` again;
        // they are already in `work`, so drop the duplicates
        g.world.touched.clear();
    }
    if i < work.len() {
        g.world.touched.extend_from_slice(&work[i..]);
    }
}

/// Re-evaluate one position. Returns true if the block there changed.
fn check(g: &mut Game, pos: BlockPos) -> bool {
    let st = g.world.get_block(pos.x, pos.y, pos.z);
    let b = &rine_data::get().blocks;
    if b.is_air(st) {
        return false;
    }
    // chests join and split with their neighbours (`ChestBlock.updateShape`)
    if let Some(ns) = super::blockinv::chest_update(g, pos, st) {
        g.world.set_block(pos, ns);
        return true;
    }
    // grass/podzol/mycelium `snowy` is vanilla `updateShape` in `blocks::grass`
    let name = b.block_of(st).name.as_str();
    let short = name.strip_prefix("minecraft:").unwrap_or(name);
    let sup = support(short, st);
    if sup == Support::None {
        return false;
    }
    let ok = match sup {
        Support::Floor => {
            let below = g.world.get_block(pos.x, pos.y - 1, pos.z);
            b.block_of(below).id == b.block_of(st).id || has_collision(below) && !floor_blocked(short, below)
        }
        Support::UpperHalf => {
            let below = g.world.get_block(pos.x, pos.y - 1, pos.z);
            b.block_of(below).id == b.block_of(st).id && b.get_prop(below, "half") == Some("lower")
        }
        Support::Side(dx, dy, dz) => has_collision(g.world.get_block(pos.x + dx, pos.y + dy, pos.z + dz)),
        Support::CeilingCenter => super::redstone::data::sturdy_center(g.world.get_block(pos.x, pos.y + 1, pos.z), super::redstone::data::DOWN),
        Support::None => true,
    };
    if ok {
        return false;
    }
    pop(g, pos, st, sup != Support::UpperHalf);
    true
}

fn has_collision(st: u16) -> bool {
    let b = &rine_data::get().blocks;
    !b.is_air(st) && !b.has(st, flag::NO_COLLISION) && !b.has(st, flag::WATER) && !b.has(st, flag::LAVA)
}

/// Floor blocks that do not hold a given block even though they collide.
fn floor_blocked(short: &str, below: u16) -> bool {
    if short == "snow" {
        let bn = rine_data::get().blocks.block_of(below).name.as_str();
        return matches!(bn, "minecraft:ice" | "minecraft:packed_ice" | "minecraft:barrier");
    }
    false
}

/// Break a block that lost its support: drops, particles for viewers.
fn pop(g: &mut Game, pos: BlockPos, st: u16, drops: bool) {
    let b = &rine_data::get().blocks;
    let water = b.get_prop(st, "waterlogged") == Some("true");
    let new = if water { b.default_state("minecraft:water") } else { 0 };
    g.world.set_block(pos, new);
    let f = out::frame(out::level_event(2001, pos, st as i32));
    g.send_chunk_viewers(rine_world::ChunkPos::new(pos.x >> 4, pos.z >> 4), &f);
    if drops {
        for d in super::loot::block_drops(st, 0) {
            let off = entity::rand_f64((pos.x as u64) ^ ((pos.z as u64) << 20) ^ ((pos.y as u64) << 40) ^ g.tick);
            entity::spawn_item(
                g,
                [pos.x as f64 + 0.25 + off * 0.5, pos.y as f64 + 0.25, pos.z as f64 + 0.25 + (1.0 - off) * 0.5],
                [(off - 0.5) * 0.2, 0.2, (0.5 - off) * 0.2],
                d,
                10,
            );
        }
    }
}

fn facing_back(st: u16) -> Option<(i32, i32, i32)> {
    let b = &rine_data::get().blocks;
    // `facing` points away from the wall the block hangs on
    Some(match b.get_prop(st, "facing")? {
        "north" => (0, 0, 1),
        "south" => (0, 0, -1),
        "west" => (1, 0, 0),
        "east" => (-1, 0, 0),
        _ => return None,
    })
}

fn support(short: &str, st: u16) -> Support {
    let b = &rine_data::get().blocks;
    // attached to a wall
    if short.ends_with("wall_torch") || short.ends_with("_wall_sign") || short.ends_with("_wall_banner") || short == "ladder" {
        return facing_back(st).map(|(x, y, z)| Support::Side(x, y, z)).unwrap_or(Support::None);
    }
    // buttons, levers, grindstones: `face` = floor / wall / ceiling
    if short.ends_with("_button") || short == "lever" {
        return match b.get_prop(st, "face") {
            Some("floor") => Support::Floor,
            Some("ceiling") => Support::Side(0, 1, 0),
            Some("wall") => facing_back(st).map(|(x, y, z)| Support::Side(x, y, z)).unwrap_or(Support::None),
            _ => Support::None,
        };
    }
    if short == "lantern" || short == "soul_lantern" || short == "copper_lantern" || short.ends_with("_copper_lantern") {
        return if b.get_prop(st, "hanging") == Some("true") { Support::Side(0, 1, 0) } else { Support::Floor };
    }
    // wall hanging signs keep the default canSurvive (always true)
    if short.ends_with("_wall_hanging_sign") {
        return Support::None;
    }
    if short.ends_with("_hanging_sign") {
        return Support::CeilingCenter;
    }
    // two-block plants and doors
    if matches!(short, "tall_grass" | "large_fern" | "sunflower" | "lilac" | "rose_bush" | "peony" | "pitcher_plant" | "tall_dry_grass" | "tall_seagrass")
        || short.ends_with("_door")
    {
        return match b.get_prop(st, "half") {
            Some("upper") => Support::UpperHalf,
            Some("lower") => Support::Floor,
            _ => Support::None,
        };
    }
    let floor = matches!(
        short,
        "short_grass" | "fern" | "dead_bush" | "dandelion" | "poppy" | "blue_orchid" | "allium" | "azure_bluet" | "oxeye_daisy" | "cornflower"
            | "lily_of_the_valley" | "wither_rose" | "torchflower" | "pink_petals" | "wildflowers" | "leaf_litter" | "brown_mushroom"
            | "red_mushroom" | "sugar_cane" | "cactus" | "cactus_flower" | "torch" | "soul_torch" | "redstone_torch" | "copper_torch"
            | "moss_carpet" | "pale_moss_carpet" | "snow" | "rail" | "powered_rail" | "detector_rail" | "activator_rail" | "redstone_wire"
            | "repeater" | "comparator" | "wheat" | "carrots" | "potatoes" | "beetroots" | "sweet_berry_bush" | "bush" | "firefly_bush"
            | "short_dry_grass" | "open_eyeblossom" | "closed_eyeblossom" | "crimson_fungus" | "warped_fungus" | "crimson_roots"
            | "warped_roots" | "nether_sprouts" | "melon_stem" | "pumpkin_stem" | "attached_melon_stem" | "attached_pumpkin_stem"
            | "flower_pot" | "bamboo_sapling" | "azalea" | "flowering_azalea" | "frogspawn"
    ) || short.ends_with("_sapling")
        || short.ends_with("_tulip")
        || short.ends_with("_carpet")
        || short.ends_with("_pressure_plate")
        || short.ends_with("_sign")
        || short.ends_with("_banner")
        || short.starts_with("potted_");
    if floor {
        Support::Floor
    } else {
        Support::None
    }
}
