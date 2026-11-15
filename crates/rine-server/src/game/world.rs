//! Loaded chunks, async generation/loading, block access and per-tick
//! batching of block changes into one packet per changed section.

use crate::out;
use ahash::{AHashMap, AHashSet};
use bytes::{Bytes, BytesMut};
use crossbeam_channel::{Receiver, Sender};
use rine_protocol::BlockPos;
use rine_world::{gen::Generator, storage::RegionStore, Chunk, ChunkPos, MAX_Y, MIN_Y};
use std::sync::Arc;

pub struct ChunkSlot {
    pub chunk: Box<Chunk>,
    /// Cached framed chunk packet and the chunk version it was built from.
    pub frame: Option<(u64, Bytes)>,
    /// Player slots that currently have this chunk loaded on their client.
    pub viewers: Vec<usize>,
    /// Tick when the last viewer left (for unloading).
    pub idle_since: u64,
    /// Unique per insertion into `chunks`, so a late lighting result for an
    /// unloaded and reloaded chunk is never applied to the new instance.
    pub uid: u64,
}

/// Lighting computed off the game thread for one chunk snapshot.
struct LightDone {
    pos: ChunkPos,
    uid: u64,
    version: u64,
    sections: Vec<(Option<Box<[u8; 2048]>>, Option<Box<[u8; 2048]>>)>,
}

pub struct Loaded {
    pub chunk: Box<Chunk>,
    pub frame: Bytes,
    /// Redstone blocks that need block entity data rebuilt (see
    /// `redstone::load_scan`).
    pub rs_scan: Vec<BlockPos>,
}

pub struct World {
    pub chunks: AHashMap<ChunkPos, ChunkSlot>,
    pending: AHashSet<ChunkPos>,
    tx: Sender<Loaded>,
    rx: Receiver<Loaded>,
    pub generator: Arc<Generator>,
    pub store: Arc<RegionStore>,
    /// Block changes this tick, keyed by section (cx, sy, cz).
    pub changes: AHashMap<(i32, i32, i32), Vec<(u16, u16)>>,
    /// Positions changed since the last neighbor pass (see `neighbors::settle`).
    pub touched: Vec<BlockPos>,
    /// Positions changed since the last fluid pass (see `fluids::tick`).
    pub fluid_touched: Vec<BlockPos>,
    /// Positions changed since the last entity tick (wakes resting TNT).
    pub wake: Vec<BlockPos>,
    /// Scheduled fluid ticks.
    pub fluids: super::fluids::Sched,
    /// Redstone engine state: scheduled block ticks, block events, moving pistons.
    pub redstone: super::redstone::Rs,
    /// Block changes made outside the redstone engine, waiting for their
    /// vanilla side effects (`redstone::process`).
    pub rs_events: Vec<super::redstone::RsEvent>,
    /// Set while the redstone engine writes, so its own writes are not queued.
    pub rs_mute: bool,
    /// Game time the redstone engine sees: the time of the last level tick
    /// (vanilla `getGameTime` between ticks, when packets are handled).
    pub rs_time: u64,
    /// Redstone positions from chunks loaded this tick (`Loaded::rs_scan`).
    pub rs_loaded: Vec<BlockPos>,
    /// Chunks whose lighting must be recomputed (after block changes).
    pub relight: AHashSet<ChunkPos>,
    /// Chunks with a lighting job running on the background pool.
    light_jobs: AHashSet<ChunkPos>,
    light_tx: Sender<LightDone>,
    light_rx: Receiver<LightDone>,
    next_uid: u64,
}

static BACKGROUND: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();

/// Run `f` on the background pool: chunk generation, lighting, chunk saves and
/// player data writes. It is deliberately not the global rayon pool. The game
/// thread's `par_iter` calls queue behind whatever sits in the global pool, so
/// a join that queued hundreds of chunk jobs stalled the next tick for as long
/// as they took. Half the cores (at least one) serve background work.
pub fn spawn_background(f: impl FnOnce() + Send + 'static) {
    BACKGROUND
        .get_or_init(|| {
            let n = std::thread::available_parallelism().map_or(2, |n| n.get()) / 2;
            rayon::ThreadPoolBuilder::new().num_threads(n.max(1)).thread_name(|i| format!("bg-{i}")).build().expect("background pool")
        })
        .spawn(f);
}

impl World {
    pub fn new(seed: i64, dir: &str) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (light_tx, light_rx) = crossbeam_channel::unbounded();
        Self {
            chunks: AHashMap::with_capacity(16384),
            pending: AHashSet::new(),
            tx,
            rx,
            generator: Arc::new(Generator::new(seed)),
            store: Arc::new(RegionStore::open(dir)),
            changes: AHashMap::new(),
            touched: Vec::new(),
            fluid_touched: Vec::new(),
            wake: Vec::new(),
            fluids: Default::default(),
            redstone: Default::default(),
            rs_events: Vec::new(),
            rs_mute: false,
            rs_time: 0,
            rs_loaded: Vec::new(),
            relight: AHashSet::new(),
            light_jobs: AHashSet::new(),
            light_tx,
            light_rx,
            next_uid: 0,
        }
    }

    /// Ask for a chunk to be loaded or generated on the worker pool.
    pub fn request(&mut self, pos: ChunkPos) {
        if self.chunks.contains_key(&pos) || !self.pending.insert(pos) {
            return;
        }
        let tx = self.tx.clone();
        let gen = self.generator.clone();
        let store = self.store.clone();
        spawn_background(move || {
            let mut chunk = match store.load(pos) {
                Some(c) => c,
                None => {
                    let mut c = gen.generate(pos);
                    rine_world::light::light_chunk(&mut c);
                    c
                }
            };
            chunk.dirty = chunk.dirty || false;
            let rs_scan = super::redstone::load_scan(&chunk);
            let mut b = BytesMut::with_capacity(16 * 1024);
            rine_world::encode::encode_chunk_packet(&chunk, &mut b);
            let frame = out::frame(b);
            let _ = tx.send(Loaded { chunk: Box::new(chunk), frame, rs_scan });
        });
    }

    /// Move finished chunks into the map. Returns their positions.
    pub fn collect_loaded(&mut self, tick: u64) -> Vec<ChunkPos> {
        let mut done = Vec::new();
        while let Ok(l) = self.rx.try_recv() {
            let pos = l.chunk.pos;
            self.pending.remove(&pos);
            let v = l.chunk.version;
            self.rs_loaded.extend_from_slice(&l.rs_scan);
            self.redstone.chunk_loaded(pos.x, pos.z, self.rs_time);
            self.next_uid += 1;
            let uid = self.next_uid;
            self.chunks.insert(pos, ChunkSlot { chunk: l.chunk, frame: Some((v, l.frame)), viewers: Vec::new(), idle_since: tick, uid });
            done.push(pos);
        }
        done
    }

    pub fn is_pending(&self, pos: &ChunkPos) -> bool {
        self.pending.contains(pos)
    }

    /// Framed chunk packet, re-encoding if the chunk changed since caching.
    pub fn chunk_frame(&mut self, pos: ChunkPos) -> Option<Bytes> {
        let slot = self.chunks.get_mut(&pos)?;
        let v = slot.chunk.version;
        match &slot.frame {
            Some((cv, f)) if *cv == v => Some(f.clone()),
            _ => {
                // A pending lighting job applies later and re-sends light to
                // the viewers; encode the current light now.
                let mut b = BytesMut::with_capacity(16 * 1024);
                rine_world::encode::encode_chunk_packet(&slot.chunk, &mut b);
                let f = out::frame(b);
                slot.frame = Some((slot.chunk.version, f.clone()));
                Some(f)
            }
        }
    }

    #[inline]
    pub fn get_block(&self, x: i32, y: i32, z: i32) -> u16 {
        if !(MIN_Y..=MAX_Y).contains(&y) {
            return 0;
        }
        match self.chunks.get(&ChunkPos::new(x >> 4, z >> 4)) {
            Some(s) => s.chunk.get(x & 15, y, z & 15),
            None => 0,
        }
    }

    pub fn is_loaded(&self, x: i32, z: i32) -> bool {
        self.chunks.contains_key(&ChunkPos::new(x >> 4, z >> 4))
    }

    /// Set a block and queue the client update. Returns the old state.
    pub fn set_block(&mut self, p: BlockPos, state: u16) -> u16 {
        if !(MIN_Y..=MAX_Y).contains(&p.y) {
            return 0;
        }
        let cp = ChunkPos::new(p.x >> 4, p.z >> 4);
        let Some(slot) = self.chunks.get_mut(&cp) else { return 0 };
        let old = slot.chunk.set(p.x & 15, p.y, p.z & 15, state);
        if old != state {
            let rel = (((p.x & 15) << 8) | ((p.z & 15) << 4) | (p.y & 15)) as u16;
            self.changes.entry((cp.x, p.y >> 4, cp.z)).or_default().push((rel, state));
            self.touched.push(p);
            self.fluid_touched.push(p);
            self.wake.push(p);
            if !self.rs_mute {
                self.rs_events.push(super::redstone::RsEvent::Set(p, old, state));
            }
            let b = &rine_data::get().blocks;
            let (so, sn) = (b.state(old), b.state(state));
            if so.light_emit != sn.light_emit || so.light_block != sn.light_block {
                self.relight.insert(cp);
            }
        }
        old
    }

    /// Encode this tick's block changes: one frame per section, plus who
    /// should receive it. Also refreshes lighting of changed chunks.
    pub fn drain_changes(&mut self) -> Vec<(Bytes, Vec<usize>)> {
        let mut outv = Vec::with_capacity(self.changes.len());
        let changes = std::mem::take(&mut self.changes);
        for ((cx, sy, cz), mut list) in changes {
            // keep the last write per position
            let mut seen = AHashSet::with_capacity(list.len());
            list.reverse();
            list.retain(|(rel, _)| seen.insert(*rel));
            let viewers = match self.chunks.get(&ChunkPos::new(cx, cz)) {
                Some(s) if !s.viewers.is_empty() => s.viewers.clone(),
                _ => continue,
            };
            let p = if list.len() == 1 {
                let (rel, st) = list[0];
                let x = cx * 16 + ((rel >> 8) & 15) as i32;
                let z = cz * 16 + ((rel >> 4) & 15) as i32;
                let y = sy * 16 + (rel & 15) as i32;
                out::block_update(BlockPos::new(x, y, z), st)
            } else {
                out::section_blocks(cx, sy, cz, &list)
            };
            outv.push((out::frame(p), viewers));
        }
        // Lighting runs on the background pool from a snapshot of each changed
        // chunk. Blocks go out now; the light arrives a tick or more later, and
        // is applied only if the chunk is still the same instance and version.
        while let Ok(done) = self.light_rx.try_recv() {
            self.light_jobs.remove(&done.pos);
            let Some(slot) = self.chunks.get_mut(&done.pos) else { continue };
            if slot.uid != done.uid {
                continue;
            }
            if slot.chunk.version != done.version {
                self.relight.insert(done.pos);
                continue;
            }
            for (s, (sky, bl)) in slot.chunk.sections.iter_mut().zip(done.sections) {
                s.sky = sky;
                s.block_light = bl;
            }
            slot.frame = None;
            if !slot.viewers.is_empty() {
                let mut b = BytesMut::new();
                b_light_update(&mut b, &slot.chunk);
                outv.push((out::frame(b), slot.viewers.clone()));
            }
        }
        let relight: Vec<ChunkPos> = self.relight.drain().collect();
        for cp in relight {
            let Some(slot) = self.chunks.get(&cp) else { continue };
            if self.light_jobs.contains(&cp) {
                self.relight.insert(cp);
                continue;
            }
            let mut tmp = Chunk::new(cp);
            tmp.sections = slot.chunk.sections.clone();
            let (uid, version) = (slot.uid, slot.chunk.version);
            self.light_jobs.insert(cp);
            let tx = self.light_tx.clone();
            spawn_background(move || {
                rine_world::light::light_chunk(&mut tmp);
                let sections = tmp.sections.into_iter().map(|s| (s.sky, s.block_light)).collect();
                let _ = tx.send(LightDone { pos: cp, uid, version, sections });
            });
        }
        outv
    }

    pub fn unload_idle(&mut self, tick: u64, keep: impl Fn(ChunkPos) -> bool) {
        let mut victims = Vec::new();
        for (p, s) in &self.chunks {
            if s.viewers.is_empty() && tick.saturating_sub(s.idle_since) > 20 * 30 && !keep(*p) {
                victims.push(*p);
            }
        }
        for p in victims {
            if let Some(mut s) = self.chunks.remove(&p) {
                self.redstone.chunk_unloaded(p.x, p.z, &mut s.chunk);
                if s.chunk.dirty {
                    let store = self.store.clone();
                    let chunk = s.chunk;
                    spawn_background(move || store.save(&chunk));
                }
            }
        }
    }

    pub fn save_all(&mut self) {
        use rayon::prelude::*;
        let store = self.store.clone();
        self.chunks.par_iter_mut().for_each(|(_, s)| {
            if s.chunk.dirty {
                store.save(&s.chunk);
                s.chunk.dirty = false;
            }
        });
        self.store.flush();
    }
}

/// light_update packet (id 49): VarInt chunkX, VarInt chunkZ, light data.
fn b_light_update(b: &mut BytesMut, c: &Chunk) {
    use rine_protocol::Writer;
    b.w_varint(rine_protocol::ids::play::C_LIGHT_UPDATE);
    b.w_varint(c.pos.x);
    b.w_varint(c.pos.z);
    rine_world::encode::write_light(c, b);
}
