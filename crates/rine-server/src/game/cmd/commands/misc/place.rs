//! /place (`PlaceCommand`). Rine has no configured-feature placement from the
//! worldgen crate, no structure generator, no jigsaw pools and no structure
//! templates. Each form runs the argument and loaded-chunk checks that come
//! before the placement in vanilla, then fails with the vanilla message:
//! `feature.failed`, `structure.failed`, `template.invalid` (no template
//! file exists), `jigsaw.failed`.

use crate::game::cmd::{Commands, Ctx};
use rine_command::{CmdError, CmdResult};
use rine_protocol::text::Text;
use rine_world::ChunkPos;

pub fn register(c: &mut Commands) {
    c.exec("place feature feature", feature);
    c.exec("place feature feature pos", feature);
    c.exec("place structure structure", structure);
    c.exec("place structure structure pos", structure);
    c.exec("place jigsaw pool target max_depth", jigsaw);
    c.exec("place jigsaw pool target max_depth position", jigsaw);
    c.exec("place template template", template);
    c.exec("place template template pos", template);
    c.exec("place template template pos rotation", template);
    c.exec("place template template pos rotation mirror", template);
    c.exec("place template template pos rotation mirror integrity", template);
    c.exec("place template template pos rotation mirror integrity seed", template);
    c.exec("place template template pos rotation mirror integrity seed strict", template);
}

/// Block position from the optional `pos` argument (`getLoadedBlockPos`),
/// or the source position floored (`BlockPos.containing`).
fn pos_or_source(x: &Ctx, name: &str) -> CmdResult<[i32; 3]> {
    if x.has(name) {
        x.block_pos(name)
    } else {
        let p = x.src.pos;
        Ok([p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32])
    }
}

/// `checkLoaded` over the chunk range `[min, max]` (inclusive).
fn check_loaded(x: &Ctx, min: ChunkPos, max: ChunkPos) -> CmdResult<()> {
    for cx in min.x..=max.x {
        for cz in min.z..=max.z {
            if !x.g.world.is_loaded(cx * 16, cz * 16) {
                return Err(CmdError::key("argument.pos.unloaded"));
            }
        }
    }
    Ok(())
}

fn chunk_of(p: [i32; 3]) -> ChunkPos {
    ChunkPos::new(p[0].div_euclid(16), p[2].div_euclid(16))
}

fn feature(x: &mut Ctx) -> CmdResult<i32> {
    let pos = pos_or_source(x, "pos")?;
    let c = chunk_of(pos);
    check_loaded(x, ChunkPos::new(c.x - 1, c.z - 1), ChunkPos::new(c.x + 1, c.z + 1))?;
    Err(CmdError::key("commands.place.feature.failed"))
}

fn structure(x: &mut Ctx) -> CmdResult<i32> {
    let _ = pos_or_source(x, "pos")?;
    Err(CmdError::key("commands.place.structure.failed"))
}

fn jigsaw(x: &mut Ctx) -> CmdResult<i32> {
    let pos = pos_or_source(x, "position")?;
    let c = chunk_of(pos);
    check_loaded(x, c, c)?;
    Err(CmdError::key("commands.place.jigsaw.failed"))
}

/// `placeTemplate`: `StructureTemplateManager.get` finds no template, so the
/// template is invalid. The loaded-position argument is checked first.
fn template(x: &mut Ctx) -> CmdResult<i32> {
    let _ = pos_or_source(x, "pos")?;
    let id = x.id("template").to_string();
    Err(CmdError::tr("commands.place.template.invalid", vec![Text::plain(id)]))
}
