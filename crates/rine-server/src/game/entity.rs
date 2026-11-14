//! Non-player entities: dropped items, primed TNT, falling blocks.
//!
//! Physics runs data-parallel (rayon) against an immutable world view;
//! side effects (explosions, pickups, block placement) are collected and
//! applied afterwards on the game thread. Network updates are encoded once
//! per entity per tick and fanned out by the tracker.

use super::player::{max_stack, Stack};
use super::Game;
use crate::out;
use ahash::{AHashMap, AHashSet};
use bytes::Bytes;
use rayon::prelude::*;
use rine_protocol::BlockPos;
use rine_world::ChunkPos;
use std::time::Instant;
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
    /// Resting TNT: physics is skipped until a block under it changes or a blast
    /// pushes it. Its fuse still counts down.
    pub asleep: bool,
}

const NO_SLOT: u32 = u32::MAX;

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
    /// Slot in `list` by entity id (ids are dense), `NO_SLOT` when absent.
    pub slot: Vec<u32>,
    /// (eid, frame) movement updates produced this tick.
    pub moves: Vec<(i32, Bytes)>,
    /// Entities removed this tick (despawned by the tracker).
    pub removed: Vec<i32>,
    /// Extra per-entity frames (metadata changes, pickups...).
    pub extra: Vec<(i32, Bytes)>,
    /// Entities spawned since the tracker last ran; their spawn packets go out
    /// right away (between ticks) instead of waiting for the periodic tracker pass.
    pub fresh: Vec<i32>,
    /// Entity ids by chunk, rebuilt on the tracker's periodic pass. Entries can
    /// be stale (an entity may have moved): the tracker checks positions.
    pub grid: AHashMap<ChunkPos, Vec<i32>>,
}

impl Entities {
    pub fn slot_of(&self, eid: i32) -> Option<usize> {
        let s = *self.slot.get(eid as usize)?;
        (s != NO_SLOT).then_some(s as usize)
    }
    pub fn get(&self, eid: i32) -> Option<&Ent> {
        self.slot_of(eid).map(|i| &self.list[i])
    }
    pub fn spawn(&mut self, e: Ent) {
        let id = e.eid as usize;
        if id >= self.slot.len() {
            self.slot.resize(id + 1, NO_SLOT);
        }
        self.slot[id] = self.list.len() as u32;
        self.fresh.push(e.eid);
        self.list.push(e);
    }
    /// Drop removed entities. Swap-remove touches only the moved entity's slot,
    /// so the cost is one pass over the flags, not a rebuild of the index.
    fn compact(&mut self) {
        let mut i = 0;
        while i < self.list.len() {
            if self.list[i].removed {
                let e = self.list.swap_remove(i);
                self.removed.push(e.eid);
                self.slot[e.eid as usize] = NO_SLOT;
                if let Some(m) = self.list.get(i) {
                    self.slot[m.eid as usize] = i as u32;
                }
            } else {
                i += 1;
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
            outv.push(out::frame(out::Meta::new(e.eid).item(8, stack).finish()));
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
    g.entities.spawn(Ent { eid, uuid: Uuid::new_v4(), kind: Kind::Item { stack, pickup_delay }, pos, vel, on_ground: false, age: 0, removed: false, sent_pos: pos, sent_vel: vel, asleep: false });
}

pub fn spawn_tnt(g: &mut Game, pos: [f64; 3], fuse: i32) {
    let eid = g.alloc_eid();
    let a = rand_f64(eid as u64) * std::f64::consts::TAU;
    let vel = [-a.sin() * 0.02, 0.2, -a.cos() * 0.02];
    g.entities.spawn(Ent { eid, uuid: Uuid::new_v4(), kind: Kind::Tnt { fuse }, pos, vel, on_ground: false, age: 0, removed: false, sent_pos: pos, sent_vel: vel, asleep: false });
}

pub fn spawn_falling(g: &mut Game, pos: [f64; 3], state: u16) {
    let eid = g.alloc_eid();
    g.entities.spawn(Ent { eid, uuid: Uuid::new_v4(), kind: Kind::Falling { state }, pos, vel: [0.0; 3], on_ground: false, age: 0, removed: false, sent_pos: pos, sent_vel: [0.0; 3], asleep: false });
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

/// Collision boxes (world coords) of all blocks touching `r`.
fn boxes_in(w: &super::world::World, r: [f64; 6], out: &mut Vec<[f64; 6]>) {
    let b = &rine_data::get().blocks;
    out.clear();
    for x in (r[0] - 1e-7).floor() as i32..=(r[3] + 1e-7).floor() as i32 {
        // one extra layer below: fences and walls are 1.5 blocks tall
        for y in (r[1] - 1.0).floor() as i32..=(r[4] + 1e-7).floor() as i32 {
            for z in (r[2] - 1e-7).floor() as i32..=(r[5] + 1e-7).floor() as i32 {
                let st = w.get_block(x, y, z);
                if b.is_air(st) {
                    continue;
                }
                for bx in &b.shapes[b.state(st).shape as usize] {
                    let (fx, fy, fz) = (x as f64, y as f64, z as f64);
                    out.push([fx + bx[0] as f64, fy + bx[1] as f64, fz + bx[2] as f64, fx + bx[3] as f64, fy + bx[4] as f64, fz + bx[5] as f64]);
                }
            }
        }
    }
}

/// Clip movement `d` along `axis` (0..3) of box `bb` against `boxes`
/// (vanilla `Shapes.collide`: boxes must overlap on the other two axes).
fn clip(boxes: &[[f64; 6]], bb: &[f64; 6], axis: usize, mut d: f64) -> f64 {
    const EPS: f64 = 1e-7;
    let (a1, a2) = match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    for bx in boxes {
        if bx[a1 + 3] <= bb[a1] + EPS || bx[a1] >= bb[a1 + 3] - EPS || bx[a2 + 3] <= bb[a2] + EPS || bx[a2] >= bb[a2 + 3] - EPS {
            continue;
        }
        if d > 0.0 && bx[axis] >= bb[axis + 3] - EPS {
            d = d.min(bx[axis] - bb[axis + 3]);
        } else if d < 0.0 && bx[axis + 3] <= bb[axis] + EPS {
            d = d.max(bx[axis + 3] - bb[axis]);
        }
    }
    d
}

fn bbox(e: &Ent) -> [f64; 6] {
    let (w, h) = e.kind.size();
    let hw = w / 2.0;
    [e.pos[0] - hw, e.pos[1], e.pos[2] - hw, e.pos[0] + hw, e.pos[1] + h, e.pos[2] + hw]
}

fn inside(boxes: &[[f64; 6]], bb: &[f64; 6]) -> bool {
    const EPS: f64 = 1e-7;
    boxes.iter().any(|bx| (0..3).all(|i| bx[i] < bb[i + 3] - EPS && bx[i + 3] > bb[i] + EPS))
}

/// Swept AABB movement against real block collision shapes. Y first, then
/// the larger of X/Z, as vanilla `Entity.collide`. An entity stuck inside a
/// block is nudged toward the nearest open neighbor (`moveTowardsClosestSpace`).
fn move_ent(w: &super::world::World, e: &mut Ent) {
    let mut bb = bbox(e);
    let v = e.vel;
    let mut boxes = Vec::new();
    boxes_in(w, bb, &mut boxes);
    if inside(&boxes, &bb) {
        let (cx, cy, cz) = (e.pos[0].floor() as i32, (bb[1] + bb[4]) * 0.5, e.pos[2].floor() as i32);
        let cy = cy.floor() as i32;
        let free = |x: i32, y: i32, z: i32| {
            let st = w.get_block(x, y, z);
            let b = &rine_data::get().blocks;
            b.shapes[b.state(st).shape as usize].is_empty()
        };
        let (fx, fz) = (e.pos[0] - cx as f64, e.pos[2] - cz as f64);
        // candidates: (distance to that face, direction)
        let mut best: Option<(f64, [f64; 3])> = None;
        for (dist, dir, (dx, dy, dz)) in [
            (fx, [-1.0, 0.0, 0.0], (-1, 0, 0)),
            (1.0 - fx, [1.0, 0.0, 0.0], (1, 0, 0)),
            (fz, [0.0, 0.0, -1.0], (0, 0, -1)),
            (1.0 - fz, [0.0, 0.0, 1.0], (0, 0, 1)),
            (1.0 - ((bb[1] + bb[4]) * 0.5 - cy as f64), [0.0, 1.0, 0.0], (0, 1, 0)),
        ] {
            if free(cx + dx, cy + dy, cz + dz) && best.map_or(true, |(d, _)| dist < d) {
                best = Some((dist, dir));
            }
        }
        let dir = best.map(|b| b.1).unwrap_or([0.0, 1.0, 0.0]);
        let push = 0.1 + 0.1 * rand_f64(e.eid as u64 ^ (e.age as u64) << 16);
        for i in 0..3 {
            if dir[i] != 0.0 {
                e.vel[i] = dir[i] * push;
            }
        }
        e.pos = [e.pos[0] + e.vel[0], e.pos[1] + e.vel[1], e.pos[2] + e.vel[2]];
        e.on_ground = false;
        return;
    }
    let reach = [bb[0] + v[0].min(0.0), bb[1] + v[1].min(0.0), bb[2] + v[2].min(0.0), bb[3] + v[0].max(0.0), bb[4] + v[1].max(0.0), bb[5] + v[2].max(0.0)];
    boxes_in(w, reach, &mut boxes);
    let dy = clip(&boxes, &bb, 1, v[1]);
    bb[1] += dy;
    bb[4] += dy;
    let order = if v[0].abs() < v[2].abs() { [2usize, 0] } else { [0, 2] };
    let mut d = [0.0, dy, 0.0];
    for axis in order {
        let m = clip(&boxes, &bb, axis, v[axis]);
        bb[axis] += m;
        bb[axis + 3] += m;
        d[axis] = m;
    }
    e.pos[0] += d[0];
    e.pos[1] += d[1];
    e.pos[2] += d[2];
    e.on_ground = v[1] < 0.0 && d[1] != v[1];
    for i in 0..3 {
        if d[i] != v[i] {
            e.vel[i] = 0.0;
        }
    }
}

/// Milliseconds since `t`; restarts the clock.
fn lap_ms(t: &mut Instant) -> f64 {
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    *t = Instant::now();
    ms
}

/// Cell (block coordinates) an entity's position falls in.
fn cell(p: [f64; 3]) -> [i32; 3] {
    [p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32]
}

pub fn tick(g: &mut Game) {
    let mut pt = Instant::now();
    let mut pf: Vec<(&'static str, f64)> = Vec::new();
    // Blocks changed since the last entity tick. A sleeping TNT wakes when the
    // block it rests on (cell below) or the cell it is in changed.
    let wakes: AHashSet<[i32; 3]> = g.world.wake.drain(..).map(|p| [p.x, p.y, p.z]).collect();
    pf.push(("wake", lap_ms(&mut pt)));
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
            if e.asleep && !wakes.is_empty() {
                let k = cell(e.pos);
                if wakes.contains(&k) || wakes.contains(&[k[0], k[1] - 1, k[2]]) {
                    e.asleep = false;
                }
            }
            if e.asleep {
                // resting TNT: only the fuse counts down
                if let Kind::Tnt { fuse } = &mut e.kind {
                    *fuse -= 1;
                    if *fuse <= 0 {
                        e.removed = true;
                        fx.explode.push([e.pos[0], e.pos[1] + 0.06125, e.pos[2]]);
                    }
                }
                return fx;
            }
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
            // TNT on the ground with no velocity is at rest: sleep it
            if matches!(e.kind, Kind::Tnt { .. }) && !e.removed && e.on_ground && e.vel == [0.0; 3] {
                e.asleep = true;
            }
            fx
        })
        .reduce(Fx::default, |mut a, b| {
            a.explode.extend(b.explode);
            a.land.extend(b.land);
            a
        });

    pf.push(("physics", lap_ms(&mut pt)));
    // ---- item merging (every 10 ticks) and pickup, both via one item index
    let idx = ItemIndex::build(&g.entities.list);
    if g.tick % 10 == 0 {
        merge_items(&mut g.entities, &idx);
    }
    pickup(g, &idx);

    pf.push(("items", lap_ms(&mut pt)));
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
    g.pending_blasts.extend(fx.explode);
    pf.push(("land", lap_ms(&mut pt)));
    super::explosion::process_pending(g);
    pf.push(("explode", lap_ms(&mut pt)));

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
            // Vanilla update intervals (EntityType.updateInterval): the client
            // simulates these entities itself between syncs. A sudden impulse
            // (explosion knockback) is sent immediately.
            let interval = match e.kind {
                Kind::Tnt { .. } => 10,
                Kind::Item { .. } | Kind::Falling { .. } => 20,
            };
            let dv = (0..3).map(|i| (e.vel[i] - e.sent_vel[i]).abs()).fold(0.0, f64::max);
            let impulse = dv > 0.3;
            if !impulse && (e.age + e.eid) % interval != 0 {
                return None;
            }
            let mut buf = bytes::BytesMut::new();
            let old = (e.sent_pos[0], e.sent_pos[1], e.sent_pos[2]);
            let new = (e.pos[0], e.pos[1], e.pos[2]);
            let mv = if ((e.age + e.eid) / interval) % 6 == 0 { None } else { out::move_entity(e.eid, old, new, None, e.on_ground) };
            let mv = mv.unwrap_or_else(|| out::position_sync(e.eid, new.0, new.1, new.2, 0.0, 0.0, e.on_ground));
            buf.extend_from_slice(&out::frame(mv));
            if vel_changed && matches!(e.kind, Kind::Tnt { .. } | Kind::Falling { .. }) {
                buf.extend_from_slice(&out::frame(out::entity_motion(e.eid, e.vel[0], e.vel[1], e.vel[2])));
                e.sent_vel = e.vel;
            }
            e.sent_pos = e.pos;
            Some((e.eid, buf.freeze()))
        })
        .collect();
    g.entities.moves.extend(moves);
    pf.push(("moves", lap_ms(&mut pt)));
    g.entities.compact();
    pf.push(("compact", lap_ms(&mut pt)));
    let total: f64 = pf.iter().map(|x| x.1).sum();
    if total > 30.0 && std::env::var_os("RINE_PROFILE").is_some() {
        eprintln!("ENTITY {:.1} {:?}", total, pf.iter().map(|(n, m)| format!("{n}={m:.1}")).collect::<Vec<_>>());
    }
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

/// `ItemEntity.isMergable` plus the stack and position of an item entity.
fn mergeable(e: &Ent) -> Option<(Stack, i32, [f64; 3])> {
    match &e.kind {
        Kind::Item { stack, pickup_delay } if !e.removed && stack.count < max_stack(stack.item) => Some((*stack, *pickup_delay, e.pos)),
        _ => None,
    }
}

/// `ItemEntity.mergeWithNeighbours`: items of the same kind (item and wear)
/// within 0.5 blocks horizontally and overlapping vertically merge when the
/// combined count fits in one stack. The smaller stack is absorbed into the
/// other one (ties: this entity is absorbed). The receiver takes the larger
/// pickup delay and the younger age, and its count is sent as metadata.
fn merge_items(ents: &mut Entities, idx: &ItemIndex) {
    let mut scratch: Vec<usize> = Vec::new();
    for i in 0..ents.list.len() {
        let Some((_, _, pos)) = mergeable(&ents.list[i]) else { continue };
        scratch.clear();
        idx.range(item_cell([pos[0] - 0.5, pos[1] - 0.25, pos[2] - 0.5]), item_cell([pos[0] + 0.5, pos[1] + 0.25, pos[2] + 0.5]), |j| scratch.push(j));
        for &j in &scratch {
            if j == i {
                continue;
            }
            let Some((si, _, pi)) = mergeable(&ents.list[i]) else { break };
            let Some((sj, _, pj)) = mergeable(&ents.list[j]) else { continue };
            if !si.same_kind(&sj) || (pj[0] - pi[0]).abs() > 0.5 || (pj[2] - pi[2]).abs() > 0.5 || (pj[1] - pi[1]).abs() >= 0.25 {
                continue;
            }
            if si.count + sj.count > max_stack(si.item) {
                continue;
            }
            // the smaller stack is absorbed; ties absorb `i` into `j`
            let (donor, recv) = if sj.count < si.count { (j, i) } else { (i, j) };
            let (sd, sr) = (if donor == i { si } else { sj }, if recv == i { si } else { sj });
            let total = sr.count + sd.count;
            let (dd, rd) = (donor_delay(&ents.list[donor]), donor_delay(&ents.list[recv]));
            let (da, ra) = (ents.list[donor].age, ents.list[recv].age);
            ents.list[donor].removed = true;
            let r = &mut ents.list[recv];
            r.kind = Kind::Item { stack: sr.with_count(total), pickup_delay: rd.max(dd) };
            r.age = ra.min(da);
            let eid = r.eid;
            ents.extra.push((eid, out::frame(out::Meta::new(eid).item(8, &sr.with_count(total)).finish())));
            if donor == i {
                break;
            }
        }
    }
}

fn donor_delay(e: &Ent) -> i32 {
    match &e.kind {
        Kind::Item { pickup_delay, .. } => *pickup_delay,
        _ => 0,
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
        let left = p.inv.add(stack);
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
        if let Some(i) = g.entities.slot_of(eid) {
            let e = &mut g.entities.list[i];
            if left > 0 {
                let rest = stack.with_count(left);
                if let Kind::Item { stack: s, .. } = &mut e.kind {
                    *s = rest;
                }
                // partial pickup: trackers see the smaller stack
                g.entities.extra.push((eid, out::frame(out::Meta::new(eid).item(8, &rest).finish())));
                taken.remove(&eid);
            } else {
                e.removed = true;
            }
        }
        super::track::equipment_changed(g, slot);
    }
}

impl Ent {
    #[cfg(test)]
    fn count(&self) -> i32 {
        match &self.kind {
            Kind::Item { stack, .. } => stack.count,
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(eid: i32, kind_item: u32, count: i32, damage: u16, x: f64) -> Ent {
        Ent {
            eid,
            uuid: Uuid::nil(),
            kind: Kind::Item { stack: Stack::damaged(kind_item, count, damage), pickup_delay: 0 },
            pos: [x, 0.0, 0.0],
            vel: [0.0; 3],
            on_ground: false,
            age: 0,
            removed: false,
            sent_pos: [x, 0.0, 0.0],
            sent_vel: [0.0; 3],
            asleep: false,
        }
    }

    #[test]
    fn merge_only_when_the_total_fits_and_wear_matches() {
        rine_data::init();
        let cobble = crate::game::items::id("minecraft:cobblestone").unwrap();
        let mut ents = Entities::default();
        ents.list = vec![
            item(1, cobble, 40, 0, 0.0),
            item(2, cobble, 20, 0, 0.2), // 40 + 20 fits: absorbed into eid 1
            item(3, cobble, 30, 0, 0.1), // 60 + 30 does not fit
            item(4, cobble, 5, 1, 0.1),  // different wear: no merge
        ];
        let idx = ItemIndex::build(&ents.list);
        merge_items(&mut ents, &idx);
        assert_eq!(ents.list[0].count(), 60);
        assert!(ents.list[1].removed);
        assert_eq!(ents.list[2].count(), 30);
        assert!(!ents.list[2].removed);
        assert_eq!(ents.list[3].count(), 5);
        assert!(ents.extra.iter().any(|(eid, _)| *eid == 1));
    }
}
