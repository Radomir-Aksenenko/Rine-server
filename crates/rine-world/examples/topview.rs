// Scratch: top-down PNG of generated chunks. cargo run --release -p rine-world --example topview -- seed cx cz n out.png [legacy]
use rine_world::gen::Generator;
use rine_world::{ChunkPos, MAX_Y, MIN_Y};
use std::collections::BTreeMap;

fn color(name: &str) -> [u8; 3] {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    if n.ends_with("_leaves") {
        return match n {
            "oak_leaves" => [40, 140, 30], "birch_leaves" => [130, 190, 70], "spruce_leaves" => [30, 80, 50],
            "jungle_leaves" => [20, 170, 20], "acacia_leaves" => [150, 160, 30], "dark_oak_leaves" => [20, 90, 15],
            "mangrove_leaves" => [60, 120, 40], "cherry_leaves" => [240, 160, 200], "azalea_leaves" | "flowering_azalea_leaves" => [90, 150, 60],
            "pale_oak_leaves" => [190, 200, 190], _ => [0, 255, 0],
        };
    }
    if n.ends_with("_log") || n.ends_with("_wood") || n.ends_with("_stem") { return [90, 60, 30]; }
    match n {
        "grass_block" => [100, 170, 80], "water" => [50, 80, 220], "sand" | "sandstone" => [220, 210, 150],
        "red_sand" => [190, 100, 40], "snow" | "snow_block" | "powder_snow" => [250, 250, 250], "ice" | "packed_ice" | "blue_ice" => [160, 190, 255],
        "stone" | "andesite" | "cobblestone" | "mossy_cobblestone" => [125, 125, 125], "gravel" => [140, 130, 130], "dirt" | "coarse_dirt" | "rooted_dirt" => [130, 90, 60],
        "podzol" => [110, 75, 35], "mycelium" => [120, 100, 120], "terracotta" => [160, 90, 60], "mud" => [70, 60, 60],
        "short_grass" | "tall_grass" | "fern" | "large_fern" => [80, 160, 60], "cactus" => [20, 110, 20], "sugar_cane" => [150, 220, 110],
        "pumpkin" => [230, 130, 20], "melon" => [140, 180, 30], "lily_pad" => [30, 120, 30], "moss_block" | "moss_carpet" => [80, 120, 40],
        "bamboo" => [100, 200, 50], "kelp" | "kelp_plant" | "seagrass" | "tall_seagrass" => [40, 110, 120],
        _ if n.contains("coral") => [250, 80, 140],
        _ if n.contains("flower") || n.contains("tulip") || n == "poppy" || n == "dandelion" || n.contains("orchid") || n == "allium" || n == "cornflower" || n.contains("daisy") => [230, 60, 60],
        _ if n.contains("terracotta") => [170, 110, 80],
        _ => [255, 0, 255],
    }
}

fn crc(data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in data { c ^= b as u32; for _ in 0..8 { c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 }; } }
    !c
}

fn chunk(out: &mut Vec<u8>, ty: &[u8], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut t = ty.to_vec(); t.extend_from_slice(data);
    out.extend_from_slice(&t);
    out.extend_from_slice(&crc(&t).to_be_bytes());
}

fn png(w: usize, h: usize, rgb: &[u8]) -> Vec<u8> {
    let mut raw = Vec::new();
    for y in 0..h { raw.push(0); raw.extend_from_slice(&rgb[y * w * 3..(y + 1) * w * 3]); }
    let mut z = vec![0x78, 0x01];
    for (i, part) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(last as u8);
        z.extend_from_slice(&(part.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(part.len() as u16)).to_le_bytes());
        z.extend_from_slice(part);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw { a = (a + x as u32) % 65521; b = (b + a) % 65521; }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes()); ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr); chunk(&mut out, b"IDAT", &z); chunk(&mut out, b"IEND", &[]);
    out
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (seed, cx0, cz0, n): (i64, i32, i32, i32) = (a[1].parse().unwrap(), a[2].parse().unwrap(), a[3].parse().unwrap(), a[4].parse().unwrap());
    let legacy = a.get(6).is_some_and(|s| s == "legacy");
    let data = rine_data::init();
    let g = Generator::with_options(seed, !legacy);
    let w = (n * 16) as usize;
    let mut rgb = vec![0u8; w * w * 3];
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for cz in 0..n {
        for cx in 0..n {
            let c = g.generate(ChunkPos::new(cx0 + cx, cz0 + cz));
            for lz in 0..16 {
                for lx in 0..16 {
                    let mut y = MAX_Y - 1;
                    while y > MIN_Y && data.blocks.is_air(c.get(lx, y, lz)) { y -= 1; }
                    let name = &data.blocks.block_of(c.get(lx, y, lz)).name;
                    *counts.entry(name.clone()).or_default() += 1;
                    let mut col = color(name);
                    let shade = ((y - 40).clamp(0, 120) as f32 / 120.0) * 0.4 + 0.8;
                    for v in &mut col { *v = (*v as f32 * shade).min(255.0) as u8; }
                    let (px, pz) = ((cx * 16 + lx) as usize, (cz * 16 + lz) as usize);
                    rgb[(pz * w + px) * 3..(pz * w + px) * 3 + 3].copy_from_slice(&col);
                }
            }
        }
    }
    std::fs::write(&a[5], png(w, w, &rgb)).unwrap();
    let mut v: Vec<_> = counts.into_iter().collect();
    v.sort_by(|x, y| y.1.cmp(&x.1));
    for (k, c) in v.iter().take(40) { println!("{c:7} {k}"); }
}
