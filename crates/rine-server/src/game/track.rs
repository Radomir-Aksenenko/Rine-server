//! Entity tracking and movement broadcast.
//!
//! Every moving entity's update is encoded once per tick into a Frame; the
//! per-viewer pass (parallel over players) only clones refcounted frames.
//! Visibility is recomputed when a viewer changes chunk, and every 10 ticks.

use super::entity;
use super::player::Stack;
use super::Game;
use crate::out;
use ahash::{AHashMap, AHashSet};
use bytes::Bytes;
use rayon::prelude::*;
use rine_world::ChunkPos;

/// Entities (players + others) grouped by chunk for visibility queries.
pub struct Grid {
    pub cells: AHashMap<ChunkPos, Vec<i32>>,
}

pub fn tick(g: &mut Game) {
    // ---- 1. movement frames for players
    let mut moves: AHashMap<i32, Vec<Bytes>> = AHashMap::new();
    for p in g.players.iter_mut().flatten() {
        let moved = p.pos != p.sent_pos;
        let rotated = (p.yaw, p.pitch) != p.sent_rot;
        p.sync_timer += 1;
        if !moved && !rotated && p.sync_timer < 60 {
            continue;
        }
        let mut v = Vec::with_capacity(2);
        let old = (p.sent_pos[0], p.sent_pos[1], p.sent_pos[2]);
        let new = (p.pos[0], p.pos[1], p.pos[2]);
        let pk = if p.sync_timer >= 60 {
            None
        } else if moved {
            out::move_entity(p.eid, old, new, Some((p.yaw, p.pitch)), p.on_ground)
        } else {
            Some(out::rotate_entity(p.eid, p.yaw, p.pitch, p.on_ground))
        };
        let pk = pk.unwrap_or_else(|| {
            p.sync_timer = 0;
            out::position_sync(p.eid, new.0, new.1, new.2, p.yaw, p.pitch, p.on_ground)
        });
        v.push(out::frame(pk));
        if rotated {
            v.push(out::frame(out::head_rot(p.eid, p.yaw)));
        }
        p.sent_pos = p.pos;
        p.sent_rot = (p.yaw, p.pitch);
        moves.insert(p.eid, v);
    }
    // non-player entity moves (encoded by the entity system)
    for (eid, f) in g.entities.moves.drain(..) {
        moves.entry(eid).or_default().push(f);
    }
    let removed: AHashSet<i32> = g.entities.removed.drain(..).collect();
    let extra: Vec<(i32, Bytes)> = std::mem::take(&mut g.entities.extra);
    let extra_map: AHashMap<i32, Vec<Bytes>> = extra.into_iter().fold(AHashMap::new(), |mut m, (e, f)| {
        m.entry(e).or_default().push(f);
        m
    });

    // ---- 2. visibility grid
    let mut cells: AHashMap<ChunkPos, Vec<i32>> = AHashMap::new();
    struct Vis {
        eid: i32,
        is_player: bool,
        range: i32,
    }
    let mut info: AHashMap<i32, (Vis, ChunkPos)> = AHashMap::new();
    for p in g.players.iter().flatten() {
        let c = p.chunk();
        cells.entry(c).or_default().push(p.eid);
        info.insert(p.eid, (Vis { eid: p.eid, is_player: true, range: 32 }, c));
    }
    for e in g.entities.list.iter() {
        if e.removed {
            continue;
        }
        let c = e.chunk();
        cells.entry(c).or_default().push(e.eid);
        info.insert(e.eid, (Vis { eid: e.eid, is_player: false, range: e.kind.track_range() }, c));
    }

    let full = g.tick % 10 == 0;
    let ents = &g.entities;
    let players_ro: AHashMap<i32, usize> = g.by_eid.clone();
    // snapshot spawn data for players (other players need it to spawn them)
    let player_spawn: AHashMap<i32, Vec<Bytes>> = if full || g.players.iter().flatten().any(|p| p.center != p.chunk() || p.joined_tick + 2 > g.tick) {
        g.players.iter().flatten().map(|p| (p.eid, player_spawn_frames(p))).collect()
    } else {
        AHashMap::new()
    };
    let removed_ref = &removed;
    let moves_ref = &moves;
    let extra_ref = &extra_map;
    let tick = g.tick;

    // ---- 3. per-viewer pass in parallel
    g.players.par_iter_mut().flatten().for_each(|p| {
        let recompute = full || p.joined_tick + 2 > tick || !player_spawn.is_empty();
        let mut despawn: Vec<i32> = Vec::new();
        // removed entities
        if !removed_ref.is_empty() {
            p.tracked.retain(|e| {
                if removed_ref.contains(e) {
                    despawn.push(*e);
                    false
                } else {
                    true
                }
            });
        }
        if recompute {
            let c = p.chunk();
            let r = p.view;
            let mut visible: AHashSet<i32> = AHashSet::with_capacity(p.tracked.len() + 16);
            for dx in -r..=r {
                for dz in -r..=r {
                    let q = ChunkPos::new(c.x + dx, c.z + dz);
                    if !p.loaded.contains(&q) {
                        continue;
                    }
                    if let Some(list) = cells.get(&q) {
                        for &e in list {
                            if e == p.eid {
                                continue;
                            }
                            let (vis, _) = &info[&e];
                            if dx.abs() <= vis.range && dz.abs() <= vis.range {
                                visible.insert(vis.eid);
                            }
                        }
                    }
                }
            }
            // despawn no longer visible
            p.tracked.retain(|e| {
                if visible.contains(e) {
                    true
                } else {
                    despawn.push(*e);
                    false
                }
            });
            // spawn newly visible
            for e in visible {
                if p.tracked.contains(&e) {
                    continue;
                }
                let (vis, _) = &info[&e];
                if vis.is_player {
                    if let Some(fs) = player_spawn.get(&e) {
                        p.out.extend(fs.iter().cloned());
                        p.tracked.insert(e);
                    }
                } else if let Some(ent) = ents.get(e) {
                    entity::spawn_frames(ent, &mut p.out);
                    p.tracked.insert(e);
                }
            }
        }
        if !despawn.is_empty() {
            p.out.push(out::frame(out::remove_entities(&despawn)));
        }
        // movement of tracked entities
        if !moves_ref.is_empty() || !extra_ref.is_empty() {
            for e in p.tracked.iter() {
                if let Some(fs) = moves_ref.get(e) {
                    p.out.extend(fs.iter().cloned());
                }
                if let Some(fs) = extra_ref.get(e) {
                    p.out.extend(fs.iter().cloned());
                }
            }
        }
    });
    let _ = players_ro;
}

/// Frames that spawn player `p` on another client.
pub fn player_spawn_frames(p: &super::player::Player) -> Vec<Bytes> {
    let ty = rine_data::get().items.entity("player");
    let mut v = Vec::with_capacity(4);
    v.push(out::frame(out::add_entity(p.eid, p.uuid, ty, p.pos[0], p.pos[1], p.pos[2], (0.0, 0.0, 0.0), p.pitch, p.yaw, p.yaw, 0)));
    v.push(out::frame(meta_for(p)));
    v.push(out::frame(out::head_rot(p.eid, p.yaw)));
    v.push(out::frame(equipment_packet(p)));
    v
}

fn shared_flags(p: &super::player::Player) -> u8 {
    let mut f = 0u8;
    if p.sneaking {
        f |= 0x02;
    }
    if p.sprinting {
        f |= 0x08;
    }
    f
}

fn meta_for(p: &super::player::Player) -> bytes::BytesMut {
    out::Meta::new(p.eid)
        .byte(0, shared_flags(p))
        .pose(6, if p.sneaking { 5 } else { 0 })
        .float(9, p.health.max(0.0))
        .humanoid_arm(15, p.info.main_hand.clamp(0, 1))
        .byte(16, p.info.skin_parts)
        .finish()
}

fn equipment_packet(p: &super::player::Player) -> bytes::BytesMut {
    use rine_protocol::Writer;
    // set_equipment: VarInt eid, then entries (u8 slot | 0x80 if more follow, ItemStack)
    let main = p.inv.hotbar(p.held);
    let off = p.inv.slots[45];
    let armor = [p.inv.slots[8], p.inv.slots[7], p.inv.slots[6], p.inv.slots[5]];
    rine_protocol::frame::packet(rine_protocol::ids::play::C_SET_EQUIPMENT, |b| {
        b.w_varint(p.eid);
        // slots: 0 mainhand, 1 offhand, 2 feet, 3 legs, 4 chest, 5 head
        let entries: [(u8, Stack); 6] = [(0, main), (1, off), (2, armor[0]), (3, armor[1]), (4, armor[2]), (5, armor[3])];
        for (i, (slot, it)) in entries.iter().enumerate() {
            let more = if i + 1 < entries.len() { 0x80 } else { 0 };
            b.w_u8(*slot | more);
            out::write_stack(b, it);
        }
    })
}

/// Send a frame to every player that tracks entity `eid`.
pub fn send_to_trackers(g: &mut Game, eid: i32, f: &Bytes) {
    for p in g.players.iter_mut().flatten() {
        if p.tracked.contains(&eid) {
            p.out.push(f.clone());
        }
    }
}

pub fn equipment_changed(g: &mut Game, slot: usize) {
    let Some(p) = g.player(slot) else { return };
    let eid = p.eid;
    let f = out::frame(equipment_packet(p));
    send_to_trackers(g, eid, &f);
}

pub fn flags_changed(g: &mut Game, slot: usize) {
    let Some(p) = g.player(slot) else { return };
    let eid = p.eid;
    let f = out::frame(meta_for(p));
    send_to_trackers(g, eid, &f);
}
