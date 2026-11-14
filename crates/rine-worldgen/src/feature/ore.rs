//! `OreFeature` (vanilla 26.3, `levelgen/feature/OreFeature` + `AbstractOreFeature`), including the
//! ellipsoid vein shape, the discard-on-air-exposure roll and the rule tests of its targets.
//! Ore writes go straight to the section (`LevelChunkSection.setBlockState(..., false)`), so they do not
//! update heightmaps.

use std::sync::OnceLock;

use super::region::{Region, Tags};
use super::rng::WorldgenRng;

/// A `RuleTest` subset: the predicates the ore target lists use.
#[derive(Clone, Debug)]
pub enum RuleTest {
    /// `minecraft:tag_match`: the block belongs to the tag (index into `Tags`).
    TagMatch(u16),
    /// `minecraft:any_of`.
    AnyOf(Vec<RuleTest>),
    /// `minecraft:all_of`.
    AllOf(Vec<RuleTest>),
    /// `minecraft:not`.
    Not(Box<RuleTest>),
    /// `minecraft:height_match`: `min <= pos.y <= max`.
    HeightMatch { min: i32, max: i32 },
}

impl RuleTest {
    /// `RuleTest.test(state, pos, random)`. None of these tests draws random numbers.
    pub fn test(&self, blk: u16, y: i32, tags: &Tags) -> bool {
        match self {
            RuleTest::TagMatch(t) => tags.has(blk, *t),
            RuleTest::AnyOf(rules) => rules.iter().any(|r| r.test(blk, y, tags)),
            RuleTest::AllOf(rules) => rules.iter().all(|r| r.test(blk, y, tags)),
            RuleTest::Not(r) => !r.test(blk, y, tags),
            RuleTest::HeightMatch { min, max } => *min <= y && y <= *max,
        }
    }
}

/// `BlockReplacement`: a target predicate and the replacement state.
#[derive(Clone, Debug)]
pub struct Target {
    pub test: RuleTest,
    pub state: u16,
}

#[derive(Clone, Debug)]
pub struct OreFeature {
    pub targets: Vec<Target>,
    pub size: i32,
    pub discard_chance_on_air_exposure: f32,
}

/// `Mth.sin(float)`: a 65536-entry table indexed by `(int)(x * 10430.378...)`.
pub fn mth_sin(x: f64) -> f32 {
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        (0..65536)
            .map(|i| (i as f64 / 10430.378350470453).sin() as f32)
            .collect()
    });
    table[((x * 10430.378350470453) as i64 & 65535) as usize]
}

/// `Mth.ceil(float)`.
fn ceil_f(v: f32) -> i32 {
    v.ceil() as i32
}

/// `Mth.floor(double)`.
fn floor_d(v: f64) -> i32 {
    v.floor() as i32
}

impl OreFeature {
    /// `OreFeature.place`. Returns whether any block was placed.
    pub fn place(&self, region: &mut Region, rng: &mut WorldgenRng, origin: (i32, i32, i32)) -> bool {
        let (ox, oy, oz) = origin;
        let dir = rng.next_float() * std::f32::consts::PI;
        let spread_xy = self.size as f32 / 8.0;
        let max_radius = ceil_f((self.size as f32 / 16.0 * 2.0 + 1.0) / 2.0);
        let (sin_dir, cos_dir) = ((dir as f64).sin(), (dir as f64).cos());
        let x0 = ox as f64 + sin_dir * spread_xy as f64;
        let x1 = ox as f64 - sin_dir * spread_xy as f64;
        let z0 = oz as f64 + cos_dir * spread_xy as f64;
        let z1 = oz as f64 - cos_dir * spread_xy as f64;
        let y0 = oy as f64 + rng.next_int_bound(3) as f64 - 2.0;
        let y1 = oy as f64 + rng.next_int_bound(3) as f64 - 2.0;
        let x_start = ox - ceil_f(spread_xy) - max_radius;
        let y_start = oy - 2 - max_radius;
        let z_start = oz - ceil_f(spread_xy) - max_radius;
        let size_xz = 2 * (ceil_f(spread_xy) + max_radius);
        let size_y = 2 * (2 + max_radius);

        for xprobe in x_start..=x_start + size_xz {
            for zprobe in z_start..=z_start + size_xz {
                if y_start <= region.ocean_floor_wg(xprobe, zprobe) {
                    return self.do_place(region, rng, [x0, x1, z0, z1, y0, y1], [x_start, y_start, z_start], size_xz, size_y);
                }
            }
        }
        false
    }

    fn do_place(
        &self,
        region: &mut Region,
        rng: &mut WorldgenRng,
        ends: [f64; 6],
        start: [i32; 3],
        size_xz: i32,
        size_y: i32,
    ) -> bool {
        let [x0, x1, z0, z1, y0, y1] = ends;
        let [x_start, y_start, z_start] = start;
        let n = self.size as usize;
        let mut placed = 0;
        let mut tested = vec![false; (size_xz * size_y * size_xz) as usize];
        // data[i] = (x, y, z, r)
        let mut data = vec![[0f64; 4]; n];

        for (i, d) in data.iter_mut().enumerate() {
            let step = i as f32 / self.size as f32;
            let xx = x0 + step as f64 * (x1 - x0);
            let yy = y0 + step as f64 * (y1 - y0);
            let zz = z0 + step as f64 * (z1 - z0);
            let ss = rng.next_double() * self.size as f64 / 16.0;
            let sin_term = mth_sin((std::f32::consts::PI * step) as f64);
            let r = ((sin_term + 1.0f32) as f64 * ss + 1.0) / 2.0;
            *d = [xx, yy, zz, r];
        }

        for i1 in 0..n.saturating_sub(1) {
            if !(data[i1][3] <= 0.0) {
                for i2 in i1 + 1..n {
                    if !(data[i2][3] <= 0.0) {
                        let dx = data[i1][0] - data[i2][0];
                        let dy = data[i1][1] - data[i2][1];
                        let dz = data[i1][2] - data[i2][2];
                        let dr = data[i1][3] - data[i2][3];
                        if dr * dr > dx * dx + dy * dy + dz * dz {
                            if dr > 0.0 {
                                data[i2][3] = -1.0;
                            } else {
                                data[i1][3] = -1.0;
                            }
                        }
                    }
                }
            }
        }

        for d in &data {
            let r = d[3];
            if r < 0.0 {
                continue;
            }
            let (xx, yy, zz) = (d[0], d[1], d[2]);
            let x_min = floor_d(xx - r).max(x_start);
            let y_min = floor_d(yy - r).max(y_start);
            let z_min = floor_d(zz - r).max(z_start);
            let x_max = floor_d(xx + r).max(x_min);
            let y_max = floor_d(yy + r).max(y_min);
            let z_max = floor_d(zz + r).max(z_min);

            for x in x_min..=x_max {
                let xd = (x as f64 + 0.5 - xx) / r;
                if xd * xd >= 1.0 {
                    continue;
                }
                for y in y_min..=y_max {
                    let yd = (y as f64 + 0.5 - yy) / r;
                    if xd * xd + yd * yd >= 1.0 {
                        continue;
                    }
                    for z in z_min..=z_max {
                        let zd = (z as f64 + 0.5 - zz) / r;
                        if !(xd * xd + yd * yd + zd * zd < 1.0) || region.outside_build_height(y) {
                            continue;
                        }
                        let bit = ((x - x_start) + (y - y_start) * size_xz + (z - z_start) * size_xz * size_y) as usize;
                        if tested[bit] {
                            continue;
                        }
                        tested[bit] = true;
                        if !region.ensure_can_write(x, y, z) {
                            continue;
                        }
                        let state = region.get_block(x, y, z);
                        for target in &self.targets {
                            if self.can_place_ore(region, rng, state, target, x, y, z) {
                                region.set_block_in_section(x, y, z, target.state);
                                placed += 1;
                                break;
                            }
                        }
                    }
                }
            }
        }
        placed > 0
    }

    /// `AbstractOreFeature.canPlaceOre`: the target test first, then the air-exposure roll.
    fn can_place_ore(&self, region: &Region, rng: &mut WorldgenRng, state: u16, target: &Target, x: i32, y: i32, z: i32) -> bool {
        if !target.test.test(state, y, &region.tags) {
            return false;
        }
        let skip_air_check = self.should_skip_air_check(rng);
        skip_air_check || !is_adjacent_to_air(region, x, y, z)
    }

    /// `shouldSkipAirCheck`: draws a float only when the chance is strictly between 0 and 1.
    fn should_skip_air_check(&self, rng: &mut WorldgenRng) -> bool {
        let d = self.discard_chance_on_air_exposure;
        d <= 0.0 || (!(d >= 1.0) && rng.next_float() >= d)
    }
}

/// `AbstractOreFeature.isAdjacentToAir`: any of the six face neighbours is air.
fn is_adjacent_to_air(region: &Region, x: i32, y: i32, z: i32) -> bool {
    [(0, -1, 0), (0, 1, 0), (0, 0, -1), (0, 0, 1), (-1, 0, 0), (1, 0, 0)]
        .iter()
        .any(|&(dx, dy, dz)| region.is_air(x + dx, y + dy, z + dz))
}
