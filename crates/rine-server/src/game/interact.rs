//! Digging, placing, item use, inventory clicks, damage and respawn.

use super::entity;
use super::player::{Dig, Player, Stack};
use super::Game;
use crate::out;
use rine_protocol::text::Text;
use rine_protocol::{BlockPos, ProtoError, Reader};

pub fn player_action(g: &mut Game, slot: usize, action: i32, pos: BlockPos, dir: u8, seq: i32) {
    let tick = g.tick;
    let Some(p) = g.player(slot) else { return };
    match action {
        0 => {
            // START_DESTROY_BLOCK
            if !within_reach(p.eye(), pos, 6.0) {
                p.ack(seq);
                return;
            }
            if p.gamemode == 1 {
                break_block(g, slot, pos, false);
            } else if p.gamemode == 0 {
                let tool = p.inv.hotbar(p.held).item;
                let st = g.world.get_block(pos.x, pos.y, pos.z);
                let progress = super::loot::destroy_progress(st, tool);
                if progress >= 1.0 {
                    break_block(g, slot, pos, true);
                } else {
                    // a new dig replaces an older one: clear the old stage
                    stop_dig(g, slot, -1);
                    // vanilla: destroyBlockProgress(state) right away
                    let stage = ((progress * 10.0) as i32).clamp(0, 9);
                    let eid = g.player(slot).map(|p| p.eid).unwrap_or(0);
                    if let Some(p) = g.player(slot) {
                        p.digging = Some(Dig { pos, start: tick, dir, stage });
                    }
                    send_stage(g, eid, pos, stage);
                }
            }
            if let Some(p) = g.player(slot) {
                p.ack(seq);
            }
        }
        1 => {
            // CHANGE_DESTROY_DIRECTION: the crosshair moved to another face of
            // the block being mined. The dig continues (vanilla only updates
            // destroyDirection here).
            if let Some(d) = p.digging.as_mut() {
                if d.pos == pos {
                    d.dir = dir;
                }
            }
            p.ack(seq);
        }
        2 => {
            // ABORT_DESTROY_BLOCK
            stop_dig(g, slot, -1);
            if let Some(p) = g.player(slot) {
                p.ack(seq);
            }
        }
        3 => {
            // STOP_DESTROY_BLOCK: client finished digging. Accept if the dig
            // started on this block (client-side break time prediction).
            // vanilla only acts on the block being mined (destroyPos)
            let dig = if matches!(p.digging, Some(d) if d.pos == pos) { p.digging.take() } else { None };
            let ok = dig.is_some() && p.gamemode == 0;
            if let Some(d) = dig {
                // vanilla: destroyBlockProgress(-1) before destroyAndAck
                let eid = p.eid;
                if d.stage >= 0 {
                    send_stage(g, eid, d.pos, -1);
                }
            }
            if ok {
                break_block(g, slot, pos, true);
            } else {
                // resync the block
                let st = g.world.get_block(pos.x, pos.y, pos.z);
                let p = g.player(slot).unwrap();
                p.send(out::block_update(pos, st));
            }
            if let Some(p) = g.player(slot) {
                p.ack(seq);
            }
        }
        4 | 5 => {
            // drop all / drop one from hand
            let held = 36 + p.held as usize;
            let s = p.inv.slots[held];
            if s.is_empty() {
                return;
            }
            let n = if action == 4 { s.count } else { 1 };
            p.inv.slots[held].count -= n;
            if p.inv.slots[held].count <= 0 {
                p.inv.slots[held] = Stack::EMPTY;
            }
            let sid = p.inv.state_id;
            let item = p.inv.slots[held];
            p.send(out::container_slot(0, sid, held as i16, &item));
            entity::drop_from_player(g, slot, s.with_count(n), true);
            super::track::equipment_changed(g, slot);
        }
        6 => super::survival::release_use_item(g, slot),
        7 => {
            // swap with offhand
            let held = 36 + p.held as usize;
            p.inv.slots.swap(held, 45);
            let inv = p.inventory_packet();
            p.send(inv);
            super::track::equipment_changed(g, slot);
        }
        _ => {}
    }
}

/// Breaking stage for everyone within 32 blocks except the miner (vanilla
/// `ServerLevel.destroyBlockProgress` skips the breaker's id). `-1` clears it.
fn send_stage(g: &mut Game, eid: i32, pos: BlockPos, stage: i32) {
    let progress = if stage < 0 { 255 } else { stage as u8 };
    let f = out::frame(out::block_destruction(eid, pos, progress));
    let miner = g.by_eid.get(&eid).copied();
    super::track::send_near(g, [pos.x as f64, pos.y as f64, pos.z as f64], 32.0, miner, &f);
}

/// Cancel the player's dig (abort, or a dig replaced by another one). The
/// stage is cleared for viewers when `stage` is `-1`.
fn stop_dig(g: &mut Game, slot: usize, stage: i32) {
    let Some(p) = g.player(slot) else { return };
    let Some(d) = p.digging.take() else { return };
    let eid = p.eid;
    if d.stage >= 0 || stage < 0 {
        send_stage(g, eid, d.pos, stage);
    }
}

/// Per-tick breaking feedback while a survival dig is in progress, as vanilla
/// `ServerPlayerGameMode.tick`: the breaking stage goes to the other players
/// nearby, and every tick a level event with no source goes to everyone within
/// 64 blocks, the miner included. 2020 (every 4th tick) is the mining hit sound
/// the client plays itself; 2021 only shows particles. The miner's client draws
/// its own cracks and predicts the first hit and the final break.
pub fn dig_tick(g: &mut Game) {
    let tick = g.tick;
    for slot in 0..g.players.len() {
        let Some(p) = g.players[slot].as_ref() else { continue };
        let Some(mut d) = p.digging else { continue };
        let eid = p.eid;
        let tool = p.inv.hotbar(p.held).item;
        let st = g.world.get_block(d.pos.x, d.pos.y, d.pos.z);
        if rine_data::get().blocks.is_air(st) {
            // the block went away (explosion, water, another player): end the dig
            if let Some(p) = g.player(slot) {
                p.digging = None;
            }
            send_stage(g, eid, d.pos, -1);
            continue;
        }
        let spent = tick.saturating_sub(d.start);
        let per_tick = super::loot::destroy_progress(st, tool);
        // vanilla: state = (int)(destroyProgress * 10), no clamp below 10
        let stage = (((per_tick as f64) * (spent as f64 + 1.0) * 10.0) as i32).clamp(0, 10);
        if stage != d.stage {
            d.stage = stage;
            send_stage(g, eid, d.pos, stage);
        }
        let event = if spent % 4 == 0 { 2020 } else { 2021 };
        let f = out::frame(out::level_event(event, d.pos, d.dir as i32));
        let at = [d.pos.x as f64, d.pos.y as f64, d.pos.z as f64];
        super::track::send_near(g, at, 64.0, None, &f);
        if let Some(p) = g.player(slot) {
            p.digging = Some(d);
        }
    }
}

fn within_reach(eye: [f64; 3], pos: BlockPos, r: f64) -> bool {
    let dx = pos.x as f64 + 0.5 - eye[0];
    let dy = pos.y as f64 + 0.5 - eye[1];
    let dz = pos.z as f64 + 0.5 - eye[2];
    dx * dx + dy * dy + dz * dz <= (r + 1.0) * (r + 1.0)
}

pub fn break_block(g: &mut Game, slot: usize, pos: BlockPos, drops: bool) {
    let st = g.world.get_block(pos.x, pos.y, pos.z);
    let b = &rine_data::get().blocks;
    if b.is_air(st) || b.state(st).hardness < 0.0 {
        return; // air or unbreakable (bedrock)
    }
    let eid = g.player(slot).map(|p| p.eid).unwrap_or(0);
    let name = g.player(slot).map(|p| p.name.clone()).unwrap_or_default();
    let ev = rine_plugin::Event::BlockBreak { name, x: pos.x, y: pos.y, z: pos.z, block: super::plugins::block_name(st) };
    if super::plugins::check(g, rine_plugin::SyncKind::BlockBreak, ev) {
        if let Some(p) = g.player(slot) {
            p.send(out::block_update(pos, st));
        }
        return;
    }
    // two-block structures (doors, tall plants, beds) break together
    let other = super::interact::other_half(st, pos);
    g.world.set_block(pos, 0);
    super::survival::exhaust(g, slot, super::survival::EXHAUSTION_MINE);
    super::containers::block_removed(g, pos, st);
    if let Some(o) = other {
        if b.block_of(g.world.get_block(o.x, o.y, o.z)).id == b.block_of(st).id {
            g.world.set_block(o, 0);
        }
    }
    // break particles + sound for everyone nearby (level event 2001)
    let f = out::frame(out::level_event(2001, pos, st as i32));
    let cp = rine_world::ChunkPos::new(pos.x >> 4, pos.z >> 4);
    // vanilla: levelEvent(player, 2001, ...) skips the breaker, whose client
    // already played the particles and sound when it predicted the break
    let skip = if eid != 0 { Some(slot) } else { None };
    g.send_chunk_viewers_except(cp, &f, skip);
    if drops {
        let tool = g.player(slot).map(|p| p.inv.hotbar(p.held)).unwrap_or(Stack::EMPTY);
        for d in super::loot::block_drops(st, tool.item) {
            let off = entity::rand_f64((pos.x as u64) ^ ((pos.z as u64) << 20) ^ g.tick);
            entity::spawn_item(
                g,
                [pos.x as f64 + 0.25 + off * 0.5, pos.y as f64 + 0.25, pos.z as f64 + 0.25 + (1.0 - off) * 0.5],
                [(off - 0.5) * 0.2, 0.2, (0.5 - off) * 0.2],
                d,
                10,
            );
        }
    }
    // wear is applied after the drops, which use the tool as it was
    let name = b.block_of(st).name.clone();
    super::durability::on_mine(g, slot, &name, b.state(st).hardness);
    gravity_check(g, pos.offset(0, 1, 0));
}

/// Sand/gravel/concrete powder above a removed block start falling.
pub fn gravity_check(g: &mut Game, pos: BlockPos) {
    let st = g.world.get_block(pos.x, pos.y, pos.z);
    let name = &rine_data::get().blocks.block_of(st).name;
    let falls = matches!(name.as_str(), "minecraft:sand" | "minecraft:red_sand" | "minecraft:gravel" | "minecraft:suspicious_sand" | "minecraft:suspicious_gravel")
        || name.ends_with("_concrete_powder")
        || name.ends_with("anvil");
    if !falls {
        return;
    }
    let below = g.world.get_block(pos.x, pos.y - 1, pos.z);
    let b = &rine_data::get().blocks;
    if b.is_air(below) || b.has(below, rine_data::blocks::flag::REPLACEABLE) {
        g.world.set_block(pos, 0);
        entity::spawn_falling(g, [pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5], st);
        gravity_check(g, pos.offset(0, 1, 0));
    }
}

pub fn other_half(st: u16, pos: BlockPos) -> Option<BlockPos> {
    let b = &rine_data::get().blocks;
    match b.get_prop(st, "half") {
        Some("lower") => Some(pos.offset(0, 1, 0)),
        Some("upper") => Some(pos.offset(0, -1, 0)),
        _ => None,
    }
}

pub fn use_item_on(g: &mut Game, slot: usize, hand: i32, pos: BlockPos, face: u8, cursor: (f32, f32, f32), seq: i32) {
    let Some(p) = g.player(slot) else { return };
    if p.gamemode == 3 || !within_reach(p.eye(), pos, 6.0) {
        p.ack(seq);
        return;
    }
    let inv_slot = if hand == 1 { 45 } else { 36 + p.held as usize };
    let stack = p.inv.slots[inv_slot];
    let (yaw, pitch, sneaking, gm) = (p.yaw, p.pitch, p.sneaking, p.gamemode);
    let clicked = g.world.get_block(pos.x, pos.y, pos.z);
    let b = &rine_data::get().blocks;

    // interacting with blocks (doors, trapdoors, fence gates, TNT + flint)
    if !sneaking || stack.is_empty() {
        if super::cmd::cmdblock::on_use(g, slot, pos) {
            if let Some(p) = g.player(slot) {
                p.ack(seq);
            }
            return;
        }
        if super::containers::open_block(g, slot, pos, clicked) {
            if let Some(p) = g.player(slot) {
                p.ack(seq);
            }
            return;
        }
        if super::place::interact_block(g, slot, pos, clicked, stack) {
            if let Some(p) = g.player(slot) {
                p.ack(seq);
            }
            return;
        }
    }

    let place_state = super::items::places(stack.item);
    if stack.is_empty() || place_state == 0 {
        if let Some(p) = g.player(slot) {
            p.ack(seq);
        }
        return;
    }
    // clicking the open side of a single slab with the same slab makes a double slab
    let slab_merge = b.block_of(clicked).id == b.block_of(place_state).id
        && matches!((b.get_prop(clicked, "type"), face), (Some("bottom"), 1) | (Some("top"), 0));
    let target = if slab_merge || (b.has(clicked, rine_data::blocks::flag::REPLACEABLE) && !b.is_air(clicked)) { pos } else { pos.relative(face) };
    let existing = g.world.get_block(target.x, target.y, target.z);
    if !slab_merge && !(b.is_air(existing) || b.has(existing, rine_data::blocks::flag::REPLACEABLE)) || !(rine_world::MIN_Y..=rine_world::MAX_Y).contains(&target.y) {
        resync(g, slot, target, seq);
        return;
    }
    // don't place blocks inside players
    let st = super::place::placement_state(g, place_state, target, face, cursor, yaw, pitch).map(|st| super::redstone::placement_state(g, target, st));
    let Some(st) = st else {
        resync(g, slot, target, seq);
        return;
    };
    if !b.has(st, rine_data::blocks::flag::NO_COLLISION) && g.players.iter().flatten().any(|o| intersects_player(o.pos, target)) {
        resync(g, slot, target, seq);
        return;
    }
    let pname = g.player(slot).map(|p| p.name.clone()).unwrap_or_default();
    let ev = rine_plugin::Event::BlockPlace { name: pname, x: target.x, y: target.y, z: target.z, block: super::plugins::block_name(st) };
    if super::plugins::check(g, rine_plugin::SyncKind::BlockPlace, ev) {
        resync(g, slot, target, seq);
        return;
    }
    g.world.set_block(target, st);
    super::place::after_place(g, target, st);
    super::redstone::placed(g, target);
    gravity_check(g, target);
    let p = g.player(slot).unwrap();
    if gm != 1 {
        p.inv.slots[inv_slot].count -= 1;
        if p.inv.slots[inv_slot].count <= 0 {
            p.inv.slots[inv_slot] = Stack::EMPTY;
        }
        let sid = p.inv.state_id;
        let it = p.inv.slots[inv_slot];
        p.send(out::container_slot(0, sid, inv_slot as i16, &it));
        super::track::equipment_changed(g, slot);
    }
    if let Some(p) = g.player(slot) {
        p.ack(seq);
    }
}

fn intersects_player(pp: [f64; 3], b: BlockPos) -> bool {
    let (x0, x1) = (pp[0] - 0.3, pp[0] + 0.3);
    let (y0, y1) = (pp[1], pp[1] + 1.8);
    let (z0, z1) = (pp[2] - 0.3, pp[2] + 0.3);
    x1 > b.x as f64 && x0 < b.x as f64 + 1.0 && y1 > b.y as f64 && y0 < b.y as f64 + 1.0 && z1 > b.z as f64 && z0 < b.z as f64 + 1.0
}

fn resync(g: &mut Game, slot: usize, pos: BlockPos, seq: i32) {
    let st = g.world.get_block(pos.x, pos.y, pos.z);
    let Some(p) = g.player(slot) else { return };
    p.send(out::block_update(pos, st));
    let inv = p.inventory_packet();
    p.send(inv);
    p.ack(seq);
}

pub fn use_item(g: &mut Game, slot: usize, _hand: i32, seq: i32, _yaw: f32, _pitch: f32) {
    // eating is client-driven; food is applied on RELEASE/finish in a later
    // milestone. Acknowledge so the client doesn't roll back.
    if let Some(p) = g.player(slot) {
        p.ack(seq);
    }
}

/// Window clicks. Window 0 (player inventory): the client already applied
/// the click locally and tells us the resulting slots; we accept item ids
/// and counts it reports (components are not modelled yet).
pub fn container_click(g: &mut Game, slot: usize, r: &mut Reader) -> Result<(), ProtoError> {
    let window = r.varint()?;
    let _state = r.varint()?;
    let _slot = r.i16()?;
    let _button = r.i8()?;
    let _mode = r.varint()?;
    let n = r.varint()?;
    if !(0..=128).contains(&n) {
        return Err(ProtoError::Invalid("too many slots"));
    }
    let mut changed = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let s = r.i16()?;
        let st = read_hashed(r)?;
        changed.push((s, st));
    }
    let carried = read_hashed(r)?;
    let Some(p) = g.player(slot) else { return Ok(()) };
    if window == 0 {
        for (s, st) in changed {
            if (0..46).contains(&s) {
                p.inv.slots[s as usize] = st;
            }
        }
        p.inv.cursor = carried;
        super::track::equipment_changed(g, slot);
    }
    Ok(())
}

fn read_hashed(r: &mut Reader) -> Result<Stack, ProtoError> {
    if !r.bool()? {
        return Ok(Stack::EMPTY);
    }
    let item = r.varint()? as u32;
    let count = r.varint()?;
    let added = r.varint()?;
    for _ in 0..added.clamp(0, 256) {
        r.varint()?;
        r.i32()?;
    }
    let removed = r.varint()?;
    for _ in 0..removed.clamp(0, 256) {
        r.varint()?;
    }
    Ok(Stack::new(item, count.clamp(0, 99)))
}

pub fn container_close(g: &mut Game, slot: usize) {
    // return crafting grid + cursor items to the inventory
    let Some(p) = g.player(slot) else { return };
    let mut back = Vec::new();
    for i in 1..5 {
        if !p.inv.slots[i].is_empty() {
            back.push(p.inv.slots[i]);
            p.inv.slots[i] = Stack::EMPTY;
        }
    }
    p.inv.slots[0] = Stack::EMPTY;
    if !p.inv.cursor.is_empty() {
        back.push(p.inv.cursor);
        p.inv.cursor = Stack::EMPTY;
    }
    let mut drop = Vec::new();
    for s in back {
        let left = p.inv.add(s);
        if left > 0 {
            drop.push(s.with_count(left));
        }
    }
    let inv = p.inventory_packet();
    p.send(inv);
    for s in drop {
        entity::drop_from_player(g, slot, s, false);
    }
}

/// Attack speed 4 gives the 5-tick strength cycle; weapon attribute modifiers
/// (damage, speed) are not in rine-data, so every item uses bare-hand values.
const ATTACK_DAMAGE: f32 = 1.0;
const ATTACK_DELAY_TICKS: f32 = 5.0;
/// `Player.isSweepAttack`: horizontal speed below 0.1 * 2.5, squared.
const SWEEP_SPEED_SQ: f64 = 0.0625;
/// `LivingEntity.DAMAGE_COOLDOWN_DURATION`.
const INVULN_TICKS: i32 = 20;
const KNOCKBACK_SOUND: &str = "minecraft:entity.player.attack.knockback";
const CRIT_SOUND: &str = "minecraft:entity.player.attack.crit";
const STRONG_SOUND: &str = "minecraft:entity.player.attack.strong";
const WEAK_SOUND: &str = "minecraft:entity.player.attack.weak";
const NODAMAGE_SOUND: &str = "minecraft:entity.player.attack.nodamage";
const SWEEP_SOUND: &str = "minecraft:entity.player.attack.sweep";

/// `Player.attack` for a player target (serverbound `attack`): strength scale,
/// critical hit, sprint knockback, the attack sounds and the victim's hurt.
pub fn attack(g: &mut Game, slot: usize, target: i32) {
    let Some(a) = g.player(slot) else { return };
    if a.dead || a.gamemode == 3 {
        return;
    }
    let (aeid, apos, ayaw, eye) = (a.eid, a.pos, a.yaw, a.eye());
    let (sprinting, on_ground, fall, ticks) = (a.sprinting, a.on_ground, a.fall_distance, a.attack_ticks);
    let vel_sq = a.vel[0] * a.vel[0] + a.vel[2] * a.vel[2];
    let held = a.inv.hotbar(a.held).item;
    let sword = rine_data::get().items.get(held).is_some_and(|i| i.name.ends_with("_sword"));
    let Some(&ts) = g.by_eid.get(&target) else { return };
    if ts == slot {
        return;
    }
    let Some(t) = g.players[ts].as_ref() else { return };
    let d2: f64 = (0..3).map(|i| (t.pos[i] - apos[i]).powi(2)).sum();
    if d2 > 36.0 || t.gamemode == 3 {
        return;
    }
    let teid = t.eid;
    // the strength scale is read before onAttack restarts the cooldown
    let scale = ((ticks as f32 + 0.5) / ATTACK_DELAY_TICKS).clamp(0.0, 1.0);
    let full = scale > 0.9;
    let sprint_hit = sprinting && full;
    let crit = full && !sprinting && fall > 0.0 && !on_ground && super::survival::crit_terrain_ok(g, apos, eye);
    let mut dmg = ATTACK_DAMAGE * (0.2 + scale * scale * 0.8);
    if crit {
        dmg *= 1.5;
    }
    let sweep = full && !crit && !sprint_hit && on_ground && sword && vel_sq < SWEEP_SPEED_SQ;

    if let Some(a) = g.player(slot) {
        a.attack_ticks = 0;
    }
    if sprint_hit {
        attack_sound(g, KNOCKBACK_SOUND, apos);
    }
    if !hurt(g, ts, dmg, "player", Some((aeid, apos))) {
        attack_sound(g, NODAMAGE_SOUND, apos);
        return;
    }
    if sprint_hit {
        // causeExtraKnockback: +0.5 from the sprint, and the attacker's sprint stops
        let yaw = ayaw.to_radians() as f64;
        if let Some(v) = g.players[ts].as_mut() {
            knock(v, 0.5, yaw.sin(), -yaw.cos());
        }
        if let Some(a) = g.player(slot) {
            a.sprinting = false;
        }
        super::track::flags_changed(g, slot);
    }
    // the victim's client applies the velocity itself
    if let Some(v) = g.players[ts].as_mut() {
        let m = v.vel;
        v.send(out::entity_motion(teid, m[0], m[1], m[2]));
    }
    if sweep {
        attack_sound(g, SWEEP_SOUND, apos);
    }
    if crit {
        attack_sound(g, CRIT_SOUND, apos);
        // ServerPlayer.crit: the target's animation goes to the attacker's trackers and to the attacker
        let f = out::frame(out::animate(teid, 1));
        super::track::send_to_trackers(g, aeid, &f);
        if let Some(a) = g.player(slot) {
            a.out.push(f);
        }
    } else if !sweep {
        attack_sound(g, if full { STRONG_SOUND } else { WEAK_SOUND }, apos);
    }
    super::durability::on_attack(g, slot);
    super::survival::exhaust(g, slot, super::survival::EXHAUSTION_ATTACK);
}

/// `Level.playSound(null, ...)` at the attacker: heard by the attacker too (distance 0).
fn attack_sound(g: &mut Game, name: &str, at: [f64; 3]) {
    super::track::sound_near(g, name, super::survival::SOUND_NEUTRAL, at, 1.0, 1.0, None);
}

/// `LivingEntity.knockback`: halves the horizontal velocity and pushes it by
/// `power` along the direction (xd, zd); a grounded entity's rise is capped at 0.4.
pub fn knock(p: &mut Player, power: f64, xd: f64, zd: f64) {
    let len = (xd * xd + zd * zd).sqrt();
    let (ux, uz) = if len < 1e-5 { (1.0, 0.0) } else { (xd / len, zd / len) };
    let on_ground = p.on_ground;
    let v = &mut p.vel;
    v[0] = v[0] / 2.0 - ux * power;
    if on_ground {
        v[1] = (v[1] / 2.0 + power).min(0.4);
    }
    v[2] = v[2] / 2.0 - uz * power;
}

/// Index of a damage type in the synced `damage_type` registry (the order
/// sent during configuration). Short causes map to vanilla type names.
fn damage_type_id(cause: &str) -> i32 {
    let name = match cause {
        "player" => "player_attack",
        "void" => "out_of_world",
        c => c,
    };
    let reg = rine_data::get().registries.synced("minecraft:damage_type");
    reg.id(&format!("minecraft:{name}")).or_else(|| reg.id("minecraft:generic")).unwrap_or(0) as i32
}

/// `LivingEntity.hurtServer` for a player: invulnerability window, the
/// attacker's 0.4 knockback, health, the damage event and death. Returns
/// whether damage was applied: a hit inside the window, a zero hit, and a hit on
/// a creative or spectator player do not count.
pub fn damage(g: &mut Game, slot: usize, amount: f32, cause: &str) -> bool {
    hurt(g, slot, amount, cause, None)
}

fn hurt(g: &mut Game, slot: usize, amount: f32, cause: &str, attacker: Option<(i32, [f64; 3])>) -> bool {
    let Some(p) = g.player(slot) else { return false };
    if p.dead || p.health <= 0.0 || p.gamemode == 1 || p.gamemode == 3 {
        return false;
    }
    let amount = amount.max(0.0);
    let mut dmg = amount;
    // inside the window only a bigger hit lands, and only its excess; the void bypasses it
    if p.hurt_cooldown > INVULN_TICKS / 2 && cause != "void" {
        if amount <= p.last_hurt {
            return false;
        }
        dmg = amount - p.last_hurt;
        p.last_hurt = amount;
    } else {
        p.last_hurt = amount;
        p.hurt_cooldown = INVULN_TICKS;
    }
    if dmg <= 0.0 {
        return false;
    }
    p.health = (p.health - dmg).max(0.0);
    if let Some((_, from)) = attacker {
        // dealDefaultKnockback: away from the attacker's position
        let (xd, zd) = (from[0] - p.pos[0], from[2] - p.pos[2]);
        knock(p, 0.4, xd, zd);
    }
    // armor takes wear unless the damage type bypasses armor (vanilla
    // `bypasses_armor` tag); only the melee and explosion causes are wired
    if matches!(cause, "player" | "explosion") {
        super::durability::hurt_armor(g, slot, dmg);
    }
    super::survival::exhaust(g, slot, super::survival::EXHAUSTION_DAMAGE);
    let Some(p) = g.player(slot) else { return true };
    let (h, f, s, eid) = (p.health, p.food, p.saturation, p.eid);
    p.send(out::set_health(h, f, s));
    // damage_event: the client plays the hurt animation and the hurt sound for
    // this damage type itself (LivingEntity.handleDamageEvent). The attacker ids
    // are its entity id + 1 (cause and direct entity are both the attacker).
    let ty = damage_type_id(cause);
    let (cause_id, direct_id) = attacker.map_or((0, 0), |(e, _)| (e + 1, e + 1));
    let hurt_f = out::frame(rine_protocol::frame::packet(rine_protocol::ids::play::C_DAMAGE_EVENT, |b| {
        use rine_protocol::Writer;
        b.w_varint(eid);
        b.w_varint(ty);
        b.w_varint(cause_id);
        b.w_varint(direct_id);
        b.w_bool(false); // no source position
    }));
    p.out.push(hurt_f.clone());
    super::track::send_to_trackers(g, eid, &hurt_f);
    // the attacker gets it too unless it already tracks the victim
    if let Some((ae, _)) = attacker {
        if let Some(&si) = g.by_eid.get(&ae) {
            if let Some(a) = g.players[si].as_mut() {
                if !a.tracked.contains(&eid) {
                    a.out.push(hurt_f.clone());
                }
            }
        }
    }
    if h <= 0.0 {
        let p = g.player(slot).unwrap();
        p.dead = true;
        let name = p.name.clone();
        let f = out::frame(out::entity_event(eid, 3));
        super::track::send_to_trackers(g, eid, &f);
        g.broadcast_msg(&Text::plain(format!("{name} погиб")).color("red"));
        // drop inventory
        let p = g.player(slot).unwrap();
        let items: Vec<Stack> = p.inv.slots.iter().filter(|s| !s.is_empty()).copied().collect();
        p.inv = Default::default();
        let pos = p.pos;
        let inv = p.inventory_packet();
        p.send(inv);
        for (i, s) in items.into_iter().enumerate() {
            let a = i as f64 * 0.7;
            entity::spawn_item(g, [pos[0], pos[1] + 1.0, pos[2]], [a.sin() * 0.2, 0.3, a.cos() * 0.2], s, 40);
        }
    }
    true
}

pub fn respawn(g: &mut Game, slot: usize) {
    let spawn = g.spawn;
    let dim = g.dim_type_id;
    let seed = g.shared.config.seed;
    let Some(p) = g.player(slot) else { return };
    if !p.dead {
        return;
    }
    p.dead = false;
    p.health = 20.0;
    p.food = 20;
    p.saturation = 5.0;
    let gm = p.gamemode;
    p.send(rine_protocol::frame::packet(rine_protocol::ids::play::C_RESPAWN, |b| {
        use rine_protocol::Writer;
        b.w_varint(dim as i32);
        b.w_str("minecraft:overworld");
        b.w_i64(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15u64 as i64));
        b.w_varint(gm as i32);
        b.w_varint(0);
        b.w_bool(false);
        b.w_bool(false);
        b.w_bool(false);
        b.w_varint(0);
        b.w_varint(63);
        b.w_i8(0);
    }));
    // client forgets everything on respawn: resend chunks and entities
    let old: Vec<_> = p.loaded.drain().collect();
    p.tracked.clear();
    p.unacked = 0;
    p.max_unacked = 1;
    p.teleport(spawn[0], spawn[1], spawn[2], 0.0, 0.0);
    p.send(out::game_event(13, 0.0));
    let c = p.chunk();
    p.center = c;
    p.send(out::chunk_center(c.x, c.z));
    p.send(out::set_health(20.0, 20, 5.0));
    let ab = p.abilities_packet();
    p.send(ab);
    let inv = p.inventory_packet();
    p.send(inv);
    for q in old {
        if let Some(s) = g.world.chunks.get_mut(&q) {
            s.viewers.retain(|v| *v != slot);
        }
    }
}
