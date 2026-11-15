//! NBT paths (`NbtPathArgument`): parsing and get/set/remove/insert.

use crate::error::{CmdError, CmdResult};
use crate::reader::Reader;

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// `{...}` at the root: root must match the compound.
    MatchRoot(rine_protocol::nbt::Tag),
    /// `name` / `"name"`.
    Key(String),
    /// `name{...}`.
    MatchKey(String, rine_protocol::nbt::Tag),
    /// `[]`.
    AllElements,
    /// `[n]` (negative from the end).
    Index(i32),
    /// `[{...}]`.
    MatchElement(rine_protocol::nbt::Tag),
}

#[derive(Debug, Clone, PartialEq)]
pub struct NbtPath {
    /// The source text (for messages: `commands.data.get.invalid`...).
    pub original: String,
    pub nodes: Vec<Node>,
}

pub fn parse(r: &mut Reader) -> CmdResult<NbtPath> {
    let _ = r;
    Err(CmdError::key("command.failed"))
}
