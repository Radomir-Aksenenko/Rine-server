use serde_json::Value;

use super::heightmap::Kind;
use crate::mth::{cos as mth_cos, sin as mth_sin};
use super::placement::parse_int_provider;
use super::predicate::BlockPredicate;
use super::region::{Pos, Region};
use super::rng::WorldgenRng;
use super::speleothem::{read, scan_column};
use super::speleothem_cluster::FloatProvider;
use super::{block_set, Compiler};

const MAX_RADIUS: usize = 16;

pub(super) struct LargeDripstone {
    replaceable: BlockPredicate,
    range: i32,
    radius: (i32, i32),
    height: FloatProvider,
    ratio: f32,
    blunt: [FloatProvider; 2],
    speed: FloatProvider,
    wind_radius: i32,
    wind_blunt: f64,
    state: u16,
    block: u16,
    water: u16,
    lava: u16,
    stone: u16,
}

impl LargeDripstone {
    pub fn compile(v: &Value, c: &mut Compiler) -> Result<Self, String> {
        let radius = parse_int_provider(&v["column_radius"])?.bounds();
        if radius.0 < 1 || radius.1 > MAX_RADIUS as i32 || radius.0 > radius.1 {
            return Err("large_dripstone radius outside codec bounds".into());
        }
        Ok(Self {
            replaceable: block_set(&v["replaceable_blocks"], &c.tag_names, &mut c.pal)?,
            range: v["floor_to_ceiling_search_range"].as_i64().unwrap_or(30) as i32,
            radius,
            height: FloatProvider::parse(&v["height_scale"])?,
            ratio: v["max_column_radius_to_cave_height_ratio"]
                .as_f64()
                .ok_or("cave radius ratio")? as f32,
            blunt: [
                FloatProvider::parse(&v["stalactite_bluntness"])?,
                FloatProvider::parse(&v["stalagmite_bluntness"])?,
            ],
            speed: FloatProvider::parse(&v["wind_speed"])?,
            wind_radius: v["min_radius_for_wind"].as_i64().ok_or("wind radius")? as i32,
            wind_blunt: v["min_bluntness_for_wind"]
                .as_f64()
                .ok_or("wind bluntness")? as f32 as f64,
            state: c.pal.default_state("minecraft:dripstone_block"),
            block: c.pal.block("minecraft:dripstone_block"),
            water: c.pal.block("minecraft:water"),
            lava: c.pal.block("minecraft:lava"),
            stone: c
                .tag_names
                .binary_search_by(|s| s.as_str().cmp("minecraft:base_stone_overworld"))
                .map_err(|_| "missing base_stone_overworld tag")? as u16,
        })
    }

    pub fn place(&self, region: &mut Region, rng: &mut WorldgenRng, (x, y, z): Pos) -> bool {
        let Some((Some(floor), Some(ceiling))) =
            scan_column(region, (x, y, z), self.range, self.water, |region, p, s| {
                let block = region.tags.block_of(s);
                block == self.block
                    || self.replaceable.test(region, p.0, p.1, p.2)
                    || block == self.lava
            })
        else {
            return false;
        };
        let gap = ceiling - floor - 1;
        if gap < 4 {
            return false;
        }
        let (min, max) = self.radius;
        let radius = rng.next_int_between(
            min,
            (gap as f32 * self.ratio).clamp(min as f32, max as f32) as i32,
        );
        let mut spikes = [(ceiling - 1, -1), (floor + 1, 1)].map(|(py, dy)| Spike {
            root: (x, py, z),
            dy,
            radius,
            blunt: self.blunt[usize::from(dy > 0)].sample(rng) as f64,
            scale: self.height.sample(rng) as f64,
        });
        let wind =
            if radius >= self.wind_radius && spikes.iter().all(|s| s.blunt >= self.wind_blunt) {
                let speed = self.speed.sample(rng);
                let angle = rng.next_float() * std::f32::consts::PI;
                Wind {
                    y,
                    max: MAX_RADIUS as i32 - radius,
                    x: mth_cos(angle as f64) * speed,
                    z: mth_sin(angle as f64) * speed,
                }
            } else {
                Wind {
                    y,
                    max: 0,
                    x: 0.0,
                    z: 0.0,
                }
            };
        let embedded = spikes
            .each_mut()
            .map(|spike| self.embed(region, spike, &wind));
        for (spike, embedded) in spikes.iter().zip(embedded) {
            if embedded {
                self.fill(region, rng, spike, &wind);
            }
        }
        true
    }

    fn open(&self, region: &Region, state: u16) -> bool {
        let block = region.tags.block_of(state);
        region.tags.is_air(state) || block == self.water || block == self.lava
    }

    fn embed(&self, region: &Region, spike: &mut Spike, wind: &Wind) -> bool {
        while spike.radius > 1 {
            let r = spike.radius as f32;
            let (x, mut y, z) = spike.root;
            for _ in 0..10.min(spike.height(0.0)) {
                if region.tags.is_block(read(region, (x, y, z)), self.lava) {
                    return false;
                }
                let (wx, wy, wz) = wind.at((x, y, z));
                if !self.open(region, read(region, (wx, wy, wz))) {
                    let mut angle = 0.0f32;
                    let mut embedded = true;
                    while angle < std::f32::consts::TAU {
                        let dx = (mth_cos(angle as f64) * r) as i32;
                        let dz = (mth_sin(angle as f64) * r) as i32;
                        if self.open(region, read(region, (wx + dx, wy, wz + dz))) {
                            embedded = false;
                            break;
                        }
                        angle += 6.0 / r;
                    }
                    if embedded {
                        spike.root.1 = y;
                        return true;
                    }
                }
                y -= spike.dy;
            }
            spike.radius /= 2;
        }
        false
    }

    fn fill(&self, region: &mut Region, rng: &mut WorldgenRng, spike: &Spike, wind: &Wind) {
        let radius = spike.radius;
        let mut heights = [-1; MAX_RADIUS * MAX_RADIUS + 1];
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                let squared = dx * dx + dz * dz;
                if squared > radius * radius {
                    continue;
                }
                let cached = &mut heights[squared as usize];
                if *cached < 0 {
                    *cached = spike.height((squared as f64).sqrt() as f32);
                }
                let mut height = *cached;
                if height == 0 {
                    continue;
                }
                if (rng.next_float() as f64) < 0.2 {
                    height = (height as f32 * (rng.next_float() * (1.0 - 0.8) + 0.8)) as i32;
                }
                let (x, mut y, z) = (spike.root.0 + dx, spike.root.1, spike.root.2 + dz);
                let max_y = if spike.dy > 0 {
                    region.height_of(Kind::WorldSurfaceWg, x, z)
                } else {
                    i32::MAX
                };
                let mut exited = false;
                for _ in 0..height {
                    if y >= max_y {
                        break;
                    }
                    let pos = wind.at((x, y, z));
                    let old = read(region, pos);
                    if self.open(region, old) {
                        exited = true;
                        region.write(pos, self.state, 2);
                    } else if exited && region.tags.has(old, self.stone) {
                        break;
                    }
                    y += spike.dy;
                }
            }
        }
    }
}

struct Spike {
    root: Pos,
    dy: i32,
    radius: i32,
    blunt: f64,
    scale: f64,
}

impl Spike {
    fn height(&self, distance: f32) -> i32 {
        let r = (distance as f64).max(self.blunt) / self.radius as f64 * 0.384;
        let h = self.scale
            * (0.75 * r.powf(1.3333333333333333)
                - r.powf(0.6666666666666666)
                - 0.3333333333333333 * r.ln());
        (h.max(0.0) / 0.384 * self.radius as f64) as i32
    }
}

struct Wind {
    y: i32,
    max: i32,
    x: f32,
    z: f32,
}

impl Wind {
    fn at(&self, (x, y, z): Pos) -> Pos {
        let dy = (self.y - y) as f64;
        let offset = |speed: f32| ((speed as f64 * dy).floor() as i32).clamp(-self.max, self.max);
        (x + offset(self.x), y, z + offset(self.z))
    }
}
