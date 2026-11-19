//! Trees. Candidate positions and shapes come from hashes of (seed, chunk, index),
//! so every chunk derives the same trees in its 3x3 neighbourhood. A chunk writes
//! only the cells it owns. Writes are order-independent: logs replace air, snow
//! layer or leaves; leaves replace air or snow layer only.

use super::biome::Biome;
use super::grid::Grid;
use super::noise::{hash, Rng, SALT_TREE};
use super::terrain::{Corners, SEA_LEVEL};
use super::Ids;

const CANDIDATES: u32 = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Oak = 0,
    BigOak = 1,
    Birch = 2,
    Spruce = 3,
    Acacia = 4,
    Jungle = 5,
}

fn choose(b: Biome, roll: f32, pick: f32) -> Option<Kind> {
    let (p, main) = match b {
        Biome::Forest | Biome::DarkForest => (0.8, Kind::Oak),
        Biome::BirchForest => (0.8, Kind::Birch),
        Biome::Taiga => (0.7, Kind::Spruce),
        Biome::SnowyTaiga => (0.5, Kind::Spruce),
        Biome::Jungle => (0.5, Kind::Jungle),
        Biome::Savanna => (0.08, Kind::Acacia),
        Biome::Swamp => (0.3, Kind::Oak),
        Biome::Plains | Biome::SunflowerPlains => (0.01, Kind::Oak),
        Biome::WindsweptHills => (0.04, Kind::Spruce),
        _ => return None,
    };
    if roll >= p {
        return None;
    }
    if main == Kind::Oak && pick < 0.15 {
        Some(Kind::BigOak)
    } else {
        Some(main)
    }
}

/// Places the trees of the 3x3 chunk neighbourhood that reach chunk (cx, cz).
pub fn place(grid: &mut Grid, ids: &Ids, seed: u64, cx: i32, cz: i32, corners: &Corners) {
    for dz in -1..=1 {
        for dx in -1..=1 {
            let (ncx, ncz) = (cx + dx, cz + dz);
            for k in 0..CANDIDATES {
                let mut rng = Rng::new(hash(seed, ncx as i64, ncz as i64, k as i64, SALT_TREE));
                let x = ncx * 16 + rng.range(0, 16);
                let z = ncz * 16 + rng.range(0, 16);
                let roll = rng.next_f32();
                let pick = rng.next_f32();
                let col = corners.column(seed, x, z);
                let Some(kind) = choose(col.biome, roll, pick) else {
                    continue;
                };
                if col.surface < SEA_LEVEL {
                    continue;
                }
                let mut w = Writer { grid: &mut *grid, ids, ox: cx * 16, oz: cz * 16 };
                grow(&mut w, &mut rng, kind, x, col.surface + 1, z);
            }
        }
    }
}

struct Writer<'a> {
    grid: &'a mut Grid,
    ids: &'a Ids,
    ox: i32,
    oz: i32,
}

impl Writer<'_> {
    fn log(&mut self, x: i32, y: i32, z: i32, kind: Kind) {
        let (lx, lz) = (x - self.ox, z - self.oz);
        if !Grid::in_bounds(lx, y, lz) {
            return;
        }
        let cur = self.grid.get(lx, y, lz);
        if cur == self.ids.air || cur == self.ids.snow_layer || self.ids.leaves.contains(&cur) {
            self.grid.set(lx, y, lz, self.ids.logs[kind as usize]);
        }
    }

    fn leaf(&mut self, x: i32, y: i32, z: i32, kind: Kind) {
        let (lx, lz) = (x - self.ox, z - self.oz);
        if !Grid::in_bounds(lx, y, lz) {
            return;
        }
        let cur = self.grid.get(lx, y, lz);
        if cur == self.ids.air || cur == self.ids.snow_layer {
            self.grid.set(lx, y, lz, self.ids.leaves[kind as usize]);
        }
    }
}

/// Grows one tree whose trunk starts at (x, y, z).
fn grow(w: &mut Writer, rng: &mut Rng, kind: Kind, x: i32, y: i32, z: i32) {
    // Leaf layers as (offset from trunk top, radius).
    let mut layers: Vec<(i32, i32)> = Vec::new();
    let trunk = match kind {
        Kind::Oak => {
            layers.extend_from_slice(&[(-3, 2), (-2, 2), (-1, 2), (0, 1), (1, 0)]);
            rng.range(4, 7)
        }
        Kind::BigOak => {
            layers.extend_from_slice(&[(-4, 3), (-3, 3), (-2, 3), (-1, 2), (0, 2), (1, 1)]);
            rng.range(6, 9)
        }
        Kind::Birch => {
            layers.extend_from_slice(&[(-3, 2), (-2, 2), (-1, 2), (0, 1), (1, 0)]);
            rng.range(5, 8)
        }
        Kind::Jungle => {
            layers.extend_from_slice(&[(-2, 2), (-1, 2), (0, 1)]);
            rng.range(5, 8)
        }
        Kind::Acacia => {
            layers.extend_from_slice(&[(-1, 2), (0, 3), (1, 2)]);
            rng.range(4, 7)
        }
        Kind::Spruce => {
            let t = rng.range(6, 9);
            for d in 0..=(t - 2) {
                let r = if d == 0 || d % 2 == 1 { 1 } else { 2 };
                layers.push((-d, r));
            }
            layers.push((1, 0));
            t
        }
    };

    for dy in 0..trunk {
        w.log(x, y + dy, z, kind);
    }
    for &(rel, r) in &layers {
        let ly = y + trunk + rel;
        for dz in -r..=r {
            for dx in -r..=r {
                if r >= 2 && dx.abs() == r && dz.abs() == r && rng.chance(0.5) {
                    continue;
                }
                w.leaf(x + dx, ly, z + dz, kind);
            }
        }
    }
}

/// Trees on the vanilla terrain, kept inside this chunk (trunks at least 3
/// blocks from the edge) until vanilla feature placement is ported.
pub fn place_local(grid: &mut Grid, ids: &Ids, seed: u64, cx: i32, cz: i32, cols: &[super::terrain::Column], enabled: &[bool; 256]) {
    for k in 0..CANDIDATES * 2 {
        let mut rng = Rng::new(hash(seed, cx as i64, cz as i64, k as i64, SALT_TREE));
        let lx = rng.range(3, 13);
        let lz = rng.range(3, 13);
        let roll = rng.next_f32() * 1.6;
        let pick = rng.next_f32();
        let column = (lz * 16 + lx) as usize;
        if !enabled[column] {
            continue;
        }
        let col = cols[column];
        let Some(kind) = choose(col.biome, roll, pick) else {
            continue;
        };
        if col.surface < SEA_LEVEL {
            continue;
        }
        let ground = grid.get(lx, col.surface, lz);
        if ground != ids.grass && ground != ids.grass_snowy && ground != ids.dirt {
            continue;
        }
        let (x, z) = (cx * 16 + lx, cz * 16 + lz);
        let mut w = Writer { grid: &mut *grid, ids, ox: cx * 16, oz: cz * 16 };
        grow(&mut w, &mut rng, kind, x, col.surface + 1, z);
    }
}
