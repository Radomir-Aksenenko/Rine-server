//! Foliage placers (`levelgen/feature/foliageplacers/*`). Every RNG draw happens where the Java code makes
//! it, including the short-circuit ones (`a && random.nextInt(2) == 0` draws only when `a` holds).
//! `poplar` is not reached by the overworld biomes and is rejected at compile time.

use serde_json::Value;

use crate::feature::placement::{parse_int_provider, sample_int, IntProvider};

use super::order::Pos;
use super::trunk::{java_floor_f32, Axis};
use super::{dist_manhattan, offset, relative, TreeCompile, Ctx, Dir, TreeFeature, HORIZONTAL};

/// `FoliagePlacer.FoliageAttachment`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attachment {
    pub pos: Pos,
    pub radius_offset_xz: i32,
    pub foliage_height_offset: i32,
    pub size_x: i32,
    pub size_z: i32,
}

impl Attachment {
    /// `FoliageAttachment(pos, radiusOffsetXZ, doubleTrunk)`.
    pub fn new(pos: Pos, radius_offset_xz: i32, double_trunk: bool) -> Attachment {
        let s = if double_trunk { 2 } else { 1 };
        Attachment { pos, radius_offset_xz, foliage_height_offset: 0, size_x: s, size_z: s }
    }

    #[inline]
    pub fn double_trunk(&self) -> bool {
        self.size_x == 2 && self.size_z == 2
    }
}

#[derive(Debug, Clone)]
pub enum FoliageKind {
    Blob { height: i32 },
    Fancy { height: i32 },
    Bush { height: i32 },
    Spruce { trunk_height: IntProvider },
    Pine { height: IntProvider },
    Acacia,
    DarkOak,
    MegaPine { crown_height: IntProvider },
    MegaJungle { height: i32 },
    RandomSpread { foliage_height: IntProvider, attempts: i32 },
    Cherry {
        height: IntProvider,
        wide_bottom_layer_hole_chance: f32,
        corner_hole_chance: f32,
        hanging_leaves_chance: f32,
        hanging_leaves_extension_chance: f32,
    },
    /// `PoplarFoliagePlacer`: a rhombus crown with logs cut into its sides.
    Poplar { height: IntProvider, side_hole_chance: f32 },
}

#[derive(Debug, Clone)]
pub struct FoliagePlacer {
    pub radius: IntProvider,
    pub offset: IntProvider,
    pub kind: FoliageKind,
}

impl FoliagePlacer {
    pub fn parse(v: &Value, _c: &TreeCompile) -> Result<FoliagePlacer, String> {
        let ty = v["type"].as_str().ok_or("foliage placer type")?;
        let radius = parse_int_provider(&v["radius"])?;
        let offset = parse_int_provider(&v["offset"])?;
        let int = |k: &str| v[k].as_i64().map(|x| x as i32).ok_or_else(|| format!("{ty}: {k}"));
        let kind = match ty {
            "minecraft:blob_foliage_placer" => FoliageKind::Blob { height: int("height")? },
            "minecraft:fancy_foliage_placer" => FoliageKind::Fancy { height: int("height")? },
            "minecraft:bush_foliage_placer" => FoliageKind::Bush { height: int("height")? },
            "minecraft:spruce_foliage_placer" => FoliageKind::Spruce { trunk_height: parse_int_provider(&v["trunk_height"])? },
            "minecraft:pine_foliage_placer" => FoliageKind::Pine { height: parse_int_provider(&v["height"])? },
            "minecraft:acacia_foliage_placer" => FoliageKind::Acacia,
            "minecraft:dark_oak_foliage_placer" => FoliageKind::DarkOak,
            "minecraft:mega_pine_foliage_placer" => FoliageKind::MegaPine { crown_height: parse_int_provider(&v["crown_height"])? },
            "minecraft:jungle_foliage_placer" => FoliageKind::MegaJungle { height: int("height")? },
            "minecraft:random_spread_foliage_placer" => FoliageKind::RandomSpread {
                foliage_height: parse_int_provider(&v["foliage_height"])?,
                attempts: int("leaf_placement_attempts")?,
            },
            "minecraft:cherry_foliage_placer" => {
                let f = |k: &str| v[k].as_f64().map(|x| x as f32).ok_or_else(|| format!("cherry {k}"));
                FoliageKind::Cherry {
                    height: parse_int_provider(&v["height"])?,
                    wide_bottom_layer_hole_chance: f("wide_bottom_layer_hole_chance")?,
                    corner_hole_chance: f("corner_hole_chance")?,
                    hanging_leaves_chance: f("hanging_leaves_chance")?,
                    hanging_leaves_extension_chance: f("hanging_leaves_extension_chance")?,
                }
            }
            "minecraft:poplar_foliage_placer" => FoliageKind::Poplar {
                height: parse_int_provider(&v["height"])?,
                side_hole_chance: v["side_hole_chance"].as_f64().ok_or("side_hole_chance")? as f32,
            },
            other => return Err(format!("unsupported foliage placer {other}")),
        };
        Ok(FoliagePlacer { radius, offset, kind })
    }

    /// `foliageHeight(random, treeHeight, tree)`.
    pub fn foliage_height(&self, ctx: &mut Ctx, tree_height: i32) -> i32 {
        match &self.kind {
            FoliageKind::Blob { height } | FoliageKind::Fancy { height } | FoliageKind::Bush { height } | FoliageKind::MegaJungle { height } => *height,
            FoliageKind::Spruce { trunk_height } => (tree_height - sample_int(trunk_height, ctx.rng)).max(4),
            FoliageKind::Pine { height } | FoliageKind::Cherry { height, .. } | FoliageKind::Poplar { height, .. } => sample_int(height, ctx.rng),
            FoliageKind::Acacia => 0,
            FoliageKind::DarkOak => 4,
            FoliageKind::MegaPine { crown_height } => sample_int(crown_height, ctx.rng),
            FoliageKind::RandomSpread { foliage_height, .. } => sample_int(foliage_height, ctx.rng),
        }
    }

    /// `foliageRadius(random, trunkHeight)`: pine adds a random extra radius.
    pub fn foliage_radius(&self, ctx: &mut Ctx, trunk_height: i32) -> i32 {
        let base = sample_int(&self.radius, ctx.rng);
        match self.kind {
            FoliageKind::Pine { .. } => base + ctx.rng.next_int_bound((trunk_height + 1).max(1)),
            _ => base,
        }
    }

    /// `createFoliage(level, setter, random, tree, treeHeight, attachment, foliageHeight, leafRadius)`: draws
    /// the offset first, then runs the placer-specific code.
    pub fn create_foliage(&self, tree: &TreeFeature, ctx: &mut Ctx, tree_height: i32, a: &Attachment, foliage_height: i32, leaf_radius: i32) {
        let off = sample_int(&self.offset, ctx.rng);
        let _ = tree_height;
        let fhwo = foliage_height + a.foliage_height_offset;
        match &self.kind {
            FoliageKind::Blob { .. } => {
                for yo in (off - fhwo..=off).rev() {
                    let cr = (leaf_radius + a.radius_offset_xz - 1 - yo / 2).max(0);
                    place_leaves_row(tree, ctx, a.pos, cr, yo, a.double_trunk(), &self.kind);
                }
            }
            FoliageKind::Bush { .. } => {
                for yo in (off - fhwo..=off).rev() {
                    let cr = leaf_radius + a.radius_offset_xz - 1 - yo;
                    place_leaves_row(tree, ctx, a.pos, cr, yo, a.double_trunk(), &self.kind);
                }
            }
            FoliageKind::Fancy { .. } => {
                for yo in (off - foliage_height..=off).rev() {
                    let cr = leaf_radius + i32::from(yo != off && yo != off - foliage_height);
                    place_leaves_row(tree, ctx, a.pos, cr, yo, a.double_trunk(), &self.kind);
                }
            }
            FoliageKind::Spruce { .. } => {
                let mut cr = ctx.rng.next_int_bound(2);
                let mut max_r = 1;
                let mut min_r = 0;
                for yo in (-fhwo..=off).rev() {
                    place_leaves_row(tree, ctx, a.pos, cr, yo, a.double_trunk(), &self.kind);
                    if cr >= max_r {
                        cr = min_r;
                        min_r = 1;
                        max_r = (max_r + 1).min(leaf_radius + a.radius_offset_xz);
                    } else {
                        cr += 1;
                    }
                }
            }
            FoliageKind::Pine { .. } => {
                let mut cr = 0;
                for yo in (off - fhwo..=off).rev() {
                    place_leaves_row(tree, ctx, a.pos, cr, yo, a.double_trunk(), &self.kind);
                    if cr >= 1 && yo == off - fhwo + 1 {
                        cr -= 1;
                    } else if cr < leaf_radius + a.radius_offset_xz {
                        cr += 1;
                    }
                }
            }
            FoliageKind::Acacia => {
                let double = a.double_trunk();
                let fp = offset(a.pos, 0, off, 0);
                place_leaves_row(tree, ctx, fp, leaf_radius + a.radius_offset_xz, -1 - fhwo, double, &self.kind);
                place_leaves_row(tree, ctx, fp, leaf_radius - 1, -fhwo, double, &self.kind);
                place_leaves_row(tree, ctx, fp, leaf_radius + a.radius_offset_xz - 1, 0, double, &self.kind);
            }
            FoliageKind::DarkOak => {
                let double = a.double_trunk();
                let p = offset(a.pos, 0, off, 0);
                if double {
                    place_leaves_row(tree, ctx, p, leaf_radius + 2, -1, double, &self.kind);
                    place_leaves_row(tree, ctx, p, leaf_radius + 3, 0, double, &self.kind);
                    place_leaves_row(tree, ctx, p, leaf_radius + 2, 1, double, &self.kind);
                    if ctx.rng.next_bool() {
                        place_leaves_row(tree, ctx, p, leaf_radius, 2, double, &self.kind);
                    }
                } else {
                    place_leaves_row(tree, ctx, p, leaf_radius + 2, -1, double, &self.kind);
                    place_leaves_row(tree, ctx, p, leaf_radius + 1, 0, double, &self.kind);
                }
            }
            FoliageKind::MegaPine { .. } => {
                let fp = a.pos;
                let mut prev_r = 0;
                for yy in (fp.1 - fhwo + off)..=(fp.1 + off) {
                    let yo = fp.1 - yy;
                    let smooth = leaf_radius + a.radius_offset_xz + java_floor_f32(yo as f32 / fhwo as f32 * 3.5f32);
                    let jagged = if yo > 0 && smooth == prev_r && (yy & 1) == 0 { smooth + 1 } else { smooth };
                    place_leaves_row(tree, ctx, (fp.0, yy, fp.2), jagged, 0, a.double_trunk(), &self.kind);
                    prev_r = smooth;
                }
            }
            FoliageKind::MegaJungle { .. } => {
                let leaf_height = (if a.double_trunk() { foliage_height } else { 1 + ctx.rng.next_int_bound(2) }) + a.foliage_height_offset;
                for yo in (off - leaf_height..=off).rev() {
                    let cr = leaf_radius + a.radius_offset_xz + 1 - yo;
                    place_leaves_row(tree, ctx, a.pos, cr, yo, a.double_trunk(), &self.kind);
                }
            }
            FoliageKind::RandomSpread { attempts, .. } => {
                let origin = a.pos;
                for _ in 0..*attempts {
                    let dx = ctx.rng.next_int_bound(leaf_radius) - ctx.rng.next_int_bound(leaf_radius);
                    let dy = ctx.rng.next_int_bound(foliage_height) - ctx.rng.next_int_bound(foliage_height);
                    let dz = ctx.rng.next_int_bound(leaf_radius) - ctx.rng.next_int_bound(leaf_radius);
                    try_place_leaf(tree, ctx, offset(origin, dx, dy, dz));
                }
            }
            FoliageKind::Poplar { side_hole_chance, .. } => {
                let fp = offset(a.pos, 0, off, 0);
                let cr = leaf_radius + a.radius_offset_xz - 1;
                let flip = ctx.rng.next_bool();
                poplar_row(tree, ctx, fp, cr - 2, fhwo - 1, a.double_trunk(), fhwo, flip, *side_hole_chance);
                poplar_row(tree, ctx, fp, cr - 1, fhwo - 2, a.double_trunk(), fhwo, flip, *side_hole_chance);
                poplar_row(tree, ctx, fp, cr - 1, fhwo - 3, a.double_trunk(), fhwo, flip, *side_hole_chance);
                for y in (1..=fhwo - 4).rev() {
                    poplar_row(tree, ctx, fp, cr, y, a.double_trunk(), fhwo, flip, *side_hole_chance);
                }
                poplar_logs(tree, ctx, fp, cr, fhwo - 4, a.double_trunk(), fhwo, flip);
                poplar_row(tree, ctx, fp, cr - 1, 0, a.double_trunk(), fhwo, flip, *side_hole_chance);
                poplar_row(tree, ctx, fp, (cr - 2).clamp(1, 2), -1, a.double_trunk(), fhwo, flip, *side_hole_chance);
            }
            FoliageKind::Cherry { wide_bottom_layer_hole_chance: _, hanging_leaves_chance, hanging_leaves_extension_chance, .. } => {
                let double = a.double_trunk();
                let fp = offset(a.pos, 0, off, 0);
                let cr = leaf_radius + a.radius_offset_xz - 1;
                place_leaves_row(tree, ctx, fp, cr - 2, fhwo - 3, double, &self.kind);
                place_leaves_row(tree, ctx, fp, cr - 1, fhwo - 4, double, &self.kind);
                for y in (0..=fhwo - 5).rev() {
                    place_leaves_row(tree, ctx, fp, cr, y, double, &self.kind);
                }
                place_leaves_row_hanging(tree, ctx, fp, cr, -1, double, *hanging_leaves_chance, *hanging_leaves_extension_chance, &self.kind);
                place_leaves_row_hanging(tree, ctx, fp, cr - 1, -2, double, *hanging_leaves_chance, *hanging_leaves_extension_chance, &self.kind);
            }
        }
    }
}

/// `shouldSkipLocation` (the unsigned one) per placer. Draws random numbers where Java does.
fn should_skip_location(kind: &FoliageKind, ctx: &mut Ctx, dx: i32, y: i32, dz: i32, cr: i32, double: bool) -> bool {
    match kind {
        FoliageKind::Blob { .. } => dx == cr && dz == cr && (ctx.rng.next_int_bound(2) == 0 || y == 0),
        FoliageKind::Fancy { .. } => {
            let (fx, fz) = (dx as f32 + 0.5f32, dz as f32 + 0.5f32);
            fx * fx + fz * fz > (cr * cr) as f32
        }
        FoliageKind::Bush { .. } => dx == cr && dz == cr && ctx.rng.next_int_bound(2) == 0,
        FoliageKind::Spruce { .. } => dx == cr && dz == cr && cr > 0,
        FoliageKind::Pine { .. } => dx == cr && dz == cr && cr > 0,
        FoliageKind::Acacia => {
            if y == 0 {
                (dx > 1 || dz > 1) && dx != 0 && dz != 0
            } else {
                dx == cr && dz == cr && cr > 0
            }
        }
        FoliageKind::DarkOak => {
            if y == -1 && !double {
                dx == cr && dz == cr
            } else {
                y == 1 && dx + dz > cr * 2 - 2
            }
        }
        FoliageKind::MegaPine { .. } | FoliageKind::MegaJungle { .. } => dx + dz >= 7 || dx * dx + dz * dz > cr * cr,
        FoliageKind::RandomSpread { .. } => false,
        // Poplar rows use `poplar_row`, which has its own skip rule.
        FoliageKind::Poplar { .. } => false,
        FoliageKind::Cherry { wide_bottom_layer_hole_chance, corner_hole_chance, .. } => {
            if y == -1 && (dx == cr || dz == cr) && ctx.rng.next_float() < *wide_bottom_layer_hole_chance {
                return true;
            }
            let corner = dx == cr && dz == cr;
            let wide_layer = cr > 2;
            if wide_layer {
                corner || (dx + dz > cr * 2 - 2 && ctx.rng.next_float() < *corner_hole_chance)
            } else {
                corner && ctx.rng.next_float() < *corner_hole_chance
            }
        }
    }
}

/// `shouldSkipLocationSigned`: the doubled-trunk offset is folded into `min(|dx|, |dx - 1|)`. Dark oak
/// overrides it with the signed edge test first.
fn should_skip_signed(kind: &FoliageKind, ctx: &mut Ctx, dx: i32, y: i32, dz: i32, cr: i32, double: bool) -> bool {
    if let FoliageKind::DarkOak = kind {
        if y == 0 && double && (dx == -cr || dx >= cr) && (dz == -cr || dz >= cr) {
            return true;
        }
    }
    let (min_dx, min_dz) = if double {
        ((dx.abs()).min((dx - 1).abs()), (dz.abs()).min((dz - 1).abs()))
    } else {
        (dx.abs(), dz.abs())
    };
    should_skip_location(kind, ctx, min_dx, y, min_dz, cr, double)
}

/// `placeLeavesRow`.
fn place_leaves_row(tree: &TreeFeature, ctx: &mut Ctx, origin: Pos, cr: i32, y: i32, double: bool, kind: &FoliageKind) {
    let off = i32::from(double);
    for dx in -cr..=cr + off {
        for dz in -cr..=cr + off {
            if !should_skip_signed(kind, ctx, dx, y, dz, cr, double) {
                try_place_leaf(tree, ctx, offset(origin, dx, y, dz));
            }
        }
    }
}

/// `placeLeavesRowWithHangingLeavesBelow`.
#[allow(clippy::too_many_arguments)]
fn place_leaves_row_hanging(
    tree: &TreeFeature,
    ctx: &mut Ctx,
    origin: Pos,
    cr: i32,
    y: i32,
    double: bool,
    hanging_chance: f32,
    extension_chance: f32,
    kind: &FoliageKind,
) {
    place_leaves_row(tree, ctx, origin, cr, y, double, kind);
    let off = i32::from(double);
    let log_pos = offset(origin, 0, -1, 0);
    for along in HORIZONTAL {
        let to_edge = along.clockwise();
        let offset_to_edge = if to_edge.is_positive() { cr + off } else { cr };
        let mut pos = relative(offset(origin, 0, y - 1, 0), to_edge, offset_to_edge);
        pos = relative(pos, along, -cr);
        let mut offset_along = -cr;
        while offset_along < cr + off {
            pos = relative(pos, Dir::Up, 1);
            let leaves_above = ctx.foliage.contains(&pos);
            pos = relative(pos, Dir::Down, 1);
            if leaves_above && try_place_extension(tree, ctx, hanging_chance, log_pos, pos) {
                pos = relative(pos, Dir::Down, 1);
                try_place_extension(tree, ctx, extension_chance, log_pos, pos);
                pos = relative(pos, Dir::Up, 1);
            }
            offset_along += 1;
            pos = relative(pos, along, 1);
        }
    }
}

fn try_place_extension(tree: &TreeFeature, ctx: &mut Ctx, chance: f32, log_pos: Pos, pos: Pos) -> bool {
    dist_manhattan(pos, log_pos) < 7 && !(ctx.rng.next_float() > chance) && try_place_leaf(tree, ctx, pos)
}

/// `FoliagePlacer.tryPlaceLeaf`.
pub fn try_place_leaf(tree: &TreeFeature, ctx: &mut Ctx, pos: Pos) -> bool {
    let cur = ctx.id(pos);
    let persistent = ctx.env.states.get(cur, "persistent") == Some("true");
    if !persistent && ctx.valid_tree_pos(pos) {
        let mut state = ctx.prov(&tree.foliage_provider, pos);
        if ctx.env.states.get(state, "waterlogged").is_some() {
            let water = ctx.is_water_at(pos);
            state = ctx.env.states.with(state, "waterlogged", if water { "true" } else { "false" });
        }
        ctx.set_foliage(pos, state);
        true
    } else {
        false
    }
}

/// `PoplarFoliagePlacer.getCornerBlocksToCutForRhombusShape`.
fn poplar_corner_cut(dx: i32, dz: i32, cr: i32, partial: bool, flip: bool) -> i32 {
    let small_corner = if flip {
        (dx > 0 && dz > 0) || (dz < 0 && dx < 0)
    } else {
        (dx > 0 && dz < 0) || (dz > 0 && dx < 0)
    };
    if small_corner {
        cr - 1
    } else if partial {
        cr + 1
    } else {
        cr
    }
}

/// `PoplarFoliagePlacer.isWithinRhombusShape`.
fn poplar_within(cr: i32, abs_dx: i32, abs_dz: i32, cut: i32, extra: i32) -> bool {
    abs_dx + abs_dz <= cr * 2 - (cut + extra)
}

/// `PoplarFoliagePlacer.placeLeavesRow` with `shouldSkipLocation`: the edge test of a partial row draws
/// nothing, the side hole draws one float (`<= sideHoleChance`).
#[allow(clippy::too_many_arguments)]
fn poplar_row(tree: &TreeFeature, ctx: &mut Ctx, origin: Pos, cr: i32, y: i32, double: bool, fhwo: i32, flip: bool, side_hole: f32) {
    let off = i32::from(double);
    let partial = fhwo - 1 == y || fhwo - 2 == y;
    for dx in -cr..=cr + off {
        for dz in -cr..=cr + off {
            let (abs_dx, abs_dz) = (dx.abs(), dz.abs());
            let is_edge = abs_dx == cr || abs_dz == cr;
            let skip = if partial && is_edge {
                true
            } else {
                let cut = poplar_corner_cut(dx, dz, cr, partial, flip);
                let extra = i32::from(ctx.rng.next_float() <= side_hole);
                !poplar_within(cr, abs_dx, abs_dz, cut, extra)
            };
            if !skip {
                try_place_leaf(tree, ctx, offset(origin, dx, y, dz));
            }
        }
    }
}

/// `PoplarFoliagePlacer.replaceLeavesWithLog`: crown cells on the two axis lines at distance >= 4 that still
/// hold the leaves state become sideways logs. Compares against the raw foliage provider state.
#[allow(clippy::too_many_arguments)]
fn poplar_logs(tree: &TreeFeature, ctx: &mut Ctx, origin: Pos, cr: i32, y: i32, double: bool, fhwo: i32, flip: bool) {
    let off = i32::from(double);
    let partial = fhwo - 1 == y || fhwo - 2 == y;
    for dx in -cr..=cr + off {
        for dz in -cr..=cr + off {
            let (abs_dx, abs_dz) = (dx.abs(), dz.abs());
            let cut = poplar_corner_cut(dx, dz, cr, partial, flip);
            if poplar_within(cr, abs_dx, abs_dz, cut, 2) && ((abs_dz == 0 && cr - abs_dx >= 4) || (abs_dx == 0 && cr - abs_dz >= 4)) {
                let pos = offset(origin, dx, y, dz);
                let leaves = ctx.prov(&tree.foliage_provider, pos);
                if ctx.id(pos) == leaves {
                    let axis = if abs_dz == 0 { Axis::X } else { Axis::Z };
                    let log = ctx.prov(&tree.trunk_provider, pos);
                    let log = match ctx.env.states.get(log, "axis") {
                        Some(_) => ctx.env.states.with(log, "axis", axis.name()),
                        None => log,
                    };
                    ctx.set_foliage(pos, log);
                }
            }
        }
    }
}
