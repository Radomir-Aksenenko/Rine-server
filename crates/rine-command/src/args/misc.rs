//! Small argument types: ranges, ids, enums, colours, time, uuid,
//! operations, particles, slots, resource references.

use crate::error::{lit, CmdError, CmdResult};
use crate::reader::{parse_java_float, Reader};
use rine_protocol::nbt::Tag;
use rine_protocol::text::Text;
use std::sync::OnceLock;

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

// ---------------------------------------------------------------------------
// Identifiers
// ---------------------------------------------------------------------------

/// Characters `Identifier.isAllowedInIdentifier` accepts (the greedy read).
fn is_id_char(c: char) -> bool {
    c.is_ascii_digit() || c.is_ascii_lowercase() || c == '_' || c == ':' || c == '/' || c == '.' || c == '-'
}

fn is_namespace_char(c: char) -> bool {
    c == '_' || c == '-' || c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.'
}

fn is_path_char(c: char) -> bool {
    c == '_' || c == '-' || c.is_ascii_lowercase() || c.is_ascii_digit() || c == '/' || c == '.'
}

/// `Identifier.bySeparator` with validation. A missing or leading separator
/// gives the `minecraft` namespace. `None` if the parts are invalid.
fn split_id(raw: &str, sep: char) -> Option<(String, String)> {
    let (ns, path) = match raw.find(sep) {
        Some(0) => ("minecraft", &raw[1..]),
        Some(i) => (&raw[..i], &raw[i + 1..]),
        None => ("minecraft", raw),
    };
    if ns == ".." || !ns.chars().all(is_namespace_char) || !path.chars().all(is_path_char) {
        return None;
    }
    if ns.len() + path.len() + 1 > i16::MAX as usize {
        return None;
    }
    Some((ns.to_string(), path.to_string()))
}

/// Reads the greedy identifier run (`Identifier.readGreedy`).
fn read_greedy_id(r: &mut Reader) -> String {
    let start = r.cursor;
    while r.can_read() && is_id_char(r.peek()) {
        r.skip();
    }
    r.slice(start, r.cursor).to_string()
}

/// `Identifier.read`: `[namespace:]path`, normalised to `namespace:path`.
/// Empty input is valid and gives `minecraft:`, as in vanilla. Errors
/// `argument.id.invalid` with the cursor reset to the start.
pub fn read_id(r: &mut Reader) -> CmdResult<String> {
    let start = r.cursor;
    let raw = read_greedy_id(r);
    match split_id(&raw, ':') {
        Some((ns, path)) => Ok(format!("{ns}:{path}")),
        None => {
            r.cursor = start;
            Err(CmdError::at(Text::key("argument.id.invalid"), r))
        }
    }
}

/// `Identifier.readNonEmpty` without the error: restores the cursor on failure.
fn try_read_id_non_empty(r: &mut Reader) -> Option<String> {
    let start = r.cursor;
    let raw = read_greedy_id(r);
    if !raw.is_empty() {
        if let Some((ns, path)) = split_id(&raw, ':') {
            return Some(format!("{ns}:{path}"));
        }
    }
    r.cursor = start;
    None
}

// ---------------------------------------------------------------------------
// Registry helpers
// ---------------------------------------------------------------------------

/// Whether `entry` is in `registry`. Unknown registries (not in `rine_data`)
/// accept everything.
fn element_known(registry: &str, entry: &str) -> bool {
    match rine_data::get().registries.find(registry) {
        Some(reg) => reg.id(entry).is_some(),
        None => true,
    }
}

/// Whether `#tag` exists in `registry`. Unknown registries, and registries
/// without any tag data, accept everything.
fn tag_known(registry: &str, tag: &str) -> bool {
    let regs = &rine_data::get().registries;
    if regs.find(registry).is_none() || !regs.tags.contains_key(registry) {
        return true;
    }
    regs.tag(registry, tag).is_some()
}

/// `StringUtils`-style wildcard match (`FilenameUtils.wildcardMatch`): `*`
/// matches any run, `?` one char.
fn wildcard_match(text: &str, pat: &str) -> bool {
    let (t, p) = (text.as_bytes(), pat.as_bytes());
    let (mut ti, mut pi) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut star_t = 0usize;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi] == t[ti]) {
            ti += 1;
            pi += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            star_t = ti;
            pi += 1;
        } else if let Some(sp) = star {
            pi = sp + 1;
            star_t += 1;
            ti = star_t;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

// ---------------------------------------------------------------------------
// Colours
// ---------------------------------------------------------------------------

/// `TeamColor` names in id order (0..=15).
const TEAM_COLORS: [&str; 16] = [
    "black",
    "dark_blue",
    "dark_green",
    "dark_aqua",
    "dark_red",
    "dark_purple",
    "gold",
    "gray",
    "dark_gray",
    "blue",
    "green",
    "aqua",
    "red",
    "light_purple",
    "yellow",
    "white",
];

/// `TeamColorArgument`: one of the 16 colours (no `reset` in 26.3).
/// Returns the colour id (0..=15).
pub fn parse_team_color(r: &mut Reader) -> CmdResult<u8> {
    let name = r.read_unquoted_string().to_string();
    match TEAM_COLORS.iter().position(|c| *c == name) {
        Some(i) => Ok(i as u8),
        None => Err(CmdError::at(Text::tr("argument.color.invalid", vec![lit(name.as_str())]), r)),
    }
}

/// `HexColorArgument`: `RGB` or `RRGGBB`, returned as opaque ARGB (`0xFF` alpha).
/// Like Java `Integer.parseInt`, each pair may carry a sign.
pub fn parse_hex_color(r: &mut Reader) -> CmdResult<i32> {
    let s = r.read_unquoted_string().to_string();
    let parts: Option<[i32; 3]> = if s.len() == 3 {
        let d = |i: usize| i32::from_str_radix(&s[i..i + 1], 16).ok().map(|v| v * 17);
        d(0).zip(d(1)).zip(d(2)).map(|((a, b), c)| [a, b, c])
    } else if s.len() == 6 {
        let p = |i: usize| i32::from_str_radix(&s[i..i + 2], 16).ok();
        p(0).zip(p(2)).zip(p(4)).map(|((a, b), c)| [a, b, c])
    } else {
        None
    };
    match parts {
        Some([a, b, c]) => {
            let argb = (0xFFu32 << 24) | (((a & 0xFF) as u32) << 16) | (((b & 0xFF) as u32) << 8) | ((c & 0xFF) as u32);
            Ok(argb as i32)
        }
        None => Err(CmdError::at(Text::tr("argument.hexcolor.invalid", vec![lit(s.as_str())]), r)),
    }
}

// ---------------------------------------------------------------------------
// Scoreboard
// ---------------------------------------------------------------------------

/// `ObjectiveArgument`: any unquoted word (existence is checked at run time).
pub fn parse_objective(r: &mut Reader) -> CmdResult<String> {
    Ok(r.read_unquoted_string().to_string())
}

/// Custom criteria names registered by `ObjectiveCriteria`.
const CUSTOM_CRITERIA: [&str; 11] = [
    "dummy",
    "trigger",
    "deathCount",
    "playerKillCount",
    "totalKillCount",
    "health",
    "food",
    "air",
    "armor",
    "xp",
    "level",
];

/// Registry of the value side of each `minecraft:stat_type`.
fn stat_value_registry(stat_type: &str) -> Option<&'static str> {
    Some(match stat_type {
        "minecraft:mined" => "minecraft:block",
        "minecraft:crafted" | "minecraft:used" | "minecraft:broken" | "minecraft:picked_up" | "minecraft:dropped" => "minecraft:item",
        "minecraft:killed" | "minecraft:killed_by" => "minecraft:entity_type",
        "minecraft:custom" => "minecraft:custom_stat",
        _ => return None,
    })
}

/// `ObjectiveCriteria.byName`. Returns the canonical name (`getName()`).
fn criteria_name(id: &str) -> Option<String> {
    if CUSTOM_CRITERIA.contains(&id) {
        return Some(id.to_string());
    }
    let team = id
        .strip_prefix("teamkill.")
        .or_else(|| id.strip_prefix("killedByTeam."))
        .is_some_and(|c| TEAM_COLORS.contains(&c));
    if team {
        return Some(id.to_string());
    }
    // `stat_type:value`, each side read as `ns.path` (default `minecraft`).
    let colon = id.find(':')?;
    let (type_ns, type_path) = split_id(&id[..colon], '.')?;
    let stat_type = format!("{type_ns}:{type_path}");
    let (val_ns, val_path) = split_id(&id[colon + 1..], '.')?;
    let key = format!("{val_ns}:{val_path}");
    let regs = &rine_data::get().registries;
    regs.id("minecraft:stat_type", &stat_type)?;
    let value_reg = stat_value_registry(&stat_type)?;
    regs.id(value_reg, &key)?;
    Some(format!("{}:{}", stat_type.replace(':', "."), key.replace(':', ".")))
}

/// `ObjectiveCriteriaArgument`: reads up to the next space. Errors
/// `argument.criteria.invalid` with the cursor at the start of the word.
pub fn parse_criteria(r: &mut Reader) -> CmdResult<String> {
    let start = r.cursor;
    while r.can_read() && r.peek() != ' ' {
        r.skip();
    }
    let id = r.slice(start, r.cursor).to_string();
    match criteria_name(&id) {
        Some(name) => Ok(name),
        None => {
            r.cursor = start;
            Err(CmdError::at(Text::tr("argument.criteria.invalid", vec![lit(id.as_str())]), r))
        }
    }
}

/// `OperationArgument`.
pub fn parse_operation(r: &mut Reader) -> CmdResult<Operation> {
    if !r.can_read() {
        return Err(CmdError::at(Text::key("arguments.operation.invalid"), r));
    }
    let start = r.cursor;
    while r.can_read() && r.peek() != ' ' {
        r.skip();
    }
    let op = r.slice(start, r.cursor).to_string();
    match op.as_str() {
        "=" => Ok(Operation::Assign),
        "+=" => Ok(Operation::Add),
        "-=" => Ok(Operation::Sub),
        "*=" => Ok(Operation::Mul),
        "/=" => Ok(Operation::Div),
        "%=" => Ok(Operation::Mod),
        "<" => Ok(Operation::Min),
        ">" => Ok(Operation::Max),
        "><" => Ok(Operation::Swap),
        // Vanilla throws this one without context (`create()`).
        _ => Err(CmdError::key("arguments.operation.invalid")),
    }
}

/// `ParticleArgument`: id checked against `minecraft:particle_type`, then
/// an optional `{...}` options compound.
pub fn parse_particle(r: &mut Reader) -> CmdResult<ParticleArg> {
    let id = read_id(r)?;
    if !element_known("minecraft:particle_type", &id) {
        return Err(CmdError::at(Text::tr("particle.notFound", vec![lit(id.as_str())]), r));
    }
    let options = if r.can_read() && r.peek() == '{' { Some(super::snbt::parse_compound(r)?) } else { None };
    Ok(ParticleArg { id, options })
}

/// `AngleArgument`: optional `~` (relative) then a finite float.
pub fn parse_angle(r: &mut Reader) -> CmdResult<Angle> {
    if !r.can_read() {
        return Err(CmdError::at(Text::key("argument.angle.incomplete"), r));
    }
    let relative = if r.peek() == '~' {
        r.skip();
        true
    } else {
        false
    };
    let value = if r.can_read() && r.peek() != ' ' { r.read_float()? } else { 0.0 };
    if value.is_finite() {
        Ok(Angle { value, relative })
    } else {
        Err(CmdError::at(Text::key("argument.angle.invalid"), r))
    }
}

/// `ScoreboardSlotArgument`: returns the `DisplaySlot` id.
pub fn parse_scoreboard_slot(r: &mut Reader) -> CmdResult<i32> {
    /// `DisplaySlot` serialized names and ids.
    const DISPLAY_SLOTS: [(&str, i32); 19] = [
        ("list", 0),
        ("sidebar", 1),
        ("below_name", 2),
        ("sidebar.team.black", 3),
        ("sidebar.team.dark_blue", 4),
        ("sidebar.team.dark_green", 5),
        ("sidebar.team.dark_aqua", 6),
        ("sidebar.team.dark_red", 7),
        ("sidebar.team.dark_purple", 8),
        ("sidebar.team.gold", 9),
        ("sidebar.team.gray", 10),
        ("sidebar.team.dark_gray", 11),
        ("sidebar.team.blue", 12),
        ("sidebar.team.green", 13),
        ("sidebar.team.aqua", 14),
        ("sidebar.team.red", 15),
        ("sidebar.team.light_purple", 16),
        ("sidebar.team.yellow", 17),
        ("sidebar.team.white", 18),
    ];
    let name = r.read_unquoted_string().to_string();
    match DISPLAY_SLOTS.iter().find(|(n, _)| *n == name) {
        Some((_, id)) => Ok(*id),
        None => Err(CmdError::at(Text::tr("argument.scoreboardDisplaySlot.invalid", vec![lit(name.as_str())]), r)),
    }
}

// ---------------------------------------------------------------------------
// Slots
// ---------------------------------------------------------------------------

fn push_single(v: &mut Vec<(String, Vec<i32>)>, name: &str, id: i32) {
    v.push((name.to_string(), vec![id]));
}

fn push_multi(v: &mut Vec<(String, Vec<i32>)>, name: &str, ids: &[i32]) {
    v.push((name.to_string(), ids.to_vec()));
}

/// `name.N` for each slot plus `name.*` for the whole range.
fn push_range(v: &mut Vec<(String, Vec<i32>)>, prefix: &str, offset: i32, size: i32) {
    for i in 0..size {
        v.push((format!("{prefix}{i}"), vec![offset + i]));
    }
    v.push((format!("{prefix}*"), (0..size).map(|i| offset + i).collect()));
}

/// `SlotRanges.SLOTS`. Armour and hand indices are `EquipmentSlot.getIndex(base)`.
fn build_slots() -> Vec<(String, Vec<i32>)> {
    let mut v = Vec::new();
    push_single(&mut v, "contents", 0);
    push_range(&mut v, "container.", 0, 54);
    push_range(&mut v, "hotbar.", 0, 9);
    push_range(&mut v, "inventory.", 9, 27);
    push_range(&mut v, "enderchest.", 200, 27);
    push_range(&mut v, "mob.inventory.", 300, 8);
    push_range(&mut v, "horse.", 500, 15);
    push_single(&mut v, "weapon", 98);
    push_single(&mut v, "weapon.mainhand", 98);
    push_single(&mut v, "weapon.offhand", 99);
    push_multi(&mut v, "weapon.*", &[98, 99]);
    push_single(&mut v, "armor.head", 103);
    push_single(&mut v, "armor.chest", 102);
    push_single(&mut v, "armor.legs", 101);
    push_single(&mut v, "armor.feet", 100);
    push_single(&mut v, "armor.body", 105);
    push_multi(&mut v, "armor.*", &[103, 102, 101, 100, 105]);
    push_single(&mut v, "saddle", 106);
    push_single(&mut v, "horse.chest", 499);
    push_single(&mut v, "player.cursor", 499);
    push_range(&mut v, "player.crafting.", 500, 4);
    v
}

fn slot_table() -> &'static [(String, Vec<i32>)] {
    static TABLE: OnceLock<Vec<(String, Vec<i32>)>> = OnceLock::new();
    TABLE.get_or_init(build_slots)
}

/// Reads one word (up to a space) and looks up its slot range.
/// Errors `slot.unknown` with the cursor after the word.
fn read_slot_range(r: &mut Reader) -> CmdResult<(String, &'static [i32])> {
    let start = r.cursor;
    while r.can_read() && r.peek() != ' ' {
        r.skip();
    }
    let name = r.slice(start, r.cursor).to_string();
    match slot_table().iter().find(|(n, _)| *n == name) {
        Some((_, ids)) => Ok((name, ids.as_slice())),
        None => Err(CmdError::at(Text::tr("slot.unknown", vec![lit(name.as_str())]), r)),
    }
}

/// `SlotArgument`: a single slot, returns its index.
pub fn parse_slot(r: &mut Reader) -> CmdResult<i32> {
    let (name, ids) = read_slot_range(r)?;
    if ids.len() != 1 {
        return Err(CmdError::at(Text::tr("slot.only_single_allowed", vec![lit(name.as_str())]), r));
    }
    Ok(ids[0])
}

/// `SlotsArgument`: all indices of the range.
pub fn parse_slots(r: &mut Reader) -> CmdResult<Vec<i32>> {
    let (_, ids) = read_slot_range(r)?;
    Ok(ids.to_vec())
}

/// `FunctionArgument.parse`: `#tag` or id. Existence is checked at run time.
pub fn parse_function(r: &mut Reader) -> CmdResult<FunctionRef> {
    let tag = if r.can_read() && r.peek() == '#' {
        r.skip();
        true
    } else {
        false
    };
    let id = read_id(r)?;
    Ok(FunctionRef { tag, id })
}

/// `EntityAnchorArgument`: `feet` = 0, `eyes` = 1.
pub fn parse_anchor(r: &mut Reader) -> CmdResult<u8> {
    let start = r.cursor;
    let name = r.read_unquoted_string().to_string();
    match name.as_str() {
        "feet" => Ok(0),
        "eyes" => Ok(1),
        _ => {
            r.cursor = start;
            Err(CmdError::at(Text::tr("argument.anchor.invalid", vec![lit(name.as_str())]), r))
        }
    }
}

// ---------------------------------------------------------------------------
// Ranges
// ---------------------------------------------------------------------------

/// One side of `MinMaxBounds`: digits, `-`, and `.` unless it starts `..`.
fn read_bound_number<T>(r: &mut Reader, conv: fn(&str) -> Option<T>, invalid_key: &'static str) -> Result<Option<T>, CmdError> {
    let start = r.cursor;
    while r.can_read() {
        let c = r.peek();
        let ok = c.is_ascii_digit() || c == '-' || (c == '.' && !r.remaining().starts_with(".."));
        if !ok {
            break;
        }
        r.skip();
    }
    let text = r.slice(start, r.cursor);
    if text.is_empty() {
        return Ok(None);
    }
    match conv(text) {
        Some(v) => Ok(Some(v)),
        None => Err(CmdError::at(Text::tr(invalid_key, vec![lit(text)]), r)),
    }
}

fn read_bounds_inner<T: Copy>(
    r: &mut Reader,
    conv: fn(&str) -> Option<T>,
    invalid_key: &'static str,
) -> Result<(Option<T>, Option<T>), CmdError> {
    let min = read_bound_number(r, conv, invalid_key)?;
    let max = if r.remaining().starts_with("..") {
        r.cursor += 2;
        read_bound_number(r, conv, invalid_key)?
    } else {
        min
    };
    if min.is_none() && max.is_none() {
        return Err(CmdError::at(Text::key("argument.range.empty"), r));
    }
    Ok((min, max))
}

/// `MinMaxBounds.Bounds.fromReader`. Vanilla rethrows every error at the
/// range start, so the error cursor is reset to `start` as well.
fn read_bounds<T: Copy>(
    r: &mut Reader,
    conv: fn(&str) -> Option<T>,
    invalid_key: &'static str,
) -> CmdResult<(Option<T>, Option<T>)> {
    let start = r.cursor;
    if !r.can_read() {
        return Err(CmdError::at(Text::key("argument.range.empty"), r));
    }
    match read_bounds_inner(r, conv, invalid_key) {
        Ok(v) => Ok(v),
        Err(e) => {
            r.cursor = start;
            Err(CmdError { cursor: start, ..e })
        }
    }
}

/// `RangeArgument.Ints`.
pub fn parse_int_range(r: &mut Reader) -> CmdResult<IntRange> {
    let start = r.cursor;
    let (min, max) = read_bounds(r, |s| s.parse::<i32>().ok(), "parsing.int.invalid")?;
    if let (Some(a), Some(b)) = (min, max) {
        if a > b {
            r.cursor = start;
            return Err(CmdError::at(Text::key("argument.range.swapped"), r));
        }
    }
    Ok(IntRange { min, max })
}

/// `RangeArgument.Floats` (`MinMaxBounds.Doubles`, parsed as Java doubles).
pub fn parse_float_range(r: &mut Reader) -> CmdResult<FloatRange> {
    let start = r.cursor;
    let (min, max) = read_bounds(r, parse_java_float, "parsing.double.invalid")?;
    if let (Some(a), Some(b)) = (min, max) {
        if a > b {
            r.cursor = start;
            return Err(CmdError::at(Text::key("argument.range.swapped"), r));
        }
    }
    Ok(FloatRange { min, max })
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// `GameModeArgument`: survival=0, creative=1, adventure=2, spectator=3.
pub fn parse_gamemode(r: &mut Reader) -> CmdResult<u8> {
    let name = r.read_unquoted_string().to_string();
    match name.as_str() {
        "survival" => Ok(0),
        "creative" => Ok(1),
        "adventure" => Ok(2),
        "spectator" => Ok(3),
        _ => Err(CmdError::at(Text::tr("argument.gamemode.invalid", vec![lit(name.as_str())]), r)),
    }
}

/// `StringRepresentableArgument`: exact serialized name, error `argument.enum.invalid`.
/// Returns the index of the name in `names`, or the value in `values`.
fn read_enum(r: &mut Reader, names: &[(&str, u8)], invalid_key: &'static str) -> CmdResult<u8> {
    let name = r.read_unquoted_string().to_string();
    match names.iter().find(|(n, _)| *n == name) {
        Some((_, v)) => Ok(*v),
        None => Err(CmdError::at(Text::tr(invalid_key, vec![lit(name.as_str())]), r)),
    }
}

/// `Mirror`: none=0, left_right=1, front_back=2.
pub fn parse_template_mirror(r: &mut Reader) -> CmdResult<u8> {
    read_enum(r, &[("none", 0), ("left_right", 1), ("front_back", 2)], "argument.enum.invalid")
}

/// `Rotation`: none=0, clockwise_90=1, clockwise_180=2 (serialized as `180`),
/// counterclockwise_90=3.
pub fn parse_template_rotation(r: &mut Reader) -> CmdResult<u8> {
    read_enum(r, &[("none", 0), ("clockwise_90", 1), ("180", 2), ("counterclockwise_90", 3)], "argument.enum.invalid")
}

/// `Heightmap.Types` kept after worldgen, lower-case names. Returns the
/// ordinal in the full enum (WORLD_SURFACE_WG is 0).
pub fn parse_heightmap(r: &mut Reader) -> CmdResult<u8> {
    read_enum(
        r,
        &[("world_surface", 1), ("ocean_floor", 3), ("motion_blocking", 4), ("motion_blocking_no_leaves", 5)],
        "argument.enum.invalid",
    )
}

/// `SwingAnimationArgument` (its own error key): none=0, whack=1, stab=2.
pub fn parse_swing_animation(r: &mut Reader) -> CmdResult<u8> {
    read_enum(r, &[("none", 0), ("whack", 1), ("stab", 2)], "argument.swing_animation.invalid")
}

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// `ResourceOrIdArgument`. Tries an id first (as the packrat alternative
/// does), then an SNBT value. An id in a known registry must exist.
pub fn parse_id_or_inline(r: &mut Reader, registry: &str) -> CmdResult<IdOrInline> {
    let start = r.cursor;
    r.skip_whitespace();
    if let Some(id) = try_read_id_non_empty(r) {
        if !element_known(registry, &id) {
            return Err(CmdError::at(Text::tr("argument.resource_or_id.no_such_element", vec![lit(id.as_str()), lit(registry)]), r));
        }
        return Ok(IdOrInline::Id(id));
    }
    // Vanilla resets to the mark taken before the whitespace.
    r.cursor = start;
    Ok(IdOrInline::Inline(super::snbt::parse_tag(r)?))
}

/// `ContextIntProvider` / `ContextFloatProvider` via `ResourceOrIdArgument`.
pub fn parse_provider(r: &mut Reader, int: bool) -> CmdResult<IdOrInline> {
    let registry = if int { "minecraft:context_int_provider" } else { "minecraft:context_float_provider" };
    parse_id_or_inline(r, registry)
}

/// `ResourceOrTagArgument`: `#tag` or element. Both must exist in a known registry.
pub fn parse_resource_or_tag(r: &mut Reader, registry: &str) -> CmdResult<ResourceOrTag> {
    let start = r.cursor;
    if r.can_read() && r.peek() == '#' {
        r.skip();
        let id = match read_id(r) {
            Ok(id) => id,
            Err(e) => {
                r.cursor = start;
                return Err(e);
            }
        };
        if !tag_known(registry, &id) {
            // The error keeps the cursor after the id; the reader goes back to '#'.
            let err = CmdError::at(Text::tr("argument.resource_tag.not_found", vec![lit(id.as_str()), lit(registry)]), r);
            r.cursor = start;
            return Err(err);
        }
        Ok(ResourceOrTag { tag: true, id })
    } else {
        let id = read_id(r)?;
        if !element_known(registry, &id) {
            return Err(CmdError::at(Text::tr("argument.resource.not_found", vec![lit(id.as_str()), lit(registry)]), r));
        }
        Ok(ResourceOrTag { tag: false, id })
    }
}

/// `ResourceOrTagKeyArgument`: no existence check at parse time.
pub fn parse_resource_or_tag_key(r: &mut Reader, _registry: &str) -> CmdResult<ResourceOrTag> {
    let start = r.cursor;
    if r.can_read() && r.peek() == '#' {
        r.skip();
        match read_id(r) {
            Ok(id) => Ok(ResourceOrTag { tag: true, id }),
            Err(e) => {
                r.cursor = start;
                Err(e)
            }
        }
    } else {
        Ok(ResourceOrTag { tag: false, id: read_id(r)? })
    }
}

/// `ResourceArgument`: an element id that must exist in a known registry.
pub fn parse_resource(r: &mut Reader, registry: &str) -> CmdResult<String> {
    let id = read_id(r)?;
    if !element_known(registry, &id) {
        return Err(CmdError::at(Text::tr("argument.resource.not_found", vec![lit(id.as_str()), lit(registry)]), r));
    }
    Ok(id)
}

/// `ResourceKeyArgument`: any id, no existence check at parse time.
pub fn parse_resource_key(r: &mut Reader, _registry: &str) -> CmdResult<String> {
    read_id(r)
}

fn is_pattern_char(c: char) -> bool {
    is_id_char(c) || c == '*' || c == '?'
}

/// `ResourceSelectorArgument`: wildcard pattern, namespaced with `minecraft:`
/// when it has no colon. Must match at least one entry of a known registry.
/// Returns the namespaced pattern.
pub fn parse_resource_selector(r: &mut Reader, registry: &str) -> CmdResult<String> {
    let start = r.cursor;
    while r.can_read() && is_pattern_char(r.peek()) {
        r.skip();
    }
    let raw = r.slice(start, r.cursor).to_string();
    let pattern = if raw.contains(':') { raw } else { format!("minecraft:{raw}") };
    if let Some(reg) = rine_data::get().registries.find(registry) {
        if !reg.entries.iter().any(|e| wildcard_match(e, &pattern)) {
            return Err(CmdError::at(Text::tr("argument.resource_selector.not_found", vec![lit(pattern.as_str()), lit(registry)]), r));
        }
    }
    Ok(pattern)
}

// ---------------------------------------------------------------------------
// Time, uuid
// ---------------------------------------------------------------------------

/// `TimeArgument`: float, then unit `d` (24000), `s` (20), `t` or nothing (1).
/// Rounds like Java `Math.round` (half up, saturating).
pub fn parse_time(r: &mut Reader, min: i32) -> CmdResult<i32> {
    let value = r.read_float()?;
    let unit = r.read_unquoted_string().to_string();
    let factor: i32 = match unit.as_str() {
        "d" => 24000,
        "s" => 20,
        "t" | "" => 1,
        _ => return Err(CmdError::at(Text::key("argument.time.invalid_unit"), r)),
    };
    let x = value * factor as f32;
    let ticks = ((x as f64) + 0.5).floor() as i32;
    if ticks < min {
        return Err(CmdError::at(Text::tr("argument.time.tick_count_too_low", vec![lit(min), lit(ticks)]), r));
    }
    Ok(ticks)
}

/// Java `Long.parseLong(s, 16)`: sign allowed, must fit in i64.
fn java_parse_long_hex(p: &str) -> Option<i64> {
    if p.is_empty() {
        return None;
    }
    i64::from_str_radix(p, 16).ok()
}

/// Java `UUID.fromString` (JDK 9+): exactly four dashes, each group parsed
/// as a long and masked to its field width.
fn java_uuid(s: &str) -> Option<u128> {
    if s.len() > 36 {
        return None;
    }
    let dashes: Vec<usize> = s.match_indices('-').map(|(i, _)| i).collect();
    if dashes.len() != 4 {
        return None;
    }
    let parts = [
        &s[..dashes[0]],
        &s[dashes[0] + 1..dashes[1]],
        &s[dashes[1] + 1..dashes[2]],
        &s[dashes[2] + 1..dashes[3]],
        &s[dashes[3] + 1..],
    ];
    let mut v = [0u64; 5];
    for (i, p) in parts.iter().enumerate() {
        v[i] = java_parse_long_hex(p)? as u64;
    }
    let most = ((v[0] & 0xffff_ffff) << 32) | ((v[1] & 0xffff) << 16) | (v[2] & 0xffff);
    let least = ((v[3] & 0xffff) << 48) | (v[4] & 0xffff_ffff_ffff);
    Some(((most as u128) << 64) | least as u128)
}

/// `UuidArgument`: the longest run of `[-A-Fa-f0-9]`, parsed like `UUID.fromString`.
pub fn parse_uuid(r: &mut Reader) -> CmdResult<u128> {
    let len = r.remaining().bytes().take_while(|b| b.is_ascii_hexdigit() || *b == b'-').count();
    if len > 0 {
        let value = java_uuid(&r.remaining()[..len]);
        if let Some(v) = value {
            r.cursor += len;
            return Ok(v);
        }
    }
    Err(CmdError::at(Text::key("argument.uuid.invalid"), r))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rd(s: &str) -> Reader {
        rine_data::init();
        Reader::new(s)
    }

    fn err_of<T: std::fmt::Debug>(r: CmdResult<T>) -> CmdError {
        r.unwrap_err()
    }

    #[test]
    fn id_normalises_and_reads_prefix() {
        let mut r = rd("stone x");
        assert_eq!(read_id(&mut r).unwrap(), "minecraft:stone");
        assert_eq!(r.cursor, 5);
        let mut r = rd("foo:bar baz");
        assert_eq!(read_id(&mut r).unwrap(), "foo:bar");
        assert_eq!(r.cursor, 7);
        let mut r = rd(":dirt");
        assert_eq!(read_id(&mut r).unwrap(), "minecraft:dirt");
    }

    #[test]
    fn id_empty_and_uppercase_are_vanilla_quirks() {
        // Identifier.read on empty or non-id input succeeds as `minecraft:`.
        let mut r = rd("");
        assert_eq!(read_id(&mut r).unwrap(), "minecraft:");
        let mut r = rd("Stone");
        assert_eq!(read_id(&mut r).unwrap(), "minecraft:");
        assert_eq!(r.cursor, 0);
    }

    #[test]
    fn id_invalid_resets_cursor() {
        let mut r = rd("a:b:c");
        let e = err_of(read_id(&mut r));
        assert_eq!(e.msg, Text::key("argument.id.invalid"));
        assert_eq!(e.cursor, 0);
        assert_eq!(r.cursor, 0);
        let mut r = rd("..:x");
        assert!(read_id(&mut r).is_err());
    }

    #[test]
    fn team_color() {
        assert_eq!(parse_team_color(&mut rd("red")).unwrap(), 12);
        assert_eq!(parse_team_color(&mut rd("black ")).unwrap(), 0);
        assert_eq!(parse_team_color(&mut rd("white")).unwrap(), 15);
        let e = err_of(parse_team_color(&mut rd("reset")));
        assert_eq!(e.msg, Text::tr("argument.color.invalid", vec![lit("reset")]));
        assert_eq!(e.cursor, 5);
    }

    #[test]
    fn hex_color() {
        assert_eq!(parse_hex_color(&mut rd("F00")).unwrap(), 0xFFFF0000u32 as i32);
        assert_eq!(parse_hex_color(&mut rd("FF0000")).unwrap(), 0xFFFF0000u32 as i32);
        assert_eq!(parse_hex_color(&mut rd("abcdef")).unwrap(), 0xFFABCDEFu32 as i32);
        assert_eq!(parse_hex_color(&mut rd("0f0")).unwrap(), 0xFF00FF00u32 as i32);
        let mut r = rd("FFFF");
        let e = err_of(parse_hex_color(&mut r));
        assert_eq!(e.msg, Text::tr("argument.hexcolor.invalid", vec![lit("FFFF")]));
        assert_eq!(e.cursor, 4);
        assert!(parse_hex_color(&mut rd("GGG")).is_err());
    }

    #[test]
    fn objective_reads_word() {
        let mut r = rd("foo bar");
        assert_eq!(parse_objective(&mut r).unwrap(), "foo");
        assert_eq!(r.cursor, 3);
        assert_eq!(parse_objective(&mut rd("")).unwrap(), "");
    }

    #[test]
    fn criteria_custom_and_stats() {
        assert_eq!(parse_criteria(&mut rd("dummy")).unwrap(), "dummy");
        assert_eq!(parse_criteria(&mut rd("teamkill.red")).unwrap(), "teamkill.red");
        assert_eq!(parse_criteria(&mut rd("killedByTeam.white")).unwrap(), "killedByTeam.white");
        assert_eq!(parse_criteria(&mut rd("minecraft.mined:minecraft.stone")).unwrap(), "minecraft.mined:minecraft.stone");
        assert_eq!(parse_criteria(&mut rd("mined:stone")).unwrap(), "minecraft.mined:minecraft.stone");
        assert_eq!(parse_criteria(&mut rd("minecraft.custom:minecraft.play_time")).unwrap(), "minecraft.custom:minecraft.play_time");
        let mut r = rd("bogus rest");
        let e = err_of(parse_criteria(&mut r));
        assert_eq!(e.msg, Text::tr("argument.criteria.invalid", vec![lit("bogus")]));
        assert_eq!(e.cursor, 0);
        let e = err_of(parse_criteria(&mut rd("minecraft.mined:minecraft.nope")));
        assert_eq!(e.cursor, 0);
    }

    #[test]
    fn operations() {
        assert_eq!(parse_operation(&mut rd("=")).unwrap(), Operation::Assign);
        assert_eq!(parse_operation(&mut rd("+= 1")).unwrap(), Operation::Add);
        assert_eq!(parse_operation(&mut rd("-=")).unwrap(), Operation::Sub);
        assert_eq!(parse_operation(&mut rd("*=")).unwrap(), Operation::Mul);
        assert_eq!(parse_operation(&mut rd("/=")).unwrap(), Operation::Div);
        assert_eq!(parse_operation(&mut rd("%=")).unwrap(), Operation::Mod);
        assert_eq!(parse_operation(&mut rd("<")).unwrap(), Operation::Min);
        assert_eq!(parse_operation(&mut rd(">")).unwrap(), Operation::Max);
        assert_eq!(parse_operation(&mut rd("><")).unwrap(), Operation::Swap);
        let e = err_of(parse_operation(&mut rd("")));
        assert_eq!(e.msg, Text::key("arguments.operation.invalid"));
        assert_eq!(e.cursor, 0);
        let e = err_of(parse_operation(&mut rd("?")));
        assert_eq!(e.msg, Text::key("arguments.operation.invalid"));
        assert!(e.input.is_none());
    }

    #[test]
    fn angles() {
        assert_eq!(parse_angle(&mut rd("0")).unwrap(), Angle { value: 0.0, relative: false });
        assert_eq!(parse_angle(&mut rd("~-5")).unwrap(), Angle { value: -5.0, relative: true });
        assert_eq!(parse_angle(&mut rd("~")).unwrap(), Angle { value: 0.0, relative: true });
        assert_eq!(parse_angle(&mut rd("90.5")).unwrap(), Angle { value: 90.5, relative: false });
        let e = err_of(parse_angle(&mut rd("")));
        assert_eq!(e.msg, Text::key("argument.angle.incomplete"));
        let e = err_of(parse_angle(&mut rd("x")));
        assert_eq!(e.msg, Text::key("parsing.float.expected"));
        assert_eq!(e.cursor, 0);
    }

    #[test]
    fn scoreboard_slots() {
        assert_eq!(parse_scoreboard_slot(&mut rd("list")).unwrap(), 0);
        assert_eq!(parse_scoreboard_slot(&mut rd("sidebar")).unwrap(), 1);
        assert_eq!(parse_scoreboard_slot(&mut rd("below_name")).unwrap(), 2);
        assert_eq!(parse_scoreboard_slot(&mut rd("sidebar.team.black")).unwrap(), 3);
        assert_eq!(parse_scoreboard_slot(&mut rd("sidebar.team.white")).unwrap(), 18);
        let mut r = rd("foo");
        let e = err_of(parse_scoreboard_slot(&mut r));
        assert_eq!(e.msg, Text::tr("argument.scoreboardDisplaySlot.invalid", vec![lit("foo")]));
        assert_eq!(e.cursor, 3);
    }

    #[test]
    fn slots() {
        assert_eq!(parse_slot(&mut rd("container.5")).unwrap(), 5);
        assert_eq!(parse_slot(&mut rd("weapon")).unwrap(), 98);
        assert_eq!(parse_slot(&mut rd("armor.head")).unwrap(), 103);
        assert_eq!(parse_slot(&mut rd("horse.chest")).unwrap(), 499);
        assert_eq!(parse_slot(&mut rd("contents")).unwrap(), 0);
        let mut r = rd("container.*");
        let e = err_of(parse_slot(&mut r));
        assert_eq!(e.msg, Text::tr("slot.only_single_allowed", vec![lit("container.*")]));
        assert_eq!(e.cursor, 11);
        let mut r = rd("nope");
        let e = err_of(parse_slot(&mut r));
        assert_eq!(e.msg, Text::tr("slot.unknown", vec![lit("nope")]));
        assert_eq!(e.cursor, 4);

        assert_eq!(parse_slots(&mut rd("container.*")).unwrap(), (0..54).collect::<Vec<i32>>());
        assert_eq!(parse_slots(&mut rd("armor.*")).unwrap(), vec![103, 102, 101, 100, 105]);
        assert_eq!(parse_slots(&mut rd("weapon")).unwrap(), vec![98]);
    }

    #[test]
    fn functions() {
        assert_eq!(
            parse_function(&mut rd("#minecraft:tick")).unwrap(),
            FunctionRef { tag: true, id: "minecraft:tick".into() }
        );
        assert_eq!(parse_function(&mut rd("my:fn")).unwrap(), FunctionRef { tag: false, id: "my:fn".into() });
        let mut r = rd("#Bad");
        assert!(parse_function(&mut r).is_ok()); // reads an empty id, as vanilla does
    }

    #[test]
    fn anchors() {
        assert_eq!(parse_anchor(&mut rd("feet")).unwrap(), 0);
        assert_eq!(parse_anchor(&mut rd("eyes")).unwrap(), 1);
        let mut r = rd("head");
        let e = err_of(parse_anchor(&mut r));
        assert_eq!(e.msg, Text::tr("argument.anchor.invalid", vec![lit("head")]));
        assert_eq!(e.cursor, 0);
    }

    #[test]
    fn int_ranges() {
        let ir = |s: &str| parse_int_range(&mut rd(s)).unwrap();
        assert_eq!(ir("0..5"), IntRange { min: Some(0), max: Some(5) });
        assert_eq!(ir("5"), IntRange { min: Some(5), max: Some(5) });
        assert_eq!(ir("-100.."), IntRange { min: Some(-100), max: None });
        assert_eq!(ir("..100"), IntRange { min: None, max: Some(100) });
        assert!(ir("3 ").matches(3));
        assert!(!ir("..100").matches(101));

        let e = err_of(parse_int_range(&mut rd("")));
        assert_eq!(e.msg, Text::key("argument.range.empty"));
        assert_eq!(e.cursor, 0);
        let e = err_of(parse_int_range(&mut rd("x")));
        assert_eq!(e.msg, Text::key("argument.range.empty"));
        let e = err_of(parse_int_range(&mut rd("5..1")));
        assert_eq!(e.msg, Text::key("argument.range.swapped"));
        assert_eq!(e.cursor, 0);
        let mut r = rd("1.5");
        let e = err_of(parse_int_range(&mut r));
        assert_eq!(e.msg, Text::tr("parsing.int.invalid", vec![lit("1.5")]));
        assert_eq!(e.cursor, 0);
        assert_eq!(r.cursor, 0);
        let e = err_of(parse_int_range(&mut rd("..")));
        assert_eq!(e.msg, Text::key("argument.range.empty"));
    }

    #[test]
    fn float_ranges() {
        let fr = |s: &str| parse_float_range(&mut rd(s)).unwrap();
        assert_eq!(fr("0..5.2"), FloatRange { min: Some(0.0), max: Some(5.2) });
        assert_eq!(fr("-5.4"), FloatRange { min: Some(-5.4), max: Some(-5.4) });
        assert_eq!(fr("-100.76.."), FloatRange { min: Some(-100.76), max: None });
        assert_eq!(fr("..100"), FloatRange { min: None, max: Some(100.0) });
        let mut r = rd("1-2");
        let e = err_of(parse_float_range(&mut r));
        assert_eq!(e.msg, Text::tr("parsing.double.invalid", vec![lit("1-2")]));
        assert_eq!(e.cursor, 0);
        let e = err_of(parse_float_range(&mut rd("2..1")));
        assert_eq!(e.msg, Text::key("argument.range.swapped"));
    }

    #[test]
    fn gamemodes() {
        assert_eq!(parse_gamemode(&mut rd("survival")).unwrap(), 0);
        assert_eq!(parse_gamemode(&mut rd("creative")).unwrap(), 1);
        assert_eq!(parse_gamemode(&mut rd("adventure")).unwrap(), 2);
        assert_eq!(parse_gamemode(&mut rd("spectator")).unwrap(), 3);
        let mut r = rd("Survival");
        let e = err_of(parse_gamemode(&mut r));
        assert_eq!(e.msg, Text::tr("argument.gamemode.invalid", vec![lit("Survival")]));
        assert_eq!(e.cursor, 8);
    }

    #[test]
    fn times() {
        assert_eq!(parse_time(&mut rd("5s"), 0).unwrap(), 100);
        assert_eq!(parse_time(&mut rd("1d"), 0).unwrap(), 24000);
        assert_eq!(parse_time(&mut rd("3"), 0).unwrap(), 3);
        assert_eq!(parse_time(&mut rd("3t"), 0).unwrap(), 3);
        assert_eq!(parse_time(&mut rd("2.5s"), 0).unwrap(), 50);
        assert_eq!(parse_time(&mut rd("0.5t"), 0).unwrap(), 1);
        assert_eq!(parse_time(&mut rd("-0.5t"), -10).unwrap(), 0);
        assert_eq!(parse_time(&mut rd("-2.5t"), -10).unwrap(), -2);
        let mut r = rd("1x");
        let e = err_of(parse_time(&mut r, 0));
        assert_eq!(e.msg, Text::key("argument.time.invalid_unit"));
        assert_eq!(e.cursor, 2);
        let mut r = rd("-1");
        let e = err_of(parse_time(&mut r, 0));
        assert_eq!(e.msg, Text::tr("argument.time.tick_count_too_low", vec![lit(0), lit(-1)]));
        assert_eq!(e.cursor, 2);
    }

    #[test]
    fn enums() {
        assert_eq!(parse_template_mirror(&mut rd("none")).unwrap(), 0);
        assert_eq!(parse_template_mirror(&mut rd("front_back")).unwrap(), 2);
        let mut r = rd("LEFT");
        let e = err_of(parse_template_mirror(&mut r));
        assert_eq!(e.msg, Text::tr("argument.enum.invalid", vec![lit("LEFT")]));
        assert_eq!(e.cursor, 4);

        assert_eq!(parse_template_rotation(&mut rd("180")).unwrap(), 2);
        assert_eq!(parse_template_rotation(&mut rd("counterclockwise_90")).unwrap(), 3);
        let e = err_of(parse_template_rotation(&mut rd("clockwise_180")));
        assert_eq!(e.msg, Text::tr("argument.enum.invalid", vec![lit("clockwise_180")]));

        assert_eq!(parse_heightmap(&mut rd("world_surface")).unwrap(), 1);
        assert_eq!(parse_heightmap(&mut rd("motion_blocking_no_leaves")).unwrap(), 5);
        let e = err_of(parse_heightmap(&mut rd("WORLD_SURFACE")));
        assert_eq!(e.msg, Text::tr("argument.enum.invalid", vec![lit("WORLD_SURFACE")]));
        assert!(parse_heightmap(&mut rd("world_surface_wg")).is_err());

        assert_eq!(parse_swing_animation(&mut rd("stab")).unwrap(), 2);
        assert_eq!(parse_swing_animation(&mut rd("whack")).unwrap(), 1);
        let e = err_of(parse_swing_animation(&mut rd("bad")));
        assert_eq!(e.msg, Text::tr("argument.swing_animation.invalid", vec![lit("bad")]));
    }

    #[test]
    fn id_or_inline_and_providers() {
        // Unknown registry: any id.
        assert_eq!(
            parse_id_or_inline(&mut rd("foo:bar"), "minecraft:loot_table").unwrap(),
            IdOrInline::Id("foo:bar".into())
        );
        // Known registry: id must exist.
        assert_eq!(
            parse_id_or_inline(&mut rd("minecraft:sharpness"), "minecraft:enchantment").unwrap(),
            IdOrInline::Id("minecraft:sharpness".into())
        );
        let mut r = rd("minecraft:nope");
        let e = err_of(parse_id_or_inline(&mut r, "minecraft:enchantment"));
        assert_eq!(
            e.msg,
            Text::tr(
                "argument.resource_or_id.no_such_element",
                vec![lit("minecraft:nope"), lit("minecraft:enchantment")]
            )
        );
        assert_eq!(e.cursor, 14);
        // Bare numbers are ids in vanilla (the id alternative comes first).
        assert_eq!(parse_provider(&mut rd("5"), true).unwrap(), IdOrInline::Id("minecraft:5".into()));
        assert_eq!(parse_provider(&mut rd("foo"), false).unwrap(), IdOrInline::Id("minecraft:foo".into()));
    }

    #[test]
    fn resource_or_tag() {
        let ok = parse_resource_or_tag(&mut rd("minecraft:sharpness"), "minecraft:enchantment").unwrap();
        assert_eq!(ok, ResourceOrTag { tag: false, id: "minecraft:sharpness".into() });
        let mut r = rd("minecraft:nope");
        let e = err_of(parse_resource_or_tag(&mut r, "minecraft:enchantment"));
        assert_eq!(e.msg, Text::tr("argument.resource.not_found", vec![lit("minecraft:nope"), lit("minecraft:enchantment")]));
        assert_eq!(e.cursor, 14);
        // Tag errors keep the id end, and the reader returns to '#'.
        let mut r = rd("#minecraft:nope");
        let e = err_of(parse_resource_or_tag(&mut r, "minecraft:enchantment"));
        assert_eq!(e.msg, Text::tr("argument.resource_tag.not_found", vec![lit("minecraft:nope"), lit("minecraft:enchantment")]));
        assert_eq!(e.cursor, 15);
        assert_eq!(r.cursor, 0);
        // Keys are not checked.
        assert_eq!(
            parse_resource_or_tag_key(&mut rd("#minecraft:whatever"), "minecraft:enchantment").unwrap(),
            ResourceOrTag { tag: true, id: "minecraft:whatever".into() }
        );
        assert_eq!(
            parse_resource_key(&mut rd("minecraft:whatever"), "minecraft:enchantment").unwrap(),
            "minecraft:whatever"
        );
    }

    #[test]
    fn resource_and_selector() {
        assert_eq!(parse_resource(&mut rd("minecraft:sharpness"), "minecraft:enchantment").unwrap(), "minecraft:sharpness");
        assert_eq!(parse_resource(&mut rd("loot:any"), "minecraft:loot_table").unwrap(), "loot:any");
        assert_eq!(
            parse_resource_selector(&mut rd("sha*"), "minecraft:enchantment").unwrap(),
            "minecraft:sha*"
        );
        assert_eq!(
            parse_resource_selector(&mut rd("minecraft:*"), "minecraft:enchantment").unwrap(),
            "minecraft:*"
        );
        let mut r = rd("nope*");
        let e = err_of(parse_resource_selector(&mut r, "minecraft:enchantment"));
        assert_eq!(e.msg, Text::tr("argument.resource_selector.not_found", vec![lit("minecraft:nope*"), lit("minecraft:enchantment")]));
        assert_eq!(e.cursor, 5);
        assert!(wildcard_match("minecraft:abc", "*:a?c"));
        assert!(!wildcard_match("minecraft:abc", "*:ab"));
    }

    #[test]
    fn uuids() {
        assert_eq!(
            parse_uuid(&mut rd("dd12be42-52a9-4a91-a8a1-11c01849e498")).unwrap(),
            0xdd12be42_52a9_4a91_a8a1_11c01849e498u128
        );
        // Lenient group widths, as UUID.fromString.
        assert_eq!(
            parse_uuid(&mut rd("1-2-3-4-5")).unwrap(),
            ((0x0000_0001_0002_0003u128) << 64) | 0x0004_0000_0000_0005u128
        );
        let mut r = rd("dd12be42-52a9-4a91-a8a1-11c01849e498 x");
        parse_uuid(&mut r).unwrap();
        assert_eq!(r.cursor, 36);
        let mut r = rd("xyz");
        let e = err_of(parse_uuid(&mut r));
        assert_eq!(e.msg, Text::key("argument.uuid.invalid"));
        assert_eq!(e.cursor, 0);
        let e = err_of(parse_uuid(&mut rd("1-2-3-4")));
        assert_eq!(e.msg, Text::key("argument.uuid.invalid"));
    }

    #[test]
    fn particles() {
        let p = parse_particle(&mut rd("minecraft:flame")).unwrap();
        assert_eq!(p, ParticleArg { id: "minecraft:flame".into(), options: None });
        let mut r = rd("minecraft:nope");
        let e = err_of(parse_particle(&mut r));
        assert_eq!(e.msg, Text::tr("particle.notFound", vec![lit("minecraft:nope")]));
        assert_eq!(e.cursor, 14);
    }
}
