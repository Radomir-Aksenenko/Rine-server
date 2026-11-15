//! `item_stack` and `item_predicate` (`ItemParser`,
//! `ItemPredicateArgument`).

use crate::error::{CmdError, CmdResult};
use crate::reader::Reader;
use rine_protocol::nbt::Tag;

/// `ItemInput`: item id and component patch (`None` = removed with `!`).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemInput {
    pub item: String,
    pub components: Vec<(String, Option<Tag>)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemTest {
    /// `component_id` presence or `component_id=value` exact match.
    Component { id: String, value: Option<Tag>, negated: bool },
    /// `predicate_id~value` sub-predicate.
    Predicate { id: String, value: Tag, negated: bool },
    /// `a|b|c` alternatives.
    Any(Vec<ItemTest>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ItemPredicate {
    /// `*`, `#tag`, or an item id.
    pub item: ItemMatch,
    pub tests: Vec<ItemTest>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemMatch {
    Any,
    Tag(String),
    Item(String),
}

pub fn parse_item_stack(r: &mut Reader) -> CmdResult<ItemInput> {
    let _ = r;
    Err(CmdError::key("command.failed"))
}

pub fn parse_item_predicate(r: &mut Reader) -> CmdResult<ItemPredicate> {
    let _ = r;
    Err(CmdError::key("command.failed"))
}
