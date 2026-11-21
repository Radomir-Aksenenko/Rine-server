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
        "iceberg_packed", "underwater_magma", "warm_ocean_vegetation", "glow_lichen", "sculk_vein", "amethyst_geode",
        "pointed_dripstone", "sulfur_spike", "dripstone_cluster", "sulfur_spike_cluster",
        "large_dripstone", "rooted_azalea_tree", "rooted_sulfur_spring", "monster_room", "monster_room_deep",
        "fossil_upper", "fossil_lower", "desert_well", "sculk_patch_deep_dark"] {
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
    let terrain = [
        "air", "grass_block", "stone", "bedrock", "poppy", "crimson_nylium", "warped_nylium",
        "mycelium", "sand", "dirt", "water", "water[level=2]", "water[level=8]", "lava", "frosted_ice", "ice", "snow_block",
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
        let scenario = fields[1];
        let (x, y, z) = (seed * 13 - 80, if scenario == "low" { -60 } else { 64 }, -47 + seed * 3);
        let (cx, cz) = (x.div_euclid(16), z.div_euclid(16));
        let mut region = dec.grower_region(cx, cz, vec![vec![0; 384 * 256]; 9]);
        // Underground scenarios (`cave`, `room`, `root`, `solid`, and block entity lines with 7 fields) use their own
        // terrain reach; effect lines (10 fields) and the rest keep theirs.
        let underground = matches!(scenario, "cave" | "room" | "root" | "solid") || fields.len() == 7;
        if matches!(scenario, "cave" | "solid") {
            region.fill_wg_heights(y + 37);
        }
        let ceiling = if seed & 1 == 0 { 8 } else { 20 };
        let radius = if underground {
            if scenario == "cave" { 16 } else { 12 }
        } else if fields.len() > 6 && !matches!(scenario, "ocean" | "magma" | "geode" | "reef" | "lichen_water") {
            12
        } else {
            16
        };
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                for dy in -36..=36 {
                    let index = match (fields[1], dx, dy, dz) {
                        ("cave", _, dy, _) => if dy < 0 || dy >= ceiling { 2 } else { 0 },
                        ("root", _, 4, _) => 9,
                        ("root", _, dy, _) => if dy < 0 || (1..=3).contains(&dy) { 2 } else { 0 },
                        ("room", 0, 0..=1, 3..=4) => 0,
                        ("room", ..) => 2,
                        ("solid", ..) => 2,
                        ("ceiling", _, dy, _) => if dy >= 1 { 2 } else { 0 },
                        ("open", ..) => 0,
                        ("buried", ..) => 2,
                        ("snow", _, -1, _) => 16,
                        ("snow", _, ..=-2, _) => 9,
                        ("reef", _, 0.., _) => 10,
                        ("ocean", _, ..=-32, _) | ("magma", _, ..=-3, _) => 2,
                        ("ocean", _, -31..=-1, _) | ("magma", _, -2.., _) => 10,
                        ("lichen_water", -1, _, _) | ("lichen_water", _, -1, _) => 2,
                        ("lichen_water", ..) => 10,
                        ("geode", 8, _, _) | ("geode", _, _, -2) => 10,
                        ("geode", ..) => 2,
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
                    if index != 0 && !region.outside_build_height(y + dy) {
                        if matches!(scenario, "cave" | "root") {
                            region.set_block(x + dx, y + dy, z + dz, index);
                        } else {
                            region.set_block_in_section(x + dx, y + dy, z + dz, index);
                        }
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
        let writes = region.take_log();
        let hash = writes.iter().fold(HASH_START, |h, &(x, y, z, state, _)| {
            mix(mix_pos(h, (x, y, z)), palette[state as usize] as i32)
        });
        assert_eq!((placed, hash, rng.next_int()), (expected, hash_expected, random_expected), "{line}");
        if fields.len() == 10 {
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
        } else if fields.len() == 7 {
            let mut entities: Vec<_> = region.block_entities.iter().flat_map(|chunk| chunk.values()).collect();
            entities.sort_by_key(|entity| entity.pos);
            assert_eq!(entity_hash(&entities), u64::from_str_radix(fields[6], 16).unwrap(), "{line}");
        }
    }
}

fn entity_hash(entities: &[&block_entity::BlockEntity]) -> u64 {
    use block_entity::BlockEntityData::*;
    fn mix(hash: &mut u64, v: i64) {
        *hash = (*hash ^ v as u64).wrapping_mul(0x100000001b3);
    }
    fn string(hash: &mut u64, s: &str) {
        for ch in s.bytes() { mix(hash, ch as i64); }
        mix(hash, 0);
    }
    let mut h = 0xcbf29ce484222325;
    for entity in entities {
        let (x, y, z) = entity.pos;
        for v in [x, y, z] { mix(&mut h, v as i64); }
        string(&mut h, &entity.type_id);
        match &entity.data {
            Loot { table, seed } => { string(&mut h, table); mix(&mut h, *seed); }
            Spawner { entity } => string(&mut h, entity),
            Beehive { ticks_in_hive } => {
                mix(&mut h, ticks_in_hive.len() as i64);
                for &ticks in ticks_in_hive {
                    string(&mut h, "minecraft:bee");
                    mix(&mut h, ticks as i64);
                    mix(&mut h, 600);
                }
            }
            SculkCatalyst => mix(&mut h, 0),
            PotentSulfur => mix(&mut h, -1),
            SculkSensor | SculkShrieker => {
                for v in [0, 0, -1] { mix(&mut h, v); }
            }
            Empty => { string(&mut h, ""); mix(&mut h, 0); }
        }
    }
    h
}

#[test]
fn replacing_a_neighbor_block_discards_its_entity() {
    let dec = Decorator::new_growers(&["minecraft:air".into()], HashMap::new(), &["minecraft:monster_room"]).unwrap();
    let chest = dec.palette().iter().position(|p| p.starts_with("minecraft:chest[facing=north")).unwrap() as u16;
    let stone = dec.palette().iter().position(|p| p == "minecraft:cobblestone").unwrap() as u16;
    let mut region = dec.grower_region(-1, 0, vec![vec![0; 384 * 256]; 9]);
    let pos = (0, 42, 0);
    region.write(pos, chest, 2);
    region.record_block_entity(pos, "minecraft:chest", block_entity::BlockEntityData::Loot {
        table: "minecraft:chests/simple_dungeon".into(), seed: -42,
    });
    assert!(region.block_entities[5].contains_key(&pos));
    region.write(pos, chest, 2);
    assert!(region.block_entities[5].contains_key(&pos));
    region.write(pos, stone, 2);
    region.write(pos, chest, 2);
    assert!(!region.block_entities[5].contains_key(&pos));
}

#[test]
fn templates_generate_fossil_and_well_with_archaeology_data() {
    let terrain = ["minecraft:air".into(), "minecraft:stone".into()];
    let dec = Decorator::new_growers(&terrain, HashMap::new(), &[
        "minecraft:fossil_coal", "minecraft:fossil_diamonds", "minecraft:desert_well", "minecraft:rooted_sulfur_spring",
    ]).unwrap();
    let mut region = dec.grower_region(0, 0, vec![vec![1; 384 * 256]; 9]);
    region.fill_wg_heights(64);
    for (name, x) in [("minecraft:fossil_coal", 8), ("minecraft:fossil_diamonds", 13)] {
        region.start_log();
        assert_eq!(dec.place_grower(name, &mut region, &mut WorldgenRng::new(0), (x, 40, 8)), Some(true));
        let placed = region.take_log();
        assert!(placed.iter().any(|w| dec.palette()[w.3 as usize].starts_with("minecraft:bone_block[")));
        assert!(placed.iter().all(|w| w.4 == Some(260)));
    }
    assert_eq!(dec.place_grower("minecraft:desert_well", &mut region, &mut WorldgenRng::new(0), (8, 64, 8)), Some(true));
    let entities: Vec<_> = region.block_entities[4].values().collect();
    assert_eq!(entities.len(), 2);
    for entity in entities {
        assert_eq!(entity.type_id, "minecraft:brushable_block");
        assert_eq!(entity.data, block_entity::BlockEntityData::Loot {
            table: "minecraft:archaeology/desert_well".into(),
            seed: {
                use crate::random::{Legacy, Rng};
                Legacy::new(crate::mth::get_seed(entity.pos.0, entity.pos.1, entity.pos.2)).next_long()
            },
        });
    }
    let ticks = region.take_effects().block_ticks;
    assert_eq!(ticks.iter().filter(|tick| dec.palette()[tick.1 as usize].starts_with("minecraft:suspicious_sand[")).count(), 2);

    let mut empty = dec.grower_region(0, 0, vec![vec![0; 384 * 256]; 9]);
    empty.fill_wg_heights(64);
    empty.start_log();
    assert_eq!(dec.place_grower("minecraft:fossil_coal", &mut empty, &mut WorldgenRng::new(0), (8, 40, 8)), Some(false));
    assert!(empty.take_log().is_empty());
}
