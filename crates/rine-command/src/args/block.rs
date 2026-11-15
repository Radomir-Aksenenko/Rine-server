//! `block_state` and `block_predicate` (`BlockStateParser`).

use crate::error::{CmdError, CmdResult};
use crate::reader::Reader;
use rine_protocol::nbt::Tag;

/// `BlockInput`: the state (defaults for unset properties), the
/// properties written explicitly, and block entity NBT.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockInput {
    pub state: u16,
    pub props: Vec<(String, String)>,
    pub nbt: Option<Tag>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BlockPredicate {
    /// Exact block with only the written properties checked.
    Block { block: String, props: Vec<(String, String)>, nbt: Option<Tag> },
    /// `#tag[props]{nbt}`: string-valued property checks.
    Tag { tag: String, props: Vec<(String, String)>, nbt: Option<Tag> },
}

pub fn parse_block_state(r: &mut Reader) -> CmdResult<BlockInput> {
    let _ = r;
    Err(CmdError::key("command.failed"))
}

pub fn parse_block_predicate(r: &mut Reader) -> CmdResult<BlockPredicate> {
    let _ = r;
    Err(CmdError::key("command.failed"))
}
