//! Golden test for the feature decoration port against a real vanilla 26.3 server.
//!
//! The golden is produced by `tools/noise-golden/run_feature_golden.js` (vanilla server + feature datapack
//! that keeps only the allowed placed features and no carvers) and `dump_mca_chunks.js`. Point
//! `RINE_FEATURE_GOLDEN` at the produced file. Without it the test is skipped.
//!
//! Only the inner chunks are compared: a chunk's final state depends on the decorations of its 5x5
//! neighbourhood, which the golden area provides only for interior chunks.

use std::collections::HashMap;
use std::sync::Arc;

use rine_worldgen::feature::Decorator;
use rine_worldgen::overworld::{ChunkBlocks, Overworld};

const SEED: i64 = 1592598566;

/// Ore placed features are the only allowed ones (same rule as `make_feature_datapack.js`).
fn ore_only(name: &str) -> bool {
    let bundle: serde_json::Value = serde_json::from_str(include_str!("../assets/worldgen/features.json")).unwrap();
    let placed = &bundle["placed"][name];
    match placed["feature"].as_str() {
        Some(f) => bundle["configured"][f]["type"] == "minecraft:ore",
        None => false,
    }
}

/// Block name only. The golden palette stores default-state blocks as bare names, so property
/// comparison needs the block report; name equality is the check here.
fn canonical(state: &str) -> String {
    state.split('[').next().unwrap_or(state).to_string()
}

struct Golden {
    chunks: HashMap<(i32, i32), (Vec<String>, Vec<u16>)>,
}

fn read_golden(bytes: &[u8]) -> Golden {
    let mut pos = 0usize;
    let u32_at = |p: &mut usize| {
        let v = u32::from_le_bytes(bytes[*p..*p + 4].try_into().unwrap());
        *p += 4;
        v
    };
    let count = u32_at(&mut pos);
    let mut chunks = HashMap::new();
    for _ in 0..count {
        let cx = u32_at(&mut pos) as i32;
        let cz = u32_at(&mut pos) as i32;
        let npal = u32_at(&mut pos) as usize;
        let mut palette = Vec::with_capacity(npal);
        for _ in 0..npal {
            let len = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]) as usize;
            pos += 2;
            palette.push(String::from_utf8(bytes[pos..pos + len].to_vec()).unwrap());
            pos += len;
        }
        let n = 384 * 256;
        let blocks: Vec<u16> = bytes[pos..pos + n * 2]
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        pos += n * 2;
        chunks.insert((cx, cz), (palette, blocks));
    }
    Golden { chunks }
}

/// Protos for the 7x7 area around `(cx, cz)`, decorated on the inner 5x5 in raster order, then the
/// centre chunk. Each decoration sees the 3x3 window around its chunk, as in vanilla.
fn final_chunk(o: &Overworld, dec: &Decorator, cache: &mut HashMap<(i32, i32), Arc<ChunkBlocks>>, cx: i32, cz: i32) -> ChunkBlocks {
    let mut area: Vec<ChunkBlocks> = Vec::with_capacity(49);
    for dz in -3..=3 {
        for dx in -3..=3 {
            let key = (cx + dx, cz + dz);
            let proto = cache
                .entry(key)
                .or_insert_with(|| Arc::new(o.generate(key.0, key.1)))
                .as_ref()
                .clone();
            area.push(proto);
        }
    }
    // area index: (dz + 3) * 7 + (dx + 3)
    for dz in -2..=2i32 {
        for dx in -2..=2i32 {
            let mut window: Vec<ChunkBlocks> = Vec::with_capacity(9);
            for wz in -1..=1i32 {
                for wx in -1..=1i32 {
                    let ix = ((dz + wz + 3) * 7 + (dx + wx + 3)) as usize;
                    window.push(area[ix].clone());
                }
            }
            dec.decorate(o, SEED, cx + dx, cz + dz, &mut window).unwrap();
            for wz in -1..=1i32 {
                for wx in -1..=1i32 {
                    let ix = ((dz + wz + 3) * 7 + (dx + wx + 3)) as usize;
                    area[ix] = window[((wz + 1) * 3 + (wx + 1)) as usize].clone();
                }
            }
        }
    }
    area[3 * 7 + 3].clone()
}

#[test]
fn ore_decoration_matches_vanilla_server() {
    let Ok(path) = std::env::var("RINE_FEATURE_GOLDEN") else {
        eprintln!("RINE_FEATURE_GOLDEN not set: skipping feature golden");
        return;
    };
    let golden = read_golden(&std::fs::read(path).expect("golden file"));

    let o = Overworld::new(SEED);
    let dec = Decorator::new(&o, &ore_only).expect("decorator");
    let mut cache = HashMap::new();

    let mut pairs: HashMap<(String, String), usize> = HashMap::new();
    let mut total_blocks = 0usize;
    let mut total_mismatch = 0usize;
    let mut keys: Vec<(i32, i32)> = golden.chunks.keys().copied().collect();
    keys.sort();
    for (cx, cz) in keys {
        let (gpal, gblocks) = &golden.chunks[&(cx, cz)];
        let mine = final_chunk(&o, &dec, &mut cache, cx, cz);
        let mut mismatch = 0usize;
        for (i, &g) in gblocks.iter().enumerate() {
            let m = mine.blocks[i] as usize;
            let (a, b) = (canonical(&gpal[g as usize]), canonical(&mine.palette[m]));
            if a != b {
                mismatch += 1;
                *pairs.entry((a, b)).or_default() += 1;
            }
        }
        total_blocks += gblocks.len();
        total_mismatch += mismatch;
        eprintln!("chunk ({cx},{cz}): {mismatch} mismatched blocks");
    }
    eprintln!("total: {total_mismatch} / {total_blocks}");
    let mut top: Vec<_> = pairs.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1));
    for ((g, m), n) in top.iter().take(15) {
        eprintln!("  golden {g} | mine {m}: {n}");
    }
    assert_eq!(total_mismatch, 0, "ore decoration differs from vanilla");
}
