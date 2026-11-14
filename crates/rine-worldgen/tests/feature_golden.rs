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

/// Tree golden. `tools/noise-golden/make_tree_golden.js` writes the datapack (synthetic tree placed features, one
/// per supported tree and fallen tree configured feature) and the bundle `RINE_TREE_BUNDLE`. Full block states
/// are compared, so leaf distance, log axis, vine faces, cocoa and bee nests all count.
///
/// `RINE_TREE_ORDER=reverse` decorates the 5x5 in reverse raster order: a mismatch that changes between the two
/// orders depends on neighbouring decorations, a mismatch that does not is a bug in the tree code.
#[test]
fn tree_decoration_matches_vanilla_server() {
    let (Ok(golden_path), Ok(bundle_path)) = (std::env::var("RINE_FEATURE_GOLDEN"), std::env::var("RINE_TREE_BUNDLE")) else {
        eprintln!("RINE_FEATURE_GOLDEN or RINE_TREE_BUNDLE not set: skipping tree golden");
        return;
    };
    let golden = read_golden(&std::fs::read(golden_path).expect("golden file"));
    let bundle: serde_json::Value = serde_json::from_slice(&std::fs::read(bundle_path).expect("tree bundle")).expect("tree bundle json");
    let reverse = std::env::var("RINE_TREE_ORDER").is_ok_and(|v| v == "reverse");

    let o = Overworld::new(SEED);
    let dec = Decorator::new_from(&bundle, &o, &|n: &str| n.starts_with("rine:tree_")).expect("decorator");
    let mut cache = HashMap::new();

    let mut pairs: HashMap<(String, String), usize> = HashMap::new();
    let mut total_blocks = 0usize;
    let mut total_mismatch = 0usize;
    // Only chunks whose 5x5 decoration neighbourhood is inside the golden area are comparable.
    let xs: Vec<i32> = golden.chunks.keys().map(|k| k.0).collect();
    let zs: Vec<i32> = golden.chunks.keys().map(|k| k.1).collect();
    let (min_x, max_x, min_z, max_z) = (
        *xs.iter().min().unwrap(),
        *xs.iter().max().unwrap(),
        *zs.iter().min().unwrap(),
        *zs.iter().max().unwrap(),
    );
    let mut keys: Vec<(i32, i32)> = golden
        .chunks
        .keys()
        .copied()
        .filter(|&(cx, cz)| cx - min_x >= 2 && max_x - cx >= 2 && cz - min_z >= 2 && max_z - cz >= 2)
        .collect();
    keys.sort();
    for (cx, cz) in keys {
        let (gpal, gblocks) = &golden.chunks[&(cx, cz)];
        let gpal: Vec<String> = gpal.iter().map(|s| expand_default(&bundle, s)).collect();
        let mine = final_chunk_order(&o, &dec, &mut cache, cx, cz, reverse);
        let mut mismatch = 0usize;
        for (i, &g) in gblocks.iter().enumerate() {
            let m = mine.blocks[i] as usize;
            let (a, b) = (gpal[g as usize].as_str(), mine.palette[m].as_str());
            if a != b {
                mismatch += 1;
                *pairs.entry((a.to_string(), b.to_string())).or_default() += 1;
            }
        }
        total_blocks += gblocks.len();
        total_mismatch += mismatch;
        eprintln!("chunk ({cx},{cz}): {mismatch} mismatched blocks");
    }
    eprintln!("total: {total_mismatch} / {total_blocks}");
    eprintln!("treeified bins approximated: {}", rine_worldgen::feature::tree::order::TREEIFIED_BINS.load(std::sync::atomic::Ordering::Relaxed));
    let mut top: Vec<_> = pairs.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1));
    for ((g, m), n) in top.iter().take(30) {
        eprintln!("  golden {g} | mine {m}: {n}");
    }
    assert_eq!(total_mismatch, 0, "tree decoration differs from vanilla");
}

/// The golden writes a state whose properties are all default as its bare name (`minecraft:oak_leaves` is
/// `oak_leaves[distance=7,persistent=false,waterlogged=false]`). Expands it to the canonical form.
fn expand_default(bundle: &serde_json::Value, state: &str) -> String {
    if state.contains('[') {
        return state.to_string();
    }
    let Some(entry) = bundle["blockProps"].get(state) else {
        return state.to_string();
    };
    let mut keys: Vec<&str> = entry["properties"].as_object().map(|o| o.keys().map(String::as_str).collect()).unwrap_or_default();
    if keys.is_empty() {
        return state.to_string();
    }
    keys.sort();
    let body: Vec<String> = keys.iter().map(|k| format!("{k}={}", entry["default"][k].as_str().expect("default value"))).collect();
    format!("{state}[{}]", body.join(","))
}

/// `final_chunk` with a selectable decoration order (reverse raster for the neighbour-order check).
fn final_chunk_order(o: &Overworld, dec: &Decorator, cache: &mut HashMap<(i32, i32), Arc<ChunkBlocks>>, cx: i32, cz: i32, reverse: bool) -> ChunkBlocks {
    let mut area: Vec<ChunkBlocks> = Vec::with_capacity(49);
    for dz in -3..=3 {
        for dx in -3..=3 {
            let key = (cx + dx, cz + dz);
            let proto = cache.entry(key).or_insert_with(|| Arc::new(o.generate(key.0, key.1))).as_ref().clone();
            area.push(proto);
        }
    }
    let mut order: Vec<(i32, i32)> = Vec::with_capacity(25);
    for dz in -2..=2i32 {
        for dx in -2..=2i32 {
            order.push((dx, dz));
        }
    }
    if reverse {
        order.reverse();
    }
    for (dx, dz) in order {
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
    area[3 * 7 + 3].clone()
}
