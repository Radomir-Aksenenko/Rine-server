//! Cached, thread-safe chunk pipeline for the server: carved proto chunks (`Overworld::generate_carved`
//! with their worldgen heightmaps) and decorated final chunks.
//!
//! Decoration semantics are the golden driver's (`tests/feature_golden.rs`): the final chunk `(cx, cz)` is
//! the centre of a 5x5 raster-order decoration of the chunks `(cx-2..=cx+2, cz-2..=cz+2)`, each decoration
//! seeing its 3x3 window of protos or of earlier decorated chunks. That is a bounded window, so every
//! request is independent of the others and of request order: results are deterministic and safe to
//! compute in parallel. Vanilla's real cross-chunk order follows the chunk scheduler and is not raster; the
//! remaining golden mismatches come from that (see the report in the crate notes).
//!
//! Caches: protos and finals live in bounded maps evicted first-in first-out (an LRU approximation that
//! needs no timestamps). A cache miss computes outside the lock; two threads may compute the same proto
//! once each, and the second insert is dropped, so results never depend on the race.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use crate::feature::allow::ported_only;
use crate::feature::heightmap::WgHeights;
use crate::feature::Decorator;
use crate::overworld::{ChunkBlocks, Overworld};

/// Proto chunks kept in memory (each is about 100 KiB of block ids plus heights).
pub const PROTO_CAPACITY: usize = 1024;
/// Final chunks kept in memory.
pub const FINAL_CAPACITY: usize = 256;

/// Radius of the decoration window: the 5x5 ring of final decorations around the requested chunk.
const RING: i32 = 2;
/// Protos needed around a requested chunk: the ring plus one chunk on each side.
const AREA: i32 = RING + 1;

/// A carved proto chunk and its worldgen heights (computed once, on the terrain-end blocks).
pub struct Proto {
    pub chunk: ChunkBlocks,
    pub wg: WgHeights,
}

/// A decorated chunk. `blocks.palette` is the decorator's output palette.
pub struct FullChunk {
    pub cx: i32,
    pub cz: i32,
    pub blocks: ChunkBlocks,
}

/// Bounded map with first-in first-out eviction.
struct Cache<T> {
    cap: usize,
    inner: Mutex<(HashMap<(i32, i32), T>, VecDeque<(i32, i32)>)>,
}

impl<T: Clone> Cache<T> {
    fn new(cap: usize) -> Self {
        Cache { cap, inner: Mutex::new((HashMap::new(), VecDeque::new())) }
    }

    fn get(&self, key: (i32, i32)) -> Option<T> {
        self.inner.lock().unwrap().0.get(&key).cloned()
    }

    /// Inserts unless the key is present, evicting the oldest entries first. Returns the stored value.
    fn insert(&self, key: (i32, i32), value: T) -> T {
        let mut guard = self.inner.lock().unwrap();
        let (map, order) = &mut *guard;
        if let Some(existing) = map.get(&key) {
            return existing.clone();
        }
        while map.len() >= self.cap {
            match order.pop_front() {
                Some(old) => {
                    map.remove(&old);
                }
                None => break,
            }
        }
        order.push_back(key);
        map.insert(key, value.clone());
        value
    }
}

/// The overworld generator with its compiled decoration and the caches.
pub struct Pipeline {
    seed: i64,
    overworld: Overworld,
    decorator: Decorator,
    protos: Cache<Arc<Proto>>,
    finals: Cache<Arc<FullChunk>>,
}

impl Pipeline {
    /// Builds the generator for `seed`: the overworld, and the decoration of the ported placed features.
    pub fn new(seed: i64) -> Result<Self, String> {
        let overworld = Overworld::new(seed);
        let decorator = Decorator::new(&overworld, &ported_only)?;
        Ok(Pipeline {
            seed,
            overworld,
            decorator,
            protos: Cache::new(PROTO_CAPACITY),
            finals: Cache::new(FINAL_CAPACITY),
        })
    }

    /// Output palette of the decorated chunks.
    pub fn palette(&self) -> &[String] {
        self.decorator.palette()
    }

    /// The carved proto chunk and its worldgen heights, cached.
    pub fn proto(&self, cx: i32, cz: i32) -> Arc<Proto> {
        if let Some(p) = self.protos.get((cx, cz)) {
            return p;
        }
        let (chunk, _mask) = self.overworld.generate_carved(cx, cz);
        let wg = self.decorator.wg_heights(&chunk);
        self.protos.insert((cx, cz), Arc::new(Proto { chunk, wg }))
    }

    /// The final (carved and decorated) chunk `(cx, cz)`, cached.
    pub fn full_chunk(&self, cx: i32, cz: i32) -> Arc<FullChunk> {
        if let Some(c) = self.finals.get((cx, cz)) {
            return c;
        }
        let chunk = Arc::new(self.decorate_centre(cx, cz));
        self.finals.insert((cx, cz), chunk)
    }

    /// The raster decoration of the ring around `(cx, cz)`, then the centre chunk.
    fn decorate_centre(&self, cx: i32, cz: i32) -> FullChunk {
        let side = (2 * AREA + 1) as usize;
        let mut area: Vec<ChunkBlocks> = Vec::with_capacity(side * side);
        let mut area_wg: Vec<WgHeights> = Vec::with_capacity(side * side);
        for dz in -AREA..=AREA {
            for dx in -AREA..=AREA {
                let p = self.proto(cx + dx, cz + dz);
                area.push(p.chunk.clone());
                area_wg.push(p.wg.clone());
            }
        }
        let index = |dx: i32, dz: i32| ((dz + AREA) as usize) * side + (dx + AREA) as usize;
        let mut window: Vec<ChunkBlocks> = Vec::with_capacity(9);
        let mut window_wg: Vec<WgHeights> = Vec::with_capacity(9);
        for dz in -RING..=RING {
            for dx in -RING..=RING {
                // Move the 3x3 window out of the area, decorate, and move it back.
                for wz in -1..=1 {
                    for wx in -1..=1 {
                        let i = index(dx + wx, dz + wz);
                        window.push(std::mem::replace(&mut area[i], empty_chunk()));
                        window_wg.push(area_wg[i].clone());
                    }
                }
                self.decorator
                    .decorate(&self.overworld, self.seed, cx + dx, cz + dz, &mut window, &window_wg)
                    .expect("decoration of a ported window");
                let mut back = window.drain(..);
                for wz in -1..=1 {
                    for wx in -1..=1 {
                        area[index(dx + wx, dz + wz)] = back.next().expect("window chunk");
                    }
                }
                window_wg.clear();
            }
        }
        let mut blocks = area.swap_remove(index(0, 0));
        blocks.palette = self.decorator.palette().to_vec();
        FullChunk { cx, cz, blocks }
    }
}

// The pipeline is shared across server threads.
const _: fn() = || {
    fn shared<T: Send + Sync>() {}
    shared::<Pipeline>();
};

/// A chunk with no blocks, used as the placeholder of a window slot while the window is decorated.
fn empty_chunk() -> ChunkBlocks {
    ChunkBlocks { min_y: 0, height: 0, palette: Vec::new(), blocks: Vec::new(), biome_palette: Vec::new(), biomes: Vec::new() }
}
