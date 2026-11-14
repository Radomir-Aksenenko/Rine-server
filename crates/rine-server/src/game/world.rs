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
}

pub struct Loaded {
    pub chunk: Box<Chunk>,
    pub frame: Bytes,
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
    /// Scheduled fluid ticks.
    pub fluids: super::fluids::Sched,
    /// Chunks whose lighting must be recomputed (after block changes).
    pub relight: AHashSet<ChunkPos>,
}

impl World {
    pub fn new(seed: i64, dir: &str) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
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
            fluids: Default::default(),
            relight: AHashSet::new(),
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
        rayon::spawn(move || {
            let mut chunk = match store.load(pos) {
                Some(c) => c,
                None => {
                    let mut c = gen.generate(pos);
                    rine_world::light::light_chunk(&mut c);
                    c
                }
            };
            chunk.dirty = chunk.dirty || false;
            let mut b = BytesMut::with_capacity(16 * 1024);
            rine_world::encode::encode_chunk_packet(&chunk, &mut b);
            let frame = out::frame(b);
            let _ = tx.send(Loaded { chunk: Box::new(chunk), frame });
        });
    }

    /// Move finished chunks into the map. Returns their positions.
    pub fn collect_loaded(&mut self, tick: u64) -> Vec<ChunkPos> {
        let mut done = Vec::new();
        while let Ok(l) = self.rx.try_recv() {
            let pos = l.chunk.pos;
            self.pending.remove(&pos);
            let v = l.chunk.version;
            self.chunks.insert(pos, ChunkSlot { chunk: l.chunk, frame: Some((v, l.frame)), viewers: Vec::new(), idle_since: tick });
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
                if self.relight.remove(&pos) {
                    rine_world::light::light_chunk(&mut slot.chunk);
                }
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
        // Lighting: recompute changed chunks and send a full chunk-light
        // refresh via a fresh chunk packet only when light really changed.
        let relight: Vec<ChunkPos> = self.relight.drain().collect();
        for cp in relight {
            if let Some(slot) = self.chunks.get_mut(&cp) {
                rine_world::light::light_chunk(&mut slot.chunk);
                if slot.viewers.is_empty() {
                    continue;
                }
                let mut b = BytesMut::new();
                b_light_update(&mut b, &slot.chunk);
                outv.push((out::frame(b), slot.viewers.clone()));
            }
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
            if let Some(s) = self.chunks.remove(&p) {
                if s.chunk.dirty {
                    let store = self.store.clone();
                    let chunk = s.chunk;
                    rayon::spawn(move || store.save(&chunk));
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
