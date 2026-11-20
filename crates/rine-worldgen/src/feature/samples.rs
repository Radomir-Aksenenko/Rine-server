use super::*;
use crate::random::Xoroshiro;
use super::state::Fluid;

const HASH_START: u64 = 0xcbf29ce484222325;

fn mix(h: u64, value: i32) -> u64 {
    (h ^ value as i64 as u64).wrapping_mul(0x100000001b3)
}

fn mix_pos(h: u64, (x, y, z): region::Pos) -> u64 {
    mix(mix(mix(h, x), y), z)
}

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
    for name in ["lake_lava_surface", "lake_lava_underground", "sulfur_pool", "ice_spike", "iceberg_blue",
        "iceberg_packed", "underwater_magma", "warm_ocean_vegetation", "glow_lichen", "sculk_vein", "amethyst_geode"] {
        assert!(allow::generation(&format!("minecraft:{name}")), "{name}");
    }
    let o = Overworld::new(42);
    Decorator::production(&o, &allow::generation).unwrap();
    Decorator::production(&o, &allow::live_generation).unwrap();
}

#[test]
fn lakes_freeze_only_still_water_in_cold_biomes() {
    let o = Overworld::new(42);
    let p = (8, 64, 8);
    for (name, cold) in [("minecraft:snowy_plains", true), ("minecraft:plains", false)] {
        let biome = o.biome_names().iter().position(|n| n == name).unwrap() as u16;
        let mut pal = Palette::new(&["minecraft:air".to_owned()], HashMap::new());
        let water = pal.block("minecraft:water");
        let still = pal.default_state("minecraft:water");
        let flowing = pal.state_from_parts("minecraft:water", &[("level", "1")]);
        let tags = Tags::new(&pal, &HashMap::new());
        let mut chunks: [ChunkBlocks; 9] = std::array::from_fn(|_| ChunkBlocks {
            min_y: o.min_y(), height: o.height(), palette: Vec::new(),
            blocks: vec![0; o.height() as usize * 256], biome_palette: Vec::new(),
            biomes: vec![biome; o.height() as usize * 4],
        });
        let wg: [WgHeights; 9] = std::array::from_fn(|_| [vec![o.min_y(); 256], vec![o.min_y(); 256]]);
        let mut region = Region::new(&tags, &[], &o, o.min_y(), o.height(), 0, 0, &mut chunks, &wg, Vec::new());
        region.write(p, still, 2);
        assert_eq!(region.should_freeze_water(p, water), cold, "{name}");
        region.write(p, flowing, 2);
        assert!(!region.should_freeze_water(p, water), "{name}");
    }
}

#[test]
fn features_match_vanilla_write_order_and_next_random_value() {
    let blocks = &rine_data::init().blocks;
    let fixture = include_str!("../../tests/data/feature_samples.txt");
    let names: std::collections::BTreeSet<String> = fixture.lines().map(|l| {
        format!("minecraft:{}", l.split_ascii_whitespace().next().unwrap())
    }).collect();
    let growers: Vec<&str> = names.iter().map(String::as_str).collect();
    let terrain = ["air", "grass_block", "stone", "bedrock", "poppy", "crimson_nylium", "warped_nylium",
        "water", "snow_block", "dirt"].map(|name| {
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
        let scenario = fields[1];
        let (x, y, z) = (seed * 13 - 80, if scenario == "low" { -60 } else { 64 }, -47 + seed * 3);
        let (cx, cz) = (x.div_euclid(16), z.div_euclid(16));
        let mut region = dec.grower_region(cx, cz, vec![vec![0; 384 * 256]; 9]);
        let radius = if matches!(scenario, "ocean" | "magma" | "geode" | "reef" | "lichen_water") { 16 } else { 12 };
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                for dy in -36..=36 {
                    let index = match (fields[1], dx, dy, dz) {
                        ("ceiling", _, dy, _) => if dy >= 1 { 2 } else { 0 },
                        ("open", ..) => 0,
                        ("buried", ..) => 2,
                        ("snow", _, -1, _) => 8,
                        ("snow", _, ..=-2, _) => 9,
                        ("reef", _, 0.., _) => 7,
                        ("ocean", _, ..=-32, _) | ("magma", _, ..=-3, _) => 2,
                        ("ocean", _, -31..=-1, _) | ("magma", _, -2.., _) => 7,
                        ("lichen_water", -1, _, _) | ("lichen_water", _, -1, _) => 2,
                        ("lichen_water", ..) => 7,
                        ("geode", 8, _, _) | ("geode", _, _, -2) => 7,
                        ("geode", ..) => 2,
                        ("pale", -1, 0..=3, 0) => 2,
                        ("moss", 1, -1, 0) | ("pool", 1, -1, _) | ("pool", _, -1, -1) => 3,
                        ("crimson", 0, 1, 0) => 4,
                        (_, _, 0.., _) => 0,
                        ("grass", ..) => 1,
                        ("crimson", ..) => 5,
                        ("warped", ..) => 6,
                        _ => 2,
                    };
                    if index != 0 && !region.outside_build_height(y + dy) {
                        region.set_block_in_section(x + dx, y + dy, z + dz, index);
                    }
                }
            }
        }
        region.start_log();
        region.set_level_random(WorldgenRng::direct(Xoroshiro::new(seed as i64 + 100)));
        let mut rng = WorldgenRng::new(seed as i64);
        let placed = dec.place_grower(&format!("minecraft:{}", fields[0]), &mut region, &mut rng, (x, y, z)).unwrap();
        let writes = region.take_log();
        let hash = writes.iter().fold(HASH_START, |h, &(x, y, z, state, _)| {
            mix(mix_pos(h, (x, y, z)), palette[state as usize] as i32)
        });
        assert_eq!((placed, hash, rng.next_int()), (expected, hash_expected, random_expected), "{line}");
        if fields.len() > 6 {
            let effects = region.take_effects();
            let flags = writes.iter().fold(HASH_START, |h, &(_, _, _, _, flags)| mix(h, flags.unwrap() as i32));
            let block_ticks = effects.block_ticks.iter().fold(HASH_START, |h, &(p, state)| {
                mix(mix_pos(h, p), palette[state as usize] as i32)
            });
            let fluid_ticks = effects.fluid_ticks.iter().fold(HASH_START, |h, &(p, fluid)| {
                let ty = match fluid {
                    Fluid::Empty => unreachable!(),
                    Fluid::Water => 1,
                    Fluid::FlowingWater => 2,
                    Fluid::Lava => 3,
                    Fluid::FlowingLava => 4,
                };
                mix(mix_pos(h, p), ty)
            });
            let post_processing = effects.post_processing.into_iter().fold(HASH_START, mix_pos);
            let expected = std::array::from_fn(|i| u64::from_str_radix(fields[i + 6], 16).unwrap());
            assert_eq!([flags, block_ticks, fluid_ticks, post_processing], expected, "effects for {line}");
        }
    }
}
