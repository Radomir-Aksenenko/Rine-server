//! Phase 3 golden test: biomes, noise fill and surface rules of the overworld against the vanilla
//! 26.3 jar (`tools/noise-golden/WorldgenGolden.java`, data in `tests/worldgen_golden.bin`).

use std::fs;
use std::path::PathBuf;

use rine_worldgen::Overworld;

const SEED: i64 = 1592598566;

struct Reader {
    data: Vec<u8>,
    pos: usize,
}

impl Reader {
    fn i32(&mut self) -> i32 {
        let v = i32::from_be_bytes(self.data[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        v
    }
    fn u16(&mut self) -> u16 {
        let v = u16::from_be_bytes(self.data[self.pos..self.pos + 2].try_into().unwrap());
        self.pos += 2;
        v
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
    fn u16s(&mut self) -> Vec<u16> {
        let n = self.i32() as usize;
        (0..n).map(|_| self.u16()).collect()
    }
}

struct GoldenChunk {
    cx: i32,
    cz: i32,
    blocks: Vec<String>,
    biomes: Vec<String>,
}

fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

fn load_golden() -> Vec<GoldenChunk> {
    let data = fs::read(tests_dir().join("worldgen_golden.bin")).expect("run tools/noise-golden/run_worldgen.sh");
    let mut r = Reader { data, pos: 0 };
    let count = r.i32();
    (0..count)
        .map(|_| {
            let cx = r.i32();
            let cz = r.i32();
            let bp = r.strings();
            let pp = r.strings();
            let blocks = r.u16s().into_iter().map(|i| bp[i as usize].clone()).collect();
            let biomes = r.u16s().into_iter().map(|i| pp[i as usize].clone()).collect();
            GoldenChunk { cx, cz, blocks, biomes }
        })
        .collect()
}

#[test]
fn parameter_list_matches_java_quantization() {
    let ow = Overworld::new(SEED);
    let text = fs::read_to_string(tests_dir().join("worldgen_params.txt")).unwrap();
    let names = ow.biome_names();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), ow.parameter_points().len());
    for (line, (space, idx)) in lines.iter().zip(ow.parameter_points()) {
        let f: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(f[0], names[idx as usize], "biome order");
        for d in 0..6 {
            let lo: i64 = f[1 + d * 2].parse().unwrap();
            let hi: i64 = f[2 + d * 2].parse().unwrap();
            assert_eq!((lo, hi), space[d], "parameter {d} of {line}");
        }
        let offset: i64 = f[13].parse().unwrap();
        assert_eq!((offset, offset), space[6], "offset of {line}");
    }
}

#[test]
fn chunks_match_vanilla() {
    let ow = Overworld::new(SEED);
    let golden = load_golden();
    let mut failed = false;
    for g in &golden {
        let ours = ow.generate(g.cx, g.cz);
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
        let mut biome_diff = 0usize;
        for (i, want) in g.biomes.iter().enumerate() {
            let got = &ours.biome_palette[ours.biomes[i] as usize];
            if got != want {
                biome_diff += 1;
            }
        }
        println!(
            "chunk {},{}: block mismatches {} / {}, quart biome mismatches {} / {}, first {:?}",
            g.cx,
            g.cz,
            block_diff,
            g.blocks.len(),
            biome_diff,
            g.biomes.len(),
            first
        );
        if block_diff > 0 || biome_diff > 0 {
            failed = true;
        }
    }
    assert!(!failed, "golden mismatch, see per-chunk counts above");
}

/// Timing probe (run with `--release --ignored --nocapture`): average time per chunk over a grid.
#[test]
#[ignore]
fn timing_per_chunk() {
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

#[test]
#[ignore]
fn timing_density_only() {
    use std::time::Instant;
    let t = rine_worldgen::Terrain::new(SEED);
    std::hint::black_box(t.chunk_density(0, 0));
    let start = Instant::now();
    for cx in 0..16 {
        std::hint::black_box(t.chunk_density(cx, 3));
    }
    println!("final density only: {:.2} ms per chunk", start.elapsed().as_secs_f64() * 1000.0 / 16.0);
}
