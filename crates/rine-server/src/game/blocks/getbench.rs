//! Micro-benchmark: block reads through the chunk map vs a last-chunk cache.
//! cargo test --release -p rine-server getbench -- --ignored --nocapture
#[test]
#[ignore]
fn getbench() {
    use ahash::AHashMap;
    use rine_world::{Chunk, ChunkPos};
    use std::time::Instant;
    rine_data::init();
    for n in [1_000i32, 40_000] {
        let side = (n as f64).sqrt() as i32;
        let mut m: AHashMap<ChunkPos, Box<Chunk>> = AHashMap::with_capacity(16384);
        for x in 0..side {
            for z in 0..side {
                let mut c = Box::new(Chunk::new(ChunkPos::new(x, z)));
                c.set(3, 64, 3, 1);
                m.insert(ChunkPos::new(x, z), c);
            }
        }
        // fire-like pattern: 370 reads around each of many fire blocks spread over the map
        let mut s: u64 = 7;
        let mut fires = Vec::new();
        for _ in 0..20_000 {
            s ^= s << 13; s ^= s >> 7; s ^= s << 17;
            let x = (s % (side as u64 * 16 - 16)) as i32 + 8;
            let z = ((s >> 20) % (side as u64 * 16 - 16)) as i32 + 8;
            fires.push((x, 64, z));
        }
        let t = Instant::now();
        let mut acc = 0u64;
        for &(fx, fy, fz) in &fires {
            for dx in -1..=1 { for dz in -1..=1 { for dy in -1..=4 {
                for k in 0..7 {
                    let (x, y, z) = (fx + dx + k % 2, fy + dy, fz + dz);
                    if let Some(c) = m.get(&ChunkPos::new(x >> 4, z >> 4)) { acc += c.get(x & 15, y, z & 15) as u64; }
                }
            }}}
        }
        let map = t.elapsed();
        let t = Instant::now();
        let mut last: (ChunkPos, *const Chunk) = (ChunkPos::new(i32::MIN, i32::MIN), std::ptr::null());
        for &(fx, fy, fz) in &fires {
            for dx in -1..=1 { for dz in -1..=1 { for dy in -1..=4 {
                for k in 0..7 {
                    let (x, y, z) = (fx + dx + k % 2, fy + dy, fz + dz);
                    let cp = ChunkPos::new(x >> 4, z >> 4);
                    if cp != last.0 {
                        last = (cp, m.get(&cp).map(|b| &**b as *const Chunk).unwrap_or(std::ptr::null()));
                    }
                    if !last.1.is_null() { acc += unsafe { &*last.1 }.get(x & 15, y, z & 15) as u64; }
                }
            }}}
        }
        let cached = t.elapsed();
        let reads = fires.len() as f64 * 378.0;
        println!("chunks={} map {:.2} ns/read, cached {:.2} ns/read (acc {acc})", m.len(), map.as_nanos() as f64 / reads, cached.as_nanos() as f64 / reads);
    }
}
