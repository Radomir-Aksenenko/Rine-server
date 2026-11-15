//! Entity selectors (`EntitySelectorParser`, `EntitySelectorOptions`),
//! game profiles, score holders and messages with embedded selectors.
//!
//! Parsing is complete and source-independent; evaluation against the
//! world happens in the server (`rine-server/src/game/cmd/select.rs`).

use super::misc::{FloatRange, IntRange};
use super::ParseEnv;
use crate::error::{CmdError, CmdResult};
use crate::reader::Reader;
use rine_protocol::nbt::Tag;
use rine_protocol::text::Text;

/// `@p`/`@a`/... sort order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Arbitrary,
    Nearest,
    Furthest,
    Random,
}

/// One context-free predicate collected from the options, in parse order.
/// `negated` = the option value started with `!`.
#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
    /// `name=` (exact match on the plain display name).
    Name { name: String, negated: bool },
    /// `type=` with an entity type id (`minecraft:pig`) or tag
    /// (`#minecraft:skeletons`, stored with the leading `#`).
    Type { id: String, negated: bool },
    /// `tag=` (empty string = "has no tags" / negated: "has any tag").
    Tag { tag: String, negated: bool },
    /// `team=` (empty = no team).
    Team { team: String, negated: bool },
    /// `gamemode=` (0..3).
    GameMode { mode: u8, negated: bool },
    /// `level=` (players only).
    Level(IntRange),
    /// `x_rotation=` (pitch), wrapped degrees.
    XRot(FloatRange),
    /// `y_rotation=` (yaw), wrapped degrees.
    YRot(FloatRange),
    /// `scores={obj=range,...}`.
    Scores(Vec<(String, IntRange)>),
    /// `nbt=` compound must match (`NbtUtils.compareNbt`, partial lists).
    Nbt { tag: Tag, negated: bool },
    /// `advancements={id=bool | id={criterion=bool}}`.
    Advancements(Vec<(String, AdvancementCheck)>),
    /// `predicate=` loot predicate id.
    Predicate { id: String, negated: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub enum AdvancementCheck {
    Done(bool),
    Criteria(Vec<(String, bool)>),
}

/// A parsed selector (vanilla `EntitySelector`).
#[derive(Debug, Clone, PartialEq)]
pub struct EntitySelector {
    /// `limit`, or 1 for @p/@r/@s/@n, `i32::MAX` for unlimited.
    pub max_results: i32,
    /// false for @p/@a/@r (players only).
    pub includes_entities: bool,
    /// Restricted to the source's dimension (any position option or
    /// distance/dx.. set, or @p/@r/@n...).
    pub world_limited: bool,
    pub filters: Vec<Filter>,
    /// `distance=`.
    pub distance: Option<FloatRange>,
    /// `x=`/`y=`/`z=` overrides of the source position.
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub z: Option<f64>,
    /// `dx`/`dy`/`dz` volume (any present: missing ones are 0).
    pub dx: Option<f64>,
    pub dy: Option<f64>,
    pub dz: Option<f64>,
    pub sort: Sort,
    /// `@s`.
    pub current_entity: bool,
    /// Plain player name (no `@`).
    pub player_name: Option<String>,
    /// UUID string form.
    pub entity_uuid: Option<u128>,
    /// Single entity type from a non-negated `type=` (fast path), if any.
    pub type_id: Option<String>,
    /// Written with `@` (needs selector permission, affects messages).
    pub uses_selector: bool,
    /// `@e`/`@n`: only living (not removed / not dead) entities.
    pub alive_only: bool,
    /// Argument flags from the tree node (for the errors at resolve time).
    pub single: bool,
    pub players_only: bool,
}

impl Default for EntitySelector {
    fn default() -> Self {
        Self {
            max_results: i32::MAX,
            includes_entities: true,
            world_limited: false,
            filters: Vec::new(),
            distance: None,
            x: None,
            y: None,
            z: None,
            dx: None,
            dy: None,
            dz: None,
            sort: Sort::Arbitrary,
            current_entity: false,
            player_name: None,
            entity_uuid: None,
            type_id: None,
            uses_selector: false,
            alive_only: false,
            single: false,
            players_only: false,
        }
    }
}

/// `game_profile`: a selector or a literal name.
#[derive(Debug, Clone, PartialEq)]
pub enum ProfileArg {
    Selector(EntitySelector),
    Name(String),
}

/// `score_holder`: `*`, a selector, or a literal name.
#[derive(Debug, Clone, PartialEq)]
pub enum ScoreHolderArg {
    Wildcard,
    Selector(EntitySelector),
    Name(String),
}

/// `message`: the text and the selectors embedded at byte offsets
/// (offsets relative to the start of the message argument).
#[derive(Debug, Clone, PartialEq)]
pub struct MessageArg {
    pub text: String,
    pub parts: Vec<(usize, usize, EntitySelector)>,
}

pub fn err(key: &'static str, r: &Reader) -> CmdError {
    CmdError::at(Text::key(key), r)
}

/// `EntityArgument.parse`: a selector with the single/players checks
/// done at parse time (`argument.entity.toomany`,
/// `argument.player.toomany`, `argument.player.entities`).
pub fn parse_entity_arg(r: &mut Reader, env: &ParseEnv, single: bool, players: bool) -> CmdResult<EntitySelector> {
    let _ = (r, env, single, players);
    Err(CmdError::key("command.failed"))
}

/// `EntitySelectorParser.parse()` (the selector itself, without the
/// argument-level checks).
pub fn parse_selector(r: &mut Reader, allow_selectors: bool) -> CmdResult<EntitySelector> {
    let _ = (r, allow_selectors);
    Err(CmdError::key("command.failed"))
}

/// `GameProfileArgument.parse`.
pub fn parse_profile(r: &mut Reader, env: &ParseEnv) -> CmdResult<ProfileArg> {
    let _ = (r, env);
    Err(CmdError::key("command.failed"))
}

/// `ScoreHolderArgument.parse`.
pub fn parse_score_holder(r: &mut Reader, env: &ParseEnv, multiple: bool) -> CmdResult<ScoreHolderArg> {
    let _ = (r, env, multiple);
    Err(CmdError::key("command.failed"))
}

/// `MessageArgument.parse` (greedy; selectors only when allowed).
pub fn parse_message(r: &mut Reader, env: &ParseEnv) -> CmdResult<MessageArg> {
    let _ = (r, env);
    Err(CmdError::key("command.failed"))
}
