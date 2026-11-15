//! Small argument types: ranges, ids, enums, colours, time, uuid,
//! operations, particles, slots, resource references.

use crate::error::{CmdError, CmdResult};
use crate::reader::Reader;
use rine_protocol::nbt::Tag;

/// `MinMaxBounds.Ints`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct IntRange {
    pub min: Option<i32>,
    pub max: Option<i32>,
}
impl IntRange {
    pub fn matches(&self, v: i32) -> bool {
        self.min.is_none_or(|m| m <= v) && self.max.is_none_or(|m| v <= m)
    }
}

/// `MinMaxBounds.Doubles` (also used for float degrees).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FloatRange {
    pub min: Option<f64>,
    pub max: Option<f64>,
}
impl FloatRange {
    pub fn matches(&self, v: f64) -> bool {
        self.min.is_none_or(|m| m <= v) && self.max.is_none_or(|m| v <= m)
    }
    /// Squared bounds check (`matchesSqr`).
    pub fn matches_sqr(&self, v2: f64) -> bool {
        self.min.is_none_or(|m| m * m <= v2) && self.max.is_none_or(|m| v2 <= m * m)
    }
}

/// `scoreboard players operation` operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Assign, // =
    Add,    // +=
    Sub,    // -=
    Mul,    // *=
    Div,    // /=
    Mod,    // %=
    Min,    // <
    Max,    // >
    Swap,   // ><
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Angle {
    pub value: f32,
    pub relative: bool,
}

/// `particle`: type id and the optional SNBT options compound.
#[derive(Debug, Clone, PartialEq)]
pub struct ParticleArg {
    pub id: String,
    pub options: Option<Tag>,
}

/// `function`: `#tag` or id.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionRef {
    pub tag: bool,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResourceOrTag {
    pub tag: bool,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IdOrInline {
    Id(String),
    Inline(Tag),
}

fn todo_err() -> CmdError {
    CmdError::key("command.failed")
}

/// `Identifier.read`: `[namespace:]path`, normalised to `namespace:path`.
/// Errors `argument.id.invalid`.
pub fn read_id(r: &mut Reader) -> CmdResult<String> {
    let _ = r;
    Err(todo_err())
}
pub fn parse_team_color(r: &mut Reader) -> CmdResult<u8> {
    let _ = r;
    Err(todo_err())
}
pub fn parse_hex_color(r: &mut Reader) -> CmdResult<i32> {
    let _ = r;
    Err(todo_err())
}
pub fn parse_objective(r: &mut Reader) -> CmdResult<String> {
    let _ = r;
    Err(todo_err())
}
pub fn parse_criteria(r: &mut Reader) -> CmdResult<String> {
    let _ = r;
    Err(todo_err())
}
pub fn parse_operation(r: &mut Reader) -> CmdResult<Operation> {
    let _ = r;
    Err(todo_err())
}
pub fn parse_particle(r: &mut Reader) -> CmdResult<ParticleArg> {
    let _ = r;
    Err(todo_err())
}
pub fn parse_angle(r: &mut Reader) -> CmdResult<Angle> {
    let _ = r;
    Err(todo_err())
}
pub fn parse_scoreboard_slot(r: &mut Reader) -> CmdResult<i32> {
    let _ = r;
    Err(todo_err())
}
pub fn parse_slot(r: &mut Reader) -> CmdResult<i32> {
    let _ = r;
    Err(todo_err())
}
pub fn parse_slots(r: &mut Reader) -> CmdResult<Vec<i32>> {
    let _ = r;
    Err(todo_err())
}
pub fn parse_function(r: &mut Reader) -> CmdResult<FunctionRef> {
    let _ = r;
    Err(todo_err())
}
pub fn parse_anchor(r: &mut Reader) -> CmdResult<u8> {
    let _ = r;
    Err(todo_err())
}
pub fn parse_int_range(r: &mut Reader) -> CmdResult<IntRange> {
    let _ = r;
    Err(todo_err())
}
pub fn parse_float_range(r: &mut Reader) -> CmdResult<FloatRange> {
    let _ = r;
    Err(todo_err())
}
pub fn parse_gamemode(r: &mut Reader) -> CmdResult<u8> {
    let _ = r;
    Err(todo_err())
}
pub fn parse_time(r: &mut Reader, min: i32) -> CmdResult<i32> {
    let _ = (r, min);
    Err(todo_err())
}
pub fn parse_resource_or_tag(r: &mut Reader, registry: &str) -> CmdResult<ResourceOrTag> {
    let _ = (r, registry);
    Err(todo_err())
}
pub fn parse_resource_or_tag_key(r: &mut Reader, registry: &str) -> CmdResult<ResourceOrTag> {
    let _ = (r, registry);
    Err(todo_err())
}
pub fn parse_resource(r: &mut Reader, registry: &str) -> CmdResult<String> {
    let _ = (r, registry);
    Err(todo_err())
}
pub fn parse_resource_key(r: &mut Reader, registry: &str) -> CmdResult<String> {
    let _ = (r, registry);
    Err(todo_err())
}
pub fn parse_resource_selector(r: &mut Reader, registry: &str) -> CmdResult<String> {
    let _ = (r, registry);
    Err(todo_err())
}
pub fn parse_template_mirror(r: &mut Reader) -> CmdResult<u8> {
    let _ = r;
    Err(todo_err())
}
pub fn parse_template_rotation(r: &mut Reader) -> CmdResult<u8> {
    let _ = r;
    Err(todo_err())
}
pub fn parse_heightmap(r: &mut Reader) -> CmdResult<u8> {
    let _ = r;
    Err(todo_err())
}
pub fn parse_id_or_inline(r: &mut Reader, registry: &str) -> CmdResult<IdOrInline> {
    let _ = (r, registry);
    Err(todo_err())
}
pub fn parse_provider(r: &mut Reader, int: bool) -> CmdResult<IdOrInline> {
    let _ = (r, int);
    Err(todo_err())
}
pub fn parse_swing_animation(r: &mut Reader) -> CmdResult<u8> {
    let _ = r;
    Err(todo_err())
}
pub fn parse_uuid(r: &mut Reader) -> CmdResult<u128> {
    let _ = r;
    Err(todo_err())
}
