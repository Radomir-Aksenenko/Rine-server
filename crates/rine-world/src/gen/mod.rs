//! Survival terrain generator. No structures, no dependencies beyond rine-data.
//!
//! A chunk is a pure function of (seed, chunk pos). Trees are the only feature that
//! crosses chunk borders: each chunk recomputes the trees of its 3x3 neighbourhood
//! from hashes and writes only the cells it owns, so chunks can run in parallel.

mod biome;
mod decor;
mod grid;
mod noise;
mod ores;
mod terrain;
mod trees;

use crate::chunk::{Chunk, ChunkPos, MIN_Y, SECTIONS};
use biome::{Biome, BIOME_COUNT};
use grid::Grid;
use terrain::{Column, Corners, Noises, SEA_LEVEL};

/// Resolved block state ids and biome-independent lookup tables.
pub struct Ids {
    pub air: u16,
    pub stone: u16,
    pub deepslate: u16,
    pub bedrock: u16,
    pub water: u16,
    pub lava: u16,
    pub ice: u16,
    pub grass: u16,
    pub grass_snowy: u16,
    pub dirt: u16,
    pub sand: u16,
    pub sandstone: u16,
    pub gravel: u16,
    pub snow_block: u16,
    pub snow_layer: u16,
    pub short_grass: u16,
    pub tall_lower: u16,
    pub tall_upper: u16,
    pub flowers: [u16; 5],
    pub dead_bush: u16,
    pub cactus: u16,
    pub sugar_cane: u16,
    pub pumpkin: u16,
    /// dirt, gravel, granite, diorite, andesite, tuff
    pub blobs: [u16; 6],
    /// coal, iron, copper, gold, redstone, lapis, diamond, emerald
    pub ore_stone: [u16; 8],
    pub ore_deep: [u16; 8],
    /// Indexed by `trees::Kind`.
    pub logs: [u16; 6],
    pub leaves: [u16; 6],
}

pub struct Generator {
    seed: u64,
    noise: Noises,
    ids: Ids,
    biome_ids: [u8; BIOME_COUNT],
}

const FLOWERS: [&str; 5] = ["dandelion", "poppy", "cornflower", "oxeye_daisy", "azure_bluet"];
const BLOB_NAMES: [&str; 6] = ["dirt", "gravel", "granite", "diorite", "andesite", "tuff"];
const ORE_NAMES: [&str; 8] = ["coal", "iron", "copper", "gold", "redstone", "lapis", "diamond", "emerald"];
const TREE_WOODS: [&str; 6] = ["oak", "oak", "birch", "spruce", "acacia", "jungle"];

impl Generator {
    pub fn new(seed: i64) -> Self {
        let data = rine_data::init();
        let b = &data.blocks;
        let reg = data.registries.synced("minecraft:worldgen/biome");
        let st = |s: &str| b.parse_state(s).unwrap_or_else(|| panic!("unknown block state {s}"));
        let dft = |n: &str| b.default_state(n);

        let mut biome_ids = [0u8; BIOME_COUNT];
        for (i, bm) in Biome::ALL.iter().enumerate() {
            biome_ids[i] = reg.id(bm.name()).unwrap_or_else(|| panic!("unknown biome {}", bm.name())) as u8;
        }

        let ids = Ids {
            air: dft("air"),
            stone: dft("stone"),
            deepslate: dft("deepslate"),
            bedrock: dft("bedrock"),
            water: dft("water"),
            lava: dft("lava"),
            ice: dft("ice"),
            grass: dft("grass_block"),
            grass_snowy: st("minecraft:grass_block[snowy=true]"),
            dirt: dft("dirt"),
            sand: dft("sand"),
            sandstone: dft("sandstone"),
            gravel: dft("gravel"),
            snow_block: dft("snow_block"),
            snow_layer: st("minecraft:snow[layers=1]"),
            short_grass: dft("short_grass"),
            tall_lower: st("minecraft:tall_grass[half=lower]"),
            tall_upper: st("minecraft:tall_grass[half=upper]"),
            flowers: FLOWERS.map(|n| dft(n)),
            dead_bush: dft("dead_bush"),
            cactus: dft("cactus"),
            sugar_cane: dft("sugar_cane"),
            pumpkin: dft("pumpkin"),
            blobs: BLOB_NAMES.map(|n| dft(n)),
            ore_stone: ORE_NAMES.map(|n| st(&format!("minecraft:{n}_ore"))),
            ore_deep: ORE_NAMES.map(|n| st(&format!("minecraft:deepslate_{n}_ore"))),
            logs: TREE_WOODS.map(|w| st(&format!("minecraft:{w}_log[axis=y]"))),
            leaves: TREE_WOODS
                .map(|w| st(&format!("minecraft:{w}_leaves[distance=1,persistent=false,waterlogged=false]"))),
        };

        Self {
            seed: seed as u64,
            noise: Noises::new(seed as u64),
            ids,
            biome_ids,
        }
    }

    /// Pure function of (seed, pos). Thread-safe.
    pub fn generate(&self, pos: ChunkPos) -> Chunk {
        let (ox, oz) = (pos.x * 16, pos.z * 16);
        // Lattice covering the chunk plus one chunk of margin on every side.
        let corners = Corners::new(&self.noise, ox - 16, oz - 16, 13, 13);

        // Surfaces of the 18x18 extended area (chunk plus one column of margin).
        let mut ext = [0i32; 18 * 18];
        for lz in 0..18usize {
            for lx in 0..18usize {
                ext[lz * 18 + lx] = corners.column(self.seed, ox + lx as i32 - 1, oz + lz as i32 - 1).surface;
            }
        }
        let mut cols = [Column { surface: 0, biome: Biome::Plains }; 256];
        for lz in 0..16usize {
            for lx in 0..16usize {
                cols[lz * 16 + lx] = corners.column(self.seed, ox + lx as i32, oz + lz as i32);
            }
        }

        let mut grid = Grid::new(self.ids.air);
        for lz in 0..16usize {
            for lx in 0..16usize {
                let col = cols[lz * 16 + lx];
                terrain::fill_column(
                    &mut grid,
                    &self.ids,
                    self.seed,
                    lx as i32,
                    lz as i32,
                    ox + lx as i32,
                    oz + lz as i32,
                    col,
                );
            }
        }
        terrain::carve(&self.noise, &mut grid, &self.ids, ox, oz, &ext);
        ores::place(&mut grid, &self.ids, self.seed, pos.x, pos.z, &cols);
        trees::place(&mut grid, &self.ids, self.seed, pos.x, pos.z, &corners);
        decor::place(&mut grid, &self.ids, self.seed, pos.x, pos.z, &cols, &corners);

        let mut chunk = Chunk::new(pos);
        for sy in 0..SECTIONS {
            let slice = &grid.data[sy * 4096..(sy + 1) * 4096];
            let first = slice[0];
            let sec = &mut chunk.sections[sy];
            if slice.iter().all(|&v| v == first) {
                sec.fill(first);
            } else {
                let mut arr = Box::new([0u16; 4096]);
                arr.copy_from_slice(slice);
                sec.blocks = Some(arr);
            }
            for qy in 0..4usize {
                for qz in 0..4usize {
                    for qx in 0..4usize {
                        let b = cols[(qz * 4 + 2) * 16 + qx * 4 + 2].biome;
                        sec.biomes[(qy << 4) | (qz << 2) | qx] = self.biome_ids[b as usize];
                    }
                }
            }
        }
        chunk.finish_bulk();
        chunk
    }

    /// Column data for one world position (spawn search, neighbour checks).
    fn column_at(&self, x: i32, z: i32) -> Column {
        terrain::column_at(&self.noise, self.seed, x, z)
    }

    /// A standing position (feet y) near (0, 0): solid ground below, air at feet and head,
    /// on land above sea level.
    pub fn find_spawn(&self) -> (i32, i32, i32) {
        let b = &rine_data::get().blocks;
        for r in 0..=1024i32 {
            let mut pts: Vec<(i32, i32)> = Vec::new();
            if r == 0 {
                pts.push((0, 0));
            } else {
                for d in -r..=r {
                    pts.push((d, -r));
                    pts.push((d, r));
                }
                for d in (-r + 1)..r {
                    pts.push((-r, d));
                    pts.push((r, d));
                }
            }
            for (x, z) in pts {
                let c = self.column_at(x, z);
                let spawn_ok = matches!(
                    c.biome,
                    Biome::Plains
                        | Biome::SunflowerPlains
                        | Biome::Forest
                        | Biome::BirchForest
                        | Biome::DarkForest
                        | Biome::Taiga
                        | Biome::Desert
                        | Biome::Savanna
                        | Biome::Jungle
                        | Biome::Beach
                );
                if c.surface < SEA_LEVEL || !spawn_ok {
                    continue;
                }
                let y = c.surface + 1;
                let chunk = self.generate(ChunkPos::of_block(x, z));
                let (lx, lz) = (x.rem_euclid(16), z.rem_euclid(16));
                let below = chunk.get(lx, y - 1, lz);
                let feet = chunk.get(lx, y, lz);
                let head = chunk.get(lx, y + 1, lz);
                let solid_below = !b.is_air(below) && !b.has(below, rine_data::blocks::flag::WATER | rine_data::blocks::flag::LAVA);
                if solid_below && b.is_air(feet) && b.is_air(head) {
                    return (x, y, z);
                }
            }
        }
        let c = self.column_at(0, 0);
        (0, c.surface.max(SEA_LEVEL) + 1, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn blocks_of(c: &Chunk) -> Vec<u16> {
        let mut v = Vec::with_capacity(SECTIONS * 4096);
        for s in &c.sections {
            for i in 0..4096 {
                v.push(s.get(i));
            }
        }
        v
    }

    #[test]
    fn generator_builds_and_bedrock_floor() {
        rine_data::init();
        let g = Generator::new(12345);
        let bedrock = g.ids.bedrock;
        let mut total_non_air = 0u32;
        for &(x, z) in &[(0, 0), (5, -3), (-7, 11)] {
            let c = g.generate(ChunkPos::new(x, z));
            for lx in 0..16 {
                for lz in 0..16 {
                    assert_eq!(c.get(lx, MIN_Y, lz), bedrock, "bedrock at chunk {x},{z}");
                }
            }
            let non_air: u32 = c.sections.iter().map(|s| s.non_air as u32).sum();
            assert!(non_air > 1000, "chunk {x},{z} has only {non_air} blocks");
            total_non_air += non_air;
        }
        assert!(total_non_air > 0);
    }

    #[test]
    fn deterministic_same_seed() {
        rine_data::init();
        let a = Generator::new(987654321);
        let b = Generator::new(987654321);
        for &(x, z) in &[(3, 4), (-20, 9)] {
            let ca = a.generate(ChunkPos::new(x, z));
            let cb = b.generate(ChunkPos::new(x, z));
            assert_eq!(blocks_of(&ca), blocks_of(&cb));
            for (sa, sb) in ca.sections.iter().zip(cb.sections.iter()) {
                assert_eq!(sa.biomes, sb.biomes);
            }
        }
    }

    #[test]
    fn spawn_is_safe() {
        rine_data::init();
        let g = Generator::new(42);
        let (x, y, z) = g.find_spawn();
        assert!(y > 62, "spawn y {y}");
        let b = &rine_data::get().blocks;
        let c = g.generate(ChunkPos::of_block(x, z));
        let below = c.get(x, y - 1, z);
        assert!(!b.is_air(below) && !b.has(below, rine_data::blocks::flag::WATER), "no solid ground under spawn");
        assert!(b.is_air(c.get(x, y, z)));
    }

    #[test]
    fn timing_64_chunks() {
        rine_data::init();
        let g = Generator::new(2024);
        let _ = g.generate(ChunkPos::new(100, 100)); // warm-up
        let n = 64;
        let start = Instant::now();
        let mut underground_air = 0u64;
        for i in 0..n {
            let c = g.generate(ChunkPos::new(i % 8, i / 8));
            for y in -50..40 {
                for lx in 0..16 {
                    for lz in 0..16 {
                        if c.get(lx, y, lz) == g.ids.air {
                            underground_air += 1;
                        }
                    }
                }
            }
        }
        let ms = start.elapsed().as_secs_f64() * 1000.0 / n as f64;
        println!("gen: {ms:.3} ms/chunk over {n} chunks (underground air cells in y -50..40: {underground_air})");
    }
}
