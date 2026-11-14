//! Entity tracking and movement broadcast.
//!
//! A client only knows a bounded set of entities (`Player::tracked`): at most
//! `MAX_TRACKED`, nearest chunks first, and at most `SPAWN_PER_TICK` spawns per
//! tick. Entities outside that set are still simulated, just never sent.
//! Moves and metadata of tracked entities are skipped while a client's
//! outbound backlog is over `BACKLOG_SOFT`: they are low priority and the next
//! sync catches the client up. Keepalive and the player's own position are
//! sent elsewhere and always go out.
//!
//! Visibility is recomputed when a viewer changes chunk and on every 10th
//! tick. The entity grid (`Entities::grid`) is rebuilt on those 10th ticks;
//! between rebuilds each candidate is checked at its current position.

use super::entity;
use super::player::{Player, Stack};
use super::Game;
use crate::out;
use ahash::{AHashMap, AHashSet};
use bytes::Bytes;
use rayon::prelude::*;
use rine_world::ChunkPos;
use std::sync::atomic::Ordering;

/// Entities a client may track at once. Nearest chunks are chosen first.
pub const MAX_TRACKED: usize = 2000;
/// Entity spawns (each one or two packets) sent to one client per tick.
pub const SPAWN_PER_TICK: u32 = 200;
/// Entities checked per between-tick spawn pass; the rest of a bulk spawn
/// (`/tnt`) waits for the periodic pass.
const FRESH_SCAN: usize = 256;
/// Outbound bytes not yet written above which low-priority updates are dropped.
pub const BACKLOG_SOFT: usize = 1 << 20;
/// Player visibility range in chunks (vanilla default tracking).
const PLAYER_RANGE: i32 = 32;

fn lagging(p: &Player) -> bool {
    p.backlog.load(Ordering::Relaxed) > BACKLOG_SOFT
}

/// Chunk offsets within `r`, nearest (Chebyshev distance) first.
fn offsets(r: i32) -> Vec<(i32, i32)> {
    let mut v: Vec<(i32, i32)> = (-r..=r).flat_map(|dx| (-r..=r).map(move |dz| (dx, dz))).collect();
    v.sort_by_key(|(dx, dz)| dx.abs().max(dz.abs()));
    v
}

/// True if chunk `q` is within the viewer's view `view` and the entity's `range`
/// around chunk `c`.
fn in_view(q: ChunkPos, c: ChunkPos, view: i32, range: i32) -> bool {
    let (dx, dz) = ((q.x - c.x).abs(), (q.z - c.z).abs());
    dx <= view && dz <= view && dx <= range && dz <= range
}

/// Spawn packets for entities created since the last tracker pass, sent to
/// every player that can see them. Runs between ticks too, so a dropped item
/// appears when the drop packet is handled instead of up to ten ticks later.
/// Bounded: at most `FRESH_SCAN` entities per call.
pub fn spawn_fresh(g: &mut Game) {
    if g.entities.fresh.is_empty() {
        return;
    }
    let mut fresh = std::mem::take(&mut g.entities.fresh);
    fresh.truncate(FRESH_SCAN);
    let Game { entities, players, .. } = g;
    for eid in fresh {
        let Some(&i) = entities.index.get(&eid) else { continue };
        let Some(e) = entities.list.get(i) else { continue };
        if e.removed || e.eid != eid {
            continue;
        }
        let c = e.chunk();
        let range = e.kind.track_range();
        for p in players.iter_mut().flatten() {
            if p.tracked.contains(&eid) || p.tracked.len() >= MAX_TRACKED || p.spawned_tick >= SPAWN_PER_TICK || lagging(p) {
                continue;
            }
            if !in_view(c, p.chunk(), p.view, range) || !p.loaded.contains(&c) {
                continue;
            }
            entity::spawn_frames(e, &mut p.out);
            p.tracked.insert(eid);
            p.spawned_tick += 1;
        }
    }
}

pub fn tick(g: &mut Game) {
    for p in g.players.iter_mut().flatten() {
        p.spawned_tick = 0;
    }
    spawn_fresh(g);
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

    // ---- 2. spatial grid of entities, rebuilt on the periodic pass
    let full = g.tick % 10 == 0;
    {
        let ents = &mut g.entities;
        if full || ents.grid.is_empty() {
            ents.grid.clear();
            for e in ents.list.iter().filter(|e| !e.removed) {
                ents.grid.entry(e.chunk()).or_default().push(e.eid);
            }
        }
    }
    // players are few: index them per tick
    let mut player_chunk: AHashMap<i32, ChunkPos> = AHashMap::new();
    for p in g.players.iter().flatten() {
        player_chunk.insert(p.eid, p.chunk());
    }
    let any_recompute = full || g.players.iter().flatten().any(|p| p.vis_chunk != Some(p.chunk()));
    // spawn data for every player (other players need it to appear)
    let player_spawn: AHashMap<i32, Vec<Bytes>> = if any_recompute { g.players.iter().flatten().map(|p| (p.eid, player_spawn_frames(p))).collect() } else { AHashMap::new() };
    let removed_ref = &removed;
    let moves_ref = &moves;
    let extra_ref = &extra_map;
    let ents = &g.entities;
    let grid = &ents.grid;

    // ---- 3. per-viewer pass in parallel
    g.players.par_iter_mut().flatten().for_each(|p| {
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
        let c = p.chunk();
        let view = p.view;
        if full || p.vis_chunk != Some(c) {
            p.vis_chunk = Some(c);
            let loaded = &p.loaded;
            // despawn whatever left the view (checked at current positions)
            p.tracked.retain(|&e| {
                let at = match player_chunk.get(&e) {
                    Some(&pc) => Some((pc, PLAYER_RANGE)),
                    None => ents.index.get(&e).map(|&i| {
                        let en = &ents.list[i];
                        (en.chunk(), en.kind.track_range())
                    }),
                };
                let keep = matches!(at, Some((q, r)) if in_view(q, c, view, r) && loaded.contains(&q));
                if !keep {
                    despawn.push(e);
                }
                keep
            });
            // players first: they are few and never count against the entity cap
            for (&e, &pc) in player_chunk.iter() {
                if e == p.eid || p.tracked.contains(&e) || !in_view(pc, c, view, PLAYER_RANGE) || !p.loaded.contains(&pc) {
                    continue;
                }
                if let Some(fs) = player_spawn.get(&e) {
                    p.out.extend(fs.iter().cloned());
                    p.tracked.insert(e);
                }
            }
            // entities: budgeted, nearest chunks first
            if !lagging(p) {
                let entities_tracked = p.tracked.iter().filter(|e| !player_chunk.contains_key(e)).count();
                let mut room = MAX_TRACKED.saturating_sub(entities_tracked).min(SPAWN_PER_TICK.saturating_sub(p.spawned_tick) as usize);
                'scan: for (dx, dz) in offsets(view) {
                    if room == 0 {
                        break;
                    }
                    let q = ChunkPos::new(c.x + dx, c.z + dz);
                    if !p.loaded.contains(&q) {
                        continue;
                    }
                    if let Some(list) = grid.get(&q) {
                        for &e in list {
                            if room == 0 {
                                break 'scan;
                            }
                            if p.tracked.contains(&e) {
                                continue;
                            }
                            let Some(&i) = ents.index.get(&e) else { continue };
                            let en = &ents.list[i];
                            if en.removed {
                                continue;
                            }
                            let ca = en.chunk();
                            if !in_view(ca, c, view, en.kind.track_range()) || !p.loaded.contains(&ca) {
                                continue;
                            }
                            entity::spawn_frames(en, &mut p.out);
                            p.tracked.insert(e);
                            p.spawned_tick += 1;
                            room -= 1;
                        }
                    }
                }
            }
        }
        if !despawn.is_empty() {
            p.out.push(out::frame(out::remove_entities(&despawn)));
        }
        // movement of tracked entities (skipped while the client is backlogged)
        if !lagging(p) && (!moves_ref.is_empty() || !extra_ref.is_empty()) {
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
        .pose(6, if p.sneaking && !p.flying { 5 } else { 0 })
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

/// Frame to every player within `range` blocks of `at` (vanilla
/// `PlayerList.broadcast`: squared distance below range², `except` skipped).
pub fn send_near(g: &mut Game, at: [f64; 3], range: f64, except: Option<usize>, f: &Bytes) {
    let r2 = range * range;
    for p in g.players.iter_mut().flatten() {
        if Some(p.slot) == except {
            continue;
        }
        let d2: f64 = (0..3).map(|i| (p.pos[i] - at[i]).powi(2)).sum();
        if d2 < r2 {
            p.out.push(f.clone());
        }
    }
}

/// Vanilla `Level.playSound(null, ...)`: heard by everyone within 16 blocks
/// (`16 * volume` for loud sounds), the source player included.
pub fn sound_near(g: &mut Game, name: &str, category: i32, at: [f64; 3], volume: f32, pitch: f32, except: Option<usize>) {
    let Some(id) = rine_data::get().registries.id("minecraft:sound_event", name) else { return };
    let f = out::frame(out::sound(id, category, at[0], at[1], at[2], volume, pitch, g.tick as i64));
    let range = if volume > 1.0 { 16.0 * volume as f64 } else { 16.0 };
    send_near(g, at, range, except, &f);
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
