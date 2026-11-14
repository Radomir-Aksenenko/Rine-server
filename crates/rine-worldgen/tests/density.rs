//! Bit-exact comparison of the overworld density port against vanilla 26.3.
//!
//! Golden data comes from `tools/noise-golden/DensityGolden.java` (run `run_density.sh`):
//! - `density_points.txt`: `router.<field>.<k> <float bits>` at the POINTS below (uncached point path).
//! - `density_chunks.bin`: big-endian f32 bits of `final_density` for CHUNKS, laid out as
//!   `((x * 16 + z) * 384 + (y - -64))`, i.e. Java's `NoiseBasedChunkGenerator.doFill` order.

use std::collections::HashMap;
use std::time::Instant;

use rine_worldgen::{RouterField, Terrain};

const SEED: i64 = 1592598566;
const MIN_Y: i32 = -64;
const HEIGHT: i32 = 384;

const POINTS: [[i32; 3]; 41] = [
    [0, 0, 0],
    [1, 64, 1],
    [-100, -64, 200],
    [1000, -300, -1000],
    [123456, 80, -654321],
    [-30000000, 0, 30000000],
    [5, 0, 7],
    [-17, 3, 40],
    [16, 1, -16],
    [-1, -1, -1],
    [300, 120, 300],
    [-2000, 15, 4000],
    [9, 0, 0],
    [0, 0, 9],
    [-4, 200, 5],
    [77, -20, -77],
    [2048, 63, -2048],
    [1, 2, 3],
    [-65536, 17, 65535],
    [31, 0, -31],
    [8, 63, 8],
    [8, 64, 8],
    [15, -64, 15],
    [16, -63, 16],
    [-16, 320, -16],
    [100, -30, 100],
    [100, 0, 100],
    [100, 30, 100],
    [100, 60, 100],
    [100, 90, 100],
    [-200, 5, 37],
    [-201, 6, 38],
    [512, 128, -512],
    [513, 129, -513],
    [4, 5, 6],
    [7, 8, 9],
    [-7, -8, -9],
    [12, 40, -44],
    [88, 73, 19],
    [-5000, 100, 5000],
    [3, 255, 3],
];

const FIELDS: [(&str, RouterField); 8] = [
    ("temperature", RouterField::Temperature),
    ("vegetation", RouterField::Vegetation),
    ("continents", RouterField::Continents),
    ("erosion", RouterField::Erosion),
    ("depth", RouterField::Depth),
    ("ridges", RouterField::Ridges),
    ("final_density", RouterField::FinalDensity),
    ("chunk_surface_level", RouterField::ChunkSurfaceLevel),
];

const CHUNKS: [(i32, i32); 3] = [(0, 0), (5, -3), (-17, 40)];

const POINT_GOLDEN: &str = include_str!("density_points.txt");
const CHUNK_GOLDEN: &[u8] = include_bytes!("density_chunks.bin");

fn parse_points() -> HashMap<String, i32> {
    let mut map = HashMap::new();
    for line in POINT_GOLDEN.lines() {
        let (label, bits) = line.rsplit_once(' ').expect("label and value");
        let bits: i32 = bits.parse().expect("integer bits");
        assert!(map.insert(label.to_string(), bits).is_none(), "duplicate {label}");
    }
    map
}

#[test]
fn router_points_match_java_bits() {
    let terrain = Terrain::new(SEED);
    let golden = parse_points();
    let mut checked = 0;
    for (name, field) in FIELDS {
        for (k, p) in POINTS.iter().enumerate() {
            let label = format!("router.{name}.{k}");
            let want = *golden.get(&label).unwrap_or_else(|| panic!("golden has no {label}"));
            let got = terrain.router_point(field, p[0], p[1], p[2]);
            assert_eq!(
                got.to_bits() as i32,
                want,
                "bit mismatch at {label} point {p:?}: got {got} ({:#x}), want bits {want:#x}",
                got.to_bits()
            );
            checked += 1;
        }
    }
    assert_eq!(checked, golden.len(), "every golden point must be checked");
}

fn golden_chunk(index: usize) -> &'static [u8] {
    let per_chunk = 16 * 16 * HEIGHT as usize * 4;
    &CHUNK_GOLDEN[index * per_chunk..(index + 1) * per_chunk]
}

#[test]
fn chunk_final_density_matches_java_bits() {
    assert_eq!(CHUNK_GOLDEN.len(), CHUNKS.len() * 16 * 16 * HEIGHT as usize * 4);
    let terrain = Terrain::new(SEED);
    for (ci, &(cx, cz)) in CHUNKS.iter().enumerate() {
        let chunk = terrain.chunk_density(cx, cz);
        assert_eq!(chunk.min_y, MIN_Y);
        assert_eq!(chunk.height, HEIGHT);
        let bytes = golden_chunk(ci);
        let mut mismatches = Vec::new();
        for x in 0..16 {
            for z in 0..16 {
                for yi in 0..HEIGHT {
                    let idx = ((x * 16 + z) * HEIGHT + yi) as usize * 4;
                    let want = i32::from_be_bytes(bytes[idx..idx + 4].try_into().unwrap());
                    let y = MIN_Y + yi;
                    let got = chunk.density(x, y, z);
                    if got.to_bits() as i32 != want {
                        mismatches.push((x, y, z, got, f32::from_bits(want as u32)));
                    }
                }
            }
        }
        assert!(
            mismatches.is_empty(),
            "chunk {:?}: {} mismatched blocks, first: {:?}",
            (cx, cz),
            mismatches.len(),
            &mismatches[..mismatches.len().min(8)]
        );
    }
}

#[test]
fn chunk_solidity_matches_java_sign() {
    let terrain = Terrain::new(SEED);
    for (ci, &(cx, cz)) in CHUNKS.iter().enumerate() {
        let chunk = terrain.chunk_density(cx, cz);
        let bytes = golden_chunk(ci);
        for x in 0..16 {
            for z in 0..16 {
                for yi in 0..HEIGHT {
                    let idx = ((x * 16 + z) * HEIGHT + yi) as usize * 4;
                    let want = f32::from_bits(u32::from_be_bytes(bytes[idx..idx + 4].try_into().unwrap()));
                    let y = MIN_Y + yi;
                    assert_eq!(chunk.is_solid(x, y, z), want > 0.0, "solidity at {cx},{cz} {x},{y},{z}");
                }
            }
        }
    }
}

/// Timing of `chunk_density` in release: `cargo test -p rine-worldgen --release -- --nocapture`.
#[test]
fn chunk_density_timing() {
    let terrain = Terrain::new(SEED);
    let _ = terrain.chunk_density(0, 0);
    let rounds = 20;
    let start = Instant::now();
    let mut acc = 0f32;
    for i in 0..rounds {
        let c = terrain.chunk_density(i, -i);
        acc += c.density(3, 70, 5);
    }
    let per = start.elapsed() / rounds as u32;
    println!("chunk_density: {per:?} per chunk (checksum {acc})");
}
