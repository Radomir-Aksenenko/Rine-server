//! Vanilla bone meal targets that place worldgen features into the live level.

use std::sync::OnceLock;

use rine_protocol::BlockPos;
use rine_world::chunk::{MAX_Y, MIN_Y};
use rine_worldgen::feature::region::{LiveRegion, Pos, Region};
use rine_worldgen::feature::rng::WorldgenRng;
use rine_worldgen::feature::Decorator;

use super::{block_id, block_of, bonemeal, is_air, sapling, Rng};
use crate::game::redstone::engine::{self, fl, prio, Lvl, Rs, UPDATE_LIMIT};

const WORLD_HEIGHT: i32 = MAX_Y - MIN_Y + 1;

const GROWERS: [&str; 5] = [
    "minecraft:moss_patch_bonemeal",
    "minecraft:pale_moss_patch_bonemeal",
    "minecraft:nylium_bonemeal",
    "minecraft:crimson_fungus_planted",
    "minecraft:warped_fungus_planted",
];

#[derive(Clone, Copy)]
enum Target {
    Grass,
    Moss(usize),
    Nylium,
    Fungus { feature: usize, ground: u16 },
    Carpet,
}

struct Tables {
    targets: Vec<Option<Target>>,
    growth: sapling::Growth,
    features: [usize; 5],
    grass_feature: usize,
    grass: u16,
    short_grass: u16,
    air: u16,
}

fn tables() -> Option<&'static Tables> {
    static TABLES: OnceLock<Option<Tables>> = OnceLock::new();
    TABLES.get_or_init(|| {
        let data = rine_data::get();
        let biomes = &data.registries.synced("minecraft:worldgen/biome").entries;
        let growth = sapling::build_with(|names, defaults| Decorator::new_bone_meal(names, defaults, &GROWERS, biomes))?;
        let mut features = [0; 5];
        for (i, name) in GROWERS.iter().enumerate() {
            features[i] = growth.dec.grower_index(name)?;
        }
        let grass_feature = growth.dec.grower_index("minecraft:grass_bonemeal")?;
        let grass = block_id("grass_block");
        let mut targets = vec![None; data.blocks.blocks.len()];
        for (name, target) in [
            ("grass_block", Target::Grass),
            ("moss_block", Target::Moss(features[0])),
            ("pale_moss_block", Target::Moss(features[1])),
            ("crimson_nylium", Target::Nylium),
            ("warped_nylium", Target::Nylium),
            ("crimson_fungus", Target::Fungus { feature: features[3], ground: block_id("crimson_nylium") }),
            ("warped_fungus", Target::Fungus { feature: features[4], ground: block_id("warped_nylium") }),
            ("pale_moss_carpet", Target::Carpet),
        ] {
            targets[block_id(name) as usize] = Some(target);
        }
        Some(Tables {
            targets, growth, features, grass_feature, grass, short_grass: block_id("short_grass"),
            air: super::default_state("air"),
        })
    }).as_ref()
}

pub(super) fn prepare() {
    let _ = tables();
}

struct Live<'a, L> {
    rs: &'a mut Rs,
    level: &'a mut L,
    growth: &'a sapling::Growth,
}

fn pos((x, y, z): Pos) -> BlockPos {
    BlockPos { x, y, z }
}

fn coords(pos: BlockPos) -> Pos {
    (pos.x, pos.y, pos.z)
}

impl<L: Lvl> LiveRegion for Live<'_, L> {
    fn block(&self, p: Pos) -> u16 {
        self.level.get(pos(p))
    }

    fn set(&mut self, p: Pos, state: u16, flags: u32) -> bool {
        engine::set_block(self.rs, self.level, pos(p), sapling::live_id(self.growth, state), flags, UPDATE_LIMIT)
    }

    fn destroy(&mut self, p: Pos) {
        engine::destroy_block(self.rs, self.level, pos(p), true, UPDATE_LIMIT);
    }

    fn schedule(&mut self, p: Pos, state: u16) {
        engine::schedule(self.rs, self.level, pos(p), block_of(sapling::live_id(self.growth, state)), 1, prio::NORMAL);
    }
}

impl<L: Lvl> Live<'_, L> {
    fn place(&mut self, index: usize, p: BlockPos, rng: &mut WorldgenRng) {
        let dec = &self.growth.dec;
        let mut region = Region::live(dec.tags(), MIN_Y, WORLD_HEIGHT, self);
        dec.place_grower_at(index, &mut region, rng, coords(p));
    }
}

fn inside(p: BlockPos) -> bool {
    (MIN_Y..MIN_Y + WORLD_HEIGHT).contains(&p.y)
}

/// GrassBlock.performBonemeal and placeBonemealEffect.
fn spread_grass<L: Lvl>(live: &mut Live<'_, L>, t: &Tables, above: BlockPos, rng: &mut WorldgenRng) {
    let dec = &t.growth.dec;
    'attempts: for attempt in 0..128 {
        let mut p = above;
        for _ in 0..attempt / 16 {
            let x = rng.next_int_between(-1, 1);
            let y = rng.next_int_between(-1, 1) * rng.next_int_bound(3) / 2;
            let z = rng.next_int_between(-1, 1);
            p = p.offset(x, y, z);
            if block_of(live.level.get(p.offset(0, -1, 0))) != t.grass
                || dec.tags().full_collision(live.level.get(p))
            {
                continue 'attempts;
            }
        }
        let state = live.level.get(p);
        if block_of(state) == t.short_grass && rng.next_float() < 0.1 {
            bonemeal::tall_grass(live.rs, live.level, t.short_grass, p);
        }
        if is_air(state) && inside(p) {
            let feature = if rng.next_float() < 0.125 {
                let flowers = dec.bone_meal_flowers(live.level.bone_meal_biome(p));
                if flowers.is_empty() {
                    continue;
                }
                flowers[rng.next_int_bound(flowers.len() as i32) as usize]
            } else {
                t.grass_feature
            };
            live.place(feature, p, rng);
        }
    }
}

pub(super) fn bone_meal<L: Lvl>(rs: &mut Rs, level: &mut L, state: u16, p: BlockPos, rng: &mut Rng) -> bool {
    let Some(t) = tables() else { return false };
    let Some(target) = t.targets[block_of(state) as usize] else { return false };
    let above = p.offset(0, 1, 0);
    let mut live = Live { rs, level, growth: &t.growth };
    let mut random = rng.feature_random();
    match target {
        Target::Grass | Target::Nylium => {
            if !inside(above) || !is_air(live.level.get(above)) {
                return false;
            }
            if matches!(target, Target::Grass) {
                spread_grass(&mut live, t, above, &mut random);
            } else {
                live.place(t.features[2], above, &mut random);
            }
        }
        Target::Moss(feature) => {
            if !is_air(live.level.get(above)) {
                return false;
            }
            live.place(feature, above, &mut random);
        }
        Target::Fungus { feature, ground } => {
            if !inside(above) || block_of(live.level.get(p.offset(0, -1, 0))) != ground {
                return false;
            }
            if (random.next_float() as f64) < 0.4 {
                live.place(feature, p, &mut random);
            }
        }
        Target::Carpet => {
            let dec = &t.growth.dec;
            let mut region = Region::live(dec.tags(), MIN_Y, WORLD_HEIGHT, &mut live);
            let Some(topper) = dec.moss_carpet_topper(&mut region, coords(p), state) else { return false };
            region.write(coords(above), topper, fl::ALL);
        }
    }
    rng.finish_feature_random(random);
    true
}

pub(super) fn carpet_update_shape<L: Lvl>(rs: &mut Rs, level: &mut L, state: u16, p: BlockPos) -> u16 {
    let Some(t) = tables() else { return state };
    let dec = &t.growth.dec;
    let mut live = Live { rs, level, growth: &t.growth };
    let region = Region::live(dec.tags(), MIN_Y, WORLD_HEIGHT, &mut live);
    sapling::live_id(&t.growth, dec.moss_carpet_update(&region, coords(p), state, t.air))
}

#[cfg(test)]
mod tests {
    use super::super::{default_state, testw::World};
    use super::*;

    const ORIGIN: BlockPos = BlockPos { x: 0, y: 64, z: 0 };

    fn put_floor(world: &mut World, state: u16) {
        for x in -8..=8 {
            for z in -8..=8 {
                world.put(ORIGIN.offset(x, 0, z), state);
            }
        }
    }

    fn biome(name: &str) -> u8 {
        rine_data::get().registries.synced("minecraft:worldgen/biome").id(name).unwrap() as u8
    }

    #[test]
    fn grass_uses_the_local_biome_and_respects_the_build_ceiling() {
        rine_data::init();
        let mut world = World::new();
        world.biome = biome("minecraft:swamp");
        let grass = default_state("grass_block");
        put_floor(&mut world, grass);
        let mut rs = Rs::default();
        let mut rng = Rng::new(3);
        assert!(bone_meal(&mut rs, &mut world, grass, ORIGIN, &mut rng));
        assert!(world.cells.values().any(|&s| s == default_state("blue_orchid")));
        assert!(!world.cells.values().any(|&s| s == default_state("poppy")));
        assert!(!bone_meal(&mut rs, &mut world, grass, BlockPos { y: MAX_Y, ..ORIGIN }, &mut rng));
        let blocked = ORIGIN.offset(6, 0, 0);
        world.put(blocked.offset(0, 1, 0), default_state("stone"));
        assert!(!bone_meal(&mut rs, &mut world, grass, blocked, &mut rng));
    }

    #[test]
    fn grass_does_not_substitute_flowers_for_a_biome_with_no_candidates() {
        rine_data::init();
        let mut world = World::new();
        world.biome = biome("minecraft:warped_forest");
        let grass = default_state("grass_block");
        put_floor(&mut world, grass);
        let mut rng = Rng::new(3);
        assert!(bone_meal(&mut Rs::default(), &mut world, grass, ORIGIN, &mut rng));
        assert!(world.cells.values().any(|&s| block_of(s) == block_id("tall_grass")));
        assert!(world.cells.values().all(|&s| [grass, default_state("short_grass")].contains(&s)
            || block_of(s) == block_id("tall_grass")));
    }

    #[test]
    fn both_moss_blocks_replace_eligible_ground_but_keep_bedrock() {
        rine_data::init();
        for name in ["moss_block", "pale_moss_block"] {
            let mut world = World::new();
            let moss = default_state(name);
            let bedrock = default_state("bedrock");
            put_floor(&mut world, default_state("stone"));
            world.put(ORIGIN, moss);
            let neighbor = ORIGIN.offset(1, 0, 0);
            world.put(neighbor, bedrock);
            let mut rs = Rs::default();
            let mut rng = Rng::new(2);
            assert!(bone_meal(&mut rs, &mut world, moss, ORIGIN, &mut rng));
            assert_eq!(world.get(neighbor), bedrock);
            assert!(world.cells.values().filter(|&&s| s == moss).count() > 1);
            world.put(ORIGIN.offset(0, 1, 0), default_state("water"));
            assert!(!bone_meal(&mut rs, &mut world, moss, ORIGIN, &mut rng));
        }
    }

    #[test]
    fn pale_moss_topper_needs_a_full_side_and_a_base_layer() {
        rine_data::init();
        let mut world = World::new();
        let carpet = default_state("pale_moss_carpet");
        let mut rs = Rs::default();
        let mut rng = Rng::new(3);
        put_floor(&mut world, default_state("stone"));
        let p = ORIGIN.offset(0, 1, 0);
        world.put(p, carpet);
        let blocks = &rine_data::get().blocks;
        world.put(p.offset(0, 0, -1), default_state("stone"));
        let north = p.offset(0, 1, -1);
        world.put(north, blocks.parse_state("minecraft:stone_slab[type=bottom,waterlogged=false]").unwrap());
        assert!(!bone_meal(&mut rs, &mut world, carpet, p, &mut rng));
        world.put(north, default_state("glass"));
        let base = carpet_update_shape(&mut rs, &mut world, carpet, p);
        world.put(p, base);
        assert!(bone_meal(&mut rs, &mut world, base, p, &mut rng));
        let top = world.get(p.offset(0, 1, 0));
        assert_eq!(blocks.get_prop(top, "bottom"), Some("false"));
        assert_eq!(blocks.get_prop(top, "north"), Some("low"));
        assert_eq!(blocks.get_prop(world.get(p), "north"), Some("tall"));
        assert!(!bone_meal(&mut rs, &mut world, top, p.offset(0, 1, 0), &mut rng));
    }

    #[test]
    fn nylium_places_the_configured_vegetation_of_its_own_kind() {
        rine_data::init();
        for (name, vegetation) in [("crimson_nylium", "crimson_roots"), ("warped_nylium", "warped_roots")] {
            let mut world = World::new();
            let nylium = default_state(name);
            put_floor(&mut world, nylium);
            let mut rs = Rs::default();
            let mut rng = Rng::new(3);
            assert!(bone_meal(&mut rs, &mut world, nylium, ORIGIN, &mut rng));
            assert!(world.cells.values().any(|&s| s == default_state(vegetation)));
            world.put(ORIGIN.offset(0, 1, 0), default_state("stone"));
            assert!(!bone_meal(&mut rs, &mut world, nylium, ORIGIN, &mut rng));
            assert!(!bone_meal(&mut rs, &mut world, nylium, BlockPos { y: MAX_Y, ..ORIGIN }, &mut rng));
        }
    }

    #[test]
    fn planted_fungi_require_matching_nylium_and_drop_replaceable_plants() {
        rine_data::init();
        for (name, ground, wrong, stem) in [
            ("crimson_fungus", "crimson_nylium", "warped_nylium", "crimson_stem"),
            ("warped_fungus", "warped_nylium", "crimson_nylium", "warped_stem"),
        ] {
            let mut world = World::new();
            let fungus = default_state(name);
            let mut rs = Rs::default();
            let mut rng = Rng::new(3);
            let p = ORIGIN.offset(0, 1, 0);
            world.put(p, fungus);
            world.put(ORIGIN, default_state(wrong));
            assert!(!bone_meal(&mut rs, &mut world, fungus, p, &mut rng));
            world.put(ORIGIN, default_state(ground));
            let success = (0..128).find(|&seed| Rng::new(seed).next_float() < 0.4).unwrap();
            let fail = (0..128).find(|&seed| Rng::new(seed).next_float() >= 0.4).unwrap();
            assert!(bone_meal(&mut rs, &mut world, fungus, p, &mut Rng::new(fail)));
            assert_eq!(world.get(p), fungus);
            let obstruction = default_state("poppy");
            world.put(p.offset(0, 1, 0), obstruction);
            assert!(bone_meal(&mut rs, &mut world, fungus, p, &mut Rng::new(success)));
            assert_eq!(block_of(world.get(p)), block_id(stem));
            assert!(world.dropped.contains(&(p.offset(0, 1, 0), obstruction)));
        }
    }
}
