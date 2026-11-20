use super::heightmap::Kind;
use super::region::{Pos, Region};
use super::state::{Fluid, Palette};

// Biome.warmEnoughToRain, evaluated at the two different block positions.
const RAIN_TEMPERATURE: f32 = 0.15;

pub struct Freeze {
    water: u16,
    ice: u16,
    snow: u16,
    full_snow: u16,
    cannot_support: u16,
    support_override: u16,
}

impl Freeze {
    pub fn compile(tags: &[String], pal: &mut Palette) -> Result<Self, String> {
        let tag = |name: &str| {
            tags.binary_search_by(|t| t.as_str().cmp(name))
                .map(|i| i as u16)
                .map_err(|_| format!("tag {name} not bundled"))
        };
        let snow = pal.default_state("minecraft:snow");
        Ok(Self {
            water: pal.block("minecraft:water"),
            ice: pal.default_state("minecraft:ice"),
            snow,
            full_snow: pal.with_property(snow, "layers", "8").ok_or("snow layers property")?,
            cannot_support: tag("minecraft:cannot_support_snow_layer")?,
            support_override: tag("minecraft:support_override_snow_layer")?,
        })
    }

    pub fn place(&self, region: &mut Region, (origin_x, _, origin_z): Pos) -> bool {
        for x in origin_x..origin_x + 16 {
            for z in origin_z..origin_z + 16 {
                let y = region.height_of(Kind::MotionBlocking, x, z);
                let Some((world, biome)) = region.biome_at(x, y, z) else { continue };
                let freeze = world.biome_temperature(biome, x, y - 1, z) < RAIN_TEMPERATURE;
                let snow = world.biome_has_precipitation(biome) && world.biome_temperature(biome, x, y, z) < RAIN_TEMPERATURE;
                if freeze && !region.outside_build_height(y - 1) {
                    let below = region.get_block(x, y - 1, z);
                    if region.tags.fluid(below) == Fluid::Water && region.tags.is_block(below, self.water) {
                        region.write((x, y - 1, z), self.ice, 2);
                    }
                }
                if snow && !region.outside_build_height(y) {
                    let top = region.get_block(x, y, z);
                    if region.tags.is_air(top) || region.tags.is_block(top, region.tags.block_of(self.snow)) {
                        let below = region.get_block(x, y - 1, z);
                        if !region.tags.has(below, self.cannot_support)
                            && (region.tags.has(below, self.support_override)
                                || region.tags.collision_full_up(below)
                                || below == self.full_snow)
                        {
                            region.write((x, y, z), self.snow, 2);
                            let below = region.get_block(x, y - 1, z);
                            if let Some(snowy) = region.tags.snowy(below) {
                                region.write((x, y - 1, z), snowy, 2);
                            }
                        }
                    }
                }
            }
        }
        true
    }
}
