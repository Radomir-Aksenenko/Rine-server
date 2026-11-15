//! /posteffect (`PostEffectCommand`). The list of post effects lives on the
//! player (`ServerPlayer.postEffects`); here it is kept per player UUID and
//! sent with `ClientboundPostEffectsPacket` when it changes.

use super::tracker::Tracker;
use super::{lit, uuid_of};
use crate::game::cmd::{Commands, Ctx};
use rine_command::{CmdError, CmdResult};
use rine_protocol::frame::packet;
use rine_protocol::text::Text;
use rine_protocol::Writer;
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

/// `minecraft:post_effects` in packets.json.
const C_POST_EFFECTS: i32 = 83;

static EFFECTS: Mutex<Option<HashMap<Uuid, Vec<String>>>> = Mutex::new(None);

pub fn register(c: &mut Commands) {
    c.exec("posteffect add targets posteffect", add);
    c.exec("posteffect remove targets posteffect", remove);
    c.exec("posteffect clear targets", clear);
    c.exec("posteffect list target", list);
}

/// Run `f` on a player's list. Returns what `f` returns.
fn with_list<R>(uuid: Uuid, f: impl FnOnce(&mut Vec<String>) -> R) -> R {
    let mut guard = EFFECTS.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    f(map.entry(uuid).or_default())
}

fn snapshot(uuid: Uuid) -> Vec<String> {
    with_list(uuid, |v| v.clone())
}

/// `ClientboundPostEffectsPacket`: varint count, then the identifiers.
fn packet_for(list: &[String]) -> bytes::BytesMut {
    packet(C_POST_EFFECTS, |b| {
        b.w_varint(list.len() as i32);
        for s in list {
            b.w_str(s);
        }
    })
}

/// Send the current list to one player.
fn push(x: &mut Ctx, slot: usize, uuid: Uuid) {
    let body = packet_for(&snapshot(uuid));
    if let Some(p) = x.g.player(slot) {
        p.send(body);
    }
}

fn add(x: &mut Ctx) -> CmdResult<i32> {
    let slots = x.players("targets")?;
    let id = x.id("posteffect").to_string();
    let mut t = Tracker::new();
    for slot in slots {
        let Some(uuid) = uuid_of(x, slot) else { continue };
        let changed = with_list(uuid, |v| {
            if v.contains(&id) {
                false
            } else {
                v.push(id.clone());
                true
            }
        });
        if changed {
            push(x, slot, uuid);
        }
        t.track_bool(slot, changed);
    }
    t.send(x, Some("commands.posteffect.add.failed"), "commands.posteffect.add.success.single", "commands.posteffect.add.success.multiple", &[lit(&id)])
}

fn remove(x: &mut Ctx) -> CmdResult<i32> {
    let slots = x.players("targets")?;
    let id = x.id("posteffect").to_string();
    let mut t = Tracker::new();
    for slot in slots {
        let Some(uuid) = uuid_of(x, slot) else { continue };
        let changed = with_list(uuid, |v| match v.iter().position(|e| *e == id) {
            Some(i) => {
                v.remove(i);
                true
            }
            None => false,
        });
        if changed {
            push(x, slot, uuid);
        }
        t.track_bool(slot, changed);
    }
    t.send(x, Some("commands.posteffect.remove.failed"), "commands.posteffect.remove.success.single", "commands.posteffect.remove.success.multiple", &[lit(&id)])
}

fn clear(x: &mut Ctx) -> CmdResult<i32> {
    let slots = x.players("targets")?;
    let mut t = Tracker::new();
    for slot in slots {
        let Some(uuid) = uuid_of(x, slot) else { continue };
        let changed = with_list(uuid, |v| {
            if v.is_empty() {
                false
            } else {
                v.clear();
                true
            }
        });
        if changed {
            push(x, slot, uuid);
        }
        t.track_bool(slot, changed);
    }
    t.send(x, Some("commands.posteffect.clear.failed"), "commands.posteffect.clear.success.single", "commands.posteffect.clear.success.multiple", &[])
}

/// `PostEffectCommand.listPostEffects`: returns the number of effects.
fn list(x: &mut Ctx) -> CmdResult<i32> {
    let slot = x.player("target")?;
    let Some(uuid) = uuid_of(x, slot) else {
        return Err(CmdError::key("argument.entity.notfound.player"));
    };
    let list = snapshot(uuid);
    let name = x.player_name(slot);
    if list.is_empty() {
        x.success(Text::tr("commands.posteffect.list.empty", vec![name]), false);
    } else {
        let names = list.join(", ");
        x.success(Text::tr("commands.posteffect.list.success", vec![name, lit(list.len()), Text::plain(names)]), false);
    }
    Ok(list.len() as i32)
}
