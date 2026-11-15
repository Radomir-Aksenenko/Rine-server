//! Entity selectors (`EntitySelectorParser`, `EntitySelectorOptions`),
//! game profiles, score holders and messages with embedded selectors.
//!
//! Parsing is complete and source-independent; evaluation against the
//! world happens in the server (`rine-server/src/game/cmd/select.rs`).

use super::misc::{FloatRange, IntRange};
use super::ParseEnv;
use crate::error::{builtin, lit, CmdError, CmdResult};
use crate::reader::{parse_java_float, Reader};
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
    /// `x_rotation=` (pitch). Bounds are already wrapped with
    /// `Mth.wrapDegrees` as in Java. When `min > max` the range wraps
    /// around the 180 degree seam: match `rot >= min || rot <= max`
    /// (plain `FloatRange::matches` would reject everything).
    XRot(FloatRange),
    /// `y_rotation=` (yaw), same wrapping rules as `XRot`.
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
    /// Search only the source's dimension. Set by `distance`, `x`, `y`,
    /// `z`, `dx`, `dy`, `dz` (vanilla `setWorldLimited`). Otherwise all
    /// dimensions are searched, also for @p/@r/@n.
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
    /// Vanilla `type` field: the only entity type that can match. Set to
    /// `minecraft:player` for @a/@p/@r, and to the id of a non-negated
    /// `type=` (not a tag). The `Filter::Type` entries still apply.
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
///
/// A UUID-shaped name is returned as `Name`. Vanilla first looks for an
/// entity with that UUID and falls back to the name, so the executor
/// must do the same lookup.
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

const PLAYER_TYPE: &str = "minecraft:player";
const SELECTOR_TYPES: &str = "aenprs";

/// `EntityArgument.parse`: a selector with the single/players checks
/// done at parse time (`argument.entity.toomany`,
/// `argument.player.toomany`, `argument.player.entities`).
pub fn parse_entity_arg(r: &mut Reader, env: &ParseEnv, single: bool, players: bool) -> CmdResult<EntitySelector> {
    let mut sel = parse_selector(r, env.selectors_allowed())?;
    // Vanilla resets the cursor to 0 for these errors.
    if sel.max_results > 1 && single {
        let key = if players { "argument.player.toomany" } else { "argument.entity.toomany" };
        return Err(CmdError::at_cursor(Text::key(key), r, 0));
    }
    if sel.includes_entities && players && !sel.current_entity {
        return Err(CmdError::at_cursor(Text::key("argument.player.entities"), r, 0));
    }
    sel.single = single;
    sel.players_only = players;
    Ok(sel)
}

/// `EntitySelectorParser.parse()` (the selector itself, without the
/// argument-level checks).
pub fn parse_selector(r: &mut Reader, allow_selectors: bool) -> CmdResult<EntitySelector> {
    let mut p = Parser::new(r, allow_selectors);
    p.parse()?;
    Ok(p.finish())
}

/// `GameProfileArgument.parse`.
pub fn parse_profile(r: &mut Reader, env: &ParseEnv) -> CmdResult<ProfileArg> {
    if r.can_read() && r.peek() == '@' {
        let sel = parse_selector(r, env.selectors_allowed())?;
        if sel.includes_entities {
            return Err(err("argument.player.entities", r));
        }
        return Ok(ProfileArg::Selector(sel));
    }
    let start = r.cursor;
    while r.can_read() && r.peek() != ' ' {
        r.skip();
    }
    Ok(ProfileArg::Name(r.slice(start, r.cursor).to_string()))
}

/// `ScoreHolderArgument.parse`.
pub fn parse_score_holder(r: &mut Reader, env: &ParseEnv, multiple: bool) -> CmdResult<ScoreHolderArg> {
    if r.can_read() && r.peek() == '@' {
        let sel = parse_selector(r, env.selectors_allowed())?;
        if !multiple && sel.max_results > 1 {
            return Err(err("argument.entity.toomany", r));
        }
        return Ok(ScoreHolderArg::Selector(sel));
    }
    let start = r.cursor;
    while r.can_read() && r.peek() != ' ' {
        r.skip();
    }
    let text = r.slice(start, r.cursor).to_string();
    if text == "*" {
        Ok(ScoreHolderArg::Wildcard)
    } else {
        Ok(ScoreHolderArg::Name(text))
    }
}

/// `MessageArgument.parse` (greedy; selectors only when allowed).
///
/// Every `@` that starts a selector with a valid type letter is parsed.
/// Other selector errors propagate. A bare `@` or an unknown type letter
/// stays as plain text (vanilla catches `missing`/`unknown` selector
/// type errors and resumes after the `@`).
pub fn parse_message(r: &mut Reader, env: &ParseEnv) -> CmdResult<MessageArg> {
    // Vanilla counts UTF-16 units and uses `create` (no context).
    let units = r.remaining().encode_utf16().count();
    if units > 256 {
        return Err(CmdError::new(Text::tr("argument.message.too_long", vec![lit(units), lit(256)])));
    }
    let text = r.remaining().to_string();
    if !env.selectors_allowed() {
        r.cursor = r.total_len();
        return Ok(MessageArg { text, parts: Vec::new() });
    }

    let offset = r.cursor;
    let mut parts = Vec::new();
    while r.can_read() {
        if r.peek() != '@' {
            r.skip();
            continue;
        }
        let start = r.cursor;
        let type_ok = r.can_read_n(2) && SELECTOR_TYPES.contains(r.peek_at(1));
        if !type_ok {
            r.cursor = start + 1;
            continue;
        }
        let sel = parse_selector(r, true)?;
        parts.push((start - offset, r.cursor - offset, sel));
    }
    Ok(MessageArg { text, parts })
}

// ---------------------------------------------------------------------------
// Selector parser state (`EntitySelectorParser` fields).

/// `InvertableSetOptionState`: a set option that accepts one positive
/// value, or any number of negated values and tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Limit {
    None,
    Single,
    Multiple,
}

#[derive(Debug, Clone)]
struct Invertable {
    limit: Limit,
    tags: Vec<String>,
}

impl Invertable {
    fn new() -> Self {
        Self { limit: Limit::None, tags: Vec::new() }
    }
    fn can_positive(&self) -> bool {
        self.limit == Limit::None
    }
    fn can_negative(&self) -> bool {
        self.limit != Limit::Single
    }
    fn can_any(&self) -> bool {
        self.limit != Limit::Single
    }
    fn can_element(&self, inverted: bool) -> bool {
        if inverted {
            self.can_negative()
        } else {
            self.can_positive()
        }
    }
    fn can_any_tag(&self) -> bool {
        self.can_negative()
    }
    fn can_tag(&self, tag: &str) -> bool {
        self.can_any_tag() && !self.tags.iter().any(|t| t == tag)
    }
    fn mark_element(&mut self, inverted: bool) {
        self.limit = if inverted { Limit::Multiple } else { Limit::Single };
    }
    fn mark_tag(&mut self, tag: String) {
        self.limit = Limit::Multiple;
        self.tags.push(tag);
    }
}

/// `MinMaxBounds` with optional ends.
#[derive(Debug, Clone, Copy)]
struct Bounds<T> {
    min: Option<T>,
    max: Option<T>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Opt {
    Name,
    Distance,
    Level,
    X,
    Y,
    Z,
    Dx,
    Dy,
    Dz,
    XRot,
    YRot,
    Limit,
    Sort,
    Gamemode,
    Team,
    Type,
    Tag,
    Nbt,
    Scores,
    Advancements,
    Predicate,
}

struct Parser<'a> {
    r: &'a mut Reader,
    allow_selectors: bool,
    max_results: i32,
    includes_entities: bool,
    world_limited: bool,
    distance: Option<FloatRange>,
    level: Option<IntRange>,
    x: Option<f64>,
    y: Option<f64>,
    z: Option<f64>,
    dx: Option<f64>,
    dy: Option<f64>,
    dz: Option<f64>,
    rot_x: bool,
    rot_y: bool,
    filters: Vec<Filter>,
    sort: Sort,
    current_entity: bool,
    player_name: Option<String>,
    entity_uuid: Option<u128>,
    type_id: Option<String>,
    alive_only: bool,
    uses_selector: bool,
    limited: bool,
    sorted: bool,
    scores_done: bool,
    advancements_done: bool,
    name_opt: Invertable,
    gamemode_opt: Invertable,
    team_opt: Invertable,
    type_opt: Invertable,
}

impl<'a> Parser<'a> {
    fn new(r: &'a mut Reader, allow_selectors: bool) -> Self {
        Self {
            r,
            allow_selectors,
            max_results: i32::MAX,
            includes_entities: true,
            world_limited: false,
            distance: None,
            level: None,
            x: None,
            y: None,
            z: None,
            dx: None,
            dy: None,
            dz: None,
            rot_x: false,
            rot_y: false,
            filters: Vec::new(),
            sort: Sort::Arbitrary,
            current_entity: false,
            player_name: None,
            entity_uuid: None,
            type_id: None,
            alive_only: false,
            uses_selector: false,
            limited: false,
            sorted: false,
            scores_done: false,
            advancements_done: false,
            name_opt: Invertable::new(),
            gamemode_opt: Invertable::new(),
            team_opt: Invertable::new(),
            type_opt: Invertable::new(),
        }
    }

    fn finish(self) -> EntitySelector {
        EntitySelector {
            max_results: self.max_results,
            includes_entities: self.includes_entities,
            world_limited: self.world_limited,
            filters: self.filters,
            distance: self.distance,
            x: self.x,
            y: self.y,
            z: self.z,
            dx: self.dx,
            dy: self.dy,
            dz: self.dz,
            sort: self.sort,
            current_entity: self.current_entity,
            player_name: self.player_name,
            entity_uuid: self.entity_uuid,
            type_id: self.type_id,
            uses_selector: self.uses_selector,
            alive_only: self.alive_only,
            single: false,
            players_only: false,
        }
    }

    /// Reset the cursor to `start` and build the error there.
    fn rollback(&mut self, start: usize, msg: Text) -> CmdError {
        self.r.cursor = start;
        CmdError::at(msg, self.r)
    }

    fn inapplicable(&mut self, start: usize, name: &str) -> CmdError {
        self.rollback(start, Text::tr("argument.entity.options.inapplicable", vec![lit(name)]))
    }

    /// `EntitySelectorParser.parse(overridePermissions = false)`.
    fn parse(&mut self) -> CmdResult<()> {
        if self.r.can_read() && self.r.peek() == '@' {
            if !self.allow_selectors {
                return Err(err("argument.entity.selector.not_allowed", self.r));
            }
            self.r.skip();
            self.parse_selector_kind()?;
        } else {
            self.parse_name_or_uuid()?;
        }
        Ok(())
    }

    /// `parseSelector`: the selector letter and its `[...]` options.
    fn parse_selector_kind(&mut self) -> CmdResult<()> {
        if !self.r.can_read() {
            return Err(err("argument.entity.selector.missing", self.r));
        }
        let start = self.r.cursor;
        let c = self.r.read();
        match c {
            'a' => {
                self.max_results = i32::MAX;
                self.includes_entities = false;
                self.type_id = Some(PLAYER_TYPE.to_string());
            }
            'e' => {
                self.max_results = i32::MAX;
                self.includes_entities = true;
                self.alive_only = true;
            }
            'n' => {
                self.max_results = 1;
                self.includes_entities = true;
                self.sort = Sort::Nearest;
                self.alive_only = true;
            }
            'p' => {
                self.max_results = 1;
                self.includes_entities = false;
                self.sort = Sort::Nearest;
                self.type_id = Some(PLAYER_TYPE.to_string());
            }
            'r' => {
                self.max_results = 1;
                self.includes_entities = false;
                self.sort = Sort::Random;
                self.type_id = Some(PLAYER_TYPE.to_string());
            }
            's' => {
                self.max_results = 1;
                self.includes_entities = true;
                self.current_entity = true;
            }
            _ => {
                self.r.cursor = start;
                return Err(CmdError::at(
                    Text::tr("argument.entity.selector.unknown", vec![lit(format!("@{c}"))]),
                    self.r,
                ));
            }
        }
        self.uses_selector = true;
        if self.r.can_read() && self.r.peek() == '[' {
            self.r.skip();
            self.parse_options()?;
        }
        Ok(())
    }

    /// `parseNameOrUUID`.
    fn parse_name_or_uuid(&mut self) -> CmdResult<()> {
        let start = self.r.cursor;
        let name = self.r.read_string()?;
        match java_uuid(&name) {
            Some(uuid) => {
                self.entity_uuid = Some(uuid);
                self.includes_entities = true;
            }
            None => {
                if name.is_empty() || name.encode_utf16().count() > 16 {
                    self.r.cursor = start;
                    return Err(err("argument.entity.invalid", self.r));
                }
                self.includes_entities = false;
                self.player_name = Some(name);
            }
        }
        self.max_results = 1;
        Ok(())
    }

    /// `parseOptions`, after the `[`.
    fn parse_options(&mut self) -> CmdResult<()> {
        self.r.skip_whitespace();
        while self.r.can_read() && self.r.peek() != ']' {
            self.r.skip_whitespace();
            let start = self.r.cursor;
            let key = self.r.read_string()?;
            let opt = self.usable_option(&key, start)?;
            self.r.skip_whitespace();
            if !self.r.can_read() || self.r.peek() != '=' {
                self.r.cursor = start;
                return Err(CmdError::at(Text::tr("argument.entity.options.valueless", vec![lit(&key)]), self.r));
            }
            self.r.skip();
            self.r.skip_whitespace();
            self.handle(opt)?;
            self.r.skip_whitespace();
            if self.r.can_read() {
                if self.r.peek() != ',' {
                    if self.r.peek() != ']' {
                        return Err(err("argument.entity.options.unterminated", self.r));
                    }
                    break;
                }
                self.r.skip();
            }
        }
        if self.r.can_read() {
            self.r.skip();
            Ok(())
        } else {
            Err(err("argument.entity.options.unterminated", self.r))
        }
    }

    /// `EntitySelectorOptions.get`: the option by name, if it can still be
    /// used at this point of the selector.
    fn usable_option(&mut self, key: &str, start: usize) -> CmdResult<Opt> {
        let opt = match key {
            "name" => Opt::Name,
            "distance" => Opt::Distance,
            "level" => Opt::Level,
            "x" => Opt::X,
            "y" => Opt::Y,
            "z" => Opt::Z,
            "dx" => Opt::Dx,
            "dy" => Opt::Dy,
            "dz" => Opt::Dz,
            "x_rotation" => Opt::XRot,
            "y_rotation" => Opt::YRot,
            "limit" => Opt::Limit,
            "sort" => Opt::Sort,
            "gamemode" => Opt::Gamemode,
            "team" => Opt::Team,
            "type" => Opt::Type,
            "tag" => Opt::Tag,
            "nbt" => Opt::Nbt,
            "scores" => Opt::Scores,
            "advancements" => Opt::Advancements,
            "predicate" => Opt::Predicate,
            _ => {
                return Err(self.rollback(start, Text::tr("argument.entity.options.unknown", vec![lit(key)])));
            }
        };
        if !self.can_use(opt) {
            return Err(self.inapplicable(start, key));
        }
        Ok(opt)
    }

    fn can_use(&self, opt: Opt) -> bool {
        match opt {
            Opt::Name => self.name_opt.can_any(),
            Opt::Distance => self.distance.is_none(),
            Opt::Level => self.level.is_none(),
            Opt::X => self.x.is_none(),
            Opt::Y => self.y.is_none(),
            Opt::Z => self.z.is_none(),
            Opt::Dx => self.dx.is_none(),
            Opt::Dy => self.dy.is_none(),
            Opt::Dz => self.dz.is_none(),
            Opt::XRot => !self.rot_x,
            Opt::YRot => !self.rot_y,
            Opt::Limit => !self.current_entity && !self.limited,
            Opt::Sort => !self.current_entity && !self.sorted,
            Opt::Gamemode => self.gamemode_opt.can_any(),
            Opt::Team => self.team_opt.can_any(),
            Opt::Type => self.type_opt.can_any(),
            Opt::Tag | Opt::Nbt | Opt::Predicate => true,
            Opt::Scores => !self.scores_done,
            Opt::Advancements => !self.advancements_done,
        }
    }

    /// Option value handlers. The cursor is on the value (after `=`).
    fn handle(&mut self, opt: Opt) -> CmdResult<()> {
        let start = self.r.cursor;
        match opt {
            Opt::Name => {
                let inverted = self.invert();
                let name = self.r.read_string()?;
                if !self.name_opt.can_element(inverted) {
                    return Err(self.inapplicable(start, "name"));
                }
                self.name_opt.mark_element(inverted);
                self.filters.push(Filter::Name { name, negated: inverted });
            }
            Opt::Distance => {
                let range = read_double_range(self.r)?;
                if is_negative(range.min) || is_negative(range.max) {
                    return Err(self.rollback(start, Text::key("argument.entity.options.distance.negative")));
                }
                self.distance = Some(range);
                self.world_limited = true;
            }
            Opt::Level => {
                let range = read_int_range(self.r)?;
                if is_negative_int(range.min) || is_negative_int(range.max) {
                    return Err(self.rollback(start, Text::key("argument.entity.options.level.negative")));
                }
                self.level = Some(range);
                self.includes_entities = false;
                self.filters.push(Filter::Level(range));
            }
            Opt::X => {
                self.world_limited = true;
                self.x = Some(self.r.read_double()?);
            }
            Opt::Y => {
                self.world_limited = true;
                self.y = Some(self.r.read_double()?);
            }
            Opt::Z => {
                self.world_limited = true;
                self.z = Some(self.r.read_double()?);
            }
            Opt::Dx => {
                self.world_limited = true;
                self.dx = Some(self.r.read_double()?);
            }
            Opt::Dy => {
                self.world_limited = true;
                self.dy = Some(self.r.read_double()?);
            }
            Opt::Dz => {
                self.world_limited = true;
                self.dz = Some(self.r.read_double()?);
            }
            Opt::XRot => {
                let bounds = read_float_bounds(self.r)?;
                self.rot_x = true;
                self.filters.push(Filter::XRot(wrap_rotation(bounds)));
            }
            Opt::YRot => {
                let bounds = read_float_bounds(self.r)?;
                self.rot_y = true;
                self.filters.push(Filter::YRot(wrap_rotation(bounds)));
            }
            Opt::Limit => {
                let count = self.r.read_int()?;
                if count < 1 {
                    return Err(self.rollback(start, Text::key("argument.entity.options.limit.toosmall")));
                }
                self.max_results = count;
                self.limited = true;
            }
            Opt::Sort => {
                let name = self.r.read_unquoted_string().to_string();
                self.sort = match name.as_str() {
                    "nearest" => Sort::Nearest,
                    "furthest" => Sort::Furthest,
                    "random" => Sort::Random,
                    "arbitrary" => Sort::Arbitrary,
                    _ => {
                        return Err(self.rollback(
                            start,
                            Text::tr("argument.entity.options.sort.irreversible", vec![lit(&name)]),
                        ))
                    }
                };
                self.sorted = true;
            }
            Opt::Gamemode => {
                let inverted = self.invert();
                if !self.gamemode_opt.can_element(inverted) {
                    return Err(self.inapplicable(start, "gamemode"));
                }
                let name = self.r.read_unquoted_string().to_string();
                let mode = match name.as_str() {
                    "survival" => 0,
                    "creative" => 1,
                    "adventure" => 2,
                    "spectator" => 3,
                    _ => {
                        return Err(self.rollback(
                            start,
                            Text::tr("argument.entity.options.mode.invalid", vec![lit(&name)]),
                        ))
                    }
                };
                self.includes_entities = false;
                self.filters.push(Filter::GameMode { mode, negated: inverted });
                self.gamemode_opt.mark_element(inverted);
            }
            Opt::Team => {
                let inverted = self.invert();
                let team = self.r.read_unquoted_string().to_string();
                if !self.team_opt.can_element(inverted) {
                    return Err(self.inapplicable(start, "team"));
                }
                self.filters.push(Filter::Team { team, negated: inverted });
                self.team_opt.mark_element(inverted);
            }
            Opt::Type => self.handle_type(start)?,
            Opt::Tag => {
                let inverted = self.invert();
                let tag = self.r.read_unquoted_string().to_string();
                self.filters.push(Filter::Tag { tag, negated: inverted });
            }
            Opt::Nbt => {
                let inverted = self.invert();
                let tag = super::snbt::parse_compound(self.r)?;
                self.filters.push(Filter::Nbt { tag, negated: inverted });
            }
            Opt::Scores => self.handle_scores()?,
            Opt::Advancements => self.handle_advancements()?,
            Opt::Predicate => {
                let inverted = self.invert();
                let id = read_identifier(self.r)?;
                self.filters.push(Filter::Predicate { id, negated: inverted });
            }
        }
        Ok(())
    }

    /// `type=`: an entity type id or `#tag`.
    fn handle_type(&mut self, start: usize) -> CmdResult<()> {
        let inverted = self.invert();
        if self.is_tag() {
            if !self.type_opt.can_any_tag() {
                return Err(self.inapplicable(start, "type"));
            }
            let id = read_identifier(self.r)?;
            if !self.type_opt.can_tag(&id) {
                return Err(self.inapplicable(start, "type"));
            }
            self.filters.push(Filter::Type { id: format!("#{id}"), negated: inverted });
            self.type_opt.mark_tag(id);
        } else {
            if !self.type_opt.can_element(inverted) {
                return Err(self.inapplicable(start, "type"));
            }
            let id = read_identifier(self.r)?;
            if !is_entity_type(&id) {
                return Err(self.rollback(start, Text::tr("argument.entity.options.type.invalid", vec![lit(&id)])));
            }
            if id == PLAYER_TYPE && !inverted {
                self.includes_entities = false;
            }
            if !inverted {
                self.type_id = Some(id.clone());
            }
            self.filters.push(Filter::Type { id, negated: inverted });
            self.type_opt.mark_element(inverted);
        }
        Ok(())
    }

    /// `scores={name=range,...}`.
    fn handle_scores(&mut self) -> CmdResult<()> {
        let r = &mut *self.r;
        r.expect('{')?;
        r.skip_whitespace();
        let mut expected: Vec<(String, IntRange)> = Vec::new();
        while r.can_read() && r.peek() != '}' {
            r.skip_whitespace();
            let name = r.read_unquoted_string().to_string();
            r.skip_whitespace();
            r.expect('=')?;
            r.skip_whitespace();
            let value = read_int_range(r)?;
            match expected.iter_mut().find(|(n, _)| *n == name) {
                Some(entry) => entry.1 = value,
                None => expected.push((name, value)),
            }
            r.skip_whitespace();
            if r.can_read() && r.peek() == ',' {
                r.skip();
            }
        }
        r.expect('}')?;
        if !expected.is_empty() {
            self.filters.push(Filter::Scores(expected));
        }
        self.scores_done = true;
        Ok(())
    }

    /// `advancements={id=bool,id={criterion=bool}}`.
    fn handle_advancements(&mut self) -> CmdResult<()> {
        let r = &mut *self.r;
        r.expect('{')?;
        r.skip_whitespace();
        let mut expected: Vec<(String, AdvancementCheck)> = Vec::new();
        while r.can_read() && r.peek() != '}' {
            r.skip_whitespace();
            let name = read_identifier(r)?;
            r.skip_whitespace();
            r.expect('=')?;
            r.skip_whitespace();
            let check = if r.can_read() && r.peek() == '{' {
                let mut progress: Vec<(String, bool)> = Vec::new();
                r.skip_whitespace();
                r.expect('{')?;
                r.skip_whitespace();
                while r.can_read() && r.peek() != '}' {
                    r.skip_whitespace();
                    let criterion = r.read_unquoted_string().to_string();
                    r.skip_whitespace();
                    r.expect('=')?;
                    r.skip_whitespace();
                    let value = r.read_boolean()?;
                    match progress.iter_mut().find(|(n, _)| *n == criterion) {
                        Some(entry) => entry.1 = value,
                        None => progress.push((criterion, value)),
                    }
                    r.skip_whitespace();
                    if r.can_read() && r.peek() == ',' {
                        r.skip();
                    }
                }
                r.skip_whitespace();
                r.expect('}')?;
                r.skip_whitespace();
                AdvancementCheck::Criteria(progress)
            } else {
                AdvancementCheck::Done(r.read_boolean()?)
            };
            match expected.iter_mut().find(|(n, _)| *n == name) {
                Some(entry) => entry.1 = check,
                None => expected.push((name, check)),
            }
            r.skip_whitespace();
            if r.can_read() && r.peek() == ',' {
                r.skip();
            }
        }
        r.expect('}')?;
        if !expected.is_empty() {
            self.filters.push(Filter::Advancements(expected));
            self.includes_entities = false;
        }
        self.advancements_done = true;
        Ok(())
    }

    /// `shouldInvertValue`: a leading `!`.
    fn invert(&mut self) -> bool {
        self.r.skip_whitespace();
        if self.r.can_read() && self.r.peek() == '!' {
            self.r.skip();
            self.r.skip_whitespace();
            true
        } else {
            false
        }
    }

    /// `isTag`: a leading `#`.
    fn is_tag(&mut self) -> bool {
        self.r.skip_whitespace();
        if self.r.can_read() && self.r.peek() == '#' {
            self.r.skip();
            self.r.skip_whitespace();
            true
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Values.

/// `Identifier.read`: `[namespace:]path`, normalised to `namespace:path`.
fn read_identifier(r: &mut Reader) -> CmdResult<String> {
    let start = r.cursor;
    while r.can_read() && is_identifier_char(r.peek()) {
        r.skip();
    }
    let raw = r.slice(start, r.cursor).to_string();
    match split_identifier(&raw) {
        Some(id) => Ok(id),
        None => {
            r.cursor = start;
            Err(err("argument.id.invalid", r))
        }
    }
}

fn is_identifier_char(c: char) -> bool {
    c.is_ascii_digit() || c.is_ascii_lowercase() || matches!(c, '_' | ':' | '/' | '.' | '-')
}

fn is_valid_path_char(c: char) -> bool {
    c == '_' || c == '-' || c.is_ascii_lowercase() || c.is_ascii_digit() || c == '/' || c == '.'
}

fn is_valid_namespace_char(c: char) -> bool {
    c == '_' || c == '-' || c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.'
}

/// `Identifier.parse` (`bySeparator`): an empty path is accepted.
fn split_identifier(raw: &str) -> Option<String> {
    let valid_path = |p: &str| p.chars().all(is_valid_path_char);
    let (ns, path) = match raw.find(':') {
        Some(0) => ("minecraft", &raw[1..]),
        Some(i) => {
            let ns = &raw[..i];
            if ns == ".." || !ns.chars().all(is_valid_namespace_char) {
                return None;
            }
            (ns, &raw[i + 1..])
        }
        None => ("minecraft", raw),
    };
    if !valid_path(path) {
        return None;
    }
    Some(format!("{ns}:{path}"))
}

/// Whether `id` is an entry of the `minecraft:entity_type` registry.
fn is_entity_type(id: &str) -> bool {
    rine_data::get().registries.find("minecraft:entity_type").is_some_and(|reg| reg.id(id).is_some())
}

/// `UUID.fromString` (JDK algorithm: exactly four dashes, each group a
/// signed hex long, masked to its width).
fn java_uuid(s: &str) -> Option<u128> {
    if s.len() > 36 {
        return None;
    }
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 5 {
        return None;
    }
    let group = |i: usize| i64::from_str_radix(parts[i], 16).ok();
    let p0 = group(0)? & 0xffff_ffff;
    let p1 = group(1)? & 0xffff;
    let p2 = group(2)? & 0xffff;
    let p3 = group(3)? & 0xffff;
    let p4 = group(4)? & 0xffff_ffff_ffff;
    let most = ((p0 as u64) << 32) | ((p1 as u64) << 16) | (p2 as u64);
    let least = ((p3 as u64) << 48) | (p4 as u64);
    Some(((most as u128) << 64) | least as u128)
}

/// `Mth.wrapDegrees(float)`.
fn wrap_degrees(v: f32) -> f32 {
    let mut n = v % 360.0;
    if n >= 180.0 {
        n -= 360.0;
    }
    if n < -180.0 {
        n += 360.0;
    }
    n
}

/// `createRotationPredicate` bounds: missing min is 0, missing max is 359,
/// both wrapped.
fn wrap_rotation(b: Bounds<f32>) -> FloatRange {
    FloatRange {
        min: Some(wrap_degrees(b.min.unwrap_or(0.0)) as f64),
        max: Some(wrap_degrees(b.max.unwrap_or(359.0)) as f64),
    }
}

fn is_negative(v: Option<f64>) -> bool {
    matches!(v, Some(n) if n < 0.0)
}

fn is_negative_int(v: Option<i32>) -> bool {
    matches!(v, Some(n) if n < 0)
}

fn swapped<T: PartialOrd>(b: &Bounds<T>) -> bool {
    matches!((&b.min, &b.max), (Some(a), Some(z)) if a > z)
}

/// `MinMaxBounds.Bounds.fromReader`. Every error reports the cursor at
/// the start of the range.
fn read_bounds<T: Copy>(
    r: &mut Reader,
    conv: fn(&str) -> Option<T>,
    invalid: fn(&Reader, &str) -> CmdError,
) -> CmdResult<Bounds<T>> {
    if !r.can_read() {
        return Err(err("argument.range.empty", r));
    }
    let start = r.cursor;
    match read_bounds_inner(r, conv, invalid) {
        Ok(b) => Ok(b),
        Err(e) => Err(CmdError::at_cursor(e.msg, r, start)),
    }
}

fn read_bounds_inner<T: Copy>(
    r: &mut Reader,
    conv: fn(&str) -> Option<T>,
    invalid: fn(&Reader, &str) -> CmdError,
) -> CmdResult<Bounds<T>> {
    let min = read_bound_number(r, conv, invalid)?;
    let max = if r.can_read_n(2) && r.peek() == '.' && r.peek_at(1) == '.' {
        r.skip();
        r.skip();
        read_bound_number(r, conv, invalid)?
    } else {
        min
    };
    if min.is_none() && max.is_none() {
        return Err(err("argument.range.empty", r));
    }
    Ok(Bounds { min, max })
}

fn read_bound_number<T: Copy>(
    r: &mut Reader,
    conv: fn(&str) -> Option<T>,
    invalid: fn(&Reader, &str) -> CmdError,
) -> CmdResult<Option<T>> {
    let start = r.cursor;
    while r.can_read() && is_bound_char(r) {
        r.skip();
    }
    if r.cursor == start {
        return Ok(None);
    }
    let text = r.slice(start, r.cursor);
    match conv(text) {
        Some(v) => Ok(Some(v)),
        None => Err(invalid(r, text)),
    }
}

/// Digits, `-`, and a `.` that does not start a `..` separator.
fn is_bound_char(r: &Reader) -> bool {
    let c = r.peek();
    c.is_ascii_digit() || c == '-' || (c == '.' && (!r.can_read_n(2) || r.peek_at(1) != '.'))
}

fn read_int_bounds(r: &mut Reader) -> CmdResult<Bounds<i32>> {
    read_bounds(r, |s| s.parse::<i32>().ok(), builtin::invalid_int)
}

fn read_double_bounds(r: &mut Reader) -> CmdResult<Bounds<f64>> {
    read_bounds(r, parse_java_float, builtin::invalid_double)
}

fn read_float_bounds(r: &mut Reader) -> CmdResult<Bounds<f32>> {
    read_bounds(r, |s| parse_java_float(s).and_then(|_| s.parse::<f32>().ok()), builtin::invalid_float)
}

/// `MinMaxBounds.Ints.fromReader` (rejects swapped bounds).
fn read_int_range(r: &mut Reader) -> CmdResult<IntRange> {
    let start = r.cursor;
    let b = read_int_bounds(r)?;
    if swapped(&b) {
        r.cursor = start;
        return Err(err("argument.range.swapped", r));
    }
    Ok(IntRange { min: b.min, max: b.max })
}

/// `MinMaxBounds.Doubles.fromReader` (rejects swapped bounds).
fn read_double_range(r: &mut Reader) -> CmdResult<FloatRange> {
    let start = r.cursor;
    let b = read_double_bounds(r)?;
    if swapped(&b) {
        r.cursor = start;
        return Err(err("argument.range.swapped", r));
    }
    Ok(FloatRange { min: b.min, max: b.max })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rine_protocol::text::Content;

    fn sel(s: &str) -> EntitySelector {
        rine_data::init();
        let mut r = Reader::new(s);
        let v = parse_selector(&mut r, true).unwrap_or_else(|e| panic!("{s}: {}", describe(&e)));
        assert_eq!(r.cursor, s.len(), "unconsumed input in {s}");
        v
    }

    fn fail(s: &str) -> CmdError {
        rine_data::init();
        let mut r = Reader::new(s);
        match parse_selector(&mut r, true) {
            Ok(v) => panic!("{s} should fail, got {v:?}"),
            Err(e) => e,
        }
    }

    /// Key and argument texts of a translatable error.
    fn describe(e: &CmdError) -> String {
        match &e.msg.content {
            Content::Translate { key, with, .. } => {
                let args: Vec<String> = with.iter().map(|t| t.text.clone()).collect();
                format!("{key} {args:?} @{}", e.cursor)
            }
            _ => "?".into(),
        }
    }

    fn assert_err(s: &str, key: &str, args: &[&str], cursor: usize) {
        let e = fail(s);
        let (k, with) = match &e.msg.content {
            Content::Translate { key, with, .. } => (key.to_string(), with.iter().map(|t| t.text.clone()).collect::<Vec<_>>()),
            _ => panic!("not translatable"),
        };
        assert_eq!(k, key, "{s}");
        assert_eq!(with, args, "{s}");
        assert_eq!(e.cursor, cursor, "cursor for {s}");
    }

    #[test]
    fn plain_selectors() {
        let a = sel("@a");
        assert_eq!(a.max_results, i32::MAX);
        assert!(!a.includes_entities);
        assert_eq!(a.type_id.as_deref(), Some(PLAYER_TYPE));
        assert!(a.uses_selector);
        assert!(!a.alive_only);
        assert_eq!(a.sort, Sort::Arbitrary);

        let e = sel("@e");
        assert!(e.includes_entities);
        assert!(e.alive_only);
        assert_eq!(e.type_id, None);

        let p = sel("@p");
        assert_eq!(p.max_results, 1);
        assert!(!p.includes_entities);
        assert_eq!(p.sort, Sort::Nearest);

        let n = sel("@n");
        assert!(n.alive_only && n.includes_entities && n.max_results == 1);

        let s = sel("@s");
        assert!(s.current_entity && s.includes_entities && !s.alive_only);
    }

    #[test]
    fn type_limit_sort() {
        let e = sel("@e[type=minecraft:pig,limit=3,sort=nearest]");
        assert_eq!(e.max_results, 3);
        assert_eq!(e.sort, Sort::Nearest);
        assert_eq!(e.type_id.as_deref(), Some("minecraft:pig"));
        assert_eq!(e.filters, vec![Filter::Type { id: "minecraft:pig".into(), negated: false }]);
        assert!(e.includes_entities);
    }

    #[test]
    fn distance_range() {
        let p = sel("@p[distance=..5]");
        assert_eq!(p.distance, Some(FloatRange { min: None, max: Some(5.0) }));
        assert!(p.world_limited);
        assert_eq!(p.max_results, 1);
    }

    #[test]
    fn tags_and_negation() {
        let e = sel("@e[tag=a,tag=!b]");
        assert_eq!(
            e.filters,
            vec![
                Filter::Tag { tag: "a".into(), negated: false },
                Filter::Tag { tag: "b".into(), negated: true },
            ]
        );
    }

    #[test]
    fn position_options() {
        let s = sel("@s[x=1,y=2,z=3,dx=4]");
        assert_eq!((s.x, s.y, s.z), (Some(1.0), Some(2.0), Some(3.0)));
        assert_eq!((s.dx, s.dy, s.dz), (Some(4.0), None, None));
        assert!(s.world_limited);
        assert!(s.current_entity);
    }

    #[test]
    fn scores_option() {
        let a = sel("@a[scores={kills=1..}]");
        assert_eq!(a.filters, vec![Filter::Scores(vec![("kills".into(), IntRange { min: Some(1), max: None })])]);
    }

    #[test]
    fn rotation_is_wrapped() {
        let p = sel("@p[x_rotation=..90]");
        assert_eq!(p.filters, vec![Filter::XRot(FloatRange { min: Some(0.0), max: Some(90.0) })]);
        let q = sel("@p[y_rotation=-90..]");
        assert_eq!(q.filters, vec![Filter::YRot(FloatRange { min: Some(-90.0), max: Some(-1.0) })]);
    }

    #[test]
    fn plain_name_and_uuid() {
        let n = sel("Steve");
        assert_eq!(n.player_name.as_deref(), Some("Steve"));
        assert_eq!(n.max_results, 1);
        assert!(!n.includes_entities);
        assert!(!n.uses_selector);

        let u = sel("123e4567-e89b-12d3-a456-426614174000");
        assert_eq!(u.entity_uuid, Some(0x123e4567_e89b_12d3_a456_426614174000));
        assert!(u.includes_entities);
        assert_eq!(u.player_name, None);
    }

    #[test]
    fn errors_with_cursor() {
        assert_err("@e[foo=1]", "argument.entity.options.unknown", &["foo"], 3);
        assert_err("@e[limit=0]", "argument.entity.options.limit.toosmall", &[], 9);
        assert_err("@a[sort=nearest,sort=furthest]", "argument.entity.options.inapplicable", &["sort"], 16);
        assert_err("@x", "argument.entity.selector.unknown", &["@x"], 1);
        assert_err("@e[type=minecraft:nope]", "argument.entity.options.type.invalid", &["minecraft:nope"], 8);
        assert_err("@p[distance=-1]", "argument.entity.options.distance.negative", &[], 12);
        assert_err("@p[x=1", "argument.entity.options.unterminated", &[], 6);
        assert_err("@e[x=1,x=2]", "argument.entity.options.inapplicable", &["x"], 7);
        assert_err("@a[sort=bogus]", "argument.entity.options.sort.irreversible", &["bogus"], 8);
        assert_err("@", "argument.entity.selector.missing", &[], 1);
        assert_err("", "argument.entity.invalid", &[], 0);
        assert_err("Steve_Steve_Steve_X", "argument.entity.invalid", &[], 0);
    }

    #[test]
    fn selectors_not_allowed() {
        rine_data::init();
        let mut r = Reader::new("@a");
        let e = parse_selector(&mut r, false).unwrap_err();
        assert_eq!(describe(&e), "argument.entity.selector.not_allowed [] @0");
    }

    #[test]
    fn entity_arg_checks() {
        rine_data::init();
        let env = ParseEnv::new(2);
        let mut r = Reader::new("@e");
        let e = parse_entity_arg(&mut r, &env, true, false).unwrap_err();
        assert_eq!(describe(&e), "argument.entity.toomany [] @0");

        let mut r = Reader::new("@a");
        let e = parse_entity_arg(&mut r, &env, true, true).unwrap_err();
        assert_eq!(describe(&e), "argument.player.toomany [] @0");

        let mut r = Reader::new("@e");
        let e = parse_entity_arg(&mut r, &env, false, true).unwrap_err();
        assert_eq!(describe(&e), "argument.player.entities [] @0");

        let mut r = Reader::new("@s");
        let s = parse_entity_arg(&mut r, &env, false, true).unwrap();
        assert!(s.players_only && !s.single);

        let mut r = Reader::new("@a");
        let e = parse_entity_arg(&mut r, &ParseEnv::new(0), false, false).unwrap_err();
        assert_eq!(describe(&e), "argument.entity.selector.not_allowed [] @0");
    }

    #[test]
    fn profile_and_score_holder() {
        rine_data::init();
        let env = ParseEnv::new(2);
        let mut r = Reader::new("Steve rest");
        assert_eq!(parse_profile(&mut r, &env).unwrap(), ProfileArg::Name("Steve".into()));
        assert_eq!(r.cursor, 5);

        let mut r = Reader::new("@e");
        let e = parse_profile(&mut r, &env).unwrap_err();
        assert_eq!(describe(&e), "argument.player.entities [] @2");

        let mut r = Reader::new("*");
        assert_eq!(parse_score_holder(&mut r, &env, false).unwrap(), ScoreHolderArg::Wildcard);

        let mut r = Reader::new("Bob");
        assert_eq!(parse_score_holder(&mut r, &env, false).unwrap(), ScoreHolderArg::Name("Bob".into()));

        let mut r = Reader::new("@a");
        let e = parse_score_holder(&mut r, &env, false).unwrap_err();
        assert_eq!(describe(&e), "argument.entity.toomany [] @2");
        let mut r = Reader::new("@a");
        assert!(matches!(parse_score_holder(&mut r, &env, true).unwrap(), ScoreHolderArg::Selector(_)));
    }

    #[test]
    fn message_with_selectors() {
        rine_data::init();
        let env = ParseEnv::new(2);
        let input = "hi @p and @e[type=minecraft:pig]!";
        let mut r = Reader::new(input);
        let m = parse_message(&mut r, &env).unwrap();
        assert_eq!(m.text, input);
        assert_eq!(r.cursor, input.len());
        assert_eq!(m.parts.len(), 2);
        assert_eq!((m.parts[0].0, m.parts[0].1), (3, 5));
        assert_eq!((m.parts[1].0, m.parts[1].1), (10, 32));
        assert_eq!(m.parts[1].2.type_id.as_deref(), Some("minecraft:pig"));
    }

    #[test]
    fn message_keeps_failed_selectors_as_text() {
        rine_data::init();
        let env = ParseEnv::new(2);
        for text in ["a @x b", "a @", "@", "mail me@home"] {
            let mut r = Reader::new(text);
            let m = parse_message(&mut r, &env).unwrap();
            assert!(m.parts.is_empty(), "{text}");
            assert_eq!(m.text, text);
        }
    }

    #[test]
    fn message_selector_errors_propagate() {
        rine_data::init();
        let env = ParseEnv::new(2);
        let mut r = Reader::new("hi @p[foo]");
        let e = parse_message(&mut r, &env).unwrap_err();
        assert_eq!(describe(&e), "argument.entity.options.unknown [\"foo\"] @6");
    }

    #[test]
    fn message_without_selectors_and_too_long() {
        rine_data::init();
        let mut r = Reader::new("hi @p");
        let m = parse_message(&mut r, &ParseEnv::new(0)).unwrap();
        assert!(m.parts.is_empty());
        assert_eq!(m.text, "hi @p");

        let long = "a".repeat(257);
        let mut r = Reader::new(&long);
        let e = parse_message(&mut r, &ParseEnv::new(2)).unwrap_err();
        assert_eq!(describe(&e), "argument.message.too_long [\"257\", \"256\"] @0");
        assert!(e.input.is_none());
    }
}
