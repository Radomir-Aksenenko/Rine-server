//! Root placers (`levelgen/feature/rootplacers/*`): mangrove only (the one the reached trees use).

use serde_json::Value;

use crate::feature::placement::{parse_int_provider, sample_int, IntProvider};

use super::order::Pos;
use crate::feature::state_provider::StateProvider as Provider;
use super::{dist_manhattan, offset, relative, TreeCompile, Ctx, Dir, TreeFeature, HORIZONTAL};

#[derive(Debug)]
pub struct RootPlacer {
    pub trunk_offset_y: IntProvider,
    pub root_provider: Provider,
    /// `above_root_placement`: `(provider, chance)`.
    pub above_root: Option<(Provider, f32)>,
    pub can_grow_through: u16,
    /// `muddy_roots_in`: block names replaced by the muddy roots provider.
    pub muddy_roots_in: Vec<String>,
    pub muddy_roots_provider: Provider,
    pub max_root_width: i32,
    pub max_root_length: i32,
    pub random_skew_chance: f32,
}

impl RootPlacer {
    pub fn parse(v: &Value, c: &mut TreeCompile) -> Result<RootPlacer, String> {
        let ty = v["type"].as_str().ok_or("root placer type")?;
        if ty != "minecraft:mangrove_root_placer" {
            return Err(format!("unsupported root placer {ty}"));
        }
        let m = &v["mangrove_root_placement"];
        let above_root = match v.get("above_root_placement") {
            Some(a) if a.is_object() => Some((
                c.provider(&a["above_root_provider"])?,
                a["above_root_placement_chance"].as_f64().ok_or("above chance")? as f32,
            )),
            _ => None,
        };
        let muddy_roots_in = m["muddy_roots_in"]
            .as_array()
            .ok_or("muddy_roots_in")?
            .iter()
            .map(|x| x.as_str().unwrap_or_default().to_string())
            .collect();
        Ok(RootPlacer {
            trunk_offset_y: parse_int_provider(&v["trunk_offset_y"])?,
            root_provider: c.provider(&v["root_provider"])?,
            above_root,
            can_grow_through: c.tag(m["can_grow_through"].as_str().ok_or("can_grow_through")?)?,
            muddy_roots_in,
            muddy_roots_provider: c.provider(&m["muddy_roots_provider"])?,
            max_root_width: m["max_root_width"].as_i64().ok_or("max_root_width")? as i32,
            max_root_length: m["max_root_length"].as_i64().ok_or("max_root_length")? as i32,
            random_skew_chance: m["random_skew_chance"].as_f64().ok_or("random_skew_chance")? as f32,
        })
    }

    /// `getTrunkOrigin`: `origin.above(trunkOffsetY.sample(random))`.
    pub fn trunk_origin(&self, origin: Pos, ctx: &mut Ctx) -> Pos {
        offset(origin, 0, sample_int(&self.trunk_offset_y, ctx.rng), 0)
    }

    /// `MangroveRootPlacer.canPlaceRoot`: `validTreePos` or a `can_grow_through` block.
    fn can_place_root(&self, ctx: &Ctx, p: Pos) -> bool {
        ctx.valid_tree_pos(p) || ctx.has_tag(p, self.can_grow_through)
    }

    /// `placeRoots`. Returns false when the roots cannot be laid out (the whole tree then fails).
    pub fn place_roots(&self, tree: &TreeFeature, ctx: &mut Ctx, origin: Pos, trunk_origin: Pos) -> bool {
        let mut root_positions: Vec<Pos> = Vec::new();
        let mut column = origin;
        while column.1 < trunk_origin.1 {
            if !self.can_place_root(ctx, column) {
                return false;
            }
            column = offset(column, 0, 1, 0);
        }
        root_positions.push(offset(trunk_origin, 0, -1, 0));
        for dir in HORIZONTAL {
            let pos = relative(trunk_origin, dir, 1);
            let mut in_dir = Vec::new();
            if !self.simulate_roots(ctx, pos, dir, trunk_origin, &mut in_dir, 0) {
                return false;
            }
            root_positions.extend(in_dir);
            root_positions.push(relative(trunk_origin, dir, 1));
        }
        for rp in root_positions {
            self.place_root(tree, ctx, rp);
        }
        true
    }

    fn simulate_roots(&self, ctx: &mut Ctx, root_pos: Pos, dir: Dir, root_origin: Pos, positions: &mut Vec<Pos>, layer: i32) -> bool {
        let max_len = self.max_root_length;
        if layer != max_len && positions.len() as i32 <= max_len {
            for pos in self.potential_root_positions(ctx, root_pos, dir, root_origin) {
                if self.can_place_root(ctx, pos) {
                    positions.push(pos);
                    if !self.simulate_roots(ctx, pos, dir, root_origin, positions, layer + 1) {
                        return false;
                    }
                }
            }
            true
        } else {
            false
        }
    }

    fn potential_root_positions(&self, ctx: &mut Ctx, pos: Pos, prev_dir: Dir, root_origin: Pos) -> Vec<Pos> {
        let below = offset(pos, 0, -1, 0);
        let next_to = relative(pos, prev_dir, 1);
        let width = dist_manhattan(pos, root_origin);
        let max_w = self.max_root_width;
        let skew = self.random_skew_chance;
        if width > max_w - 3 && width <= max_w {
            if ctx.rng.next_float() < skew {
                vec![below, offset(next_to, 0, -1, 0)]
            } else {
                vec![below]
            }
        } else if width > max_w {
            vec![below]
        } else if ctx.rng.next_float() < skew {
            vec![below]
        } else if ctx.rng.next_bool() {
            vec![next_to]
        } else {
            vec![below]
        }
    }

    /// `RootPlacer.getPotentiallyWaterloggedState`: `waterlogged` follows the water at `pos`.
    fn waterlogged(&self, ctx: &Ctx, pos: Pos, id: u16) -> u16 {
        if ctx.env.states.get(id, "waterlogged").is_some() {
            let w = ctx.is_water_at(pos);
            ctx.env.states.with(id, "waterlogged", if w { "true" } else { "false" })
        } else {
            id
        }
    }

    /// `MangroveRootPlacer.placeRoot` (muddy roots first, else the base placement with the above-root roll).
    fn place_root(&self, tree: &TreeFeature, ctx: &mut Ctx, pos: Pos) {
        let cur = ctx.id(pos);
        let base = ctx.env.states.base_name(cur);
        if self.muddy_roots_in.iter().any(|n| n == base) {
            let muddy = ctx.prov(&self.muddy_roots_provider, pos);
            let id = self.waterlogged(ctx, pos, muddy);
            ctx.set_root(pos, id);
            return;
        }
        // RootPlacer.placeRoot
        if self.can_place_root(ctx, pos) {
            let rp = ctx.prov(&self.root_provider, pos);
            let id = self.waterlogged(ctx, pos, rp);
            ctx.set_root(pos, id);
            if let Some((above_provider, chance)) = &self.above_root {
                let above = offset(pos, 0, 1, 0);
                if ctx.rng.next_float() < *chance && ctx.is_air(above) {
                    let s = ctx.prov(&above_provider, above);
                    let id = self.waterlogged(ctx, above, s);
                    ctx.set_root(above, id);
                }
            }
        }
        let _ = tree;
    }
}
