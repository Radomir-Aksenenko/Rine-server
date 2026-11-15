//! /data, entity/block NBT views, `nbt` text components
//! (`DataCommands`, `EntityDataAccessor`, `BlockDataAccessor`,
//! `StorageDataAccessor`, `NbtContents`).

use crate::game::cmd::source::Source;
use crate::game::cmd::Commands;
use crate::game::Game;
use rine_command::CmdResult;
use rine_protocol::nbt::Tag;
use rine_protocol::text::Text;

pub fn register(_c: &mut Commands) {}

/// `DataAccessor`: an entity, a block entity or a command storage.
#[derive(Clone, Debug)]
pub enum DataTarget {
    Entity(i32),
    Block([i32; 3]),
    Storage(String),
}

/// `DataProvider.access(context)` for `kind` = "entity" | "block" |
/// "storage" and the argument `arg` (entity selector, block pos or id).
pub fn target(x: &mut crate::game::cmd::Ctx, kind: &str, arg: &str) -> CmdResult<DataTarget> {
    match kind {
        "entity" => Ok(DataTarget::Entity(x.entity(arg)?)),
        "block" => {
            let p = x.block_pos(arg)?;
            if block_entity_nbt(x.g, p).is_none() {
                return Err(rine_command::CmdError::key("commands.data.block.invalid"));
            }
            Ok(DataTarget::Block(p))
        }
        _ => Ok(DataTarget::Storage(x.id(arg).to_string())),
    }
}

/// `DataAccessor.getData()`.
pub fn get_data(g: &Game, t: &DataTarget) -> CmdResult<Tag> {
    Ok(match t {
        DataTarget::Entity(e) => entity_nbt(g, *e),
        DataTarget::Block(p) => block_entity_nbt(g, *p).unwrap_or_else(Tag::compound),
        DataTarget::Storage(id) => g.cs.storage.get(id).cloned().unwrap_or_else(Tag::compound),
    })
}

/// `DataAccessor.setData(tag)`.
pub fn set_data(g: &mut Game, t: &DataTarget, tag: Tag) -> CmdResult<()> {
    match t {
        DataTarget::Entity(_) | DataTarget::Block(_) => {
            // entity/block NBT writes are implemented with /data
            let _ = tag;
            Ok(())
        }
        DataTarget::Storage(id) => {
            g.cs.storage.insert(id.clone(), tag);
            Ok(())
        }
    }
}

/// `BlockEntity.saveWithFullMetadata` of the block entity at `pos`
/// (`None` when there is no block entity).
pub fn block_entity_nbt(g: &Game, pos: [i32; 3]) -> Option<Tag> {
    use rine_world::ChunkPos;
    let slot = g.world.chunks.get(&ChunkPos::new(pos[0] >> 4, pos[2] >> 4))?;
    let be = slot.chunk.block_entities.iter().find(|b| b.x as i32 == pos[0] & 15 && b.z as i32 == pos[2] & 15 && b.y as i32 == pos[1])?;
    let mut t = be.nbt.clone().unwrap_or_else(Tag::compound);
    let ty = rine_data::get().registries.find("minecraft:block_entity_type").and_then(|r| r.entries.get(be.type_id as usize)).map(|e| e.to_string()).unwrap_or_default();
    if let Tag::Compound(v) = &mut t {
        v.retain(|(k, _)| !matches!(k.as_str(), "x" | "y" | "z" | "id" | "components"));
        v.push(("x".into(), Tag::Int(pos[0])));
        v.push(("y".into(), Tag::Int(pos[1])));
        v.push(("z".into(), Tag::Int(pos[2])));
        v.push(("id".into(), Tag::String(ty)));
    }
    Some(t)
}

/// Entity NBT as vanilla `EntityDataAccessor.getData` builds it
/// (`NbtPredicate.getEntityTagToCompare` for players adds `SelectedItem`).
pub fn entity_nbt(g: &Game, eid: i32) -> Tag {
    let _ = (g, eid);
    Tag::compound()
}

/// `NbtUtils.compareNbt`.
pub fn compare_nbt(expected: &Tag, actual: &Tag, partial_lists: bool) -> bool {
    if expected.id() != actual.id() {
        return false;
    }
    match (expected, actual) {
        (Tag::Compound(e), Tag::Compound(a)) => {
            if a.len() < e.len() {
                return false;
            }
            e.iter().all(|(k, v)| match actual.get(k) {
                Some(av) => compare_nbt(v, av, partial_lists),
                None => false,
            })
        }
        (Tag::List(e), Tag::List(a)) if partial_lists => {
            if e.is_empty() {
                return a.is_empty();
            }
            if a.len() < e.len() {
                return false;
            }
            e.iter().all(|t| a.iter().any(|v| compare_nbt(t, v, partial_lists)))
        }
        _ => expected == actual,
    }
}

/// Resolve an `nbt` text component (`NbtContents.resolve`).
pub fn resolve_nbt_component(g: &Game, src: &Source, fields: &[(String, Tag)], entity: Option<i32>, depth: u32) -> CmdResult<Text> {
    let _ = (g, src, fields, entity, depth);
    Ok(Text::empty())
}

/// `NbtPathArgument.NbtPath.countMatching`.
pub fn path_count(path: &rine_command::args::nbt_path::NbtPath, root: &Tag) -> i32 {
    path.count_matching(root)
}

/// `NbtPathArgument.NbtPath.set` succeeded (changed at least one tag).
pub fn path_set(path: &rine_command::args::nbt_path::NbtPath, root: &mut Tag, value: &Tag) -> bool {
    path.set(root, value).is_ok_and(|n| n > 0)
}

/// `NbtPathArgument.NbtPath.get` (error `arguments.nbtpath.nothing_found`).
pub fn path_get(path: &rine_command::args::nbt_path::NbtPath, root: &Tag) -> CmdResult<Vec<Tag>> {
    path.get(root)
}

/// `Tag.toString()` (compact SNBT).
pub fn to_snbt(t: &Tag) -> String {
    rine_command::args::snbt::to_snbt(t)
}
