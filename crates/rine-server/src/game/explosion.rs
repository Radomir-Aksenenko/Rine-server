//! Batched TNT explosions.
//!
//! Follows vanilla `ServerExplosion` semantics for a whole batch of centers at
//! once: block rays run in parallel, knockback is summed per entity through a
//! spatial hash, players get exposure-weighted knockback and damage, and the
//! block changes, drops and chained TNT are applied on the game thread.
//!
//! Approximations (deliberate, for batch speed):
//! - Rays read the world as it was at the start of the batch. Vanilla resolves
//!   explosions one after another, so a block removed by an earlier blast still
//!   resists later ones. Overlapping blasts merge, so the visible result is close.
//! - Items, primed TNT and falling blocks use exposure 1.0 (no shielding).
//! - Player exposure is the share of 8 AABB-corner rays from the blast that
//!   reach the player without hitting a solid block (vanilla casts many more).
//! - Knockback sum per player is capped at `KNOCK_CAP` blocks per tick, and
//!   damage is the largest single-blast value (vanilla i-frames do the same job).

use super::entity::{self, rand_f64};
use super::loot;
use super::player::{max_stack, Stack};
use super::world::{ChunkSlot, World};
use super::Game;
use crate::out;
use ahash::AHashMap;
use bytes::Bytes;
use rayon::prelude::*;
use rine_protocol::BlockPos;
use rine_world::ChunkPos;
use std::sync::OnceLock;

const RAY_STEP: f64 = 0.3;
const RAY_DECAY: f64 = 0.22500001;
/// `explosion_emitter` particle is used for power >= this, when blocks broke.
const EMITTER_MIN_POWER: f32 = 2.0;
/// Explode packets only reach players within this distance of the blast.
const PACKET_RANGE: f64 = 64.0;
/// Max explode packets per player per tick (nearest blasts win).
const PACKETS_PER_PLAYER: usize = 32;
/// Max knockback magnitude (blocks per tick) a player receives in one tick.
const KNOCK_CAP: f64 = 4.0;
const PLAYER_HALF_W: f64 = 0.3;
const PLAYER_H: f64 = 1.8;
const ITEM_DROP_DELAY: i32 = 10;

/// Per-state data the ray march needs.
#[derive(Clone, Copy, Debug)]
pub struct BlockProps {
    pub air: bool,
    pub resistance: f32,
    /// `hardness >= 0`; unbreakable blocks (bedrock) resist but are never removed.
    pub breakable: bool,
}

/// Vanilla ray march over the 16^3 cube surface.
///
/// `state(x, y, z)` gives the block state at a cell, `props(state)` its
/// resistance data. Destroyed cells are written to `out`, sorted and deduped.
/// Pure function: no game state, usable from tests and rayon threads.
pub fn blast_cells(
    center: [f64; 3],
    power: f32,
    seed: u64,
    mut state: impl FnMut(i32, i32, i32) -> u16,
    props: impl Fn(u16) -> BlockProps,
    out: &mut Vec<(BlockPos, u16)>,
) {
    out.clear();
    let radius = power as f64;
    let mut rng = seed;
    for i in 0..16i32 {
        for j in 0..16i32 {
            for k in 0..16i32 {
                if i != 0 && i != 15 && j != 0 && j != 15 && k != 0 && k != 15 {
                    continue;
                }
                let mut dx = i as f64 / 15.0 * 2.0 - 1.0;
                let mut dy = j as f64 / 15.0 * 2.0 - 1.0;
                let mut dz = k as f64 / 15.0 * 2.0 - 1.0;
                let len = (dx * dx + dy * dy + dz * dz).sqrt();
                dx /= len;
                dy /= len;
                dz /= len;

                rng = rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
                let mut f = radius * (0.7 + rand_f64(rng) * 0.6);
                let (mut x, mut y, mut z) = (center[0], center[1], center[2]);
                let mut cell = (i32::MIN, i32::MIN, i32::MIN);
                let mut cur_state = 0u16;
                let mut cur = BlockProps { air: true, resistance: 0.0, breakable: true };
                while f > 0.0 {
                    let c = (x.floor() as i32, y.floor() as i32, z.floor() as i32);
                    if c != cell {
                        cell = c;
                        cur_state = state(c.0, c.1, c.2);
                        cur = props(cur_state);
                    }
                    if !cur.air {
                        f -= (cur.resistance as f64 + 0.3) * 0.3;
                    }
                    if f > 0.0 && !cur.air && cur.breakable {
                        out.push((BlockPos::new(c.0, c.1, c.2), cur_state));
                    }
                    x += dx * RAY_STEP;
                    y += dy * RAY_STEP;
                    z += dz * RAY_STEP;
                    f -= RAY_DECAY;
                }
            }
        }
    }
    out.sort_unstable_by_key(|(p, _)| (p.x, p.y, p.z));
    out.dedup_by_key(|(p, _)| *p);
}

/// Block reader that caches the last chunk lookup (rays move through few chunks).
struct ChunkReader<'a> {
    world: &'a World,
    cx: i32,
    cz: i32,
    slot: Option<&'a ChunkSlot>,
    valid: bool,
}

impl<'a> ChunkReader<'a> {
    fn new(world: &'a World) -> Self {
        Self { world, cx: 0, cz: 0, slot: None, valid: false }
    }
    #[inline]
    fn get(&mut self, x: i32, y: i32, z: i32) -> u16 {
        let (cx, cz) = (x >> 4, z >> 4);
        if !self.valid || cx != self.cx || cz != self.cz {
            self.slot = self.world.chunks.get(&ChunkPos::new(cx, cz));
            self.cx = cx;
            self.cz = cz;
            self.valid = true;
        }
        match self.slot {
            Some(s) => s.chunk.get(x & 15, y, z & 15),
            None => 0,
        }
    }
}

fn props_of(s: u16) -> BlockProps {
    let b = &rine_data::get().blocks;
    let st = b.state(s);
    BlockProps { air: b.is_air(s), resistance: st.resistance, breakable: st.hardness >= 0.0 }
}

fn seed_for(c: [f64; 3], tick: u64) -> u64 {
    c[0].to_bits() ^ c[1].to_bits().rotate_left(21) ^ c[2].to_bits().rotate_left(42) ^ tick.wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

fn cell_of(p: &[f64; 3], size: f64) -> [i32; 3] {
    [(p[0] / size).floor() as i32, (p[1] / size).floor() as i32, (p[2] / size).floor() as i32]
}

/// Uniform grid over blast centers. `visit` covers every center within `size`.
struct Grid {
    size: f64,
    cells: AHashMap<[i32; 3], Vec<u32>>,
}

impl Grid {
    fn build(points: &[[f64; 3]], size: f64) -> Self {
        let mut cells: AHashMap<[i32; 3], Vec<u32>> = AHashMap::new();
        for (i, p) in points.iter().enumerate() {
            cells.entry(cell_of(p, size)).or_default().push(i as u32);
        }
        Grid { size, cells }
    }
    fn visit(&self, p: [f64; 3], mut f: impl FnMut(usize)) {
        let c = cell_of(&p, self.size);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(v) = self.cells.get(&[c[0] + dx, c[1] + dy, c[2] + dz]) {
                        for &i in v {
                            f(i as usize);
                        }
                    }
                }
            }
        }
    }
}

/// Fraction of 8 AABB-corner rays from `c` that reach the player unobstructed.
fn exposure(rd: &mut ChunkReader, c: [f64; 3], feet: [f64; 3]) -> f64 {
    let b = &rine_data::get().blocks;
    let mut clear = 0u32;
    for &x in &[feet[0] - PLAYER_HALF_W, feet[0] + PLAYER_HALF_W] {
        for &y in &[feet[1], feet[1] + PLAYER_H] {
            for &z in &[feet[2] - PLAYER_HALF_W, feet[2] + PLAYER_HALF_W] {
                let d = [x - c[0], y - c[1], z - c[2]];
                let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                let n = ((len / 0.2).ceil() as i32).max(1);
                let mut ok = true;
                for k in 1..n {
                    let t = k as f64 / n as f64;
                    let p = [c[0] + d[0] * t, c[1] + d[1] * t, c[2] + d[2] * t];
                    let st = rd.get(p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32);
                    if !b.is_air(st) && !b.has(st, rine_data::blocks::flag::NO_COLLISION) {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    clear += 1;
                }
            }
        }
    }
    clear as f64 / 8.0
}

struct Ids {
    emitter: u32,
    explosion: u32,
    sound: u32,
}

fn ids() -> &'static Ids {
    static IDS: OnceLock<Ids> = OnceLock::new();
    IDS.get_or_init(|| {
        let r = &rine_data::get().registries;
        Ids {
            emitter: r.id("minecraft:particle_type", "minecraft:explosion_emitter").unwrap_or(0),
            explosion: r.id("minecraft:particle_type", "minecraft:explosion").unwrap_or(0),
            sound: r.id("minecraft:sound_event", "minecraft:entity.generic.explode").unwrap_or(0),
        }
    })
}

/// Up to `k` nearest blasts within `range` of `p`, nearest first.
fn nearest(centers: &[[f64; 3]], p: [f64; 3], k: usize, range: f64) -> Vec<usize> {
    let r2 = range * range;
    let mut best: Vec<(f64, usize)> = Vec::with_capacity(k + 1);
    for (i, c) in centers.iter().enumerate() {
        let d = [c[0] - p[0], c[1] - p[1], c[2] - p[2]];
        let d2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        if d2 > r2 || (best.len() == k && d2 >= best[k - 1].0) {
            continue;
        }
        let at = best.partition_point(|(bd, _)| *bd <= d2);
        best.insert(at, (d2, i));
        if best.len() > k {
            best.pop();
        }
    }
    best.into_iter().map(|(_, i)| i).collect()
}

/// Per-player result of the knockback/damage pass.
struct Hit {
    slot: usize,
    knock: [f64; 3],
    /// Strongest contributing blast (distance, index), used for the personal packet.
    nearest: Option<(f64, usize)>,
    damage: f32,
}

pub fn explode_batch(g: &mut Game, centers: Vec<[f64; 3]>, power: f32) {
    if centers.is_empty() || !(power > 0.0) {
        return;
    }
    let ids = ids();
    let tick = g.tick;
    let p = power as f64;
    let reach = 2.0 * p;
    let grid = Grid::build(&centers, reach);

    // ---- phase A: block rays, parallel, read-only world
    let blasts: Vec<Vec<(BlockPos, u16)>> = {
        let world = &g.world;
        centers
            .par_iter()
            .map_init(Vec::new, |buf, c| {
                let mut rd = ChunkReader::new(world);
                blast_cells(*c, power, seed_for(*c, tick), |x, y, z| rd.get(x, y, z), props_of, buf);
                buf.clone()
            })
            .collect()
    };
    let counts: Vec<i32> = blasts.iter().map(|v| v.len() as i32).collect();

    // ---- phase C: entity knockback, each entity sums impulses from nearby blasts
    g.entities.list.par_iter_mut().for_each(|e| {
        if e.removed {
            return;
        }
        let m = e.mid();
        let mut acc = [0.0f64; 3];
        grid.visit(m, |i| {
            let c = centers[i];
            let d = [m[0] - c[0], m[1] - c[1], m[2] - c[2]];
            let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if dist > reach {
                return;
            }
            let impact = 1.0 - dist / reach;
            let dir = if dist > 1e-9 { [d[0] / dist, d[1] / dist, d[2] / dist] } else { [0.0, 1.0, 0.0] };
            acc[0] += dir[0] * impact;
            acc[1] += dir[1] * impact;
            acc[2] += dir[2] * impact;
        });
        e.vel[0] += acc[0];
        e.vel[1] += acc[1];
        e.vel[2] += acc[2];
    });

    // ---- phase D: players, exposure-weighted knockback and damage (pre-blast world)
    let players: Vec<(usize, [f64; 3], [f64; 3], bool, bool)> = g
        .players
        .iter()
        .flatten()
        .map(|pl| (pl.slot, pl.pos, pl.eye(), pl.dead, pl.gamemode == 3))
        .collect();
    let hits: Vec<Hit> = {
        let world = &g.world;
        players
            .par_iter()
            .filter(|(_, _, _, dead, spectator)| !dead && !spectator)
            .map(|&(slot, feet, eye, _, _)| {
                let mut rd = ChunkReader::new(world);
                let mut knock = [0.0f64; 3];
                let mut nearest_hit: Option<(f64, usize)> = None;
                let mut damage = 0.0f32;
                grid.visit(eye, |i| {
                    let c = centers[i];
                    let d = [eye[0] - c[0], eye[1] - c[1], eye[2] - c[2]];
                    let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                    if dist > reach {
                        return;
                    }
                    let impact = (1.0 - dist / reach) * exposure(&mut rd, c, feet);
                    if impact <= 0.0 {
                        return;
                    }
                    let dir = if dist > 1e-9 { [d[0] / dist, d[1] / dist, d[2] / dist] } else { [0.0, 1.0, 0.0] };
                    for a in 0..3 {
                        knock[a] += dir[a] * impact;
                    }
                    let dmg = ((impact * impact + impact) / 2.0 * 7.0 * reach + 1.0) as f32;
                    damage = damage.max(dmg);
                    if nearest_hit.map_or(true, |(d0, _)| dist < d0) {
                        nearest_hit = Some((dist, i));
                    }
                });
                Hit { slot, knock, nearest: nearest_hit, damage }
            })
            .collect()
    };
    for h in &hits {
        if h.damage > 0.0 {
            super::interact::damage(g, h.slot, h.damage, "explosion");
        }
    }

    // ---- phase B: block removal, primed TNT, merged drops (game thread)
    let mut all: Vec<(BlockPos, u16)> = blasts.iter().flatten().copied().collect();
    all.sort_unstable_by_key(|(p, _)| (p.x, p.y, p.z));
    all.dedup_by_key(|(p, _)| *p);
    drop(blasts);

    let b = &rine_data::get().blocks;
    let air = b.default_state("minecraft:air");
    let mut drops: AHashMap<(u32, i32, i32, i32), i32> = AHashMap::new();
    for (pos, st) in all {
        if !g.world.is_loaded(pos.x, pos.z) {
            continue;
        }
        let is_tnt = b.block_of(st).name == "minecraft:tnt";
        g.world.set_block(pos, air);
        if is_tnt {
            let fuse = 10 + (rand_f64(seed_for([pos.x as f64, pos.y as f64, pos.z as f64], tick)) * 20.0) as i32;
            entity::spawn_tnt(g, [pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5], fuse);
        } else {
            for s in loot::block_drops(st, 0) {
                *drops.entry((s.item, pos.x >> 1, pos.y >> 1, pos.z >> 1)).or_insert(0) += s.count;
            }
        }
    }
    let mut drops: Vec<((u32, i32, i32, i32), i32)> = drops.into_iter().collect();
    drops.sort_unstable_by_key(|(k, _)| *k);
    for (n, ((item, cx, cy, cz), count)) in drops.into_iter().enumerate() {
        let max = max_stack(item).max(1);
        let center = [cx as f64 * 2.0 + 1.0, cy as f64 * 2.0 + 1.0, cz as f64 * 2.0 + 1.0];
        let mut left = count;
        let mut k = 0u64;
        while left > 0 {
            let take = left.min(max);
            left -= take;
            let s = seed_for(center, tick ^ ((n as u64) << 20) ^ k);
            k += 1;
            let vel = [(rand_f64(s) - 0.5) * 0.2, 0.2, (rand_f64(s.rotate_left(17)) - 0.5) * 0.2];
            super::entity::spawn_item(g, center, vel, Stack::new(item, take), ITEM_DROP_DELAY);
        }
    }

    // ---- phase E: network
    let particle_for = |n: i32| if power >= EMITTER_MIN_POWER && n > 0 { ids.emitter } else { ids.explosion };
    let shared: Vec<Bytes> = centers
        .par_iter()
        .zip(counts.par_iter())
        .map(|(c, &n)| out::frame(out::explode(c[0], c[1], c[2], power, n, None, particle_for(n), ids.sound)))
        .collect();
    let lists: Vec<Vec<usize>> = players
        .par_iter()
        .filter(|(_, _, _, dead, spectator)| !dead && !spectator)
        .map(|&(_, _, eye, _, _)| nearest(&centers, eye, PACKETS_PER_PLAYER, PACKET_RANGE))
        .collect();
    for (h, list) in hits.iter().zip(lists.iter()) {
        let Some(pl) = g.player(h.slot) else { continue };
        let mut skip = None;
        let knock_len = (h.knock[0] * h.knock[0] + h.knock[1] * h.knock[1] + h.knock[2] * h.knock[2]).sqrt();
        if knock_len > 0.0 {
            if let Some((_, i)) = h.nearest {
                let scale = if knock_len > KNOCK_CAP { KNOCK_CAP / knock_len } else { 1.0 };
                let k = [h.knock[0] * scale, h.knock[1] * scale, h.knock[2] * scale];
                let c = centers[i];
                let n = counts[i];
                pl.out.push(out::frame(out::explode(c[0], c[1], c[2], power, n, Some((k[0], k[1], k[2])), particle_for(n), ids.sound)));
                skip = Some(i);
            }
        }
        for &i in list {
            if Some(i) != skip {
                pl.out.push(shared[i].clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AIR: u16 = 0;
    const STONE: u16 = 1;
    const BEDROCK: u16 = 2;
    const DIRT: u16 = 3;

    /// Synthetic resistances: stone 6.0, dirt 0.5, bedrock unbreakable.
    fn test_props(s: u16) -> BlockProps {
        match s {
            AIR => BlockProps { air: true, resistance: 0.0, breakable: true },
            BEDROCK => BlockProps { air: false, resistance: 3_600_000.0, breakable: false },
            DIRT => BlockProps { air: false, resistance: 0.5, breakable: true },
            _ => BlockProps { air: false, resistance: 6.0, breakable: true },
        }
    }

    fn run(center: [f64; 3], power: f32, state: impl Fn(i32, i32, i32) -> u16) -> Vec<(BlockPos, u16)> {
        let mut out = Vec::new();
        blast_cells(center, power, 42, |x, y, z| state(x, y, z), test_props, &mut out);
        out
    }

    fn max_dist(out: &[(BlockPos, u16)], c: [f64; 3]) -> f64 {
        out.iter()
            .map(|(p, _)| {
                let d = [p.x as f64 + 0.5 - c[0], p.y as f64 + 0.5 - c[1], p.z as f64 + 0.5 - c[2]];
                (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
            })
            .fold(0.0, f64::max)
    }

    #[test]
    fn stone_crater_is_small_and_centered() {
        // Vanilla: stone (resistance 6) eats ~2.1 intensity per 0.3 step, so a
        // power-4 blast only opens a small hole in solid stone.
        let out = run([0.5, 0.5, 0.5], 4.0, |_, _, _| STONE);
        assert!(!out.is_empty());
        assert!(out.iter().any(|(p, _)| *p == BlockPos::new(0, 0, 0)));
        assert!(max_dist(&out, [0.5, 0.5, 0.5]) <= 2.5, "stone crater too big: {}", max_dist(&out, [0.5, 0.5, 0.5]));
    }

    #[test]
    fn low_resistance_volume_gives_roughly_spherical_blast() {
        let c = [0.5, 0.5, 0.5];
        let out = run(c, 4.0, |_, _, _| DIRT);
        let r = max_dist(&out, c);
        assert!(r >= 2.5 && r <= 6.0, "unexpected reach {r}");
        // Roughly spherical: neighbours along several axes are removed.
        assert!(out.iter().any(|(p, _)| *p == BlockPos::new(1, 0, 0)));
        assert!(out.iter().any(|(p, _)| *p == BlockPos::new(0, -1, 0)));
        // Output is sorted and unique.
        assert!(out.windows(2).all(|w| (w[0].0.x, w[0].0.y, w[0].0.z) < (w[1].0.x, w[1].0.y, w[1].0.z)));
    }

    #[test]
    fn bedrock_is_never_removed() {
        let out = run([0.5, 0.5, 0.5], 4.0, |_, _, _| BEDROCK);
        assert!(out.is_empty());
    }

    #[test]
    fn nothing_beyond_reach_in_air() {
        // All air: rays die at ~6.9 blocks, and air is never removed.
        let out = run([0.5, 0.5, 0.5], 4.0, |_, _, _| AIR);
        assert!(out.is_empty());
    }

    /// Worst-case ray cost: 10,000 blasts in all-air (no block is destroyed, so
    /// every ray runs its full length). Run with
    /// `cargo test --release -p rine-server explosion -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench_10k_blasts() {
        let mut out = Vec::new();
        let start = std::time::Instant::now();
        let mut total = 0usize;
        for n in 0..10_000u32 {
            let c = [n as f64 * 0.37 + 0.5, 64.5, (n % 97) as f64 + 0.5];
            blast_cells(c, 4.0, seed_for(c, 1), |_, _, _| STONE, test_props, &mut out);
            total += out.len();
        }
        let el = start.elapsed();
        println!("10000 blasts (stone, single thread): {:?}, {} blocks", el, total);
    }
}
