//! Serverbound play packet dispatch (docs/spec/03 section 5).

use super::Game;
use crate::out;
use rine_protocol::ids::play::*;
use rine_protocol::text::Text;
use rine_protocol::{ProtoError, Reader};
use std::time::Instant;

const MAX_PACKETS_PER_TICK: usize = 400;

pub fn process_inbound(g: &mut Game, slot: usize) {
    for _ in 0..MAX_PACKETS_PER_TICK {
        let Some(p) = g.player(slot) else { return };
        let Ok(pk) = p.inbound.try_recv() else { return };
        if let Err(e) = handle(g, slot, &pk) {
            if let Some(p) = g.player(slot) {
                tracing::debug!("{}: bad packet: {e}", p.name);
                p.kick = Some(format!("Ошибка протокола: {e}"));
            }
            return;
        }
    }
}

fn handle(g: &mut Game, slot: usize, data: &[u8]) -> Result<(), ProtoError> {
    let mut r = Reader::new(data);
    let id = r.varint()?;
    match id {
        S_ACCEPT_TELEPORTATION => {
            let tid = r.varint()?;
            let p = g.player(slot).unwrap();
            if tid == p.tp_id {
                p.tp_pending = false;
            }
        }
        S_KEEP_ALIVE => {
            let kid = r.i64()?;
            let p = g.player(slot).unwrap();
            if kid == p.ka_id {
                if let Some(t) = p.ka_sent.take() {
                    p.ping_ms = t.elapsed().as_millis() as i32;
                }
                p.last_ka_reply = Instant::now();
            }
        }
        S_MOVE_PLAYER_POS => {
            let (x, y, z) = (r.f64()?, r.f64()?, r.f64()?);
            let f = r.u8()?;
            on_move(g, slot, Some((x, y, z)), None, f & 1 != 0);
        }
        S_MOVE_PLAYER_POS_ROT => {
            let (x, y, z) = (r.f64()?, r.f64()?, r.f64()?);
            let (yaw, pitch) = (r.f32()?, r.f32()?);
            let f = r.u8()?;
            on_move(g, slot, Some((x, y, z)), Some((yaw, pitch)), f & 1 != 0);
        }
        S_MOVE_PLAYER_ROT => {
            let (yaw, pitch) = (r.f32()?, r.f32()?);
            let f = r.u8()?;
            on_move(g, slot, None, Some((yaw, pitch)), f & 1 != 0);
        }
        S_MOVE_PLAYER_STATUS_ONLY => {
            let f = r.u8()?;
            on_move(g, slot, None, None, f & 1 != 0);
        }
        S_CHUNK_BATCH_RECEIVED => {
            let d = r.f32()?;
            let p = g.player(slot).unwrap();
            p.unacked = p.unacked.saturating_sub(1);
            p.desired = if d.is_nan() { 0.01 } else { d.clamp(0.01, 64.0) };
            p.max_unacked = 10;
            if p.unacked == 0 {
                p.quota = 1.0;
            }
        }
        S_CHAT => {
            let msg = r.string(256)?.to_string();
            chat(g, slot, &msg);
        }
        S_CHAT_COMMAND | S_CHAT_COMMAND_SIGNED => {
            let cmd = r.string(32767)?.to_string();
            super::commands::run(g, slot, &cmd);
        }
        S_PLAYER_ACTION => {
            let action = r.varint()?;
            let pos = r.block_pos()?;
            let dir = r.u8()?;
            let seq = r.varint()?;
            super::interact::player_action(g, slot, action, pos, dir, seq);
        }
        S_USE_ITEM_ON => {
            let hand = r.varint()?;
            let pos = r.block_pos()?;
            let face = r.varint()?;
            let (cx, cy, cz) = (r.f32()?, r.f32()?, r.f32()?);
            let _inside = r.bool()?;
            let _border = r.bool()?;
            let seq = r.varint()?;
            super::interact::use_item_on(g, slot, hand, pos, face as u8, (cx, cy, cz), seq);
        }
        S_USE_ITEM => {
            let hand = r.varint()?;
            let seq = r.varint()?;
            let (yaw, pitch) = (r.f32()?, r.f32()?);
            super::interact::use_item(g, slot, hand, seq, yaw, pitch);
        }
        S_SET_CARRIED_ITEM => {
            let s = r.i16()?;
            if (0..9).contains(&s) {
                g.player(slot).unwrap().held = s as u8;
                super::track::equipment_changed(g, slot);
            }
        }
        S_SET_CREATIVE_MODE_SLOT => {
            let s = r.i16()?;
            let count = r.varint()?;
            let p = g.player(slot).unwrap();
            if p.gamemode != 1 {
                return Ok(());
            }
            let stack = if count > 0 {
                let item = r.varint()? as u32;
                // component patch (delimited): we keep only item + count for now
                super::player::Stack::new(item, count.min(99))
            } else {
                super::player::Stack::EMPTY
            };
            if (0..46).contains(&s) {
                p.inv.slots[s as usize] = stack;
                super::track::equipment_changed(g, slot);
            } else if s == -1 && !stack.is_empty() {
                super::entity::drop_from_player(g, slot, stack, true);
            }
        }
        S_CONTAINER_CLICK => {
            super::interact::container_click(g, slot, &mut r)?;
        }
        S_CONTAINER_CLOSE => {
            let _w = r.varint()?;
            super::interact::container_close(g, slot);
        }
        S_PUNCH => {
            let p = g.player(slot).unwrap();
            let f = out::frame(out::swing(p.eid, 0));
            let eid = p.eid;
            super::track::send_to_trackers(g, eid, &f);
        }
        S_ATTACK => {
            let target = r.varint()?;
            super::interact::attack(g, slot, target);
        }
        S_INTERACT => {}
        S_PLAYER_COMMAND => {
            let _eid = r.varint()?;
            let action = r.varint()?;
            let p = g.player(slot).unwrap();
            match action {
                1 => p.sprinting = true,
                2 => p.sprinting = false,
                _ => {}
            }
            super::track::flags_changed(g, slot);
        }
        S_PLAYER_INPUT => {
            let flags = r.u8()?;
            let p = g.player(slot).unwrap();
            let sneak = flags & 0x20 != 0;
            if sneak != p.sneaking {
                p.sneaking = sneak;
                super::track::flags_changed(g, slot);
            }
        }
        S_PLAYER_ABILITIES => {
            let f = r.u8()?;
            let p = g.player(slot).unwrap();
            if p.gamemode == 1 || p.gamemode == 3 {
                p.flying = f & 0x02 != 0;
            }
        }
        S_CLIENT_COMMAND => {
            let action = r.varint()?;
            if action == 0 {
                super::interact::respawn(g, slot);
            }
        }
        S_CLIENT_INFORMATION => {
            let _lang = r.string(16)?;
            let vd = r.i8()?;
            let _chat = r.varint()?;
            let _colors = r.bool()?;
            let skin = r.u8()?;
            let p = g.player(slot).unwrap();
            p.info.skin_parts = skin;
            let max = g.shared.config.view_distance as i32;
            let p = g.player(slot).unwrap();
            let nv = (vd as i32).clamp(2, max);
            if nv != p.view {
                p.view = nv;
                p.stream_dirty = true;
            }
        }
        S_PING_REQUEST => {
            let t = r.i64()?;
            let p = g.player(slot).unwrap();
            p.send(rine_protocol::frame::packet(63, |b| {
                use rine_protocol::Writer;
                b.w_i64(t)
            }));
        }
        _ => {} // chat_ack, session update, tick end, loaded, custom payload...
    }
    Ok(())
}

fn on_move(g: &mut Game, slot: usize, pos: Option<(f64, f64, f64)>, rot: Option<(f32, f32)>, on_ground: bool) {
    let p = g.player(slot).unwrap();
    if p.tp_pending {
        return; // ignore until the client confirmed our teleport
    }
    if let Some((x, y, z)) = pos {
        if !(x.is_finite() && y.is_finite() && z.is_finite()) || x.abs() > 3.0e7 || z.abs() > 3.0e7 {
            p.kick = Some("Недопустимая позиция".into());
            return;
        }
        let dx = x - p.pos[0];
        let dy = y - p.pos[1];
        let dz = z - p.pos[2];
        let d2 = dx * dx + dy * dy + dz * dz;
        // generous anti-teleport check (elytra/riptide can be fast)
        if d2 > 100.0 * 100.0 && p.gamemode != 1 {
            let (px, py, pz, yaw, pitch) = (p.pos[0], p.pos[1], p.pos[2], p.yaw, p.pitch);
            p.teleport(px, py, pz, yaw, pitch);
            return;
        }
        // fall damage bookkeeping (survival)
        if !on_ground && y > p.fall_start {
            p.fall_start = y;
        }
        let was_ground = p.on_ground;
        p.pos = [x, y, z];
        if on_ground && !was_ground {
            let fall = p.fall_start - y;
            p.fall_start = y;
            if fall > 3.0 && (p.gamemode == 0 || p.gamemode == 2) && !p.flying {
                let dmg = (fall - 3.0).ceil() as f32;
                super::interact::damage(g, slot, dmg, "fall");
            }
        } else if on_ground {
            p.fall_start = y;
        }
    }
    let p = g.player(slot).unwrap();
    if let Some((yaw, pitch)) = rot {
        p.yaw = yaw;
        p.pitch = pitch.clamp(-90.0, 90.0);
    }
    p.on_ground = on_ground;
}

fn chat(g: &mut Game, slot: usize, msg: &str) {
    let p = g.player(slot).unwrap();
    let name = p.name.clone();
    let clean: String = msg.chars().filter(|c| !c.is_control() && *c != '§').collect();
    if clean.is_empty() {
        return;
    }
    if super::plugins::check(g, rine_plugin::SyncKind::Chat, rine_plugin::Event::Chat { name: name.clone(), message: clean.clone() }) {
        return;
    }
    tracing::info!("<{name}> {clean}");
    let t = Text::plain("<").color("gray").push(Text::plain(name).color("white")).push(Text::plain("> ").color("gray")).push(Text::plain(clean).color("white"));
    g.broadcast_msg(&t);
}
