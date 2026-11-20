use serde_json::Value;

use super::predicate::BlockPredicate;
use super::region::{Pos, Region};
use super::rng::WorldgenRng;
use super::shape::DIRECTIONS;
use super::state::{Fluid, Palette};
use super::{block_set, Compiler};

pub(super) const HORIZONTAL: [usize; 4] = [2, 5, 3, 4];

pub(super) fn offset((x, y, z): Pos, dir: usize) -> Pos {
    let (dx, dy, dz) = DIRECTIONS[dir];
    (x + dx, y + dy, z + dz)
}

pub(super) fn read(region: &Region, (x, y, z): Pos) -> u16 {
    region.get_block(x, y, z)
}

pub(super) fn is_water(region: &Region, state: u16) -> bool {
    matches!(region.tags.fluid(state), Fluid::Water | Fluid::FlowingWater)
}

pub(super) fn pointed_states(pal: &mut Palette, pointed: &str) -> [[[u16; 2]; 5]; 2] {
    ["down", "up"].map(|dir| {
        ["base", "middle", "frustum", "tip", "tip_merge"].map(|thick| {
            ["false", "true"].map(|water| {
                pal.state_from_parts(
                    pointed,
                    &[
                        ("thickness", thick),
                        ("vertical_direction", dir),
                        ("waterlogged", water),
                    ],
                )
            })
        })
    })
}

pub(super) struct SpeleothemBlocks {
    pub base: u16,
    pub pointed: u16,
    pub water: u16,
    pub replaceable: BlockPredicate,
    base_state: u16,
    states: [[[u16; 2]; 5]; 2],
}

impl SpeleothemBlocks {
    pub fn compile(cfg: &Value, c: &mut Compiler) -> Result<Self, String> {
        let id = |k: &str| {
            cfg[k]
                .as_str()
                .or_else(|| cfg[k]["id"].as_str())
                .ok_or_else(|| format!("speleothem: {k}"))
        };
        let base = id("base_block")?;
        let pointed = id("pointed_block")?;
        if !matches!(
            pointed,
            "minecraft:pointed_dripstone" | "minecraft:sulfur_spike"
        ) {
            return Err(format!("unsupported pointed block {pointed}"));
        }
        let states = pointed_states(&mut c.pal, pointed);
        Ok(Self {
            base: c.pal.block(base),
            pointed: c.pal.block(pointed),
            water: c.pal.block("minecraft:water"),
            replaceable: block_set(&cfg["replaceable_blocks"], &c.tag_names, &mut c.pal)?,
            base_state: c.pal.default_state(base),
            states,
        })
    }

    pub fn is_base(&self, region: &Region, pos: Pos) -> bool {
        region.tags.is_block(read(region, pos), self.base)
            || self.replaceable.test(region, pos.0, pos.1, pos.2)
    }

    pub fn place_base(&self, region: &mut Region, pos: Pos) -> bool {
        if !self.replaceable.test(region, pos.0, pos.1, pos.2) {
            return false;
        }
        region.write(pos, self.base_state, 2);
        true
    }

    pub fn grow(&self, region: &mut Region, mut pos: Pos, dir: usize, height: i32, merge: bool) {
        if !self.is_base(region, offset(pos, dir ^ 1)) {
            return;
        }
        for i in 0..height {
            let thickness = match height - i {
                1 => {
                    if merge {
                        4
                    } else {
                        3
                    }
                }
                2 => 2,
                _ if i == 0 => 0,
                _ => 1,
            };
            let water = is_water(region, read(region, pos));
            region.write(pos, self.states[dir][thickness][usize::from(water)], 2);
            pos = offset(pos, dir);
        }
    }
}

pub(super) fn scan_column(
    region: &Region,
    (x, y, z): Pos,
    range: i32,
    water: u16,
    edge: impl Fn(&Region, Pos, u16) -> bool,
) -> Option<(Option<i32>, Option<i32>)> {
    let inside = |state| region.tags.is_air(state) || region.tags.is_block(state, water);
    if !inside(region.get_block(x, y, z)) {
        return None;
    }
    let scan = |dy| {
        let mut py = y;
        let mut state = region.get_block(x, py, z);
        for _ in 1..range {
            if !inside(state) {
                break;
            }
            py += dy;
            state = region.get_block(x, py, z);
        }
        edge(region, (x, py, z), state).then_some(py)
    };
    let ceiling = scan(1);
    let floor = scan(-1);
    Some((floor, ceiling))
}

pub(super) struct Speleothem {
    blocks: SpeleothemBlocks,
    taller: f32,
    spread: [f32; 3],
}

impl Speleothem {
    pub fn compile(cfg: &Value, c: &mut Compiler) -> Result<Self, String> {
        let chance = |k: &str, default| cfg[k].as_f64().unwrap_or(default) as f32;
        Ok(Self {
            blocks: SpeleothemBlocks::compile(cfg, c)?,
            taller: chance("chance_of_taller_generation", 0.2),
            spread: [
                chance("chance_of_directional_spread", 0.7),
                chance("chance_of_spread_radius2", 0.5),
                chance("chance_of_spread_radius3", 0.5),
            ],
        })
    }

    pub fn place(&self, region: &mut Region, rng: &mut WorldgenRng, pos: Pos) -> bool {
        let above = self.blocks.is_base(region, offset(pos, 1));
        let below = self.blocks.is_base(region, offset(pos, 0));
        let dir = match (above, below) {
            (true, true) => usize::from(!rng.next_bool()),
            (true, false) => 0,
            (false, true) => 1,
            (false, false) => return false,
        };
        let root = offset(pos, dir ^ 1);
        self.blocks.place_base(region, root);
        for d in HORIZONTAL {
            if rng.next_float() <= self.spread[0] {
                let mut p = offset(root, d);
                self.blocks.place_base(region, p);
                for &chance in &self.spread[1..] {
                    if rng.next_float() > chance {
                        break;
                    }
                    p = offset(p, rng.next_int_bound(6) as usize);
                    self.blocks.place_base(region, p);
                }
            }
        }
        let height = if rng.next_float() < self.taller {
            let target = read(region, offset(pos, dir));
            if region.tags.is_air(target) || region.tags.is_block(target, self.blocks.water) {
                2
            } else {
                1
            }
        } else {
            1
        };
        self.blocks.grow(region, pos, dir, height, false);
        true
    }
}
