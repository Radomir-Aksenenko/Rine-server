//! Top-down render of generated terrain: `cargo run --release -p rine-world --example map -- <seed> <radius_chunks> <out.bmp>`
use rayon::prelude::*;
use rine_world::gen::Generator;
use rine_world::{ChunkPos, MAX_Y, MIN_Y};

fn color(name: &str, y: i32) -> [u8; 3] {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    let base: [u8; 3] = match n {
        "water" => [40, 80, 200],
        "grass_block" => [80, 160, 60],
        "sand" | "sandstone" => [220, 210, 150],
        "red_sand" => [200, 110, 50],
        "snow" | "snow_block" | "powder_snow" => [245, 245, 250],
        "ice" | "packed_ice" | "blue_ice" => [160, 200, 250],
        "stone" | "andesite" | "cobblestone" | "tuff" => [125, 125, 125],
        "gravel" => [140, 130, 125],
        "dirt" | "coarse_dirt" | "podzol" | "rooted_dirt" => [120, 85, 55],
        "mud" | "mycelium" => [100, 90, 100],
        "lava" => [230, 100, 20],
        n if n.ends_with("leaves") => [40, 110, 40],
        n if n.ends_with("_log") => [100, 70, 40],
        n if n.contains("terracotta") => [180, 100, 70],
        "short_grass" | "tall_grass" | "fern" => [90, 170, 70],
        _ => [170, 120, 170],
    };
    let k = (0.6 + (y as f32 - 40.0) / 250.0).clamp(0.4, 1.3);
    base.map(|c| (c as f32 * k).min(255.0) as u8)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let seed: i64 = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1592598566);
    let r: i32 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(16);
    let out = a.get(3).cloned().unwrap_or("map.bmp".into());
    let data = rine_data::init();
    let g = Generator::new(seed);
    let side = (2 * r) as usize * 16;
    let t = std::time::Instant::now();
    let chunks: Vec<(i32, i32)> = (-r..r).flat_map(|z| (-r..r).map(move |x| (x, z))).collect();
    let cols: Vec<((i32, i32), Vec<[u8; 3]>)> = chunks
        .par_iter()
        .map(|&(cx, cz)| {
            let c = g.generate(ChunkPos::new(cx, cz));
            let mut px = vec![[0u8; 3]; 256];
            for lz in 0..16 {
                for lx in 0..16 {
                    let mut y = MAX_Y;
                    while y > MIN_Y && data.blocks.is_air(c.get(lx, y, lz)) {
                        y -= 1;
                    }
                    px[(lz * 16 + lx) as usize] = color(&data.blocks.block_of(c.get(lx, y, lz)).name, y);
                }
            }
            ((cx, cz), px)
        })
        .collect();
    eprintln!("{} chunks in {:?}", chunks.len(), t.elapsed());
    let mut img = vec![[0u8; 3]; side * side];
    for ((cx, cz), px) in cols {
        for lz in 0..16 {
            for lx in 0..16 {
                let x = ((cx + r) * 16 + lx) as usize;
                let z = ((cz + r) * 16 + lz) as usize;
                img[z * side + x] = px[(lz * 16 + lx) as usize];
            }
        }
    }
    let row = (side * 3 + 3) & !3;
    let mut f = Vec::with_capacity(54 + row * side);
    let size = (54 + row * side) as u32;
    f.extend_from_slice(b"BM");
    f.extend_from_slice(&size.to_le_bytes());
    f.extend_from_slice(&[0; 4]);
    f.extend_from_slice(&54u32.to_le_bytes());
    f.extend_from_slice(&40u32.to_le_bytes());
    f.extend_from_slice(&(side as i32).to_le_bytes());
    f.extend_from_slice(&(-(side as i32)).to_le_bytes());
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&24u16.to_le_bytes());
    f.extend_from_slice(&[0; 24]);
    for z in 0..side {
        for x in 0..side {
            let p = img[z * side + x];
            f.extend_from_slice(&[p[2], p[1], p[0]]);
        }
        f.resize(f.len() + row - side * 3, 0);
    }
    std::fs::write(out, f).unwrap();
}
