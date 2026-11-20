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
    for name in ["pointed_dripstone", "sulfur_spike", "dripstone_cluster", "sulfur_spike_cluster",
        "large_dripstone", "rooted_azalea_tree", "rooted_sulfur_spring", "monster_room", "monster_room_deep",
        "fossil_upper", "fossil_lower", "desert_well", "sculk_patch_deep_dark"] {
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
    let terrain = ["air", "grass_block", "stone", "bedrock", "poppy", "crimson_nylium", "warped_nylium", "dirt"].map(|name| {
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
        if matches!(fields[1], "cave" | "solid") {
            region.fill_wg_heights(y + 37);
        }
        let ceiling = if seed & 1 == 0 { 8 } else { 20 };
        let reach = if fields[1] == "cave" { 16 } else { 12 };
        for dz in -reach..=reach {
            for dx in -reach..=reach {
                for dy in -36..=36 {
                    let index = match (fields[1], dx, dy, dz) {
                        ("cave", _, dy, _) => if dy < 0 || dy >= ceiling { 2 } else { 0 },
                        ("root", _, 4, _) => 7,
                        ("root", _, dy, _) => if dy < 0 || (1..=3).contains(&dy) { 2 } else { 0 },
                        ("room", 0, 0..=1, 3..=4) => 0,
                        ("room", ..) => 2,
                        ("solid", ..) => 2,
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
                        if matches!(fields[1], "cave" | "root") {
                            region.set_block(x + dx, y + dy, z + dz, index);
                        } else {
                            region.set_block_in_section(x + dx, y + dy, z + dz, index);
                        }
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
        if let Some(saved) = fields.get(6) {
            let mut entities: Vec<_> = region.block_entities.iter().flat_map(|chunk| chunk.values()).collect();
            entities.sort_by_key(|entity| entity.pos);
            assert_eq!(entity_hash(&entities), u64::from_str_radix(saved, 16).unwrap(), "{line}");
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
    let ticks = region.take_scheduled();
    assert_eq!(ticks.iter().filter(|tick| dec.palette()[tick.1 as usize].starts_with("minecraft:suspicious_sand[")).count(), 2);

    let mut empty = dec.grower_region(0, 0, vec![vec![0; 384 * 256]; 9]);
    empty.fill_wg_heights(64);
    empty.start_log();
    assert_eq!(dec.place_grower("minecraft:fossil_coal", &mut empty, &mut WorldgenRng::new(0), (8, 40, 8)), Some(false));
    assert!(empty.take_log().is_empty());
}
