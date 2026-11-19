use super::*;
use crate::random::Xoroshiro;

#[test]
fn normal_generation_compiles_the_new_vanilla_placed_features() {
    for name in [
        "flower_plains", "flower_flower_forest", "flower_swamp", "flower_meadow", "flower_cherry",
        "wildflowers_birch_forest", "flower_pale_garden", "pale_moss_patch", "pale_garden_vegetation",
        "lush_caves_clay", "lush_caves_vegetation", "lush_caves_ceiling_vegetation",
    ] {
        assert!(allow::live_generation(&format!("minecraft:{name}")), "{name}");
    }
    for name in ["crimson_fungi", "warped_fungi", "crimson_forest_vegetation", "nether_sprouts", "twisting_vines", "weeping_vines"] {
        assert!(allow::generation(&format!("minecraft:{name}")), "{name}");
    }
    let o = Overworld::new(42);
    Decorator::production(&o, &allow::generation).unwrap();
    Decorator::production(&o, &allow::live_generation).unwrap();
}

#[test]
fn features_match_vanilla_write_order_and_next_random_value() {
    let blocks = &rine_data::init().blocks;
    let fixture = include_str!("../../tests/data/feature_samples.txt");
    let names: std::collections::BTreeSet<String> = fixture.lines().map(|l| {
        format!("minecraft:{}", l.split_ascii_whitespace().next().unwrap())
    }).collect();
    let growers: Vec<&str> = names.iter().map(String::as_str).collect();
    let terrain = ["air", "grass_block", "stone", "bedrock", "poppy", "crimson_nylium", "warped_nylium"].map(|name| {
        let full = format!("minecraft:{name}");
        let blk = blocks.block(&full).unwrap();
        let props = blk.props.iter().map(|p| {
            format!("{}={}", p.name, blocks.get_prop(blk.default_state as u16, &p.name).unwrap())
        }).collect::<Vec<_>>().join(",");
        if props.is_empty() { full } else { format!("{full}[{props}]") }
    });
    let dec = Decorator::new_growers(&terrain, HashMap::new(), &growers).unwrap();
    let palette: Vec<_> = dec.palette().iter().map(|name| blocks.parse_state(name).unwrap()).collect();

    for line in fixture.lines() {
        let fields: Vec<_> = line.split_ascii_whitespace().collect();
        let seed: i32 = fields[2].parse().unwrap();
        let expected = fields[3].parse().unwrap();
        let hash_expected = u64::from_str_radix(fields[4], 16).unwrap();
        let random_expected: i32 = fields[5].parse().unwrap();
        let (x, y, z) = (seed * 13 - 80, 64, -47 + seed * 3);
        let (cx, cz) = (x.div_euclid(16), z.div_euclid(16));
        let mut region = dec.grower_region(cx, cz, vec![vec![0; 384 * 256]; 9]);
        for dz in -12..=12 {
            for dx in -12..=12 {
                for dy in -36..=36 {
                    let index = match (fields[1], dx, dy, dz) {
                        ("ceiling", _, dy, _) => if dy >= 1 { 2 } else { 0 },
                        ("pale", -1, 0..=3, 0) => 2,
                        ("moss", 1, -1, 0) | ("pool", 1, -1, _) | ("pool", _, -1, -1) => 3,
                        ("crimson", 0, 1, 0) => 4,
                        (_, _, 0.., _) => 0,
                        ("grass", ..) => 1,
                        ("crimson", ..) => 5,
                        ("warped", ..) => 6,
                        _ => 2,
                    };
                    if index != 0 {
                        region.set_block_in_section(x + dx, y + dy, z + dz, index);
                    }
                }
            }
        }
        region.start_log();
        region.set_level_random(WorldgenRng::direct(Xoroshiro::new(seed as i64 + 100)));
        let mut rng = WorldgenRng::new(seed as i64);
        let placed = dec.place_grower(&format!("minecraft:{}", fields[0]), &mut region, &mut rng, (x, y, z)).unwrap();
        let hash = region.take_log().into_iter().fold(0xcbf29ce484222325u64, |mut h, (x, y, z, state, _)| {
            for v in [x, y, z, palette[state as usize] as i32] {
                h = (h ^ v as i64 as u64).wrapping_mul(0x100000001b3);
            }
            h
        });
        assert_eq!((placed, hash, rng.next_int()), (expected, hash_expected, random_expected), "{line}");
    }
}
