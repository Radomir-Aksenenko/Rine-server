use serde_json::Value;

use super::heightmap::Kind;
use super::predicate::{self, BlockPredicate};
use super::region::{Pos, Region};
use super::rng::WorldgenRng;
use super::state::Fluid;
use super::state_provider::{self, StateProvider};
use super::{block_set, provider_value, Compiler, Decorator};

pub(super) struct RootSystem {
    tree: usize,
    headroom: i32,
    level_distance: i32,
    level_deviation: i32,
    radius: i32,
    replaceable: BlockPredicate,
    root: StateProvider,
    attempts: i32,
    column: i32,
    hanging_radius: i32,
    hanging_span: i32,
    hanging: u16,
    hanging_attempts: i32,
    water_headroom: i32,
    allowed: BlockPredicate,
}

impl RootSystem {
    pub fn compile(cfg: &Value, c: &mut Compiler) -> Result<Self, String> {
        let int = |k: &str| {
            cfg[k]
                .as_i64()
                .map(|n| n as i32)
                .ok_or_else(|| format!("root_system: {k}"))
        };
        let hanging = provider_value(c.bundle, &cfg["hanging_root_state_provider"]);
        if !matches!(
            hanging.as_str().or_else(|| hanging["id"].as_str()),
            Some("minecraft:hanging_roots" | "minecraft:sulfur")
        ) {
            return Err("root_system: unsupported hanging-root survival rule".into());
        }
        Ok(Self {
            tree: c.placed_ref(&cfg["feature"])?,
            headroom: int("required_vertical_space_for_tree")?,
            level_distance: int("level_test_distance")?,
            level_deviation: int("max_level_deviation")?,
            radius: int("root_radius")?,
            replaceable: block_set(&cfg["root_replaceable"], &c.tag_names, &mut c.pal)?,
            root: state_provider::parse(
                provider_value(c.bundle, &cfg["root_state_provider"]),
                &c.tag_names,
                &mut c.pal,
            )?,
            attempts: int("root_placement_attempts")?,
            column: int("root_column_max_height")?,
            hanging_radius: int("hanging_root_radius")?,
            hanging_span: int("hanging_roots_vertical_span")?,
            hanging: state_provider::state_of(hanging, &mut c.pal)?,
            hanging_attempts: int("hanging_root_placement_attempts")?,
            water_headroom: int("allowed_vertical_water_for_tree")?,
            allowed: predicate::parse(&cfg["allowed_tree_position"], &c.tag_names, &mut c.pal)?,
        })
    }

    fn space(&self, region: &Region, (x, y, z): Pos) -> bool {
        for dy in 1..=self.headroom {
            let id = region.get_block(x, y + dy, z);
            if !(region.tags.is_air(id)
                || dy < self.water_headroom
                    && matches!(region.tags.fluid(id), Fluid::Water | Fluid::FlowingWater))
            {
                return false;
            }
        }
        let d = self.level_distance;
        let h = self.level_deviation;
        d == 0
            || [(0, d), (-d, 0), (0, -d), (d, 0)]
                .into_iter()
                .all(|(dx, dz)| {
                    let below = region.get_block(x + dx, y - h, z + dz);
                    let above = region.get_block(x + dx, y + h, z + dz);
                    !region.tags.is_air(below) && region.tags.is_air(above)
                })
    }

    pub fn place(
        &self,
        decorator: &Decorator,
        region: &mut Region,
        rng: &mut WorldgenRng,
        (x, y, z): Pos,
    ) -> bool {
        if !region.tags.is_air(region.get_block(x, y, z)) {
            return false;
        }
        for dy in 1..=self.column {
            let tree_y = y + dy;
            if region.height_of(Kind::WorldSurface, x, z) < tree_y {
                break;
            }
            if !self.allowed.test(region, x, tree_y, z) || !self.space(region, (x, tree_y, z)) {
                continue;
            }
            let below = region.get_block(x, tree_y - 1, z);
            if matches!(region.tags.fluid(below), Fluid::Lava | Fluid::FlowingLava)
                || !region.tags.solid(below)
            {
                break;
            }
            if !decorator.run_placed(self.tree, region, rng, (x, tree_y, z), None) {
                continue;
            }
            for py in y..tree_y - 1 {
                for _ in 0..self.attempts {
                    let px = x + offset(rng, self.radius);
                    let pz = z + offset(rng, self.radius);
                    if self.replaceable.test(region, px, py, pz) {
                        let state = self.root.get(region, rng, px, py, pz);
                        region.write((px, py, pz), state, 2);
                    }
                }
            }
            for _ in 0..self.hanging_attempts {
                let px = x + offset(rng, self.hanging_radius);
                let py = y + offset(rng, self.hanging_span);
                let pz = z + offset(rng, self.hanging_radius);
                if region.is_air(px, py, pz)
                    && region.tags.sturdy(region.get_block(px, py + 1, pz), 0)
                {
                    region.write((px, py, pz), self.hanging, 2);
                }
            }
            break;
        }
        true
    }
}

fn offset(rng: &mut WorldgenRng, bound: i32) -> i32 {
    rng.next_int_bound(bound) - rng.next_int_bound(bound)
}
