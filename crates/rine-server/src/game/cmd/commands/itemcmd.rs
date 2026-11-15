//! /item and /loot (`ItemCommands`, `item/*Accessor`, `LootCommand`,
//! `SlotRanges`, `SlotCollection`, `ItemProvider`, `LootItemFunctions`).
//!
//! Slots follow `Player.getSlot` and `ItemEntity.getSlot`. Player inventory
//! ids map onto `player::Inventory` (window-0 numbering). Block containers
//! need accessors in `containers.rs` that do not exist yet: the vanilla checks
//! run, then any container slot that would be touched fails with a Rine error.
//! Loot is evaluated only for block loot tables (`loot.rs`).

use crate::game::cmd::{feedback, lit, Commands, Ctx, SimpleFn};
use crate::game::containers::craft_result;
use crate::game::entity::{self, Kind};
use crate::game::items;
use crate::game::loot;
use crate::game::player::{max_stack, Player, Stack};
use crate::game::track;
use crate::game::Game;
use crate::out;
use rine_command::args::misc::{self, IdOrInline};
use rine_command::args::Arg;
use rine_command::reader::Reader;
use rine_command::{CmdError, CmdResult};
use rine_protocol::nbt::Tag;
use rine_protocol::text::{hover_item, hover_text, Text};
use std::collections::HashSet;
use std::sync::OnceLock;

const NOT_CONTAINER_TARGET: &str = "commands.item.target.not_a_container";
const NOT_CONTAINER_SOURCE: &str = "commands.item.source.not_a_container";

pub fn register(c: &mut Commands) {
    let tree = c.tree.clone();
    for n in tree.nodes.iter().filter(|n| n.executable) {
        let f: SimpleFn = match n.path.split(' ').next() {
            Some("item") => item_cmd as SimpleFn,
            Some("loot") => loot_cmd as SimpleFn,
            _ => continue,
        };
        c.exec(n.path, f);
    }
}

/// Failure for a branch that Rine cannot model yet.
fn unmodelled(what: &str) -> CmdError {
    CmdError::new(Text::key("command.failed").on_hover(hover_text(&Text::plain(format!("Rine: {what} is not implemented yet")))))
}

// ---------------------------------------------------------------------------
// Response tracking (`CommandResponseTracker`, NON_ZERO element type only)
// ---------------------------------------------------------------------------

struct Tracker<E: Clone> {
    total: i32,
    nonzero: usize,
    /// The element, when exactly one non-zero element was tracked.
    only: Option<E>,
}

impl<E: Clone> Tracker<E> {
    fn new() -> Self {
        Self { total: 0, nonzero: 0, only: None }
    }
    fn track(&mut self, e: E, value: i32) {
        self.total += value;
        if value != 0 {
            self.nonzero += 1;
            self.only = if self.nonzero == 1 { Some(e) } else { None };
        }
    }
}

// ---------------------------------------------------------------------------
// Item providers (`ArrayItemProvider`, `Cycled`, `FallbackItemProvider`)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// `ItemProvider.of`: each item once.
    Of,
    /// `ItemProvider.cycle`: repeats forever.
    Cycle,
    /// `ItemProvider.of(...).orElseProvide(EMPTY)`: never runs out.
    OrEmpty,
}

struct Provider {
    items: Vec<Stack>,
    pos: usize,
    mode: Mode,
}

impl Provider {
    fn restart(&mut self) {
        self.pos = 0;
    }
    fn has_next(&self) -> bool {
        match self.mode {
            Mode::Of => self.pos < self.items.len(),
            Mode::Cycle => !self.items.is_empty(),
            Mode::OrEmpty => true,
        }
    }
    fn next(&mut self) -> Stack {
        match self.mode {
            Mode::Of | Mode::OrEmpty => {
                if self.pos < self.items.len() {
                    self.pos += 1;
                    self.items[self.pos - 1]
                } else {
                    Stack::EMPTY
                }
            }
            Mode::Cycle => {
                let s = self.items[self.pos % self.items.len()];
                self.pos += 1;
                s
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Slots
// ---------------------------------------------------------------------------

/// Equipment slots that validate what they accept (`createEquipmentSlotAccess`).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Equip {
    Chest,
    Legs,
    Feet,
}

#[derive(Clone, Copy, Debug)]
enum Slot {
    /// Index in `Player.inv.slots` (window-0 numbering).
    Inv { player: usize, idx: usize, equip: Option<Equip> },
    Cursor { player: usize },
    /// Body and saddle: no storage, only empty stacks can be set.
    Virtual,
    /// `ItemEntity.getSlot(0)`.
    Contents { eid: i32 },
}

/// Armour item kind from its name (no equipment data is loaded).
fn equip_of(item: u32) -> Option<Equip> {
    let name = rine_data::get().items.get(item)?.name.as_str();
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    if n.ends_with("_chestplate") || n == "elytra" {
        Some(Equip::Chest)
    } else if n.ends_with("_leggings") {
        Some(Equip::Legs)
    } else if n.ends_with("_boots") {
        Some(Equip::Feet)
    } else {
        None
    }
}

fn player_ref(g: &Game, p: usize) -> Option<&Player> {
    g.players.get(p)?.as_ref()
}

/// `Entity.getSlot` for the entities Rine has: players and item entities.
fn entity_slot(g: &Game, eid: i32, id: i32) -> Option<Slot> {
    if let Some(&p) = g.by_eid.get(&eid) {
        let held = player_ref(g, p)?.held as usize;
        let inv = |idx: usize, equip: Option<Equip>| Some(Slot::Inv { player: p, idx, equip });
        return match id {
            0..=8 => inv(36 + id as usize, None),
            9..=35 => inv(id as usize, None),
            98 => inv(36 + held, None),
            99 => inv(45, None),
            100 => inv(8, Some(Equip::Feet)),
            101 => inv(7, Some(Equip::Legs)),
            102 => inv(6, Some(Equip::Chest)),
            103 => inv(5, None),
            105 | 106 => Some(Slot::Virtual),
            499 => Some(Slot::Cursor { player: p }),
            500..=503 => inv(1 + (id - 500) as usize, None),
            _ => None,
        };
    }
    match g.entities.get(eid).map(|e| &e.kind) {
        Some(Kind::Item { .. }) if id == 0 => Some(Slot::Contents { eid }),
        _ => None,
    }
}

fn entity_slots(g: &Game, eid: i32, ids: &[i32]) -> Vec<Slot> {
    ids.iter().filter_map(|&i| entity_slot(g, eid, i)).collect()
}

fn get_slot(g: &Game, s: Slot) -> Stack {
    match s {
        Slot::Inv { player, idx, .. } => player_ref(g, player).map(|p| p.inv.slots[idx]).unwrap_or(Stack::EMPTY),
        Slot::Cursor { player } => player_ref(g, player).map(|p| p.inv.cursor).unwrap_or(Stack::EMPTY),
        Slot::Virtual => Stack::EMPTY,
        Slot::Contents { eid } => match g.entities.slot_of(eid).map(|i| &g.entities.list[i].kind) {
            Some(Kind::Item { stack, .. }) => *stack,
            _ => Stack::EMPTY,
        },
    }
}

/// `SlotAccess.set`: false when the slot rejects the stack.
fn set_slot(g: &mut Game, s: Slot, v: Stack) -> bool {
    match s {
        Slot::Inv { player, idx, equip } => {
            if let Some(e) = equip {
                if !v.is_empty() && equip_of(v.item) != Some(e) {
                    return false;
                }
            }
            let Some(p) = g.player(player) else { return false };
            p.inv.slots[idx] = v;
            if (1..=4).contains(&idx) {
                let result = craft_result(2, &p.inv.slots[1..5]);
                p.inv.slots[0] = result;
            }
            true
        }
        Slot::Cursor { player } => {
            let Some(p) = g.player(player) else { return false };
            p.inv.cursor = v;
            true
        }
        Slot::Virtual => v.is_empty(),
        Slot::Contents { eid } => {
            let Some(i) = g.entities.slot_of(eid) else { return false };
            let e = &mut g.entities.list[i];
            let Kind::Item { stack, .. } = &mut e.kind else { return false };
            *stack = v;
            if v.is_empty() {
                // ItemEntity with an empty stack is discarded on its next tick.
                e.removed = true;
            } else {
                // Same metadata entry the spawn path sends (index 8).
                let pkt = out::Meta::new(eid).item(8, &v).finish();
                g.broadcast.push(out::frame(pkt));
            }
            true
        }
    }
}

/// Send the inventory to a player and refresh the held item for others.
fn sync_player(g: &mut Game, slot: usize) {
    if let Some(p) = g.player(slot) {
        let f = p.inventory_packet();
        p.send(f);
    }
    track::equipment_changed(g, slot);
}

fn sync_entity(g: &mut Game, eid: i32) {
    if let Some(&p) = g.by_eid.get(&eid) {
        sync_player(g, p);
    }
}

/// `SlotRanges` names as `(name, slot ids)`, parsed by the rine-command grammar.
struct SlotName {
    name: String,
    ids: Vec<i32>,
}

fn slot_arg(x: &Ctx, n: &str) -> CmdResult<SlotName> {
    let raw = match x.arg(n) {
        Arg::IdOrInline(IdOrInline::Id(s)) => s.clone(),
        _ => return Err(unmodelled("slot sources defined in data packs")),
    };
    let name = raw.strip_prefix("minecraft:").unwrap_or(&raw).to_string();
    let ids = misc::parse_slots(&mut Reader::new(&name)).map_err(|_| CmdError::tr("slot.unknown", vec![lit(&name)]))?;
    Ok(SlotName { name, ids })
}

fn no_such_slot_target(slot: &SlotName) -> CmdError {
    CmdError::tr("commands.item.target.no_such_slot", vec![lit(&slot.name)])
}

// ---------------------------------------------------------------------------
// Block containers (`BlockItemAccessor.getContainer`)
// ---------------------------------------------------------------------------

/// Slot count of block entities that implement `Container`; `None` otherwise.
fn container_size(block: &str) -> Option<usize> {
    let name = block.strip_prefix("minecraft:").unwrap_or(block);
    Some(match name {
        "chest" | "trapped_chest" | "barrel" => 27,
        "furnace" | "blast_furnace" | "smoker" => 3,
        "hopper" | "brewing_stand" => 5,
        "dispenser" | "dropper" | "crafter" => 9,
        "campfire" | "soul_campfire" => 4,
        "chiseled_bookshelf" => 6,
        "decorated_pot" | "jukebox" | "lectern" => 1,
        s if s.ends_with("shulker_box") => 27,
        _ => return None,
    })
}

fn container_size_at(g: &Game, pos: [i32; 3], key: &'static str) -> CmdResult<usize> {
    let state = g.world.get_block(pos[0], pos[1], pos[2]);
    let name = rine_data::get().blocks.block_of(state).name.as_str();
    container_size(name).ok_or_else(|| CmdError::tr(key, vec![lit(pos[0]), lit(pos[1]), lit(pos[2])]))
}

/// Number of listed slot ids that exist in the container at `pos`.
fn container_valid(g: &Game, pos: [i32; 3], key: &'static str, ids: &[i32]) -> CmdResult<usize> {
    let size = container_size_at(g, pos, key)?;
    Ok(ids.iter().filter(|&&i| i >= 0 && (i as usize) < size).count())
}

// ---------------------------------------------------------------------------
// Item stacks and display
// ---------------------------------------------------------------------------

/// `ItemStack.getDisplayName`: name in square brackets with the item hover.
/// Rarity is not modelled, so every item uses the common colour.
fn item_display(s: &Stack) -> Text {
    let name = rine_data::get().items.get(s.item).map(|i| i.name.as_str()).unwrap_or("minecraft:air");
    let inner = Text::key(feedback::item_description_id(name));
    let t = Text::tr("chat.square_brackets", vec![inner]);
    if s.is_empty() {
        t
    } else {
        t.color("white").on_hover(hover_item(name, s.count, None))
    }
}

/// `ItemArgument.createItemStack(count)`: component data is not stored.
fn plain_stack(name: &str, plain: bool, count: i32) -> CmdResult<Stack> {
    if !plain {
        return Err(unmodelled("item data components"));
    }
    let id = items::id(name).ok_or_else(|| CmdError::tr("argument.item.id.invalid", vec![lit(name)]))?;
    let max = max_stack(id);
    if count > max {
        let full = rine_data::get().items.get(id).map(|i| i.name.clone()).unwrap_or_default();
        return Err(CmdError::tr("arguments.item.overstacked", vec![lit(full), lit(max)]));
    }
    Ok(Stack::new(id, count))
}

fn with_item(x: &Ctx) -> CmdResult<Stack> {
    let (name, plain) = match x.arg("item") {
        Arg::Item(i) => (i.item.clone(), i.components.is_empty()),
        _ => return Err(CmdError::key("argument.item.id.invalid")),
    };
    plain_stack(&name, plain, x.int_or("count", 1))
}

// ---------------------------------------------------------------------------
// Item modifiers (`LootItemFunction` subset on `player::Stack`)
// ---------------------------------------------------------------------------

enum Num {
    Const(f64),
    Uniform(f64, f64),
}

enum Func {
    SetCount { count: Num, add: bool },
    LimitCount { min: Option<Num>, max: Option<Num> },
    SetDamage { value: Num, add: bool },
    SetItem(u32),
    Discard,
    Sequence(Vec<Func>),
}

/// `RandomSource` stand-in: a counter-based stream seeded from the tick.
fn rng_seed(g: &Game) -> u64 {
    g.tick.wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

fn rnd(rng: &mut u64) -> f64 {
    *rng = rng.wrapping_add(1);
    entity::rand_f64(*rng)
}

fn eval_f(n: &Num, rng: &mut u64) -> f64 {
    match n {
        Num::Const(v) => *v,
        Num::Uniform(a, b) => a + rnd(rng) * (b - a),
    }
}

/// `UniformInt.getInt`: inclusive bounds.
fn eval_i(n: &Num, rng: &mut u64) -> i32 {
    match n {
        Num::Const(v) => *v as i32,
        Num::Uniform(a, b) => {
            let lo = *a as i32;
            let hi = *b as i32;
            if hi <= lo {
                lo
            } else {
                lo + (rnd(rng) * f64::from(hi - lo + 1)).floor() as i32
            }
        }
    }
}

/// Path of a registry id in the `minecraft` namespace (the codec default).
/// `None` for other namespaces.
fn registry_name(id: &str) -> Option<&str> {
    match id.split_once(':') {
        Some(("minecraft", n)) => Some(n),
        None => Some(id),
        _ => None,
    }
}

fn num_of(t: &Tag) -> Option<f64> {
    Some(match t {
        Tag::Byte(v) => f64::from(*v),
        Tag::Short(v) => f64::from(*v),
        Tag::Int(v) => f64::from(*v),
        Tag::Long(v) => *v as f64,
        Tag::Float(v) => f64::from(*v),
        Tag::Double(v) => *v,
        _ => return None,
    })
}

/// Number provider: a plain number, `minecraft:constant` or `minecraft:uniform`.
fn parse_num(t: &Tag) -> CmdResult<Num> {
    if let Some(v) = num_of(t) {
        return Ok(Num::Const(v));
    }
    let ty = match t.get("type") {
        Some(Tag::String(s)) => s.as_str(),
        _ => return Err(unmodelled("number provider without a type")),
    };
    let field = |k: &str| t.get(k).and_then(num_of).ok_or_else(|| unmodelled(&format!("number provider field {k}")));
    match registry_name(ty) {
        Some("constant") => Ok(Num::Const(field("value")?)),
        // Int uniform is inclusive on both ends; float uniform ends exclusive.
        Some("uniform") => {
            let lo = field("min_inclusive")?;
            let hi = match t.get("max_inclusive").or_else(|| t.get("max_exclusive")).and_then(num_of) {
                Some(v) => v,
                None => return Err(unmodelled("uniform provider without a maximum")),
            };
            Ok(Num::Uniform(lo, hi))
        }
        _ => Err(unmodelled(&format!("number provider {ty}"))),
    }
}

fn num_field(t: &Tag, key: &str) -> CmdResult<Num> {
    match t.get(key) {
        Some(v) => parse_num(v),
        None => Err(unmodelled(&format!("item modifier without {key}"))),
    }
}

fn flag(t: &Tag, key: &str) -> bool {
    matches!(t.get(key), Some(Tag::Byte(v)) if *v != 0)
}

/// Inline item modifier: a function object, or a list of them (sequence).
fn parse_func(t: &Tag) -> CmdResult<Func> {
    match t {
        Tag::List(v) => Ok(Func::Sequence(v.iter().map(parse_func).collect::<CmdResult<Vec<_>>>()?)),
        Tag::Compound(_) => {
            if let Some(c) = t.get("conditions") {
                if !matches!(c, Tag::List(v) if v.is_empty()) {
                    return Err(unmodelled("loot conditions in item modifiers"));
                }
            }
            let ty = match t.get("function") {
                Some(Tag::String(s)) => s.as_str(),
                _ => return Err(unmodelled("item modifier without a function")),
            };
            let name = match registry_name(ty) {
                Some(n) => n,
                None => return Err(unmodelled(&format!("item modifier {ty}"))),
            };
            match name {
                "set_count" => Ok(Func::SetCount { count: num_field(t, "count")?, add: flag(t, "add") }),
                "set_damage" => Ok(Func::SetDamage { value: num_field(t, "damage")?, add: flag(t, "add") }),
                "limit_count" => {
                    let limit = match t.get("limit") {
                        Some(l) => l,
                        None => return Err(unmodelled("limit_count without limit")),
                    };
                    let bound = |k: &str| -> CmdResult<Option<Num>> {
                        match limit.get(k) {
                            Some(v) => Ok(Some(parse_num(v)?)),
                            None => Ok(None),
                        }
                    };
                    Ok(Func::LimitCount { min: bound("min")?, max: bound("max")? })
                }
                "set_item" => match t.get("item") {
                    Some(Tag::String(id)) => {
                        let id = items::id(id).ok_or_else(|| CmdError::tr("argument.item.id.invalid", vec![lit(id)]))?;
                        Ok(Func::SetItem(id))
                    }
                    _ => Err(unmodelled("set_item without an item")),
                },
                "discard" => Ok(Func::Discard),
                "sequence" => match t.get("functions") {
                    Some(Tag::List(v)) => Ok(Func::Sequence(v.iter().map(parse_func).collect::<CmdResult<Vec<_>>>()?)),
                    _ => Err(unmodelled("sequence without functions")),
                },
                other => Err(unmodelled(&format!("item modifier function {other}"))),
            }
        }
        _ => Err(unmodelled("item modifier that is not a function")),
    }
}

fn modifier_arg(x: &Ctx) -> CmdResult<Func> {
    match x.arg("modifier") {
        Arg::IdOrInline(IdOrInline::Inline(t)) => parse_func(t),
        _ => Err(unmodelled("item modifiers defined in data packs")),
    }
}

/// `LootItemFunction.run` on a stack, without the final size limit.
fn run(f: &Func, s: Stack, rng: &mut u64) -> Stack {
    match f {
        Func::SetCount { count, add } => {
            let base = if *add { s.count } else { 0 };
            s.with_count(base + eval_i(count, rng))
        }
        Func::LimitCount { min, max } => {
            let lo = min.as_ref().map(|m| eval_i(m, rng));
            let hi = max.as_ref().map(|m| eval_i(m, rng));
            let n = match (lo, hi) {
                (Some(lo), Some(hi)) => s.count.max(lo).min(hi),
                (Some(lo), None) => s.count.max(lo),
                (None, Some(hi)) => s.count.min(hi),
                (None, None) => s.count,
            };
            s.with_count(n)
        }
        Func::SetDamage { value, add } => {
            let max = items::info(s.item).map(|i| i.max_damage).unwrap_or(0);
            if s.is_empty() || max <= 0 {
                return s;
            }
            let maxf = max as f32;
            let base = if *add { 1.0f32 - (s.damage as f32) / maxf } else { 0.0 };
            let v = eval_f(value, rng) as f32;
            let pct = 1.0f32 - (v + base).clamp(0.0, 1.0);
            let dmg = ((pct * maxf).floor() as i32).clamp(0, max);
            Stack::damaged(s.item, s.count, dmg as u16)
        }
        Func::SetItem(id) => {
            if s.is_empty() {
                s
            } else {
                Stack::damaged(*id, s.count, s.damage)
            }
        }
        Func::Discard => Stack::EMPTY,
        Func::Sequence(v) => v.iter().fold(s, |acc, f| run(f, acc, rng)),
    }
}

/// `applyModifier`: run the function, then `limitSize(maxStackSize)`.
fn apply_modifier(f: &Func, s: Stack, rng: &mut u64) -> Stack {
    let r = run(f, s, rng);
    let max = max_stack(r.item);
    if r.count > max {
        r.with_count(max)
    } else {
        r
    }
}

// ---------------------------------------------------------------------------
// /item
// ---------------------------------------------------------------------------

enum Target {
    Entities(Vec<i32>),
    Block([i32; 3]),
}

fn resolve_target(x: &mut Ctx, kind: &str, name: &str) -> CmdResult<Target> {
    if kind == "entity" {
        Ok(Target::Entities(x.entities(name)?))
    } else {
        Ok(Target::Block(x.block_pos(name)?))
    }
}

fn item_cmd(x: &mut Ctx) -> CmdResult<i32> {
    let w: Vec<&'static str> = x.path().split(' ').collect();
    let kind = w[2];
    match w[1] {
        "modify" => item_modify(x, kind),
        verb => {
            let from = if w.get(5) == Some(&"from") { Some(w[6]) } else { None };
            item_set(x, verb, kind, from)
        }
    }
}

/// `ItemCommands.setItems`: replace, fill and override.
fn item_set(x: &mut Ctx, verb: &str, kind: &str, from: Option<&str>) -> CmdResult<i32> {
    let slot = slot_arg(x, "slots")?;
    let target = resolve_target(x, kind, "target")?;
    let (items, known) = match from {
        Some(sk) => (source_items(x, sk, &slot)?, None),
        None => {
            let s = with_item(x)?;
            (vec![s], Some(s))
        }
    };
    let mode = match verb {
        "fill" => Mode::Cycle,
        "override" => Mode::OrEmpty,
        _ => Mode::Of,
    };
    let mut prov = Provider { items, pos: 0, mode };
    let mut selected = 0usize;
    let mut tr: Tracker<i32> = Tracker::new();

    match target {
        Target::Entities(eids) => {
            for eid in eids {
                let slots = entity_slots(x.g, eid, &slot.ids);
                prov.restart();
                let mut count = 0;
                for s in slots {
                    if !prov.has_next() {
                        break;
                    }
                    selected += 1;
                    if set_slot(x.g, s, prov.next()) {
                        count += 1;
                    }
                }
                tr.track(eid, count);
                if count > 0 {
                    sync_entity(x.g, eid);
                }
            }
        }
        Target::Block(pos) => {
            if container_valid(x.g, pos, NOT_CONTAINER_TARGET, &slot.ids)? > 0 {
                return Err(unmodelled("writing block container slots"));
            }
        }
    }

    if selected == 0 {
        return Err(no_such_slot_target(&slot));
    }
    let total = tr.total;
    let nonzero = tr.nonzero;
    if nonzero == 0 {
        return Err(match &known {
            Some(s) => CmdError::tr("commands.item.target.failed.known_item", vec![item_display(s)]),
            None => CmdError::key("commands.item.target.failed"),
        });
    }
    let name_of = |x: &Ctx| tr.only.map(|e| x.name_of(e));
    let text = match (&known, nonzero) {
        (Some(s), 1) => Text::tr(
            "commands.item.entity.replace.success.single.known_item",
            vec![lit(total), name_of(x).unwrap_or_else(|| Text::plain("")), item_display(s)],
        ),
        (Some(s), _) => Text::tr("commands.item.entity.replace.success.multiple.known_item", vec![lit(nonzero), item_display(s)]),
        (None, 1) => Text::tr("commands.item.entity.replace.success.single", vec![lit(total), name_of(x).unwrap_or_else(|| Text::plain(""))]),
        (None, _) => Text::tr("commands.item.entity.replace.success.multiple", vec![lit(nonzero)]),
    };
    x.success(text, true);
    Ok(total)
}

/// `ItemCommands.getItems`: copies of the source slots, with the optional modifier.
fn source_items(x: &mut Ctx, kind: &str, slot: &SlotName) -> CmdResult<Vec<Stack>> {
    let mut items: Vec<Stack> = Vec::new();
    if kind == "entity" {
        let eids = x.entities("source")?;
        for eid in eids {
            for s in entity_slots(x.g, eid, &slot.ids) {
                items.push(get_slot(x.g, s));
            }
        }
    } else {
        let pos = x.block_pos("source")?;
        if container_valid(x.g, pos, NOT_CONTAINER_SOURCE, &slot.ids)? > 0 {
            return Err(unmodelled("reading block container slots"));
        }
    }
    if x.has("modifier") {
        let m = modifier_arg(x)?;
        let mut rng = rng_seed(x.g);
        items = items.into_iter().map(|s| apply_modifier(&m, s, &mut rng)).collect();
    }
    if items.is_empty() {
        return Err(CmdError::tr("commands.item.source.no_such_slot", vec![lit(&slot.name)]));
    }
    Ok(items)
}

/// `ItemCommands.modifyItems`.
fn item_modify(x: &mut Ctx, kind: &str) -> CmdResult<i32> {
    let slot = slot_arg(x, "slots")?;
    let target = resolve_target(x, kind, "target")?;
    let m = modifier_arg(x)?;
    let mut rng = rng_seed(x.g);
    let mut selected = 0usize;
    let mut tr: Tracker<i32> = Tracker::new();

    match target {
        Target::Entities(eids) => {
            for eid in eids {
                let mut updated = 0;
                for s in entity_slots(x.g, eid, &slot.ids) {
                    let cur = get_slot(x.g, s);
                    if cur.is_empty() {
                        continue;
                    }
                    selected += 1;
                    let new = apply_modifier(&m, cur, &mut rng);
                    if set_slot(x.g, s, new) {
                        updated += 1;
                    }
                }
                tr.track(eid, updated);
                if updated > 0 {
                    sync_entity(x.g, eid);
                }
            }
        }
        Target::Block(pos) => {
            if container_valid(x.g, pos, NOT_CONTAINER_TARGET, &slot.ids)? > 0 {
                return Err(unmodelled("modifying block container slots"));
            }
        }
    }

    if selected == 0 {
        return Err(no_such_slot_target(&slot));
    }
    let total = tr.total;
    let nonzero = tr.nonzero;
    if nonzero == 0 {
        return Err(CmdError::key("commands.item.target.failed"));
    }
    let text = match (nonzero, tr.only) {
        (1, Some(e)) => Text::tr("commands.item.entity.modify.success.single", vec![lit(total), x.name_of(e)]),
        _ => Text::tr("commands.item.entity.modify.success.multiple", vec![lit(nonzero)]),
    };
    x.success(text, true);
    Ok(total)
}

// ---------------------------------------------------------------------------
// /loot
// ---------------------------------------------------------------------------

fn loot_cmd(x: &mut Ctx) -> CmdResult<i32> {
    let w: Vec<&'static str> = x.path().split(' ').collect();
    // Index of the source keyword (loot | kill | mine | fish).
    let at = if w[1] == "replace" {
        if x.has("count") {
            6
        } else {
            5
        }
    } else {
        3
    };
    let (drops, table) = loot_source(x, &w, at)?;
    let tr = loot_output(x, &w, &drops)?;
    let total = tr.total;
    let nonzero = tr.nonzero as i32;
    let text = match (tr.only, table) {
        (Some(d), Some(t)) => Text::tr("commands.drop.success.single_with_table", vec![lit(d.count), item_display(&d), lit(t)]),
        (Some(d), None) => Text::tr("commands.drop.success.single", vec![lit(d.count), item_display(&d)]),
        (None, Some(t)) => Text::tr("commands.drop.success.multiple_with_table", vec![lit(nonzero), lit(t)]),
        (None, None) => Text::tr("commands.drop.success.multiple", vec![lit(nonzero)]),
    };
    x.success(text, false);
    Ok(total)
}

/// Drops and the loot table id they come from (`None`: no table in messages).
fn loot_source(x: &mut Ctx, w: &[&str], at: usize) -> CmdResult<(Vec<Stack>, Option<String>)> {
    match w[at] {
        "kill" => {
            let eid = x.entity("target")?;
            if x.g.by_eid.contains_key(&eid) {
                // entities/player has no pools: an empty table.
                return Ok((Vec::new(), Some("minecraft:entities/player".to_string())));
            }
            Err(CmdError::tr("commands.drop.no_loot_table.entity", vec![x.name_of(eid)]))
        }
        "mine" => {
            let pos = x.block_pos("pos")?;
            let tool = tool_arg(x, w, at + 1)?;
            mine(x.g, pos, tool.item)
        }
        "fish" => {
            x.block_pos("pos")?;
            tool_arg(x, w, at + 2)?;
            Err(unmodelled("fishing loot tables (gameplay/fishing) are not loaded"))
        }
        _ => Err(unmodelled("loot tables other than block loot tables are not loaded")),
    }
}

/// The tool argument, or the hand named by a literal after `pos` (`pos_i`).
fn tool_arg(x: &mut Ctx, w: &[&str], pos_i: usize) -> CmdResult<Stack> {
    if x.has("tool") {
        let (name, plain) = match x.arg("tool") {
            Arg::Item(i) => (i.item.clone(), i.components.is_empty()),
            _ => return Err(CmdError::key("argument.item.id.invalid")),
        };
        return plain_stack(&name, plain, 1);
    }
    match w.get(pos_i + 1).copied() {
        Some("mainhand") => hand(x, false),
        Some("offhand") => hand(x, true),
        _ => Ok(Stack::EMPTY),
    }
}

/// `getSourceHandItem`.
fn hand(x: &mut Ctx, offhand: bool) -> CmdResult<Stack> {
    let eid = x.source_entity()?;
    if let Some(&p) = x.g.by_eid.get(&eid) {
        if let Some(pl) = player_ref(x.g, p) {
            return Ok(if offhand { pl.inv.slots[45] } else { pl.inv.hotbar(pl.held) });
        }
    }
    Err(CmdError::tr("commands.drop.no_held_items", vec![x.name_of(eid)]))
}

fn has_block_table(name: &str) -> bool {
    static TABLES: OnceLock<HashSet<String>> = OnceLock::new();
    TABLES
        .get_or_init(|| {
            let m: serde_json::Map<String, serde_json::Value> = serde_json::from_str(rine_data::raw::BLOCK_LOOT).unwrap_or_default();
            m.keys().cloned().collect()
        })
        .contains(name)
}

/// `LootCommand.dropBlockLoot`: the block's loot table run with `tool`.
/// Rine's `loot::block_drops` also applies the harvest check, which vanilla's
/// `getDrops` does not, so drops of tool-gated blocks can differ.
fn mine(g: &Game, pos: [i32; 3], tool: u32) -> CmdResult<(Vec<Stack>, Option<String>)> {
    let state = g.world.get_block(pos[0], pos[1], pos[2]);
    let name = rine_data::get().blocks.block_of(state).name.clone();
    let short = name.strip_prefix("minecraft:").unwrap_or(&name).to_string();
    if !has_block_table(&name) {
        return Err(CmdError::tr("commands.drop.no_loot_table.block", vec![Text::key(format!("block.minecraft.{short}"))]));
    }
    let drops = loot::block_drops(state, tool);
    Ok((drops, Some(format!("minecraft:blocks/{short}"))))
}

/// `LootCommand` output consumers (replace, insert, give, spawn).
fn loot_output(x: &mut Ctx, w: &[&str], drops: &[Stack]) -> CmdResult<Tracker<Stack>> {
    let mut tr: Tracker<Stack> = Tracker::new();
    match (w[1], w[2]) {
        ("replace", "entity") => replace_entity(x, drops, &mut tr)?,
        ("replace", _) => replace_block(x, drops)?,
        ("insert", _) => insert_block(x, drops)?,
        ("give", _) => give_players(x, drops, &mut tr)?,
        _ => spawn_drops(x, drops, &mut tr),
    }
    Ok(tr)
}

fn slot_int(x: &Ctx, n: &str) -> i32 {
    match x.arg(n) {
        Arg::Slot(s) => *s,
        Arg::Int(i) => *i,
        _ => 0,
    }
}

/// `LootCommand.entityReplace` / `setSlots`.
fn replace_entity(x: &mut Ctx, drops: &[Stack], tr: &mut Tracker<Stack>) -> CmdResult<()> {
    let eids = x.entities("entities")?;
    let start = slot_int(x, "slot");
    let count = if x.has("count") { x.int("count") } else { drops.len() as i32 };
    for eid in eids {
        for i in 0..count {
            let item = drops.get(i as usize).copied().unwrap_or(Stack::EMPTY);
            if let Some(s) = entity_slot(x.g, eid, start + i) {
                if set_slot(x.g, s, item) {
                    tr.track(item, 1);
                }
            }
        }
        sync_entity(x.g, eid);
    }
    Ok(())
}

/// `LootCommand.blockReplace`: the container checks run, then writing is unmodelled.
fn replace_block(x: &mut Ctx, drops: &[Stack]) -> CmdResult<()> {
    let pos = x.block_pos("targetPos")?;
    let start = slot_int(x, "slot");
    let count = if x.has("count") { x.int("count") } else { drops.len() as i32 };
    let size = container_size_at(x.g, pos, NOT_CONTAINER_TARGET)?;
    if start < 0 || start as usize >= size {
        return Err(CmdError::tr("commands.item.target.no_such_slot", vec![lit(start)]));
    }
    if count > 0 {
        return Err(unmodelled("writing block container slots from loot"));
    }
    Ok(())
}

/// `LootCommand.blockDistribute`: the container check runs, then writing is unmodelled.
fn insert_block(x: &mut Ctx, drops: &[Stack]) -> CmdResult<()> {
    let pos = x.block_pos("targetPos")?;
    container_size_at(x.g, pos, NOT_CONTAINER_TARGET)?;
    if !drops.is_empty() {
        return Err(unmodelled("inserting loot into block containers"));
    }
    Ok(())
}

/// `LootCommand.playerGive`: `Inventory.add` counts a drop when any part fits.
fn give_players(x: &mut Ctx, drops: &[Stack], tr: &mut Tracker<Stack>) -> CmdResult<()> {
    let players = x.players("players")?;
    let mut changed: Vec<usize> = Vec::new();
    for d in drops {
        for &p in &players {
            let Some(pl) = x.g.player(p) else { continue };
            let left = pl.inv.add(*d);
            if left < d.count {
                tr.track(*d, 1);
                if !changed.contains(&p) {
                    changed.push(p);
                }
            }
        }
    }
    for p in changed {
        sync_player(x.g, p);
    }
    Ok(())
}

/// `LootCommand.dropInWorld`: item entities with the default pickup delay.
fn spawn_drops(x: &mut Ctx, drops: &[Stack], tr: &mut Tracker<Stack>) {
    let pos = x.vec3("targetPos");
    let mut rng = rng_seed(x.g);
    for d in drops {
        let vel = [rnd(&mut rng) * 0.2 - 0.1, 0.2, rnd(&mut rng) * 0.2 - 0.1];
        entity::spawn_item(x.g, pos, vel, *d, 10);
        tr.track(*d, 1);
    }
}

#[cfg(test)]
mod tests {
    use crate::game::cmd::Commands;

    /// Grammar: single slot names and inline item modifiers parse at level 4.
    /// Multi-slot names (`hotbar.*`) are left out: rine-command rejects them.
    #[test]
    fn item_forms_parse() {
        rine_data::init();
        let mut c = Commands::new();
        let lines = [
            "item replace entity @s hotbar.0 with stone 5",
            "item replace block 0 64 0 contents with stone",
            "item modify entity @s hotbar.0 {function:\"minecraft:set_count\",count:3}",
            "item modify entity @s hotbar.0 {function:\"set_count\",count:3}",
            "item replace entity @s hotbar.1 from entity @p hotbar.2 {function:\"minecraft:discard\"}",
            "loot give @s loot minecraft:blocks/stone",
        ];
        for l in lines {
            assert!(c.parse(l, 4).is_ok(), "does not parse: {l}");
        }
    }
}
