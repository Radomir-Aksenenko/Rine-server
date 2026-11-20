use super::heightmap::Kind;
use super::region::{Pos, Region};
use super::rng::WorldgenRng;
use super::state::Palette;

// Integer half-width of each row in BambooFeature's radius 1..=4 disk.
const PODZOL_ROWS: [&[i32]; 4] = [&[1, 0], &[2, 1, 0], &[3, 2, 2, 0], &[4, 3, 3, 2, 0]];

pub struct Bamboo {
    probability: f32,
    support: u16,
    podzol_replaceable: u16,
    podzol: u16,
    trunk: u16,
    final_large: u16,
    top_large: u16,
    top_small: u16,
}

impl Bamboo {
    pub fn compile(probability: f32, tags: &[String], pal: &mut Palette) -> Result<Self, String> {
        let tag = |name: &str| {
            tags.binary_search_by(|t| t.as_str().cmp(name))
                .map(|i| i as u16)
                .map_err(|_| format!("tag {name} not bundled"))
        };
        let mut state = |leaves, stage| {
            pal.state_from_parts("minecraft:bamboo", &[("age", "1"), ("leaves", leaves), ("stage", stage)])
        };
        Ok(Self {
            probability,
            support: tag("minecraft:supports_bamboo")?,
            podzol_replaceable: tag("minecraft:beneath_bamboo_podzol_replaceable")?,
            trunk: state("none", "0"),
            final_large: state("large", "1"),
            top_large: state("large", "0"),
            top_small: state("small", "0"),
            podzol: pal.default_state("minecraft:podzol"),
        })
    }

    pub fn place(&self, region: &mut Region, rng: &mut WorldgenRng, (x, y, z): Pos) -> bool {
        if !region.is_air(x, y, z) {
            return false;
        }
        if !region.tags.has(region.get_block(x, y - 1, z), self.support) {
            return true;
        }
        let height = rng.next_int_bound(12) + 5;
        if rng.next_float() < self.probability {
            let radius = rng.next_int_bound(4) + 1;
            for dx in -radius..=radius {
                let row = PODZOL_ROWS[(radius - 1) as usize][dx.unsigned_abs() as usize];
                for dz in -row..=row {
                    if region.ensure_can_write(x + dx, y, z + dz) {
                        let below = region.height_of(Kind::WorldSurface, x + dx, z + dz) - 1;
                        if region.tags.has(region.get_block(x + dx, below, z + dz), self.podzol_replaceable) {
                            region.write((x + dx, below, z + dz), self.podzol, 2);
                        }
                    }
                }
            }
        }
        let mut top = y;
        while top - y < height && region.is_air(x, top, z) {
            region.write((x, top, z), self.trunk, 2);
            top += 1;
        }
        if top - y >= 3 {
            region.write((x, top, z), self.final_large, 2);
            region.write((x, top - 1, z), self.top_large, 2);
            region.write((x, top - 2, z), self.top_small, 2);
        }
        true
    }
}
