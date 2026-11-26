//! Phase 4: the overworld carvers (`ChunkGenerator.applyCarvers` of vanilla 26.3, i.e.
//! `NoiseBasedChunkGenerator.generateCarvers` + `applyCarvingMask`).
//!
//! Two parts, as in vanilla:
//! 1. `carve`: for every source chunk within 8 chunks, each configured carver of its biome (`cave`,
//!    `cave_extra_underground`, `canyon`) draws from a random seeded by the world seed, the source chunk
//!    and the carver index, and sets bits of the `CarvingMask`. No block is touched.
//! 2. `apply_mask`: the bits are walked in mask order (`CarvingMask.visit`), and each carved block takes
//!    the aquifer's substance (air, water or lava) unless it is bedrock. Dirt under a carved grass or
//!    mycelium column becomes the surface top material (`topMaterial`).
//!
//! The blocks are changed in place, and the WORLD_SURFACE_WG heights are updated as `ProtoChunk.setBlockState`
//! does, because the top material reads them.

pub mod carvers;
pub mod mask;

use std::sync::OnceLock;

use crate::density::tree::Ctx;
use crate::volume::Vol;
use crate::overworld::aquifer::Aquifer;
use crate::overworld::blocks::{Blk, AIR};
use crate::overworld::{top_material, BiomeMap, Overworld, Samplers};
use crate::random::{Legacy, Rng};

pub use mask::CarvingMask;

use carvers::{Carver, Env, CANYON, CAVE, CAVE_EXTRA_UNDERGROUND};

/// `overworld_carvers.txt`: carvers of every overworld biome, bundled from the 26.3 biome json.
const CARVERS_TXT: &str = include_str!("../../assets/worldgen/overworld_carvers.txt");

/// Carver list shared by every overworld biome. The bundled table has one list for all biomes (checked
/// here): then the list does not depend on the source biome, and the biome lookups of `generateCarvers`
/// are not needed. A table with different lists panics, since the per-source biome lookup is not ported.
fn overworld_carvers() -> &'static [Carver] {
    static LIST: OnceLock<Vec<Carver>> = OnceLock::new();
    LIST.get_or_init(|| {
        let mut common: Option<Vec<Carver>> = None;
        for line in CARVERS_TXT.lines().filter(|l| !l.starts_with('#')) {
            let mut parts = line.split(' ');
            let _biome = parts.next().expect("biome name");
            let list = parts.next().unwrap_or("");
            let carvers: Vec<Carver> = list
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|name| match name {
                    "cave" => Carver::Cave(&CAVE),
                    "cave_extra_underground" => Carver::Cave(&CAVE_EXTRA_UNDERGROUND),
                    "canyon" => Carver::Canyon(&CANYON),
                    other => panic!("carver {other} is not ported"),
                })
                .collect();
            match &common {
                None => common = Some(carvers),
                Some(first) => assert!(
                    first.len() == carvers.len() && first.iter().zip(&carvers).all(|(a, b)| same_carver(a, b)),
                    "overworld biomes have different carver lists; per-source biome lookup is required"
                ),
            }
        }
        common.expect("overworld carver table")
    })
}

fn same_carver(a: &Carver, b: &Carver) -> bool {
    match (a, b) {
        (Carver::Cave(x), Carver::Cave(y)) => std::ptr::eq(*x, *y),
        (Carver::Canyon(x), Carver::Canyon(y)) => std::ptr::eq(*x, *y),
        _ => false,
    }
}

/// `Heightmap.Types.WORLD_SURFACE_WG` update for one block change (`Heightmap.update` with the NOT_AIR
/// predicate). `heights` holds the first available y (highest block + 1) per column, `x * 16 + z`.
#[inline]
fn update_world_surface(blocks: &[Blk], heights: &mut [i32; 256], min_y: i32, x: i32, y: i32, z: i32, state: Blk) {
    let col = (x * 16 + z) as usize;
    let first = heights[col];
    if y <= first - 2 {
        return;
    }
    if state != AIR {
        if y >= first {
            heights[col] = y + 1;
        }
    } else if first - 1 == y {
        let mut yy = y - 1;
        while yy >= min_y {
            if blocks[(((yy - min_y) * 16 + z) * 16 + x) as usize] != AIR {
                heights[col] = yy + 1;
                return;
            }
            yy -= 1;
        }
        heights[col] = min_y;
    }
}

/// `ProtoChunk.setBlockState` for the carver step: writes the block, then updates the heightmap.
#[inline]
fn set_block(blocks: &mut [Blk], heights: &mut [i32; 256], min_y: i32, x: i32, y: i32, z: i32, state: Blk) {
    blocks[(((y - min_y) * 16 + z) * 16 + x) as usize] = state;
    update_world_surface(blocks, heights, min_y, x, y, z, state);
}

/// `NoiseBasedChunkGenerator.generateCarvers` + `applyCarvingMask` for chunk `(cx, cz)`. `blocks` and
/// `heights` are the surface-stage result and are updated in place. Returns the carving mask.
pub(crate) fn apply_carvers<'a>(
    o: &'a Overworld,
    cx: i32,
    cz: i32,
    blocks: &mut Vec<Blk>,
    heights: &mut [i32; 256],
    biomes: &BiomeMap<'a>,
) -> CarvingMask {
    let min_gen_y = o.min_y();
    let gen_depth = o.height();
    // `new CarvingMask(minGenY + 1, maxY)`, maxY = minGenY + genDepth - 1 - 7 (protected blocks on top).
    let mut mask = CarvingMask::new(min_gen_y + 1, min_gen_y + gen_depth - 1 - 7);
    let env = Env { cx, cz, min_gen_y, gen_depth };
    let carvers = overworld_carvers();
    let seed = o.seed();
    let mut rng = Legacy::new(0);

    for dx in -8..=8 {
        for dz in -8..=8 {
            let (sx, sz) = (cx + dx, cz + dz);
            for (index, carver) in carvers.iter().enumerate() {
                // random.setLargeFeatureSeed(seed + index, sourcePos.x(), sourcePos.z())
                let feature_seed = seed.wrapping_add(index as i64);
                rng.set_seed(feature_seed);
                let x_scale = rng.next_long();
                let z_scale = rng.next_long();
                rng.set_seed(((sx as i64).wrapping_mul(x_scale)) ^ ((sz as i64).wrapping_mul(z_scale)) ^ feature_seed);
                if carver.is_start_chunk(&mut rng) {
                    carver.carve(&env, &mut rng, (sx, sz), &mut mask);
                }
            }
        }
    }

    if !mask.is_empty() {
        apply_mask(o, cx, cz, blocks, heights, biomes, &mask);
    }
    mask
}

/// Per block-state flags, indexed by `Blk`, computed once per chunk.
struct Kinds {
    bedrock: Vec<bool>,
    grass_or_mycelium: Vec<bool>,
    dirt: Vec<bool>,
    fluid: Vec<bool>,
}

impl Kinds {
    fn new(o: &Overworld) -> Self {
        let interner = o.interner();
        let n = interner.names().len();
        let mut k = Kinds {
            bedrock: vec![false; n],
            grass_or_mycelium: vec![false; n],
            dirt: vec![false; n],
            fluid: vec![false; n],
        };
        for i in 0..n {
            let b = Blk(i as u16);
            k.bedrock[i] = interner.is_block(b, "minecraft:bedrock");
            k.grass_or_mycelium[i] = interner.is_block(b, "minecraft:grass_block") || interner.is_block(b, "minecraft:mycelium");
            k.dirt[i] = interner.is_block(b, "minecraft:dirt");
            k.fluid[i] = interner.is_fluid(b);
        }
        k
    }
}

/// `applyCarvingMask`. The aquifer and the surface samplers are rebuilt for this pass: their caches are
/// pure memos of the chunk, so the result is the same as the surface step's objects.
fn apply_mask<'a>(
    o: &'a Overworld,
    cx: i32,
    cz: i32,
    blocks: &mut Vec<Blk>,
    heights: &mut [i32; 256],
    biomes: &BiomeMap<'a>,
    mask: &CarvingMask,
) {
    let min_y = o.min_y();
    let height = o.height();
    let kinds = Kinds::new(o);
    let (roots, global, rng) = o.aquifer_parts();
    let full = Vol::new(16, height, 16, cx * 16, min_y, cz * 16);
    let mut samplers = Samplers {
        tree: o.tree(),
        ctx: Ctx::cached(o.tree().cache_count),
    };
    let mut aquifer = Aquifer::new(&mut samplers, roots, global, rng, &full);

    mask.visit(|x, z, bottom_y, top_y| {
        let mut has_grass = false;
        let world_x = cx * 16 + x;
        let world_z = cz * 16 + z;
        let mut world_y = top_y;
        while world_y >= bottom_y {
            let state = blocks[(((world_y - min_y) * 16 + z) * 16 + x) as usize];
            if !kinds.bedrock[state.0 as usize] {
                if kinds.grass_or_mycelium[state.0 as usize] {
                    has_grass = true;
                }
                if let Some(new_state) = aquifer.compute_substance(&mut samplers, world_x, world_y, world_z, 0.0) {
                    set_block(blocks, heights, min_y, x, world_y, z, new_state);
                    if has_grass {
                        let below = world_y - 1;
                        let under = blocks[(((below - min_y) * 16 + z) * 16 + x) as usize];
                        if kinds.dirt[under.0 as usize] {
                            let fresh = Samplers {
                                tree: o.tree(),
                                ctx: Ctx::cached(o.tree().cache_count),
                            };
                            let under_fluid = kinds.fluid[new_state.0 as usize];
                            if let Some(top) = top_material(o, fresh, biomes, cx, cz, blocks, heights, world_x, below, world_z, under_fluid) {
                                set_block(blocks, heights, min_y, x, below, z, top);
                            }
                        }
                    }
                }
            }
            world_y -= 1;
        }
    });
}
