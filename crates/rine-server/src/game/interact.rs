//! Digging, placing, item use, inventory clicks, damage and respawn.

use super::entity;
use super::player::Stack;
use super::Game;
use crate::out;
use rine_protocol::text::Text;
use rine_protocol::{BlockPos, ProtoError, Reader};

pub fn player_action(g: &mut Game, slot: usize, action: i32, pos: BlockPos, _dir: u8, seq: i32) {
    let tick = g.tick;
    let Some(p) = g.player(slot) else { return };
    match action {
        0 => {
            // START_DESTROY_BLOCK
            if !within_reach(p.eye(), pos, 6.0) {
                p.send(out::block_ack(seq));
                return;
            }
            if p.gamemode == 1 {
                break_block(g, slot, pos, false);
            } else if p.gamemode == 0 {
                let st = g.world.get_block(pos.x, pos.y, pos.z);
                let hard = rine_data::get().blocks.state(st).hardness;
                let p = g.player(slot).unwrap();
                if hard == 0.0 {
                    break_block(g, slot, pos, true);
                } else {
                    p.digging = Some((pos, tick));
                }
            }
            if let Some(p) = g.player(slot) {
                p.send(out::block_ack(seq));
            }
        }
        1 | 2 => {
            p.digging = None;
            p.send(out::block_ack(seq));
        }
        3 => {
            // STOP_DESTROY_BLOCK: client finished digging. Accept if the dig
            // started on this block (client-side break time prediction).
            let ok = matches!(p.digging, Some((dp, _)) if dp == pos) && p.gamemode == 0;
            p.digging = None;
            if ok {
                break_block(g, slot, pos, true);
            } else {
                // resync the block
                let st = g.world.get_block(pos.x, pos.y, pos.z);
                let p = g.player(slot).unwrap();
                p.send(out::block_update(pos, st));
            }
            if let Some(p) = g.player(slot) {
                p.send(out::block_ack(seq));
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
            let item = p.inv.slots[held].tuple();
            p.send(out::container_slot(0, sid, held as i16, item));
            entity::drop_from_player(g, slot, Stack::new(s.item, n), true);
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
    // the breaker already predicted the particles, vanilla still sends it to all
    g.send_chunk_viewers(cp, &f);
    let _ = eid;
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
        p.send(out::block_ack(seq));
        return;
    }
    let inv_slot = if hand == 1 { 45 } else { 36 + p.held as usize };
    let stack = p.inv.slots[inv_slot];
    let (yaw, pitch, sneaking, gm) = (p.yaw, p.pitch, p.sneaking, p.gamemode);
    let clicked = g.world.get_block(pos.x, pos.y, pos.z);
    let b = &rine_data::get().blocks;

    // interacting with blocks (doors, trapdoors, fence gates, TNT + flint)
    if !sneaking || stack.is_empty() {
        if super::containers::open_block(g, slot, pos, clicked) {
            if let Some(p) = g.player(slot) {
                p.send(out::block_ack(seq));
            }
            return;
        }
        if super::place::interact_block(g, slot, pos, clicked, stack) {
            if let Some(p) = g.player(slot) {
                p.send(out::block_ack(seq));
            }
            return;
        }
    }

    let place_state = super::items::places(stack.item);
    if stack.is_empty() || place_state == 0 {
        if let Some(p) = g.player(slot) {
            p.send(out::block_ack(seq));
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
    let st = super::place::placement_state(g, place_state, target, face, cursor, yaw, pitch);
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
    gravity_check(g, target);
    let p = g.player(slot).unwrap();
    if gm != 1 {
        p.inv.slots[inv_slot].count -= 1;
        if p.inv.slots[inv_slot].count <= 0 {
            p.inv.slots[inv_slot] = Stack::EMPTY;
        }
        let sid = p.inv.state_id;
        let it = p.inv.slots[inv_slot].tuple();
        p.send(out::container_slot(0, sid, inv_slot as i16, it));
        super::track::equipment_changed(g, slot);
    }
    if let Some(p) = g.player(slot) {
        p.send(out::block_ack(seq));
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
    p.send(out::block_ack(seq));
}

pub fn use_item(g: &mut Game, slot: usize, _hand: i32, seq: i32, _yaw: f32, _pitch: f32) {
    // eating is client-driven; food is applied on RELEASE/finish in a later
    // milestone. Acknowledge so the client doesn't roll back.
    if let Some(p) = g.player(slot) {
        p.send(out::block_ack(seq));
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
        let left = p.inv.add(s.item, s.count);
        if left > 0 {
            drop.push(Stack::new(s.item, left));
        }
    }
    let inv = p.inventory_packet();
    p.send(inv);
    for s in drop {
        entity::drop_from_player(g, slot, s, false);
    }
}

pub fn attack(g: &mut Game, slot: usize, target: i32) {
    let Some(attacker) = g.player(slot) else { return };
    let (aeid, apos, ayaw) = (attacker.eid, attacker.pos, attacker.yaw);
    if let Some(&ts) = g.by_eid.get(&target) {
        if ts == slot {
            return;
        }
        let t = g.players[ts].as_ref().unwrap();
        let d2: f64 = (0..3).map(|i| (t.pos[i] - apos[i]).powi(2)).sum();
        if d2 > 36.0 || t.gamemode == 1 || t.gamemode == 3 {
            return;
        }
        super::survival::exhaust(g, slot, super::survival::EXHAUSTION_ATTACK);
        damage(g, ts, 1.0, "player");
        // knockback
        let (kx, kz) = (-(ayaw.to_radians() as f64).sin() * 0.4, (ayaw.to_radians() as f64).cos() * 0.4);
        let t = g.players[ts].as_mut().unwrap();
        let teid = t.eid;
        t.send(out::entity_motion(teid, kx, 0.36, kz));
        let _ = aeid;
    }
}

pub fn damage(g: &mut Game, slot: usize, amount: f32, _cause: &str) {
    let Some(p) = g.player(slot) else { return };
    if p.dead || p.gamemode == 1 || p.gamemode == 3 {
        return;
    }
    p.health = (p.health - amount).max(0.0);
    let _ = p;
    super::survival::exhaust(g, slot, super::survival::EXHAUSTION_DAMAGE);
    let Some(p) = g.player(slot) else { return };
    let (h, f, s, eid) = (p.health, p.food, p.saturation, p.eid);
    p.send(out::set_health(h, f, s));
    let hurt = out::frame(rine_protocol::frame::packet(rine_protocol::ids::play::C_HURT_ANIMATION, |b| {
        use rine_protocol::Writer;
        b.w_varint(eid);
        b.w_f32(0.0);
    }));
    p.out.push(hurt.clone());
    super::track::send_to_trackers(g, eid, &hurt);
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
