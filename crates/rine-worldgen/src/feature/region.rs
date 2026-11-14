//! The decoration view of a 3x3 chunk region (`WorldGenRegion` with write radius 1): block reads and
//! writes over nine chunks, the `OCEAN_FLOOR_WG` first-available heights of those chunks, and the
//! biome lookups used by `biome` placement filters.

use std::collections::HashMap;

use crate::overworld::{BiomeMap, ChunkBlocks};

/// Block tag membership per palette index. Tag ids are indices into the sorted tag name list.
pub struct Tags {
    names: Vec<String>,
    words: usize,
    bits: Vec<u64>,
    air: Vec<bool>,
}

impl Tags {
    /// Builds membership for every palette entry. `block_tags` maps tag name to block ids.
    pub fn new(palette: &[String], block_tags: &HashMap<String, Vec<String>>) -> Self {
        let mut names: Vec<String> = block_tags.keys().cloned().collect();
        names.sort();
        let words = names.len().div_ceil(64).max(1);
        let mut bits = vec![0u64; palette.len() * words];
        let mut air = vec![false; palette.len()];
        for (i, state) in palette.iter().enumerate() {
            let name = state.split('[').next().unwrap_or(state);
            air[i] = matches!(name, "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air");
            for (t, tag) in names.iter().enumerate() {
                if block_tags[tag].iter().any(|b| b == name) {
                    bits[i * words + t / 64] |= 1 << (t % 64);
                }
            }
        }
        Tags { names, words, bits, air }
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
    /// `OCEAN_FLOOR_WG` first-available height per column, `(z & 15) * 16 + (x & 15)`.
    ocean_floor: Vec<Vec<i32>>,
}

impl<'a> Region<'a> {
    pub(crate) fn new(
        tags: &'a Tags,
        placed_in_biome: &'a [Vec<bool>],
        biomes: BiomeMap<'a>,
        min_y: i32,
        height: i32,
        cx: i32,
        cz: i32,
        chunks: &[ChunkBlocks],
    ) -> Self {
        assert_eq!(chunks.len(), 9, "region needs the 3x3 chunks");
        let blocks: Vec<Vec<u16>> = chunks.iter().map(|c| c.blocks.clone()).collect();
        let ocean_floor = blocks.iter().map(|b| compute_ocean_floor(b, tags, min_y, height)).collect();
        Region { tags, placed_in_biome, biomes, min_y, height, cx, cz, blocks, ocean_floor }
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

    /// Block palette index at a position inside the region.
    #[inline]
    pub fn get_block(&self, x: i32, y: i32, z: i32) -> u16 {
        let (c, i) = self.slot(x, y, z).expect("read outside the 3x3 region or build height");
        self.blocks[c][i]
    }

    /// `isAir()` of the block at a position (outside the build height counts as not air).
    #[inline]
    pub fn is_air(&self, x: i32, y: i32, z: i32) -> bool {
        match self.slot(x, y, z) {
            Some((c, i)) => self.tags.is_air(self.blocks[c][i]),
            None => false,
        }
    }

    /// `LevelChunkSection.setBlockState(..., false)` via a bulk section: writes the block only.
    #[inline]
    pub fn set_block_in_section(&mut self, x: i32, y: i32, z: i32, blk: u16) {
        let (c, i) = self.slot(x, y, z).expect("write outside the 3x3 region or build height");
        self.blocks[c][i] = blk;
    }

    /// `WorldGenRegion.getHeight(OCEAN_FLOOR_WG, x, z)` (first-available height).
    #[inline]
    pub fn ocean_floor_wg(&self, x: i32, z: i32) -> i32 {
        let c = self.chunk_index(x, z).expect("height outside the 3x3 region");
        self.ocean_floor[c][((z & 15) * 16 + (x & 15)) as usize]
    }

    /// `level.getBiome(pos).hasFeature(top)`.
    pub fn biome_has_feature(&self, x: i32, y: i32, z: i32, placed: usize) -> bool {
        let biome = self.biomes.fiddled(x, y, z) as usize;
        self.placed_in_biome[biome][placed]
    }
}

/// `OCEAN_FLOOR_WG`: `MATERIAL_MOTION_BLOCKING` (`blocks_motion_in_heightmap`) first-available heights.
fn compute_ocean_floor(blocks: &[u16], tags: &Tags, min_y: i32, height: i32) -> Vec<i32> {
    let motion = tags.id("minecraft:blocks_motion_in_heightmap");
    let mut out = vec![min_y; 256];
    let Some(motion) = motion else { return out };
    for z in 0..16usize {
        for x in 0..16usize {
            for rel in (0..height as usize).rev() {
                let blk = blocks[(rel * 16 + z) * 16 + x];
                if tags.has(blk, motion) {
                    out[z * 16 + x] = min_y + rel as i32 + 1;
                    break;
                }
            }
        }
    }
    out
}
