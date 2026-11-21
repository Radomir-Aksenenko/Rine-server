//! Tree decorators (`levelgen/feature/treedecorators/*`) and `TreeDecorator.Context`.
//!
//! The context lists are the position sets in Java `HashSet` order, stably sorted by Y (`ObjectArrayList`
//! sort), so every decorator's RNG draws follow the same order as vanilla.
//! Decorators write through [`Ctx::set_decoration`] for trees (the position joins the decorations set) and
//! through [`Ctx::set`] for fallen trees (`decorationSetter` is a plain `setBlock` there).

use serde_json::Value;


use super::order::Pos;
use crate::feature::state_provider::StateProvider as Provider;
use super::{offset, relative, TreeCompile, Ctx, Dir, HORIZONTAL};

/// Positions visible to decorators: `roots`, `logs`, `leaves` (sorted by Y, stable).
pub struct DecoratorCtx {
    pub roots: Vec<Pos>,
    pub logs: Vec<Pos>,
    pub leaves: Vec<Pos>,
    /// Tree: decorations are added to the decorations set. Fallen tree: plain `setBlock`.
    pub track_decorations: bool,
}

impl DecoratorCtx {
    pub fn new(roots: Vec<Pos>, logs: Vec<Pos>, leaves: Vec<Pos>, track_decorations: bool) -> Self {
        let sort = |mut v: Vec<Pos>| {
            v.sort_by_key(|p| p.1);
            v
        };
        DecoratorCtx { roots: sort(roots), logs: sort(logs), leaves: sort(leaves), track_decorations }
    }

    fn set(&self, ctx: &mut Ctx, p: Pos, id: u16) {
        if self.track_decorations {
            ctx.set_decoration(p, id);
        } else {
            ctx.set(p, id);
        }
    }

    /// `Context.placeVine`: `Blocks.VINE.defaultBlockState().setValue(direction, true)`.
    fn place_vine(&self, ctx: &mut Ctx, p: Pos, face: Dir) {
        let id = ctx.env.states.id("minecraft:vine", &[(face.name(), "true")]);
        self.set(ctx, p, id);
    }
}

#[derive(Debug)]
pub enum TreeDecorator {
    LeaveVine { probability: f32 },
    TrunkVine,
    Cocoa { probability: f32 },
    Beehive { probability: f32 },
    AttachedToLeaves {
        probability: f32,
        exclusion_radius_xz: i32,
        exclusion_radius_y: i32,
        provider: Provider,
        required_empty_blocks: i32,
        directions: Vec<Dir>,
    },
    AttachedToLogs { probability: f32, provider: Provider, directions: Vec<Dir> },
    PlaceOnGround { tries: i32, radius: i32, height: i32, provider: Provider },
    AlterGround { provider: Provider },
    PaleMoss { leaves_probability: f32, trunk_probability: f32, ground_probability: f32 },
    CreakingHeart { probability: f32 },
    ShelfMushroom { probability: f32 },
}

impl TreeDecorator {
    pub fn parse(v: &Value, c: &mut TreeCompile) -> Result<TreeDecorator, String> {
        let ty = v["type"].as_str().ok_or("decorator type")?;
        let f = |k: &str| v[k].as_f64().map(|x| x as f32).ok_or_else(|| format!("{ty}: {k}"));
        let dirs = |k: &str| -> Result<Vec<Dir>, String> {
            v[k].as_array()
                .ok_or_else(|| format!("{ty}: {k}"))?
                .iter()
                .map(|d| Dir::from_name(d.as_str().unwrap_or_default()).ok_or_else(|| format!("direction {d}")))
                .collect()
        };
        Ok(match ty {
            "minecraft:leave_vine" => TreeDecorator::LeaveVine { probability: f("probability")? },
            "minecraft:trunk_vine" => TreeDecorator::TrunkVine,
            "minecraft:cocoa" => TreeDecorator::Cocoa { probability: f("probability")? },
            "minecraft:beehive" => TreeDecorator::Beehive { probability: f("probability")? },
            "minecraft:attached_to_leaves" => TreeDecorator::AttachedToLeaves {
                probability: f("probability")?,
                exclusion_radius_xz: v["exclusion_radius_xz"].as_i64().ok_or("exclusion_radius_xz")? as i32,
                exclusion_radius_y: v["exclusion_radius_y"].as_i64().ok_or("exclusion_radius_y")? as i32,
                provider: c.provider(&v["block_provider"])?,
                required_empty_blocks: v["required_empty_blocks"].as_i64().ok_or("required_empty_blocks")? as i32,
                directions: dirs("directions")?,
            },
            "minecraft:attached_to_logs" => TreeDecorator::AttachedToLogs {
                probability: f("probability")?,
                provider: c.provider(&v["block_provider"])?,
                directions: dirs("directions")?,
            },
            "minecraft:place_on_ground" => TreeDecorator::PlaceOnGround {
                tries: v["tries"].as_i64().unwrap_or(128) as i32,
                radius: v["radius"].as_i64().unwrap_or(2) as i32,
                height: v["height"].as_i64().unwrap_or(1) as i32,
                provider: c.provider(&v["block_state_provider"])?,
            },
            "minecraft:alter_ground" => TreeDecorator::AlterGround { provider: c.provider(&v["provider"])? },
            "minecraft:pale_moss" => TreeDecorator::PaleMoss {
                leaves_probability: f("leaves_probability")?,
                trunk_probability: f("trunk_probability")?,
                ground_probability: f("ground_probability")?,
            },
            "minecraft:creaking_heart" => TreeDecorator::CreakingHeart { probability: f("probability")? },
            "minecraft:shelf_mushroom" => TreeDecorator::ShelfMushroom { probability: f("probability")? },
            other => return Err(format!("unsupported tree decorator {other}")),
        })
    }

    pub fn place(&self, d: &mut DecoratorCtx, ctx: &mut Ctx) {
        match self {
            TreeDecorator::LeaveVine { probability } => leave_vine(*probability, d, ctx),
            TreeDecorator::TrunkVine => trunk_vine(d, ctx),
            TreeDecorator::Cocoa { probability } => cocoa(*probability, d, ctx),
            TreeDecorator::Beehive { probability } => beehive(*probability, d, ctx),
            TreeDecorator::AttachedToLeaves { probability, exclusion_radius_xz, exclusion_radius_y, provider, required_empty_blocks, directions } => {
                attached_to_leaves(*probability, *exclusion_radius_xz, *exclusion_radius_y, provider, *required_empty_blocks, directions, d, ctx)
            }
            TreeDecorator::AttachedToLogs { probability, provider, directions } => attached_to_logs(*probability, provider, directions, d, ctx),
            TreeDecorator::PlaceOnGround { tries, radius, height, provider } => place_on_ground(*tries, *radius, *height, provider, d, ctx),
            TreeDecorator::AlterGround { provider } => alter_ground(provider, d, ctx),
            TreeDecorator::PaleMoss { leaves_probability, trunk_probability, ground_probability } => {
                pale_moss(*leaves_probability, *trunk_probability, *ground_probability, d, ctx)
            }
            TreeDecorator::CreakingHeart { probability } => creaking_heart(*probability, d, ctx),
            TreeDecorator::ShelfMushroom { probability } => shelf_mushroom(*probability, d, ctx),
        }
    }
}

/// `LeaveVineDecorator`.
fn leave_vine(p: f32, d: &mut DecoratorCtx, ctx: &mut Ctx) {
    let leaves = d.leaves.clone();
    for pos in leaves {
        if ctx.rng.next_float() < p {
            let w = offset(pos, -1, 0, 0);
            if ctx.is_air(w) {
                add_hanging_vine(w, Dir::East, d, ctx);
            }
        }
        if ctx.rng.next_float() < p {
            let e = offset(pos, 1, 0, 0);
            if ctx.is_air(e) {
                add_hanging_vine(e, Dir::West, d, ctx);
            }
        }
        if ctx.rng.next_float() < p {
            let n = offset(pos, 0, 0, -1);
            if ctx.is_air(n) {
                add_hanging_vine(n, Dir::South, d, ctx);
            }
        }
        if ctx.rng.next_float() < p {
            let s = offset(pos, 0, 0, 1);
            if ctx.is_air(s) {
                add_hanging_vine(s, Dir::North, d, ctx);
            }
        }
    }
}

fn add_hanging_vine(pos: Pos, face: Dir, d: &DecoratorCtx, ctx: &mut Ctx) {
    d.place_vine(ctx, pos, face);
    let mut max_dir = 4;
    let mut p = offset(pos, 0, -1, 0);
    while ctx.is_air(p) && max_dir > 0 {
        d.place_vine(ctx, p, face);
        p = offset(p, 0, -1, 0);
        max_dir -= 1;
    }
}

/// `TrunkVineDecorator`.
fn trunk_vine(d: &mut DecoratorCtx, ctx: &mut Ctx) {
    let logs = d.logs.clone();
    for pos in logs {
        if ctx.rng.next_int_bound(3) > 0 {
            let w = offset(pos, -1, 0, 0);
            if ctx.is_air(w) {
                d.place_vine(ctx, w, Dir::East);
            }
        }
        if ctx.rng.next_int_bound(3) > 0 {
            let e = offset(pos, 1, 0, 0);
            if ctx.is_air(e) {
                d.place_vine(ctx, e, Dir::West);
            }
        }
        if ctx.rng.next_int_bound(3) > 0 {
            let n = offset(pos, 0, 0, -1);
            if ctx.is_air(n) {
                d.place_vine(ctx, n, Dir::South);
            }
        }
        if ctx.rng.next_int_bound(3) > 0 {
            let s = offset(pos, 0, 0, 1);
            if ctx.is_air(s) {
                d.place_vine(ctx, s, Dir::North);
            }
        }
    }
}

/// `CocoaDecorator`.
fn cocoa(p: f32, d: &mut DecoratorCtx, ctx: &mut Ctx) {
    if ctx.rng.next_float() >= p {
        return;
    }
    if d.logs.is_empty() {
        return;
    }
    let tree_y = d.logs[0].1;
    let logs = d.logs.clone();
    for pos in logs.into_iter().filter(|p| p.1 - tree_y <= 2) {
        for direction in HORIZONTAL {
            if ctx.rng.next_float() <= 0.25f32 {
                let opposite = direction.opposite();
                let (ox, _, oz) = opposite.step();
                let cocoa_pos = offset(pos, ox, 0, oz);
                if ctx.is_air(cocoa_pos) {
                    let age = ctx.rng.next_int_bound(3).to_string();
                    let id = ctx.env.states.id("minecraft:cocoa", &[("age", age.as_str()), ("facing", direction.name())]);
                    d.set(ctx, cocoa_pos, id);
                }
            }
        }
    }
}

/// `Util.shuffle`: `for i = size down to 2: swap(i - 1, nextInt(i))`.
pub fn java_shuffle<T>(list: &mut [T], ctx: &mut Ctx) {
    let mut i = list.len();
    while i > 1 {
        let swap_to = ctx.rng.next_int_bound(i as i32) as usize;
        list.swap(i - 1, swap_to);
        i -= 1;
    }
}

/// `BeehiveDecorator` (the bee nest's bees draw their ticks; the block entity exists during decoration).
fn beehive(p: f32, d: &mut DecoratorCtx, ctx: &mut Ctx) {
    if d.logs.is_empty() {
        return;
    }
    if ctx.rng.next_float() >= p {
        return;
    }
    let hive_y = if !d.leaves.is_empty() {
        (d.leaves[0].1 - 1).max(d.logs[0].1 + 1)
    } else {
        (d.logs[0].1 + 1 + ctx.rng.next_int_bound(3)).min(d.logs[d.logs.len() - 1].1)
    };
    const SPAWN: [Dir; 3] = [Dir::East, Dir::South, Dir::West];
    let mut placements: Vec<Pos> = Vec::new();
    for &pos in d.logs.iter().filter(|p| p.1 == hive_y) {
        for dir in SPAWN {
            placements.push(relative(pos, dir, 1));
        }
    }
    if placements.is_empty() {
        return;
    }
    java_shuffle(&mut placements, ctx);
    let found = placements.iter().copied().find(|&p| ctx.is_air(p) && ctx.is_air(relative(p, Dir::South, 1)));
    if let Some(hive) = found {
        let id = ctx.env.states.id("minecraft:bee_nest", &[("facing", "south")]);
        d.set(ctx, hive, id);
        if ctx.region.get_block(hive.0, hive.1, hive.2) == id {
            let num_bees = 2 + ctx.rng.next_int_bound(2);
            let ticks_in_hive = (0..num_bees).map(|_| ctx.rng.next_int_bound(599)).collect();
            ctx.region.record_block_entity(hive, "minecraft:beehive",
                crate::feature::block_entity::BlockEntityData::Beehive { ticks_in_hive });
        }
    }
}

/// `AttachedToLeavesDecorator`.
#[allow(clippy::too_many_arguments)]
fn attached_to_leaves(
    p: f32,
    xz: i32,
    y: i32,
    provider: &Provider,
    required: i32,
    directions: &[Dir],
    d: &mut DecoratorCtx,
    ctx: &mut Ctx,
) {
    let mut blacklist = super::order::JavaSet::new();
    let mut leaves = d.leaves.clone();
    java_shuffle(&mut leaves, ctx);
    for leaf in leaves {
        let dir = directions[ctx.rng.next_int_bound(directions.len() as i32) as usize];
        let placement = relative(leaf, dir, 1);
        if !blacklist.contains(&placement) && ctx.rng.next_float() < p && has_required_empty(required, leaf, dir, ctx) {
            for dx in -xz..=xz {
                for dy in -y..=y {
                    for dz in -xz..=xz {
                        blacklist.insert(offset(placement, dx, dy, dz));
                    }
                }
            }
            let id = ctx.prov(&provider, placement);
            d.set(ctx, placement, id);
        }
    }
}

fn has_required_empty(required: i32, leaf: Pos, dir: Dir, ctx: &Ctx) -> bool {
    for i in 1..=required {
        if !ctx.is_air(relative(leaf, dir, i)) {
            return false;
        }
    }
    true
}

/// `AttachedToLogsDecorator`.
fn attached_to_logs(p: f32, provider: &Provider, directions: &[Dir], d: &mut DecoratorCtx, ctx: &mut Ctx) {
    let mut logs = d.logs.clone();
    java_shuffle(&mut logs, ctx);
    for pos in logs {
        let dir = directions[ctx.rng.next_int_bound(directions.len() as i32) as usize];
        let placement = relative(pos, dir, 1);
        if ctx.rng.next_float() <= p && ctx.is_air(placement) {
            let id = ctx.prov(&provider, placement);
            d.set(ctx, placement, id);
        }
    }
}

/// `Heightmap.Types.MOTION_BLOCKING_NO_LEAVES` first-available height at `(x, z)`, from the current blocks:
/// the highest `blocks_motion_no_leaves` block plus one, or the region bottom when there is none.
fn motion_blocking_no_leaves(ctx: &Ctx, x: i32, z: i32) -> i32 {
    let min_y = ctx.region.min_y();
    let top = min_y + ctx.region.height() - 1;
    let mut y = top;
    while y >= min_y {
        if ctx.has_tag((x, y, z), ctx.env.tags.blocks_motion_no_leaves) {
            return y + 1;
        }
        y -= 1;
    }
    min_y
}

/// `PlaceOnGroundDecorator`.
fn place_on_ground(tries: i32, radius: i32, height: i32, provider: &Provider, d: &mut DecoratorCtx, ctx: &mut Ctx) {
    let positions = super::lowest_trunk_or_root(&d.roots, &d.logs);
    if positions.is_empty() {
        return;
    }
    let origin = positions[0];
    let min_y = origin.1;
    let (mut min_x, mut max_x, mut min_z, mut max_z) = (origin.0, origin.0, origin.2, origin.2);
    for p in &positions {
        if p.1 == min_y {
            min_x = min_x.min(p.0);
            max_x = max_x.max(p.0);
            min_z = min_z.min(p.2);
            max_z = max_z.max(p.2);
        }
    }
    // BoundingBox(min..max).inflatedBy(radius, height, radius)
    let (bx0, bx1) = (min_x - radius, max_x + radius);
    let (by0, by1) = (min_y - height, min_y + height);
    let (bz0, bz1) = (min_z - radius, max_z + radius);
    for _ in 0..tries {
        let x = ctx.rng.next_int_between(bx0, bx1);
        let y = ctx.rng.next_int_between(by0, by1);
        let z = ctx.rng.next_int_between(bz0, bz1);
        let pos = (x, y, z);
        let above = offset(pos, 0, 1, 0);
        let above_id = ctx.id(above);
        let above_ok = ctx.is_air(above) || ctx.env.states.is_vine(above_id);
        if above_ok && solid_render(ctx, pos) && motion_blocking_no_leaves(ctx, pos.0, pos.2) <= above.1 {
            let id = ctx.prov(&provider, above);
            d.set(ctx, above, id);
        }
    }
}

/// `BlockStateBase.isSolidRender` at a position.
fn solid_render(ctx: &Ctx, p: Pos) -> bool {
    super::fallen::is_solid_render(ctx, p)
}

/// `AlterGroundDecorator`.
fn alter_ground(provider: &Provider, d: &mut DecoratorCtx, ctx: &mut Ctx) {
    let positions = super::lowest_trunk_or_root(&d.roots, &d.logs);
    if positions.is_empty() {
        return;
    }
    let min_y = positions[0].1;
    for pos in positions.into_iter().filter(|p| p.1 == min_y) {
        place_circle(provider, d, ctx, offset(pos, -1, 0, -1));
        place_circle(provider, d, ctx, offset(pos, 2, 0, -1));
        place_circle(provider, d, ctx, offset(pos, -1, 0, 2));
        place_circle(provider, d, ctx, offset(pos, 2, 0, 2));
        for _ in 0..5 {
            let placement = ctx.rng.next_int_bound(64);
            let xx = placement % 8;
            let zz = placement / 8;
            if xx == 0 || xx == 7 || zz == 0 || zz == 7 {
                place_circle(provider, d, ctx, offset(pos, -3 + xx, 0, -3 + zz));
            }
        }
    }
}

fn place_circle(provider: &Provider, d: &mut DecoratorCtx, ctx: &mut Ctx, pos: Pos) {
    for xx in -2i32..=2 {
        for zz in -2i32..=2 {
            if xx.abs() != 2 || zz.abs() != 2 {
                place_block_at(provider, d, ctx, offset(pos, xx, 0, zz));
            }
        }
    }
}

fn place_block_at(provider: &Provider, d: &mut DecoratorCtx, ctx: &mut Ctx, pos: Pos) {
    let mut dy = 2;
    while dy >= -3 {
        let cursor = offset(pos, 0, dy, 0);
        if let Some(id) = ctx.prov_opt(&provider, cursor) {
            d.set(ctx, cursor, id);
            break;
        }
        if !ctx.is_air(cursor) && dy < 0 {
            break;
        }
        dy -= 1;
    }
}

/// `PaleMossDecorator.place`.
fn pale_moss(leaves_p: f32, trunk_p: f32, ground_p: f32, d: &mut DecoratorCtx, ctx: &mut Ctx) {
    let mut logs = d.logs.clone();
    java_shuffle(&mut logs, ctx);
    // Collections.min by Y: first minimal element in list order.
    let Some(origin) = logs.iter().copied().reduce(|a, b| if b.1 < a.1 { b } else { a }) else { return };
    if ctx.rng.next_float() < ground_p {
        if let Some((decorator, patch)) = ctx.decorator.and_then(|d| Some((d, d.pale_moss_patch?))) {
            decorator.place_configured(patch, ctx.region, ctx.rng, offset(origin, 0, 1, 0));
        }
    }
    for pos in d.logs.clone() {
        if ctx.rng.next_float() < trunk_p {
            let down = offset(pos, 0, -1, 0);
            if ctx.is_air(down) {
                add_moss_hanger(down, d, ctx);
            }
        }
    }
    for pos in d.leaves.clone() {
        if ctx.rng.next_float() < leaves_p {
            let down = offset(pos, 0, -1, 0);
            if ctx.is_air(down) {
                add_moss_hanger(down, d, ctx);
            }
        }
    }
}

fn add_moss_hanger(mut pos: Pos, d: &DecoratorCtx, ctx: &mut Ctx) {
    while ctx.is_air(offset(pos, 0, -1, 0)) && !(ctx.rng.next_float() < 0.5) {
        let id = ctx.env.states.id("minecraft:pale_hanging_moss", &[("tip", "false")]);
        d.set(ctx, pos, id);
        pos = offset(pos, 0, -1, 0);
    }
    let id = ctx.env.states.id("minecraft:pale_hanging_moss", &[("tip", "true")]);
    d.set(ctx, pos, id);
}

/// `CreakingHeartDecorator`.
fn creaking_heart(p: f32, d: &mut DecoratorCtx, ctx: &mut Ctx) {
    if d.logs.is_empty() {
        return;
    }
    if ctx.rng.next_float() >= p {
        return;
    }
    let mut placements = d.logs.clone();
    java_shuffle(&mut placements, ctx);
    let logs_tag = ctx.env.tags.logs;
    let target = placements.iter().copied().find(|&pos| Dir::ALL.iter().all(|&dir| ctx.has_tag(relative(pos, dir, 1), logs_tag)));
    if let Some(t) = target {
        let id = ctx.env.states.id("minecraft:creaking_heart", &[("creaking_heart_state", "dormant"), ("natural", "true")]);
        d.set(ctx, t, id);
    }
}

/// `ShelfMushroomDecorator`: shelf mushrooms on the sides of a standing trunk (1 to 4 blocks above the first
/// log) or on a fallen log. Replaceable is approximated by air or `replaceable_by_trees` (vanilla reads
/// `canBeReplaced()`, which has no tag).
fn shelf_mushroom(p: f32, d: &mut DecoratorCtx, ctx: &mut Ctx) {
    if ctx.rng.next_float() >= p {
        return;
    }
    let logs = d.logs.clone();
    if logs.is_empty() {
        return;
    }
    let fallen = logs[0].1 == logs[logs.len() - 1].1;
    if fallen {
        shelf_on_fallen(&logs, d, ctx);
    } else {
        shelf_on_standing(&logs, d, ctx);
    }
}

/// `ShelfMushroomDecorator.placeOnStandingTree`.
fn shelf_on_standing(logs: &[Pos], d: &mut DecoratorCtx, ctx: &mut Ctx) {
    let first = HORIZONTAL[ctx.rng.next_int_bound(4) as usize];
    let directions = [first, first.clockwise()];
    let base_y = logs[0].1;
    for &log in logs {
        let dy = log.1 - base_y;
        if !(1..=4).contains(&dy) {
            continue;
        }
        for facing in directions {
            // `!(random.nextFloat() > 0.25F)`: the draw happens for every direction tried.
            let roll = !(ctx.rng.next_float() > 0.25);
            if roll && shelf_on_standing_try(log, facing, d, ctx) {
                break;
            }
        }
    }
}

fn shelf_on_standing_try(log: Pos, facing: Dir, d: &mut DecoratorCtx, ctx: &mut Ctx) -> bool {
    let pos = relative(log, facing, 1);
    if !shelf_replaceable(pos, ctx) {
        return false;
    }
    if shelf_at(offset(pos, 0, -1, 0), ctx) {
        return false;
    }
    shelf_place(pos, facing, d, ctx);
    true
}

/// `ShelfMushroomDecorator.placeOnFallenLog`: both perpendicular sides, no neighbouring shelf mushroom.
fn shelf_on_fallen(logs: &[Pos], d: &mut DecoratorCtx, ctx: &mut Ctx) {
    let (first, last) = (logs[0], logs[logs.len() - 1]);
    let directions = if first.0 != last.0 { [Dir::North, Dir::South] } else { [Dir::East, Dir::West] };
    for &log in logs {
        for facing in directions {
            if !(ctx.rng.next_float() > 0.25) {
                let pos = relative(log, facing, 1);
                if shelf_replaceable(pos, ctx) && !shelf_beside(pos, ctx) && !shelf_beside(log, ctx) {
                    shelf_place(pos, facing, d, ctx);
                }
            }
        }
    }
}

fn shelf_replaceable(pos: Pos, ctx: &Ctx) -> bool {
    (ctx.is_air(pos) || ctx.has_tag(pos, ctx.env.tags.replaceable_by_trees)) && !shelf_water_near(pos, ctx)
}

fn shelf_water_near(pos: Pos, ctx: &Ctx) -> bool {
    ctx.is_water_at(pos)
        || ctx.is_water_at(offset(pos, 1, 0, 0))
        || ctx.is_water_at(offset(pos, -1, 0, 0))
        || ctx.is_water_at(offset(pos, 0, 0, -1))
        || ctx.is_water_at(offset(pos, 0, 0, 1))
}

fn shelf_at(pos: Pos, ctx: &Ctx) -> bool {
    ctx.env.states.base_name(ctx.id(pos)) == "minecraft:shelf_mushroom"
}

fn shelf_beside(pos: Pos, ctx: &Ctx) -> bool {
    HORIZONTAL.iter().any(|&dir| shelf_at(relative(pos, dir, 1), ctx))
}

/// `placeMushroom`: `age = random.nextInt(2)`, `facing` the side the mushroom grows out of.
fn shelf_place(pos: Pos, facing: Dir, d: &mut DecoratorCtx, ctx: &mut Ctx) {
    let age = if ctx.rng.next_int_bound(2) == 0 { "0" } else { "1" };
    let id = ctx.env.states.id("minecraft:shelf_mushroom", &[("age", age), ("facing", facing.name())]);
    d.set(ctx, pos, id);
}
