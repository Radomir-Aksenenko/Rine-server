//! /locate (`LocateCommand`).
//!
//! Biomes are searched over the loaded chunks: candidate points lie on a
//! 32-block grid around the source within 6400 blocks, sampled at the source
//! height. Vanilla samples the biome source (which covers unloaded terrain)
//! over several heights, so a biome that exists only in unloaded chunks is
//! reported as not found. Structures and POIs are not generated in Rine, so
//! they are never found.

use super::brackets;
use crate::game::cmd::{Commands, Ctx};
use rine_command::args::misc::ResourceOrTag;
use rine_command::args::Arg;
use rine_command::{CmdError, CmdResult};
use rine_protocol::text::{click_suggest, hover_text, Text};
use rine_world::ChunkPos;

const BIOME_REGISTRY: &str = "minecraft:worldgen/biome";
const MAX_BIOME_RADIUS: i64 = 6400;
const BIOME_STEP: i64 = 32;

pub fn register(c: &mut Commands) {
    c.exec("locate structure structure", structure);
    c.exec("locate biome biome", biome);
    c.exec("locate poi poi", poi);
}

/// `ResourceOrTagArgument.Result.asPrintable`: `#tag` or the element id.
fn printable(r: &ResourceOrTag) -> String {
    if r.tag {
        format!("#{}", r.id)
    } else {
        r.id.clone()
    }
}

fn target(x: &Ctx, name: &str) -> CmdResult<ResourceOrTag> {
    match x.arg(name) {
        Arg::ResourceOrTag(r) => Ok(r.clone()),
        _ => Err(super::stub(x)),
    }
}

fn structure(x: &mut Ctx) -> CmdResult<i32> {
    let r = target(x, "structure")?;
    Err(CmdError::tr("commands.locate.structure.not_found", vec![Text::plain(printable(&r))]))
}

fn poi(x: &mut Ctx) -> CmdResult<i32> {
    let r = target(x, "poi")?;
    Err(CmdError::tr("commands.locate.poi.not_found", vec![Text::plain(printable(&r))]))
}

/// Biome ids (registry ids stored in chunk biome bytes) that match the target.
fn matching_biomes(r: &ResourceOrTag) -> Vec<bool> {
    let data = rine_data::get();
    let reg = data.registries.reg(BIOME_REGISTRY);
    let mut ok = vec![false; 256];
    for (id, slot) in ok.iter_mut().enumerate() {
        let Some(name) = reg.name(id as u32) else { continue };
        let bare = name.strip_prefix("minecraft:").unwrap_or(name);
        *slot = if r.tag {
            let tag = r.id.strip_prefix("minecraft:").unwrap_or(&r.id);
            data.registries.tag_contains(BIOME_REGISTRY, tag, bare)
        } else {
            name == r.id
        };
    }
    ok
}

fn biome(x: &mut Ctx) -> CmdResult<i32> {
    let r = target(x, "biome")?;
    let ok = matching_biomes(&r);
    let src = x.src.pos;
    let (sx, sy, sz) = (src[0].floor() as i64, src[1].floor() as i32, src[2].floor() as i64);
    let mut best: Option<(i64, [i32; 3], u8)> = None;
    let steps = MAX_BIOME_RADIUS / BIOME_STEP;
    for gx in -steps..=steps {
        for gz in -steps..=steps {
            let (dx, dz) = (gx * BIOME_STEP, gz * BIOME_STEP);
            let d2 = dx * dx + dz * dz;
            if d2 > MAX_BIOME_RADIUS * MAX_BIOME_RADIUS {
                continue;
            }
            if let Some(best) = best {
                if best.0 <= d2 {
                    continue;
                }
            }
            let (wx, wz) = ((sx + dx) as i32, (sz + dz) as i32);
            let Some(slot) = x.g.world.chunks.get(&ChunkPos::new(wx.div_euclid(16), wz.div_euclid(16))) else { continue };
            let b = slot.chunk.biome_at(wx, sy, wz);
            if ok[b as usize] {
                best = Some((d2, [wx, sy, wz], b));
            }
        }
    }
    let Some((_, pos, found)) = best else {
        return Err(CmdError::tr("commands.locate.biome.not_found", vec![Text::plain(printable(&r))]));
    };
    let found_name = rine_data::get().registries.reg(BIOME_REGISTRY).name(found as u32).unwrap_or("").to_string();
    let label = if r.tag {
        format!("{} ({found_name})", printable(&r))
    } else {
        printable(&r)
    };
    let dx = (pos[0] as i64 - sx) as f32;
    let dy = (pos[1] - sy) as f32;
    let dz = (pos[2] as i64 - sz) as f32;
    let dist = (dx * dx + dy * dy + dz * dz).sqrt().floor() as i32;
    let coords = locate_coords(pos, true);
    x.success(Text::tr("commands.locate.biome.success", vec![Text::plain(label), coords, Text::plain(dist.to_string())]), false);
    Ok(dist)
}

/// `coordinates` in `showLocateResult`: green `[x, y, z]` with a tp suggestion.
fn locate_coords(pos: [i32; 3], include_y: bool) -> Text {
    let y = if include_y { pos[1].to_string() } else { "~".to_string() };
    let inner = Text::tr(
        "chat.coordinates",
        vec![Text::plain(pos[0].to_string()), Text::plain(y.clone()), Text::plain(pos[2].to_string())],
    );
    brackets(inner)
        .color("green")
        .on_click(click_suggest(format!("/tp @s {} {} {}", pos[0], y, pos[2])))
        .on_hover(hover_text(&Text::key("chat.coordinates.tooltip")))
}
