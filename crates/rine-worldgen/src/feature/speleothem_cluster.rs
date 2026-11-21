use serde_json::Value;

use super::placement::{parse_int_provider, sample_int, IntProvider};
use super::region::{Pos, Region};
use super::rng::WorldgenRng;
use super::speleothem::{is_water, offset, read, scan_column, SpeleothemBlocks, HORIZONTAL};
use super::Compiler;

pub(super) enum FloatProvider {
    Constant(f32),
    Uniform(f32, f32),
    Normal {
        mean: f32,
        deviation: f32,
        min: f32,
        max: f32,
    },
}

impl FloatProvider {
    pub fn parse(v: &Value) -> Result<Self, String> {
        let f = |k: &str| {
            v[k].as_f64()
                .map(|n| n as f32)
                .ok_or_else(|| format!("float provider: {k}"))
        };
        if let Some(n) = v.as_f64() {
            return Ok(Self::Constant(n as f32));
        }
        match v["type"].as_str() {
            Some("minecraft:uniform") => {
                Ok(Self::Uniform(f("min_inclusive")?, f("max_exclusive")?))
            }
            Some("minecraft:clamped_normal") => Ok(Self::Normal {
                mean: f("mean")?,
                deviation: f("deviation")?,
                min: f("min")?,
                max: f("max")?,
            }),
            _ => Err(format!("unsupported float provider {v}")),
        }
    }

    pub fn sample(&self, rng: &mut WorldgenRng) -> f32 {
        match *self {
            Self::Constant(v) => v,
            Self::Uniform(min, max) => rng.next_float() * (max - min) + min,
            Self::Normal {
                mean,
                deviation,
                min,
                max,
            } => normal(rng, mean, deviation, min, max),
        }
    }
}

fn normal(rng: &mut WorldgenRng, mean: f32, deviation: f32, min: f32, max: f32) -> f32 {
    (mean + rng.next_gaussian() as f32 * deviation).clamp(min, max)
}

pub(super) struct SpeleothemCluster {
    blocks: SpeleothemBlocks,
    range: i32,
    height: IntProvider,
    radius: IntProvider,
    height_diff: i32,
    deviation: f32,
    layer: IntProvider,
    density: FloatProvider,
    wetness: FloatProvider,
    edge_chance: f32,
    chance_distance: f32,
    bias_distance: f64,
    base_stone: u16,
    lava: u16,
    water_state: u16,
}

impl SpeleothemCluster {
    pub fn compile(v: &Value, c: &mut Compiler) -> Result<Self, String> {
        let int = |k: &str| {
            v[k].as_i64()
                .map(|n| n as i32)
                .ok_or_else(|| format!("speleothem_cluster: {k}"))
        };
        Ok(Self {
            blocks: SpeleothemBlocks::compile(v, c)?,
            range: int("floor_to_ceiling_search_range")?,
            height: parse_int_provider(&v["height"])?,
            radius: parse_int_provider(&v["radius"])?,
            height_diff: int("max_stalagmite_stalactite_height_diff")?,
            deviation: int("height_deviation")? as f32,
            layer: parse_int_provider(&v["speleothem_block_layer_thickness"])?,
            density: FloatProvider::parse(&v["density"])?,
            wetness: FloatProvider::parse(&v["wetness"])?,
            edge_chance: v["chance_of_speleothem_at_max_distance_from_center"]
                .as_f64()
                .ok_or("edge chance")? as f32,
            chance_distance: int("max_distance_from_edge_affecting_chance_of_speleothem")? as f32,
            bias_distance: int("max_distance_from_center_affecting_height_bias")? as f64,
            base_stone: c
                .tag_names
                .binary_search_by(|s| s.as_str().cmp("minecraft:base_stone_overworld"))
                .map_err(|_| "missing base_stone_overworld tag")? as u16,
            lava: c.pal.block("minecraft:lava"),
            water_state: c.pal.default_state("minecraft:water"),
        })
    }

    pub fn place(&self, region: &mut Region, rng: &mut WorldgenRng, pos: Pos) -> bool {
        let state = read(region, pos);
        if !(region.tags.is_air(state) || region.tags.is_block(state, self.blocks.water)) {
            return false;
        }
        let height = sample_int(&self.height, rng);
        let wetness = self.wetness.sample(rng);
        let density = self.density.sample(rng);
        let xr = sample_int(&self.radius, rng);
        let zr = sample_int(&self.radius, rng);
        for dx in -xr..=xr {
            for dz in -zr..=zr {
                let edge = (xr - dx.abs()).min(zr - dz.abs()) as f32;
                let chance = self.edge_chance
                    + (edge / self.chance_distance).min(1.0) * (1.0 - self.edge_chance);
                self.place_column(
                    region,
                    rng,
                    (pos.0 + dx, pos.1, pos.2 + dz),
                    dx.abs() + dz.abs(),
                    height,
                    wetness,
                    density,
                    chance as f64,
                );
            }
        }
        true
    }

    fn place_column(
        &self,
        region: &mut Region,
        rng: &mut WorldgenRng,
        (x, y, z): Pos,
        distance: i32,
        height: i32,
        wetness: f32,
        density: f32,
        chance: f64,
    ) {
        let Some((mut floor, ceiling)) = scan_column(
            region,
            (x, y, z),
            self.range,
            self.blocks.water,
            |region, _, state| {
                !region.tags.is_air(state) && !region.tags.is_block(state, self.blocks.water)
            },
        ) else {
            return;
        };
        if floor.is_none() && ceiling.is_none() {
            return;
        }
        if rng.next_float() < wetness {
            if let Some(f) = floor.filter(|&f| self.can_pool(region, (x, f, z))) {
                region.write((x, f, z), self.water_state, 2);
                floor = Some(f - 1);
            }
        }
        let want_down = rng.next_double() < chance;
        let mut down = 0;
        if let Some(c) = ceiling
            .filter(|&c| want_down && !region.tags.is_block(read(region, (x, c, z)), self.lava))
        {
            self.replace_layer(region, rng, (x, c, z), 1);
            down = self.height_at(
                rng,
                distance,
                density,
                floor.map_or(height, |f| height.min(c - f)),
            );
        }
        let want_up = rng.next_double() < chance;
        let mut up = 0;
        if let Some(f) =
            floor.filter(|&f| want_up && !region.tags.is_block(read(region, (x, f, z)), self.lava))
        {
            self.replace_layer(region, rng, (x, f, z), 0);
            up = if ceiling.is_some() {
                (down + rng.next_int_between(-self.height_diff, self.height_diff)).max(0)
            } else {
                self.height_at(rng, distance, density, height)
            };
        }
        let mut gap = None;
        if let (Some(f), Some(c)) = (floor, ceiling) {
            gap = Some(c - f - 1);
            if c - down <= f + up {
                let bottom = rng.next_int_between((c - down).max(f + 1), (f + up).min(c - 1) + 1);
                down = c - bottom;
                up = bottom - 1 - f;
            }
        }
        let merge = rng.next_bool() && down > 0 && up > 0 && gap == Some(down + up);
        if let Some(c) = ceiling {
            self.blocks.grow(region, (x, c - 1, z), 0, down, merge);
        }
        if let Some(f) = floor {
            self.blocks.grow(region, (x, f + 1, z), 1, up, merge);
        }
    }

    fn height_at(&self, rng: &mut WorldgenRng, distance: i32, density: f32, max: i32) -> i32 {
        if rng.next_float() > density {
            return 0;
        }
        let center = max as f64 / 2.0;
        let mean = center + (distance as f64 / self.bias_distance).min(1.0) * -center;
        normal(rng, mean as f32, self.deviation, 0.0, max as f32) as i32
    }

    fn replace_layer(&self, region: &mut Region, rng: &mut WorldgenRng, mut pos: Pos, dir: usize) {
        for _ in 0..sample_int(&self.layer, rng) {
            if !self.blocks.place_base(region, pos) {
                break;
            }
            pos = offset(pos, dir);
        }
    }

    fn can_pool(&self, region: &Region, pos: Pos) -> bool {
        let block = region.tags.block_of(read(region, pos));
        ![self.blocks.water, self.blocks.base, self.blocks.pointed].contains(&block)
            && !is_water(region, read(region, offset(pos, 1)))
            && HORIZONTAL.into_iter().chain([0]).all(|dir| {
                let state = read(region, offset(pos, dir));
                region.tags.has(state, self.base_stone) || is_water(region, state)
            })
    }
}
