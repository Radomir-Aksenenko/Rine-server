//! /data (`DataCommands`, `EntityDataAccessor`, `BlockDataAccessor`,
//! `StorageDataAccessor`) and `nbt` text components (`NbtContents`).
//!
//! Executors are attached to every executable `data` path of the tree and
//! tell the branch apart by the literal / argument names of the parsed stage
//! (shared tree nodes make `x.path()` ambiguous).

use crate::game::cmd::source::Source;
use crate::game::cmd::{feedback, lit, select, Commands, Ctx};
use crate::game::entity::Kind;
use crate::game::player::Stack;
use crate::game::Game;
use rine_command::args::misc::IdOrInline;
use rine_command::args::nbt_path::{self, NbtPath};
use rine_command::args::{coords, selector, snbt, Arg};
use rine_command::{CmdError, CmdResult, Reader};
use rine_protocol::buf::{BlockPos, Writer};
use rine_protocol::nbt::Tag;
use rine_protocol::text::{hover_text, Text};
use rine_world::ChunkPos;

/// `NbtPath.MAX_DEPTH`.
const MAX_DEPTH: usize = 512;

/// `DataAccessor`: an entity, a block entity or a command storage.
#[derive(Clone, Debug)]
pub enum DataTarget {
    Entity(i32),
    Block([i32; 3]),
    Storage(String),
}

pub fn register(c: &mut Commands) {
    let paths: Vec<&'static str> = c.tree.nodes.iter().filter(|n| n.executable && n.path.split(' ').next() == Some("data")).map(|n| n.path).collect();
    for p in paths {
        c.exec(p, run);
    }
}

/// `DataProvider.access(context)` for `kind` = "entity" | "block" |
/// "storage" and the argument `arg` (entity selector, block pos or id).
pub fn target(x: &mut Ctx, kind: &str, arg: &str) -> CmdResult<DataTarget> {
    match kind {
        "entity" => Ok(DataTarget::Entity(x.entity(arg)?)),
        "block" => {
            let p = x.block_pos(arg)?;
            if block_entity_nbt(x.g, p).is_none() {
                return Err(CmdError::key("commands.data.block.invalid"));
            }
            Ok(DataTarget::Block(p))
        }
        _ => Ok(DataTarget::Storage(x.id(arg).to_string())),
    }
}

/// `DataAccessor.getData()`.
pub fn get_data(g: &Game, t: &DataTarget) -> CmdResult<Tag> {
    Ok(match t {
        DataTarget::Entity(e) => entity_nbt(g, *e),
        DataTarget::Block(p) => block_entity_nbt(g, *p).unwrap_or_else(Tag::compound),
        DataTarget::Storage(id) => g.cs.storage.get(id).cloned().unwrap_or_else(Tag::compound),
    })
}

/// `DataAccessor.setData(tag)`.
pub fn set_data(g: &mut Game, t: &DataTarget, tag: Tag) -> CmdResult<()> {
    match t {
        DataTarget::Entity(e) => set_entity(g, *e, tag),
        DataTarget::Block(p) => {
            set_block(g, *p, tag);
            Ok(())
        }
        DataTarget::Storage(id) => {
            g.cs.storage.insert(id.clone(), tag);
            Ok(())
        }
    }
}

/// `BlockEntity.saveWithFullMetadata` of the block entity at `pos`
/// (`None` when there is no block entity).
pub fn block_entity_nbt(g: &Game, pos: [i32; 3]) -> Option<Tag> {
    let slot = g.world.chunks.get(&ChunkPos::new(pos[0] >> 4, pos[2] >> 4))?;
    let be = slot.chunk.block_entities.iter().find(|b| b.x as i32 == pos[0] & 15 && b.z as i32 == pos[2] & 15 && b.y as i32 == pos[1])?;
    let mut t = be.nbt.clone().unwrap_or_else(Tag::compound);
    let ty = rine_data::get().registries.find("minecraft:block_entity_type").and_then(|r| r.entries.get(be.type_id as usize)).map(|e| e.to_string()).unwrap_or_default();
    if let Tag::Compound(v) = &mut t {
        // `saveWithFullMetadata`: the block entity's components (empty when none)
        let comps = v.iter().find(|(k, _)| k == "components").map(|(_, c)| c.clone()).unwrap_or_else(Tag::compound);
        v.retain(|(k, _)| !matches!(k.as_str(), "x" | "y" | "z" | "id" | "components"));
        v.insert(0, ("components".into(), comps));
        if let Some(items) = crate::game::containers::block_items(rine_protocol::buf::BlockPos::new(pos[0], pos[1], pos[2])) {
            v.retain(|(k, _)| k != "Items");
            v.push(("Items".into(), items_tag(&items)));
        }
        // fields a fresh block entity of this type saves
        let short = ty.strip_prefix("minecraft:").unwrap_or(&ty);
        let defaults: &[(&str, fn() -> Tag)] = match short {
            "chest" | "trapped_chest" | "barrel" | "dispenser" | "dropper" | "crafter" => &[("Items", || Tag::List(Vec::new()))],
            "hopper" => &[("TransferCooldown", || Tag::Int(-1)), ("Items", || Tag::List(Vec::new()))],
            "furnace" | "smoker" | "blast_furnace" => &[
                ("lit_total_time", || Tag::Int(0)),
                ("cooking_time_spent", || Tag::Int(0)),
                ("speed_multiplier", || Tag::Float(1.0)),
                ("cooking_total_time", || Tag::Int(0)),
                ("Items", || Tag::List(Vec::new())),
                ("lit_time_remaining", || Tag::Int(0)),
                ("RecipesUsed", Tag::compound),
            ],
            "brewing_stand" => &[
                ("Fuel", || Tag::Int(0)),
                ("speed_multiplier", || Tag::Float(1.0)),
                ("Items", || Tag::List(Vec::new())),
                ("BrewTime", || Tag::Short(0)),
                ("total_brew_time", || Tag::Int(0)),
                ("total_fuel", || Tag::Int(0)),
            ],
            _ => &[],
        };
        for (k, f) in defaults {
            if !v.iter().any(|(n, _)| n == k) {
                v.push((k.to_string(), f()));
            }
        }
        // `saveMetadata` puts the id and position after the data
        v.push(("id".into(), Tag::String(ty)));
        v.push(("x".into(), Tag::Int(pos[0])));
        v.push(("y".into(), Tag::Int(pos[1])));
        v.push(("z".into(), Tag::Int(pos[2])));
    }
    Some(t)
}

use crate::game::blockent::{items_from_tag, items_tag};

/// `EntityDataAccessor.getData` = `NbtPredicate.getEntityTagToCompare`:
/// `saveWithoutId`, plus `SelectedItem` for players.
pub fn entity_nbt(g: &Game, eid: i32) -> Tag {
    let mut out: Vec<(String, Tag)> = Vec::new();
    let meta = g.ent_meta.get(&eid);
    if let Some(p) = g.by_eid.get(&eid).and_then(|&s| g.players.get(s)).and_then(|p| p.as_ref()) {
        let gm = p.gamemode;
        put(&mut out, "Pos", dlist(&p.pos));
        put(&mut out, "Motion", dlist(&p.vel));
        put(&mut out, "Rotation", flist(&[p.yaw, p.pitch]));
        put(&mut out, "FallDistance", Tag::Float(p.fall_distance as f32));
        put(&mut out, "OnGround", byte(p.on_ground));
        put(&mut out, "Health", Tag::Float(p.health));
        put(&mut out, "foodLevel", Tag::Int(p.food));
        put(&mut out, "XpLevel", Tag::Int(super::experience::level_of(g, eid)));
        put(&mut out, "playerGameType", Tag::Int(i32::from(gm)));
        put(&mut out, "SelectedItemSlot", Tag::Int(i32::from(p.held)));
        put(&mut out, "Inventory", inventory_tag(&p.inv.slots));
        let selected = p.inv.hotbar(p.held);
        if !selected.is_empty() {
            put(&mut out, "SelectedItem", stack_tag(selected));
        }
        put(
            &mut out,
            "abilities",
            Tag::Compound(vec![
                ("flying".into(), byte(p.flying || gm == 3)),
                ("invulnerable".into(), byte(gm == 1 || gm == 3)),
                ("mayfly".into(), byte(gm == 1 || gm == 3)),
                ("instabuild".into(), byte(gm == 1)),
                ("mayBuild".into(), byte(gm != 2)),
                ("flySpeed".into(), Tag::Float(0.05)),
                ("walkSpeed".into(), Tag::Float(0.1)),
            ]),
        );
        put(&mut out, "Dimension", Tag::String("minecraft:overworld".into()));
        put(&mut out, "UUID", uuid_tag(p.uuid));
        common_meta(&mut out, meta);
    } else if let Some(e) = g.entities.get(eid) {
        put(&mut out, "Pos", dlist(&e.pos));
        put(&mut out, "Motion", dlist(&e.vel));
        put(&mut out, "Rotation", flist(&meta.map(|m| m.rot).unwrap_or([0.0, 0.0])));
        put(&mut out, "OnGround", byte(e.on_ground));
        put(&mut out, "UUID", uuid_tag(e.uuid));
        common_meta(&mut out, meta);
        match &e.kind {
            Kind::Item { stack, pickup_delay } => {
                put(&mut out, "Age", Tag::Short(e.age as i16));
                put(&mut out, "PickupDelay", Tag::Short(*pickup_delay as i16));
                put(&mut out, "Health", Tag::Short(5));
                put(&mut out, "Item", stack_tag(*stack));
            }
            Kind::Tnt { fuse } => {
                put(&mut out, "fuse", Tag::Short(*fuse as i16));
            }
            Kind::Falling { state } => {
                put(&mut out, "Time", Tag::Int(e.age));
                put(&mut out, "DropItem", byte(true));
                put(&mut out, "BlockState", block_state_tag(*state));
            }
        }
    }
    if let Some(Tag::Compound(extra)) = meta.and_then(|m| m.extra.as_ref()) {
        for (k, v) in extra {
            if !out.iter().any(|(n, _)| n == k) {
                out.push((k.clone(), v.clone()));
            }
        }
    }
    Tag::Compound(out)
}

/// Fields every command-visible entity has (`Entity.addAdditionalSaveData`).
fn common_meta(out: &mut Vec<(String, Tag)>, meta: Option<&crate::game::cmd::EntMeta>) {
    let Some(m) = meta else { return };
    if !m.tags.is_empty() {
        put(out, "Tags", Tag::List(m.tags.iter().map(|t| Tag::String(t.clone())).collect()));
    }
    if let Some(name) = &m.custom_name {
        put(out, "CustomName", name.to_nbt());
        put(out, "CustomNameVisible", byte(m.custom_name_visible));
    }
    put(out, "NoGravity", byte(m.no_gravity));
    put(out, "Silent", byte(m.silent));
    put(out, "Glowing", byte(m.glowing));
    put(out, "Invulnerable", byte(m.invulnerable));
}

/// `NbtUtils.compareNbt`.
pub fn compare_nbt(expected: &Tag, actual: &Tag, partial_lists: bool) -> bool {
    if expected.id() != actual.id() {
        return false;
    }
    match (expected, actual) {
        (Tag::Compound(e), Tag::Compound(a)) => {
            if a.len() < e.len() {
                return false;
            }
            e.iter().all(|(k, v)| match actual.get(k) {
                Some(av) => compare_nbt(v, av, partial_lists),
                None => false,
            })
        }
        (Tag::List(e), Tag::List(a)) if partial_lists => {
            if e.is_empty() {
                return a.is_empty();
            }
            if a.len() < e.len() {
                return false;
            }
            e.iter().all(|t| a.iter().any(|v| compare_nbt(t, v, partial_lists)))
        }
        _ => expected == actual,
    }
}

/// `NbtContents.resolve`: the `nbt` text component. `fields` are the raw
/// component fields (`nbt`, `interpret`, `plain`, `separator` and one of
/// `block` / `entity` / `storage`).
pub fn resolve_nbt_component(g: &Game, src: &Source, fields: &[(String, Tag)], entity: Option<i32>, depth: u32) -> CmdResult<Text> {
    let get = |k: &str| fields.iter().find(|(n, _)| n == k).map(|(_, v)| v);
    let Some(Tag::String(path_src)) = get("nbt") else { return Ok(Text::empty()) };
    let Ok(path) = nbt_path::parse(&mut Reader::new(path_src)) else { return Ok(Text::empty()) };
    let interpret = matches!(get("interpret"), Some(Tag::Byte(v)) if *v != 0);
    let plain = matches!(get("plain"), Some(Tag::Byte(v)) if *v != 0);

    // DataSource.getData
    let sources: Vec<Tag> = if let Some(Tag::String(b)) = get("block") {
        let Ok(c) = coords::parse_block_pos(&mut Reader::new(b)) else { return Ok(Text::empty()) };
        let pos = c.block_pos(&select::pose(g, src));
        if g.world.is_loaded(pos[0], pos[2]) {
            block_entity_nbt(g, pos).into_iter().collect()
        } else {
            Vec::new()
        }
    } else if let Some(Tag::String(e)) = get("entity") {
        let Ok(sel) = selector::parse_selector(&mut Reader::new(e), true) else { return Ok(Text::empty()) };
        select::find_entities(g, src, &sel).unwrap_or_default().into_iter().map(|e| entity_nbt(g, e)).collect()
    } else if let Some(Tag::String(s)) = get("storage") {
        let id = if s.contains(':') { s.clone() } else { format!("minecraft:{s}") };
        vec![g.cs.storage.get(&id).cloned().unwrap_or_else(Tag::compound)]
    } else {
        return Ok(Text::empty());
    };

    let elements: Vec<Tag> = sources.iter().flat_map(|t| path.get(t).unwrap_or_default()).collect();
    let sep = match get("separator") {
        Some(s) => select::resolve_text(g, src, &select::text_from_tag(s), entity, depth)?,
        None => Text::plain(", "),
    };
    // `NbtContents.resolve`: elements.reduce((l, r) -> l.append(sep).append(r)),
    // so the first element is the root and inherits nothing from a wrapper.
    let mut out: Option<Text> = None;
    for el in elements.iter() {
        let part = if interpret {
            select::resolve_text(g, src, &select::text_from_tag(el), entity, depth)?
        } else if plain {
            Text::plain(to_pretty_text(el).plain_string())
        } else {
            to_pretty_text(el)
        };
        match &mut out {
            None => out = Some(part),
            Some(root) => {
                root.extra.push(sep.clone());
                root.extra.push(part);
            }
        }
    }
    Ok(out.unwrap_or_else(Text::empty))
}

/// `NbtPathArgument.NbtPath.countMatching`.
pub fn path_count(path: &NbtPath, root: &Tag) -> i32 {
    path.count_matching(root)
}

/// `NbtPathArgument.NbtPath.set` succeeded (changed at least one tag).
pub fn path_set(path: &NbtPath, root: &mut Tag, value: &Tag) -> bool {
    path.set(root, value).is_ok_and(|n| n > 0)
}

/// `NbtPathArgument.NbtPath.get` (error `arguments.nbtpath.nothing_found`).
pub fn path_get(path: &NbtPath, root: &Tag) -> CmdResult<Vec<Tag>> {
    path.get(root)
}

/// `Tag.toString()` (compact SNBT).
pub fn to_snbt(t: &Tag) -> String {
    snbt::to_snbt(t)
}

// ---------------------------------------------------------------------------
// Executors
// ---------------------------------------------------------------------------

/// Names of the nodes of the current stage (literals and arguments).
fn names(x: &Ctx) -> Vec<&'static str> {
    let tree = x.g.cmds.tree.clone();
    x.c.nodes.iter().map(|(id, _, _)| tree.node(*id).name).collect()
}

fn run(x: &mut Ctx) -> CmdResult<i32> {
    let s = names(x);
    match s[1] {
        "get" => get(x, &s),
        "merge" => merge(x, &s),
        "remove" => remove(x, &s),
        _ => modify(x, &s),
    }
}

/// `data get <kind> target [path [scale]]`.
fn get(x: &mut Ctx, s: &[&str]) -> CmdResult<i32> {
    let t = target(x, s[2], "target")?;
    if s.len() == 4 {
        let tag = get_data(x.g, &t)?;
        let msg = query_msg(x.g, &t, &tag);
        x.success(msg, false);
        return Ok(1);
    }
    let path = path_arg(x, "path")?;
    let tag = get_data(x.g, &t)?;
    let single = single_tag(&path, &tag)?;
    if s.len() == 5 {
        let n = match &single {
            Tag::Byte(_) | Tag::Short(_) | Tag::Int(_) | Tag::Long(_) | Tag::Float(_) | Tag::Double(_) => numeric(&single).map_or(0, |v| v.floor() as i32),
            Tag::List(l) => l.len() as i32,
            Tag::ByteArray(a) => a.len() as i32,
            Tag::IntArray(a) => a.len() as i32,
            Tag::LongArray(a) => a.len() as i32,
            Tag::Compound(c) => c.len() as i32,
            Tag::String(v) => v.encode_utf16().count() as i32,
        };
        let msg = query_msg(x.g, &t, &single);
        x.success(msg, false);
        Ok(n)
    } else {
        let scale = x.double("scale");
        let Some(v) = numeric(&single) else {
            return Err(CmdError::tr("commands.data.get.invalid", vec![lit(&path.original)]));
        };
        let result = (v * scale).floor() as i32;
        let msg = get_msg(x.g, &t, &path, scale, result);
        x.success(msg, false);
        Ok(result)
    }
}

/// `data merge <kind> target nbt`.
fn merge(x: &mut Ctx, s: &[&str]) -> CmdResult<i32> {
    let t = target(x, s[2], "target")?;
    let nbt = x.nbt("nbt").clone();
    let old = get_data(x.g, &t)?;
    if too_deep(&nbt, 0) {
        return Err(CmdError::key("arguments.nbtpath.too_deep"));
    }
    let mut result = old.clone();
    merge_compound(&mut result, &nbt);
    if result == old {
        return Err(CmdError::key("commands.data.merge.failed"));
    }
    set_data(x.g, &t, result)?;
    let msg = modified_msg(x.g, &t);
    x.success(msg, true);
    Ok(1)
}

/// `data remove <kind> target path`.
fn remove(x: &mut Ctx, s: &[&str]) -> CmdResult<i32> {
    let t = target(x, s[2], "target")?;
    let path = path_arg(x, "path")?;
    let mut root = get_data(x.g, &t)?;
    let count = path.remove(&mut root);
    if count == 0 {
        return Err(CmdError::key("commands.data.merge.failed"));
    }
    set_data(x.g, &t, root)?;
    let msg = modified_msg(x.g, &t);
    x.success(msg, true);
    Ok(count)
}

#[derive(Clone, Copy)]
enum Op {
    Insert(i32),
    Prepend,
    Append,
    Set,
    Merge,
}

/// `data modify <kind> target targetPath <op> <source>`.
fn modify(x: &mut Ctx, s: &[&str]) -> CmdResult<i32> {
    let kind = s[2];
    // stage: 0 data, 1 modify, 2 kind, 3 target, 4 targetPath, 5 op
    let (op, mut i) = match s[5] {
        "prepend" => (Op::Prepend, 6),
        "append" => (Op::Append, 6),
        "set" => (Op::Set, 6),
        "merge" => (Op::Merge, 6),
        _ => (Op::Insert(x.int("index")), 7),
    };
    // Sources are resolved before the target, as in `manipulateData`.
    let source: Vec<Tag> = match s[i] {
        "value" => vec![x.nbt("value").clone()],
        "from" => {
            let src = target(x, s[i + 1], "source")?;
            let root = get_data(x.g, &src)?;
            if s.len() > i + 3 {
                path_arg(x, "sourcePath")?.get(&root)?
            } else {
                vec![root]
            }
        }
        "string" => {
            let src = target(x, s[i + 1], "source")?;
            let root = get_data(x.g, &src)?;
            let items = if s.len() > i + 3 { path_arg(x, "sourcePath")?.get(&root)? } else { vec![root] };
            let start = (s.len() > i + 4).then(|| x.int("start"));
            let end = (s.len() > i + 5).then(|| x.int("end"));
            let mut out = Vec::with_capacity(items.len());
            for t in &items {
                let text = as_text(t)?;
                let v = match (start, end) {
                    (None, _) => text,
                    (Some(a), None) => substring(&text, a, None)?,
                    (Some(a), Some(b)) => substring(&text, a, Some(b))?,
                };
                out.push(Tag::String(v));
            }
            out
        }
        _ => {
            // compute <context> [target] <float|integer> provider
            let ctx = s[i + 1];
            match ctx {
                "entity" => {
                    x.entity("computeTarget")?;
                }
                "block" => {
                    x.block_pos("computePos")?;
                }
                _ => {}
            }
            i = s.len() - 2;
            let value = match x.arg("provider") {
                Arg::IdOrInline(IdOrInline::Inline(t)) => constant(t),
                _ => None,
            };
            let Some(v) = value else {
                return Err(unimplemented(x));
            };
            if s[i] == "float" {
                vec![Tag::Float(v as f32)]
            } else {
                vec![Tag::Int(v as i32)]
            }
        }
    };

    let t = target(x, kind, "target")?;
    let path = path_arg(x, "targetPath")?;
    let mut root = get_data(x.g, &t)?;
    let n = apply(op, &path, &mut root, &source)?;
    if n == 0 {
        return Err(CmdError::key("commands.data.merge.failed"));
    }
    set_data(x.g, &t, root)?;
    let msg = modified_msg(x.g, &t);
    x.success(msg, true);
    Ok(n)
}

/// The manipulation of one `modify` stage (`DataCommands.decorateModification`).
fn apply(op: Op, path: &NbtPath, root: &mut Tag, source: &[Tag]) -> CmdResult<i32> {
    match op {
        Op::Insert(i) => path.insert(i, root, source),
        Op::Prepend => path.insert(0, root, source),
        Op::Append => path.insert(-1, root, source),
        Op::Set => match source.last() {
            Some(last) => path.set(root, last),
            None => Ok(0),
        },
        Op::Merge => {
            let mut combined = Tag::compound();
            for t in source {
                if too_deep(t, 0) {
                    return Err(CmdError::key("arguments.nbtpath.too_deep"));
                }
                if !matches!(t, Tag::Compound(_)) {
                    return Err(expected_object(t));
                }
                merge_compound(&mut combined, t);
            }
            let mut err = None;
            let n = path.modify_each_with(root, true, &Tag::compound(), &mut |target| {
                if !matches!(target, Tag::Compound(_)) {
                    err = Some(expected_object(target));
                    return 0;
                }
                let before = target.clone();
                merge_compound(target, &combined);
                i32::from(*target != before)
            })?;
            match err {
                Some(e) => Err(e),
                None => Ok(n),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers for the executors
// ---------------------------------------------------------------------------

fn path_arg(x: &Ctx, name: &str) -> CmdResult<NbtPath> {
    match x.arg(name) {
        Arg::NbtPath(p) => Ok((**p).clone()),
        _ => Err(CmdError::key("arguments.nbtpath.nothing_found")),
    }
}

/// `DataCommands.getSingleTag`.
fn single_tag(path: &NbtPath, root: &Tag) -> CmdResult<Tag> {
    let mut tags = path.get(root)?;
    if tags.len() > 1 {
        return Err(CmdError::key("commands.data.get.multiple"));
    }
    tags.pop().ok_or_else(|| CmdError::key("arguments.nbtpath.nothing_found"))
}

fn numeric(t: &Tag) -> Option<f64> {
    match t {
        Tag::Byte(v) => Some(f64::from(*v)),
        Tag::Short(v) => Some(f64::from(*v)),
        Tag::Int(v) => Some(f64::from(*v)),
        Tag::Long(v) => Some(*v as f64),
        Tag::Float(v) => Some(f64::from(*v)),
        Tag::Double(v) => Some(*v),
        _ => None,
    }
}

/// `StringTag` value or the SNBT of a primitive tag (`getAsText`).
fn as_text(t: &Tag) -> CmdResult<String> {
    match t {
        Tag::String(s) => Ok(s.clone()),
        Tag::Byte(_) | Tag::Short(_) | Tag::Int(_) | Tag::Long(_) | Tag::Float(_) | Tag::Double(_) => Ok(snbt::to_snbt(t)),
        _ => Err(expected_value(t)),
    }
}

/// `DataCommands.substring`: negative indices count from the end, in UTF-16 units.
fn substring(input: &str, start: i32, end: Option<i32>) -> CmdResult<String> {
    let units: Vec<u16> = input.encode_utf16().collect();
    let len = units.len() as i32;
    let a = if start >= 0 { start } else { len + start };
    let b = match end {
        Some(e) if e >= 0 => e,
        Some(e) => len + e,
        None => len,
    };
    if a >= 0 && b <= len && a <= b {
        Ok(String::from_utf16_lossy(&units[a as usize..b as usize]))
    } else {
        Err(CmdError::tr("commands.data.modify.invalid_substring", vec![lit(a), lit(b)]))
    }
}

/// Constant value of an inline context provider (loot providers that need a
/// loot table or randomness are not evaluated).
fn constant(t: &Tag) -> Option<f64> {
    if let Some(v) = numeric(t) {
        return Some(v);
    }
    match (t.get("type"), t.get("value")) {
        (Some(Tag::String(ty)), Some(v)) if ty == "minecraft:constant" => numeric(v),
        _ => None,
    }
}

fn unimplemented(x: &Ctx) -> CmdError {
    CmdError::new(Text::key("command.failed").on_hover(hover_text(&Text::plain(format!("Rine: /{} is not implemented yet", x.path())))))
}

fn expected_object(t: &Tag) -> CmdError {
    CmdError::tr("commands.data.modify.expected_object", vec![lit(snbt::to_snbt(t))])
}

fn expected_value(t: &Tag) -> CmdError {
    CmdError::tr("commands.data.modify.expected_value", vec![lit(snbt::to_snbt(t))])
}

/// `NbtPath.isTooDeep`.
fn too_deep(t: &Tag, depth: usize) -> bool {
    if depth >= MAX_DEPTH {
        return true;
    }
    match t {
        Tag::Compound(c) => c.iter().any(|(_, v)| too_deep(v, depth + 1)),
        Tag::List(l) => l.iter().any(|v| too_deep(v, depth + 1)),
        _ => false,
    }
}

/// `CompoundTag.merge`: nested compounds merge, other values replace.
fn merge_compound(dst: &mut Tag, src: &Tag) {
    let (Tag::Compound(d), Tag::Compound(s)) = (dst, src) else { return };
    for (k, v) in s {
        match d.iter_mut().find(|(n, _)| n == k) {
            Some((_, dv)) => {
                if matches!(dv, Tag::Compound(_)) && matches!(v, Tag::Compound(_)) {
                    merge_compound(dv, v);
                } else {
                    *dv = v.clone();
                }
            }
            None => d.push((k.clone(), v.clone())),
        }
    }
}

fn modified_msg(g: &Game, t: &DataTarget) -> Text {
    match t {
        DataTarget::Entity(e) => Text::tr("commands.data.entity.modified", vec![feedback::display_name(g, *e)]),
        DataTarget::Block(p) => Text::tr("commands.data.block.modified", vec![lit(p[0]), lit(p[1]), lit(p[2])]),
        DataTarget::Storage(id) => Text::tr("commands.data.storage.modified", vec![lit(id)]),
    }
}

fn query_msg(g: &Game, t: &DataTarget, tag: &Tag) -> Text {
    let pretty = to_pretty_text(tag);
    match t {
        DataTarget::Entity(e) => Text::tr("commands.data.entity.query", vec![feedback::display_name(g, *e), pretty]),
        DataTarget::Block(p) => Text::tr("commands.data.block.query", vec![lit(p[0]), lit(p[1]), lit(p[2]), pretty]),
        DataTarget::Storage(id) => Text::tr("commands.data.storage.query", vec![lit(id), pretty]),
    }
}

fn get_msg(g: &Game, t: &DataTarget, path: &NbtPath, scale: f64, value: i32) -> Text {
    let sc = format!("{scale:.2}");
    match t {
        DataTarget::Entity(e) => Text::tr("commands.data.entity.get", vec![lit(&path.original), feedback::display_name(g, *e), lit(sc), lit(value)]),
        DataTarget::Block(p) => Text::tr("commands.data.block.get", vec![lit(&path.original), lit(p[0]), lit(p[1]), lit(p[2]), lit(sc), lit(value)]),
        DataTarget::Storage(id) => Text::tr("commands.data.storage.get", vec![lit(&path.original), lit(id), lit(sc), lit(value)]),
    }
}

/// `NbtUtils.toPrettyComponent` (rich styling).
fn to_pretty_text(t: &Tag) -> Text {
    snbt::to_pretty_text(t)
}

fn put(out: &mut Vec<(String, Tag)>, k: &str, v: Tag) {
    out.push((k.to_string(), v));
}

fn byte(v: bool) -> Tag {
    Tag::Byte(i8::from(v))
}

fn dlist(v: &[f64]) -> Tag {
    Tag::List(v.iter().map(|d| Tag::Double(*d)).collect())
}

fn flist(v: &[f32]) -> Tag {
    Tag::List(v.iter().map(|f| Tag::Float(*f)).collect())
}

/// `UUIDUtil.uuidToIntArray`.
fn uuid_tag(u: uuid::Uuid) -> Tag {
    let v = u.as_u128();
    Tag::IntArray(vec![(v >> 96) as i32, (v >> 64) as i32, (v >> 32) as i32, v as i32])
}

/// `ItemStack.save`: `{id, count}`.
fn stack_tag(s: Stack) -> Tag {
    let name = crate::game::items::info(s.item).map(|i| i.name.clone()).unwrap_or_else(|| "minecraft:air".into());
    Tag::Compound(vec![("id".into(), Tag::String(name)), ("count".into(), Tag::Int(s.count))])
}

/// `Inventory.save`: main slots by vanilla index, armor at 100-103, offhand at 150.
fn inventory_tag(slots: &[Stack; 46]) -> Tag {
    let mut list = Vec::new();
    let mut push = |vanilla: i32, s: Stack| {
        if !s.is_empty() {
            let mut c = match stack_tag(s) {
                Tag::Compound(c) => c,
                _ => Vec::new(),
            };
            c.insert(0, ("Slot".into(), Tag::Byte(vanilla as i8)));
            list.push(Tag::Compound(c));
        }
    };
    for v in 0..36 {
        let i = if v < 9 { 36 + v } else { v };
        push(v as i32, slots[i]);
    }
    for i in [8usize, 7, 6, 5] {
        push(100 + (8 - i) as i32, slots[i]);
    }
    push(150, slots[45]);
    Tag::List(list)
}

/// `BlockState` of a falling block: `{Name}`.
fn block_state_tag(state: u16) -> Tag {
    let name = rine_data::get().blocks.block_of(state).name.as_str();
    let full = if name.contains(':') { name.to_string() } else { format!("minecraft:{name}") };
    Tag::Compound(vec![("Name".into(), Tag::String(full))])
}

/// `EntityDataAccessor.setData`: players are refused, other entities take the
/// fields Rine models and keep the rest in `EntMeta.extra`.
fn set_entity(g: &mut Game, eid: i32, tag: Tag) -> CmdResult<()> {
    if g.by_eid.contains_key(&eid) {
        return Err(CmdError::key("commands.data.entity.invalid"));
    }
    let Tag::Compound(fields) = tag else { return Ok(()) };
    let Some(i) = g.entities.slot_of(eid) else { return Ok(()) };
    let meta = g.ent_meta.entry(eid).or_default();
    let ent = &mut g.entities.list[i];

    // Absent fields take their defaults, as `Entity.load` does.
    ent.vel = [0.0; 3];
    ent.on_ground = false;
    meta.tags.clear();
    meta.custom_name = None;
    meta.custom_name_visible = false;
    meta.no_gravity = false;
    meta.silent = false;
    meta.glowing = false;
    meta.invulnerable = false;
    match &mut ent.kind {
        Kind::Item { pickup_delay, .. } => {
            ent.age = 0;
            *pickup_delay = 0;
        }
        Kind::Tnt { fuse } => *fuse = 80,
        Kind::Falling { .. } => {}
    }

    let mut extra = Vec::new();
    for (k, v) in fields {
        match k.as_str() {
            "Pos" => {
                if let Some(p) = nums(&v, 3) {
                    ent.pos = [p[0], p[1], p[2]];
                }
            }
            "Motion" => {
                if let Some(p) = nums(&v, 3) {
                    ent.vel = [p[0], p[1], p[2]];
                }
            }
            "Rotation" => {
                if let Some(r) = nums(&v, 2) {
                    meta.rot = [r[0] as f32, r[1] as f32];
                }
            }
            "OnGround" => ent.on_ground = as_bool(&v),
            "Tags" => {
                if let Tag::List(l) = &v {
                    meta.tags = l.iter().filter_map(|t| if let Tag::String(s) = t { Some(s.clone()) } else { None }).collect();
                }
            }
            "CustomName" => meta.custom_name = Some(select::text_from_tag(&v)),
            "CustomNameVisible" => meta.custom_name_visible = as_bool(&v),
            "NoGravity" => meta.no_gravity = as_bool(&v),
            "Silent" => meta.silent = as_bool(&v),
            "Glowing" => meta.glowing = as_bool(&v),
            "Invulnerable" => meta.invulnerable = as_bool(&v),
            "Age" => {
                if let Some(a) = as_int(&v) {
                    if matches!(ent.kind, Kind::Item { .. }) {
                        ent.age = a;
                    }
                }
            }
            "Time" => {
                if let (Kind::Falling { .. }, Some(a)) = (&ent.kind, as_int(&v)) {
                    ent.age = a;
                }
            }
            "PickupDelay" => {
                if let (Kind::Item { pickup_delay, .. }, Some(a)) = (&mut ent.kind, as_int(&v)) {
                    *pickup_delay = a;
                }
            }
            "Item" => {
                if let Kind::Item { stack, .. } = &mut ent.kind {
                    if let Some(s) = stack_from_tag(&v) {
                        *stack = s;
                    }
                }
            }
            "fuse" => {
                if let (Kind::Tnt { fuse }, Some(a)) = (&mut ent.kind, as_int(&v)) {
                    *fuse = a;
                }
            }
            "UUID" => {}
            _ => extra.push((k.clone(), v)),
        }
    }
    meta.extra = if extra.is_empty() { None } else { Some(Tag::Compound(extra)) };
    Ok(())
}

/// Stores a block entity's NBT (without x/y/z/id) and sends it to every player
/// (`ClientboundBlockEntityDataPacket`). Keeps `components` when the new tag
/// does not carry them (`get` does not show them).
fn set_block(g: &mut Game, pos: [i32; 3], tag: Tag) {
    let Tag::Compound(mut fields) = tag else { return };
    fields.retain(|(k, _)| !matches!(k.as_str(), "x" | "y" | "z" | "id"));
    let bp = rine_protocol::buf::BlockPos::new(pos[0], pos[1], pos[2]);
    if let Some(old) = crate::game::containers::block_items(bp) {
        let items = fields.iter().find(|(k, _)| k == "Items").map(|(_, v)| items_from_tag(v, old.len())).unwrap_or_else(|| vec![Default::default(); old.len()]);
        crate::game::containers::set_block_items(g, bp, items);
        fields.retain(|(k, _)| k != "Items");
    }
    let Some(slot) = g.world.chunks.get_mut(&ChunkPos::new(pos[0] >> 4, pos[2] >> 4)) else { return };
    let (lx, lz) = ((pos[0] & 15) as u8, (pos[2] & 15) as u8);
    let Some(be) = slot.chunk.block_entities.iter_mut().find(|b| b.x == lx && b.z == lz && b.y as i32 == pos[1]) else { return };
    if !fields.iter().any(|(k, _)| k == "components") {
        if let Some(Tag::Compound(old)) = &be.nbt {
            if let Some(kv) = old.iter().find(|(k, _)| k == "components") {
                fields.push(kv.clone());
            }
        }
    }
    let nbt = Tag::Compound(fields);
    be.nbt = Some(nbt.clone());
    let type_id = be.type_id;
    slot.chunk.dirty = true;

    let pkt = rine_protocol::frame::packet(rine_protocol::ids::play::C_BLOCK_ENTITY_DATA, |b| {
        b.w_i64(BlockPos { x: pos[0], y: pos[1], z: pos[2] }.pack());
        b.w_varint(type_id as i32);
        nbt.write_network(b);
    });
    g.broadcast.push(crate::out::frame(pkt));
}

fn nums(t: &Tag, n: usize) -> Option<Vec<f64>> {
    let Tag::List(l) = t else { return None };
    if l.len() != n {
        return None;
    }
    l.iter().map(numeric).collect()
}

fn as_bool(t: &Tag) -> bool {
    matches!(t, Tag::Byte(v) if *v != 0)
}

fn as_int(t: &Tag) -> Option<i32> {
    match t {
        Tag::Byte(v) => Some(i32::from(*v)),
        Tag::Short(v) => Some(i32::from(*v)),
        Tag::Int(v) => Some(*v),
        Tag::Long(v) => Some(*v as i32),
        _ => None,
    }
}

fn stack_from_tag(t: &Tag) -> Option<Stack> {
    let Some(Tag::String(id)) = t.get("id") else { return None };
    let item = rine_data::get().items.by_name(id)?.id;
    let count = as_int(t.get("count")?)?;
    Some(Stack::new(item, count))
}
