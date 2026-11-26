//! /summon (`SummonCommand`).
//!
//! Rine spawns only `minecraft:item`, `minecraft:tnt` and
//! `minecraft:falling_block`. Other types fail with `commands.summon.failed`.

use crate::game::cmd::commands::teleport::in_spawnable_bounds;
use crate::game::cmd::select::text_from_tag;
use crate::game::cmd::source::Source;
use crate::game::cmd::{Commands, Ctx, EntMeta};
use crate::game::entity::{Ent, Kind};
use crate::stack::Stack;
use crate::game::Game;
use rine_command::{CmdError, CmdResult};
use rine_protocol::nbt::Tag;
use rine_protocol::text::Text;
use uuid::Uuid;

pub fn register(c: &mut Commands) {
    c.exec("summon entity", |x| {
        let pos = x.src.pos;
        let ty = x.id("entity").to_string();
        spawn(x, &ty, pos, Tag::compound(), true)
    });
    c.exec("summon entity pos", |x| {
        let pos = x.vec3("pos");
        let ty = x.id("entity").to_string();
        spawn(x, &ty, pos, Tag::compound(), true)
    });
    c.exec("summon entity pos nbt", |x| {
        let pos = x.vec3("pos");
        let ty = x.id("entity").to_string();
        let nbt = x.nbt("nbt").clone();
        spawn(x, &ty, pos, nbt, false)
    });
}

fn spawn(x: &mut Ctx, ty: &str, pos: [f64; 3], nbt: Tag, finalize: bool) -> CmdResult<i32> {
    let eid = create_entity(x.g, x.src, ty, pos, &nbt, finalize)?;
    let name = x.name_of(eid);
    x.success(Text::tr("commands.summon.success", vec![name]), true);
    Ok(1)
}

fn num(t: &Tag, k: &str) -> Option<f64> {
    match t.get(k)? {
        Tag::Byte(v) => Some(*v as f64),
        Tag::Short(v) => Some(*v as f64),
        Tag::Int(v) => Some(*v as f64),
        Tag::Long(v) => Some(*v as f64),
        Tag::Float(v) => Some(*v as f64),
        Tag::Double(v) => Some(*v),
        _ => None,
    }
}

fn flag(t: &Tag, k: &str) -> bool {
    num(t, k).is_some_and(|v| v != 0.0)
}

/// Three numbers from a list (`Motion`) or two (`Rotation`).
fn num_list(t: &Tag, k: &str, n: usize) -> Option<Vec<f64>> {
    match t.get(k)? {
        Tag::List(items) if items.len() == n => items
            .iter()
            .map(|v| match v {
                Tag::Byte(v) => Some(*v as f64),
                Tag::Short(v) => Some(*v as f64),
                Tag::Int(v) => Some(*v as f64),
                Tag::Long(v) => Some(*v as f64),
                Tag::Float(v) => Some(*v as f64),
                Tag::Double(v) => Some(*v),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

/// `ItemStack.CODEC` for the `Item` field; anything unreadable is empty.
fn item_stack(t: &Tag) -> Stack {
    let Some(item) = t.get("Item") else { return Stack::EMPTY };
    let name = match item.get("id") {
        Some(Tag::String(s)) => s.clone(),
        _ => return Stack::EMPTY,
    };
    let count = num(item, "count").unwrap_or(1.0) as i32;
    match rine_data::get().items.by_name(&name) {
        Some(info) if count >= 1 => Stack::new(info.id, count),
        _ => Stack::EMPTY,
    }
}

/// `BlockState.CODEC` for falling blocks; falls back to sand.
fn falling_state(t: &Tag) -> u16 {
    let blocks = &rine_data::get().blocks;
    let fallback = blocks.default_state("sand");
    let Some(bs) = t.get("BlockState") else { return fallback };
    let Some(Tag::String(name)) = bs.get("Name") else { return fallback };
    let mut s = name.clone();
    if let Some(Tag::Compound(props)) = bs.get("Properties") {
        if !props.is_empty() {
            s.push('[');
            let parts: Vec<String> = props
                .iter()
                .filter_map(|(k, v)| match v {
                    Tag::String(v) => Some(format!("{k}={v}")),
                    _ => None,
                })
                .collect();
            s.push_str(&parts.join(","));
            s.push(']');
        }
    }
    blocks.parse_state(&s).unwrap_or(fallback)
}

fn uuid_of(t: &Tag) -> Option<Uuid> {
    match t.get("UUID")? {
        Tag::IntArray(v) if v.len() == 4 => {
            let w = |i: usize| v[i] as u32 as u128;
            Some(Uuid::from_u128((w(0) << 96) | (w(1) << 64) | (w(2) << 32) | w(3)))
        }
        _ => None,
    }
}

/// `SummonCommand.createEntity`: bounds, load the type's fields, add it.
/// `randomize` (finalize) only affects mobs, which Rine cannot spawn.
pub fn create_entity(g: &mut Game, src: &Source, ty: &str, pos: [f64; 3], tag: &Tag, randomize: bool) -> CmdResult<i32> {
    let _ = (src, randomize);
    if !in_spawnable_bounds(pos) {
        return Err(CmdError::key("commands.summon.invalidPosition"));
    }
    let ty = ty.strip_prefix("minecraft:").unwrap_or(ty);
    let vel = num_list(tag, "Motion", 3).map(|v| [v[0], v[1], v[2]]).unwrap_or([0.0; 3]);
    let rot = num_list(tag, "Rotation", 2).map(|v| [v[1] as f32, v[0] as f32]).unwrap_or([0.0; 2]); // [xRot, yRot]

    let mut removed = false;
    let (kind, age) = match ty {
        "item" => {
            let stack = item_stack(tag);
            // an empty item is discarded on its first tick
            removed = stack.is_empty();
            let pickup = num(tag, "PickupDelay").unwrap_or(0.0) as i32;
            let age = num(tag, "Age").unwrap_or(0.0) as i32;
            (Kind::Item { stack, pickup_delay: pickup }, age)
        }
        "tnt" => (Kind::Tnt { fuse: num(tag, "fuse").unwrap_or(80.0) as i32 }, 0),
        "falling_block" => {
            let state = falling_state(tag);
            (Kind::Falling { state }, num(tag, "Time").unwrap_or(0.0) as i32)
        }
        _ => return Err(CmdError::key("commands.summon.failed")),
    };

    let uuid = match uuid_of(tag) {
        Some(u) => {
            let used = g.entities.list.iter().any(|e| e.uuid == u) || g.players.iter().flatten().any(|p| p.uuid == u);
            if used {
                return Err(CmdError::key("commands.summon.failed.uuid"));
            }
            u
        }
        None => Uuid::new_v4(),
    };

    let mut meta = EntMeta::default();
    if let Some(Tag::List(items)) = tag.get("Tags") {
        for item in items {
            if let Tag::String(s) = item {
                if meta.tags.len() < 1024 && !meta.tags.contains(s) {
                    meta.tags.push(s.clone());
                }
            }
        }
    }
    if let Some(name) = tag.get("CustomName") {
        meta.custom_name = Some(text_from_tag(name));
    }
    meta.custom_name_visible = flag(tag, "CustomNameVisible");
    meta.no_gravity = flag(tag, "NoGravity");
    meta.silent = flag(tag, "Silent");
    meta.glowing = flag(tag, "Glowing");
    meta.invulnerable = flag(tag, "Invulnerable");
    meta.rot = rot;

    let eid = g.alloc_eid();
    g.entities.spawn(Ent {
        eid,
        uuid,
        kind,
        pos,
        vel,
        on_ground: false,
        age,
        removed,
        sent_pos: pos,
        sent_vel: vel,
        asleep: false,
    });
    let default_meta = meta.tags.is_empty()
        && meta.custom_name.is_none()
        && !meta.custom_name_visible
        && !meta.no_gravity
        && !meta.silent
        && !meta.glowing
        && !meta.invulnerable
        && rot == [0.0, 0.0];
    if !default_meta {
        g.ent_meta.insert(eid, meta);
    }
    Ok(eid)
}
