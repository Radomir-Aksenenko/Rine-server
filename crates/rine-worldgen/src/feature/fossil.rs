use serde_json::Value;

use super::heightmap::Kind;
use super::region::{Pos, Region};
use super::rng::WorldgenRng;
use super::structure_template::{
    processors, Bounds, Processor, Template, TemplatePlacer, UPDATE_NONE,
};
use super::Compiler;

pub(super) struct Fossil {
    pairs: Vec<(Template, Template)>,
    bone: Vec<Processor>,
    ore: Vec<Processor>,
    max_empty: usize,
    empty: [u16; 2],
    placer: TemplatePlacer,
}

impl Fossil {
    pub fn compile(v: &Value, c: &mut Compiler) -> Result<Self, String> {
        let bone = v["fossil_structures"].as_array().ok_or("missing fossils")?;
        let overlay = v["overlay_structures"]
            .as_array()
            .ok_or("missing fossil overlays")?;
        if bone.is_empty() || bone.len() != overlay.len() {
            return Err("invalid fossil template pairs".into());
        }
        let pairs = bone
            .iter()
            .zip(overlay)
            .map(|(a, b)| {
                Ok::<_, String>((
                    Template::compile(a.as_str().ok_or("missing fossil id")?, c)?,
                    Template::compile(b.as_str().ok_or("missing fossil overlay id")?, c)?,
                ))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            pairs,
            bone: processors(&v["fossil_processors"], c)?,
            ore: processors(&v["overlay_processors"], c)?,
            max_empty: v["max_empty_corners_allowed"]
                .as_u64()
                .ok_or("missing max_empty_corners_allowed")? as usize,
            empty: [
                c.pal.block("minecraft:water"),
                c.pal.block("minecraft:lava"),
            ],
            placer: TemplatePlacer::compile(c),
        })
    }

    pub fn place(&self, region: &mut Region, rng: &mut WorldgenRng, (x, y, z): Pos) -> bool {
        let rotation = rng.next_int_bound(4) as usize;
        let (bone, overlay) = &self.pairs[rng.next_int_bound(self.pairs.len() as i32) as usize];
        let [sx, sy, sz] = bone.size;
        let (width, depth) = if rotation % 2 == 0 {
            (sx, sz)
        } else {
            (sz, sx)
        };
        let (low_x, low_z) = (x - width / 2, z - depth / 2);
        let mut low_y = y;
        for px in low_x..low_x + width {
            for pz in low_z..low_z + depth {
                low_y = low_y.min(region.height_of(Kind::OceanFloorWg, px, pz));
            }
        }
        low_y = (low_y - 15 - rng.next_int_bound(10)).max(region.min_y() + 10);
        let max_x = low_x + width - 1;
        let max_z = low_z + depth - 1;
        let mut empty = 0;
        for px in [low_x, max_x] {
            for py in [low_y, low_y + sy - 1] {
                for pz in [low_z, max_z] {
                    let state = region.get_block(px, py, pz);
                    empty += usize::from(
                        region.tags.is_air(state)
                            || self.empty.contains(&region.tags.block_of(state)),
                    );
                }
            }
        }
        if empty > self.max_empty {
            return false;
        }
        let pos = match rotation {
            0 => (low_x, low_y, low_z),
            1 => (max_x, low_y, low_z),
            2 => (max_x, low_y, max_z),
            _ => (low_x, low_y, max_z),
        };
        let (chunk_x, chunk_z) = (x & !15, z & !15);
        let bounds = Some(Bounds {
            min: (chunk_x - 16, region.min_y(), chunk_z - 16),
            max: (
                chunk_x + 31,
                region.min_y() + region.height() - 1,
                chunk_z + 31,
            ),
        });
        self.placer.place(
            bone,
            &self.bone,
            region,
            rng,
            pos,
            rotation,
            bounds,
            UPDATE_NONE,
        );
        self.placer.place(
            overlay,
            &self.ore,
            region,
            rng,
            pos,
            rotation,
            bounds,
            UPDATE_NONE,
        );
        true
    }
}
