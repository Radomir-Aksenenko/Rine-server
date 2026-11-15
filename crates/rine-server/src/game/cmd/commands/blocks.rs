//! /setblock, /fill, /clone, /fillbiome (`SetBlockCommand`, `FillCommand`,
//! `CloneCommands`, `FillBiomeCommand`).
//!
//! Block changes go through `World::set_block`, which batches client packets
//! per section. Neighbour updates run from the settle pass of the tick, so
//! `strict` only drops the positions this command queued for that pass.

use crate::game::cmd::commands::execute::block_test;
use crate::game::cmd::{lit, Commands, Ctx};
use crate::game::{containers, entity, interact, loot, Game};
use crate::out;
use rine_command::args::block::BlockPredicate;
use rine_command::args::Arg;
use rine_command::{CmdError, CmdResult};
use rine_protocol::nbt::Tag;
use rine_protocol::text::Text;
use rine_protocol::BlockPos;
use rine_world::chunk::BlockEntity;
use rine_world::{ChunkPos, MAX_Y, MIN_Y, SECTIONS};

const OVERWORLD: &str = "minecraft:overworld";
const BIOME_REG: &str = "minecraft:worldgen/biome";

pub fn register(c: &mut Commands) {
    c.exec("setblock pos block", |x| setblock(x, false, false, false));
    c.exec("setblock pos block replace", |x| setblock(x, false, false, false));
    c.exec("setblock pos block destroy", |x| setblock(x, true, false, false));
    c.exec("setblock pos block keep", |x| setblock(x, false, true, false));
    c.exec("setblock pos block strict", |x| setblock(x, false, false, true));
    // every executable path under each command gets the same executor;
    // the executor reads the literals of `x.path()` to pick the mode
    for p in paths(c, "fill ") {
        c.exec(p, fill_cmd);
    }
    for p in paths(c, "clone ") {
        c.exec(p, clone_cmd);
    }
    for p in paths(c, "fillbiome ") {
        c.exec(p, fillbiome_cmd);
    }
}

fn paths(c: &Commands, prefix: &str) -> Vec<&'static str> {
    c.tree.nodes.iter().filter(|n| n.executable && n.path.starts_with(prefix)).map(|n| n.path).collect()
}

// ---- shared helpers ------------------------------------------------------

fn bp(p: [i32; 3]) -> BlockPos {
    BlockPos::new(p[0], p[1], p[2])
}

fn in_world_y(y: i32) -> bool {
    (MIN_Y..=MAX_Y).contains(&y)
}

fn is_air(state: u16) -> bool {
    rine_data::get().blocks.is_air(state)
}

fn air_state() -> u16 {
    rine_data::get().blocks.default_state("minecraft:air")
}

/// `BlockEntityType` id for a block state, by vanilla type name. Only
/// block entity types that exist in the registry are returned.
fn be_type_id(state: u16) -> Option<u32> {
    let blocks = &rine_data::get().blocks;
    let name = blocks.block_of(state).name.as_str();
    let s = name.strip_prefix("minecraft:").unwrap_or(name);
    let ty = if s.ends_with("hanging_sign") {
        "hanging_sign"
    } else if s.ends_with("_sign") {
        "sign"
    } else if s.ends_with("_banner") {
        "banner"
    } else if s.ends_with("_bed") {
        "bed"
    } else if s.ends_with("shulker_box") {
        "shulker_box"
    } else {
        s
    };
    rine_data::get().registries.id("minecraft:block_entity_type", &format!("minecraft:{ty}")).or_else(|| {
        // exact block name for types that match it (chest, furnace, command_block...)
        rine_data::get().registries.id("minecraft:block_entity_type", &format!("minecraft:{s}"))
    })
}

/// Drop the keys `BlockEntity.loadCustomOnly` ignores (position and id).
fn clean_nbt(t: &Tag) -> Tag {
    match t {
        Tag::Compound(v) => Tag::Compound(v.iter().filter(|(k, _)| !matches!(k.as_str(), "x" | "y" | "z" | "id")).cloned().collect()),
        o => o.clone(),
    }
}

fn remove_be(g: &mut Game, p: [i32; 3]) {
    let Some(slot) = g.world.chunks.get_mut(&ChunkPos::new(p[0] >> 4, p[2] >> 4)) else { return };
    let (lx, lz, y) = ((p[0] & 15) as u8, (p[2] & 15) as u8, p[1] as i16);
    let before = slot.chunk.block_entities.len();
    slot.chunk.block_entities.retain(|b| !(b.x == lx && b.z == lz && b.y == y));
    if slot.chunk.block_entities.len() != before {
        slot.chunk.version += 1;
        slot.chunk.dirty = true;
    }
}

fn set_be(g: &mut Game, p: [i32; 3], type_id: u32, nbt: Tag) {
    let Some(slot) = g.world.chunks.get_mut(&ChunkPos::new(p[0] >> 4, p[2] >> 4)) else { return };
    let (lx, lz, y) = ((p[0] & 15) as u8, (p[2] & 15) as u8, p[1] as i16);
    slot.chunk.block_entities.retain(|b| !(b.x == lx && b.z == lz && b.y == y));
    slot.chunk.block_entities.push(BlockEntity { x: lx, y, z: lz, type_id, nbt: Some(nbt) });
    slot.chunk.version += 1;
    slot.chunk.dirty = true;
}

/// Block entity at `p`: its type and saved NBT.
fn be_at(g: &Game, p: [i32; 3]) -> Option<(u32, Tag)> {
    let slot = g.world.chunks.get(&ChunkPos::new(p[0] >> 4, p[2] >> 4))?;
    let (lx, lz, y) = ((p[0] & 15) as u8, (p[2] & 15) as u8, p[1] as i16);
    let be = slot.chunk.block_entities.iter().find(|b| b.x == lx && b.z == lz && b.y == y)?;
    Some((be.type_id, be.nbt.clone().unwrap_or_else(Tag::compound)))
}

/// Set a state like `level.setBlock(pos, state, flags)`: false when the
/// chunk is not loaded or the state did not change. A changed position
/// loses its old block entity.
fn set_state(g: &mut Game, p: [i32; 3], state: u16) -> bool {
    if !in_world_y(p[1]) || !g.world.is_loaded(p[0], p[2]) {
        return false;
    }
    let old = g.world.set_block(bp(p), state);
    if old == state {
        return false;
    }
    remove_be(g, p);
    true
}

/// `BlockInput.place`: the state, then the block entity NBT when the block
/// has one. False when nothing changed.
fn place(g: &mut Game, p: [i32; 3], state: u16, nbt: Option<&Tag>) -> bool {
    if !set_state(g, p, state) {
        return false;
    }
    if let (Some(t), Some(ty)) = (nbt, be_type_id(state)) {
        set_be(g, p, ty, clean_nbt(t));
    }
    true
}

/// `Level.destroyBlock(pos, true)`: break particles, loot, container
/// contents, then air (or water for waterlogged blocks). False for air.
fn destroy_block(g: &mut Game, p: [i32; 3]) -> bool {
    if !g.world.is_loaded(p[0], p[2]) {
        return false;
    }
    let st = g.world.get_block(p[0], p[1], p[2]);
    let blocks = &rine_data::get().blocks;
    if blocks.is_air(st) {
        return false;
    }
    let water = blocks.get_prop(st, "waterlogged") == Some("true");
    let new = if water { blocks.default_state("minecraft:water") } else { air_state() };
    let f = out::frame(out::level_event(2001, bp(p), st as i32));
    g.send_chunk_viewers(ChunkPos::new(p[0] >> 4, p[2] >> 4), &f);
    containers::block_removed(g, bp(p), st);
    set_state(g, p, new);
    for (n, d) in loot::block_drops(st, 0).into_iter().enumerate() {
        let off = entity::rand_f64((p[0] as u64) ^ ((p[2] as u64) << 20) ^ ((p[1] as u64) << 40) ^ g.tick ^ n as u64);
        entity::spawn_item(
            g,
            [p[0] as f64 + 0.25 + off * 0.5, p[1] as f64 + 0.25, p[2] as f64 + 0.25 + (1.0 - off) * 0.5],
            [(off - 0.5) * 0.2, 0.2, (0.5 - off) * 0.2],
            d,
            10,
        );
    }
    interact::gravity_check(g, bp([p[0], p[1] + 1, p[2]]));
    true
}

fn min_max(a: [i32; 3], b: [i32; 3]) -> ([i32; 3], [i32; 3]) {
    (
        [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])],
        [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])],
    )
}

/// Inclusive box volume (`BoundingBox` span product).
fn volume(lo: [i32; 3], hi: [i32; 3]) -> i64 {
    (hi[0] - lo[0] + 1) as i64 * (hi[1] - lo[1] + 1) as i64 * (hi[2] - lo[2] + 1) as i64
}

fn check_dimension(x: &Ctx, arg: &str) -> CmdResult<()> {
    let id = x.id(arg);
    if id != OVERWORLD {
        return Err(CmdError::tr("argument.dimension.invalid", vec![lit(id)]));
    }
    Ok(())
}

/// `ChunkMap` loaded check for the chunks spanned by `lo..=hi`.
fn chunks_loaded(g: &Game, lo: [i32; 3], hi: [i32; 3]) -> bool {
    for cz in (lo[2] >> 4)..=(hi[2] >> 4) {
        for cx in (lo[0] >> 4)..=(hi[0] >> 4) {
            if !g.world.chunks.contains_key(&ChunkPos::new(cx, cz)) {
                return false;
            }
        }
    }
    true
}

// ---- /setblock -----------------------------------------------------------

/// `SetBlockCommand.setBlock`. `keep` tests the target is air, `destroy`
/// breaks the old block first, `strict` skips the neighbour pass.
fn setblock(x: &mut Ctx, destroy: bool, keep: bool, strict: bool) -> CmdResult<i32> {
    let pos = x.block_pos("pos")?;
    let (state, nbt) = match x.arg("block") {
        Arg::Block(b) => (b.state, b.nbt.clone()),
        _ => return Err(CmdError::key("commands.setblock.failed")),
    };
    let g = &mut *x.g;
    if keep && !is_air(g.world.get_block(pos[0], pos[1], pos[2])) {
        return Err(CmdError::key("commands.setblock.failed"));
    }
    let mut place_needed = true;
    if destroy {
        destroy_block(g, pos);
        place_needed = !is_air(state) || !is_air(g.world.get_block(pos[0], pos[1], pos[2]));
    }
    let mark = g.world.touched.len();
    let placed = !place_needed || place(g, pos, state, nbt.as_ref());
    if strict {
        g.world.touched.truncate(mark);
    }
    if !placed {
        return Err(CmdError::key("commands.setblock.failed"));
    }
    x.success(Text::tr("commands.setblock.success", vec![lit(pos[0]), lit(pos[1]), lit(pos[2])]), true);
    Ok(1)
}

// ---- /fill ---------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum FillMode {
    Replace,
    Outline,
    Hollow,
    Destroy,
}

enum FillFilter {
    None,
    /// `keep`: only air positions.
    Air,
    Block(BlockPredicate),
}

/// Executor for every `fill` path; mode and strictness come from the path.
fn fill_cmd(x: &mut Ctx) -> CmdResult<i32> {
    let path = x.path();
    let mode = if path.contains(" outline") {
        FillMode::Outline
    } else if path.contains(" hollow") {
        FillMode::Hollow
    } else if path.contains(" destroy") {
        FillMode::Destroy
    } else {
        FillMode::Replace
    };
    let strict = path.ends_with(" strict");
    let filter = if path.contains(" keep") {
        FillFilter::Air
    } else if x.has("filter") {
        match x.arg("filter") {
            Arg::BlockPredicate(p) => FillFilter::Block((**p).clone()),
            _ => FillFilter::None,
        }
    } else {
        FillFilter::None
    };
    let from = x.block_pos("from")?;
    let to = x.block_pos("to")?;
    let (state, nbt) = match x.arg("block") {
        Arg::Block(b) => (b.state, b.nbt.clone()),
        _ => return Err(CmdError::key("commands.fill.failed")),
    };
    let (lo, hi) = min_max(from, to);
    let area = volume(lo, hi);
    let limit = x.g.rules.int("max_block_modifications");
    if area > limit as i64 {
        return Err(CmdError::tr("commands.fill.toobig", vec![lit(limit), lit(area)]));
    }
    let count = fill_blocks(&mut *x.g, lo, hi, state, nbt.as_ref(), mode, &filter, strict);
    if count == 0 {
        return Err(CmdError::key("commands.fill.failed"));
    }
    x.success(Text::tr("commands.fill.success", vec![lit(count)]), true);
    Ok(count)
}

/// `FillCommand.fillBlocks` loop: x fastest, then y, then z (`betweenClosed`).
#[allow(clippy::too_many_arguments)]
fn fill_blocks(g: &mut Game, lo: [i32; 3], hi: [i32; 3], state: u16, nbt: Option<&Tag>, mode: FillMode, filter: &FillFilter, strict: bool) -> i32 {
    let mark = g.world.touched.len();
    let air = air_state();
    let mut count = 0;
    for z in lo[2]..=hi[2] {
        for y in lo[1]..=hi[1] {
            for x in lo[0]..=hi[0] {
                let p = [x, y, z];
                let pass = match filter {
                    FillFilter::None => true,
                    FillFilter::Air => is_air(g.world.get_block(x, y, z)),
                    FillFilter::Block(pred) => block_test(g, pred, p),
                };
                if !pass {
                    continue;
                }
                let interior = x != lo[0] && x != hi[0] && y != lo[1] && y != hi[1] && z != lo[2] && z != hi[2];
                let affected = mode == FillMode::Destroy && destroy_block(g, p);
                let target = match mode {
                    FillMode::Outline if interior => None,
                    FillMode::Hollow if interior => Some((air, None)),
                    _ => Some((state, nbt)),
                };
                match target {
                    None => {
                        if affected {
                            count += 1;
                        }
                    }
                    Some((st, t)) => {
                        if place(g, p, st, t) || affected {
                            count += 1;
                        }
                    }
                }
            }
        }
    }
    if strict {
        g.world.touched.truncate(mark);
    }
    count
}

// ---- /clone --------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum CloneMode {
    Normal,
    Force,
    Move,
}

/// Executor for every `clone` path. Literals in the path pick the filter
/// (`masked`, `filtered`), the mode (`force`, `move`, `normal`) and `strict`.
fn clone_cmd(x: &mut Ctx) -> CmdResult<i32> {
    let path = x.path();
    let mode = if path.ends_with(" force") {
        CloneMode::Force
    } else if path.ends_with(" move") {
        CloneMode::Move
    } else {
        CloneMode::Normal
    };
    let strict = path.split(' ').any(|t| t == "strict");
    let masked = path.split(' ').any(|t| t == "masked");
    let filtered = path.split(' ').any(|t| t == "filtered");

    if x.has("sourceDimension") {
        check_dimension(x, "sourceDimension")?;
    }
    let begin = x.block_pos("begin")?;
    let end = x.block_pos("end")?;
    if x.has("targetDimension") {
        check_dimension(x, "targetDimension")?;
    }
    let dest = x.block_pos("destination")?;
    let filter = if filtered {
        match x.arg("filter") {
            Arg::BlockPredicate(p) => Some((**p).clone()),
            _ => None,
        }
    } else {
        None
    };

    // only the overworld exists, so source and target are always the same level
    let (from_lo, from_hi) = min_max(begin, end);
    let dest_hi = [dest[0] + from_hi[0] - from_lo[0], dest[1] + from_hi[1] - from_lo[1], dest[2] + from_hi[2] - from_lo[2]];
    let overlaps = dest[0] <= from_hi[0]
        && from_lo[0] <= dest_hi[0]
        && dest[1] <= from_hi[1]
        && from_lo[1] <= dest_hi[1]
        && dest[2] <= from_hi[2]
        && from_lo[2] <= dest_hi[2];
    if mode == CloneMode::Normal && overlaps {
        return Err(CmdError::key("commands.clone.overlap"));
    }
    let area = volume(from_lo, from_hi);
    let limit = x.g.rules.int("max_block_modifications");
    if area > limit as i64 {
        return Err(CmdError::tr("commands.clone.toobig", vec![lit(limit), lit(area)]));
    }
    let g = &mut *x.g;
    if !chunks_loaded(g, from_lo, from_hi) || !chunks_loaded(g, dest, dest_hi) {
        return Err(CmdError::key("argument.pos.unloaded"));
    }

    let blocks = &rine_data::get().blocks;
    let barrier = blocks.default_state("minecraft:barrier");
    let offset = [dest[0] - from_lo[0], dest[1] - from_lo[1], dest[2] - from_lo[2]];

    // (destination, state, block entity) in vanilla order: solid, block
    // entities, other blocks; `clear` lists the sources to empty for `move`.
    let mut solid = Vec::new();
    let mut with_be = Vec::new();
    let mut other = Vec::new();
    let mut clear = Vec::new();
    for z in from_lo[2]..=from_hi[2] {
        for y in from_lo[1]..=from_hi[1] {
            for x in from_lo[0]..=from_hi[0] {
                let src = [x, y, z];
                let dst = [x + offset[0], y + offset[1], z + offset[2]];
                let st = g.world.get_block(x, y, z);
                let pass = match (masked, &filter) {
                    (true, _) => !is_air(st),
                    (false, Some(pred)) => block_test(g, pred, src),
                    (false, None) => true,
                };
                if !pass {
                    continue;
                }
                clear.push(src);
                if let Some(be) = be_at(g, src) {
                    with_be.push((dst, st, Some(be)));
                } else if !blocks.has(st, rine_data::blocks::flag::OPAQUE) && !blocks.has(st, rine_data::blocks::flag::FULL_CUBE) {
                    other.push((dst, st, None));
                } else {
                    solid.push((dst, st, None));
                }
            }
        }
    }

    let mark = g.world.touched.len();
    if mode == CloneMode::Move {
        for &p in &clear {
            set_state(g, p, air_state());
        }
    }
    let placed: Vec<_> = solid.into_iter().chain(with_be).chain(other).collect();
    // vanilla counts a position when the copy differs from the barrier it
    // placed first, so a barrier copy is not counted
    let mut count = 0;
    for (dst, st, _) in &placed {
        if *st != barrier {
            count += 1;
        }
        set_state(g, *dst, *st);
    }
    for (dst, st, be) in &placed {
        if let Some((ty, nbt)) = be {
            if be_type_id(*st).is_some() {
                set_be(g, *dst, *ty, nbt.clone());
            }
        }
    }
    if strict {
        g.world.touched.truncate(mark);
    }
    if count == 0 {
        return Err(CmdError::key("commands.clone.failed"));
    }
    x.success(Text::tr("commands.clone.success", vec![lit(count)]), true);
    Ok(count)
}

// ---- /fillbiome ----------------------------------------------------------

/// Biome quart coordinate rounding: `QuartPos.toBlock(QuartPos.fromBlock(v))`.
fn quantize(v: i32) -> i32 {
    (v >> 2) << 2
}

fn fillbiome_cmd(x: &mut Ctx) -> CmdResult<i32> {
    let from = x.block_pos("from")?;
    let to = x.block_pos("to")?;
    let biome = x.id("biome").to_string();
    let filter: Option<(bool, String)> = if x.has("filter") {
        match x.arg("filter") {
            Arg::ResourceOrTag(r) => Some((r.tag, r.id.clone())),
            _ => None,
        }
    } else {
        None
    };
    let registries = &rine_data::get().registries;
    let reg = registries.synced(BIOME_REG);
    let Some(target) = reg.id(&biome) else {
        return Err(CmdError::tr("argument.id.invalid", Vec::new()));
    };
    let target = target as u8;

    let from = [quantize(from[0]), quantize(from[1]), quantize(from[2])];
    let to = [quantize(to[0]), quantize(to[1]), quantize(to[2])];
    let (lo, hi) = min_max(from, to);
    let volume = volume(lo, hi);
    let limit = x.g.rules.int("max_block_modifications");
    if volume > limit as i64 {
        return Err(CmdError::tr("commands.fillbiome.toobig", vec![lit(limit), lit(volume)]));
    }
    let g = &mut *x.g;
    for cz in (lo[2] >> 4)..=(hi[2] >> 4) {
        for cx in (lo[0] >> 4)..=(hi[0] >> 4) {
            if !g.world.chunks.contains_key(&ChunkPos::new(cx, cz)) {
                return Err(CmdError::key("argument.pos.unloaded"));
            }
        }
    }

    let passes = |b: u8| -> bool {
        match &filter {
            None => true,
            Some((tag, id)) => {
                let name = reg.name(u32::from(b)).unwrap_or("");
                if *tag {
                    registries.tag_contains(BIOME_REG, id, name)
                } else {
                    name == id
                }
            }
        }
    };

    let mut total: i32 = 0;
    let mut changed_chunks = Vec::new();
    for cz in (lo[2] >> 4)..=(hi[2] >> 4) {
        for cx in (lo[0] >> 4)..=(hi[0] >> 4) {
            let cp = ChunkPos::new(cx, cz);
            let Some(slot) = g.world.chunks.get_mut(&cp) else { continue };
            let mut changed = 0;
            for sy in 0..SECTIONS {
                let sec = &mut slot.chunk.sections[sy];
                for qy in 0..4 {
                    for qz in 0..4 {
                        for qx in 0..4 {
                            let bx = cx * 16 + qx * 4;
                            let by = MIN_Y + sy as i32 * 16 + qy * 4;
                            let bz = cz * 16 + qz * 4;
                            let inside = bx >= lo[0] && bx <= hi[0] && by >= lo[1] && by <= hi[1] && bz >= lo[2] && bz <= hi[2];
                            if !inside {
                                continue;
                            }
                            let idx = ((qy as usize) << 4) | ((qz as usize) << 2) | qx as usize;
                            let cur = sec.biomes[idx];
                            if !passes(cur) {
                                continue;
                            }
                            if cur != target {
                                changed += 1;
                            }
                            sec.biomes[idx] = target;
                        }
                    }
                }
            }
            if changed > 0 {
                slot.chunk.version += 1;
                slot.chunk.dirty = true;
                total += changed;
                changed_chunks.push(cp);
            }
        }
    }

    // resend the changed chunks (vanilla sends biomes only; Rine sends the chunk)
    for cp in changed_chunks {
        if let Some(f) = g.world.chunk_frame(cp) {
            g.send_chunk_viewers(cp, &f);
        }
    }
    if total == 0 {
        return Err(CmdError::key("commands.fillbiome.no_changes"));
    }
    x.success(
        Text::tr(
            "commands.fillbiome.success.count",
            vec![lit(total), lit(lo[0]), lit(lo[1]), lit(lo[2]), lit(hi[0]), lit(hi[1]), lit(hi[2])],
        ),
        true,
    );
    Ok(total)
}
