//! SNBT (`TagParser` / `SnbtGrammar` of 26.3) into `Tag`.

use crate::error::{CmdError, CmdResult};
use crate::reader::Reader;
use rine_protocol::nbt::Tag;

/// Any tag (`NbtTagArgument`).
pub fn parse_tag(r: &mut Reader) -> CmdResult<Tag> {
    let _ = r;
    Err(CmdError::key("command.failed"))
}

/// A compound (`CompoundTagArgument`); errors `argument.nbt.expected.compound`.
pub fn parse_compound(r: &mut Reader) -> CmdResult<Tag> {
    let _ = r;
    Err(CmdError::key("command.failed"))
}

/// Parse a whole string as SNBT (functions, storage, tests).
pub fn parse_str(s: &str) -> CmdResult<Tag> {
    let mut r = Reader::new(s);
    parse_tag(&mut r)
}

/// SNBT text of a tag as vanilla prints it (`NbtUtils.toPrettyComponent`
/// plain form / `Tag.toString()`).
pub fn to_snbt(t: &Tag) -> String {
    let _ = t;
    String::new()
}
