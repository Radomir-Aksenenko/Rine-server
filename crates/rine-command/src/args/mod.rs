//! Argument types (the 62 entries of `minecraft:command_argument_type`) and
//! the values they parse into. Parse results never depend on the
//! executing source: selectors and coordinates stay unresolved.

pub mod coords;
pub mod selector;
pub mod snbt;
pub mod nbt_path;
pub mod component;
pub mod block;
pub mod item;
pub mod misc;

use crate::error::{builtin, CmdResult};
use crate::reader::Reader;
use rine_protocol::nbt::Tag;

pub use coords::Coords;
pub use selector::EntitySelector;

/// Parse-time environment (vanilla passes the source to `parse`).
#[derive(Debug, Clone, Copy)]
pub struct ParseEnv {
    /// Op permission level of the source (0..=4).
    pub level: u8,
}

impl ParseEnv {
    pub fn new(level: u8) -> Self {
        Self { level }
    }
    /// `Permissions.COMMANDS_ENTITY_SELECTORS` (op level 2).
    pub fn selectors_allowed(&self) -> bool {
        self.level >= 2
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringKind {
    Word,
    Phrase,
    Greedy,
}

/// A parser with its properties.
#[derive(Debug, Clone, PartialEq)]
pub enum ArgType {
    Bool,
    Float { min: f32, max: f32 },
    Double { min: f64, max: f64 },
    Integer { min: i32, max: i32 },
    Long { min: i64, max: i64 },
    String(StringKind),
    Entity { single: bool, players: bool },
    GameProfile,
    BlockPos,
    ColumnPos,
    Vec3,
    Vec2,
    BlockState,
    BlockPredicate,
    ItemStack,
    ItemPredicate,
    TeamColor,
    HexColor,
    Component,
    Style,
    Message,
    NbtCompound,
    NbtTag,
    NbtPath,
    Objective,
    ObjectiveCriteria,
    Operation,
    Particle,
    Angle,
    Rotation,
    ScoreboardSlot,
    ScoreHolder { multiple: bool },
    Swizzle,
    Team,
    ItemSlot,
    ItemSlots,
    ResourceLocation,
    Function,
    EntityAnchor,
    IntRange,
    FloatRange,
    Dimension,
    GameMode,
    Time { min: i32 },
    ResourceOrTag { registry: String },
    ResourceOrTagKey { registry: String },
    Resource { registry: String },
    ResourceKey { registry: String },
    ResourceSelector { registry: String },
    TemplateMirror,
    TemplateRotation,
    Heightmap,
    LootTable,
    LootPredicate,
    LootModifier,
    FloatProvider,
    IntProvider,
    SlotSource,
    Dialog,
    Feature,
    SwingAnimation,
    Uuid,
}

impl ArgType {
    pub fn from_parser(p: &str, props: Option<&serde_json::Value>) -> ArgType {
        let g = |k: &str| props.and_then(|v| v.get(k));
        let f64_or = |k: &str, d: f64| g(k).and_then(|v| v.as_f64()).unwrap_or(d);
        let i64_or = |k: &str, d: i64| g(k).and_then(|v| v.as_i64()).unwrap_or(d);
        let reg = || g("registry").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let flag = |k: &str| g(k).and_then(|v| v.as_bool()).unwrap_or(false);
        match p {
            "brigadier:bool" => ArgType::Bool,
            "brigadier:float" => ArgType::Float { min: f64_or("min", -f32::MAX as f64) as f32, max: f64_or("max", f32::MAX as f64) as f32 },
            "brigadier:double" => ArgType::Double { min: f64_or("min", -f64::MAX), max: f64_or("max", f64::MAX) },
            "brigadier:integer" => ArgType::Integer { min: i64_or("min", i32::MIN as i64) as i32, max: i64_or("max", i32::MAX as i64) as i32 },
            "brigadier:long" => ArgType::Long { min: i64_or("min", i64::MIN), max: i64_or("max", i64::MAX) },
            "brigadier:string" => ArgType::String(match g("type").and_then(|v| v.as_str()) {
                Some("phrase") => StringKind::Phrase,
                Some("greedy") => StringKind::Greedy,
                _ => StringKind::Word,
            }),
            "minecraft:entity" => ArgType::Entity { single: flag("single"), players: flag("players") },
            "minecraft:game_profile" => ArgType::GameProfile,
            "minecraft:block_pos" => ArgType::BlockPos,
            "minecraft:column_pos" => ArgType::ColumnPos,
            "minecraft:vec3" => ArgType::Vec3,
            "minecraft:vec2" => ArgType::Vec2,
            "minecraft:block_state" => ArgType::BlockState,
            "minecraft:block_predicate" => ArgType::BlockPredicate,
            "minecraft:item_stack" => ArgType::ItemStack,
            "minecraft:item_predicate" => ArgType::ItemPredicate,
            "minecraft:team_color" => ArgType::TeamColor,
            "minecraft:hex_color" => ArgType::HexColor,
            "minecraft:component" => ArgType::Component,
            "minecraft:style" => ArgType::Style,
            "minecraft:message" => ArgType::Message,
            "minecraft:nbt_compound_tag" => ArgType::NbtCompound,
            "minecraft:nbt_tag" => ArgType::NbtTag,
            "minecraft:nbt_path" => ArgType::NbtPath,
            "minecraft:objective" => ArgType::Objective,
            "minecraft:objective_criteria" => ArgType::ObjectiveCriteria,
            "minecraft:operation" => ArgType::Operation,
            "minecraft:particle" => ArgType::Particle,
            "minecraft:angle" => ArgType::Angle,
            "minecraft:rotation" => ArgType::Rotation,
            "minecraft:scoreboard_slot" => ArgType::ScoreboardSlot,
            "minecraft:score_holder" => ArgType::ScoreHolder { multiple: flag("multiple") },
            "minecraft:swizzle" => ArgType::Swizzle,
            "minecraft:team" => ArgType::Team,
            "minecraft:item_slot" => ArgType::ItemSlot,
            "minecraft:item_slots" => ArgType::ItemSlots,
            "minecraft:resource_location" => ArgType::ResourceLocation,
            "minecraft:function" => ArgType::Function,
            "minecraft:entity_anchor" => ArgType::EntityAnchor,
            "minecraft:int_range" => ArgType::IntRange,
            "minecraft:float_range" => ArgType::FloatRange,
            "minecraft:dimension" => ArgType::Dimension,
            "minecraft:gamemode" => ArgType::GameMode,
            "minecraft:time" => ArgType::Time { min: i64_or("min", 0) as i32 },
            "minecraft:resource_or_tag" => ArgType::ResourceOrTag { registry: reg() },
            "minecraft:resource_or_tag_key" => ArgType::ResourceOrTagKey { registry: reg() },
            "minecraft:resource" => ArgType::Resource { registry: reg() },
            "minecraft:resource_key" => ArgType::ResourceKey { registry: reg() },
            "minecraft:resource_selector" => ArgType::ResourceSelector { registry: reg() },
            "minecraft:template_mirror" => ArgType::TemplateMirror,
            "minecraft:template_rotation" => ArgType::TemplateRotation,
            "minecraft:heightmap" => ArgType::Heightmap,
            "minecraft:loot_table" => ArgType::LootTable,
            "minecraft:loot_predicate" => ArgType::LootPredicate,
            "minecraft:loot_modifier" => ArgType::LootModifier,
            "minecraft:context_float_provider" => ArgType::FloatProvider,
            "minecraft:context_int_provider" => ArgType::IntProvider,
            "minecraft:slot_source" => ArgType::SlotSource,
            "minecraft:dialog" => ArgType::Dialog,
            "minecraft:feature" => ArgType::Feature,
            "minecraft:swing_animation" => ArgType::SwingAnimation,
            "minecraft:uuid" => ArgType::Uuid,
            other => panic!("unknown argument parser {other}"),
        }
    }
}

/// Parsed argument values.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Bool(bool),
    Float(f32),
    Double(f64),
    Int(i32),
    Long(i64),
    /// brigadier:string, objective, team, objective_criteria, resource
    /// selector patterns: the raw string.
    Str(String),
    Selector(Box<EntitySelector>),
    Profile(Box<selector::ProfileArg>),
    ScoreHolder(Box<selector::ScoreHolderArg>),
    Message(Box<selector::MessageArg>),
    /// block_pos, column_pos, vec3, vec2, rotation.
    Coords(Coords),
    Swizzle(u8),
    Block(Box<block::BlockInput>),
    BlockPredicate(Box<block::BlockPredicate>),
    Item(Box<item::ItemInput>),
    ItemPredicate(Box<item::ItemPredicate>),
    /// team_color: `ChatFormatting` id (0..=15 colours, 21 = reset).
    Color(u8),
    HexColor(i32),
    /// component / style / nbt_tag / nbt_compound_tag: the parsed SNBT.
    Nbt(Box<Tag>),
    NbtPath(Box<nbt_path::NbtPath>),
    Operation(misc::Operation),
    Particle(Box<misc::ParticleArg>),
    Angle(misc::Angle),
    /// scoreboard_slot: `DisplaySlot` id.
    ScoreboardSlot(i32),
    /// item_slot: slot index.
    Slot(i32),
    /// item_slots: slot indices.
    Slots(Vec<i32>),
    /// Resource locations, normalised to `namespace:path`
    /// (resource_location, dimension, resource, resource_key, dialog id...).
    Id(String),
    Function(misc::FunctionRef),
    /// Tag or element reference (`#minecraft:logs` or `minecraft:oak_log`).
    ResourceOrTag(misc::ResourceOrTag),
    /// Registry entry by id, or an inline SNBT definition.
    IdOrInline(misc::IdOrInline),
    /// entity_anchor (0 feet, 1 eyes), template_mirror, template_rotation,
    /// heightmap, gamemode, swing_animation: enum ordinal.
    Enum(u8),
    IntRange(misc::IntRange),
    FloatRange(misc::FloatRange),
    Uuid(u128),
}

fn check_range<T: PartialOrd + Copy>(
    r: &mut Reader,
    start: usize,
    v: T,
    min: T,
    max: T,
    low: impl Fn(&Reader, T, T) -> crate::error::CmdError,
    high: impl Fn(&Reader, T, T) -> crate::error::CmdError,
) -> CmdResult<T> {
    if v < min {
        r.cursor = start;
        return Err(low(r, v, min));
    }
    if v > max {
        r.cursor = start;
        return Err(high(r, v, max));
    }
    Ok(v)
}

/// Parse one argument of type `ty`.
pub fn parse(ty: &ArgType, r: &mut Reader, env: &ParseEnv) -> CmdResult<Arg> {
    let start = r.cursor;
    Ok(match ty {
        ArgType::Bool => Arg::Bool(r.read_boolean()?),
        ArgType::Float { min, max } => {
            let v = r.read_float()?;
            Arg::Float(check_range(r, start, v, *min, *max, builtin::float_too_low, builtin::float_too_high)?)
        }
        ArgType::Double { min, max } => {
            let v = r.read_double()?;
            Arg::Double(check_range(r, start, v, *min, *max, builtin::double_too_low, builtin::double_too_high)?)
        }
        ArgType::Integer { min, max } => {
            let v = r.read_int()?;
            Arg::Int(check_range(r, start, v, *min, *max, builtin::integer_too_low, builtin::integer_too_high)?)
        }
        ArgType::Long { min, max } => {
            let v = r.read_long()?;
            Arg::Long(check_range(r, start, v, *min, *max, builtin::long_too_low, builtin::long_too_high)?)
        }
        ArgType::String(k) => Arg::Str(match k {
            StringKind::Word => r.read_unquoted_string().to_string(),
            StringKind::Phrase => r.read_string()?,
            StringKind::Greedy => {
                let s = r.remaining().to_string();
                r.cursor = r.total_len();
                s
            }
        }),
        ArgType::Entity { single, players } => Arg::Selector(Box::new(selector::parse_entity_arg(r, env, *single, *players)?)),
        ArgType::GameProfile => Arg::Profile(Box::new(selector::parse_profile(r, env)?)),
        ArgType::ScoreHolder { multiple } => Arg::ScoreHolder(Box::new(selector::parse_score_holder(r, env, *multiple)?)),
        ArgType::Message => Arg::Message(Box::new(selector::parse_message(r, env)?)),
        ArgType::BlockPos => Arg::Coords(coords::parse_block_pos(r)?),
        ArgType::ColumnPos => Arg::Coords(coords::parse_column_pos(r)?),
        ArgType::Vec3 => Arg::Coords(coords::parse_vec3(r, true)?),
        ArgType::Vec2 => Arg::Coords(coords::parse_vec2(r, true)?),
        ArgType::Rotation => Arg::Coords(coords::parse_rotation(r)?),
        ArgType::Swizzle => Arg::Swizzle(coords::parse_swizzle(r)?),
        ArgType::BlockState => Arg::Block(Box::new(block::parse_block_state(r)?)),
        ArgType::BlockPredicate => Arg::BlockPredicate(Box::new(block::parse_block_predicate(r)?)),
        ArgType::ItemStack => Arg::Item(Box::new(item::parse_item_stack(r)?)),
        ArgType::ItemPredicate => Arg::ItemPredicate(Box::new(item::parse_item_predicate(r)?)),
        ArgType::TeamColor => Arg::Color(misc::parse_team_color(r)?),
        ArgType::HexColor => Arg::HexColor(misc::parse_hex_color(r)?),
        ArgType::Component => Arg::Nbt(Box::new(component::parse_component(r)?)),
        ArgType::Style => Arg::Nbt(Box::new(component::parse_style(r)?)),
        ArgType::NbtCompound => Arg::Nbt(Box::new(snbt::parse_compound(r)?)),
        ArgType::NbtTag => Arg::Nbt(Box::new(snbt::parse_tag(r)?)),
        ArgType::NbtPath => Arg::NbtPath(Box::new(nbt_path::parse(r)?)),
        ArgType::Objective => Arg::Str(misc::parse_objective(r)?),
        ArgType::ObjectiveCriteria => Arg::Str(misc::parse_criteria(r)?),
        ArgType::Operation => Arg::Operation(misc::parse_operation(r)?),
        ArgType::Particle => Arg::Particle(Box::new(misc::parse_particle(r)?)),
        ArgType::Angle => Arg::Angle(misc::parse_angle(r)?),
        ArgType::ScoreboardSlot => Arg::ScoreboardSlot(misc::parse_scoreboard_slot(r)?),
        ArgType::Team => Arg::Str(r.read_unquoted_string().to_string()),
        ArgType::ItemSlot => Arg::Slot(misc::parse_slot(r)?),
        ArgType::ItemSlots => Arg::Slots(misc::parse_slots(r)?),
        ArgType::ResourceLocation => Arg::Id(misc::read_id(r)?),
        ArgType::Function => Arg::Function(misc::parse_function(r)?),
        ArgType::EntityAnchor => Arg::Enum(misc::parse_anchor(r)?),
        ArgType::IntRange => Arg::IntRange(misc::parse_int_range(r)?),
        ArgType::FloatRange => Arg::FloatRange(misc::parse_float_range(r)?),
        ArgType::Dimension => Arg::Id(misc::read_id(r)?),
        ArgType::GameMode => Arg::Enum(misc::parse_gamemode(r)?),
        ArgType::Time { min } => Arg::Int(misc::parse_time(r, *min)?),
        ArgType::ResourceOrTag { registry } => Arg::ResourceOrTag(misc::parse_resource_or_tag(r, registry)?),
        ArgType::ResourceOrTagKey { registry } => Arg::ResourceOrTag(misc::parse_resource_or_tag_key(r, registry)?),
        ArgType::Resource { registry } => Arg::Id(misc::parse_resource(r, registry)?),
        ArgType::ResourceKey { registry } => Arg::Id(misc::parse_resource_key(r, registry)?),
        ArgType::ResourceSelector { registry } => Arg::Str(misc::parse_resource_selector(r, registry)?),
        ArgType::TemplateMirror => Arg::Enum(misc::parse_template_mirror(r)?),
        ArgType::TemplateRotation => Arg::Enum(misc::parse_template_rotation(r)?),
        ArgType::Heightmap => Arg::Enum(misc::parse_heightmap(r)?),
        ArgType::LootTable => Arg::IdOrInline(misc::parse_id_or_inline(r, "minecraft:loot_table")?),
        ArgType::LootPredicate => Arg::IdOrInline(misc::parse_id_or_inline(r, "minecraft:predicate")?),
        ArgType::LootModifier => Arg::IdOrInline(misc::parse_id_or_inline(r, "minecraft:item_modifier")?),
        ArgType::FloatProvider => Arg::IdOrInline(misc::parse_provider(r, false)?),
        ArgType::IntProvider => Arg::IdOrInline(misc::parse_provider(r, true)?),
        ArgType::SlotSource => Arg::IdOrInline(misc::parse_id_or_inline(r, "minecraft:slot_source")?),
        ArgType::Dialog => Arg::IdOrInline(misc::parse_id_or_inline(r, "minecraft:dialog")?),
        ArgType::Feature => Arg::IdOrInline(misc::parse_id_or_inline(r, "minecraft:worldgen/configured_feature")?),
        ArgType::SwingAnimation => Arg::Enum(misc::parse_swing_animation(r)?),
        ArgType::Uuid => Arg::Uuid(misc::parse_uuid(r)?),
    })
}
