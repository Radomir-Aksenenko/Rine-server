//! Ore veins and rock blobs (dirt, gravel, granite, diorite, andesite, tuff).
//! Placed per chunk from hashes; veins and blobs are clipped at the chunk border.

use super::biome::Biome;
use super::grid::Grid;
use super::noise::{hash, Rng, SALT_BLOB, SALT_ORE};
use super::terrain::Column;
use super::Ids;

struct Ore {
    attempts: u32,
    y_min: i32,
    y_max: i32,
    size: u32,
    /// Only generated in mountain biomes, with a 25% roll per attempt.
    mountain: bool,
}

/// Order matches `Ids::ore_stone` / `Ids::ore_deep`.
const ORES: [Ore; 8] = [
    Ore { attempts: 20, y_min: 0, y_max: 192, size: 17, mountain: false }, // coal
    Ore { attempts: 10, y_min: -24, y_max: 56, size: 9, mountain: false }, // iron
    Ore { attempts: 6, y_min: -16, y_max: 112, size: 10, mountain: false }, // copper
    Ore { attempts: 4, y_min: -64, y_max: 32, size: 9, mountain: false }, // gold
    Ore { attempts: 8, y_min: -64, y_max: 15, size: 8, mountain: false }, // redstone
    Ore { attempts: 2, y_min: -32, y_max: 32, size: 7, mountain: false }, // lapis
    Ore { attempts: 3, y_min: -64, y_max: 16, size: 4, mountain: false }, // diamond
    Ore { attempts: 1, y_min: 4, y_max: 32, size: 1, mountain: true },   // emerald
];

struct Blob {
    attempts: u32,
    y_min: i32,
    y_max: i32,
    /// Replaces only deepslate (tuff).
    deep_only: bool,
}

/// Order matches `Ids::blobs`: dirt, gravel, granite, diorite, andesite, tuff.
const BLOBS: [Blob; 6] = [
    Blob { attempts: 10, y_min: -64, y_max: 255, deep_only: false },
    Blob { attempts: 8, y_min: -64, y_max: 255, deep_only: false },
    Blob { attempts: 2, y_min: -64, y_max: 60, deep_only: false },
    Blob { attempts: 2, y_min: -64, y_max: 60, deep_only: false },
    Blob { attempts: 2, y_min: -64, y_max: 60, deep_only: false },
    Blob { attempts: 2, y_min: -64, y_max: 0, deep_only: true },
];

fn is_mountain(b: Biome) -> bool {
    matches!(b, Biome::StonyPeaks | Biome::SnowySlopes | Biome::WindsweptHills)
}

pub fn place(grid: &mut Grid, ids: &Ids, seed: u64, cx: i32, cz: i32, cols: &[Column]) {
    for (k, b) in BLOBS.iter().enumerate() {
        let mut rng = Rng::new(hash(seed, cx as i64, cz as i64, k as i64, SALT_BLOB));
        let state = ids.blobs[k];
        for _ in 0..b.attempts {
            let (cx_l, cz_l) = (rng.range(0, 16), rng.range(0, 16));
            let cy = rng.range(b.y_min, b.y_max + 1);
            let r = 2 + rng.below(3) as i32;
            for dy in -r..=r {
                for dz in -r..=r {
                    for dx in -r..=r {
                        if dx * dx + dy * dy + dz * dz > r * r {
                            continue;
                        }
                        let (x, y, z) = (cx_l + dx, cy + dy, cz_l + dz);
                        if !Grid::in_bounds(x, y, z) {
                            continue;
                        }
                        let cur = grid.get(x, y, z);
                        let ok = if b.deep_only {
                            cur == ids.deepslate
                        } else {
                            cur == ids.stone || cur == ids.deepslate
                        };
                        if ok && rng.chance(0.8) {
                            grid.set(x, y, z, state);
                        }
                    }
                }
            }
        }
    }

    for (k, o) in ORES.iter().enumerate() {
        let mut rng = Rng::new(hash(seed, cx as i64, cz as i64, k as i64, SALT_ORE));
        for _ in 0..o.attempts {
            let (lx, lz) = (rng.range(0, 16), rng.range(0, 16));
            let y = rng.range(o.y_min, o.y_max + 1);
            if o.mountain {
                let roll = rng.chance(0.25);
                if !roll || !is_mountain(cols[(lz * 16 + lx) as usize].biome) {
                    continue;
                }
            }
            let (mut x, mut yy, mut z) = (lx, y, lz);
            for _ in 0..o.size {
                if Grid::in_bounds(x, yy, z) {
                    let cur = grid.get(x, yy, z);
                    let rock = cur == ids.stone
                        || cur == ids.deepslate
                        || ids.blobs[2..].contains(&cur);
                    if rock {
                        let st = if cur == ids.deepslate { ids.ore_deep[k] } else { ids.ore_stone[k] };
                        grid.set(x, yy, z, st);
                    }
                }
                match rng.below(6) {
                    0 => x += 1,
                    1 => x -= 1,
                    2 => yy += 1,
                    3 => yy -= 1,
                    4 => z += 1,
                    _ => z -= 1,
                }
            }
        }
    }
}
