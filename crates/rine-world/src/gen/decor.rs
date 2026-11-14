//! Ground cover: grass, tall grass, flowers, pumpkins, dead bushes, cactus and sugar cane.
//! Runs after trees and only fills air cells of this chunk.

use super::biome::Biome;
use super::grid::Grid;
use super::noise::{hash, unit, SALT_DECOR};
use super::terrain::{Column, Corners, SEA_LEVEL};
use super::Ids;

pub fn place(grid: &mut Grid, ids: &Ids, seed: u64, cx: i32, cz: i32, cols: &[Column], corners: &Corners) {
    let (ox, oz) = (cx * 16, cz * 16);
    for lz in 0..16i32 {
        for lx in 0..16i32 {
            let col = cols[(lz * 16 + lx) as usize];
            let s = col.surface;
            if s < SEA_LEVEL {
                continue;
            }
            let y = s + 1;
            if grid.get(lx, y, lz) != ids.air {
                continue;
            }
            let (wx, wz) = (ox + lx, oz + lz);
            let r = unit(hash(seed, wx as i64, wz as i64, 0, SALT_DECOR));
            let r2 = unit(hash(seed, wx as i64, wz as i64, 1, SALT_DECOR));
            match col.biome {
                Biome::Desert => {
                    if r < 0.01 {
                        grid.set(lx, y, lz, ids.dead_bush);
                    } else if r < 0.016 {
                        let h = 1 + (r2 * 3.0) as i32;
                        column(grid, ids.air, lx, y, lz, ids.cactus, h);
                    }
                }
                Biome::Beach => {
                    if s == SEA_LEVEL && r < 0.5 && water_next(corners, seed, wx, wz) {
                        let h = 1 + (r2 * 3.0) as i32;
                        column(grid, ids.air, lx, y, lz, ids.sugar_cane, h);
                    }
                }
                Biome::Plains
                | Biome::SunflowerPlains
                | Biome::Forest
                | Biome::BirchForest
                | Biome::DarkForest
                | Biome::Taiga
                | Biome::Jungle
                | Biome::Savanna
                | Biome::Swamp => {
                    let (grass_p, tall_p, flower_p) = match col.biome {
                        Biome::Jungle => (0.35, 0.06, 0.0),
                        Biome::Plains | Biome::SunflowerPlains => (0.3, 0.05, 0.04),
                        Biome::Forest | Biome::BirchForest | Biome::DarkForest => (0.2, 0.03, 0.02),
                        _ => (0.15, 0.02, 0.0),
                    };
                    if r < grass_p {
                        grid.set(lx, y, lz, ids.short_grass);
                    } else if r < grass_p + tall_p {
                        if grid.get(lx, y + 1, lz) == ids.air {
                            grid.set(lx, y, lz, ids.tall_lower);
                            grid.set(lx, y + 1, lz, ids.tall_upper);
                        }
                    } else if r < grass_p + tall_p + flower_p {
                        let i = ((r2 * 5.0) as usize).min(4);
                        grid.set(lx, y, lz, ids.flowers[i]);
                    } else if col.biome == Biome::Plains && r2 > 0.998 {
                        grid.set(lx, y, lz, ids.pumpkin);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Stacks up to `h` blocks of `state` upward from (lx, y, lz), only into air.
fn column(grid: &mut Grid, air: u16, lx: i32, y: i32, lz: i32, state: u16, h: i32) {
    for dy in 0..h {
        if !Grid::in_bounds(lx, y + dy, lz) || grid.get(lx, y + dy, lz) != air {
            break;
        }
        grid.set(lx, y + dy, lz, state);
    }
}

/// True if a horizontally adjacent column holds water at sea level.
fn water_next(corners: &Corners, seed: u64, x: i32, z: i32) -> bool {
    [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|&(dx, dz)| {
        let n = corners.column(seed, x + dx, z + dz);
        n.surface < SEA_LEVEL && !n.biome.is_frozen_water()
    })
}
