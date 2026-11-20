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
fn surface_batch_is_reachable_in_production() {
    let names = [
        "patch_berry_common", "patch_berry_rare", "patch_bush", "patch_dry_grass_badlands",
        "patch_dry_grass_desert", "patch_leaf_litter", "patch_melon", "patch_melon_sparse",
        "patch_pumpkin", "patch_red_shrub", "patch_waterlily", "spore_blossom", "patch_cactus_decorated",
        "patch_cactus_desert", "patch_sugar_cane", "patch_sugar_cane_badlands", "patch_sugar_cane_desert",
        "patch_sugar_cane_swamp", "freeze_top_layer", "vines", "classic_vines_cave_feature", "forest_rock",
        "bamboo", "bamboo_light", "bamboo_vegetation", "birch_tall", "dark_forest_vegetation",
        "mushroom_island_vegetation", "kelp_cold", "kelp_warm", "disk_grass",
    ];
    for name in names {
        assert!(allow::generation(&format!("minecraft:{name}")), "{name}");
    }
    for name in bundle()["placed"].as_object().unwrap().keys() {
        if ["trees_", "brown_mushroom_", "red_mushroom_", "seagrass_", "patch_firefly_bush"]
            .iter().any(|prefix| name.trim_start_matches("minecraft:").starts_with(prefix))
        {
            assert!(allow::generation(name), "{name}");
        }
    }
    let overworld = Overworld::new(42);
    Decorator::production(&overworld, &allow::generation).unwrap();
}

#[test]
fn features_match_vanilla_write_order_and_next_random_value() {
    let blocks = &rine_data::init().blocks;
    let fixture = include_str!("../../tests/data/feature_samples.txt");
    let names: std::collections::BTreeSet<String> = fixture.lines().map(|l| {
        format!("minecraft:{}", l.split_ascii_whitespace().next().unwrap())
    }).collect();
    let growers: Vec<&str> = names.iter().map(String::as_str).collect();
    let terrain = [
        "air", "grass_block", "stone", "bedrock", "poppy", "crimson_nylium", "warped_nylium",
        "mycelium", "sand", "dirt", "water", "water[level=2]", "water[level=8]", "lava", "frosted_ice", "ice",
    ].map(|name| {
        let full = format!("minecraft:{name}");
        if name.contains('[') {
            return full;
        }
        let blk = blocks.block(&full).unwrap();
        let props = blk.props.iter().map(|p| {
            format!("{}={}", p.name, blocks.get_prop(blk.default_state as u16, &p.name).unwrap())
        }).collect::<Vec<_>>().join(",");
        if props.is_empty() { full } else { format!("{full}[{props}]") }
    });
    let dec = Decorator::new_growers(&terrain, HashMap::new(), &growers).unwrap();
    let palette: Vec<_> = dec.palette().iter().map(|name| blocks.parse_state(name).unwrap()).collect();
    let world = Overworld::new(0);
    let biome = |name| world.biome_names().iter().position(|n| n == name).unwrap() as u16;
    let plains = biome("minecraft:plains");
    let snowy_plains = biome("minecraft:snowy_plains");
    let wg = std::array::from_fn::<_, 9, _>(|_| [vec![0; 256], vec![0; 256]]);
    let survival_state = |name| dec.palette().iter().position(|s| s == name).unwrap() as u16;

    for line in fixture.lines() {
        let fields: Vec<_> = line.split_ascii_whitespace().collect();
        let seed: i32 = fields[2].parse().unwrap();
        let expected = fields[3].parse().unwrap();
        let hash_expected = u64::from_str_radix(fields[4], 16).unwrap();
        let random_expected: i32 = fields[5].parse().unwrap();
        let (x, y, z) = (seed * 13 - 80, 64, -47 + seed * 3);
        let (cx, cz) = (x.div_euclid(16), z.div_euclid(16));
        let mut region = dec.grower_region(cx, cz, vec![vec![0; 384 * 256]; 9]);
        for dz in -16..=16 {
            for dx in -16..=16 {
                for dy in -36..=36 {
                    let index = match (fields[1], dx, dy, dz) {
                        ("ceiling", _, dy, _) => if dy >= 1 { 2 } else { 0 },
                        ("pale", -1, 0..=3, 0) => 2,
                        ("moss", 1, -1, 0) | ("pool", 1, -1, _) | ("pool", _, -1, -1) => 3,
                        ("crimson", 0, 1, 0) => 4,
                        ("wall", _, 0, -1) | ("wall", -1, 0, _) | ("bamboo_obstacle", 0, 3, 0) => 2,
                        ("cactus_lava", _, 0, -1) => 13,
                        ("cane_water", _, -1, -1) => 11,
                        ("cane_frost", _, -1, -1) => 14,
                        ("cane_ice", _, -1, -1) => 15,
                        ("water", _, 0..=11, _) | ("shallow" | "flowing_top", _, 0, _)
                        | ("water_surface", _, -1, _) | ("falling", _, 1, _) => 10,
                        ("cold", dx, -1, _) if dx & 1 == 0 => 10,
                        ("flowing", _, 0..=1, _) | ("flowing_top", _, 1, _) => 11,
                        ("falling", _, 0, _) => 12,
                        (_, _, 0.., _) => 0,
                        ("grass" | "cold" | "bamboo_obstacle" | "cane_water" | "cane_frost" | "cane_ice", ..) => 1,
                        ("crimson", ..) => 5,
                        ("warped", ..) => 6,
                        ("mycelium", ..) => 7,
                        ("sand" | "cactus_lava", ..) => 8,
                        ("dirt", ..) => 9,
                        _ => 2,
                    };
                    if index != 0 {
                        region.set_block_in_section(x + dx, y + dy, z + dz, index);
                    }
                }
            }
        }
        let mut chunks = Vec::new();
        let mut region = if matches!(fields[0], "freeze_top_layer" | "bamboo_some_podzol" | "bamboo_no_podzol") {
            let biome = if fields[1] == "cold" { snowy_plains } else { plains };
            chunks.extend(region.into_blocks().into_iter().map(|blocks| ChunkBlocks {
                min_y: -64, height: 384, blocks, biomes: vec![biome; 4 * 4 * 96],
                palette: Vec::new(), biome_palette: Vec::new(),
            }));
            Region::new(dec.tags(), &[], &world, -64, 384, cx, cz, &mut chunks, &wg, Vec::new())
        } else {
            region
        };
        region.start_log();
        region.set_level_random(WorldgenRng::direct(Xoroshiro::new(seed as i64 + 100)));
        let mut rng = WorldgenRng::new(seed as i64);
        let survival = match fields[0] {
            "cactus" => Some(survival_state("minecraft:cactus[age=0]")),
            "sugar_cane" => Some(survival_state("minecraft:sugar_cane[age=0]")),
            _ => None,
        };
        let placed = survival.is_none_or(|s| vegetation::can_survive(&region, s, x, y, z))
            && dec.place_grower(&format!("minecraft:{}", fields[0]), &mut region, &mut rng, (x, y, z)).unwrap();
        let hash = region.take_log().into_iter().fold(0xcbf29ce484222325u64, |mut h, (x, y, z, state, _)| {
            for v in [x, y, z, palette[state as usize] as i32] {
                h = (h ^ v as i64 as u64).wrapping_mul(0x100000001b3);
            }
            h
        });
        assert_eq!((placed, hash, rng.next_int()), (expected, hash_expected, random_expected), "{line}");
    }
}
