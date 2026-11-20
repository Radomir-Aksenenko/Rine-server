//! The decoration view of a 3x3 chunk region (`WorldGenRegion` with write radius 1): block reads and
//! writes over nine chunks, the heightmaps of those chunks (`heightmap.rs` documents which are frozen and
//! which are maintained), and the biome lookups used by `biome` placement filters.

use std::collections::HashMap;

use super::heightmap::{self, Kind, WgHeights, FINAL};
use super::state::{Fluid, Palette};
use crate::overworld::{ChunkBlocks, Overworld};

/// Sorted tag names of the bundle: tag ids are indices into this list.
pub fn tag_names(block_tags: &HashMap<String, Vec<String>>) -> Vec<String> {
    let mut names: Vec<String> = block_tags.keys().cloned().collect();
    names.sort();
    names
}

/// Block tag membership, block identity, fluid kind and heightmap predicates per palette index.
pub struct Tags {
    names: Vec<String>,
    words: usize,
    bits: Vec<u64>,
    air: Vec<bool>,
    block_of: Vec<u16>,
    fluid_of: Vec<Fluid>,
    /// `Heightmap.Types` predicates as bit masks (see `heightmap::Kind::bit`).
    opaque: Vec<u8>,
    shapes: Vec<[u8; 4]>,
    /// Vegetation survival tables (`vegetation::VegTable`), installed after the palette is final.
    pub(crate) veg: super::vegetation::VegTable,
}

impl Tags {
    /// Builds the tables for every palette entry. `block_tags` maps tag name to block ids.
    pub fn new(palette: &Palette, block_tags: &HashMap<String, Vec<String>>) -> Self {
        let names = tag_names(block_tags);
        let words = names.len().div_ceil(64).max(1);
        // Membership per block first (tags name blocks, not states), then copied to every state.
        let mut block_bits = vec![0u64; palette.block_count() * words];
        for (t, tag) in names.iter().enumerate() {
            for b in &block_tags[tag] {
                if let Some(bi) = palette.block_id(b) {
                    block_bits[bi as usize * words + t / 64] |= 1 << (t % 64);
                }
            }
        }
        let block_of = palette.block_of().to_vec();
        let mut bits = vec![0u64; block_of.len() * words];
        let mut air = vec![false; block_of.len()];
        for (i, &b) in block_of.iter().enumerate() {
            bits[i * words..(i + 1) * words].copy_from_slice(&block_bits[b as usize * words..(b as usize + 1) * words]);
            air[i] = matches!(palette.block_name(b), "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air");
        }
        let fluid_of = palette.fluid_of().to_vec();
        let tag_of = |name: &str| names.binary_search_by(|n| n.as_str().cmp(name)).ok();
        let motion_tag = tag_of("minecraft:blocks_motion_in_heightmap");
        let no_leaves_tag = tag_of("minecraft:blocks_motion_in_heightmap_no_leaves");
        let has = |s: usize, t: Option<usize>| t.is_some_and(|t| bits[s * words + t / 64] & (1 << (t % 64)) != 0);
        let n = block_of.len();
        let motion: Vec<bool> = (0..n).map(|s| has(s, motion_tag)).collect();
        let no_leaves: Vec<bool> = (0..n).map(|s| has(s, no_leaves_tag)).collect();
        let fluid_nonempty: Vec<bool> = fluid_of.iter().map(|&f| f != Fluid::Empty).collect();
        let opaque = heightmap::opaque_masks(&air, &fluid_nonempty, &motion, &no_leaves);
        let shapes = super::shape::palette_shapes(palette);
        Tags { names, words, bits, air, block_of, fluid_of, opaque, shapes, veg: Default::default() }
    }

    /// Tag id for a tag name, if the bundle has it.
    pub fn id(&self, name: &str) -> Option<u16> {
        self.names.binary_search_by(|n| n.as_str().cmp(name)).ok().map(|i| i as u16)
    }

    #[inline]
    pub fn has(&self, blk: u16, tag: u16) -> bool {
        let t = tag as usize;
        self.bits[blk as usize * self.words + t / 64] & (1 << (t % 64)) != 0
    }

    #[inline]
    pub fn is_air(&self, blk: u16) -> bool {
        self.air[blk as usize]
    }

    /// `Heightmap.Types.isOpaque` of a palette state.
    #[inline]
    pub fn is_opaque(&self, blk: u16, kind: Kind) -> bool {
        self.opaque[blk as usize] & kind.bit() != 0
    }

    /// `BlockState.is(block)` for a block index (`Palette::block`).
    #[inline]
    pub fn is_block(&self, blk: u16, block: u16) -> bool {
        self.block_of[blk as usize] == block
    }

    /// Block index of a palette state.
    #[inline]
    pub fn block_of(&self, blk: u16) -> u16 {
        self.block_of[blk as usize]
    }

    /// `FluidState.getType()` of a palette state.
    #[inline]
    pub fn fluid(&self, blk: u16) -> Fluid {
        self.fluid_of[blk as usize]
    }

    #[inline]
    pub fn face_full(&self, state: u16, face: usize) -> bool {
        self.shapes[state as usize][0] & (1 << face) != 0
    }

    #[inline]
    pub fn sturdy(&self, state: u16, face: usize) -> bool {
        self.shapes[state as usize][1] & (1 << face) != 0
    }

    #[inline]
    pub fn full_collision(&self, state: u16) -> bool {
        self.shapes[state as usize][0] & 64 != 0
    }

    #[inline]
    pub fn solid(&self, state: u16) -> bool {
        self.shapes[state as usize][1] & 64 != 0
    }

    #[inline]
    pub fn replaceable(&self, state: u16) -> bool {
        self.shapes[state as usize][0] & 128 != 0
    }

    pub fn solid_render(&self, state: u16) -> bool {
        self.shapes[state as usize][2] & 1 != 0
    }

    pub fn liquid(&self, state: u16) -> bool {
        self.shapes[state as usize][2] & 2 != 0
    }

    pub fn collision_full_up(&self, state: u16) -> bool {
        self.shapes[state as usize][2] & 4 != 0
    }

    pub fn sturdy_down_center(&self, state: u16) -> bool {
        self.shapes[state as usize][2] & 8 != 0
    }

    pub fn fluid_full(&self, state: u16) -> bool {
        self.shapes[state as usize][2] & 16 != 0
    }

    pub(crate) fn snowy(&self, state: u16) -> Option<u16> {
        self.veg.snowy[state as usize]
    }

    #[inline]
    pub(super) fn face_occluding(&self, state: u16, face: usize) -> bool {
        self.shapes[state as usize][3] & (1 << face) != 0
    }

    #[inline]
    pub(super) fn collision_top(&self, state: u16) -> bool {
        self.shapes[state as usize][3] & 64 != 0
    }
}

pub type Pos = (i32, i32, i32);
pub type BlockWrite = (i32, i32, i32, u16, Option<u32>);

/// Deferred effects emitted by a FEATURES pass, in emission order within each list.
#[derive(Default)]
pub struct DecorationEffects {
    /// Block identities are represented by their palette state.
    pub block_ticks: Vec<(Pos, u16)>,
    /// Fluid identities retain the source/flowing distinction.
    pub fluid_ticks: Vec<(Pos, Fluid)>,
    pub post_processing: Vec<Pos>,
}

/// Live placement performs each write immediately, including its neighbor updates.
pub trait LiveRegion {
    fn block(&self, pos: Pos) -> u16;
    fn set(&mut self, pos: Pos, state: u16, flags: u32) -> bool;
    fn destroy(&mut self, pos: Pos);
    fn schedule(&mut self, pos: Pos, state: u16);
    fn raw_brightness(&self, _pos: Pos) -> u8 {
        15
    }
}

struct DecorationBiomes<'a> {
    overworld: &'a Overworld,
    quarts: [&'a [u16]; 9],
}

/// Mutable 3x3 chunk region around the chunk being decorated.
pub struct Region<'a> {
    pub tags: &'a Tags,
    live: Option<&'a mut dyn LiveRegion>,
    level_random: Option<super::rng::WorldgenRng>,
    effects: DecorationEffects,
    /// `placed_in_biome[biome][placed]`: `Biome.hasFeature` over the biome's full feature lists.
    placed_in_biome: &'a [Vec<bool>],
    /// `None` for the grower views (`grower_region`), which have no overworld biomes.
    biomes: Option<DecorationBiomes<'a>>,
    min_y: i32,
    height: i32,
    cx: i32,
    cz: i32,
    /// Nine chunks in raster order: index `(dz + 1) * 3 + (dx + 1)`.
    blocks: Vec<Vec<u16>>,
    /// Frozen worldgen heights, `[(chunk * 2 + kind) * 256 + column]`, kind 0 WORLD_SURFACE_WG, 1 OCEAN_FLOOR_WG.
    wg: Vec<i32>,
    /// Maintained final heights, `[(chunk * 4 + kind) * 256 + column]`, kind in `heightmap::FINAL` order.
    fin: Vec<i32>,
    /// Ordered writes since `start_log`. A missing flag uses the caller's update flags.
    log: Option<Vec<BlockWrite>>,
}

impl<'a> Region<'a> {
    pub fn live(tags: &'a Tags, min_y: i32, height: i32, live: &'a mut dyn LiveRegion) -> Self {
        Self {
            tags, live: Some(live), level_random: None, effects: DecorationEffects::default(), min_y, height, cx: 0, cz: 0,
            placed_in_biome: &[], biomes: None, blocks: Vec::new(), wg: Vec::new(),
            fin: Vec::new(), log: None,
        }
    }

    pub(crate) fn set_level_random(&mut self, rng: super::rng::WorldgenRng) {
        self.level_random = Some(rng);
    }

    pub(crate) fn level_next_bool(&mut self, rng: &mut super::rng::WorldgenRng) -> bool {
        self.level_random.as_mut().unwrap_or(rng).next_bool()
    }

    pub fn write(&mut self, (x, y, z): Pos, state: u16, flags: u32) -> bool {
        if self.outside_build_height(y) {
            return false;
        }
        match self.live.as_mut() {
            Some(live) => live.set((x, y, z), state, flags),
            None => self.set_chunk_block(x, y, z, state, Some(flags)),
        }
    }

    pub fn destroy(&mut self, pos: Pos, air: u16) {
        if let Some(live) = self.live.as_mut() {
            live.destroy(pos);
        } else if !self.is_air(pos.0, pos.1, pos.2) {
            self.write(pos, air, 3);
        }
    }

    pub fn schedule(&mut self, pos: Pos, state: u16) {
        if let Some(live) = self.live.as_mut() {
            live.schedule(pos, state);
        } else if !self.outside_build_height(pos.1) && self.ensure_can_write(pos.0, pos.1, pos.2) {
            self.effects.block_ticks.push((pos, state));
        }
    }

    pub fn take_effects(&mut self) -> DecorationEffects {
        std::mem::take(&mut self.effects)
    }

    pub(super) fn mark_post_process(&mut self, pos: Pos) {
        if self.slot(pos.0, pos.1, pos.2).is_some() {
            self.effects.post_processing.push(pos);
        }
    }

    pub(super) fn schedule_fluid(&mut self, pos: Pos, fluid: Fluid) {
        if self.slot(pos.0, pos.1, pos.2).is_some() {
            self.effects.fluid_ticks.push((pos, fluid));
        }
    }

    pub(super) fn world_seed(&self) -> i64 {
        self.biomes.as_ref().map_or(0, |b| b.overworld.seed())
    }

    pub(super) fn mark_above(&mut self, (x, y, z): Pos) {
        for py in y + 1..=y + 2 {
            if self.is_air(x, py, z) {
                break;
            }
            self.mark_post_process((x, py, z));
        }
    }

    /// A region for single-feature placement on the live world (saplings): `blocks` are the nine chunks in
    /// raster order, the caller fills the cells it needs and leaves the rest as air (palette id 0 in the live
    /// palette). Heightmaps are not primed: tree placement reads no heights.
    pub fn for_growers(tags: &'a Tags, min_y: i32, height: i32, cx: i32, cz: i32, blocks: Vec<Vec<u16>>) -> Self {
        assert_eq!(blocks.len(), 9, "region needs the 3x3 chunks");
        let n = (height as usize) * 256;
        assert!(blocks.iter().all(|b| b.len() == n), "chunk arrays must be full height");
        Region {
            tags,
            live: None,
            level_random: None,
            effects: DecorationEffects::default(),
            placed_in_biome: &[],
            biomes: None,
            min_y,
            height,
            cx,
            cz,
            blocks,
            wg: vec![0; 9 * 2 * 256],
            fin: vec![0; 9 * 4 * 256],
            log: None,
        }
    }

    /// Starts recording every block write (`set_block` and `set_block_in_section`) in order.
    pub fn start_log(&mut self) {
        self.log = Some(Vec::new());
    }

    /// Stops recording and returns the writes in the order they happened.
    pub fn take_log(&mut self) -> Vec<BlockWrite> {
        self.log.take().unwrap_or_default()
    }

    /// Takes ownership of the nine chunk block buffers until decoration returns them.
    pub(crate) fn new(
        tags: &'a Tags,
        placed_in_biome: &'a [Vec<bool>],
        overworld: &'a Overworld,
        min_y: i32,
        height: i32,
        cx: i32,
        cz: i32,
        chunks: &'a mut [ChunkBlocks],
        wg: &[WgHeights],
        mut fin: Vec<i32>,
    ) -> Self {
        assert_eq!(chunks.len(), 9, "region needs the 3x3 chunks");
        assert_eq!(wg.len(), 9, "region needs the 3x3 worldgen heights");
        let blocks: Vec<Vec<u16>> = chunks.iter_mut().map(|c| std::mem::take(&mut c.blocks)).collect();
        let mut wg_flat = Vec::with_capacity(9 * 2 * 256);
        for h in wg {
            wg_flat.extend_from_slice(&h[0]);
            wg_flat.extend_from_slice(&h[1]);
        }
        // Priming (`primeHeightmaps`) of the final types from the current blocks.
        if fin.is_empty() {
            fin.reserve(9 * 4 * 256);
            for b in &blocks {
                for kind in FINAL {
                    for col in 0..256 {
                        fin.push(heightmap::prime_column(b, tags, kind, min_y, height, col));
                    }
                }
            }
        }
        let quarts = std::array::from_fn(|i| chunks[i].biomes.as_slice());
        let biomes = Some(DecorationBiomes { overworld, quarts });
        Region { tags, live: None, level_random: None, effects: DecorationEffects::default(),
            placed_in_biome, biomes, min_y, height, cx, cz, blocks, wg: wg_flat, fin, log: None }
    }

    pub(crate) fn take_final_heights(&mut self) -> Vec<i32> {
        std::mem::take(&mut self.fin)
    }

    /// Consumes the region and returns the nine (mutated) block arrays in raster order.
    pub fn into_blocks(self) -> Vec<Vec<u16>> {
        self.blocks
    }

    #[inline]
    pub fn min_y(&self) -> i32 {
        self.min_y
    }

    #[inline]
    pub fn height(&self) -> i32 {
        self.height
    }

    #[inline]
    pub fn outside_build_height(&self, y: i32) -> bool {
        y < self.min_y || y >= self.min_y + self.height
    }

    /// `WorldGenRegion.ensureCanWrite` for the FEATURES step (write radius 1).
    #[inline]
    pub fn ensure_can_write(&self, x: i32, _y: i32, z: i32) -> bool {
        self.live.is_some() || self.chunk_index(x, z).is_some()
    }

    #[inline]
    fn chunk_index(&self, x: i32, z: i32) -> Option<usize> {
        let dx = x.div_euclid(16) - self.cx;
        let dz = z.div_euclid(16) - self.cz;
        if dx.abs() <= 1 && dz.abs() <= 1 {
            Some(((dz + 1) * 3 + (dx + 1)) as usize)
        } else {
            None
        }
    }

    #[inline]
    fn slot(&self, x: i32, y: i32, z: i32) -> Option<(usize, usize)> {
        let c = self.chunk_index(x, z)?;
        let rel_y = y - self.min_y;
        if rel_y < 0 || rel_y >= self.height {
            return None;
        }
        Some((c, ((rel_y as usize) * 16 + (z & 15) as usize) * 16 + (x & 15) as usize))
    }

    /// Block palette index at a position inside the region. Outside the build height vanilla returns
    /// `VOID_AIR`; the palette id 0 (air) stands in for it, which every ported check treats the same.
    #[inline]
    pub fn get_block(&self, x: i32, y: i32, z: i32) -> u16 {
        if let Some(live) = self.live.as_ref() {
            return live.block((x, y, z));
        }
        match self.slot(x, y, z) {
            Some((c, i)) => self.blocks[c][i],
            None => 0,
        }
    }

    /// `WorldGenRegion.setBlock` for the FEATURES step: writes when the position is inside the region
    /// and build height, otherwise the write is dropped and `false` is returned. The final heightmaps of
    /// the chunk are updated as `ProtoChunk.setBlockState` does (`Heightmap.update`).
    #[inline]
    pub fn set_block(&mut self, x: i32, y: i32, z: i32, blk: u16) -> bool {
        if self.live.is_some() {
            return self.write((x, y, z), blk, 2);
        }
        self.set_chunk_block(x, y, z, blk, None)
    }

    fn set_chunk_block(&mut self, x: i32, y: i32, z: i32, blk: u16, flags: Option<u32>) -> bool {
        match self.slot(x, y, z) {
            Some((c, i)) if self.ensure_can_write(x, y, z) => {
                let old = self.blocks[c][i];
                self.blocks[c][i] = blk;
                if old != blk {
                    self.update_heights(c, x, y, z, blk);
                }
                if let Some(log) = self.log.as_mut() {
                    log.push((x, y, z, blk, flags));
                }
                true
            }
            _ => false,
        }
    }

    /// `Heightmap.update` of the four final types for a block that became `blk` at `(x, y, z)` in chunk `c`.
    fn update_heights(&mut self, c: usize, x: i32, y: i32, z: i32, blk: u16) {
        let col = ((z & 15) * 16 + (x & 15)) as usize;
        let (xl, zl) = ((x & 15) as usize, (z & 15) as usize);
        for (t, &kind) in FINAL.iter().enumerate() {
            let idx = (c * 4 + t) * 256 + col;
            let first = self.fin[idx];
            self.fin[idx] = heightmap::update_column(&self.blocks[c], self.tags, kind, self.min_y, first, xl, y, zl, blk);
        }
    }

    /// `isAir()` of the block at a position. Outside the build height vanilla reads `VOID_AIR`, which is air.
    #[inline]
    pub fn is_air(&self, x: i32, y: i32, z: i32) -> bool {
        if let Some(live) = self.live.as_ref() {
            return self.tags.is_air(live.block((x, y, z)));
        }
        match self.slot(x, y, z) {
            Some((c, i)) => self.tags.is_air(self.blocks[c][i]),
            None => self.outside_build_height(y) && self.chunk_index(x, z).is_some(),
        }
    }

    /// `LevelChunkSection.setBlockState(..., false)` via a bulk section: writes the block only, no heightmap
    /// update (`BulkSectionAccess` in `OreFeature`).
    #[inline]
    pub fn set_block_in_section(&mut self, x: i32, y: i32, z: i32, blk: u16) {
        let (c, i) = self.slot(x, y, z).expect("write outside the 3x3 region or build height");
        self.blocks[c][i] = blk;
        if let Some(log) = self.log.as_mut() {
            log.push((x, y, z, blk, None));
        }
    }

    /// `WorldGenRegion.getHeight(kind, x, z)`: the first-available height of the column (top + 1).
    #[inline]
    pub fn height_of(&self, kind: Kind, x: i32, z: i32) -> i32 {
        let c = self.chunk_index(x, z).expect("height outside the 3x3 region");
        let col = ((z & 15) * 16 + (x & 15)) as usize;
        match kind.final_index() {
            Some(t) => self.fin[(c * 4 + t) * 256 + col],
            None => {
                let k = if kind == Kind::WorldSurfaceWg { 0 } else { 1 };
                self.wg[(c * 2 + k) * 256 + col]
            }
        }
    }

    /// `WorldGenRegion.getHeight(OCEAN_FLOOR_WG, x, z)` (first-available height).
    #[inline]
    pub fn ocean_floor_wg(&self, x: i32, z: i32) -> i32 {
        self.height_of(Kind::OceanFloorWg, x, z)
    }

    /// `level.getBiome(pos).hasFeature(top)`.
    pub fn biome_has_feature(&self, x: i32, y: i32, z: i32, placed: usize) -> bool {
        self.biome_at(x, y, z).is_some_and(|(_, biome)| self.placed_in_biome[biome as usize][placed])
    }

    pub(crate) fn biome_at(&self, x: i32, y: i32, z: i32) -> Option<(&Overworld, u16)> {
        let b = self.biomes.as_ref()?;
        let (qx, qy, qz) = b.overworld.biome_quart(x, y, z);
        let chunk = self.chunk_index(qx * 4, qz * 4)?;
        let local_y = (qy - (self.min_y >> 2)).clamp(0, (self.height >> 2) - 1);
        let index = ((local_y * 4 + (qz & 3)) * 4 + (qx & 3)) as usize;
        Some((b.overworld, b.quarts[chunk][index]))
    }

    // FEATURES precedes INITIALIZE_LIGHT. The visible block light is zero and the
    // uninitialized Overworld sky light is 15 (SkyLightSectionStorage.getLightValue).
    pub(crate) fn feature_raw_brightness(&self, pos: Pos) -> u8 {
        self.live.as_ref().map_or(15, |live| live.raw_brightness(pos))

    }

    pub(super) fn should_freeze_water(&self, p: Pos, water: u16) -> bool {
        let (x, y, z) = p;
        let state = self.get_block(x, y, z);
        !self.outside_build_height(y) && self.tags.is_block(state, water) && self.tags.fluid(state) == Fluid::Water
            && self.biome_at(x, y, z).is_some_and(|(o, biome)| o.biome_temperature(biome, x, y, z) < 0.15)
    }
}
