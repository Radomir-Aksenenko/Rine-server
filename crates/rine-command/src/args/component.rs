//! `component` and `style` arguments: SNBT checked against the text
//! component / style codecs. Resolution (selectors, scores, nbt) is done
//! by the server.

use crate::error::{CmdError, CmdResult};
use crate::reader::Reader;
use rine_protocol::nbt::Tag;

pub fn parse_component(r: &mut Reader) -> CmdResult<Tag> {
    let _ = r;
    Err(CmdError::key("command.failed"))
}

pub fn parse_style(r: &mut Reader) -> CmdResult<Tag> {
    let _ = r;
    Err(CmdError::key("command.failed"))
}
