//! The decoration view of a 3x3 chunk region (`WorldGenRegion` with write radius 1): block reads and
//! writes over nine chunks, the heightmaps of those chunks (`heightmap.rs` documents which are frozen and
//! which are maintained), and the biome lookups used by `biome` placement filters.

use std::collections::HashMap;

use super::heightmap::{self, Kind, WgHeights, FINAL};
use super::state::{Fluid, Palette};
use crate::overworld::{BiomeMap, ChunkBlocks};

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
        Tags { names, words, bits, air, block_of, fluid_of, opaque, veg: Default::default() }
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
}

/// Mutable 3x3 chunk region around the chunk being decorated.
pub struct Region<'a> {
    pub tags: &'a Tags,
    /// `placed_in_biome[biome][placed]`: `Biome.hasFeature` over the biome's full feature lists.
    placed_in_biome: &'a [Vec<bool>],
    biomes: BiomeMap<'a>,
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
}

impl<'a> Region<'a> {
    /// `chunks` are the current blocks of the nine chunks; `wg` their terrain-end worldgen heights.
    pub(crate) fn new(
        tags: &'a Tags,
        placed_in_biome: &'a [Vec<bool>],
        biomes: BiomeMap<'a>,
        min_y: i32,
        height: i32,
        cx: i32,
        cz: i32,
        chunks: &[ChunkBlocks],
        wg: &[WgHeights],
    ) -> Self {
        assert_eq!(chunks.len(), 9, "region needs the 3x3 chunks");
        assert_eq!(wg.len(), 9, "region needs the 3x3 worldgen heights");
        let blocks: Vec<Vec<u16>> = chunks.iter().map(|c| c.blocks.clone()).collect();
        let mut wg_flat = Vec::with_capacity(9 * 2 * 256);
        for h in wg {
            wg_flat.extend_from_slice(&h[0]);
            wg_flat.extend_from_slice(&h[1]);
        }
        // Priming (`primeHeightmaps`) of the final types from the current blocks.
        let mut fin = Vec::with_capacity(9 * 4 * 256);
        for b in &blocks {
            for kind in FINAL {
                for col in 0..256 {
                    fin.push(heightmap::prime_column(b, tags, kind, min_y, height, col));
                }
            }
        }
        Region { tags, placed_in_biome, biomes, min_y, height, cx, cz, blocks, wg: wg_flat, fin }
    }

    /// Consumes the region and returns the nine (mutated) block arrays in raster order.
    pub(crate) fn into_blocks(self) -> Vec<Vec<u16>> {
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
        self.chunk_index(x, z).is_some()
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
        match self.slot(x, y, z) {
            Some((c, i)) => self.blocks[c][i],
            None => {
                assert!(self.chunk_index(x, z).is_some(), "read outside the 3x3 region");
                0
            }
        }
    }

    /// `WorldGenRegion.setBlock` for the FEATURES step: writes when the position is inside the region
    /// and build height, otherwise the write is dropped and `false` is returned. The final heightmaps of
    /// the chunk are updated as `ProtoChunk.setBlockState` does (`Heightmap.update`).
    #[inline]
    pub fn set_block(&mut self, x: i32, y: i32, z: i32, blk: u16) -> bool {
        match self.slot(x, y, z) {
            Some((c, i)) if self.ensure_can_write(x, y, z) => {
                let old = self.blocks[c][i];
                self.blocks[c][i] = blk;
                if old != blk {
                    self.update_heights(c, x, y, z, blk);
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
        let biome = self.biomes.fiddled(x, y, z) as usize;
        self.placed_in_biome[biome][placed]
    }
}
