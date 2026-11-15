//! /waypoint (`WaypointCommand`). Players are the waypoint transmitters
//! (Rine has no other living entities). Icon state (colour, style) is kept
//! per player UUID; a change is sent as UNTRACK then TRACK to every player.

use super::{hex_rgb, lit, uuid_of};
use crate::game::cmd::{Commands, Ctx};
use rine_command::args::Arg;
use rine_command::{CmdError, CmdResult};
use rine_protocol::frame::packet;
use rine_protocol::text::{click_suggest, hover_text, Text};
use rine_protocol::Writer;
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

/// `minecraft:waypoint` in packets.json.
const C_WAYPOINT: i32 = 141;
const OP_TRACK: i32 = 0;
const OP_UNTRACK: i32 = 1;
/// `TrackedWaypoint.Type.VEC3I` (ordinal 1) and `EMPTY` (ordinal 0).
const TYPE_EMPTY: i32 = 0;
const TYPE_VEC3I: i32 = 1;
const DEFAULT_STYLE: &str = "minecraft:default";
const OVERWORLD: &str = "minecraft:overworld";

/// `TeamColor` in ordinal order, with its `rgb()` value.
const TEAM_COLORS: [(&str, i32); 16] = [
    ("black", 0x000000),
    ("dark_blue", 0x0000AA),
    ("dark_green", 0x00AA00),
    ("dark_aqua", 0x00AAAA),
    ("dark_red", 0xAA0000),
    ("dark_purple", 0xAA00AA),
    ("gold", 0xFFAA00),
    ("gray", 0xAAAAAA),
    ("dark_gray", 0x555555),
    ("blue", 0x5555FF),
    ("green", 0x55FF55),
    ("aqua", 0x55FFFF),
    ("red", 0xFF5555),
    ("light_purple", 0xFF55FF),
    ("yellow", 0xFFFF55),
    ("white", 0xFFFFFF),
];

/// `Waypoint.Icon`.
#[derive(Clone, Debug, PartialEq)]
pub struct Icon {
    pub style: String,
    pub color: Option<i32>,
}

impl Default for Icon {
    fn default() -> Self {
        Self { style: DEFAULT_STYLE.to_string(), color: None }
    }
}

static ICONS: Mutex<Option<HashMap<Uuid, Icon>>> = Mutex::new(None);

pub fn register(c: &mut Commands) {
    c.exec("waypoint list", list);
    c.exec("waypoint modify waypoint color color", set_team_color);
    c.exec("waypoint modify waypoint color hex color", set_hex_color);
    c.exec("waypoint modify waypoint color reset", reset_color);
    c.exec("waypoint modify waypoint style reset", reset_style);
    c.exec("waypoint modify waypoint style set style", set_style);
}

/// Icon of a player, default when never changed.
pub fn icon_of(uuid: Uuid) -> Icon {
    let guard = ICONS.lock().unwrap_or_else(|e| e.into_inner());
    guard.as_ref().and_then(|m| m.get(&uuid).cloned()).unwrap_or_default()
}

fn update(uuid: Uuid, f: impl FnOnce(&mut Icon)) -> Icon {
    let mut guard = ICONS.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    let icon = map.entry(uuid).or_default();
    f(icon);
    icon.clone()
}

/// `ClientboundTrackedWaypointPacket`. Vec3i waypoints carry their block
/// position; UNTRACK carries an empty waypoint with the same identifier.
fn packet_for(op: i32, uuid: Uuid, icon: &Icon, pos: [i32; 3]) -> bytes::BytesMut {
    packet(C_WAYPOINT, |b| {
        b.w_varint(op);
        // Either<UUID, String>: left
        b.w_bool(true);
        b.w_uuid(uuid);
        // Waypoint.Icon: style key, optional RGB colour
        b.w_str(&icon.style);
        match icon.color {
            Some(c) => {
                b.w_bool(true);
                b.w_u8(((c >> 16) & 0xFF) as u8);
                b.w_u8(((c >> 8) & 0xFF) as u8);
                b.w_u8((c & 0xFF) as u8);
            }
            None => b.w_bool(false),
        }
        if op == OP_UNTRACK {
            b.w_varint(TYPE_EMPTY);
        } else {
            b.w_varint(TYPE_VEC3I);
            b.w_varint(pos[0]);
            b.w_varint(pos[1]);
            b.w_varint(pos[2]);
        }
    })
}

/// Re-track a player's waypoint for everyone: `untrackWaypoint` then `trackWaypoint`.
fn retrack(x: &mut Ctx, slot: usize, uuid: Uuid, icon: &Icon) {
    let pos = match x.g.players.get(slot).and_then(|p| p.as_ref()) {
        Some(p) => [p.pos[0].floor() as i32, p.pos[1].floor() as i32, p.pos[2].floor() as i32],
        None => [0, 0, 0],
    };
    let untrack = packet_for(OP_UNTRACK, uuid, &Icon::default(), pos);
    let track = packet_for(OP_TRACK, uuid, icon, pos);
    for s in 0..x.g.players.len() {
        if let Some(p) = x.g.players[s].as_mut() {
            p.send(untrack.clone());
            p.send(track.clone());
        }
    }
}

/// `WaypointArgument.getWaypoint`: the entity must be a transmitter (a player).
fn waypoint(x: &mut Ctx) -> CmdResult<(usize, Uuid)> {
    let eid = x.entity("waypoint")?;
    let slot = x.g.by_eid.get(&eid).copied().ok_or_else(|| CmdError::key("argument.waypoint.invalid"))?;
    let uuid = uuid_of(x, slot).ok_or_else(|| CmdError::key("argument.waypoint.invalid"))?;
    Ok((slot, uuid))
}

fn apply(x: &mut Ctx, slot: usize, uuid: Uuid, f: impl FnOnce(&mut Icon)) {
    let icon = update(uuid, f);
    retrack(x, slot, uuid, &icon);
}

/// `WaypointCommand.listWaypoints`: returns the number of transmitters.
fn list(x: &mut Ctx) -> CmdResult<i32> {
    let dim = OVERWORLD;
    let players: Vec<usize> = (0..x.g.players.len()).filter(|&s| x.g.players[s].is_some()).collect();
    if players.is_empty() {
        x.success(Text::tr("commands.waypoint.list.empty", vec![Text::plain(dim)]), false);
        return Ok(0);
    }
    let mut names = Text::empty();
    for (i, &slot) in players.iter().enumerate() {
        if i > 0 {
            names = names.push(Text::plain(", ").color("gray"));
        }
        let Some(p) = x.g.players[slot].as_ref() else { continue };
        let (bx, by, bz) = (p.pos[0].floor() as i32, p.pos[1].floor() as i32, p.pos[2].floor() as i32);
        let uuid = p.uuid;
        let name = p.name.clone();
        let colour = icon_of(uuid).color.map(hex_rgb).unwrap_or_else(|| "#FFFFFF".to_string());
        let entry = Text::plain(name)
            .color(colour)
            .on_click(click_suggest(format!("/execute in {dim} run tp @s {bx} {by} {bz}")))
            .on_hover(hover_text(&Text::key("chat.coordinates.tooltip")));
        names = names.push(entry);
    }
    let count = players.len();
    x.success(Text::tr("commands.waypoint.list.success", vec![lit(count), Text::plain(dim), names]), false);
    Ok(count as i32)
}

fn set_team_color(x: &mut Ctx) -> CmdResult<i32> {
    let (slot, uuid) = waypoint(x)?;
    let idx = match x.arg("color") {
        Arg::Color(v) => *v as usize,
        _ => return Err(super::stub(x)),
    };
    let (name, rgb) = TEAM_COLORS[idx.min(15)];
    apply(x, slot, uuid, |i| i.color = Some(rgb));
    x.success(Text::tr("commands.waypoint.modify.color", vec![Text::plain(name).color(hex_rgb(rgb))]), false);
    Ok(0)
}

fn set_hex_color(x: &mut Ctx) -> CmdResult<i32> {
    let (slot, uuid) = waypoint(x)?;
    let rgb = match x.arg("color") {
        Arg::HexColor(v) => *v & 0xFF_FFFF,
        _ => return Err(super::stub(x)),
    };
    apply(x, slot, uuid, |i| i.color = Some(rgb));
    let digits = format!("{rgb:06X}");
    x.success(Text::tr("commands.waypoint.modify.color", vec![Text::plain(digits).color(hex_rgb(rgb))]), false);
    Ok(0)
}

fn reset_color(x: &mut Ctx) -> CmdResult<i32> {
    let (slot, uuid) = waypoint(x)?;
    apply(x, slot, uuid, |i| i.color = None);
    x.success(Text::key("commands.waypoint.modify.color.reset"), false);
    Ok(0)
}

fn reset_style(x: &mut Ctx) -> CmdResult<i32> {
    let (slot, uuid) = waypoint(x)?;
    apply(x, slot, uuid, |i| i.style = DEFAULT_STYLE.to_string());
    x.success(Text::key("commands.waypoint.modify.style"), false);
    Ok(0)
}

fn set_style(x: &mut Ctx) -> CmdResult<i32> {
    let (slot, uuid) = waypoint(x)?;
    let style = x.id("style").to_string();
    apply(x, slot, uuid, |i| i.style = style);
    x.success(Text::key("commands.waypoint.modify.style"), false);
    Ok(0)
}
