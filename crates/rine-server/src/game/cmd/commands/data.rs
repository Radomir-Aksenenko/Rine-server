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
