//! Times `Pipeline::full_chunk` (release build): sequential and parallel over the same chunks, with the
//! protos already cached (the final cache is not hit, so every call decorates).
//!
//!   cargo run -p rine-worldgen --release --example pipeline_bench -- [side] [threads]
use std::time::Instant;

use rine_worldgen::pipeline::Pipeline;

const SEED: i64 = 1592598566;

fn main() {
    let mut args = std::env::args().skip(1);
    let side: i32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(8);
    let threads: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(8);

    let pipe = Pipeline::new(SEED).expect("pipeline");
    // Protos for the area of the benchmark plus the decoration ring.
    let t = Instant::now();
    for cz in -3..side + 3 {
        for cx in -3..side + 3 {
            let _ = pipe.proto(cx, cz);
        }
    }
    println!("protos: {} chunks in {:.1} ms", (side + 6) * (side + 6), t.elapsed().as_secs_f64() * 1e3);

    let chunks: Vec<(i32, i32)> = (0..side).flat_map(|z| (0..side).map(move |x| (x, z))).collect();

    let t = Instant::now();
    for &(cx, cz) in &chunks {
        let _ = pipe.full_chunk(cx, cz);
    }
    let seq = t.elapsed().as_secs_f64() * 1e3;
    println!("sequential: {} chunks, {:.2} ms/chunk", chunks.len(), seq / chunks.len() as f64);

    // Fresh chunks for the parallel run (the finals of the first run are cached now).
    let offset = side + 8;
    let chunks: Vec<(i32, i32)> = chunks.iter().map(|&(x, z)| (x + offset, z)).collect();
    for cz in -3..side + 3 {
        for cx in offset - 3..offset + side + 3 {
            let _ = pipe.proto(cx, cz);
        }
    }
    let t = Instant::now();
    let per = chunks.len().div_ceil(threads);
    std::thread::scope(|s| {
        for part in chunks.chunks(per) {
            let pipe = &pipe;
            s.spawn(move || {
                for &(cx, cz) in part {
                    let _ = pipe.full_chunk(cx, cz);
                }
            });
        }
    });
    let par = t.elapsed().as_secs_f64() * 1e3;
    println!(
        "parallel ({threads} threads): {} chunks, {:.2} ms/chunk wall, {:.2} ms/chunk per thread",
        chunks.len(),
        par / chunks.len() as f64,
        par * threads as f64 / chunks.len() as f64
    );
}
