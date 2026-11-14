//! Phase 4 golden test: overworld carvers (CARVERS step of vanilla 26.3) against the vanilla jar.
//! Data from `tools/noise-golden/CarverGolden.java`: `tests/carver_golden.bin`, `tests/carver_sin_table.bin`,
//! `tests/carver_biomes.txt`.

use std::fs;
use std::path::PathBuf;

use rine_worldgen::carver::trig;
use rine_worldgen::Overworld;

const SEED: i64 = 1592598566;

fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

struct Reader {
    data: Vec<u8>,
    pos: usize,
}

impl Reader {
    fn take<const N: usize>(&mut self) -> [u8; N] {
        let v: [u8; N] = self.data[self.pos..self.pos + N].try_into().unwrap();
        self.pos += N;
        v
    }
    fn i32(&mut self) -> i32 {
        i32::from_be_bytes(self.take())
    }
    fn u16(&mut self) -> u16 {
        u16::from_be_bytes(self.take())
    }
    fn i64(&mut self) -> i64 {
        i64::from_be_bytes(self.take())
    }
    fn strings(&mut self) -> Vec<String> {
        let n = self.i32() as usize;
        (0..n)
            .map(|_| {
                let len = self.u16() as usize;
                let s = String::from_utf8(self.data[self.pos..self.pos + len].to_vec()).unwrap();
                self.pos += len;
                s
            })
            .collect()
    }
}

struct GoldenChunk {
    cx: i32,
    cz: i32,
    /// Block state name per cell, `((y - minY) * 16 + z) * 16 + x`.
    blocks: Vec<String>,
    /// `CarvingMask` words (`BitSet.toLongArray`).
    mask_words: Vec<u64>,
    mask_min_y: i32,
    mask_max_y: i32,
    dirt_to_grass: i32,
}

fn load_golden() -> Vec<GoldenChunk> {
    let data = fs::read(tests_dir().join("carver_golden.bin")).expect("run tools/noise-golden/run_carver.sh");
    let mut r = Reader { data, pos: 0 };
    let count = r.i32();
    (0..count)
        .map(|_| {
            let cx = r.i32();
            let cz = r.i32();
            let palette = r.strings();
            let n = r.i32() as usize;
            let blocks = (0..n).map(|_| palette[r.u16() as usize].clone()).collect();
            let mask_min_y = r.i32();
            let mask_max_y = r.i32();
            let words_len = r.i32() as usize;
            let mask_words = (0..words_len).map(|_| r.i64() as u64).collect();
            let dirt_to_grass = r.i32();
            GoldenChunk {
                cx,
                cz,
                blocks,
                mask_words,
                mask_min_y,
                mask_max_y,
                dirt_to_grass,
            }
        })
        .collect()
}

#[test]
fn sin_table_matches_jvm() {
    let data = fs::read(tests_dir().join("carver_sin_table.bin")).expect("carver_sin_table.bin");
    let table = trig::sin_table();
    let mut bad = 0;
    for (i, want) in data.chunks_exact(4).enumerate() {
        let want = u32::from_be_bytes(want.try_into().unwrap());
        if table[i].to_bits() != want {
            bad += 1;
        }
    }
    assert_eq!(bad, 0, "Mth.SIN entries differing from the JVM table");
}

#[test]
fn bundled_carvers_match_registry() {
    let bundled = fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/worldgen/overworld_carvers.txt")).unwrap();
    let live = fs::read_to_string(tests_dir().join("carver_biomes.txt")).unwrap();
    for line in bundled.lines().filter(|l| !l.starts_with('#')) {
        assert!(live.lines().any(|l| l == line), "biome line {line} differs from the registry");
    }
}

#[test]
fn carved_chunks_match_vanilla() {
    let ow = Overworld::new(SEED);
    let golden = load_golden();
    let mut failed = false;
    for g in &golden {
        let (ours, mask) = ow.generate_carved(g.cx, g.cz);
        assert_eq!((mask.min_y(), mask.max_y()), (g.mask_min_y, g.mask_max_y), "mask range");

        let mut block_diff = 0usize;
        let mut first = None;
        for (i, want) in g.blocks.iter().enumerate() {
            let got = &ours.palette[ours.blocks[i] as usize];
            if got != want {
                block_diff += 1;
                if first.is_none() {
                    first = Some((i, want.clone(), got.clone()));
                }
            }
        }

        let ours_words = mask.words();
        let n = ours_words.len().max(g.mask_words.len());
        let mut mask_diff_bits = 0u32;
        for w in 0..n {
            let a = ours_words.get(w).copied().unwrap_or(0);
            let b = g.mask_words.get(w).copied().unwrap_or(0);
            mask_diff_bits += (a ^ b).count_ones();
        }
        let cells: usize = g.mask_words.iter().map(|w| w.count_ones() as usize).sum();

        println!(
            "chunk {},{}: block mismatches {} / {}, carving mask bit mismatches {} (golden cells {}), dirt->grass golden {}, first {:?}",
            g.cx,
            g.cz,
            block_diff,
            g.blocks.len(),
            mask_diff_bits,
            cells,
            g.dirt_to_grass,
            first
        );
        if block_diff > 0 || mask_diff_bits > 0 {
            failed = true;
        }
    }
    assert!(!failed, "carver golden mismatch, see per-chunk counts above");
}

/// Timing probe (run with `--release --ignored --nocapture`): average time per carved chunk over a grid.
#[test]
#[ignore]
fn timing_carved_per_chunk() {
    use std::time::Instant;
    let ow = Overworld::new(SEED);
    let mut coords = Vec::new();
    for cx in 0..8 {
        for cz in 0..8 {
            coords.push((cx * 3 - 10, cz * 5 - 17));
        }
    }
    for &(cx, cz) in &coords {
        std::hint::black_box(ow.generate_carved(cx, cz));
    }
    let start = Instant::now();
    for &(cx, cz) in &coords {
        std::hint::black_box(ow.generate_carved(cx, cz));
    }
    let per = start.elapsed().as_secs_f64() * 1000.0 / coords.len() as f64;
    println!("generate_carved: {per:.2} ms per chunk (single thread, {} chunks)", coords.len());
}

#[test]
#[ignore]
fn timing_uncarved_per_chunk() {
    use std::time::Instant;
    let ow = Overworld::new(SEED);
    let mut coords = Vec::new();
    for cx in 0..8 {
        for cz in 0..8 {
            coords.push((cx * 3 - 10, cz * 5 - 17));
        }
    }
    for &(cx, cz) in &coords {
        std::hint::black_box(ow.generate(cx, cz));
    }
    let start = Instant::now();
    for &(cx, cz) in &coords {
        std::hint::black_box(ow.generate(cx, cz));
    }
    let per = start.elapsed().as_secs_f64() * 1000.0 / coords.len() as f64;
    println!("generate: {per:.2} ms per chunk (single thread, {} chunks)", coords.len());
}
