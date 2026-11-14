//! Non-player entities: dropped items, primed TNT, falling blocks.
//!
//! Physics runs data-parallel (rayon) against an immutable world view;
//! side effects (explosions, pickups, block placement) are collected and
//! applied afterwards on the game thread. Network updates are encoded once
//! per entity per tick and fanned out by the tracker.

use super::player::{max_stack, Stack};
use super::Game;
use crate::out;
use ahash::AHashMap;
use bytes::Bytes;
use rayon::prelude::*;
use rine_protocol::BlockPos;
use rine_world::ChunkPos;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub enum Kind {
    Item { stack: Stack, pickup_delay: i32 },
    Tnt { fuse: i32 },
    Falling { state: u16 },
}

impl Kind {
    /// Tracking range in chunks.
    pub fn track_range(&self) -> i32 {
        match self {
            Kind::Item { .. } => 6,
            Kind::Tnt { .. } => 10,
            Kind::Falling { .. } => 10,
        }
    }
    fn size(&self) -> (f64, f64) {
        match self {
            Kind::Item { .. } => (0.25, 0.25),
            _ => (0.98, 0.98),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Ent {
    pub eid: i32,
    pub uuid: Uuid,
    pub kind: Kind,
    pub pos: [f64; 3],
    pub vel: [f64; 3],
    pub on_ground: bool,
    pub age: i32,
    pub removed: bool,
    pub sent_pos: [f64; 3],
    pub sent_vel: [f64; 3],
}

impl Ent {
    pub fn chunk(&self) -> ChunkPos {
        ChunkPos::new((self.pos[0].floor() as i32) >> 4, (self.pos[2].floor() as i32) >> 4)
    }
    /// Approximate visual center (used for explosion exposure and knockback).
    pub fn mid(&self) -> [f64; 3] {
        let h = match &self.kind {
            Kind::Item { .. } => 0.125,
            _ => 0.49,
        };
        [self.pos[0], self.pos[1] + h, self.pos[2]]
    }
}

#[derive(Default)]
pub struct Entities {
    pub list: Vec<Ent>,
    pub index: AHashMap<i32, usize>,
    /// (eid, frame) movement updates produced this tick.
    pub moves: Vec<(i32, Bytes)>,
    /// Entities removed this tick (despawned by the tracker).
    pub removed: Vec<i32>,
    /// Extra per-entity frames (metadata changes, pickups...).
    pub extra: Vec<(i32, Bytes)>,
}

impl Entities {
    pub fn get(&self, eid: i32) -> Option<&Ent> {
        self.index.get(&eid).map(|&i| &self.list[i])
    }
    pub fn spawn(&mut self, e: Ent) {
        self.index.insert(e.eid, self.list.len());
        self.list.push(e);
    }
    fn compact(&mut self) {
        if self.list.iter().any(|e| e.removed) {
            for e in self.list.iter().filter(|e| e.removed) {
                self.removed.push(e.eid);
            }
            self.list.retain(|e| !e.removed);
            self.index.clear();
            for (i, e) in self.list.iter().enumerate() {
                self.index.insert(e.eid, i);
            }
        }
    }
}

pub fn spawn_frames(e: &Ent, outv: &mut Vec<Bytes>) {
    let items = &rine_data::get().items;
    match &e.kind {
        Kind::Item { stack, .. } => {
            let ty = items.entity("item");
            outv.push(out::frame(out::add_entity(e.eid, e.uuid, ty, e.pos[0], e.pos[1], e.pos[2], (e.vel[0], e.vel[1], e.vel[2]), 0.0, 0.0, 0.0, 0)));
            outv.push(out::frame(out::Meta::new(e.eid).item(8, stack.item, stack.count).finish()));
        }
        Kind::Tnt { fuse } => {
            let ty = items.entity("tnt");
            outv.push(out::frame(out::add_entity(e.eid, e.uuid, ty, e.pos[0], e.pos[1], e.pos[2], (e.vel[0], e.vel[1], e.vel[2]), 0.0, 0.0, 0.0, 0)));
            let tnt = rine_data::get().blocks.default_state("tnt");
            outv.push(out::frame(out::Meta::new(e.eid).varint(8, *fuse).block_state(9, tnt).finish()));
        }
        Kind::Falling { state } => {
            let ty = items.entity("falling_block");
            outv.push(out::frame(out::add_entity(e.eid, e.uuid, ty, e.pos[0], e.pos[1], e.pos[2], (e.vel[0], e.vel[1], e.vel[2]), 0.0, 0.0, 0.0, *state as i32)));
        }
    }
}

pub fn spawn_item(g: &mut Game, pos: [f64; 3], vel: [f64; 3], stack: Stack, pickup_delay: i32) {
    if stack.is_empty() {
        return;
    }
    let eid = g.alloc_eid();
    g.entities.spawn(Ent { eid, uuid: Uuid::new_v4(), kind: Kind::Item { stack, pickup_delay }, pos, vel, on_ground: false, age: 0, removed: false, sent_pos: pos, sent_vel: vel });
}

pub fn spawn_tnt(g: &mut Game, pos: [f64; 3], fuse: i32) {
    let eid = g.alloc_eid();
    let a = rand_f64(eid as u64) * std::f64::consts::TAU;
    let vel = [-a.sin() * 0.02, 0.2, -a.cos() * 0.02];
    g.entities.spawn(Ent { eid, uuid: Uuid::new_v4(), kind: Kind::Tnt { fuse }, pos, vel, on_ground: false, age: 0, removed: false, sent_pos: pos, sent_vel: vel });
}

pub fn spawn_falling(g: &mut Game, pos: [f64; 3], state: u16) {
    let eid = g.alloc_eid();
    g.entities.spawn(Ent { eid, uuid: Uuid::new_v4(), kind: Kind::Falling { state }, pos, vel: [0.0; 3], on_ground: false, age: 0, removed: false, sent_pos: pos, sent_vel: [0.0; 3] });
}

/// Drop an item from a player's view direction (Q key / creative drop).
pub fn drop_from_player(g: &mut Game, slot: usize, stack: Stack, thrown: bool) {
    let Some(p) = g.player(slot) else { return };
    let eye = p.eye();
    let (yaw, pitch) = (p.yaw.to_radians() as f64, p.pitch.to_radians() as f64);
    let sp = if thrown { 0.3 } else { 0.1 };
    let vel = [-yaw.sin() * pitch.cos() * sp, -pitch.sin() * sp + 0.1, yaw.cos() * pitch.cos() * sp];
    spawn_item(g, [eye[0], eye[1] - 0.3, eye[2]], vel, stack, 40);
}

pub fn rand_f64(seed: u64) -> f64 {
    let mut z = seed.wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

/// Is the block at (x,y,z) solid for entity collision?
fn solid_at(g_world: &super::world::World, x: i32, y: i32, z: i32) -> bool {
    let st = g_world.get_block(x, y, z);
    let b = &rine_data::get().blocks;
    !b.has(st, rine_data::blocks::flag::NO_COLLISION) && !b.is_air(st)
}

/// Simple AABB-vs-voxel movement: axis by axis, full-block collision.
fn move_ent(w: &super::world::World, e: &mut Ent) {
    let (hw, h) = e.kind.size();
    let hw = hw / 2.0;
    for axis in [1usize, 0, 2] {
        let d = e.vel[axis];
        if d == 0.0 {
            continue;
        }
        let mut np = e.pos;
        np[axis] += d;
        let min = [np[0] - hw, np[1], np[2] - hw];
        let max = [np[0] + hw, np[1] + h, np[2] + hw];
        let mut hit = false;
        'outer: for x in min[0].floor() as i32..=(max[0] - 1e-7).floor() as i32 {
            for y in min[1].floor() as i32..=(max[1] - 1e-7).floor() as i32 {
                for z in min[2].floor() as i32..=(max[2] - 1e-7).floor() as i32 {
                    if solid_at(w, x, y, z) {
                        hit = true;
                        break 'outer;
                    }
                }
            }
        }
        if hit {
            if axis == 1 {
                if d < 0.0 {
                    e.on_ground = true;
                    e.pos[1] = (e.pos[1] + d).floor() + 1.0;
                    if e.pos[1] > np[1] + 1.0 {
                        e.pos[1] = np[1].ceil();
                    }
                }
                e.vel[1] = 0.0;
            } else {
                e.vel[axis] = 0.0;
            }
        } else {
            e.pos[axis] = np[axis];
            if axis == 1 {
                e.on_ground = false;
            }
        }
    }
}

pub fn tick(g: &mut Game) {
    // ---- parallel physics
    let world = &g.world;
    let sim_r = g.shared.config.simulation_distance as i32 + 2;
    let player_chunks: Vec<ChunkPos> = g.players.iter().flatten().map(|p| p.chunk()).collect();
    let near = |c: ChunkPos| player_chunks.iter().any(|p| (p.x - c.x).abs() <= sim_r && (p.z - c.z).abs() <= sim_r);
    #[derive(Default)]
    struct Fx {
        explode: Vec<[f64; 3]>,
        land: Vec<(BlockPos, u16, i32)>,
    }
    let fx: Fx = g
        .entities
        .list
        .par_iter_mut()
        .fold(Fx::default, |mut fx, e| {
            if e.removed {
                return fx;
            }
            let c = e.chunk();
            if !world.is_loaded(e.pos[0].floor() as i32, e.pos[2].floor() as i32) || !near(c) {
                return fx; // frozen outside simulation distance
            }
            e.age += 1;
            if let Kind::Item { pickup_delay, .. } = &mut e.kind {
                if *pickup_delay > 0 {
                    *pickup_delay -= 1;
                }
            }
            // All kinds gravity + collide first; the kind-specific tail runs after
            // (kept out of the match so `move_ent(e)` never overlaps a kind borrow).
            e.vel[1] -= 0.04;
            move_ent(world, e);
            match &mut e.kind {
                Kind::Item { .. } => {
                    let fr = if e.on_ground { 0.6 * 0.98 } else { 0.98 };
                    e.vel[0] *= fr;
                    e.vel[1] *= 0.98;
                    e.vel[2] *= fr;
                    if e.on_ground {
                        e.vel[1] *= -0.5;
                    }
                    if e.age > 6000 {
                        e.removed = true;
                    }
                }
                Kind::Tnt { fuse } => {
                    e.vel[0] *= 0.98;
                    e.vel[1] *= 0.98;
                    e.vel[2] *= 0.98;
                    if e.on_ground {
                        e.vel[0] *= 0.7;
                        e.vel[2] *= 0.7;
                        e.vel[1] *= -0.5;
                    }
                    *fuse -= 1;
                    if *fuse <= 0 {
                        e.removed = true;
                        fx.explode.push([e.pos[0], e.pos[1] + 0.06125, e.pos[2]]);
                    }
                }
                Kind::Falling { state } => {
                    e.vel[1] *= 0.98;
                    if e.on_ground || e.pos[1] < rine_world::MIN_Y as f64 - 64.0 {
                        e.removed = true;
                        let bp = BlockPos::new(e.pos[0].floor() as i32, e.pos[1].round() as i32, e.pos[2].floor() as i32);
                        fx.land.push((bp, *state, e.eid));
                    }
                }
            }
            fx
        })
        .reduce(Fx::default, |mut a, b| {
            a.explode.extend(b.explode);
            a.land.extend(b.land);
            a
        });

    // ---- item merging (every 10 ticks) and pickup, both via one item index
    let idx = ItemIndex::build(&g.entities.list);
    if g.tick % 10 == 0 {
        merge_items(&mut g.entities, &idx);
    }
    pickup(g, &idx);

    // ---- falling blocks land
    for (bp, st, _) in fx.land {
        let cur = g.world.get_block(bp.x, bp.y, bp.z);
        let b = &rine_data::get().blocks;
        if b.is_air(cur) || b.has(cur, rine_data::blocks::flag::REPLACEABLE) {
            g.world.set_block(bp, st);
        } else {
            let item = b.state(st).item;
            spawn_item(g, [bp.x as f64 + 0.5, bp.y as f64 + 0.5, bp.z as f64 + 0.5], [0.0, 0.1, 0.0], Stack::new(item, 1), 10);
        }
    }

    // ---- explosions (batched, parallel)
    if !fx.explode.is_empty() {
        super::explosion::explode_batch(g, fx.explode, 4.0);
    }

    // ---- movement packets: one encode per moved entity
    let moves: Vec<(i32, Bytes)> = g
        .entities
        .list
        .par_iter_mut()
        .filter_map(|e| {
            if e.removed {
                return None;
            }
            let moved = e.pos != e.sent_pos;
            let vel_changed = e.vel != e.sent_vel;
            if !moved && !vel_changed {
                return None;
            }
            let mut buf = bytes::BytesMut::new();
            let old = (e.sent_pos[0], e.sent_pos[1], e.sent_pos[2]);
            let new = (e.pos[0], e.pos[1], e.pos[2]);
            let mv = if e.age % 60 == 0 { None } else { out::move_entity(e.eid, old, new, None, e.on_ground) };
            let mv = mv.unwrap_or_else(|| out::position_sync(e.eid, new.0, new.1, new.2, 0.0, 0.0, e.on_ground));
            buf.extend_from_slice(&out::frame(mv));
            if vel_changed && matches!(e.kind, Kind::Tnt { .. } | Kind::Falling { .. }) && e.age % 4 == 0 {
                buf.extend_from_slice(&out::frame(out::entity_motion(e.eid, e.vel[0], e.vel[1], e.vel[2])));
                e.sent_vel = e.vel;
            }
            e.sent_pos = e.pos;
            Some((e.eid, buf.freeze()))
        })
        .collect();
    g.entities.moves.extend(moves);
    g.entities.compact();
}

/// Item entities bucketed by 2x2-block cells. Built once per tick; sorted
/// `(cell, list index)` pairs, so lookups are binary searches and no per-cell
/// allocation happens. Entries may point at removed entities: callers check.
#[derive(Default)]
struct ItemIndex {
    cells: Vec<([i32; 3], usize)>,
}

fn item_cell(p: [f64; 3]) -> [i32; 3] {
    [(p[0] / 2.0).floor() as i32, (p[1] / 2.0).floor() as i32, (p[2] / 2.0).floor() as i32]
}

impl ItemIndex {
    fn build(list: &[Ent]) -> Self {
        let mut cells: Vec<([i32; 3], usize)> = list
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.removed && matches!(e.kind, Kind::Item { .. }))
            .map(|(i, e)| (item_cell(e.pos), i))
            .collect();
        cells.sort_unstable();
        Self { cells }
    }

    /// Calls `f` with the list index of every item in cells `lo..=hi`.
    fn range(&self, lo: [i32; 3], hi: [i32; 3], mut f: impl FnMut(usize)) {
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    let key = [x, y, z];
                    let start = self.cells.partition_point(|(k, _)| *k < key);
                    for (k, i) in &self.cells[start..] {
                        if *k != key {
                            break;
                        }
                        f(*i);
                    }
                }
            }
        }
    }
}

/// Vanilla-style merge: identical items within 0.5 blocks stack up to max size.
/// The donor is removed when empty; the receiver gets a count metadata update.
fn merge_items(ents: &mut Entities, idx: &ItemIndex) {
    let mut scratch: Vec<usize> = Vec::new();
    for i in 0..ents.list.len() {
        let (item, pos) = match &ents.list[i] {
            Ent { removed: false, kind: Kind::Item { stack, .. }, pos, .. } if stack.count < max_stack(stack.item) => (stack.item, *pos),
            _ => continue,
        };
        scratch.clear();
        idx.range(item_cell([pos[0] - 0.5, pos[1] - 0.5, pos[2] - 0.5]), item_cell([pos[0] + 0.5, pos[1] + 0.5, pos[2] + 0.5]), |j| scratch.push(j));
        for &j in &scratch {
            if j == i {
                continue;
            }
            let (jitem, jcount, jpos, jremoved) = match &ents.list[j] {
                Ent { removed, kind: Kind::Item { stack, .. }, pos, .. } => (stack.item, stack.count, *pos, *removed),
                _ => continue,
            };
            if jremoved || jitem != item {
                continue;
            }
            if (jpos[0] - pos[0]).abs() > 0.5 || (jpos[1] - pos[1]).abs() > 0.5 || (jpos[2] - pos[2]).abs() > 0.5 {
                continue;
            }
            let cur = match &ents.list[i].kind {
                Kind::Item { stack, .. } => stack.count,
                _ => break,
            };
            let room = max_stack(item) - cur;
            if room <= 0 {
                break;
            }
            let t = room.min(jcount);
            let left = jcount - t;
            if let Kind::Item { stack, .. } = &mut ents.list[i].kind {
                stack.count = cur + t;
            }
            if let Kind::Item { stack, .. } = &mut ents.list[j].kind {
                stack.count = left;
            }
            if left <= 0 {
                ents.list[j].removed = true;
            }
            let eid = ents.list[i].eid;
            ents.extra.push((eid, out::frame(out::Meta::new(eid).item(8, item, cur + t).finish())));
        }
    }
}

fn pickup(g: &mut Game, idx: &ItemIndex) {
    let mut picks: Vec<(usize, i32, Stack)> = Vec::new();
    {
        let ents = &g.entities;
        for p in g.players.iter().flatten() {
            if p.dead || p.gamemode == 3 {
                continue;
            }
            let lo = item_cell([p.pos[0] - 1.425, p.pos[1] - 0.6, p.pos[2] - 1.425]);
            let hi = item_cell([p.pos[0] + 1.425, p.pos[1] + 2.4, p.pos[2] + 1.425]);
            idx.range(lo, hi, |i| {
                let e = &ents.list[i];
                if e.removed {
                    return;
                }
                if let Kind::Item { stack, pickup_delay } = &e.kind {
                    if *pickup_delay > 0 {
                        return;
                    }
                    let dx = e.pos[0] - p.pos[0];
                    let dy = e.pos[1] - (p.pos[1] + 0.9);
                    let dz = e.pos[2] - p.pos[2];
                    if dx.abs() < 1.425 && dy.abs() < 1.5 && dz.abs() < 1.425 {
                        picks.push((p.slot, e.eid, *stack));
                    }
                }
            });
        }
    }
    let mut taken = ahash::AHashSet::new();
    for (slot, eid, stack) in picks {
        if !taken.insert(eid) {
            continue;
        }
        let p = g.player(slot).unwrap();
        let left = p.inv.add(stack.item, stack.count);
        let got = stack.count - left;
        if got <= 0 {
            taken.remove(&eid);
            continue;
        }
        let collector = p.eid;
        let inv = p.inventory_packet();
        p.send(inv);
        let f = out::frame(out::take_item(eid, collector, got));
        g.entities.extra.push((eid, f.clone()));
        if let Some(p) = g.player(slot) {
            p.out.push(f);
        }
        if let Some(&i) = g.entities.index.get(&eid) {
            let e = &mut g.entities.list[i];
            if left > 0 {
                if let Kind::Item { stack: s, .. } = &mut e.kind {
                    s.count = left;
                }
                taken.remove(&eid);
            } else {
                e.removed = true;
            }
        }
        super::track::equipment_changed(g, slot);
    }
}
