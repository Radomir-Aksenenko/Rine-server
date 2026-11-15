//! /summon (`SummonCommand`).

use crate::game::cmd::source::Source;
use crate::game::cmd::Commands;
use crate::game::Game;
use rine_command::{CmdError, CmdResult};
use rine_protocol::nbt::Tag;

pub fn register(_c: &mut Commands) {}

/// `SummonCommand.createEntity(source, type, pos, tag, randomizeProperties)`:
/// returns the new entity id or the vanilla error.
pub fn create_entity(g: &mut Game, src: &Source, ty: &str, pos: [f64; 3], tag: &Tag, randomize: bool) -> CmdResult<i32> {
    let _ = (g, src, ty, pos, tag, randomize);
    Err(CmdError::key("commands.summon.failed"))
}
