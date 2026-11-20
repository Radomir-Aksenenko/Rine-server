use serde_json::Value;

use super::state::Fluid;
use super::{
    block_set,
    predicate::BlockPredicate,
    region::{Pos, Region},
    rng::WorldgenRng,
    shape::DIRECTIONS,
    Compiler,
};

pub(super) const FACE_NAMES: [&str; 6] = ["down", "up", "north", "south", "west", "east"];
pub(super) const HORIZONTAL: [usize; 4] = [2, 5, 3, 4];

pub(super) fn relative((x, y, z): Pos, face: usize) -> Pos {
    let (dx, dy, dz) = DIRECTIONS[face];
    (x + dx, y + dy, z + dz)
}

pub(super) fn shuffle<T>(values: &mut [T], rng: &mut WorldgenRng) {
    for count in (2..=values.len()).rev() {
        values.swap(count - 1, rng.next_int_bound(count as i32) as usize);
    }
}

pub(super) struct Multiface {
    block: u16,
    water: u16,
    variants: [u16; 128],
    masks: Box<[u8]>,
    directions: Vec<usize>,
    support: BlockPredicate,
    range: i32,
    chance: f32,
    sculk: Option<Sculk>,
}

struct Sculk {
    excluded: [u16; 3],
    fire: [u16; 2],
}

impl Multiface {
    pub fn compile(cfg: &Value, c: &mut Compiler) -> Result<Self, String> {
        let name = cfg["block"].as_str().ok_or("multiface block")?;
        if !matches!(name, "minecraft:glow_lichen" | "minecraft:sculk_vein") {
            return Err(format!("unsupported multiface block {name}"));
        }
        let block = c.pal.block(name);
        let water = c.pal.block("minecraft:water");
        let default = c.pal.default_state(name);
        let variants = std::array::from_fn(|mask| {
            FACE_NAMES
                .into_iter()
                .chain(["waterlogged"])
                .enumerate()
                .fold(default, |id, (i, name)| {
                    let value = if mask & (1 << i) != 0 {
                        "true"
                    } else {
                        "false"
                    };
                    c.pal.with_property(id, name, value).unwrap_or(id)
                })
        });
        let mut masks = vec![0; c.pal.states().len()];
        for (mask, &id) in variants.iter().enumerate() {
            masks[id as usize] = mask as u8;
        }
        // Terrain and other features can already have interned the same states with a different property order.
        let data = &rine_data::init().blocks;
        for (id, state) in c
            .pal
            .states()
            .iter()
            .enumerate()
            .filter(|(_, s)| s.starts_with(name))
        {
            if let Some(state) = data.parse_state(state) {
                masks[id] = FACE_NAMES
                    .into_iter()
                    .chain(["waterlogged"])
                    .enumerate()
                    .fold(0, |mask, (i, prop)| {
                        mask | (u8::from(data.get_prop(state, prop) == Some("true")) << i)
                    });
            }
        }
        let mut directions = Vec::with_capacity(6);
        if cfg["can_place_on_ceiling"].as_bool().unwrap_or(false) {
            directions.push(1);
        }
        if cfg["can_place_on_floor"].as_bool().unwrap_or(false) {
            directions.push(0);
        }
        if cfg["can_place_on_wall"].as_bool().unwrap_or(false) {
            directions.extend(HORIZONTAL);
        }
        Ok(Self {
            block,
            water,
            variants,
            masks: masks.into_boxed_slice(),
            directions,
            support: block_set(&cfg["can_be_placed_on"], &c.tag_names, &mut c.pal)?,
            range: cfg["search_range"].as_i64().unwrap_or(10) as i32,
            chance: cfg["chance_of_spreading"].as_f64().unwrap_or(0.5) as f32,
            sculk: (name == "minecraft:sculk_vein").then(|| Sculk {
                excluded: [
                    "minecraft:sculk",
                    "minecraft:sculk_catalyst",
                    "minecraft:moving_piston",
                ]
                .map(|n| c.pal.block(n)),
                fire: ["minecraft:fire", "minecraft:soul_fire"].map(|n| c.pal.block(n)),
            }),
        })
    }

    pub fn place(&self, level: &mut Region, rng: &mut WorldgenRng, pos: Pos) -> bool {
        let old = level.get_block(pos.0, pos.1, pos.2);
        if !level.tags.is_air(old) && !level.tags.is_block(old, self.water) {
            return false;
        }
        let mut directions = [0; 6];
        let options = &mut directions[..self.directions.len()];
        options.copy_from_slice(&self.directions);
        shuffle(options, rng);
        if self.try_place(level, rng, pos, old, options) {
            return true;
        }
        for &face in options.iter() {
            let mut choices = [0; 6];
            let mut count = 0;
            for &d in self.directions.iter().filter(|&&d| d != face ^ 1) {
                choices[count] = d;
                count += 1;
            }
            let choices = &mut choices[..count];
            shuffle(choices, rng);
            // Vanilla resets the cursor from the origin on every iteration.
            let p = relative(pos, face);
            let old = level.get_block(p.0, p.1, p.2);
            if self.range > 0
                && (level.tags.is_air(old)
                    || [self.water, self.block].contains(&level.tags.block_of(old)))
                && self.try_place(level, rng, p, old, choices)
            {
                return true;
            }
        }
        false
    }

    fn try_place(
        &self,
        level: &mut Region,
        rng: &mut WorldgenRng,
        pos: Pos,
        old: u16,
        options: &[usize],
    ) -> bool {
        for &face in options {
            let p = relative(pos, face);
            if !self.support.test(level, p.0, p.1, p.2) {
                continue;
            }
            let Some(mask) = self.place_mask(level, pos, old, face) else {
                return false;
            };
            level.write(pos, self.variants[mask as usize], 3);
            level.mark_post_process(pos);
            if rng.next_float() < self.chance {
                self.spread(level, rng, pos, face, mask);
            }
            return true;
        }
        false
    }

    fn place_mask(&self, level: &Region, pos: Pos, old: u16, face: usize) -> Option<u8> {
        let mask = if level.tags.is_block(old, self.block) {
            self.masks.get(old as usize).copied().unwrap_or_default()
        } else {
            u8::from(level.tags.fluid(old) == Fluid::Water) << 6
        };
        let p = relative(pos, face);
        if (mask & (1 << face)) != 0
            || !level
                .tags
                .face_full(level.get_block(p.0, p.1, p.2), face ^ 1)
        {
            return None;
        }
        Some(mask | (1 << face))
    }

    fn spread(&self, level: &mut Region, rng: &mut WorldgenRng, pos: Pos, from: usize, mask: u8) {
        let mut directions = [0, 1, 2, 3, 4, 5];
        shuffle(&mut directions, rng);
        for d in directions {
            if d / 2 == from / 2 || mask & (1 << d) != 0 {
                continue;
            }
            let adjacent = relative(pos, d);
            for (p, face, wrap) in [
                (pos, d, false),
                (adjacent, from, false),
                (relative(adjacent, from), d ^ 1, true),
            ] {
                let old = level.get_block(p.0, p.1, p.2);
                let block = level.tags.block_of(old);
                let fluid = level.tags.fluid(old);
                let default = level.tags.is_air(old)
                    || block == self.block
                    || block == self.water && fluid == Fluid::Water;
                let replace = if let Some(sculk) = &self.sculk {
                    let support = relative(p, face);
                    let against = level
                        .tags
                        .block_of(level.get_block(support.0, support.1, support.2));
                    !sculk.excluded.contains(&against)
                        && (!wrap
                            || !level
                                .tags
                                .sturdy(level.get_block(adjacent.0, adjacent.1, adjacent.2), face))
                        && matches!(fluid, Fluid::Empty | Fluid::Water)
                        && !sculk.fire.contains(&block)
                        && (level.tags.replaceable(old) || default)
                } else {
                    default
                };
                if replace {
                    if let Some(mask) = self.place_mask(level, p, old, face) {
                        level.mark_post_process(p);
                        if level.write(p, self.variants[mask as usize], 2) {
                            return;
                        }
                        break;
                    }
                }
            }
        }
    }
}
