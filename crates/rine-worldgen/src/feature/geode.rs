use std::sync::OnceLock;

use serde_json::Value;

use super::{
    block_set,
    multiface::{relative, FACE_NAMES},
    placement::{parse_int_provider, sample_int, IntProvider},
    predicate::BlockPredicate,
    provider_value,
    region::{Pos, Region},
    rng::WorldgenRng,
    state::Fluid,
    state_provider::{self, StateProvider},
    Compiler,
};
use crate::{
    noise::{NoiseStack, NormalNoise},
    random::AnyRng,
};

const DISTANCE_TABLE_LEN: usize = 4096;
const MAX_DISTRIBUTION_POINTS: usize = 20;
static INVERSE_DISTANCES: OnceLock<[f64; DISTANCE_TABLE_LEN]> = OnceLock::new();

pub(super) struct Geode {
    layers: [StateProvider; 5],
    thresholds: [f64; 4],
    crystals: Vec<[[u16; 2]; 6]>,
    protected: BlockPredicate,
    invalid: BlockPredicate,
    walls: IntProvider,
    distribution: IntProvider,
    point_offset: IntProvider,
    offsets: (i32, i32),
    cracks: (f64, f64, i32),
    alternate: f64,
    crystal_chance: f64,
    require_alternate: bool,
    noise_scale: f64,
    max_invalid: i32,
    air: u16,
    water: u16,
    falling_water: [u16; 8],
    noise: OnceLock<(i64, NoiseStack)>,
}

impl Geode {
    pub fn compile(cfg: &Value, c: &mut Compiler) -> Result<Self, String> {
        let blocks = &cfg["blocks"];
        let mut provider = |key| {
            state_provider::parse(
                provider_value(c.bundle, &blocks[key]),
                &c.tag_names,
                &mut c.pal,
            )
        };
        let layers = [
            provider("filling_provider")?,
            provider("inner_layer_provider")?,
            provider("alternate_inner_layer_provider")?,
            provider("middle_layer_provider")?,
            provider("outer_layer_provider")?,
        ];
        let mut crystals = Vec::new();
        for state in blocks["inner_placements"]
            .as_array()
            .filter(|v| !v.is_empty())
            .ok_or("inner_placements")?
        {
            let state = state_provider::state_of(state, &mut c.pal)?;
            crystals.push(FACE_NAMES.map(|face| {
                let state = c.pal.with_property(state, "facing", face).unwrap_or(state);
                ["false", "true"].map(|water| {
                    c.pal
                        .with_property(state, "waterlogged", water)
                        .unwrap_or(state)
                })
            }));
        }
        let range = |name, min, max| {
            if let Some(v) = cfg.get(name) {
                parse_int_provider(v)
            } else {
                Ok(IntProvider::Uniform { min, max })
            }
        };
        let layer = &cfg["layers"];
        let crack = &cfg["crack"];
        let distribution = range("distribution_points", 3, 4)?;
        let (least, most) = distribution.bounds();
        if least < 1 || most > MAX_DISTRIBUTION_POINTS as i32 {
            return Err("distribution_points outside geode range".into());
        }
        Ok(Self {
            layers,
            crystals,
            thresholds: [
                layer["filling"].as_f64().unwrap_or(1.7),
                layer["inner_layer"].as_f64().unwrap_or(2.2),
                layer["middle_layer"].as_f64().unwrap_or(3.2),
                layer["outer_layer"].as_f64().unwrap_or(4.2),
            ],
            protected: block_set(&blocks["cannot_replace"], &c.tag_names, &mut c.pal)?,
            invalid: block_set(&blocks["invalid_blocks"], &c.tag_names, &mut c.pal)?,
            walls: range("outer_wall_distance", 4, 5)?,
            distribution,
            point_offset: range("point_offset", 1, 2)?,
            offsets: (
                cfg["min_gen_offset"].as_i64().unwrap_or(-16) as i32,
                cfg["max_gen_offset"].as_i64().unwrap_or(16) as i32,
            ),
            cracks: (
                crack["generate_crack_chance"].as_f64().unwrap_or(1.0),
                crack["base_crack_size"].as_f64().unwrap_or(2.0),
                crack["crack_point_offset"].as_i64().unwrap_or(2) as i32,
            ),
            alternate: cfg["use_alternate_layer0_chance"].as_f64().unwrap_or(0.0),
            crystal_chance: cfg["use_potential_placements_chance"]
                .as_f64()
                .unwrap_or(0.35),
            require_alternate: cfg["placements_require_layer0_alternate"]
                .as_bool()
                .unwrap_or(true),
            noise_scale: cfg["noise_multiplier"].as_f64().unwrap_or(0.05),
            max_invalid: cfg["invalid_blocks_threshold"]
                .as_i64()
                .ok_or("invalid_blocks_threshold")? as i32,
            air: c.pal.default_state("minecraft:air"),
            water: c.pal.block("minecraft:water"),
            falling_water: std::array::from_fn(|i| {
                c.pal
                    .state_from_parts("minecraft:water", &[("level", &(8 + i).to_string())])
            }),
            noise: OnceLock::new(),
        })
    }

    pub fn place(&self, level: &mut Region, rng: &mut WorldgenRng, (ox, oy, oz): Pos) -> bool {
        let count = sample_int(&self.distribution, rng) as usize;
        let adjustment = count as f64 / self.walls.bounds().1 as f64;
        let [filling, inner, middle, outer] = self.thresholds;
        let [filling, inner, middle, outer] = [
            filling,
            inner + adjustment,
            middle + adjustment,
            outer + adjustment,
        ]
        .map(|v| 1.0 / v.sqrt());
        let (crack_chance, base, crack_offset) = self.cracks;
        let crack_threshold = 1.0
            / (base + rng.next_double() / 2.0 + if count > 3 { adjustment } else { 0.0 }).sqrt();
        let crack = (rng.next_float() as f64) < crack_chance;
        let mut points = [((0, 0, 0), 0); MAX_DISTRIBUTION_POINTS];
        let points = &mut points[..count];
        let mut invalid = 0;
        for point in points.iter_mut() {
            let x = sample_int(&self.walls, rng);
            let y = sample_int(&self.walls, rng);
            let z = sample_int(&self.walls, rng);
            let pos = (ox + x, oy + y, oz + z);
            if level.is_air(pos.0, pos.1, pos.2) || self.invalid.test(level, pos.0, pos.1, pos.2) {
                invalid += 1;
                if invalid > self.max_invalid {
                    return false;
                }
            }
            *point = ((x, y, z), sample_int(&self.point_offset, rng));
        }
        let crack_points = if crack {
            let offset = count as i32 * 2 + 1;
            let (x, z) = [(offset, 0), (0, offset), (offset, offset), (0, 0)]
                [rng.next_int_bound(4) as usize];
            [7, 5, 1].map(|y| ((x, y, z), crack_offset))
        } else {
            [((0, 0, 0), 0); 3]
        };
        let mut potential = Vec::new();
        let seed = level.world_seed();
        let (stored_seed, noise) = self.noise.get_or_init(|| (seed, geode_noise(seed)));
        let fallback;
        let noise = if *stored_seed == seed {
            noise
        } else {
            fallback = geode_noise(seed);
            &fallback
        };
        let table =
            INVERSE_DISTANCES.get_or_init(|| std::array::from_fn(|i| 1.0 / (i as f64).sqrt()));
        let (min, max) = if self.offsets.0 <= self.offsets.1 {
            self.offsets
        } else {
            (self.offsets.1, self.offsets.0)
        };
        for z in min..=max {
            for y in min..=max {
                for x in min..=max {
                    let pos = (ox + x, oy + y, oz + z);
                    let offset = noise.get_value(pos.0 as f64, pos.1 as f64, pos.2 as f64) as f64
                        * self.noise_scale;
                    let sum = influence(points, (x, y, z), offset, table);
                    if sum < outer {
                        continue;
                    }
                    if sum >= filling {
                        self.set_layer(level, rng, pos, 0);
                    } else if crack
                        && influence(&crack_points, (x, y, z), offset, table) >= crack_threshold
                    {
                        self.safe_set(level, pos, self.air);
                        for d in 0..6 {
                            let p = relative(pos, d);
                            let fluid = level.tags.fluid(level.get_block(p.0, p.1, p.2));
                            if fluid != Fluid::Empty {
                                level.schedule_fluid(p, fluid);
                            }
                        }
                    } else if sum >= inner {
                        let alternate = (rng.next_float() as f64) < self.alternate;
                        self.set_layer(level, rng, pos, if alternate { 2 } else { 1 });
                        if (!self.require_alternate || alternate)
                            && (rng.next_float() as f64) < self.crystal_chance
                        {
                            potential.push(pos);
                        }
                    } else if sum >= middle {
                        self.set_layer(level, rng, pos, 3);
                    } else if sum >= outer {
                        self.set_layer(level, rng, pos, 4);
                    }
                }
            }
        }
        for pos in potential {
            let variants = &self.crystals[rng.next_int_bound(self.crystals.len() as i32) as usize];
            for (face, states) in variants.iter().enumerate() {
                let p = relative(pos, face);
                let old = level.get_block(p.0, p.1, p.2);
                let fluid = level.tags.fluid(old);
                if level.tags.is_air(old)
                    || level.tags.is_block(old, self.water)
                        && (fluid == Fluid::Water || self.falling_water.contains(&old))
                {
                    self.safe_set(
                        level,
                        p,
                        states[usize::from(matches!(fluid, Fluid::Water | Fluid::Lava))],
                    );
                    break;
                }
            }
        }
        true
    }

    fn set_layer(&self, level: &mut Region, rng: &mut WorldgenRng, p: Pos, layer: usize) {
        let state = self.layers[layer].get(level, rng, p.0, p.1, p.2);
        self.safe_set(level, p, state);
    }

    fn safe_set(&self, level: &mut Region, p: Pos, state: u16) {
        if !self.protected.test(level, p.0, p.1, p.2) {
            level.write(p, state, 2);
        }
    }
}

fn influence(points: &[(Pos, i32)], (x, y, z): Pos, noise: f64, table: &[f64]) -> f64 {
    points.iter().fold(0.0, |sum, &((px, py, pz), offset)| {
        let dx = x as f64 - px as f64;
        let dy = y as f64 - py as f64;
        let dz = z as f64 - pz as f64;
        let d = dx * dx + dy * dy + dz * dz + offset as f64;
        let weight = table
            .get(d as usize)
            .copied()
            .unwrap_or_else(|| 1.0 / d.sqrt());
        sum + (weight + noise)
    })
}

fn geode_noise(seed: i64) -> NoiseStack {
    NormalNoise::create_parity(-4, &[1.0]).create(&mut AnyRng::new_legacy(seed))
}
