//! Overworld biomes: the parameter list (from the datagen report), per-biome temperature, the
//! `BiomeManager` zoom (fiddled) lookup, and `Biome.getTemperature` with its noises.

use crate::noise::SimplexNoise;
use crate::overworld::climate::{quantize, ParameterList, Space, Target};
use crate::overworld::sha256;
use crate::random::Legacy;

const BUNDLE: &str = include_str!("../../assets/worldgen/overworld_biomes.txt");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemperatureModifier {
    None,
    Frozen,
}

pub struct Biomes {
    pub names: Vec<String>,
    pub base_temperature: Vec<f32>,
    pub modifier: Vec<TemperatureModifier>,
    pub list: ParameterList,
    /// Quantized parameter points in list order, with biome index (for tests).
    pub points: Vec<(Space, u16)>,
    /// `BiomeManager.obfuscateSeed(seed)` for the level seed.
    zoom_seed: i64,
    temp_noise: Simplex2,
    frozen: Vec<(Simplex2, f64, f32)>,
    info_noise: Simplex2,
}

/// A 2D `SimplexNoise` (discarded offset) wrapped with the `NoiseStack` layer rule when needed.
struct Simplex2(SimplexNoise);

impl Simplex2 {
    fn get(&self, x: f64, y: f64) -> f32 {
        self.0.get_value_2d(x, y)
    }
}

/// `Biome.BIOME_INFO_NOISE.get(x, z)` (`Noise.get(double, double)`, a float widened by the caller). A static in
/// vanilla (`new SimplexNoise(new WorldgenRandom(new LegacyRandomSource(2345L)), true)`), independent of the seed.
pub fn biome_info_noise(x: f64, z: f64) -> f32 {
    static NOISE: std::sync::OnceLock<SimplexNoise> = std::sync::OnceLock::new();
    NOISE.get_or_init(|| simplex_discard(2345).1).get_value_2d(x, z)
}

/// `Biome.FROZEN_TEMPERATURE_NOISE`, `Biome.TEMPERATURE_NOISE`, `Biome.BIOME_INFO_NOISE` use
/// `new WorldgenRandom(new LegacyRandomSource(seed))`; `WorldgenRandom` forwards to the legacy source.
fn simplex_discard(seed: i64) -> (Legacy, SimplexNoise) {
    let mut rng = Legacy::new(seed);
    let noise = SimplexNoise::new_discard_offset(&mut rng);
    (rng, noise)
}

fn parse_f32(s: &str) -> f32 {
    s.parse::<f32>().expect("bundled float")
}

impl Biomes {
    pub fn overworld(seed: i64) -> Self {
        let mut names = Vec::new();
        let mut base_temperature = Vec::new();
        let mut modifier = Vec::new();
        let mut points: Vec<(Space, u16)> = Vec::new();
        for line in BUNDLE.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            match f.first() {
                Some(&"B") => {
                    names.push(f[1].to_string());
                    base_temperature.push(parse_f32(f[2]));
                    modifier.push(match f[3] {
                        "none" => TemperatureModifier::None,
                        "frozen" => TemperatureModifier::Frozen,
                        other => panic!("unknown temperature modifier {other}"),
                    });
                }
                Some(&"P") => {
                    let idx: u16 = f[1].parse().expect("biome index");
                    let mut space: Space = [(0, 0); 7];
                    for d in 0..6 {
                        let lo = parse_f32(f[2 + d * 2]);
                        let hi = parse_f32(f[3 + d * 2]);
                        // Climate.Parameter.span(min, max): quantizeCoord(min), quantizeCoord(max).
                        space[d] = (quantize(lo), quantize(hi));
                    }
                    let offset = quantize(parse_f32(f[14]));
                    space[6] = (offset, offset);
                    points.push((space, idx));
                }
                _ => {}
            }
        }
        let list = ParameterList::new(&points);
        let points_copy = points.clone();

        let zoom_seed = obfuscate_seed(seed);
        let (_, temp_noise) = simplex_discard(1234);
        let mut rng = Legacy::new(3456);
        let mut frozen = Vec::new();
        for (freq, amp) in [(1.0f64, 0.14285715f32), (0.5, 0.2857143), (0.25, 0.5714286)] {
            let noise = SimplexNoise::new_discard_offset(&mut rng);
            frozen.push((Simplex2(noise), freq, amp));
        }
        let (_, info_noise) = simplex_discard(2345);

        Biomes {
            names,
            base_temperature,
            modifier,
            list,
            points: points_copy,
            zoom_seed,
            temp_noise: Simplex2(temp_noise),
            frozen,
            info_noise: Simplex2(info_noise),
        }
    }

    /// `Climate.target(...)`: the quantized target of the six router samples.
    pub fn target(temperature: f32, humidity: f32, continentalness: f32, erosion: f32, depth: f32, weirdness: f32) -> Target {
        [
            quantize(temperature),
            quantize(humidity),
            quantize(continentalness),
            quantize(erosion),
            quantize(depth),
            quantize(weirdness),
            0,
        ]
    }

    /// `Climate.RTree` lookup of a quantized target: biome index.
    pub fn noise_biome(&self, target: &Target) -> u16 {
        self.list.find(target)
    }

    fn frozen_noise(&self, x: f64, z: f64) -> f32 {
        let mut value = 0.0f32;
        for (noise, freq, amp) in &self.frozen {
            value += *amp * noise.get(x * freq, z * freq);
        }
        value
    }

    /// `Biome.TemperatureModifier.FROZEN.modifyTemperature`.
    fn modify(&self, modifier: TemperatureModifier, x: i32, z: i32, base: f32) -> f32 {
        match modifier {
            TemperatureModifier::None => base,
            TemperatureModifier::Frozen => {
                let large = (self.frozen_noise(x as f64 * 0.05, z as f64 * 0.05) * 7.0f32) as f64;
                let edge = self.info_noise.get(x as f64 * 0.2, z as f64 * 0.2) as f64;
                let ice_patches = large + edge;
                if ice_patches < 0.3 {
                    let small = self.info_noise.get(x as f64 * 0.09, z as f64 * 0.09) as f64;
                    if small < 0.8 {
                        return 0.2;
                    }
                }
                base
            }
        }
    }

    /// `Biome.getTemperature(pos, seaLevel)` for biome index `biome`.
    pub fn temperature(&self, biome: u16, x: i32, y: i32, z: i32, sea_level: i32) -> f32 {
        let i = biome as usize;
        let adjusted = self.modify(self.modifier[i], x, z, self.base_temperature[i]);
        let snow_level = sea_level + 17;
        if y > snow_level {
            let v = self
                .temp_noise
                .get((x as f32 / 8.0f32) as f64, (z as f32 / 8.0f32) as f64)
                * 8.0f32;
            adjusted - (v + y as f32 - snow_level as f32) * 0.05f32 / 40.0f32
        } else {
            adjusted
        }
    }

    /// `Biome.warmEnoughToRain`.
    pub fn warm_enough_to_rain(&self, biome: u16, x: i32, y: i32, z: i32, sea_level: i32) -> bool {
        self.temperature(biome, x, y, z, sea_level) >= 0.15f32
    }

    /// `Biome.shouldMeltFrozenOceanIcebergSlightly`.
    pub fn melts_iceberg(&self, biome: u16, x: i32, y: i32, z: i32, sea_level: i32) -> bool {
        self.temperature(biome, x, y, z, sea_level) > 0.1f32
    }

    /// `BiomeManager.getBiome(x, y, z)`: picks the nearest of the 8 surrounding quart corners after
    /// the zoom fiddle. `lookup(qx, qy, qz)` is the stored (region) biome of a quart cell.
    pub fn fiddled<F: Fn(i32, i32, i32) -> u16>(&self, x: i32, y: i32, z: i32, lookup: F) -> u16 {
        let abs_x = x - 2;
        let abs_y = y - 2;
        let abs_z = z - 2;
        let parent_x = abs_x >> 2;
        let parent_y = abs_y >> 2;
        let parent_z = abs_z >> 2;
        let fract_x = (abs_x & 3) as f64 / 4.0;
        let fract_y = (abs_y & 3) as f64 / 4.0;
        let fract_z = (abs_z & 3) as f64 / 4.0;
        let mut min_i = 0usize;
        let mut min_distance = f64::INFINITY;
        for i in 0..8usize {
            let x_even = (i & 4) == 0;
            let y_even = (i & 2) == 0;
            let z_even = (i & 1) == 0;
            let corner_x = if x_even { parent_x } else { parent_x + 1 };
            let corner_y = if y_even { parent_y } else { parent_y + 1 };
            let corner_z = if z_even { parent_z } else { parent_z + 1 };
            let dx = if x_even { fract_x } else { fract_x - 1.0 };
            let dy = if y_even { fract_y } else { fract_y - 1.0 };
            let dz = if z_even { fract_z } else { fract_z - 1.0 };
            let next = self.fiddled_distance(corner_x, corner_y, corner_z, dx, dy, dz);
            if min_distance > next {
                min_i = i;
                min_distance = next;
            }
        }
        let bx = if (min_i & 4) == 0 { parent_x } else { parent_x + 1 };
        let by = if (min_i & 2) == 0 { parent_y } else { parent_y + 1 };
        let bz = if (min_i & 1) == 0 { parent_z } else { parent_z + 1 };
        lookup(bx, by, bz)
    }

    fn fiddled_distance(&self, xr: i32, yr: i32, zr: i32, dx: f64, dy: f64, dz: f64) -> f64 {
        let seed = self.zoom_seed;
        let mut r = seed;
        r = lcg(r, xr as i64);
        r = lcg(r, yr as i64);
        r = lcg(r, zr as i64);
        r = lcg(r, xr as i64);
        r = lcg(r, yr as i64);
        r = lcg(r, zr as i64);
        let fiddle_x = fiddle(r);
        r = lcg(r, seed);
        let fiddle_y = fiddle(r);
        r = lcg(r, seed);
        let fiddle_z = fiddle(r);
        square(dz + fiddle_z) + square(dy + fiddle_y) + square(dx + fiddle_x)
    }
}

fn square(v: f64) -> f64 {
    v * v
}

/// `LinearCongruentialGenerator.next(rval, c)`.
fn lcg(rval: i64, c: i64) -> i64 {
    let r = rval.wrapping_mul(
        rval.wrapping_mul(6364136223846793005i64)
            .wrapping_add(1442695040888963407i64),
    );
    r.wrapping_add(c)
}

fn fiddle(rval: i64) -> f64 {
    ((((rval >> 24) & 1023) - 512) as f64) * (0.9 / 1024.0)
}

/// `BiomeManager.obfuscateSeed`: `Hashing.sha256().hashLong(seed).asLong()`. `hashLong` writes the
/// little-endian bytes; `asLong` reads the first eight digest bytes little-endian.
pub fn obfuscate_seed(seed: i64) -> i64 {
    let digest = sha256::digest(&seed.to_le_bytes());
    i64::from_le_bytes([
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
    ])
}

#[cfg(test)]
mod tests {
    use super::obfuscate_seed;

    #[test]
    fn zoom_seed_matches_guava() {
        // Value computed with node:crypto (sha256 over the little-endian long, first 8 bytes LE).
        assert_eq!(obfuscate_seed(1592598566), -4372399098431425435);
    }
}
