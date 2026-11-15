//! Random block ticks (vanilla `ServerChunkCache.tickChunks` ->
//! `ServerLevel.tickChunk`).
//!
//! Vanilla, per game tick, for every entity-ticking chunk (Chebyshev distance
//! <= simulation distance from a player that has chunk tickets), in that
//! chunk:
//! - `randomTickSpeed` times: with chance 1/48, precipitation at a random
//!   column (`tickPrecipitation`: freezing water, snow layers, cauldrons);
//! - for every section with randomly ticking blocks, `randomTickSpeed` times:
//!   a uniformly random position in the section; if its state is randomly
//!   ticking, `randomTick`; then if its fluid is randomly ticking (lava),
//!   the fluid's `randomTick` too (lava blocks get both: MC-224294).
//!
//! The level's random source is seeded from the clock in vanilla, so the
//! exact rolls are not observable. Rine keeps the distribution and the
//! per-tick structure: the same number of attempts per section, uniform
//! positions, the same per-block chances.
//!
//! Speed: the set of ticking chunks is kept incrementally (a player moving
//! to another chunk adjusts reference counts; nothing is rebuilt per tick).
//! Sections without ticking blocks are skipped by their counter. Sampling
//! is read-only and runs in parallel on groups of chunks with a per-chunk
//! random stream; the hits are applied serially in chunk order, so the
//! result does not depend on the thread count.

use super::Rng;
use crate::game::Game;
use ahash::AHashMap;
use rayon::prelude::*;
use rine_data::blocks::flag;
use rine_protocol::BlockPos;
use rine_world::{ChunkPos, MIN_Y};
use std::sync::OnceLock;

/// One sampled position whose block (and maybe fluid) ticks.
#[derive(Clone, Copy, Debug)]
pub enum Hit {
    Block(BlockPos, u16),
    /// A column chosen for `tickPrecipitation` (block x, z).
    Precip(i32, i32),
}

#[derive(Default)]
pub struct State {
    /// Chunk of each player that holds simulation tickets, by player slot.
    centers: AHashMap<usize, ChunkPos>,
    /// Number of players whose simulation square covers each chunk.
    refs: AHashMap<ChunkPos, u16>,
    /// Chunks with refs, sorted; rebuilt only when refs change.
    list: Vec<ChunkPos>,
    dirty: bool,
    radius: i32,
}

static SPEED: OnceLock<Option<usize>> = OnceLock::new();
static SPECTATORS: OnceLock<Option<usize>> = OnceLock::new();

fn rule_idx(cell: &OnceLock<Option<usize>>, name: &str) -> Option<usize> {
    *cell.get_or_init(|| crate::game::cmd::rules::GameRules::index(name))
}

fn int_rule(g: &Game, cell: &OnceLock<Option<usize>>, name: &str, default: i32) -> i32 {
    match rule_idx(cell, name).map(|i| g.rules.get(i)) {
        Some(crate::game::cmd::rules::Value::Int(v)) => v,
        Some(crate::game::cmd::rules::Value::Bool(b)) => b as i32,
        None => default,
    }
}

fn add_square(st: &mut State, c: ChunkPos, delta: i32) {
    let r = st.radius;
    for dx in -r..=r {
        for dz in -r..=r {
            let k = ChunkPos::new(c.x + dx, c.z + dz);
            if delta > 0 {
                *st.refs.entry(k).or_insert(0) += 1;
            } else if let Some(v) = st.refs.get_mut(&k) {
                *v -= 1;
                if *v == 0 {
                    st.refs.remove(&k);
                }
            }
        }
    }
    st.dirty = true;
}

/// Bring the ticking-chunk set up to date with player positions.
fn update_centers(g: &mut Game) {
    let radius = g.shared.config.simulation_distance as i32;
    let spectators = int_rule(g, &SPECTATORS, "spectators_generate_chunks", 1) != 0;
    let mut st = std::mem::take(&mut g.blocks.random);
    if st.radius != radius {
        st.centers.clear();
        st.refs.clear();
        st.radius = radius;
        st.dirty = true;
    }
    // players that left or stopped holding tickets
    let gone: Vec<usize> = st
        .centers
        .keys()
        .copied()
        .filter(|&s| match g.players.get(s).and_then(|p| p.as_ref()) {
            Some(p) => p.gamemode == 3 && !spectators,
            None => true,
        })
        .collect();
    for s in gone {
        if let Some(c) = st.centers.remove(&s) {
            add_square(&mut st, c, -1);
        }
    }
    for p in g.players.iter().flatten() {
        // `ChunkMap.skipPlayer`: spectators hold no tickets unless the rule allows
        if p.gamemode == 3 && !spectators {
            continue;
        }
        let c = ChunkPos::of_block(p.pos[0].floor() as i32, p.pos[2].floor() as i32);
        match st.centers.get(&p.slot).copied() {
            Some(old) if old == c => {}
            Some(old) => {
                add_square(&mut st, old, -1);
                add_square(&mut st, c, 1);
                st.centers.insert(p.slot, c);
            }
            None => {
                add_square(&mut st, c, 1);
                st.centers.insert(p.slot, c);
            }
        }
    }
    if st.dirty {
        st.list.clear();
        st.list.extend(st.refs.keys().copied());
        st.list.sort_unstable_by_key(|c| (c.x, c.z));
        st.dirty = false;
    }
    g.blocks.random = st;
}

/// Sample one chunk: the precipitation columns and randomly ticking blocks.
fn sample_chunk(c: &rine_world::Chunk, speed: i32, rng: &mut Rng, out: &mut Vec<Hit>) {
    let (bx, bz) = (c.pos.x * 16, c.pos.z * 16);
    // `tickChunk`: ice and snow
    for _ in 0..speed {
        if rng.next_int(48) == 0 {
            let r = rng.next_u32();
            out.push(Hit::Precip(bx + (r & 15) as i32, bz + ((r >> 8) & 15) as i32));
        }
    }
    let b = &rine_data::get().blocks;
    for (si, sec) in c.sections.iter().enumerate() {
        if sec.ticking == 0 {
            continue;
        }
        let y0 = MIN_Y + si as i32 * 16;
        for _ in 0..speed {
            let i = (rng.next_u32() & 4095) as usize;
            let st = sec.get(i);
            if b.has(st, flag::RANDOM_TICK) {
                out.push(Hit::Block(BlockPos::new(bx + (i & 15) as i32, y0 + (i >> 8) as i32, bz + ((i >> 4) & 15) as i32), st));
            }
        }
    }
}

/// Chunks per parallel work item.
const GROUP: usize = 32;

/// Random ticks for this game tick. Call between fluid ticks and block
/// events (vanilla `ServerLevel.tick` order). Stops under `/tick freeze`.
pub fn tick(g: &mut Game) {
    update_centers(g);
    let speed = int_rule(g, &SPEED, "random_tick_speed", 3);
    let list = std::mem::take(&mut g.blocks.random.list);
    if list.is_empty() || speed <= 0 {
        g.blocks.random.list = list;
        return;
    }
    let seed = g.blocks.seed ^ g.tick.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    let chunks = &g.world.chunks;
    let run = |grp: &[ChunkPos]| -> Vec<Hit> {
        let mut out = Vec::new();
        for cp in grp {
            if let Some(slot) = chunks.get(cp) {
                let mut rng = Rng::derive(seed, cp.x as i64, cp.z as i64);
                sample_chunk(&slot.chunk, speed, &mut rng, &mut out);
            }
        }
        out
    };
    let groups: Vec<Vec<Hit>> = if list.len() <= GROUP { vec![run(&list)] } else { list.par_chunks(GROUP).map(run).collect() };
    g.blocks.random.list = list;
    let mut rng = Rng::derive(seed, i64::MIN, 0);
    for hits in groups {
        if !hits.is_empty() {
            crate::game::redstone::random_ticks(g, &hits, &mut rng);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_hits_only_ticking_states_with_vanilla_rate() {
        rine_data::init();
        let b = &rine_data::get().blocks;
        let mut c = rine_world::Chunk::new(ChunkPos::new(0, 0));
        let wheat = b.default_state("wheat");
        // one section full of wheat, one with a single wheat
        c.sections[5].fill(wheat);
        c.sections[6].set(0, wheat);
        let mut rng = Rng::new(1);
        let mut out = Vec::new();
        let rounds = 2000;
        for _ in 0..rounds {
            sample_chunk(&c, 3, &mut rng, &mut out);
        }
        let full = out.iter().filter(|h| matches!(h, Hit::Block(p, _) if p.y >= MIN_Y + 80 && p.y < MIN_Y + 96)).count();
        let single = out.iter().filter(|h| matches!(h, Hit::Block(p, _) if p.y >= MIN_Y + 96 && p.y < MIN_Y + 112)).count();
        let precip = out.iter().filter(|h| matches!(h, Hit::Precip(..))).count();
        // full section: every attempt hits
        assert_eq!(full, rounds * 3);
        // one block in 4096: expected 1.46 hits over 6000 attempts
        assert!(single <= 10, "{single}");
        // 1/48 per attempt: expected 125
        assert!((80..180).contains(&precip), "{precip}");
        for h in &out {
            if let Hit::Block(p, st) = h {
                assert_eq!(*st, wheat);
                assert!((0..16).contains(&p.x) && (0..16).contains(&p.z));
            }
        }
    }

    #[test]
    fn squares_add_and_remove() {
        let mut st = State { radius: 2, ..Default::default() };
        add_square(&mut st, ChunkPos::new(0, 0), 1);
        add_square(&mut st, ChunkPos::new(1, 0), 1);
        assert_eq!(st.refs.len(), 30);
        assert_eq!(st.refs[&ChunkPos::new(0, 0)], 2);
        add_square(&mut st, ChunkPos::new(0, 0), -1);
        assert_eq!(st.refs.len(), 25);
        add_square(&mut st, ChunkPos::new(1, 0), -1);
        assert!(st.refs.is_empty());
    }
}
