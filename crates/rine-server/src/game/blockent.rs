//! Block entities shared by every system: which blocks have one (vanilla
//! `BlockEntityTypes`), creating the entry when such a block is placed, and
//! the `ContainerHelper` item list format.
//!
//! The chunk's `block_entities` entry is the stored form. Systems that tick
//! often (containers) keep a typed copy while the chunk is loaded and write
//! it back before the chunk is saved.

use crate::stack::Stack;
use rine_protocol::nbt::Tag;
use rine_protocol::BlockPos;
use rine_world::chunk::BlockEntity;
use rine_world::Chunk;
use std::sync::OnceLock;

/// `BlockEntityTypes` registration for a block, by block name without the
/// namespace. Moving pistons are left to the redstone engine.
pub fn type_name_for(block: &str) -> Option<&'static str> {
    let b = block.strip_prefix("minecraft:").unwrap_or(block);
    Some(match b {
        "chest" => "chest",
        "trapped_chest" => "trapped_chest",
        "ender_chest" => "ender_chest",
        "spawner" => "mob_spawner",
        "campfire" | "soul_campfire" => "campfire",
        "bee_nest" | "beehive" => "beehive",
        "command_block" | "chain_command_block" | "repeating_command_block" => "command_block",
        "suspicious_sand" | "suspicious_gravel" => "brushable_block",
        "furnace" | "smoker" | "blast_furnace" | "hopper" | "dispenser" | "dropper" | "brewing_stand" | "barrel" | "jukebox"
        | "beacon" | "enchanting_table" | "end_portal" | "end_gateway" | "daylight_detector" | "comparator" | "structure_block"
        | "jigsaw" | "conduit" | "lectern" | "bell" | "sculk_sensor" | "calibrated_sculk_sensor" | "sculk_catalyst"
        | "sculk_shrieker" | "chiseled_bookshelf" | "decorated_pot" | "crafter" | "trial_spawner" | "vault" | "test_block"
        | "test_instance_block" | "creaking_heart" | "potent_sulfur" => return static_name(b),
        _ if b.ends_with("copper_chest") => "chest",
        _ if b.ends_with("_hanging_sign") => "hanging_sign",
        _ if b.ends_with("_sign") => "sign",
        _ if b.ends_with("_banner") => "banner",
        _ if b.ends_with("_bed") => "bed",
        _ if b.ends_with("shulker_box") => "shulker_box",
        _ if b.ends_with("_shelf") => "shelf",
        _ if b.ends_with("copper_golem_statue") => "copper_golem_statue",
        _ if (b.ends_with("_skull") || b.ends_with("_head")) && b != "piston_head" => "skull",
        _ => return None,
    })
}

fn static_name(b: &str) -> Option<&'static str> {
    const NAMES: &[&str] = &[
        "furnace", "smoker", "blast_furnace", "hopper", "dispenser", "dropper", "brewing_stand", "barrel", "jukebox", "beacon",
        "enchanting_table", "end_portal", "end_gateway", "daylight_detector", "comparator", "structure_block", "jigsaw", "conduit",
        "lectern", "bell", "sculk_sensor", "calibrated_sculk_sensor", "sculk_catalyst", "sculk_shrieker", "chiseled_bookshelf",
        "decorated_pot", "crafter", "trial_spawner", "vault", "test_block", "test_instance_block", "creaking_heart", "potent_sulfur",
    ];
    NAMES.iter().copied().find(|n| *n == b)
}

/// `minecraft:block_entity_type` id of the block entity a block id carries.
pub fn type_of_block(block_id: u32) -> Option<u32> {
    static TABLE: OnceLock<Vec<Option<u32>>> = OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let d = rine_data::get();
        let reg = d.registries.find("minecraft:block_entity_type");
        let n = d.blocks.blocks.len();
        let mut v = vec![None; n];
        for (i, b) in d.blocks.blocks.iter().enumerate() {
            if let (Some(name), Some(reg)) = (type_name_for(&b.name), reg) {
                v[i] = reg.id(&format!("minecraft:{name}"));
            }
        }
        v
    });
    t.get(block_id as usize).copied().flatten()
}

/// Registry name of a block entity type id ("minecraft:chest").
pub fn type_name(type_id: u32) -> String {
    rine_data::get()
        .registries
        .find("minecraft:block_entity_type")
        .and_then(|r| r.entries.get(type_id as usize))
        .map(|e| e.to_string())
        .unwrap_or_default()
}

pub fn find(c: &Chunk, pos: BlockPos) -> Option<&BlockEntity> {
    let (lx, lz) = ((pos.x & 15) as u8, (pos.z & 15) as u8);
    c.block_entities.iter().find(|b| b.x == lx && b.z == lz && b.y as i32 == pos.y)
}

pub fn find_mut(c: &mut Chunk, pos: BlockPos) -> Option<&mut BlockEntity> {
    let (lx, lz) = ((pos.x & 15) as u8, (pos.z & 15) as u8);
    c.block_entities.iter_mut().find(|b| b.x == lx && b.z == lz && b.y as i32 == pos.y)
}

/// World position of a chunk's block entity.
pub fn pos_of(c: &Chunk, be: &BlockEntity) -> BlockPos {
    BlockPos::new(c.pos.x * 16 + be.x as i32, be.y as i32, c.pos.z * 16 + be.z as i32)
}

/// `ContainerHelper.saveAllItems`: non-empty stacks with their `Slot`.
pub fn items_tag(items: &[Stack]) -> Tag {
    let reg = rine_data::get().registries.reg("minecraft:item");
    let mut v = Vec::new();
    for (i, st) in items.iter().enumerate() {
        if st.count <= 0 || st.item == 0 {
            continue;
        }
        let mut c = Tag::compound();
        if st.damage > 0 {
            c = c.with("components", Tag::compound().with("minecraft:damage", Tag::Int(i32::from(st.damage))));
        }
        c = c
            .with("count", Tag::Int(st.count))
            .with("Slot", Tag::Byte(i as i8))
            .with("id", Tag::String(reg.name(st.item).unwrap_or("minecraft:air").to_string()));
        v.push(c);
    }
    Tag::List(v)
}

/// `ContainerHelper.loadAllItems` into an inventory of `size` slots.
pub fn items_from_tag(t: &Tag, size: usize) -> Vec<Stack> {
    let reg = rine_data::get().registries.reg("minecraft:item");
    let mut out = vec![Stack::EMPTY; size];
    let Tag::List(l) = t else { return out };
    for e in l {
        let slot = match e.get("Slot") {
            Some(Tag::Byte(b)) => (*b as u8) as usize,
            _ => continue,
        };
        let Some(Tag::String(id)) = e.get("id") else { continue };
        let Some(item) = reg.id(id) else { continue };
        let count = match e.get("count") {
            Some(Tag::Int(c)) => *c,
            Some(Tag::Byte(c)) => i32::from(*c),
            _ => 1,
        };
        let damage = match e.get("components").and_then(|c| c.get("minecraft:damage")) {
            Some(Tag::Int(d)) => (*d).clamp(0, u16::MAX as i32) as u16,
            _ => 0,
        };
        if slot < size && count > 0 {
            out[slot] = Stack { item, count, damage };
        }
    }
    out
}

/// Reads an int-like field.
pub fn int_of(t: &Tag, k: &str) -> Option<i32> {
    match t.get(k)? {
        Tag::Byte(v) => Some(i32::from(*v)),
        Tag::Short(v) => Some(i32::from(*v)),
        Tag::Int(v) => Some(*v),
        Tag::Long(v) => Some(*v as i32),
        _ => None,
    }
}

/// Puts `(k, v)` into a compound, replacing an existing key.
pub fn put(t: &mut Tag, k: &str, v: Tag) {
    if let Tag::Compound(f) = t {
        match f.iter_mut().find(|(n, _)| n == k) {
            Some(e) => e.1 = v,
            None => f.push((k.to_string(), v)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_entity_types() {
        rine_data::init();
        let b = &rine_data::get().blocks;
        let ty = |n: &str| type_of_block(b.block(n).unwrap().id).map(type_name);
        assert_eq!(ty("chest").as_deref(), Some("minecraft:chest"));
        assert_eq!(ty("hopper").as_deref(), Some("minecraft:hopper"));
        assert_eq!(ty("red_shulker_box").as_deref(), Some("minecraft:shulker_box"));
        assert_eq!(ty("oak_wall_hanging_sign").as_deref(), Some("minecraft:hanging_sign"));
        assert_eq!(ty("oak_sign").as_deref(), Some("minecraft:sign"));
        assert_eq!(ty("repeating_command_block").as_deref(), Some("minecraft:command_block"));
        assert_eq!(ty("stone"), None);
        assert_eq!(ty("piston_head"), None);
    }

    #[test]
    fn items_round_trip() {
        rine_data::init();
        let reg = rine_data::get().registries.reg("minecraft:item");
        let mut v = vec![Stack::EMPTY; 27];
        v[0] = Stack::new(reg.id("minecraft:diamond").unwrap(), 3);
        v[7] = Stack::damaged(reg.id("minecraft:iron_pickaxe").unwrap(), 1, 5);
        assert_eq!(items_from_tag(&items_tag(&v), 27), v);
    }
}
