//! Thread-safe chunk pipeline for the server: carved proto chunks (`Overworld::generate_carved` with their
//! worldgen heightmaps) and decorated final chunks.
//!
//! Decoration runs once per chunk, as in vanilla's FEATURES step: a chunk is decorated when the 3x3 protos
//! around it exist, its writes land in those nine chunks, and a chunk is final once all nine chunks around
//! it have been decorated. So a final chunk costs one decoration plus its proto, not the 25 decorations a
//! request-independent 5x5 window needs.
//!
//! The cost is the one vanilla pays too: the result at chunk borders depends on the order chunks are
//! requested in, and the partial frontier (chunks that neighbours decorated into but that are not final yet)
//! cannot be rebuilt from the seed. It lives in memory, spills to a `SlotStore` under memory pressure and is
//! written there by `persist`, which the server calls with every world save. A final chunk is handed over
//! once and the server must save it; asking for it again after it left the `finals` cache rebuilds it from
//! its proto without the features its neighbours wrote (see `full_chunk`). A handed-over chunk leaves a
//! final marker (in memory and, at `persist`, in the store) so its decoration never runs a second time.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};

use crate::feature::allow::generation;
use crate::feature::heightmap::WgHeights;
use crate::feature::region::Pos;
use crate::feature::Decorator;
use crate::overworld::{ChunkBlocks, Overworld};

/// Final chunks kept in memory after hand-over (a re-request inside this window is served from here).
pub const FINAL_CAPACITY: usize = 256;
/// Partial chunks kept in memory before the oldest unpinned ones spill to the store.
pub const SLOT_CAPACITY: usize = 1024;
/// Final markers kept in memory once they are in the store; older ones are read back from the store.
pub const FINAL_MARKERS: usize = 1 << 16;
const HEIGHT_VALUES: usize = 256 * crate::feature::heightmap::FINAL.len();

/// A carved proto chunk and its worldgen heights (computed once, on the terrain-end blocks).
pub struct Proto {
    pub chunk: ChunkBlocks,
    pub wg: WgHeights,
    heights: Vec<i32>,
}

/// A decorated chunk. `blocks.palette` is the decorator's output palette.
pub struct FullChunk {
    pub cx: i32,
    pub cz: i32,
    pub blocks: ChunkBlocks,
    pub block_ticks: Vec<(Pos, u16)>,
}

/// A partial chunk as written to a `SlotStore`. Block and tick states index `palette`, biomes index
/// `biome_palette`; both are name lists so a record survives palette changes between builds.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedSlot {
    /// The chunk was handed over as final: only this flag is meaningful, the vectors are empty.
    pub final_chunk: bool,
    pub decorated: bool,
    pub palette: Vec<String>,
    pub blocks: Vec<u16>,
    pub biome_palette: Vec<String>,
    pub biomes: Vec<u16>,
    pub wg: WgHeights,
    pub heights: Vec<i32>,
    pub ticks: Vec<(Pos, u16)>,
}

/// Durable storage of partial chunks, keyed by chunk position.
pub trait SlotStore: Send + Sync {
    fn load(&self, cx: i32, cz: i32) -> Option<SavedSlot>;
    /// Whether the record is a final marker; stores override it to skip decoding a partial record.
    fn is_final(&self, cx: i32, cz: i32) -> bool {
        self.load(cx, cz).is_some_and(|s| s.final_chunk)
    }
    fn save(&self, cx: i32, cz: i32, slot: &SavedSlot);
    fn delete(&self, cx: i32, cz: i32);
}

struct Slot {
    /// `None` while a decoration holds this chunk.
    blocks: Option<Vec<u16>>,
    biomes: Vec<u16>,
    wg: WgHeights,
    heights: Vec<i32>,
    decorated: bool,
    /// A decoration wrote through this chunk's window: the state is no longer a function of the seed.
    touched: bool,
    /// Changed since it was last written to the store.
    dirty: bool,
    /// Requests that need this chunk in memory (spilling skips it).
    pins: u32,
    ticks: Vec<(Pos, u16)>,
    /// Stand-in proto of a handed-over chunk, only read by a neighbour's decoration after a hard stop lost the
    /// frontier. Its writes are dropped (the real chunk is saved), it is never stored.
    ghost: bool,
}

#[derive(Default)]
struct State {
    slots: HashMap<(i32, i32), Slot>,
    /// Positions a thread is building (proto or store load); others wait for them.
    loading: HashSet<(i32, i32)>,
    /// Records being written by a spill, readable until the write is done.
    spilling: HashMap<(i32, i32), Arc<SavedSlot>>,
    /// Insertion order, for choosing spill victims (stale entries are skipped).
    order: VecDeque<(i32, i32)>,
    /// Handed-over finals: their decoration ran, they are never resident again. `true` once the final
    /// marker is in the store (with the server's own chunk save, at `persist`).
    finals: HashMap<(i32, i32), bool>,
    final_order: VecDeque<(i32, i32)>,
    /// Finals being built by a `full_chunk` call; a second request for the same chunk waits for it.
    requests: HashSet<(i32, i32)>,
    #[cfg(test)]
    decorations: HashMap<(i32, i32), u32>,
}

/// The overworld generator with its compiled decoration and the decoration scheduler.
pub struct Pipeline {
    seed: i64,
    overworld: Arc<Overworld>,
    decorator: Decorator,
    state: Mutex<State>,
    changed: Condvar,
    finals: Mutex<(HashMap<(i32, i32), Arc<FullChunk>>, VecDeque<(i32, i32)>)>,
    store: Option<Box<dyn SlotStore>>,
    capacity: usize,
    /// Decorator palette name -> index, for importing store records.
    palette_index: OnceLock<HashMap<String, u16>>,
    biome_names: Vec<String>,
}

impl Pipeline {
    /// Builds the generator for `seed`: the overworld, and the decoration of the ported placed features.
    pub fn new(seed: i64) -> Result<Self, String> {
        Self::with_features(Arc::new(Overworld::new(seed)), &generation)
    }

    pub fn with_features(overworld: Arc<Overworld>, allow: &dyn Fn(&str) -> bool) -> Result<Self, String> {
        let decorator = Decorator::production(&overworld, allow)?;
        Ok(Self::from_decorator(overworld, decorator))
    }

    /// The full vanilla decoration (`Decorator::vanilla`): unported placed features are no-ops, listed by
    /// [`Pipeline::skipped`].
    pub fn vanilla(overworld: Arc<Overworld>) -> Result<Self, String> {
        let decorator = Decorator::vanilla(&overworld)?;
        Ok(Self::from_decorator(overworld, decorator))
    }

    /// Placed features left out of the decoration (`name: error`).
    pub fn skipped(&self) -> &[String] {
        self.decorator.skipped()
    }

    fn from_decorator(overworld: Arc<Overworld>, decorator: Decorator) -> Self {
        let seed = overworld.seed();
        let biome_names = overworld.biome_names().to_vec();
        Pipeline {
            seed,
            overworld,
            decorator,
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            finals: Mutex::new((HashMap::new(), VecDeque::new())),
            store: None,
            capacity: SLOT_CAPACITY,
            palette_index: OnceLock::new(),
            biome_names,
        }
    }

    /// Keeps the partial frontier in `store` (spills and `persist`). Without a store partial chunks stay
    /// in memory.
    pub fn with_store(mut self, store: Box<dyn SlotStore>) -> Self {
        self.store = Some(store);
        self
    }

    #[cfg(test)]
    fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity;
        self
    }

    /// Output palette of the decorated chunks.
    pub fn palette(&self) -> &[String] {
        self.decorator.palette()
    }

    /// The carved proto chunk and its worldgen heights. Not cached: the scheduler keeps each chunk once.
    pub fn proto(&self, cx: i32, cz: i32) -> Proto {
        let (chunk, _mask) = self.overworld.generate_carved(cx, cz);
        let wg = self.decorator.wg_heights(&chunk);
        let heights = self.decorator.final_heights(&chunk);
        Proto { chunk, wg, heights }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The final (carved and decorated) chunk `(cx, cz)`. Decorates the chunks around it that are not
    /// decorated yet. The chunk is handed over once: it leaves the scheduler, and a request after it left the
    /// `finals` cache would rebuild it from the proto without its neighbours' features, so the caller saves it.
    pub fn full_chunk(&self, cx: i32, cz: i32) -> Arc<FullChunk> {
        let cached = || self.finals.lock().unwrap_or_else(|e| e.into_inner()).0.get(&(cx, cz)).cloned();
        if let Some(c) = cached() {
            return c;
        }
        {
            let mut g = self.lock();
            while g.requests.contains(&(cx, cz)) {
                g = self.changed.wait(g).unwrap_or_else(|e| e.into_inner());
            }
            // The request that just finished cached its result before releasing the chunk.
            if let Some(c) = cached() {
                return c;
            }
            g.requests.insert((cx, cz));
        }
        let chunk = self.build_final(cx, cz);
        let mut g = self.lock();
        g.requests.remove(&(cx, cz));
        self.changed.notify_all();
        chunk
    }

    /// `full_chunk` without the cache and request bookkeeping.
    fn build_final(&self, cx: i32, cz: i32) -> Arc<FullChunk> {
        if self.is_final(cx, cz) {
            // Handed over before and no longer cached: the neighbours' features are gone. The server saves
            // every generated chunk, so this only happens after a lost save.
            eprintln!("worldgen: chunk {cx},{cz} was already handed over; rebuilding it without decoration");
            let p = self.proto(cx, cz);
            let mut blocks = p.chunk;
            blocks.palette = self.decorator.palette().to_vec();
            return Arc::new(FullChunk { cx, cz, blocks, block_ticks: Vec::new() });
        }
        let area: Vec<(i32, i32)> = (-2..=2).flat_map(|dz| (-2..=2).map(move |dx| (cx + dx, cz + dz))).collect();
        let pinned = self.ensure(&area);
        for dz in -1..=1 {
            for dx in -1..=1 {
                self.decorate_once(cx + dx, cz + dz);
            }
        }
        let chunk = Arc::new(self.take(cx, cz, &pinned));
        let mut f = self.finals.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = f.0.get(&(cx, cz)) {
            return Arc::clone(existing);
        }
        while f.0.len() >= FINAL_CAPACITY {
            match f.1.pop_front() {
                Some(old) => {
                    f.0.remove(&old);
                }
                None => break,
            }
        }
        f.1.push_back((cx, cz));
        f.0.insert((cx, cz), Arc::clone(&chunk));
        chunk
    }

    fn is_final(&self, cx: i32, cz: i32) -> bool {
        let g = self.lock();
        if g.finals.contains_key(&(cx, cz)) {
            return true;
        }
        if g.slots.contains_key(&(cx, cz)) {
            return false;
        }
        drop(g);
        self.store.as_ref().is_some_and(|s| s.is_final(cx, cz))
    }

    /// The caller saved final chunk `(cx, cz)`: its final marker goes to the store now, so markers never lag
    /// behind saved chunks (a hard stop between saves would otherwise decorate it a second time).
    pub fn mark_stored(&self, cx: i32, cz: i32) {
        let Some(store) = self.store.as_ref() else { return };
        store.save(cx, cz, &final_marker());
        let mut g = self.lock();
        if let Some(stored) = g.finals.get_mut(&(cx, cz)) {
            *stored = true;
        }
    }

    /// Makes every position of `area` resident and pins the slots it holds (returned). Builds missing chunks outside the lock; a chunk
    /// another thread is building is waited for, so parallel requests share the work.
    fn ensure(&self, area: &[(i32, i32)]) -> Vec<(i32, i32)> {
        let mut g = self.lock();
        loop {
            let mut claimed = Vec::new();
            let mut waiting = false;
            for &p in area {
                if g.slots.contains_key(&p) || g.finals.contains_key(&p) {
                    continue;
                }
                if let Some(saved) = g.spilling.get(&p).cloned() {
                    let slot = self.import(&saved);
                    g.slots.insert(p, slot);
                    g.order.push_back(p);
                    continue;
                }
                if g.loading.contains(&p) {
                    waiting = true;
                } else {
                    g.loading.insert(p);
                    claimed.push(p);
                }
            }
            if claimed.is_empty() && !waiting {
                let mut pinned = Vec::with_capacity(area.len());
                for p in area {
                    if let Some(s) = g.slots.get_mut(p) {
                        s.pins += 1;
                        pinned.push(*p);
                    }
                }
                return pinned;
            }
            if claimed.is_empty() {
                g = self.changed.wait(g).unwrap_or_else(|e| e.into_inner());
                continue;
            }
            drop(g);
            let built = self.build(&claimed);
            g = self.lock();
            for (p, slot) in claimed.iter().copied().zip(built) {
                g.loading.remove(&p);
                match slot {
                    Some(slot) => {
                        g.slots.insert(p, slot);
                        g.order.push_back(p);
                    }
                    None => self.note_final(&mut g, p, true),
                }
            }
            self.changed.notify_all();
        }
    }

    /// Store record or fresh proto for each position. `None` for a chunk the store marks final.
    fn build(&self, positions: &[(i32, i32)]) -> Vec<Option<Slot>> {
        let one = |&(cx, cz): &(i32, i32)| -> Option<Slot> {
            if let Some(saved) = self.store.as_ref().and_then(|s| s.load(cx, cz)) {
                return (!saved.final_chunk).then(|| self.import(&saved));
            }
            let p = self.proto(cx, cz);
            Some(Slot {
                blocks: Some(p.chunk.blocks),
                biomes: p.chunk.biomes,
                wg: p.wg,
                heights: p.heights,
                decorated: false,
                touched: false,
                dirty: false,
                pins: 0,
                ticks: Vec::new(),
                ghost: false,
            })
        };
        // Sequential on purpose: callers already run in parallel (the server's chunk pool), and threads per
        // request multiply into hundreds of busy threads after a teleport, starving the tick thread.
        positions.iter().map(one).collect()
    }

    /// `ChunkGenerator.applyBiomeDecoration` for chunk `(cx, cz)` if it has not run yet. The nine chunks
    /// around it are resident and pinned by the caller.
    fn decorate_once(&self, cx: i32, cz: i32) {
        let window: [(i32, i32); 9] = std::array::from_fn(|i| (cx + (i % 3) as i32 - 1, cz + (i / 3) as i32 - 1));
        let mut g = self.lock();
        let mut ghosts: Vec<(i32, i32)> = Vec::new();
        loop {
            // A handed-over chunk was decorated before it became final.
            if g.finals.contains_key(&(cx, cz)) || g.slots[&(cx, cz)].decorated {
                for p in &ghosts {
                    if let Some(s) = g.slots.get_mut(p) {
                        s.pins -= 1;
                    }
                }
                return;
            }
            // A final's neighbours are all decorated, so a final in the window means a hard stop lost the
            // frontier. Decorate against stand-in protos of those finals rather than fail.
            let missing: Vec<(i32, i32)> = window.iter().copied().filter(|p| !g.slots.contains_key(p)).collect();
            if !missing.is_empty() {
                drop(g);
                eprintln!("worldgen: chunk {cx},{cz} decorates next to handed-over chunks {missing:?}; their part of its features is lost");
                let built: Vec<Slot> = missing
                    .iter()
                    .map(|&(x, z)| {
                        let p = self.proto(x, z);
                        Slot {
                            blocks: Some(p.chunk.blocks),
                            biomes: p.chunk.biomes,
                            wg: p.wg,
                            heights: p.heights,
                            decorated: true,
                            touched: false,
                            dirty: false,
                            pins: 0,
                            ticks: Vec::new(),
                            ghost: true,
                        }
                    })
                    .collect();
                g = self.lock();
                for (p, slot) in missing.into_iter().zip(built) {
                    let s = g.slots.entry(p).or_insert(slot);
                    s.pins += 1;
                    ghosts.push(p);
                }
                continue;
            }
            if window.iter().all(|p| g.slots[p].blocks.is_some()) {
                break;
            }
            g = self.changed.wait(g).unwrap_or_else(|e| e.into_inner());
        }
        let mut chunks = Vec::with_capacity(9);
        let mut wg = Vec::with_capacity(9);
        let mut heights = Vec::with_capacity(9 * HEIGHT_VALUES);
        for p in &window {
            let s = g.slots.get_mut(p).expect("pinned");
            chunks.push(ChunkBlocks {
                min_y: self.overworld_min_y(),
                height: self.overworld_height(),
                palette: Vec::new(),
                blocks: s.blocks.take().expect("free"),
                biome_palette: Vec::new(),
                biomes: std::mem::take(&mut s.biomes),
            });
            wg.push(s.wg.clone());
            heights.extend_from_slice(&s.heights);
        }
        #[cfg(test)]
        {
            *g.decorations.entry((cx, cz)).or_default() += 1;
        }
        drop(g);
        // `present_biomes` reads the biome list length from the first chunk.
        chunks[0].biome_palette = self.biome_names.clone();
        let scheduled = self.decorator.decorate_window(&self.overworld, self.seed, cx, cz, &mut chunks, &wg, &mut heights);
        let mut g = self.lock();
        for (j, (p, chunk)) in window.iter().zip(chunks).enumerate() {
            let s = g.slots.get_mut(p).expect("pinned");
            s.blocks = Some(chunk.blocks);
            s.biomes = chunk.biomes;
            s.heights.copy_from_slice(&heights[j * HEIGHT_VALUES..(j + 1) * HEIGHT_VALUES]);
            if !s.ghost {
                s.touched = true;
                s.dirty = true;
            }
        }
        for (pos, state) in scheduled {
            if let Some(s) = g.slots.get_mut(&(pos.0 >> 4, pos.2 >> 4)).filter(|s| !s.ghost) {
                s.ticks.push((pos, state));
            }
        }
        for p in &ghosts {
            if let Some(s) = g.slots.get_mut(p) {
                s.pins -= 1;
            }
        }
        g.slots.get_mut(&(cx, cz)).expect("pinned").decorated = true;
        self.changed.notify_all();
    }

    fn overworld_min_y(&self) -> i32 {
        self.overworld.min_y()
    }

    fn overworld_height(&self) -> i32 {
        self.overworld.height()
    }

    /// Removes the final chunk from the scheduler, unpins the positions `ensure` pinned (`pinned`; stand-ins
    /// that decoration created afterwards were never pinned by it) and spills if over capacity.
    fn take(&self, cx: i32, cz: i32, pinned: &[(i32, i32)]) -> FullChunk {
        let mut g = self.lock();
        let slot = g.slots.remove(&(cx, cz)).expect("pinned");
        for p in pinned {
            if let Some(s) = g.slots.get_mut(p) {
                s.pins -= 1;
            }
        }
        self.note_final(&mut g, (cx, cz), false);
        let victims = self.pick_victims(&mut g);
        drop(g);
        self.spill(victims);
        FullChunk {
            cx,
            cz,
            blocks: ChunkBlocks {
                min_y: self.overworld_min_y(),
                height: self.overworld_height(),
                palette: self.decorator.palette().to_vec(),
                blocks: slot.blocks.expect("all neighbours decorated"),
                biome_palette: self.biome_names.clone(),
                biomes: slot.biomes,
            },
            block_ticks: slot.ticks,
        }
    }

    /// Records a handed-over chunk; `stored` when its final marker is already in the store. Stored markers
    /// beyond `FINAL_MARKERS` are forgotten (the store answers for them).
    fn note_final(&self, g: &mut State, p: (i32, i32), stored: bool) {
        if g.finals.insert(p, stored).is_none() {
            g.final_order.push_back(p);
        }
        if self.store.is_none() {
            return;
        }
        let mut rounds = g.final_order.len();
        while g.finals.len() > FINAL_MARKERS && rounds > 0 {
            rounds -= 1;
            let Some(q) = g.final_order.pop_front() else { break };
            if g.finals.get(&q) == Some(&true) {
                g.finals.remove(&q);
            } else if g.finals.contains_key(&q) {
                g.final_order.push_back(q);
            }
        }
    }

    /// Oldest unpinned, free chunks until the scheduler is back to 3/4 of capacity. Untouched chunks are
    /// dropped (rebuilt from the seed when needed); touched ones move to `spilling` for the store. Without a
    /// store touched chunks stay.
    fn pick_victims(&self, g: &mut State) -> Vec<((i32, i32), Arc<SavedSlot>)> {
        let mut out = Vec::new();
        if g.slots.len() <= self.capacity {
            return out;
        }
        let target = self.capacity * 3 / 4;
        let mut keep = VecDeque::new();
        while g.slots.len() > target {
            let Some(p) = g.order.pop_front() else { break };
            let Some(s) = g.slots.get(&p) else { continue };
            if s.pins > 0 || s.blocks.is_none() || (s.touched && !s.ghost && self.store.is_none()) {
                keep.push_back(p);
                continue;
            }
            let s = g.slots.remove(&p).expect("present");
            if s.touched {
                let saved = Arc::new(self.export(&s));
                if s.dirty {
                    g.spilling.insert(p, Arc::clone(&saved));
                    out.push((p, saved));
                }
            }
        }
        // Skipped entries stay in age order ahead of the rest.
        while let Some(p) = keep.pop_back() {
            g.order.push_front(p);
        }
        out
    }

    fn spill(&self, victims: Vec<((i32, i32), Arc<SavedSlot>)>) {
        let Some(store) = self.store.as_ref() else { return };
        if victims.is_empty() {
            return;
        }
        for (p, saved) in &victims {
            store.save(p.0, p.1, saved);
        }
        let mut g = self.lock();
        for (p, saved) in victims {
            // A newer record may have replaced this one meanwhile.
            if g.spilling.get(&p).is_some_and(|s| Arc::ptr_eq(s, &saved)) {
                g.spilling.remove(&p);
            }
        }
    }

    /// Writes every changed partial chunk to the store and replaces the records of finals handed over since
    /// the last call with final markers. The server calls it after saving its own chunks, so both land at the same save point.
    pub fn persist(&self) {
        let Some(store) = self.store.as_ref() else { return };
        let mut g = self.lock();
        let mut batch = Vec::new();
        let mut keys: Vec<(i32, i32)> = g.slots.keys().copied().collect();
        keys.sort_unstable();
        for p in keys {
            let s = g.slots.get_mut(&p).expect("present");
            if s.touched && s.dirty && s.blocks.is_some() {
                s.dirty = false;
                let s = &g.slots[&p];
                batch.push((p, self.export(s)));
            }
        }
        let mut finals: Vec<(i32, i32)> = g.finals.iter().filter(|(_, &stored)| !stored).map(|(&p, _)| p).collect();
        finals.sort_unstable();
        drop(g);
        // Two writers: enough for the disk, little CPU taken from the tick and the generator.
        let threads = 2;
        let per = batch.len().div_ceil(threads).max(1);
        std::thread::scope(|s| {
            for part in batch.chunks(per) {
                s.spawn(move || {
                    for (p, saved) in part {
                        store.save(p.0, p.1, saved);
                    }
                });
            }
        });
        let marker = final_marker();
        for p in &finals {
            store.save(p.0, p.1, &marker);
        }
        // Only now may `note_final` forget these markers: they are on disk.
        let mut g = self.lock();
        for p in finals {
            if let Some(stored) = g.finals.get_mut(&p) {
                *stored = true;
            }
        }
    }

    fn export(&self, s: &Slot) -> SavedSlot {
        SavedSlot {
            final_chunk: false,
            decorated: s.decorated,
            palette: self.decorator.palette().to_vec(),
            blocks: s.blocks.clone().expect("free"),
            biome_palette: self.biome_names.clone(),
            biomes: s.biomes.clone(),
            wg: s.wg.clone(),
            heights: s.heights.clone(),
            ticks: s.ticks.clone(),
        }
    }

    fn import(&self, saved: &SavedSlot) -> Slot {
        let index = self.palette_index.get_or_init(|| {
            self.decorator.palette().iter().enumerate().map(|(i, n)| (n.clone(), i as u16)).collect()
        });
        let remap: Vec<u16> = saved.palette.iter().map(|n| index.get(n).copied().unwrap_or(0)).collect();
        let biome_remap: Vec<u16> = saved
            .biome_palette
            .iter()
            .map(|n| self.biome_names.iter().position(|b| b == n).unwrap_or(0) as u16)
            .collect();
        let map = |v: u16, r: &[u16]| r.get(v as usize).copied().unwrap_or(0);
        Slot {
            blocks: Some(saved.blocks.iter().map(|&v| map(v, &remap)).collect()),
            biomes: saved.biomes.iter().map(|&v| map(v, &biome_remap)).collect(),
            wg: saved.wg.clone(),
            heights: saved.heights.clone(),
            decorated: saved.decorated,
            touched: true,
            dirty: false,
            pins: 0,
            ticks: saved.ticks.iter().map(|&(p, v)| (p, map(v, &remap))).collect(),
            ghost: false,
        }
    }
}

fn final_marker() -> SavedSlot {
    SavedSlot {
        final_chunk: true,
        decorated: true,
        palette: Vec::new(),
        blocks: Vec::new(),
        biome_palette: Vec::new(),
        biomes: Vec::new(),
        wg: [Vec::new(), Vec::new()],
        heights: Vec::new(),
        ticks: Vec::new(),
    }
}

// The pipeline is shared across server threads.
const _: fn() = || {
    fn shared<T: Send + Sync>() {}
    shared::<Pipeline>();
};

#[cfg(test)]
mod tests {
    use super::*;
    fn pipe(seed: i64) -> Pipeline {
        Pipeline::vanilla(Arc::new(Overworld::new(seed))).expect("pipeline")
    }

    #[test]
    fn vanilla_decoration_runs_trees_and_ores() {
        let p = pipe(1);
        let skipped: Vec<&str> = p.skipped().iter().map(|s| s.split_once(": ").map_or(s.as_str(), |(name, _)| name)).collect();
        for name in ["minecraft:trees_plains", "minecraft:trees_birch", "minecraft:trees_jungle", "minecraft:trees_taiga",
            "minecraft:trees_savanna", "minecraft:trees_mangrove", "minecraft:trees_cherry", "minecraft:dark_forest_vegetation",
            "minecraft:ore_iron_upper", "minecraft:ore_diamond", "minecraft:ore_coal_upper", "minecraft:flower_plains"] {
            assert!(!skipped.contains(&name), "{name} skipped: {:?}", p.skipped());
        }
        println!("skipped {}: {skipped:?}", skipped.len());
    }

    /// Store in memory, shared between pipelines to model a restart.
    #[derive(Default, Clone)]
    struct MemStore(Arc<Mutex<HashMap<(i32, i32), SavedSlot>>>);

    impl SlotStore for MemStore {
        fn load(&self, cx: i32, cz: i32) -> Option<SavedSlot> {
            self.0.lock().unwrap().get(&(cx, cz)).cloned()
        }
        fn save(&self, cx: i32, cz: i32, slot: &SavedSlot) {
            self.0.lock().unwrap().insert((cx, cz), slot.clone());
        }
        fn delete(&self, cx: i32, cz: i32) {
            self.0.lock().unwrap().remove(&(cx, cz));
        }
    }

    const REQUESTS: [(i32, i32); 6] = [(0, 0), (1, 0), (0, 1), (3, 0), (1, 1), (2, 2)];

    /// Each chunk is decorated exactly once, and a final has all nine neighbours decorated.
    #[test]
    fn decorates_each_chunk_once() {
        let p = pipe(2024);
        for &(x, z) in &REQUESTS {
            p.full_chunk(x, z);
        }
        let g = p.lock();
        assert!(g.decorations.values().all(|&n| n == 1), "{:?}", g.decorations);
        for &(x, z) in &REQUESTS {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    assert_eq!(g.decorations.get(&(x + dx, z + dz)), Some(&1));
                }
            }
        }
        assert!(g.slots.values().all(|s| s.pins == 0));
    }

    /// The scheduler's finals match a hand-driven sequential decoration in the same order.
    #[test]
    fn matches_sequential_reference() {
        let p = pipe(77);
        let finals: Vec<_> = REQUESTS.iter().map(|&(x, z)| p.full_chunk(x, z)).collect();

        let r = pipe(77);
        let mut world: HashMap<(i32, i32), (ChunkBlocks, WgHeights, Vec<i32>, bool, Vec<(Pos, u16)>)> = HashMap::new();
        let mut out = Vec::new();
        let mut taken = HashSet::new();
        for &(cx, cz) in &REQUESTS {
            for dz in -2..=2 {
                for dx in -2..=2 {
                    if taken.contains(&(cx + dx, cz + dz)) {
                        continue;
                    }
                    world.entry((cx + dx, cz + dz)).or_insert_with(|| {
                        let pr = r.proto(cx + dx, cz + dz);
                        (pr.chunk, pr.wg, pr.heights, false, Vec::new())
                    });
                }
            }
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let (nx, nz) = (cx + dx, cz + dz);
                    if taken.contains(&(nx, nz)) || world[&(nx, nz)].3 {
                        continue;
                    }
                    let win: Vec<(i32, i32)> = (0..9).map(|i| (nx + i % 3 - 1, nz + i / 3 - 1)).collect();
                    let mut chunks: Vec<ChunkBlocks> = win.iter().map(|q| world[q].0.clone()).collect();
                    let wg: Vec<WgHeights> = win.iter().map(|q| world[q].1.clone()).collect();
                    let mut heights: Vec<i32> = win.iter().flat_map(|q| world[q].2.clone()).collect();
                    let ticks = r.decorator.decorate_window(&r.overworld, r.seed, nx, nz, &mut chunks, &wg, &mut heights);
                    for (j, q) in win.iter().enumerate() {
                        let e = world.get_mut(q).unwrap();
                        e.0.blocks = std::mem::take(&mut chunks[j].blocks);
                        e.2.copy_from_slice(&heights[j * HEIGHT_VALUES..(j + 1) * HEIGHT_VALUES]);
                    }
                    for (pos, st) in ticks {
                        world.get_mut(&(pos.0 >> 4, pos.2 >> 4)).unwrap().4.push((pos, st));
                    }
                    world.get_mut(&(nx, nz)).unwrap().3 = true;
                }
            }
            let e = world.remove(&(cx, cz)).unwrap();
            taken.insert((cx, cz));
            out.push((e.0.blocks, e.0.biomes, e.4));
        }
        for (f, (blocks, biomes, ticks)) in finals.iter().zip(out) {
            assert!(f.blocks.blocks == blocks, "blocks of {},{}", f.cx, f.cz);
            assert_eq!(f.blocks.biomes, biomes);
            assert_eq!(f.block_ticks, ticks);
        }
    }

    /// A restart in the middle (persist, new pipeline on the same store) gives the same finals as one run.
    #[test]
    fn persist_survives_restart() {
        let one = pipe(5);
        let expect: Vec<_> = REQUESTS.iter().map(|&(x, z)| one.full_chunk(x, z)).collect();

        let store = MemStore::default();
        let a = pipe(5).with_store(Box::new(store.clone()));
        let mut got: Vec<_> = REQUESTS[..3].iter().map(|&(x, z)| a.full_chunk(x, z)).collect();
        a.persist();
        assert!(store.0.lock().unwrap()[&REQUESTS[0]].final_chunk, "a handed-over final leaves a marker");
        drop(a);
        let b = pipe(5).with_store(Box::new(store.clone()));
        got.extend(REQUESTS[3..].iter().map(|&(x, z)| b.full_chunk(x, z)));
        for (e, g) in expect.iter().zip(&got) {
            assert!(e.blocks.blocks == g.blocks.blocks, "blocks of {},{}", e.cx, e.cz);
            assert_eq!(e.block_ticks, g.block_ticks);
        }
    }

    /// A hard stop after the world saved a chunk (no `persist`): its marker is already stored, so the next
    /// run does not decorate it again.
    #[test]
    fn marker_survives_a_hard_stop() {
        let store = MemStore::default();
        let a = pipe(8).with_store(Box::new(store.clone()));
        a.full_chunk(0, 0);
        a.mark_stored(0, 0);
        drop(a);
        let b = pipe(8).with_store(Box::new(store.clone()));
        b.full_chunk(1, 0);
        assert!(!b.lock().decorations.contains_key(&(0, 0)), "decorated twice");
        // (1, 0) itself was decorated against a stand-in of (0, 0), which never becomes a stored record.
        assert!(b.lock().slots.get(&(0, 0)).is_none_or(|s| s.ghost && !s.touched));
    }

    /// Spilling under a tiny capacity changes nothing in the result.
    #[test]
    fn spill_is_transparent() {
        let one = pipe(9);
        let expect: Vec<_> = REQUESTS.iter().map(|&(x, z)| one.full_chunk(x, z)).collect();
        let store = MemStore::default();
        let small = pipe(9).with_store(Box::new(store.clone())).with_capacity(30);
        let got: Vec<_> = REQUESTS.iter().map(|&(x, z)| small.full_chunk(x, z)).collect();
        assert!(small.lock().slots.len() <= 40);
        assert!(!store.0.lock().unwrap().is_empty(), "something spilled");
        for (e, g) in expect.iter().zip(&got) {
            assert!(e.blocks.blocks == g.blocks.blocks, "blocks of {},{}", e.cx, e.cz);
        }
    }

    /// Many threads asking for the same chunks get one build each, never a second one without decoration.
    #[test]
    fn same_chunk_from_many_threads() {
        let p = pipe(12);
        let got: Vec<Arc<FullChunk>> = std::thread::scope(|s| {
            let hs: Vec<_> = (0..6).map(|_| s.spawn(|| p.full_chunk(4, 4))).collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(got.iter().all(|c| Arc::ptr_eq(c, &got[0])));
        assert!(p.lock().decorations.values().all(|&n| n == 1));
    }

    /// Requests from many threads finish, each chunk is decorated once.
    #[test]
    fn parallel_requests() {
        let p = pipe(31);
        std::thread::scope(|s| {
            for t in 0..4 {
                let p = &p;
                s.spawn(move || {
                    for i in 0..4 {
                        p.full_chunk(t * 2 + i % 2, i / 2 * 3);
                    }
                });
            }
        });
        assert!(p.lock().decorations.values().all(|&n| n == 1));
    }

    #[test]
    #[ignore]
    fn timing() {
        let p = pipe(2024);
        let _ = p.full_chunk(-20, -20);
        let t = std::time::Instant::now();
        let n = 64;
        for i in 0..n {
            p.full_chunk(i % 8, i / 8);
        }
        println!("scheduler: {:.2} ms/chunk over {n} sequential requests", t.elapsed().as_secs_f64() * 1e3 / n as f64);
    }
}
