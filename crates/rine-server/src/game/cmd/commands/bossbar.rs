//! /bossbar (`BossBarCommands`, `CustomBossEvents`, `CustomBossEvent`,
//! `ServerBossEvent`).
//!
//! A bar keeps the UUIDs of its players (`CustomBossEvent.players`, saved).
//! The online members are the players that are connected and in that set
//! (`ServerBossEvent.players`), so join and quit only (re)send packets.

use crate::game::cmd::feedback;
use crate::game::cmd::select;
use crate::game::cmd::{lit, Commands, Ctx};
use crate::game::Game;
use crate::out;
use bytes::BytesMut;
use rine_command::{CmdError, CmdResult};
use rine_protocol::frame::packet;
use rine_protocol::nbt::Tag;
use rine_protocol::text::{hover_text, Text};
use rine_protocol::Writer;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

/// Clientbound `boss_event` (protocol 777, vanilla packets.json).
const C_BOSS_EVENT: i32 = 9;

const OP_ADD: i32 = 0;
const OP_REMOVE: i32 = 1;
const OP_PROGRESS: i32 = 2;
const OP_NAME: i32 = 3;
const OP_STYLE: i32 = 4;
const OP_PROPERTIES: i32 = 5;

/// `BossEvent.BossBarColor` order.
const COLORS: [&str; 7] = ["pink", "blue", "red", "green", "yellow", "purple", "white"];
/// Chat formatting of each colour (`BossBarColor.getFormatting`).
const COLOR_FORMAT: [&str; 7] = ["red", "blue", "dark_red", "green", "yellow", "dark_blue", "white"];
/// `BossBarOverlay` order.
const OVERLAYS: [&str; 5] = ["progress", "notched_6", "notched_10", "notched_12", "notched_20"];

/// One custom boss bar (`CustomBossEvent` plus its `ServerBossEvent` state).
#[derive(Clone, Debug)]
pub struct Boss {
    pub uuid: u128,
    pub name: Text,
    pub visible: bool,
    pub value: i32,
    pub max: i32,
    pub progress: f32,
    pub color: u8,
    pub overlay: u8,
    pub darken: bool,
    pub music: bool,
    pub fog: bool,
    /// Player UUIDs in the bar (online or not).
    pub players: BTreeSet<u128>,
}

/// Custom boss bars by id (`CustomBossEvents`).
#[derive(Default)]
pub struct State {
    pub events: BTreeMap<String, Boss>,
}

fn new_boss(uuid: u128, name: Text) -> Boss {
    Boss {
        uuid,
        name,
        visible: true,
        value: 0,
        max: 100,
        progress: 0.0,
        color: 6,
        overlay: 0,
        darken: false,
        music: false,
        fog: false,
        players: BTreeSet::new(),
    }
}

/// `Mth.clamp((float)value / max, 0, 1)`.
fn progress_of(value: i32, max: i32) -> f32 {
    (value as f32 / max as f32).clamp(0.0, 1.0)
}

fn uuid_ints(u: u128) -> [i32; 4] {
    [(u >> 96) as i32, (u >> 64) as i32, (u >> 32) as i32, u as i32]
}

fn frame_to(g: &mut Game, slot: usize, p: BytesMut) {
    if let Some(Some(pl)) = g.players.get_mut(slot) {
        pl.send(p);
    }
}

/// Slots of online players that are in the bar.
fn online_members(g: &Game, b: &Boss) -> Vec<usize> {
    g.players
        .iter()
        .enumerate()
        .filter_map(|(s, p)| p.as_ref().filter(|p| b.players.contains(&p.uuid.as_u128())).map(|_| s))
        .collect()
}

fn boss_pkt(uuid: u128, op: i32, body: impl FnOnce(&mut BytesMut)) -> BytesMut {
    packet(C_BOSS_EVENT, |b| {
        b.w_uuid(uuid_from(uuid));
        b.w_varint(op);
        body(b);
    })
}

fn uuid_from(u: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(u)
}

fn add_pkt(b: &Boss) -> BytesMut {
    boss_pkt(b.uuid, OP_ADD, |w| {
        out::write_text(w, &b.name);
        w.w_f32(b.progress);
        w.w_varint(b.color as i32);
        w.w_varint(b.overlay as i32);
        w.w_u8(props(b));
    })
}

fn props(b: &Boss) -> u8 {
    (b.darken as u8) | ((b.music as u8) << 1) | ((b.fog as u8) << 2)
}

/// Packet for an update (`ServerBossEvent` setters), built from the bar.
fn update_pkt(b: &Boss, op: i32) -> BytesMut {
    match op {
        OP_PROGRESS => boss_pkt(b.uuid, op, |w| w.w_f32(b.progress)),
        OP_NAME => boss_pkt(b.uuid, op, |w| out::write_text(w, &b.name)),
        OP_STYLE => boss_pkt(b.uuid, op, |w| {
            w.w_varint(b.color as i32);
            w.w_varint(b.overlay as i32);
        }),
        _ => boss_pkt(b.uuid, OP_PROPERTIES, |w| w.w_u8(props(b))),
    }
}

/// `ServerBossEvent.broadcast`: to online members, only when visible.
fn broadcast(g: &mut Game, id: &str, op: i32) {
    let (slots, pkt) = {
        let Some(b) = g.cs.bossbar.events.get(id) else { return };
        if !b.visible {
            return;
        }
        (online_members(g, b), update_pkt(b, op))
    };
    for s in slots {
        frame_to(g, s, pkt.clone());
    }
}

/// `CustomBossEvent.setValue` (clamped), sends the progress update.
pub fn set_value(g: &mut Game, id: &str, v: i32) {
    let changed = {
        let Some(b) = g.cs.bossbar.events.get_mut(id) else { return };
        b.value = v;
        let p = progress_of(v, b.max);
        let changed = p != b.progress;
        b.progress = p;
        changed
    };
    if changed {
        broadcast(g, id, OP_PROGRESS);
    }
}

/// `CustomBossEvent.setMax`, sends the progress update.
pub fn set_max(g: &mut Game, id: &str, v: i32) {
    let changed = {
        let Some(b) = g.cs.bossbar.events.get_mut(id) else { return };
        b.max = v;
        let p = progress_of(b.value, v);
        let changed = p != b.progress;
        b.progress = p;
        changed
    };
    if changed {
        broadcast(g, id, OP_PROGRESS);
    }
}

/// A custom boss bar with this id exists.
pub fn exists(g: &Game, id: &str) -> bool {
    g.cs.bossbar.events.contains_key(id)
}

/// `CustomBossEvents.onPlayerConnect`: send an add packet for each bar the
/// joining player is a member of (visible bars only).
pub fn on_join(g: &mut Game, slot: usize) {
    let Some(uuid) = g.players.get(slot).and_then(|p| p.as_ref()).map(|p| p.uuid) else { return };
    let pkts: Vec<BytesMut> = g.cs.bossbar.events.values().filter(|b| b.visible && b.players.contains(&uuid.as_u128())).map(add_pkt).collect();
    for p in pkts {
        frame_to(g, slot, p);
    }
}

/// `CustomBossEvents.onPlayerDisconnect`: the player is no longer online,
/// so the bar stops listing it. The UUID stays in the bar (saved state).
pub fn on_quit(g: &mut Game, uuid: uuid::Uuid) {
    let _ = (g, uuid);
}

/// `CustomBossEvents` as a compound: id to bar fields (level.dat layout).
pub fn to_tag(g: &Game) -> Tag {
    let entries: Vec<(String, Tag)> = g
        .cs
        .bossbar
        .events
        .iter()
        .map(|(id, b)| {
            let players: Vec<Tag> = b.players.iter().map(|u| Tag::IntArray(uuid_ints(*u).to_vec())).collect();
            let t = Tag::compound()
                .with("Name", b.name.to_nbt())
                .with("Visible", Tag::Byte(b.visible as i8))
                .with("Value", Tag::Int(b.value))
                .with("Max", Tag::Int(b.max))
                .with("Color", Tag::String(COLORS[b.color as usize % 7].into()))
                .with("Overlay", Tag::String(OVERLAYS[b.overlay as usize % 5].into()))
                .with("DarkenScreen", Tag::Byte(b.darken as i8))
                .with("PlayBossMusic", Tag::Byte(b.music as i8))
                .with("CreateWorldFog", Tag::Byte(b.fog as i8))
                .with("Players", Tag::List(players));
            (id.clone(), t)
        })
        .collect();
    Tag::Compound(entries)
}

/// Replace the bars with those of a `CustomBossEvents` compound.
pub fn from_tag(g: &mut Game, t: &Tag) {
    let Tag::Compound(entries) = t else { return };
    let mut events = BTreeMap::new();
    for (id, e) in entries {
        let name = e.get("Name").map(select::text_from_tag).unwrap_or_else(|| Text::plain(id.clone()));
        let mut b = new_boss(super::random::random_u128(), name);
        b.visible = byte(e, "Visible") != 0;
        b.value = int(e, "Value");
        b.max = match e.get("Max") {
            Some(Tag::Int(v)) => *v,
            _ => 100,
        };
        b.color = match e.get("Color") {
            Some(Tag::String(s)) => COLORS.iter().position(|c| c == s).unwrap_or(6) as u8,
            _ => 6,
        };
        b.overlay = match e.get("Overlay") {
            Some(Tag::String(s)) => OVERLAYS.iter().position(|c| c == s).unwrap_or(0) as u8,
            _ => 0,
        };
        b.darken = byte(e, "DarkenScreen") != 0;
        b.music = byte(e, "PlayBossMusic") != 0;
        b.fog = byte(e, "CreateWorldFog") != 0;
        if let Some(Tag::List(ps)) = e.get("Players") {
            for p in ps {
                if let Tag::IntArray(v) = p {
                    if v.len() == 4 {
                        let u = ((v[0] as u32 as u128) << 96) | ((v[1] as u32 as u128) << 64) | ((v[2] as u32 as u128) << 32) | v[3] as u32 as u128;
                        b.players.insert(u);
                    }
                }
            }
        }
        b.progress = progress_of(b.value, b.max);
        events.insert(id.clone(), b);
    }
    g.cs.bossbar.events = events;
}

fn byte(t: &Tag, k: &str) -> i8 {
    match t.get(k) {
        Some(Tag::Byte(b)) => *b,
        _ => 0,
    }
}

fn int(t: &Tag, k: &str) -> i32 {
    match t.get(k) {
        Some(Tag::Int(v)) => *v,
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

pub fn register(c: &mut Commands) {
    c.exec("bossbar list", list);
    c.exec("bossbar add id name", add);
    c.exec("bossbar remove id", remove);
    c.exec("bossbar get id value", get_value);
    c.exec("bossbar get id max", get_max);
    c.exec("bossbar get id visible", get_visible);
    c.exec("bossbar get id players", get_players);
    c.exec("bossbar set id name name", set_name);
    c.exec("bossbar set id color pink", |x| set_color(x, 0));
    c.exec("bossbar set id color blue", |x| set_color(x, 1));
    c.exec("bossbar set id color red", |x| set_color(x, 2));
    c.exec("bossbar set id color green", |x| set_color(x, 3));
    c.exec("bossbar set id color yellow", |x| set_color(x, 4));
    c.exec("bossbar set id color purple", |x| set_color(x, 5));
    c.exec("bossbar set id color white", |x| set_color(x, 6));
    c.exec("bossbar set id style progress", |x| set_style(x, 0));
    c.exec("bossbar set id style notched_6", |x| set_style(x, 1));
    c.exec("bossbar set id style notched_10", |x| set_style(x, 2));
    c.exec("bossbar set id style notched_12", |x| set_style(x, 3));
    c.exec("bossbar set id style notched_20", |x| set_style(x, 4));
    c.exec("bossbar set id value value", set_value_cmd);
    c.exec("bossbar set id max max", set_max_cmd);
    c.exec("bossbar set id visible visible", set_visible);
    c.exec("bossbar set id players", set_players_none);
    c.exec("bossbar set id players targets", set_players);
}

/// `[name]` with the colour, hover (id) and insertion (id), as `getDisplayName`.
fn display_of(b: &Boss, id: &str) -> Text {
    let fmt = COLOR_FORMAT[b.color as usize % 7];
    Text::tr("chat.square_brackets", vec![b.name.clone().color(fmt).on_hover(hover_text(&Text::plain(id))).insertion(id)])
}

fn id_arg(x: &Ctx) -> String {
    x.id("id").to_string()
}

fn boss_of(x: &Ctx, id: &str) -> CmdResult<Boss> {
    x.g.cs.bossbar.events.get(id).cloned().ok_or_else(|| CmdError::tr("commands.bossbar.unknown", vec![lit(id)]))
}

fn join_texts(items: Vec<Text>) -> Text {
    crate::game::cmd::feedback::format_list(items, &crate::game::cmd::feedback::list_sep())
}

fn list(x: &mut Ctx) -> CmdResult<i32> {
    let (n, msg) = {
        let items: Vec<Text> = x.g.cs.bossbar.events.iter().map(|(id, b)| display_of(b, id)).collect();
        let n = items.len();
        let msg = if n == 0 {
            Text::key("commands.bossbar.list.bars.none")
        } else {
            Text::tr("commands.bossbar.list.bars.some", vec![lit(n), join_texts(items)])
        };
        (n, msg)
    };
    x.success(msg, false);
    Ok(n as i32)
}

fn add(x: &mut Ctx) -> CmdResult<i32> {
    let id = id_arg(x);
    let name = x.component("name")?;
    if x.g.cs.bossbar.events.contains_key(&id) {
        return Err(CmdError::tr("commands.bossbar.create.failed", vec![lit(&id)]));
    }
    let uuid = super::random::random_u128();
    let b = new_boss(uuid, name);
    let msg = Text::tr("commands.bossbar.create.success", vec![display_of(&b, &id)]);
    x.g.cs.bossbar.events.insert(id, b);
    let n = x.g.cs.bossbar.events.len();
    x.success(msg, true);
    Ok(n as i32)
}

/// `removeBar`: members get a remove packet, then the bar is dropped.
fn remove(x: &mut Ctx) -> CmdResult<i32> {
    let id = id_arg(x);
    let b = boss_of(x, &id)?;
    let msg = Text::tr("commands.bossbar.remove.success", vec![display_of(&b, &id)]);
    let slots = online_members(x.g, &b);
    if b.visible {
        for s in slots {
            frame_to(x.g, s, boss_pkt(b.uuid, OP_REMOVE, |_| {}));
        }
    }
    x.g.cs.bossbar.events.remove(&id);
    let n = x.g.cs.bossbar.events.len();
    x.success(msg, true);
    Ok(n as i32)
}

fn get_value(x: &mut Ctx) -> CmdResult<i32> {
    let id = id_arg(x);
    let b = boss_of(x, &id)?;
    x.success(Text::tr("commands.bossbar.get.value", vec![display_of(&b, &id), lit(b.value)]), true);
    Ok(b.value)
}

fn get_max(x: &mut Ctx) -> CmdResult<i32> {
    let id = id_arg(x);
    let b = boss_of(x, &id)?;
    x.success(Text::tr("commands.bossbar.get.max", vec![display_of(&b, &id), lit(b.max)]), true);
    Ok(b.max)
}

fn get_visible(x: &mut Ctx) -> CmdResult<i32> {
    let id = id_arg(x);
    let b = boss_of(x, &id)?;
    let (key, r) = if b.visible { ("commands.bossbar.get.visible.visible", 1) } else { ("commands.bossbar.get.visible.hidden", 0) };
    x.success(Text::tr(key, vec![display_of(&b, &id)]), true);
    Ok(r)
}

fn get_players(x: &mut Ctx) -> CmdResult<i32> {
    let id = id_arg(x);
    let b = boss_of(x, &id)?;
    let members = online_members(x.g, &b);
    let n = members.len();
    let disp = display_of(&b, &id);
    let msg = if n == 0 {
        Text::tr("commands.bossbar.get.players.none", vec![disp])
    } else {
        let names: Vec<Text> = members.iter().map(|s| feedback::player_display_name(x.g, *s)).collect();
        Text::tr("commands.bossbar.get.players.some", vec![disp, lit(n), join_texts(names)])
    };
    x.success(msg, true);
    Ok(n as i32)
}

fn set_name(x: &mut Ctx) -> CmdResult<i32> {
    let id = id_arg(x);
    let name = x.component("name")?;
    let b = boss_of(x, &id)?;
    if b.name == name {
        return Err(CmdError::key("commands.bossbar.set.name.unchanged"));
    }
    if let Some(bb) = x.g.cs.bossbar.events.get_mut(&id) {
        bb.name = name;
    }
    broadcast(x.g, &id, OP_NAME);
    let b = boss_of(x, &id)?;
    x.success(Text::tr("commands.bossbar.set.name.success", vec![display_of(&b, &id)]), true);
    Ok(0)
}

fn set_color(x: &mut Ctx, color: u8) -> CmdResult<i32> {
    let id = id_arg(x);
    let b = boss_of(x, &id)?;
    if b.color == color {
        return Err(CmdError::key("commands.bossbar.set.color.unchanged"));
    }
    if let Some(bb) = x.g.cs.bossbar.events.get_mut(&id) {
        bb.color = color;
    }
    broadcast(x.g, &id, OP_STYLE);
    let b = boss_of(x, &id)?;
    x.success(Text::tr("commands.bossbar.set.color.success", vec![display_of(&b, &id)]), true);
    Ok(0)
}

fn set_style(x: &mut Ctx, overlay: u8) -> CmdResult<i32> {
    let id = id_arg(x);
    let b = boss_of(x, &id)?;
    if b.overlay == overlay {
        return Err(CmdError::key("commands.bossbar.set.style.unchanged"));
    }
    if let Some(bb) = x.g.cs.bossbar.events.get_mut(&id) {
        bb.overlay = overlay;
    }
    broadcast(x.g, &id, OP_STYLE);
    let b = boss_of(x, &id)?;
    x.success(Text::tr("commands.bossbar.set.style.success", vec![display_of(&b, &id)]), true);
    Ok(0)
}

fn set_value_cmd(x: &mut Ctx) -> CmdResult<i32> {
    let id = id_arg(x);
    let v = x.int("value");
    let b = boss_of(x, &id)?;
    if b.value == v {
        return Err(CmdError::key("commands.bossbar.set.value.unchanged"));
    }
    set_value(x.g, &id, v);
    x.success(Text::tr("commands.bossbar.set.value.success", vec![display_of(&b, &id), lit(v)]), true);
    Ok(v)
}

fn set_max_cmd(x: &mut Ctx) -> CmdResult<i32> {
    let id = id_arg(x);
    let v = x.int("max");
    let b = boss_of(x, &id)?;
    if b.max == v {
        return Err(CmdError::key("commands.bossbar.set.max.unchanged"));
    }
    set_max(x.g, &id, v);
    x.success(Text::tr("commands.bossbar.set.max.success", vec![display_of(&b, &id), lit(v)]), true);
    Ok(v)
}

fn set_visible(x: &mut Ctx) -> CmdResult<i32> {
    let id = id_arg(x);
    let visible = x.bool("visible");
    let b = boss_of(x, &id)?;
    if b.visible == visible {
        let key = if visible { "commands.bossbar.set.visibility.unchanged.visible" } else { "commands.bossbar.set.visibility.unchanged.hidden" };
        return Err(CmdError::key(key));
    }
    let slots = online_members(x.g, &b);
    if let Some(bb) = x.g.cs.bossbar.events.get_mut(&id) {
        bb.visible = visible;
    }
    for s in slots {
        let p = if visible { add_pkt(&b) } else { boss_pkt(b.uuid, OP_REMOVE, |_| {}) };
        frame_to(x.g, s, p);
    }
    let key = if visible { "commands.bossbar.set.visible.success.visible" } else { "commands.bossbar.set.visible.success.hidden" };
    x.success(Text::tr(key, vec![display_of(&b, &id)]), true);
    Ok(0)
}

fn set_players_none(x: &mut Ctx) -> CmdResult<i32> {
    apply_players(x, Vec::new())
}

fn set_players(x: &mut Ctx) -> CmdResult<i32> {
    let targets = x.optional_players("targets")?;
    apply_players(x, targets)
}

/// `CustomBossEvent.setPlayers` plus the feedback message.
fn apply_players(x: &mut Ctx, targets: Vec<usize>) -> CmdResult<i32> {
    let id = id_arg(x);
    let b = boss_of(x, &id)?;
    let target_uuids: BTreeSet<u128> = targets.iter().filter_map(|s| x.g.players.get(*s).and_then(|p| p.as_ref()).map(|p| p.uuid.as_u128())).collect();
    let to_remove: Vec<u128> = b.players.iter().filter(|u| !target_uuids.contains(u)).copied().collect();
    let to_add: Vec<u128> = target_uuids.iter().filter(|u| !b.players.contains(u)).copied().collect();
    if to_remove.is_empty() && to_add.is_empty() {
        return Err(CmdError::key("commands.bossbar.set.players.unchanged"));
    }
    // Removals: online members get a remove packet (when visible).
    for u in &to_remove {
        let slot = x.g.players.iter().position(|p| p.as_ref().is_some_and(|p| p.uuid.as_u128() == *u));
        if let (Some(s), true) = (slot, b.visible) {
            frame_to(x.g, s, boss_pkt(b.uuid, OP_REMOVE, |_| {}));
        }
    }
    // Additions: add packets for online players (when visible).
    for u in &to_add {
        let slot = x.g.players.iter().position(|p| p.as_ref().is_some_and(|p| p.uuid.as_u128() == *u));
        if let (Some(s), true) = (slot, b.visible) {
            frame_to(x.g, s, add_pkt(&b));
        }
    }
    if let Some(bb) = x.g.cs.bossbar.events.get_mut(&id) {
        for u in &to_remove {
            bb.players.remove(u);
        }
        for u in &to_add {
            bb.players.insert(*u);
        }
    }
    let b2 = boss_of(x, &id)?;
    let online = online_members(x.g, &b2).len();
    let disp = display_of(&b2, &id);
    let msg = if target_uuids.is_empty() {
        Text::tr("commands.bossbar.set.players.success.none", vec![disp])
    } else {
        let names: Vec<Text> = targets.iter().map(|s| feedback::player_display_name(x.g, *s)).collect();
        Text::tr("commands.bossbar.set.players.success.some", vec![disp, lit(targets.len()), join_texts(names)])
    };
    x.success(msg, true);
    Ok(online as i32)
}
