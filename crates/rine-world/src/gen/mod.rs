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
mod slots;
mod terrain;
mod trees;

use crate::chunk::{Chunk, ChunkPos, MIN_Y, SECTIONS};
use std::sync::Arc;
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
    /// Bit-exact vanilla terrain, biomes and surface (rine-worldgen).
    vanilla: Arc<rine_worldgen::Overworld>,
    vegetation: Option<rine_worldgen::pipeline::Pipeline>,
    /// rine-worldgen palette string -> state id (same palette for every chunk).
    palette: parking_lot::RwLock<Vec<u16>>,
    /// rine-worldgen biome palette index -> synced biome registry id.
    biome_map: parking_lot::RwLock<Vec<u8>>,
    seed: u64,
    noise: Noises,
    ids: Ids,
    biome_ids: [u8; BIOME_COUNT],
    /// Synced biome id -> index in the rine-worldgen biome list (`u16::MAX` if not an overworld biome).
    biome_vanilla: Vec<u16>,
    /// Synced biome id -> `Biome.hasPrecipitation`.
    biome_precip: Vec<bool>,
    pale_garden: Option<u8>,
}

const FLOWERS: [&str; 5] = ["dandelion", "poppy", "cornflower", "oxeye_daisy", "azure_bluet"];
const BLOB_NAMES: [&str; 6] = ["dirt", "gravel", "granite", "diorite", "andesite", "tuff"];
const ORE_NAMES: [&str; 8] = ["coal", "iron", "copper", "gold", "redstone", "lapis", "diamond", "emerald"];
const TREE_WOODS: [&str; 6] = ["oak", "oak", "birch", "spruce", "acacia", "jungle"];

impl Generator {
    /// Generator with the long-standing live decoration (no vanilla vegetation pass).
    pub fn new(seed: i64) -> Self {
        Self::with_options(seed, false)
    }

    /// `vanilla_vegetation`: decorate through `rine-worldgen` (carved terrain plus vanilla
    /// grass, flowers and moss). Changes newly generated chunks and is about 3x slower
    /// per chunk, so it is off unless the owner turns it on (`rine.toml`).
    pub fn with_options(seed: i64, vanilla_vegetation: bool) -> Self {
        Self::build(seed, vanilla_vegetation, None)
    }

    /// As `with_options`, keeping the vanilla decoration frontier under `<world_dir>/worldgen`.
    pub fn for_world(seed: i64, vanilla_vegetation: bool, world_dir: &str) -> Self {
        Self::build(seed, vanilla_vegetation, Some(world_dir))
    }

    fn build(seed: i64, vanilla_vegetation: bool, world_dir: Option<&str>) -> Self {
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

        // Overworld biomes with `has_precipitation: false` in the 26.3 worldgen/biome JSON.
        const NO_PRECIPITATION: [&str; 7] = [
            "minecraft:badlands",
            "minecraft:desert",
            "minecraft:eroded_badlands",
            "minecraft:savanna",
            "minecraft:savanna_plateau",
            "minecraft:windswept_savanna",
            "minecraft:wooded_badlands",
        ];
        let vanilla = Arc::new(rine_worldgen::Overworld::new(seed));
        let vegetation = if !vanilla_vegetation {
            None
        } else {
            match rine_worldgen::pipeline::Pipeline::with_features(Arc::clone(&vanilla), &rine_worldgen::feature::allow::live_generation) {
                Ok(pipeline) => Some(match world_dir {
                    Some(dir) => pipeline.with_store(Box::new(slots::DiskSlots::open(dir))),
                    None => pipeline,
                }),
                Err(error) => {
                    eprintln!("failed to compile vegetation decoration: {error}");
                    None
                }
            }
        };
        let mut biome_vanilla = vec![u16::MAX; 256];
        let mut biome_precip = vec![true; 256];
        for (i, name) in vanilla.biome_names().iter().enumerate() {
            if let Some(id) = reg.id(name).filter(|&id| id < 256) {
                biome_vanilla[id as usize] = i as u16;
                biome_precip[id as usize] = !NO_PRECIPITATION.contains(&name.as_str());
            }
        }

        Self {
            vanilla,
            vegetation,
            palette: parking_lot::RwLock::new(Vec::new()),
            biome_map: parking_lot::RwLock::new(Vec::new()),
            seed: seed as u64,
            noise: Noises::new(seed as u64),
            ids,
            biome_ids,
            biome_vanilla,
            biome_precip,
            pale_garden: reg.id("minecraft:pale_garden").and_then(|id| u8::try_from(id).ok()),
        }
    }

    /// Generated chunks depend on the order they were requested in (vanilla decoration): the caller must
    /// save every generated chunk, since asking again does not rebuild the same chunk.
    pub fn saves_generated(&self) -> bool {
        self.vegetation.is_some()
    }

    /// Writes the decoration frontier. Call after the generated chunks are saved, at every world save.
    pub fn persist(&self) {
        if let Some(p) = &self.vegetation {
            p.persist();
        }
    }

    /// `Biome.getTemperature(pos, seaLevel)` of a synced biome id at a block position (0.8 if unknown).
    pub fn temperature(&self, biome: u8, x: i32, y: i32, z: i32) -> f32 {
        match self.biome_vanilla[biome as usize] {
            u16::MAX => 0.8,
            i => self.vanilla.biome_temperature(i, x, y, z),
        }
    }

    /// `Biome.hasPrecipitation` of a synced biome id.
    pub fn has_precipitation(&self, biome: u8) -> bool {
        self.biome_precip[biome as usize]
    }

    pub fn bone_meal_biome_sample(&self, x: i32, y: i32, z: i32) -> (i32, i32, i32) {
        let (qx, qy, qz) = self.vanilla.biome_quart(x, y, z);
        (qx * 4, qy * 4, qz * 4)
    }

    /// Map palette names to state / biome ids, extending the caches as the
    /// (global, append-only) rine-worldgen palettes grow.
    fn map_palettes(&self, cb: &rine_worldgen::ChunkBlocks) -> (Vec<u16>, Vec<u8>) {
        let data = rine_data::get();
        {
            let p = self.palette.read();
            let bm = self.biome_map.read();
            if p.len() >= cb.palette.len() && bm.len() >= cb.biome_palette.len() {
                return (p.clone(), bm.clone());
            }
        }
        let mut p = self.palette.write();
        for name in &cb.palette[p.len().min(cb.palette.len())..] {
            let id = data.blocks.parse_state(name).unwrap_or_else(|| {
                let base = name.split('[').next().unwrap_or(name);
                data.blocks.default_state(base)
            });
            p.push(id);
        }
        let reg = data.registries.synced("minecraft:worldgen/biome");
        let mut bm = self.biome_map.write();
        for name in &cb.biome_palette[bm.len().min(cb.biome_palette.len())..] {
            bm.push(reg.id(name).unwrap_or(0) as u8);
        }
        (p.clone(), bm.clone())
    }

    /// Pure function of (seed, pos). Thread-safe.
    pub fn generate(&self, pos: ChunkPos) -> Chunk {
        self.generate_with_ticks(pos).0
    }

    pub fn generate_with_ticks(&self, pos: ChunkPos) -> (Chunk, Vec<(rine_protocol::BlockPos, u16)>) {
        let generated;
        let raw;
        let (cb, ticks) = match self.vegetation.as_ref() {
            Some(pipeline) => {
                generated = pipeline.full_chunk(pos.x, pos.z);
                (&generated.blocks, generated.block_ticks.as_slice())
            }
            None => {
                raw = self.vanilla.generate(pos.x, pos.z);
                (&raw, &[][..])
            }
        };
        let (pal, bmap) = self.map_palettes(cb);
        debug_assert_eq!(cb.min_y, MIN_Y);
        let mut grid = Grid::new(self.ids.air);
        // rine-worldgen: [((y - min_y) * 16 + z) * 16 + x]; Grid uses the same order
        for (i, &v) in cb.blocks.iter().enumerate().take(grid.data.len()) {
            grid.data[i] = pal[v as usize];
        }
        // columns: surface = highest motion-blocking non-fluid block, biome there
        let b = &rine_data::get().blocks;
        let qh = (cb.height / 4) as usize;
        let mut cols = [Column { surface: MIN_Y, biome: Biome::Plains }; 256];
        let mut legacy_trees = [true; 256];
        for lz in 0..16i32 {
            for lx in 0..16i32 {
                let mut y = MIN_Y + cb.height - 1;
                while y > MIN_Y {
                    let st = grid.get(lx, y, lz);
                    if !b.is_air(st) && !b.has(st, rine_data::blocks::flag::NO_COLLISION) {
                        break;
                    }
                    y -= 1;
                }
                let qy = (((y + 1 - MIN_Y) >> 2) as usize).min(qh - 1);
                let bi = cb.biomes[(qy * 4 + (lz as usize >> 2)) * 4 + (lx as usize >> 2)];
                let name = cb.biome_palette[bi as usize].as_str();
                let column = (lz * 16 + lx) as usize;
                cols[column] = Column { surface: y, biome: biome::from_vanilla(name) };
                legacy_trees[column] = self.vegetation.is_none() || self.pale_garden != Some(bmap[bi as usize]);
            }
        }
        ores::place(&mut grid, &self.ids, self.seed, pos.x, pos.z, &cols);
        trees::place_local(&mut grid, &self.ids, self.seed, pos.x, pos.z, &cols, &legacy_trees);
        decor::place_local(&mut grid, &self.ids, self.seed, pos.x, pos.z, &cols, self.vegetation.is_none());

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
                        let gy = (sy * 4 + qy).min(qh - 1);
                        let bi = cb.biomes[(gy * 4 + qz) * 4 + qx];
                        sec.biomes[(qy << 4) | (qz << 2) | qx] = bmap[bi as usize];
                    }
                }
            }
        }
        chunk.finish_bulk();
        let ticks = ticks.iter().map(|&((x, y, z), state)| {
            (rine_protocol::BlockPos { x, y, z }, b.state(pal[state as usize]).block)
        }).collect();
        (chunk, ticks)
    }

    /// Column data for one world position (spawn search, neighbour checks).
    fn column_at(&self, x: i32, z: i32) -> Column {
        terrain::column_at(&self.noise, self.seed, x, z)
    }

    /// A standing position (feet y) near (0, 0): solid dry ground with two
    /// air blocks above, searched outward chunk by chunk on the real terrain.
    pub fn find_spawn(&self) -> (i32, i32, i32) {
        let b = &rine_data::get().blocks;
        for r in 0..=16i32 {
            for cz in -r..=r {
                for cx in -r..=r {
                    if cx.abs() != r && cz.abs() != r {
                        continue;
                    }
                    let chunk = self.generate(ChunkPos::new(cx, cz));
                    for lz in [8, 4, 12, 0, 15] {
                        for lx in [8, 4, 12, 0, 15] {
                            let mut y = crate::chunk::MAX_Y - 1;
                            while y > SEA_LEVEL && b.is_air(chunk.get(lx, y, lz)) {
                                y -= 1;
                            }
                            let below = chunk.get(lx, y, lz);
                            if y <= SEA_LEVEL || b.is_air(below) || b.has(below, rine_data::blocks::flag::WATER | rine_data::blocks::flag::LAVA | rine_data::blocks::flag::NO_COLLISION) {
                                continue;
                            }
                            let n = &b.block_of(below).name;
                            if n.ends_with("leaves") || n.ends_with("_log") {
                                continue;
                            }
                            if b.is_air(chunk.get(lx, y + 1, lz)) && b.is_air(chunk.get(lx, y + 2, lz)) {
                                return (cx * 16 + lx, y + 1, cz * 16 + lz);
                            }
                        }
                    }
                }
            }
        }
        (0, 100, 0)
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
    fn biome_climate_follows_the_vanilla_biome() {
        rine_data::init();
        let g = Generator::new(42);
        let reg = rine_data::get().registries.synced("minecraft:worldgen/biome");
        let id = |n: &str| reg.id(&format!("minecraft:{n}")).expect("biome") as u8;
        assert!(g.temperature(id("snowy_plains"), 0, 70, 0) < 0.15);
        assert!(g.temperature(id("plains"), 0, 70, 0) >= 0.15);
        assert!(!g.has_precipitation(id("desert")));
        assert!(g.has_precipitation(id("snowy_plains")));
        // not a bundled overworld biome: the warm default
        assert_eq!(g.temperature(id("the_end"), 0, 70, 0), 0.8);
    }

    #[test]
    fn generator_builds_and_bedrock_floor() {
        rine_data::init();
        let g = Generator::with_options(12345, true);
        assert!(g.vegetation.is_some());
        assert!(Generator::new(12345).vegetation.is_none(), "off by default");
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
        timing(false);
        timing(true);
    }

    fn timing(vanilla: bool) {
        rine_data::init();
        let g = Generator::with_options(2024, vanilla);
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
        println!("gen (vanilla_vegetation={vanilla}): {ms:.3} ms/chunk over {n} chunks (underground air cells in y -50..40: {underground_air})");
    }
}
