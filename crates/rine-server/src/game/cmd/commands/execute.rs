//! /execute (`ExecuteCommand`): source modifiers, conditions, stores.
//!
//! Stage layout: after a redirect the next stage's node list starts at
//! the child of `execute` (no `execute` literal), so literals are read
//! with `sub(x, i)` (0 = the subcommand literal). The commands packet
//! shares equal nodes: all `store (result|success) (entity|block|storage)
//! ... <type> scale` branches end in one node, and the bossbar `value` /
//! `max` nodes are shared by `result` and `success`; handlers read the
//! literals of the stage to know which branch was taken.

use super::{bossbar, data, scoreboard, stopwatch, summon};
use crate::game::cmd::exec::Callback;
use crate::game::cmd::source::Source;
use crate::game::cmd::{lit, select, Commands, Ctx};
use crate::game::Game;
use rine_command::args::block::BlockPredicate;
use rine_command::args::coords::facing;
use rine_command::args::Arg;
use rine_command::{CmdError, CmdResult};
use rine_data::blocks::flag;
use rine_protocol::nbt::Tag;
use rine_protocol::text::Text;
use std::sync::Arc;

const RELATIONS: [&str; 8] = ["owner", "leasher", "target", "attacker", "vehicle", "controller", "origin", "passengers"];
const OPS: [&str; 5] = ["=", "<", "<=", ">", ">="];
/// `ExecuteCommand.MAX_TEST_AREA`.
const MAX_TEST_AREA: i64 = 32768;

/// Literal `i` positions after `execute` in this stage.
fn sub(x: &Ctx, i: usize) -> &'static str {
    let off = usize::from(x.lit(0) == "execute");
    x.lit(off + i)
}

/// Last literal of the stage.
fn last(x: &Ctx) -> &'static str {
    x.lit(x.c.nodes.len() - 1)
}

pub fn register(c: &mut Commands) {
    c.modifier("execute as targets", true, |x| {
        let ents = x.optional_entities("targets")?;
        Ok(ents.into_iter().map(|e| x.src.with_entity(x.g, e)).collect())
    });
    c.modifier("execute at targets", true, |x| {
        let ents = x.optional_entities("targets")?;
        Ok(ents
            .into_iter()
            .filter_map(|e| {
                let p = select::entity_pos(x.g, e)?;
                Some(x.src.with_dim(crate::game::cmd::source::OVERWORLD).with_pos(p).with_rot(select::entity_rot(x.g, e)))
            })
            .collect())
    });
    c.modifier("execute positioned pos", false, |x| {
        let p = x.vec3("pos");
        Ok(vec![x.src.with_pos(p).with_anchor(false)])
    });
    c.modifier("execute positioned as targets", true, |x| {
        let ents = x.optional_entities("targets")?;
        Ok(ents.into_iter().filter_map(|e| select::entity_pos(x.g, e).map(|p| x.src.with_pos(p))).collect())
    });
    c.modifier("execute positioned over heightmap", false, |x| {
        let p = x.src.pos;
        let (bx, bz) = (p[0].floor() as i32, p[2].floor() as i32);
        if !x.g.world.is_loaded(bx, bz) {
            return Err(CmdError::key("argument.pos.unloaded"));
        }
        let h = height(x.g, x.enum_("heightmap"), bx, bz);
        Ok(vec![x.src.with_pos([p[0], h as f64, p[2]])])
    });
    c.modifier("execute rotated rot", false, |x| {
        let r = x.rotation("rot");
        Ok(vec![x.src.with_rot(r)])
    });
    c.modifier("execute rotated as targets", true, |x| {
        let ents = x.optional_entities("targets")?;
        Ok(ents.into_iter().map(|e| x.src.with_rot(select::entity_rot(x.g, e))).collect())
    });
    c.modifier("execute facing pos", false, |x| {
        let to = x.vec3("pos");
        let from = x.pose().anchor_pos;
        Ok(vec![x.src.with_rot(facing(from, to))])
    });
    c.modifier("execute facing entity targets anchor", true, |x| {
        let eyes = x.enum_("anchor") == 1;
        let ents = x.optional_entities("targets")?;
        let from = x.pose().anchor_pos;
        Ok(ents
            .into_iter()
            .filter_map(|e| {
                let mut p = select::entity_pos(x.g, e)?;
                if eyes {
                    p[1] += select::eye_height(x.g, e);
                }
                Some(x.src.with_rot(facing(from, p)))
            })
            .collect())
    });
    c.modifier("execute align axes", false, |x| {
        let axes = match x.arg("axes") {
            Arg::Swizzle(s) => *s,
            _ => 0,
        };
        let mut p = x.src.pos;
        for (i, v) in p.iter_mut().enumerate() {
            if axes & (1 << i) != 0 {
                *v = v.floor();
            }
        }
        Ok(vec![x.src.with_pos(p)])
    });
    c.modifier("execute anchored anchor", false, |x| {
        let eyes = x.enum_("anchor") == 1;
        Ok(vec![x.src.with_anchor(eyes)])
    });
    c.modifier("execute in dimension", false, |x| {
        let d = x.id("dimension").to_string();
        match dimension(&d) {
            Some(dim) => Ok(vec![x.src.with_dim(dim)]),
            None => Err(CmdError::tr("argument.dimension.invalid", vec![lit(&d)])),
        }
    });
    c.modifier("execute summon entity", false, |x| {
        let ty = x.id("entity").to_string();
        let pos = x.src.pos;
        let src = x.src.clone();
        let e = summon::create_entity(x.g, &src, &ty, pos, &Tag::compound(), true)?;
        Ok(vec![x.src.with_entity(x.g, e)])
    });
    for r in RELATIONS {
        // relations need owners, leashes, vehicles, targets: no Rine entity
        // has any yet, so every relation yields no sources
        c.modifier(&format!("execute on {r}"), true, |_x| Ok(Vec::new()));
    }

    for kw in ["if", "unless"] {
        let mut boolean = vec![
            format!("execute {kw} block pos block"),
            format!("execute {kw} biome pos biome"),
            format!("execute {kw} loaded pos"),
            format!("execute {kw} dimension dimension"),
            format!("execute {kw} predicate predicate"),
            format!("execute {kw} stopwatch id range"),
            format!("execute {kw} score target targetObjective matches range"),
        ];
        for op in OPS {
            boolean.push(format!("execute {kw} score target targetObjective {op} source sourceObjective"));
        }
        for p in &boolean {
            c.modifier(p, true, |x| {
                let ok = bool_test(x)?;
                fork_if(x, ok)
            });
            c.exec(p, |x| {
                if bool_test(x)? == expected(x) {
                    x.success(Text::key("commands.execute.conditional.pass"), false);
                    Ok(1)
                } else {
                    Err(CmdError::key("commands.execute.conditional.fail"))
                }
            });
        }
        let mut counting = vec![format!("execute {kw} entity entities")];
        for k in ["entity", "block", "storage"] {
            counting.push(format!("execute {kw} data {k} source path"));
        }
        for k in ["entity", "block"] {
            counting.push(format!("execute {kw} items {k} source slots item_predicate"));
            counting.push(format!("execute {kw} slots {k} source slots"));
        }
        for p in &counting {
            c.modifier(p, true, |x| {
                let n = count_test(x)?;
                fork_if(x, n > 0)
            });
            c.exec(p, |x| {
                let n = count_test(x)?;
                numeric(x, n)
            });
        }
        for mode in ["all", "masked"] {
            let p = format!("execute {kw} blocks start end destination {mode}");
            c.modifier(&p, true, |x| {
                let r = check_regions(x)?;
                fork_if(x, r.is_some())
            });
            c.exec(&p, |x| match (check_regions(x)?, expected(x)) {
                (Some(n), true) => {
                    x.success(Text::tr("commands.execute.conditional.pass_count", vec![lit(n)]), false);
                    Ok(n)
                }
                (None, true) => Err(CmdError::key("commands.execute.conditional.fail")),
                (Some(n), false) => Err(CmdError::tr("commands.execute.conditional.fail_count", vec![lit(n)])),
                (None, false) => {
                    x.success(Text::key("commands.execute.conditional.pass"), false);
                    Ok(1)
                }
            });
        }
        c.modifier_custom(&format!("execute {kw} function name"), true, super::function::execute_if_function);
    }

    // stores: score and bossbar (bossbar nodes shared by result/success)
    for kind in ["result", "success"] {
        c.modifier(&format!("execute store {kind} score targets objective"), false, |x| {
            let holders = x.score_holders_wild("targets")?;
            let obj = x.objective("objective")?;
            let result = sub(x, 1) == "result";
            let cb: Callback = Some(Arc::new(move |g, ok, v| {
                let value = if result { v } else { i32::from(ok) };
                for h in &holders {
                    scoreboard::set_score(g, h, &obj, value);
                }
            }));
            Ok(vec![chain_store(x.src, cb)])
        });
    }
    for which in ["value", "max"] {
        c.modifier(&format!("execute store result bossbar id {which}"), false, |x| {
            let id = x.id("id").to_string();
            if !bossbar::exists(x.g, &id) {
                return Err(CmdError::tr("commands.bossbar.unknown", vec![lit(&id)]));
            }
            let result = sub(x, 1) == "result";
            let into_value = last(x) == "value";
            let cb: Callback = Some(Arc::new(move |g, ok, v| {
                let value = if result { v } else { i32::from(ok) };
                if into_value {
                    bossbar::set_value(g, &id, value);
                } else {
                    bossbar::set_max(g, &id, value);
                }
            }));
            Ok(vec![chain_store(x.src, cb)])
        });
    }
    c.modifier("execute store result entity target path int scale", false, store_data);
}

fn dimension(id: &str) -> Option<&'static str> {
    match id {
        "minecraft:overworld" => Some("minecraft:overworld"),
        "minecraft:the_nether" => Some("minecraft:the_nether"),
        "minecraft:the_end" => Some("minecraft:the_end"),
        _ => None,
    }
}

fn expected(x: &Ctx) -> bool {
    sub(x, 0) == "if"
}

/// `source.withCallback(cb, CommandResultCallback::chain)`.
fn chain_store(src: &Source, cb: Callback) -> Source {
    src.with_callback(crate::game::cmd::exec::chain_cb(&src.callback, &cb))
}

fn fork_if(x: &Ctx, result: bool) -> CmdResult<Vec<Source>> {
    Ok(if result == expected(x) { vec![x.src.clone()] } else { Vec::new() })
}

/// `createNumericConditionalHandler`.
fn numeric(x: &mut Ctx, count: i32) -> CmdResult<i32> {
    if expected(x) {
        if count > 0 {
            x.success(Text::tr("commands.execute.conditional.pass_count", vec![lit(count)]), false);
            Ok(count)
        } else {
            Err(CmdError::key("commands.execute.conditional.fail"))
        }
    } else if count == 0 {
        x.success(Text::key("commands.execute.conditional.pass"), false);
        Ok(1)
    } else {
        Err(CmdError::tr("commands.execute.conditional.fail_count", vec![lit(count)]))
    }
}

/// The boolean conditions (`addConditional` users).
fn bool_test(x: &mut Ctx) -> CmdResult<bool> {
    match sub(x, 1) {
        "block" => {
            let pos = x.block_pos("pos")?;
            let pred = match x.arg("block") {
                Arg::BlockPredicate(p) => (**p).clone(),
                _ => return Ok(false),
            };
            Ok(block_test(x.g, &pred, pos))
        }
        "biome" => {
            let pos = x.block_pos("pos")?;
            let biome = biome_at(x.g, pos);
            Ok(match x.arg("biome") {
                Arg::ResourceOrTag(r) if r.tag => rine_data::get().registries.tag_contains("minecraft:worldgen/biome", &r.id, &biome),
                Arg::ResourceOrTag(r) => r.id == biome,
                _ => false,
            })
        }
        "loaded" => {
            // `isChunkLoaded`: the position must be inside the world
            let p = x.block_pos_unchecked("pos");
            if !(rine_world::MIN_Y..rine_world::MIN_Y + rine_world::HEIGHT).contains(&p[1]) {
                return Err(CmdError::key("argument.pos.outofworld"));
            }
            Ok(x.g.world.is_loaded(p[0], p[2]))
        }
        "dimension" => {
            let d = x.id("dimension").to_string();
            match dimension(&d) {
                Some(dim) => Ok(dim == x.src.dim),
                None => Err(CmdError::tr("argument.dimension.invalid", vec![lit(&d)])),
            }
        }
        // loot predicates are not implemented: no predicate holds
        "predicate" => Ok(false),
        "stopwatch" => {
            let id = x.id("id").to_string();
            let range = match x.arg("range") {
                Arg::FloatRange(r) => *r,
                _ => return Ok(false),
            };
            match stopwatch::elapsed_seconds(x.g, &id) {
                Some(s) => Ok(range.matches(s)),
                None => Err(CmdError::tr("commands.stopwatch.does_not_exist", vec![lit(&id)])),
            }
        }
        _ => score_test(x),
    }
}

/// `checkScore` / `checkScore(range)`.
fn score_test(x: &mut Ctx) -> CmdResult<bool> {
    let op = sub(x, 4);
    let t = x.score_holder("target")?;
    let to = x.objective("targetObjective")?;
    if op == "matches" {
        let range = match x.arg("range") {
            Arg::IntRange(r) => *r,
            _ => return Ok(false),
        };
        return Ok(scoreboard::score(x.g, &t.name, &to).is_some_and(|v| range.matches(v)));
    }
    let s = x.score_holder("source")?;
    let so = x.objective("sourceObjective")?;
    let (Some(a), Some(b)) = (scoreboard::score(x.g, &t.name, &to), scoreboard::score(x.g, &s.name, &so)) else { return Ok(false) };
    Ok(match op {
        "=" => a == b,
        "<" => a < b,
        "<=" => a <= b,
        ">" => a > b,
        _ => a >= b,
    })
}

/// The counting conditions (`createNumericConditionalHandler` users).
fn count_test(x: &mut Ctx) -> CmdResult<i32> {
    match sub(x, 1) {
        "entity" => Ok(x.optional_entities("entities")?.len() as i32),
        "data" => {
            let kind = sub(x, 2);
            let target = data::target(x, kind, "source")?;
            let tag = data::get_data(x.g, &target)?;
            let path = match x.arg("path") {
                Arg::NbtPath(p) => (**p).clone(),
                _ => return Ok(0),
            };
            Ok(data::path_count(&path, &tag))
        }
        // item containers on entities and blocks are not modelled yet
        _ => {
            let kind = sub(x, 2);
            if kind == "entity" {
                x.optional_entities("source")?;
            } else {
                x.block_pos("source")?;
            }
            Ok(0)
        }
    }
}

/// `checkRegions`: matching block count, `None` on the first mismatch.
fn check_regions(x: &mut Ctx) -> CmdResult<Option<i32>> {
    let skip_air = last(x) == "masked";
    let a = x.block_pos("start")?;
    let b = x.block_pos("end")?;
    let d = x.block_pos("destination")?;
    let min = [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])];
    let max = [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])];
    let area = (max[0] - min[0] + 1) as i64 * (max[1] - min[1] + 1) as i64 * (max[2] - min[2] + 1) as i64;
    if area > MAX_TEST_AREA {
        return Err(CmdError::tr("commands.execute.blocks.toobig", vec![lit(MAX_TEST_AREA as i32), lit(area as i32)]));
    }
    let off = [d[0] - min[0], d[1] - min[1], d[2] - min[2]];
    let blocks = &rine_data::get().blocks;
    let mut count = 0;
    for z in min[2]..=max[2] {
        for y in min[1]..=max[1] {
            for xx in min[0]..=max[0] {
                let s = x.g.world.get_block(xx, y, z);
                if skip_air && is_air(blocks.block_of(s).name.as_str()) {
                    continue;
                }
                let q = [xx + off[0], y + off[1], z + off[2]];
                if s != x.g.world.get_block(q[0], q[1], q[2]) {
                    return Ok(None);
                }
                if let Some(be_a) = data::block_entity_nbt(x.g, [xx, y, z]) {
                    let Some(be_b) = data::block_entity_nbt(x.g, q) else { return Ok(None) };
                    if strip_pos(be_a) != strip_pos(be_b) {
                        return Ok(None);
                    }
                }
                count += 1;
            }
        }
    }
    Ok(Some(count))
}

fn is_air(name: &str) -> bool {
    name.strip_prefix("minecraft:").unwrap_or(name) == "air"
}

/// `saveCustomOnly` comparison: block entity data without x/y/z/id.
fn strip_pos(mut t: Tag) -> Tag {
    if let Tag::Compound(v) = &mut t {
        v.retain(|(k, _)| !matches!(k.as_str(), "x" | "y" | "z" | "id"));
    }
    t
}

/// `storeData` for the shared `... <path> <type> <scale>` node.
fn store_data(x: &mut Ctx) -> CmdResult<Vec<Source>> {
    let result = sub(x, 1) == "result";
    let kind = sub(x, 2);
    let ty = x.lit(x.c.nodes.len() - 2);
    let scale = x.double("scale");
    let target = data::target(x, kind, "target")?;
    let path = match x.arg("path") {
        Arg::NbtPath(p) => (**p).clone(),
        _ => unreachable!(),
    };
    let cb: Callback = Some(Arc::new(move |g, ok, v| {
        let v = f64::from(if result { v } else { i32::from(ok) }) * scale;
        let tag = match ty {
            "int" => Tag::Int(v as i32),
            "float" => Tag::Float(v as f32),
            "short" => Tag::Short(v as i32 as i16),
            "long" => Tag::Long(v as i64),
            "double" => Tag::Double(v),
            _ => Tag::Byte(v as i32 as i8),
        };
        let Ok(mut root) = data::get_data(g, &target) else { return };
        if data::path_set(&path, &mut root, &tag) {
            let _ = data::set_data(g, &target, root);
        }
    }));
    Ok(vec![chain_store(x.src, cb)])
}

/// `Level.getHeight(type, x, z)` for `Heightmap.Types` ordinals
/// (WORLD_SURFACE_WG, WORLD_SURFACE, OCEAN_FLOOR_WG, OCEAN_FLOOR,
/// MOTION_BLOCKING, MOTION_BLOCKING_NO_LEAVES).
pub fn height(g: &Game, kind: u8, x: i32, z: i32) -> i32 {
    use rine_world::{ChunkPos, MIN_Y};
    let Some(slot) = g.world.chunks.get(&ChunkPos::new(x >> 4, z >> 4)) else { return MIN_Y };
    let i = ((x & 15) + (z & 15) * 16) as usize;
    let ch = &slot.chunk;
    let v = match kind {
        0 | 1 => ch.world_surface[i],
        4 => ch.motion_blocking[i],
        5 => ch.motion_blocking_no_leaves[i],
        _ => {
            // OCEAN_FLOOR: highest motion-blocking block, fluids excluded
            let blocks = &rine_data::get().blocks;
            let mut y = MIN_Y + rine_world::HEIGHT - 1;
            while y >= MIN_Y {
                let s = ch.get(x & 15, y, z & 15);
                let f = blocks.state(s).flags;
                if f & flag::SOLID != 0 && f & (flag::WATER | flag::LAVA) == 0 {
                    break;
                }
                y -= 1;
            }
            return y + 1;
        }
    };
    MIN_Y + i32::from(v)
}

fn biome_at(g: &Game, p: [i32; 3]) -> String {
    use rine_world::ChunkPos;
    let Some(slot) = g.world.chunks.get(&ChunkPos::new(p[0] >> 4, p[2] >> 4)) else { return String::new() };
    let id = slot.chunk.biome_at(p[0] & 15, p[1], p[2] & 15);
    rine_data::get().registries.synced("minecraft:worldgen/biome").name(u32::from(id)).unwrap_or("").to_string()
}

/// `BlockPredicate.test(BlockInWorld)`.
pub fn block_test(g: &Game, pred: &BlockPredicate, pos: [i32; 3]) -> bool {
    let blocks = &rine_data::get().blocks;
    let s = g.world.get_block(pos[0], pos[1], pos[2]);
    let name = blocks.block_of(s).name.as_str();
    let full = if name.contains(':') { name.to_string() } else { format!("minecraft:{name}") };
    let (ok, props, nbt) = match pred {
        BlockPredicate::Block { block, props, nbt } => (*block == full, props, nbt),
        BlockPredicate::Tag { tag, props, nbt } => (rine_data::get().registries.tag_contains("minecraft:block", tag, &full), props, nbt),
    };
    if !ok {
        return false;
    }
    if props.iter().any(|(k, v)| blocks.get_prop(s, k) != Some(v.as_str())) {
        return false;
    }
    match nbt {
        None => true,
        Some(t) => data::block_entity_nbt(g, pos).is_some_and(|be| data::compare_nbt(t, &be, true)),
    }
}
