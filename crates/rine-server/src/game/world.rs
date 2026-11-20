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
    pub block_ticks: Vec<(BlockPos, u16)>,
}

pub struct World {
    pub chunks: AHashMap<ChunkPos, ChunkSlot>,
    pending: AHashSet<ChunkPos>,
    /// Unloaded chunks whose background save has not finished: a request in that window reads them from
    /// here, since a generated chunk cannot be generated again (vanilla decoration).
    saving: Arc<parking_lot::Mutex<AHashMap<ChunkPos, Arc<Chunk>>>>,
    /// A background write of the decoration frontier is running.
    persisting: Arc<std::sync::atomic::AtomicBool>,
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
    /// Positions whose block entity was dropped by a block change; their
    /// typed container state is forgotten on the next container tick.
    pub be_removed: Vec<BlockPos>,
    /// Positions that got a new block entity (hoppers start ticking).
    pub be_added: Vec<BlockPos>,
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
    pub fn new(seed: i64, dir: &str, vanilla_vegetation: bool) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (light_tx, light_rx) = crossbeam_channel::unbounded();
        Self {
            chunks: AHashMap::with_capacity(16384),
            pending: AHashSet::new(),
            saving: Default::default(),
            persisting: Default::default(),
            tx,
            rx,
            generator: Arc::new(Generator::for_world(seed, vanilla_vegetation, dir)),
            store: Arc::new(RegionStore::open(dir)),
            changes: AHashMap::new(),
            touched: Vec::new(),
            fluid_touched: Vec::new(),
            wake: Vec::new(),
            fluids: Default::default(),
            redstone: Default::default(),
            rs_events: Vec::new(),
            be_removed: Vec::new(),
            be_added: Vec::new(),
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

    /// Spawn search over the saved chunks first, the generator for the rest.
    pub fn find_spawn(&self) -> (i32, i32, i32) {
        let store = self.store.clone();
        self.generator.find_spawn_with(move |pos| store.load(pos))
    }

    /// Ask for a chunk to be loaded or generated on the worker pool.
    pub fn request(&mut self, pos: ChunkPos) {
        if self.chunks.contains_key(&pos) || !self.pending.insert(pos) {
            return;
        }
        let tx = self.tx.clone();
        let gen = self.generator.clone();
        let store = self.store.clone();
        let saving = self.saving.clone();
        spawn_background(move || {
            let in_flight = saving.lock().get(&pos).map(|c| Chunk::clone(c));
            // `dirty` means "differs from disk": a loaded chunk, one whose queued write is
            // still running, and a freshly saved generated chunk are all clean. Otherwise
            // every save rewrote every loaded chunk.
            let (mut chunk, block_ticks) = match in_flight.or_else(|| store.load(pos)) {
                Some(mut c) => {
                    c.dirty = false;
                    (c, Vec::new())
                }
                None => {
                    let (mut c, block_ticks) = gen.generate_with_ticks(pos);
                    rine_world::light::light_chunk(&mut c);
                    // Saved here, off the tick. Vanilla decoration depends on request order,
                    // so such a chunk is never regenerated; the legacy generator's chunk
                    // would be written at its first save or unload anyway.
                    store.save(&c);
                    c.dirty = false;
                    if gen.saves_generated() {
                        gen.chunk_saved(pos);
                    }
                    (c, block_ticks)
                }
            };
            let rs_scan = super::redstone::load_scan(&chunk);
            let mut b = BytesMut::with_capacity(16 * 1024);
            rine_world::encode::encode_chunk_packet(&chunk, &mut b);
            let frame = out::frame(b);
            let _ = tx.send(Loaded { chunk: Box::new(chunk), frame, rs_scan, block_ticks });
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
            for (p, block) in l.block_ticks {
                self.redstone.schedule_at(p, block, self.rs_time + 1, super::redstone::engine::prio::NORMAL);
            }
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
            // `LevelChunk.setBlockState`: a different block drops the old
            // block entity (same block, other properties, keeps it)
            if !slot.chunk.block_entities.is_empty() && b.block_of(old).id != b.block_of(state).id {
                let (lx, lz) = ((p.x & 15) as u8, (p.z & 15) as u8);
                let before = slot.chunk.block_entities.len();
                slot.chunk.block_entities.retain(|e| !(e.x == lx && e.z == lz && e.y as i32 == p.y));
                if slot.chunk.block_entities.len() != before {
                    slot.chunk.dirty = true;
                    self.be_removed.push(p);
                }
            }
            // a block with a block entity gets an empty one (its system
            // fills in the data); same-block property changes keep the old one
            if b.block_of(old).id != b.block_of(state).id {
                if let Some(ty) = super::blockent::type_of_block(b.block_of(state).id) {
                    slot.chunk.block_entities.push(rine_world::chunk::BlockEntity {
                        x: (p.x & 15) as u8,
                        y: p.y as i16,
                        z: (p.z & 15) as u8,
                        type_id: ty,
                        nbt: super::blocks::signs::fresh_nbt(state),
                    });
                    slot.chunk.dirty = true;
                    self.be_added.push(p);
                }
            }
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
            // light is stored with the chunk; a save between the block change and this
            // result must not leave the old light on disk
            slot.chunk.dirty = true;
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
            super::blockinv::chunk_unloading(self, p);
            if let Some(mut s) = self.chunks.remove(&p) {
                self.redstone.chunk_unloaded(p.x, p.z, &mut s.chunk);
                if s.chunk.dirty {
                    self.queue_save(Arc::from(s.chunk));
                }
            }
        }
    }

    /// Write `chunk` on the background pool. `saving` holds the newest snapshot per
    /// position, and an entry exists exactly while one writer for it runs: a newer
    /// snapshot replaces the entry and that writer writes again, so two writes of one
    /// chunk never overlap and the last one always lands.
    fn queue_save(&self, chunk: Arc<Chunk>) {
        let pos = chunk.pos;
        if self.saving.lock().insert(pos, chunk).is_some() {
            return;
        }
        let store = self.store.clone();
        let saving = self.saving.clone();
        spawn_background(move || loop {
            let Some(c) = saving.lock().get(&pos).cloned() else { return };
            store.save(&c);
            let mut g = saving.lock();
            if g.get(&pos).is_some_and(|n| Arc::ptr_eq(n, &c)) {
                g.remove(&pos);
                return;
            }
        });
    }

    /// Saves the changed chunks, then writes the decoration frontier in the background.
    pub fn save_all(&mut self) {
        self.save_chunks();
        if self.persisting.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return;
        }
        let gen = self.generator.clone();
        let busy = self.persisting.clone();
        spawn_background(move || {
            gen.persist();
            busy.store(false, std::sync::atomic::Ordering::Release);
        });
    }

    /// `save_all` that returns once everything is on disk (server stop, `save-all flush`).
    pub fn save_all_blocking(&mut self) {
        self.save_chunks();
        while self.persisting.load(std::sync::atomic::Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        self.generator.persist();
        let saving = self.saving.clone();
        while !saving.lock().is_empty() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        self.store.flush();
    }

    /// Queue every changed chunk for a background write. The game thread only takes
    /// a snapshot of each (vanilla serializes the chunk on its main thread too and
    /// leaves the disk to its IO worker); `save-all` used to write them all here and
    /// stalled the tick for seconds after a burst of new chunks.
    fn save_chunks(&mut self) {
        super::blockinv::flush_all(self);
        let mut snaps = Vec::new();
        for s in self.chunks.values_mut() {
            if s.chunk.dirty {
                s.chunk.dirty = false;
                snaps.push(Arc::new(Chunk::clone(&s.chunk)));
            }
        }
        for c in snaps {
            self.queue_save(c);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn load(w: &mut World, pos: ChunkPos) {
        w.request(pos);
        while !w.chunks.contains_key(&pos) {
            w.collect_loaded(0);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// A chunk equal to what is on disk is clean, so a save writes only edited chunks
    /// (`save-all` used to rewrite every loaded chunk on the game thread).
    #[test]
    fn only_edited_chunks_are_saved() {
        rine_data::init();
        let dir = std::env::temp_dir().join(format!("rine-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let dir_s = dir.to_string_lossy().into_owned();
        let stone = rine_data::get().blocks.parse_state("minecraft:stone").expect("stone");
        let (a, b) = (ChunkPos::new(40, 40), ChunkPos::new(41, 40));
        let p = BlockPos::new(a.x * 16 + 3, 250, a.z * 16 + 5);
        {
            let mut w = World::new(1, &dir_s, false);
            load(&mut w, a);
            load(&mut w, b);
            // generated chunks are written by the worker and come back clean
            assert!(!w.chunks[&a].chunk.dirty && !w.chunks[&b].chunk.dirty);
            assert!(w.store.load(b).is_some());
            w.set_block(p, stone);
            assert!(w.chunks[&a].chunk.dirty && !w.chunks[&b].chunk.dirty);
            w.save_all();
            assert!(!w.chunks[&a].chunk.dirty);
            w.save_all_blocking();
        }
        let mut w = World::new(1, &dir_s, false);
        load(&mut w, a);
        assert!(!w.chunks[&a].chunk.dirty, "a loaded chunk is clean");
        assert_eq!(w.get_block(p.x, p.y, p.z), stone, "the edit reached the disk");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Snapshots queued while a write runs end with the newest one on disk.
    #[test]
    fn newest_queued_snapshot_lands() {
        rine_data::init();
        let dir = std::env::temp_dir().join(format!("rine-saveq-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let w = World::new(1, &dir.to_string_lossy(), false);
        let pos = ChunkPos::new(-7, 3);
        for i in 1..=50u16 {
            let mut c = Chunk::new(pos);
            c.set(0, 0, 0, i);
            w.queue_save(Arc::new(c));
        }
        while !w.saving.lock().is_empty() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(w.store.load(pos).expect("saved").get(0, 0, 0), 50);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
