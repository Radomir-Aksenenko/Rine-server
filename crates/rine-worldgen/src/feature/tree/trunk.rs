//! Trunk placers (`levelgen/feature/trunkplacers/*`): straight, forking, giant, mega_jungle, dark_oak,
//! fancy, upwards_branching, cherry and bending. `poplar` is not ported yet and is rejected at compile time.

use std::sync::OnceLock;

use serde_json::Value;

use crate::feature::placement::{parse_int_provider, sample_int, IntProvider};

use super::foliage::Attachment;
use super::order::Pos;
use super::{offset, relative, Ctx, TreeCompile, Dir, TreeFeature, HORIZONTAL};

/// `Mth.floor(float)` / `Mth.floor(double)` (`(int)v`, minus one when `v < (int)v`).
#[inline]
pub fn java_floor_f64(v: f64) -> i32 {
    let i = v as i32;
    if v < i as f64 {
        i - 1
    } else {
        i
    }
}

#[inline]
pub fn java_floor_f32(v: f32) -> i32 {
    let i = v as i32;
    if v < i as f32 {
        i - 1
    } else {
        i
    }
}

fn sin_table() -> &'static [f32] {
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    TABLE.get_or_init(|| (0..65536).map(|i| (i as f64 / 10430.378350470453).sin() as f32).collect())
}

/// `Mth.sin(double)`.
#[inline]
pub fn mth_sin(i: f64) -> f32 {
    sin_table()[((i * 10430.378350470453) as i64 & 65535) as usize]
}

/// `Mth.cos(double)`.
#[inline]
pub fn mth_cos(i: f64) -> f32 {
    sin_table()[((i * 10430.378350470453 + 16384.0) as i64 & 65535) as usize]
}

/// Axis value for `RotatedPillarBlock.AXIS` (`x`, `y`, `z`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub fn name(self) -> &'static str {
        match self {
            Axis::X => "x",
            Axis::Y => "y",
            Axis::Z => "z",
        }
    }

    fn of_dir(d: Dir) -> Axis {
        match d {
            Dir::North | Dir::South => Axis::Z,
            Dir::East | Dir::West => Axis::X,
            Dir::Up | Dir::Down => Axis::Y,
        }
    }
}

#[derive(Debug, Clone)]
pub enum TrunkKind {
    Straight,
    Forking,
    Giant,
    MegaJungle,
    DarkOak,
    Fancy,
    UpwardsBranching {
        extra_branch_steps: IntProvider,
        place_branch_per_log_probability: f32,
        extra_branch_length: IntProvider,
        can_grow_through: u16,
    },
    Cherry {
        /// `weighted_list` distribution: `(value, weight)` in list order.
        branch_count: Vec<(i32, i32)>,
        branch_horizontal_length: IntProvider,
        branch_start_offset_from_top: (i32, i32),
        branch_end_offset_from_top: IntProvider,
    },
    /// `BendingTrunkPlacer`: a horizontal bend after the vertical part, with leaves along both.
    Bending {
        min_height_for_leaves: i32,
        bend_length: IntProvider,
    },
    /// `PoplarTrunkPlacer`: one attachment at the top of the trunk, branches from a random horizontal set.
    Poplar {
        trunk_height_above_branches: IntProvider,
        branch_amount: IntProvider,
    },
}

pub struct TrunkPlacer {
    pub base_height: i32,
    pub height_rand_a: i32,
    pub height_rand_b: i32,
    pub kind: TrunkKind,
}

impl std::fmt::Debug for TrunkPlacer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TrunkPlacer({:?})", self.kind)
    }
}

impl TrunkPlacer {
    pub fn parse(v: &Value, c: &TreeCompile) -> Result<TrunkPlacer, String> {
        let ty = v["type"].as_str().ok_or("trunk placer type")?;
        let base_height = v["base_height"].as_i64().ok_or("base_height")? as i32;
        let height_rand_a = v["height_rand_a"].as_i64().ok_or("height_rand_a")? as i32;
        let height_rand_b = v["height_rand_b"].as_i64().ok_or("height_rand_b")? as i32;
        let kind = match ty {
            "minecraft:straight_trunk_placer" => TrunkKind::Straight,
            "minecraft:forking_trunk_placer" => TrunkKind::Forking,
            "minecraft:giant_trunk_placer" => TrunkKind::Giant,
            "minecraft:mega_jungle_trunk_placer" => TrunkKind::MegaJungle,
            "minecraft:dark_oak_trunk_placer" => TrunkKind::DarkOak,
            "minecraft:fancy_trunk_placer" => TrunkKind::Fancy,
            "minecraft:upwards_branching_trunk_placer" => TrunkKind::UpwardsBranching {
                extra_branch_steps: parse_int_provider(&v["extra_branch_steps"])?,
                place_branch_per_log_probability: v["place_branch_per_log_probability"].as_f64().ok_or("probability")? as f32,
                extra_branch_length: parse_int_provider(&v["extra_branch_length"])?,
                can_grow_through: c.tag(v["can_grow_through"].as_str().ok_or("can_grow_through")?)?,
            },
            "minecraft:cherry_trunk_placer" => {
                let s = &v["branch_start_offset_from_top"];
                TrunkKind::Cherry {
                    branch_count: parse_weighted_int(&v["branch_count"])?,
                    branch_horizontal_length: parse_int_provider(&v["branch_horizontal_length"])?,
                    branch_start_offset_from_top: (
                        s["min_inclusive"].as_i64().ok_or("cherry start min")? as i32,
                        s["max_inclusive"].as_i64().ok_or("cherry start max")? as i32,
                    ),
                    branch_end_offset_from_top: parse_int_provider(&v["branch_end_offset_from_top"])?,
                }
            }
            "minecraft:bending_trunk_placer" => TrunkKind::Bending {
                min_height_for_leaves: v["min_height_for_leaves"].as_i64().unwrap_or(1) as i32,
                bend_length: parse_int_provider(&v["bend_length"])?,
            },
            "minecraft:poplar_trunk_placer" => TrunkKind::Poplar {
                trunk_height_above_branches: parse_int_provider(&v["trunk_height_above_branches"])?,
                branch_amount: parse_int_provider(&v["branch_amount"])?,
            },
            other => return Err(format!("unsupported trunk placer {other}")),
        };
        Ok(TrunkPlacer { base_height, height_rand_a, height_rand_b, kind })
    }

    /// `getTreeHeight`.
    pub fn tree_height(&self, ctx: &mut Ctx) -> i32 {
        self.base_height + ctx.rng.next_int_bound(self.height_rand_a + 1) + ctx.rng.next_int_bound(self.height_rand_b + 1)
    }

    /// `validTreePos` of the placer (upwards branching also accepts its `can_grow_through` blocks).
    fn valid_tree_pos(&self, ctx: &Ctx, p: Pos) -> bool {
        if ctx.valid_tree_pos(p) {
            return true;
        }
        match self.kind {
            TrunkKind::UpwardsBranching { can_grow_through, .. } => ctx.has_tag(p, can_grow_through),
            _ => false,
        }
    }

    /// `TrunkPlacer.isFree`.
    pub fn is_free(&self, ctx: &Ctx, p: Pos) -> bool {
        self.valid_tree_pos(ctx, p) || ctx.has_tag(p, ctx.env.tags.logs)
    }

    fn place_below_trunk_block(&self, tree: &TreeFeature, ctx: &mut Ctx, p: Pos) {
        if let Some(id) = ctx.prov_opt(&tree.below_trunk, p) {
            ctx.set_trunk(p, id);
        }
    }

    /// `placeLog` with an optional `trySetValue(AXIS, axis)` modifier. Returns whether a log was written.
    fn place_log(&self, tree: &TreeFeature, ctx: &mut Ctx, p: Pos, axis: Option<Axis>) -> bool {
        if self.valid_tree_pos(ctx, p) {
            let mut id = ctx.prov(&tree.trunk_provider, p);
            if let Some(a) = axis {
                if ctx.env.states.get(id, "axis").is_some() {
                    id = ctx.env.states.with(id, "axis", a.name());
                }
            }
            ctx.set_trunk(p, id);
            true
        } else {
            false
        }
    }

    fn place_log_if_free(&self, tree: &TreeFeature, ctx: &mut Ctx, p: Pos) {
        if self.is_free(ctx, p) {
            self.place_log(tree, ctx, p, None);
        }
    }

    /// `placeTrunk`.
    pub fn place_trunk(&self, tree: &TreeFeature, ctx: &mut Ctx, tree_height: i32, origin: Pos) -> Vec<Attachment> {
        match &self.kind {
            TrunkKind::Straight => {
                self.place_below_trunk_block(tree, ctx, offset(origin, 0, -1, 0));
                for y in 0..tree_height {
                    self.place_log(tree, ctx, offset(origin, 0, y, 0), None);
                }
                vec![Attachment::new(offset(origin, 0, tree_height, 0), 0, false)]
            }
            TrunkKind::Forking => self.forking(tree, ctx, tree_height, origin),
            TrunkKind::Giant => self.giant(tree, ctx, tree_height, origin),
            TrunkKind::MegaJungle => self.mega_jungle(tree, ctx, tree_height, origin),
            TrunkKind::DarkOak => self.dark_oak(tree, ctx, tree_height, origin),
            TrunkKind::Fancy => self.fancy(tree, ctx, tree_height, origin),
            TrunkKind::UpwardsBranching { .. } => self.upwards(tree, ctx, tree_height, origin),
            TrunkKind::Cherry { .. } => self.cherry(tree, ctx, tree_height, origin),
            TrunkKind::Bending { min_height_for_leaves, bend_length } => {
                self.bending(tree, ctx, tree_height, origin, *min_height_for_leaves, bend_length)
            }
            TrunkKind::Poplar { trunk_height_above_branches, branch_amount } => {
                self.poplar(tree, ctx, tree_height, origin, trunk_height_above_branches, branch_amount)
            }
        }
    }

    /// `PoplarTrunkPlacer.placeTrunk`: the trunk goes up `treeHeight` logs. Each row draws a shuffle of the six
    /// directions (the horizontal ones keep their order); at the branch row, `branch_amount` sideways logs
    /// are set along the first directions. One attachment at the branch row.
    fn poplar(
        &self,
        tree: &TreeFeature,
        ctx: &mut Ctx,
        tree_height: i32,
        origin: Pos,
        trunk_height_above_branches: &IntProvider,
        branch_amount: &IntProvider,
    ) -> Vec<Attachment> {
        self.place_below_trunk_block(tree, ctx, offset(origin, 0, -1, 0));
        let branch_row = tree_height - sample_int(trunk_height_above_branches, ctx.rng);
        for y in 0..tree_height {
            let at = offset(origin, 0, y, 0);
            self.place_log(tree, ctx, at, None);
            // `Direction.allShuffled(random)` filtered to the horizontal ones, drawn on every row.
            let mut dirs = Dir::ALL;
            super::decorator::java_shuffle(&mut dirs, ctx);
            let horizontal: Vec<Dir> = dirs.into_iter().filter(|d| !matches!(d, Dir::Up | Dir::Down)).collect();
            if branch_row - 1 == y {
                let branches = sample_int(branch_amount, ctx.rng);
                for &dir in horizontal.iter().take(branches.max(0) as usize) {
                    let side = relative(at, dir, 1);
                    self.place_log(tree, ctx, side, Some(Axis::of_dir(dir)));
                }
            }
        }
        vec![Attachment::new(offset(origin, 0, branch_row, 0), 0, false)]
    }

    /// `BendingTrunkPlacer.placeTrunk`: the first `treeHeight` logs go up, the trunk steps one block sideways
    /// near its top, then `bend_length + 1` logs run along the bend direction. Attachments follow the trunk
    /// from `min_height_for_leaves` on and along the bend.
    fn bending(
        &self,
        tree: &TreeFeature,
        ctx: &mut Ctx,
        tree_height: i32,
        origin: Pos,
        min_height_for_leaves: i32,
        bend_length: &IntProvider,
    ) -> Vec<Attachment> {
        let direction = HORIZONTAL[ctx.rng.next_int_bound(4) as usize];
        let log_height = tree_height - 1;
        let mut pos = origin;
        self.place_below_trunk_block(tree, ctx, offset(pos, 0, -1, 0));
        let mut attachments = Vec::new();

        for i in 0..=log_height {
            // `i + 1 >= logHeight + random.nextInt(2)`: the draw happens on every iteration.
            if i + 1 >= log_height + ctx.rng.next_int_bound(2) {
                pos = relative(pos, direction, 1);
            }
            if self.valid_tree_pos(ctx, pos) {
                self.place_log(tree, ctx, pos, None);
            }
            if i >= min_height_for_leaves {
                attachments.push(Attachment::new(pos, 0, false));
            }
            pos = offset(pos, 0, 1, 0);
        }

        let dir_length = sample_int(bend_length, ctx.rng);
        for _ in 0..=dir_length {
            if self.valid_tree_pos(ctx, pos) {
                self.place_log(tree, ctx, pos, None);
            }
            attachments.push(Attachment::new(pos, 0, false));
            pos = relative(pos, direction, 1);
        }
        attachments
    }

    fn forking(&self, tree: &TreeFeature, ctx: &mut Ctx, tree_height: i32, origin: Pos) -> Vec<Attachment> {
        self.place_below_trunk_block(tree, ctx, offset(origin, 0, -1, 0));
        let mut attachments = Vec::new();
        let lean_dir = HORIZONTAL[ctx.rng.next_int_bound(4) as usize];
        let lean_height = tree_height - ctx.rng.next_int_bound(4) - 1;
        let mut lean_steps = 3 - ctx.rng.next_int_bound(3);
        let (mut tx, mut tz) = (origin.0, origin.2);
        let mut ey: Option<i32> = None;
        for yo in 0..tree_height {
            let yy = origin.1 + yo;
            if yo >= lean_height && lean_steps > 0 {
                let (sx, _, sz) = lean_dir.step();
                tx += sx;
                tz += sz;
                lean_steps -= 1;
            }
            if self.place_log(tree, ctx, (tx, yy, tz), None) {
                ey = Some(yy + 1);
            }
        }
        if let Some(ey) = ey {
            attachments.push(Attachment::new((tx, ey, tz), 1, false));
        }

        tx = origin.0;
        tz = origin.2;
        let branch_dir = HORIZONTAL[ctx.rng.next_int_bound(4) as usize];
        if branch_dir != lean_dir {
            let branch_pos = lean_height - ctx.rng.next_int_bound(2) - 1;
            let mut branch_steps = 1 + ctx.rng.next_int_bound(3);
            ey = None;
            let mut yo = branch_pos;
            while yo < tree_height && branch_steps > 0 {
                if yo >= 1 {
                    let yy = origin.1 + yo;
                    let (sx, _, sz) = branch_dir.step();
                    tx += sx;
                    tz += sz;
                    if self.place_log(tree, ctx, (tx, yy, tz), None) {
                        ey = Some(yy + 1);
                    }
                }
                yo += 1;
                branch_steps -= 1;
            }
            if let Some(ey) = ey {
                attachments.push(Attachment::new((tx, ey, tz), 0, false));
            }
        }
        attachments
    }

    fn giant(&self, tree: &TreeFeature, ctx: &mut Ctx, tree_height: i32, origin: Pos) -> Vec<Attachment> {
        let below = offset(origin, 0, -1, 0);
        self.place_below_trunk_block(tree, ctx, below);
        self.place_below_trunk_block(tree, ctx, offset(below, 1, 0, 0));
        self.place_below_trunk_block(tree, ctx, offset(below, 0, 0, 1));
        self.place_below_trunk_block(tree, ctx, offset(below, 1, 0, 1));
        for hh in 0..tree_height {
            self.place_log_if_free(tree, ctx, offset(origin, 0, hh, 0));
            if hh < tree_height - 1 {
                self.place_log_if_free(tree, ctx, offset(origin, 1, hh, 0));
                self.place_log_if_free(tree, ctx, offset(origin, 1, hh, 1));
                self.place_log_if_free(tree, ctx, offset(origin, 0, hh, 1));
            }
        }
        vec![Attachment::new(offset(origin, 0, tree_height, 0), 0, true)]
    }

    fn mega_jungle(&self, tree: &TreeFeature, ctx: &mut Ctx, tree_height: i32, origin: Pos) -> Vec<Attachment> {
        let mut attachments = self.giant(tree, ctx, tree_height, origin);
        let mut branch_height = tree_height - 2 - ctx.rng.next_int_bound(4);
        while branch_height > tree_height / 2 {
            let angle = ctx.rng.next_float() * (std::f64::consts::PI * 2.0) as f32;
            let (mut bx, mut bz) = (0i32, 0i32);
            for b in 0..5 {
                bx = (1.5f32 + mth_cos(angle as f64) * b as f32) as i32;
                bz = (1.5f32 + mth_sin(angle as f64) * b as f32) as i32;
                let pos = offset(origin, bx, branch_height - 3 + b / 2, bz);
                self.place_log(tree, ctx, pos, None);
            }
            attachments.push(Attachment::new(offset(origin, bx, branch_height, bz), -2, false));
            branch_height -= 2 + ctx.rng.next_int_bound(4);
        }
        attachments
    }

    fn dark_oak(&self, tree: &TreeFeature, ctx: &mut Ctx, tree_height: i32, origin: Pos) -> Vec<Attachment> {
        let mut attachments = Vec::new();
        let below = offset(origin, 0, -1, 0);
        self.place_below_trunk_block(tree, ctx, below);
        self.place_below_trunk_block(tree, ctx, offset(below, 1, 0, 0));
        self.place_below_trunk_block(tree, ctx, offset(below, 0, 0, 1));
        self.place_below_trunk_block(tree, ctx, offset(below, 1, 0, 1));
        let lean_dir = HORIZONTAL[ctx.rng.next_int_bound(4) as usize];
        let lean_height = tree_height - ctx.rng.next_int_bound(4);
        let mut lean_steps = 2 - ctx.rng.next_int_bound(3);
        let (x, y, z) = origin;
        let (mut tx, mut tz) = (x, z);
        let ey = y + tree_height - 1;
        for dy in 0..tree_height {
            if dy >= lean_height && lean_steps > 0 {
                let (sx, _, sz) = lean_dir.step();
                tx += sx;
                tz += sz;
                lean_steps -= 1;
            }
            let yy = y + dy;
            let p = (tx, yy, tz);
            if ctx.is_air_or_leaves(p) {
                self.place_log(tree, ctx, p, None);
                self.place_log(tree, ctx, offset(p, 1, 0, 0), None);
                self.place_log(tree, ctx, offset(p, 0, 0, 1), None);
                self.place_log(tree, ctx, offset(p, 1, 0, 1), None);
            }
        }
        attachments.push(Attachment::new((tx, ey, tz), 0, true));
        for ox in -1..=2 {
            for oz in -1..=2 {
                if (ox < 0 || ox > 1 || oz < 0 || oz > 1) && ctx.rng.next_int_bound(3) <= 0 {
                    let length = ctx.rng.next_int_bound(3) + 2;
                    for branch_y in 0..length {
                        self.place_log(tree, ctx, (x + ox, ey - branch_y - 1, z + oz), None);
                    }
                    attachments.push(Attachment::new((x + ox, ey, z + oz), 0, false));
                }
            }
        }
        attachments
    }

    fn fancy(&self, tree: &TreeFeature, ctx: &mut Ctx, tree_height: i32, origin: Pos) -> Vec<Attachment> {
        let height = tree_height + 2;
        let trunk_height = java_floor_f64(height as f64 * 0.618);
        self.place_below_trunk_block(tree, ctx, offset(origin, 0, -1, 0));
        let clusters_per_y = 1.min(java_floor_f64(1.382 + (1.0 * height as f64 / 13.0).powf(2.0)));
        let trunk_top = origin.1 + trunk_height;
        let mut relative_y = height - 5;
        let mut coords: Vec<FancyCoords> = vec![FancyCoords { attachment: Attachment::new(offset(origin, 0, relative_y, 0), 0, false), branch_base: trunk_top }];

        while relative_y >= 0 {
            let tree_shape = fancy_tree_shape(height, relative_y);
            if !(tree_shape < 0.0) {
                for _ in 0..clusters_per_y {
                    let radius = 1.0 * tree_shape as f64 * (ctx.rng.next_float() as f64 + 0.328);
                    let angle = (ctx.rng.next_float() * 2.0f32) as f64 * std::f64::consts::PI;
                    let x = radius * angle.sin() + 0.5;
                    let z = radius * angle.cos() + 0.5;
                    let check_start = offset(origin, java_floor_f64(x), relative_y - 1, java_floor_f64(z));
                    let check_end = offset(check_start, 0, 5, 0);
                    if self.make_limb(tree, ctx, check_start, check_end, false) {
                        let dx = origin.0 - check_start.0;
                        let dz = origin.2 - check_start.2;
                        let branch_height = check_start.1 as f64 - ((dx * dx + dz * dz) as f64).sqrt() * 0.381;
                        let branch_top = if branch_height > trunk_top as f64 { trunk_top } else { branch_height as i32 };
                        let check_branch_base = (origin.0, branch_top, origin.2);
                        if self.make_limb(tree, ctx, check_branch_base, check_start, false) {
                            coords.push(FancyCoords {
                                attachment: Attachment::new(check_start, 0, false),
                                branch_base: check_branch_base.1,
                            });
                        }
                    }
                }
            }
            relative_y -= 1;
        }

        self.make_limb(tree, ctx, origin, offset(origin, 0, trunk_height, 0), true);
        // makeBranches
        for c in &coords {
            let base = (origin.0, c.branch_base, origin.2);
            if base != c.attachment.pos && trim_branches(height, c.branch_base - origin.1) {
                self.make_limb(tree, ctx, base, c.attachment.pos, true);
            }
        }
        let mut attachments = Vec::new();
        for c in &coords {
            if trim_branches(height, c.branch_base - origin.1) {
                attachments.push(c.attachment);
            }
        }
        attachments
    }

    fn make_limb(&self, tree: &TreeFeature, ctx: &mut Ctx, start: Pos, end: Pos, do_place: bool) -> bool {
        if !do_place && start == end {
            return true;
        }
        let delta = (end.0 - start.0, end.1 - start.1, end.2 - start.2);
        let steps = delta.0.abs().max(delta.1.abs()).max(delta.2.abs());
        let dx = delta.0 as f32 / steps as f32;
        let dy = delta.1 as f32 / steps as f32;
        let dz = delta.2 as f32 / steps as f32;
        for i in 0..=steps {
            let p = offset(
                start,
                java_floor_f32(0.5f32 + i as f32 * dx),
                java_floor_f32(0.5f32 + i as f32 * dy),
                java_floor_f32(0.5f32 + i as f32 * dz),
            );
            if do_place {
                let axis = fancy_log_axis(start, p);
                self.place_log(tree, ctx, p, Some(axis));
            } else if !self.is_free(ctx, p) {
                return false;
            }
        }
        true
    }

    fn upwards(&self, tree: &TreeFeature, ctx: &mut Ctx, tree_height: i32, origin: Pos) -> Vec<Attachment> {
        let TrunkKind::UpwardsBranching { extra_branch_steps, place_branch_per_log_probability, extra_branch_length, .. } = &self.kind else {
            unreachable!()
        };
        let mut attachments = Vec::new();
        for height_pos in 0..tree_height {
            let current = origin.1 + height_pos;
            let log_pos = (origin.0, current, origin.2);
            if self.place_log(tree, ctx, log_pos, None)
                && height_pos < tree_height - 1
                && ctx.rng.next_float() < *place_branch_per_log_probability
            {
                let branch_dir = HORIZONTAL[ctx.rng.next_int_bound(4) as usize];
                let branch_len = sample_int(extra_branch_length, ctx.rng);
                let branch_pos = (branch_len - sample_int(extra_branch_length, ctx.rng) - 1).max(0);
                let branch_steps = sample_int(extra_branch_steps, ctx.rng);
                self.place_branch(tree, ctx, tree_height, &mut attachments, log_pos, current, branch_dir, branch_pos, branch_steps);
            }
            if height_pos == tree_height - 1 {
                attachments.push(Attachment::new((origin.0, current + 1, origin.2), 0, false));
            }
        }
        attachments
    }

    #[allow(clippy::too_many_arguments)]
    fn place_branch(
        &self,
        tree: &TreeFeature,
        ctx: &mut Ctx,
        tree_height: i32,
        attachments: &mut Vec<Attachment>,
        log_pos: Pos,
        current: i32,
        branch_dir: Dir,
        branch_pos: i32,
        mut branch_steps: i32,
    ) {
        let mut height_along = current + branch_pos;
        let (mut log_x, mut log_z) = (log_pos.0, log_pos.2);
        let mut idx = branch_pos;
        while idx < tree_height && branch_steps > 0 {
            if idx >= 1 {
                let placement_height = current + idx;
                let (sx, _, sz) = branch_dir.step();
                log_x += sx;
                log_z += sz;
                height_along = placement_height;
                if self.place_log(tree, ctx, (log_x, placement_height, log_z), None) {
                    height_along += 1;
                }
                attachments.push(Attachment::new((log_x, placement_height, log_z), 0, false));
            }
            idx += 1;
            branch_steps -= 1;
        }
        if height_along - current > 1 {
            let foliage_pos = (log_x, height_along, log_z);
            attachments.push(Attachment::new(foliage_pos, 0, false));
            attachments.push(Attachment::new(offset(foliage_pos, 0, -2, 0), 0, false));
        }
    }

    fn cherry(&self, tree: &TreeFeature, ctx: &mut Ctx, tree_height: i32, origin: Pos) -> Vec<Attachment> {
        let TrunkKind::Cherry { branch_count, branch_horizontal_length, branch_start_offset_from_top, branch_end_offset_from_top } = &self.kind else {
            unreachable!()
        };
        self.place_below_trunk_block(tree, ctx, offset(origin, 0, -1, 0));
        let (s_min, s_max) = *branch_start_offset_from_top;
        let first = (tree_height - 1 + uniform(ctx, s_min, s_max)).max(0);
        let mut second = (tree_height - 1 + uniform(ctx, s_min, s_max - 1)).max(0);
        if second >= first {
            second += 1;
        }
        let count = weighted_int(branch_count, ctx.rng);
        let has_middle = count == 3;
        let has_both = count >= 2;
        let trunk_height = if has_middle {
            tree_height
        } else if has_both {
            first.max(second) + 1
        } else {
            first + 1
        };
        for y in 0..trunk_height {
            self.place_log(tree, ctx, offset(origin, 0, y, 0), None);
        }
        let mut attachments = Vec::new();
        if has_middle {
            attachments.push(Attachment::new(offset(origin, 0, trunk_height, 0), 0, false));
        }
        let tree_dir = HORIZONTAL[ctx.rng.next_int_bound(4) as usize];
        let sideways = Axis::of_dir(tree_dir);
        attachments.push(self.generate_branch(tree, ctx, tree_height, origin, sideways, tree_dir, first, first < trunk_height - 1, branch_horizontal_length, branch_end_offset_from_top));
        if has_both {
            attachments.push(self.generate_branch(tree, ctx, tree_height, origin, sideways, tree_dir.opposite(), second, second < trunk_height - 1, branch_horizontal_length, branch_end_offset_from_top));
        }
        attachments
    }

    #[allow(clippy::too_many_arguments)]
    fn generate_branch(
        &self,
        tree: &TreeFeature,
        ctx: &mut Ctx,
        tree_height: i32,
        origin: Pos,
        sideways: Axis,
        branch_dir: Dir,
        offset_from_origin: i32,
        middle_continues_upwards: bool,
        horizontal_length: &IntProvider,
        end_offset: &IntProvider,
    ) -> Attachment {
        let mut log_pos = offset(origin, 0, offset_from_origin, 0);
        let branch_end_offset = tree_height - 1 + sample_int(end_offset, ctx.rng);
        let extend_away = middle_continues_upwards || branch_end_offset < offset_from_origin;
        let distance_to_trunk = sample_int(horizontal_length, ctx.rng) + i32::from(extend_away);
        let branch_end = offset(relative(origin, branch_dir, distance_to_trunk), 0, branch_end_offset, 0);
        let steps = if extend_away { 2 } else { 1 };
        for _ in 0..steps {
            log_pos = relative(log_pos, branch_dir, 1);
            self.place_log(tree, ctx, log_pos, Some(sideways));
        }
        let vertical = if branch_end.1 > log_pos.1 { Dir::Up } else { Dir::Down };
        loop {
            let distance = super::dist_manhattan(log_pos, branch_end);
            if distance == 0 {
                return Attachment::new(offset(branch_end, 0, 1, 0), 0, false);
            }
            let chance = ((branch_end.1 - log_pos.1).abs() as f32) / distance as f32;
            let grow_vertically = ctx.rng.next_float() < chance;
            log_pos = relative(log_pos, if grow_vertically { vertical } else { branch_dir }, 1);
            self.place_log(tree, ctx, log_pos, if grow_vertically { None } else { Some(sideways) });
        }
    }
}

/// `minecraft:weighted_list` int provider (`WeightedList.getRandomOrThrow`: `nextInt(totalWeight)`, then the entry
/// whose cumulative weight covers the selection).
fn parse_weighted_int(v: &Value) -> Result<Vec<(i32, i32)>, String> {
    if v["type"].as_str() != Some("minecraft:weighted_list") {
        let p = parse_int_provider(v)?;
        return Ok(vec![(sample_fixed(&p), 1)]);
    }
    v["distribution"]
        .as_array()
        .ok_or("weighted_list distribution")?
        .iter()
        .map(|e| Ok((e["data"].as_i64().ok_or("data")? as i32, e["weight"].as_i64().ok_or("weight")? as i32)))
        .collect()
}

/// Constant value of an int provider that is not random (only `constant` maps to a single entry here).
fn sample_fixed(p: &IntProvider) -> i32 {
    match *p {
        IntProvider::Constant(v) => v,
        _ => panic!("weighted_list entries must be constants"),
    }
}

fn weighted_int(entries: &[(i32, i32)], rng: &mut crate::feature::rng::WorldgenRng) -> i32 {
    let total: i32 = entries.iter().map(|e| e.1).sum();
    let mut sel = rng.next_int_bound(total);
    for &(v, w) in entries {
        if sel < w {
            return v;
        }
        sel -= w;
    }
    unreachable!("selection inside total weight")
}

/// `UniformInt.sample`.
fn uniform(ctx: &mut Ctx, min: i32, max: i32) -> i32 {
    ctx.rng.next_int_between(min, max)
}

#[derive(Clone, Copy)]
struct FancyCoords {
    attachment: Attachment,
    branch_base: i32,
}

/// `FancyTrunkPlacer.treeShape`.
fn fancy_tree_shape(height: i32, y: i32) -> f32 {
    if (y as f32) < height as f32 * 0.3f32 {
        return -1.0;
    }
    let radius = height as f32 / 2.0f32;
    let adjacent = radius - y as f32;
    let mut distance = (radius * radius - adjacent * adjacent).sqrt();
    if adjacent == 0.0 {
        distance = radius;
    } else if adjacent.abs() >= radius {
        return 0.0;
    }
    distance * 0.5f32
}

/// `FancyTrunkPlacer.trimBranches`.
fn trim_branches(height: i32, local_y: i32) -> bool {
    local_y as f64 >= height as f64 * 0.2
}

/// `FancyTrunkPlacer.getLogAxis`.
fn fancy_log_axis(start: Pos, p: Pos) -> Axis {
    let xdiff = (p.0 - start.0).abs();
    let zdiff = (p.2 - start.2).abs();
    let maxdiff = xdiff.max(zdiff);
    if maxdiff > 0 {
        if xdiff == maxdiff {
            Axis::X
        } else {
            Axis::Z
        }
    } else {
        Axis::Y
    }
}
