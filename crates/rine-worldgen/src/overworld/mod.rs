//! Phase 3: overworld block states after `fillFromNoise` and `buildSurface` (no carvers, features or
//! structure terrain), plus the biome of every quart cell. Mirrors `NoiseBasedChunkGenerator.doFill`,
//! `MaterialSystem.buildSurface` and `ChunkGenerator.doCreateBiomes` of vanilla 26.3.
//!
//! The biome lookup is the pure form of the `Climate.RTree` search (see `climate.rs`), so the result
//! of one lookup never depends on earlier ones. Every quart cell of a chunk uses the climate volume
//! sampled on the chunk's own 4-block grid, exactly like `createResolverForChunk`.

pub mod aquifer;
pub mod biome;
pub mod blocks;
pub mod climate;
mod rules;
mod sha256;
mod surface;

pub(crate) use surface::top_material;

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::carver::CarvingMask;
use crate::density::build::{bundled, Builder};
use crate::density::tree::{Ctx, Tree};
use crate::density::volume::Vol;
use crate::random::{AnyRng, Positional, Rng, Xoroshiro};

use aquifer::{AquiferRoots, FluidStatus, GlobalFluid};
use biome::Biomes;
use blocks::{Blk, Interner, AIR};
use rules::{Cond, RuleCompiler, Rule};

pub use biome::TemperatureModifier;

/// Quart cells of one chunk column: 4 wide, 4 deep, `height / 4` high.
pub(crate) const QUARTS: i32 = 4;

/// One `Ctx` (caching sampler context) plus the tree: the `NoiseChunk.cachingSamplers` of a chunk.
pub(crate) struct Samplers<'a> {
    pub tree: &'a Tree,
    pub ctx: Ctx,
}

impl<'a> Samplers<'a> {
    pub fn value(&mut self, id: u32, x: i32, y: i32, z: i32) -> f32 {
        self.tree.value(id, &mut self.ctx, x, y, z)
    }

    pub fn volume(&mut self, id: u32, vol: &Vol, out: &mut [f32]) {
        self.tree.volume(id, &mut self.ctx, vol, out)
    }
}

/// Generated chunk. `blocks[((y - min_y) * 16 + z) * 16 + x]` indexes `palette` (canonical block states).
/// `biomes[((qy) * 4 + qz) * 4 + qx]` (qy relative to `min_y / 4`) indexes `biome_palette`.
#[derive(Clone, Debug)]
pub struct ChunkBlocks {
    pub min_y: i32,
    pub height: i32,
    pub palette: Vec<String>,
    pub blocks: Vec<u16>,
    pub biome_palette: Vec<String>,
    pub biomes: Vec<u16>,
}

#[derive(Clone, Copy, Debug)]
struct RouterRoots {
    temperature: u32,
    vegetation: u32,
    continents: u32,
    erosion: u32,
    depth: u32,
    ridges: u32,
    preliminary_surface: u32,
    final_density: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SurfaceNoises {
    surface: usize,
    surface_secondary: usize,
    clay_bands_offset: usize,
    badlands_pillar: usize,
    badlands_pillar_roof: usize,
    badlands_surface: usize,
    iceberg_pillar: usize,
    iceberg_pillar_roof: usize,
    iceberg_surface: usize,
}

pub(crate) struct Ids {
    pub stone: Blk,
    pub water: Blk,
    pub lava: Blk,
    pub snow_block: Blk,
    pub packed_ice: Blk,
}

/// The compiled overworld generator for one seed. `Sync`: chunks can be generated in parallel.
pub struct Overworld {
    seed: i64,
    tree: Tree,
    min_y: i32,
    height: i32,
    sea_level: i32,
    biomes: Biomes,
    router: RouterRoots,
    aquifer: AquiferRoots,
    global_fluid: GlobalFluid,
    aquifer_rng: Positional,
    root_random: Positional,
    rules: Vec<Rule>,
    conds: Vec<Cond>,
    root_rule: u32,
    noises: SurfaceNoises,
    bands: [Blk; 192],
    interner: Interner,
    ids: Ids,
    /// Stored chunk biomes by chunk position. A chunk's biomes are a pure function of the seed, so the
    /// 3x3 neighbourhoods of the surface step and the decoration share them instead of recomputing the
    /// climate of nine chunks per chunk. FIFO-evicted.
    biome_grids: Mutex<(HashMap<(i32, i32), Arc<[u16]>>, VecDeque<(i32, i32)>)>,
    /// `Tree::y_free` of the six climate roots, in `compute_grid` order.
    climate_y_free: [bool; 6],
}

/// Chunk biome grids kept in memory (3 KiB each).
const BIOME_GRIDS: usize = 4096;

fn load_json(rel: &str) -> Value {
    serde_json::from_str(bundled(rel).expect("bundled json")).expect("valid bundled json")
}

fn clamp_qy(qy: i32, min_qy: i32, count: i32) -> i32 {
    qy.clamp(min_qy, min_qy + count - 1)
}

/// Index of chunk-local quart `(lx, ly, lz)` in a stored biome grid (`Vol::index` of the climate volume).
#[inline]
fn grid_index(qy_count: i32, lx: i32, ly: i32, lz: i32) -> usize {
    (ly + (lx + lz * QUARTS) * qy_count) as usize
}

/// The 3x3 chunk neighbourhood used by the surface step (`WorldGenRegion.getNoiseBiome`).
pub(crate) struct BiomeMap<'a> {
    o: &'a Overworld,
    cx: i32,
    cz: i32,
    grids: [Arc<[u16]>; 9],
}

impl Overworld {
    /// Stored biomes of chunk `(gx, gz)`, indexed by `grid_index`: `noise_biome` of every quart of its climate
    /// volume. Cached; two threads may compute the same grid, the result is the same.
    pub(crate) fn chunk_biomes(&self, gx: i32, gz: i32) -> Arc<[u16]> {
        if let Some(g) = self.biome_grids.lock().unwrap_or_else(|e| e.into_inner()).0.get(&(gx, gz)) {
            return Arc::clone(g);
        }
        let grid = compute_grid(self, gx, gz);
        let mut c = self.biome_grids.lock().unwrap_or_else(|e| e.into_inner());
        let (map, order) = &mut *c;
        if let Some(g) = map.get(&(gx, gz)) {
            return Arc::clone(g);
        }
        while map.len() >= BIOME_GRIDS {
            match order.pop_front() {
                Some(old) => {
                    map.remove(&old);
                }
                None => break,
            }
        }
        order.push_back((gx, gz));
        map.insert((gx, gz), Arc::clone(&grid));
        grid
    }
}

/// The climate volume of one chunk (4x96x4 quarts) and the biome of every quart.
fn compute_grid(o: &Overworld, gx: i32, gz: i32) -> Arc<[u16]> {
    let qy_count = o.height / 4;
    let vol = Vol::with_step(QUARTS, qy_count, QUARTS, gx * 16, o.min_y, gz * 16, 4, 4, 4);
    let mut ctx = Samplers {
        tree: &o.tree,
        ctx: Ctx::cached(o.tree.cache_count),
    };
    let size = vol.size();
    let mut fields = [vec![0f32; size], vec![0f32; size], vec![0f32; size], vec![0f32; size], vec![0f32; size], vec![0f32; size]];
    let roots = [
        o.router.temperature,
        o.router.vegetation,
        o.router.continents,
        o.router.erosion,
        o.router.depth,
        o.router.ridges,
    ];
    // Climate fields that do not vary with y (all but depth) are sampled on one quart layer and copied
    // up the column: same values, 1/96 of the noise work.
    let flat = Vol::with_step(QUARTS, 1, QUARTS, gx * 16, o.min_y, gz * 16, 4, 4, 4);
    let mut layer = vec![0f32; flat.size()];
    for ((root, buf), &y_free) in roots.iter().zip(fields.iter_mut()).zip(&o.climate_y_free) {
        if !y_free {
            ctx.volume(*root, &vol, buf);
            continue;
        }
        ctx.volume(*root, &flat, &mut layer);
        for lz in 0..QUARTS {
            for lx in 0..QUARTS {
                let v = layer[flat.index(lx, 0, lz)];
                for ly in 0..qy_count {
                    buf[vol.index(lx, ly, lz)] = v;
                }
            }
        }
    }
    debug_assert_eq!(vol.index(1, 2, 3), grid_index(qy_count, 1, 2, 3));
    (0..size)
        .map(|i| {
            let target = Biomes::target(fields[0][i], fields[1][i], fields[2][i], fields[3][i], fields[4][i], fields[5][i]);
            o.biomes.noise_biome(&target)
        })
        .collect()
}

impl<'a> BiomeMap<'a> {
    pub(crate) fn new(o: &'a Overworld, cx: i32, cz: i32) -> Self {
        let grids = std::array::from_fn(|i| o.chunk_biomes(cx + (i % 3) as i32 - 1, cz + (i / 3) as i32 - 1));
        BiomeMap { o, cx, cz, grids }
    }

    /// `ChunkAccess.getNoiseBiome`: quart coordinates clamp to the chunk's vertical range.
    pub(crate) fn quart(&self, qx: i32, qy: i32, qz: i32) -> u16 {
        let min_qy = self.o.min_y >> 2;
        let qy = clamp_qy(qy, min_qy, self.o.height / 4);
        let gx = qx.div_euclid(4);
        let gz = qz.div_euclid(4);
        let (dx, dz) = (gx - self.cx, gz - self.cz);
        assert!((-1..=1).contains(&dx) && (-1..=1).contains(&dz), "quart outside the 3x3 chunk region");
        let grid = &self.grids[((dz + 1) * 3 + (dx + 1)) as usize];
        grid[grid_index(self.o.height / 4, qx - gx * 4, qy - min_qy, qz - gz * 4)]
    }

    /// `BiomeManager.getBiome(x, y, z)` over this region.
    pub(crate) fn fiddled(&self, x: i32, y: i32, z: i32) -> u16 {
        self.o.biomes.fiddled(x, y, z, |qx, qy, qz| self.quart(qx, qy, qz))
    }
}

fn generate_bands(random: &mut AnyRng, ids: &[Blk; 7]) -> [Blk; 192] {
    let [terracotta, orange, yellow, brown, red, white, light_gray] = *ids;
    let mut bands = [terracotta; 192];

    let mut i = 0usize;
    while i < 192 {
        i += (random.next_int_bound(5) + 1) as usize;
        if i < 192 {
            bands[i] = orange;
        }
        i += 1;
    }

    let make = |random: &mut AnyRng, bands: &mut [Blk; 192], base_width: i32, state: Blk| {
        let band_count = random.next_int_bound(10) + 6;
        for _ in 0..band_count {
            let width = base_width + random.next_int_bound(3);
            let start = random.next_int_bound(192) as usize;
            let mut p = 0usize;
            while start + p < 192 && (p as i32) < width {
                bands[start + p] = state;
                p += 1;
            }
        }
    };
    make(random, &mut bands, 1, yellow);
    make(random, &mut bands, 2, brown);
    make(random, &mut bands, 1, red);

    let white_count = random.next_int_bound(7) + 9;
    let mut count = 0;
    let mut start = 0usize;
    while count < white_count && start < 192 {
        bands[start] = white;
        if start >= 2 && random.next_bool() {
            bands[start - 1] = light_gray;
        }
        if start + 1 < 192 && random.next_bool() {
            bands[start + 1] = light_gray;
        }
        count += 1;
        start += (random.next_int_bound(16) + 4) as usize;
    }
    bands
}

impl Overworld {
    /// Compiles the overworld density, aquifer and surface rules for `seed` from the bundled 26.3 data.
    pub fn new(seed: i64) -> Self {
        let settings = load_json("noise_settings/overworld.json");
        let min_y = settings["noise"]["min_y"].as_i64().expect("min_y") as i32;
        let height = settings["noise"]["height"].as_i64().expect("height") as i32;
        let sea_level = settings["sea_level"].as_i64().expect("sea_level") as i32;

        let biomes = Biomes::overworld(seed);
        let mut builder = Builder::new(seed);
        let mut df = |key: &str| -> u32 { builder.df(&settings["noise_router"][key]).expect("router entry") };
        let router = RouterRoots {
            temperature: df("temperature"),
            vegetation: df("vegetation"),
            continents: df("continents"),
            erosion: df("erosion"),
            depth: df("depth"),
            ridges: df("ridges"),
            preliminary_surface: df("chunk_surface_level"),
            final_density: df("final_density"),
        };
        let mut aq_df = |key: &str| -> u32 { builder.df(&settings["aquifers"][key]).expect("aquifer entry") };
        let aquifer = AquiferRoots {
            barrier: aq_df("barrier"),
            floodedness: aq_df("fluid_level_floodedness"),
            spread: aq_df("fluid_level_spread"),
            lava: aq_df("lava"),
            exclusion: aq_df("exclusion"),
            surface_level: aq_df("surface_level"),
        };
        let mut noise = |name: &str| -> usize { builder.noise(name).expect("surface noise") };
        let noises = SurfaceNoises {
            surface: noise("minecraft:surface"),
            surface_secondary: noise("minecraft:surface_secondary"),
            clay_bands_offset: noise("minecraft:clay_bands_offset"),
            badlands_pillar: noise("minecraft:badlands_pillar"),
            badlands_pillar_roof: noise("minecraft:badlands_pillar_roof"),
            badlands_surface: noise("minecraft:badlands_surface"),
            iceberg_pillar: noise("minecraft:iceberg_pillar"),
            iceberg_pillar_roof: noise("minecraft:iceberg_pillar_roof"),
            iceberg_surface: noise("minecraft:iceberg_surface"),
        };

        let root_random = Xoroshiro::new(seed).fork_positional();
        let mut interner = Interner::new();
        let ids = Ids {
            stone: interner.get("minecraft:stone"),
            water: interner.get("minecraft:water"),
            lava: interner.get("minecraft:lava"),
            snow_block: interner.get("minecraft:snow_block"),
            packed_ice: interner.get("minecraft:packed_ice"),
        };
        let terracotta = [
            interner.get("minecraft:terracotta"),
            interner.get("minecraft:orange_terracotta"),
            interner.get("minecraft:yellow_terracotta"),
            interner.get("minecraft:brown_terracotta"),
            interner.get("minecraft:red_terracotta"),
            interner.get("minecraft:white_terracotta"),
            interner.get("minecraft:light_gray_terracotta"),
        ];

        let (rules, conds, root_rule) = {
            let mut rc = RuleCompiler::new(&mut builder, &mut interner, &biomes.names, root_random, min_y, height);
            let root = rc
                .rule(&Value::String("minecraft:overworld".to_string()))
                .expect("overworld material rule compiles");
            (rc.rules, rc.conds, root)
        };

        let mut clay_random = root_random.from_hash_of("minecraft:clay_bands");
        let bands = generate_bands(&mut clay_random, &terracotta);

        let global_fluid = GlobalFluid {
            lava: FluidStatus {
                level: -54,
                state: ids.lava,
            },
            sea: FluidStatus {
                level: sea_level,
                state: ids.water,
            },
            sea_level,
        };
        let aquifer_rng = root_random.from_hash_of("minecraft:aquifer").fork_positional();

        let tree = builder.into_tree();
        let climate_y_free = [router.temperature, router.vegetation, router.continents, router.erosion, router.depth, router.ridges]
            .map(|r| tree.y_free(r));
        Overworld {
            seed,
            tree,
            climate_y_free,
            min_y,
            height,
            sea_level,
            biomes,
            router,
            aquifer,
            global_fluid,
            aquifer_rng,
            root_random,
            rules,
            conds,
            root_rule,
            noises,
            bands,
            interner,
            ids,
            biome_grids: Mutex::new((HashMap::new(), VecDeque::new())),
        }
    }

    /// Biome names in bundle order (indices used by `ChunkBlocks::biomes` before palette mapping).
    pub fn biome_names(&self) -> &[String] {
        &self.biomes.names
    }

    /// Quantized overworld parameter points with their biome index, in list order.
    pub fn parameter_points(&self) -> Vec<(climate::Space, u16)> {
        self.biomes.points.clone()
    }

    pub fn min_y(&self) -> i32 {
        self.min_y
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    pub fn sea_level(&self) -> i32 {
        self.sea_level
    }

    /// `Biome.getTemperature(pos, seaLevel)` for a bundle biome index (`biome_names` order).
    pub fn biome_temperature(&self, biome: u16, x: i32, y: i32, z: i32) -> f32 {
        self.biomes.temperature(biome, x, y, z, self.sea_level)
    }

    pub(crate) fn biome_has_precipitation(&self, biome: u16) -> bool {
        self.biomes.precipitation[biome as usize]
    }

    pub fn biome_quart(&self, x: i32, y: i32, z: i32) -> (i32, i32, i32) {
        self.biomes.fiddled_quart(x, y, z)
    }

    /// Biome name of quart cell `(qx, qy, qz)` in stored (unfiddled) chunk biomes.
    pub fn biome_at(&self, qx: i32, qy: i32, qz: i32) -> &str {
        let cx = qx.div_euclid(4);
        let cz = qz.div_euclid(4);
        let grid = self.chunk_biomes(cx, cz);
        let min_qy = self.min_y >> 2;
        let qy = clamp_qy(qy, min_qy, self.height / 4);
        let b = grid[grid_index(self.height / 4, qx - cx * 4, qy - min_qy, qz - cz * 4)];
        &self.biomes.names[b as usize]
    }

    /// Approximate surface height (preliminary surface level of the router) at a column. Cheap: one
    /// density sample, no aquifer or surface rules.
    pub fn height_hint(&self, x: i32, z: i32) -> i32 {
        let mut s = Samplers {
            tree: &self.tree,
            ctx: Ctx::uncached(),
        };
        s.value(self.router.preliminary_surface, x, 0, z).floor() as i32
    }

    pub(crate) fn tree(&self) -> &Tree {
        &self.tree
    }

    pub(crate) fn root_random(&self) -> Positional {
        self.root_random
    }

    pub(crate) fn bands(&self) -> &[Blk; 192] {
        &self.bands
    }

    pub(crate) fn ids(&self) -> &Ids {
        &self.ids
    }

    pub(crate) fn interner(&self) -> &Interner {
        &self.interner
    }

    pub(crate) fn biome_source(&self) -> &Biomes {
        &self.biomes
    }

    pub(crate) fn seed(&self) -> i64 {
        self.seed
    }

    /// Aquifer roots, global fluid levels and the aquifer random factory, as `NoiseChunk` builds them.
    pub(crate) fn aquifer_parts(&self) -> (&AquiferRoots, &GlobalFluid, Positional) {
        (&self.aquifer, &self.global_fluid, self.aquifer_rng)
    }

    /// Generates chunk `(cx, cz)`.
    pub fn generate(&self, cx: i32, cz: i32) -> ChunkBlocks {
        self.generate_stages(cx, cz, false).0
    }

    /// `generate` plus `applyCarvers` (see `crate::carver`). Returns the carved chunk and its carving mask.
    pub fn generate_carved(&self, cx: i32, cz: i32) -> (ChunkBlocks, CarvingMask) {
        let (chunk, mask) = self.generate_stages(cx, cz, true);
        (chunk, mask.expect("carving mask"))
    }

    /// Shared body of `generate` and `generate_carved`. With `carve`, the carver step runs after the
    /// surface step, on the same blocks and heights (`ChunkGenerator.applyCarvers`).
    pub(crate) fn generate_stages(&self, cx: i32, cz: i32, carve: bool) -> (ChunkBlocks, Option<CarvingMask>) {
        let mut samplers = Samplers {
            tree: &self.tree,
            ctx: Ctx::cached(self.tree.cache_count),
        };
        let full = Vol::new(16, self.height, 16, cx * 16, self.min_y, cz * 16);
        let column_count = (self.height as usize) * 256;

        // NoiseChunk: the aquifer prefills its surface-level grid from the shared caching context.
        let mut aq = aquifer::Aquifer::new(&mut samplers, &self.aquifer, &self.global_fluid, self.aquifer_rng, &full);

        // doFill: final density over the whole chunk, then the aquifer picks each block.
        let mut density = vec![0f32; full.size()];
        samplers.volume(self.router.final_density, &full, &mut density);
        let mut blocks = vec![AIR; column_count];
        // WORLD_SURFACE_WG first-available heights, indexed x * 16 + z.
        let mut heights = [self.min_y; 256];
        for z in 0..16 {
            for x in 0..16 {
                let col = (x * 16 + z) as usize;
                for y in (self.min_y..self.min_y + self.height).rev() {
                    let rel = y - self.min_y;
                    let d = density[full.index(x, rel, z)];
                    let state = aq
                        .compute_substance(&mut samplers, cx * 16 + x, y, cz * 16 + z, d as f64)
                        .unwrap_or(self.ids.stone);
                    if state != AIR {
                        blocks[((rel as usize) * 16 + z as usize) * 16 + x as usize] = state;
                        if y >= heights[col] {
                            heights[col] = y + 1;
                        }
                    }
                }
            }
        }
        drop(aq);

        let biome_map = BiomeMap::new(self, cx, cz);
        surface::build(self, samplers, &biome_map, cx, cz, &mut blocks, &mut heights);

        let mask = if carve {
            Some(crate::carver::apply_carvers(self, cx, cz, &mut blocks, &mut heights, &biome_map))
        } else {
            None
        };

        // Output.
        let mut biomes = vec![0u16; (self.height as usize / 4) * 16];
        let own = &biome_map.grids[4];
        let qy_count = self.height / 4;
        for qy in 0..qy_count {
            for qz in 0..4 {
                for qx in 0..4 {
                    let b = own[grid_index(qy_count, qx, qy, qz)];
                    biomes[((qy * 4 + qz) * 4 + qx) as usize] = b;
                }
            }
        }
        let chunk = ChunkBlocks {
            min_y: self.min_y,
            height: self.height,
            palette: self.interner.names().to_vec(),
            blocks: blocks.iter().map(|b| b.0).collect(),
            biome_palette: self.biomes.names.clone(),
            biomes,
        };
        (chunk, mask)
    }
}

#[cfg(test)]
mod identity_test {
    /// Proto output (blocks, biomes, palette) of a few chunks across the seed space, pinned by SHA-256.
    /// Proto speedups must keep these digests: the values were taken before the biome-grid cache, the flat
    /// climate layer and the surface memo, and stayed the same through them.
    #[test]
    fn proto_digests_are_pinned() {
        let o = super::Overworld::new(-4172144997902289642);
        let mut got = Vec::new();
        for (cx, cz) in [(-100, -150), (11, -150), (159, -97), (-63, 9), (85, 115), (122, 168)] {
            let (c, _mask) = o.generate_carved(cx, cz);
            let mut bytes = Vec::with_capacity(c.blocks.len() * 2 + c.biomes.len() * 2);
            for v in c.blocks.iter().chain(&c.biomes) {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            for n in c.palette.iter().chain(&c.biome_palette) {
                bytes.extend_from_slice(n.as_bytes());
                bytes.push(0);
            }
            let d = super::sha256::digest(&bytes);
            got.push(d[..8].iter().map(|b| format!("{b:02x}")).collect::<String>());
        }
        let expect: [&str; 6] = EXPECT;
        assert_eq!(got, expect);
    }

    const EXPECT: [&str; 6] = [
        "89ce27308f973a84",
        "d1dc17a7246728c4",
        "80001516c4240c5d",
        "20065323724d1e95",
        "cfa449213e22e488",
        "0f2aaa8caa75aa1b",
    ];
}

#[cfg(test)]
mod y_free_test {
    #[test]
    fn climate_fields_except_depth_are_flat() {
        let o = super::Overworld::new(1);
        assert_eq!(o.climate_y_free, [true, true, true, true, false, true]);
    }
}
