//! Tree features (`TreeFeature`, `FallenTreeFeature`) ported from vanilla 26.3.
//!
//! Layout: `order` (java HashSet iteration order), `state` (interned block states), `provider`
//! (`BlockStateProvider` + predicates), `trunk`, `foliage`, `root`, `decorator`, `fallen`. This file holds the
//! config compile (`features.json` configured feature -> [`TreeFeature`]), `TreeFeature.place`, `doPlace`,
//! `getMaxFreeTreeHeight` and `updateLeaves`.
//!
//! Writes go through [`Ctx::set`], which is `WorldGenRegion.setBlock` for the decoration region: the
//! position is added to the feature's set before the write, and writes outside the writable 3x3 chunks are
//! dropped (still counted in the sets, as vanilla does).

pub mod decorator;
pub mod fallen;
pub mod foliage;
pub mod order;
pub mod root;
pub mod state;
pub mod trunk;

use serde_json::Value;

use crate::feature::region::{Region, Tags};
use crate::feature::rng::WorldgenRng;

use decorator::TreeDecorator;
use foliage::FoliagePlacer;
use order::{JavaSet, Pos};
use crate::feature::state::Palette;
use crate::feature::state_provider::{self, StateProvider as Provider};
use root::RootPlacer;
use state::TreeStates;
use trunk::TrunkPlacer;

/// `Direction`, in vanilla id order (`DOWN, UP, NORTH, SOUTH, WEST, EAST`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

/// `Direction.Plane.HORIZONTAL` order.
pub const HORIZONTAL: [Dir; 4] = [Dir::North, Dir::East, Dir::South, Dir::West];

impl Dir {
    pub const ALL: [Dir; 6] = [Dir::Down, Dir::Up, Dir::North, Dir::South, Dir::West, Dir::East];

    #[inline]
    pub fn step(self) -> (i32, i32, i32) {
        match self {
            Dir::Down => (0, -1, 0),
            Dir::Up => (0, 1, 0),
            Dir::North => (0, 0, -1),
            Dir::South => (0, 0, 1),
            Dir::West => (-1, 0, 0),
            Dir::East => (1, 0, 0),
        }
    }

    #[inline]
    pub fn opposite(self) -> Dir {
        match self {
            Dir::Down => Dir::Up,
            Dir::Up => Dir::Down,
            Dir::North => Dir::South,
            Dir::South => Dir::North,
            Dir::West => Dir::East,
            Dir::East => Dir::West,
        }
    }

    /// `Direction.getClockWise()` for horizontal directions.
    pub fn clockwise(self) -> Dir {
        match self {
            Dir::North => Dir::East,
            Dir::East => Dir::South,
            Dir::South => Dir::West,
            Dir::West => Dir::North,
            other => other,
        }
    }

    /// `AxisDirection.POSITIVE`.
    pub fn is_positive(self) -> bool {
        matches!(self, Dir::Up | Dir::South | Dir::East)
    }

    pub fn from_name(s: &str) -> Option<Dir> {
        Some(match s {
            "down" => Dir::Down,
            "up" => Dir::Up,
            "north" => Dir::North,
            "south" => Dir::South,
            "west" => Dir::West,
            "east" => Dir::East,
            _ => return None,
        })
    }

    /// `Direction.getName()`, used by property names (`axis=x`).
    pub fn name(self) -> &'static str {
        match self {
            Dir::Down => "down",
            Dir::Up => "up",
            Dir::North => "north",
            Dir::South => "south",
            Dir::West => "west",
            Dir::East => "east",
        }
    }
}

#[inline]
pub fn offset(p: Pos, dx: i32, dy: i32, dz: i32) -> Pos {
    (p.0 + dx, p.1 + dy, p.2 + dz)
}

#[inline]
pub fn relative(p: Pos, d: Dir, n: i32) -> Pos {
    let (x, y, z) = d.step();
    (p.0 + x * n, p.1 + y * n, p.2 + z * n)
}

#[inline]
pub fn dist_manhattan(a: Pos, b: Pos) -> i32 {
    (a.0 - b.0).abs() + (a.1 - b.1).abs() + (a.2 - b.2).abs()
}

/// Tag ids the tree code tests from Java `BlockTags`.
pub struct TreeTags {
    pub leaves: u16,
    pub logs: u16,
    pub replaceable_by_trees: u16,
    pub prevents_nearby_leaf_decay: u16,
    pub mangrove_roots_can_grow_through: u16,
    pub mangrove_logs_can_grow_through: u16,
    pub blocks_motion_no_leaves: u16,
}

impl TreeTags {
    pub fn new(tags: &Tags) -> Result<Self, String> {
        let get = |n: &str| tags.id(n).ok_or_else(|| format!("tag {n} not bundled"));
        Ok(TreeTags {
            leaves: get("minecraft:leaves")?,
            logs: get("minecraft:logs")?,
            replaceable_by_trees: get("minecraft:replaceable_by_trees")?,
            prevents_nearby_leaf_decay: get("minecraft:prevents_nearby_leaf_decay")?,
            mangrove_roots_can_grow_through: get("minecraft:mangrove_roots_can_grow_through")?,
            mangrove_logs_can_grow_through: get("minecraft:mangrove_logs_can_grow_through")?,
            blocks_motion_no_leaves: get("minecraft:blocks_motion_no_leaves")?,
        })
    }
}

/// Everything a tree needs from the decoration region and the palette.
pub struct TreeEnv {
    pub states: TreeStates,
    pub tags: TreeTags,
    /// Palette id of `minecraft:air`: what vanilla reads above and below the build height (void air).
    pub air: u16,
}

/// Mutable state of one tree placement: the region, the RNG and the position sets that
/// `TreeFeature.place` builds (roots, trunk logs, foliage, decorations).
pub struct Ctx<'a, 'r> {
    pub region: &'a mut Region<'r>,
    pub env: &'a TreeEnv,
    pub rng: &'a mut WorldgenRng,
    pub(crate) decorator: Option<&'a super::Decorator>,
    pub roots: JavaSet,
    pub trunks: JavaSet,
    pub foliage: JavaSet,
    pub decorations: JavaSet,
}

impl<'a, 'r> Ctx<'a, 'r> {
    pub fn new(region: &'a mut Region<'r>, env: &'a TreeEnv, rng: &'a mut WorldgenRng) -> Self {
        Ctx {
            region,
            env,
            rng,
            decorator: None,
            roots: JavaSet::new(),
            trunks: JavaSet::new(),
            foliage: JavaSet::new(),
            decorations: JavaSet::new(),
        }
    }

    /// Block at `p`. Outside the build height vanilla reads void air, so that is what is returned here.
    #[inline]
    pub fn id(&self, p: Pos) -> u16 {
        if self.region.outside_build_height(p.1) {
            return self.env.air;
        }
        self.region.get_block(p.0, p.1, p.2)
    }

    #[inline]
    pub fn is_air(&self, p: Pos) -> bool {
        self.region.tags.is_air(self.id(p))
    }

    #[inline]
    pub fn has_tag(&self, p: Pos, tag: u16) -> bool {
        self.region.tags.has(self.id(p), tag)
    }

    /// `TreeFeature.validTreePos`: air or `replaceable_by_trees`.
    #[inline]
    pub fn valid_tree_pos(&self, p: Pos) -> bool {
        let id = self.id(p);
        self.region.tags.is_air(id) || self.region.tags.has(id, self.env.tags.replaceable_by_trees)
    }

    /// `TreeFeature.isAirOrLeaves`.
    #[inline]
    pub fn is_air_or_leaves(&self, p: Pos) -> bool {
        let id = self.id(p);
        self.region.tags.is_air(id) || self.region.tags.has(id, self.env.tags.leaves)
    }

    /// `level.isFluidAtPosition(pos, s -> s.is(FluidTags.WATER))`.
    #[inline]
    pub fn is_water_at(&self, p: Pos) -> bool {
        self.env.states.is_water(self.id(p))
    }

    /// `WorldGenRegion.setBlock` on the decoration region (writes only inside the writable 3x3 chunks and the
    /// build height). The caller adds the position to its set first.
    #[inline]
    pub fn set(&mut self, p: Pos, id: u16) {
        if !self.region.ensure_can_write(p.0, p.1, p.2) || self.region.outside_build_height(p.1) {
            return;
        }
        self.region.set_block_in_section(p.0, p.1, p.2, id);
    }

    /// `getState` of a provider at `pos` (draws from the RNG as vanilla does).
    #[inline]
    pub fn prov(&mut self, p: &Provider, pos: Pos) -> u16 {
        p.get(&*self.region, self.rng, pos.0, pos.1, pos.2)
    }

    /// `getOptionalState`: `None` when no rule applies.
    #[inline]
    pub fn prov_opt(&mut self, p: &Provider, pos: Pos) -> Option<u16> {
        p.get_optional(&*self.region, self.rng, pos.0, pos.1, pos.2)
    }

    /// Tree setter: `rootPositions.add(pos)` then `setBlock`.
    #[inline]
    pub fn set_root(&mut self, p: Pos, id: u16) {
        self.roots.insert(p);
        self.set(p, id);
    }

    #[inline]
    pub fn set_trunk(&mut self, p: Pos, id: u16) {
        self.trunks.insert(p);
        self.set(p, id);
    }

    #[inline]
    pub fn set_foliage(&mut self, p: Pos, id: u16) {
        self.foliage.insert(p);
        self.set(p, id);
    }

    #[inline]
    pub fn set_decoration(&mut self, p: Pos, id: u16) {
        self.decorations.insert(p);
        self.set(p, id);
    }
}

/// `TreeFeature.getLowestTrunkOrRootOfTree`: logs, roots, or both, from the context lists.
pub fn lowest_trunk_or_root(roots: &[Pos], logs: &[Pos]) -> Vec<Pos> {
    if roots.is_empty() {
        logs.to_vec()
    } else if !logs.is_empty() && roots[0].1 == logs[0].1 {
        let mut v = logs.to_vec();
        v.extend_from_slice(roots);
        v
    } else {
        roots.to_vec()
    }
}

/// `FeatureSize` (`two_layers_feature_size`, `three_layers_feature_size`).
#[derive(Clone, Copy, Debug)]
pub enum FeatureSize {
    Two { limit: i32, lower: i32, upper: i32, min_clipped: Option<i32> },
    Three { limit: i32, upper_limit: i32, lower: i32, middle: i32, upper: i32, min_clipped: Option<i32> },
}

impl FeatureSize {
    pub fn size_at_height(&self, tree_height: i32, yo: i32) -> i32 {
        match *self {
            FeatureSize::Two { limit, lower, upper, .. } => {
                if yo < limit {
                    lower
                } else {
                    upper
                }
            }
            FeatureSize::Three { limit, upper_limit, lower, middle, upper, .. } => {
                if yo < limit {
                    lower
                } else if yo >= tree_height - upper_limit {
                    upper
                } else {
                    middle
                }
            }
        }
    }

    fn min_clipped(&self) -> Option<i32> {
        match *self {
            FeatureSize::Two { min_clipped, .. } | FeatureSize::Three { min_clipped, .. } => min_clipped,
        }
    }
}

/// A compiled `minecraft:tree` configured feature.
pub struct TreeFeature {
    pub trunk_provider: Provider,
    pub trunk: TrunkPlacer,
    pub foliage_provider: Provider,
    pub foliage: FoliagePlacer,
    pub root: Option<RootPlacer>,
    pub min_size: FeatureSize,
    pub decorators: Vec<TreeDecorator>,
    pub ignore_vines: bool,
    pub below_trunk: Provider,
}

/// Compile-time inputs: the palette, the tree block schemas, the bundled block state providers (by
/// `minecraft:` name) and the tag names. Providers and predicates are the shared `state_provider` and
/// `predicate` modules.
pub struct TreeCompile<'a> {
    pub states: &'a TreeStates,
    pub pal: &'a mut Palette,
    pub tag_names: &'a [String],
    pub providers: &'a Value,
}

impl TreeCompile<'_> {
    /// Tag id for a `minecraft:` or `#minecraft:` name.
    pub fn tag(&self, name: &str) -> Result<u16, String> {
        let n = name.strip_prefix('#').unwrap_or(name);
        self.tag_names
            .binary_search_by(|t| t.as_str().cmp(n))
            .map(|i| i as u16)
            .map_err(|_| format!("tag {n} not bundled"))
    }

    /// `BlockStateProvider`: a provider or block state object, or a bundled provider name
    /// (`minecraft:soil_beneath_tree`).
    pub fn provider(&mut self, v: &Value) -> Result<Provider, String> {
        let providers = self.providers;
        if let Some(json) = v.as_str().and_then(|name| providers.get(name)) {
            return state_provider::parse(json, self.tag_names, self.pal);
        }
        state_provider::parse(v, self.tag_names, self.pal)
    }

    pub fn trunk(&self, v: &Value) -> Result<TrunkPlacer, String> {
        trunk::TrunkPlacer::parse(v, self)
    }

    pub fn foliage(&self, v: &Value) -> Result<FoliagePlacer, String> {
        foliage::FoliagePlacer::parse(v, self)
    }
}

impl TreeFeature {
    /// Compiles a `minecraft:tree` configured feature (`features.json` `configured` entry).
    pub fn compile(cfg: &Value, c: &mut TreeCompile) -> Result<Self, String> {
        let root = match cfg.get("root_placer") {
            Some(r) => Some(RootPlacer::parse(r, c)?),
            None => None,
        };
        let decorators = cfg["decorators"]
            .as_array()
            .ok_or("decorators")?
            .iter()
            .map(|d| TreeDecorator::parse(d, c))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(TreeFeature {
            trunk_provider: c.provider(&cfg["trunk_provider"])?,
            trunk: c.trunk(&cfg["trunk_placer"])?,
            foliage_provider: c.provider(&cfg["foliage_provider"])?,
            foliage: c.foliage(&cfg["foliage_placer"])?,
            root,
            min_size: parse_feature_size(&cfg["minimum_size"])?,
            decorators,
            ignore_vines: cfg["ignore_vines"].as_bool().unwrap_or(false),
            below_trunk: c.provider(&cfg["below_trunk_provider"])?,
        })
    }

    /// `TreeFeature.place`. Returns whether the tree was placed.
    pub fn place(&self, ctx: &mut Ctx<'_, '_>, origin: Pos) -> bool {
        let result = self.do_place(ctx, origin);
        if result && (!ctx.trunks.is_empty() || !ctx.foliage.is_empty()) {
            if !self.decorators.is_empty() {
                let roots = ctx.roots.to_vec();
                let logs = ctx.trunks.to_vec();
                let leaves = ctx.foliage.to_vec();
                let mut dec = decorator::DecoratorCtx::new(roots, logs, leaves, true);
                for d in &self.decorators {
                    d.place(&mut dec, ctx);
                }
            }
            let all = ctx
                .roots
                .to_vec()
                .into_iter()
                .chain(ctx.trunks.to_vec())
                .chain(ctx.foliage.to_vec())
                .chain(ctx.decorations.to_vec());
            let Some(bounds) = Bounds::encapsulating(all) else {
                return false;
            };
            let mut fill = ctx.decorations.to_vec();
            fill.extend(ctx.roots.to_vec());
            let logs = ctx.trunks.to_vec();
            update_leaves(ctx, bounds, &fill, &logs);
            return true;
        }
        false
    }

    /// `doPlace`.
    fn do_place(&self, ctx: &mut Ctx<'_, '_>, origin: Pos) -> bool {
        let tree_height = self.trunk.tree_height(ctx);
        let foliage_height = self.foliage.foliage_height(ctx, tree_height);
        let trunk_height = tree_height - foliage_height;
        let leaf_radius = self.foliage.foliage_radius(ctx, trunk_height);
        let trunk_origin = match &self.root {
            Some(r) => r.trunk_origin(origin, ctx),
            None => origin,
        };
        let min_y = origin.1.min(trunk_origin.1);
        let max_y = origin.1.max(trunk_origin.1) + tree_height + 1;
        let (lo, hi) = (ctx.region.min_y(), ctx.region.min_y() + ctx.region.height() - 1);
        if min_y >= lo + 1 && max_y <= hi + 1 {
            let min_clipped = self.min_size.min_clipped();
            let clipped = self.max_free_tree_height(ctx, tree_height, trunk_origin);
            if clipped >= tree_height || min_clipped.is_some_and(|m| clipped >= m) {
                if let Some(r) = &self.root {
                    if !r.place_roots(self, ctx, origin, trunk_origin) {
                        return false;
                    }
                }
                let attachments = self.trunk.place_trunk(self, ctx, clipped, trunk_origin);
                for a in attachments {
                    self.foliage.create_foliage(self, ctx, clipped, &a, foliage_height, leaf_radius);
                }
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    /// `getMaxFreeTreeHeight`.
    fn max_free_tree_height(&self, ctx: &Ctx<'_, '_>, max_tree_height: i32, tree_pos: Pos) -> i32 {
        for y in 0..=max_tree_height + 1 {
            let r = self.min_size.size_at_height(max_tree_height, y);
            for x in -r..=r {
                for z in -r..=r {
                    let p = offset(tree_pos, x, y, z);
                    if !self.trunk.is_free(ctx, p) || (!self.ignore_vines && ctx.env.states.is_vine(ctx.id(p))) {
                        return y - 2;
                    }
                }
            }
        }
        max_tree_height
    }
}

/// `BoundingBox.encapsulatingPositions`, as `(min, max)` inclusive.
#[derive(Clone, Copy)]
pub struct Bounds {
    pub min: Pos,
    pub max: Pos,
}

impl Bounds {
    pub fn encapsulating(it: impl IntoIterator<Item = Pos>) -> Option<Bounds> {
        let mut it = it.into_iter();
        let first = it.next()?;
        let mut b = Bounds { min: first, max: first };
        for p in it {
            b.min = (b.min.0.min(p.0), b.min.1.min(p.1), b.min.2.min(p.2));
            b.max = (b.max.0.max(p.0), b.max.1.max(p.1), b.max.2.max(p.2));
        }
        Some(b)
    }

    #[inline]
    pub fn inside(&self, p: Pos) -> bool {
        p.0 >= self.min.0 && p.0 <= self.max.0 && p.1 >= self.min.1 && p.1 <= self.max.1 && p.2 >= self.min.2 && p.2 <= self.max.2
    }
}

/// `TreeFeature.updateLeaves`: the BFS from the logs (distance 0) through leaves and decorations that
/// assigns `distance` to every leaf within 6 blocks of a log. Returns nothing; writes the leaf states.
fn update_leaves(ctx: &mut Ctx<'_, '_>, bounds: Bounds, roots_and_decorations: &[Pos], logs: &[Pos]) {
    let (sx, sy, sz) = (
        (bounds.max.0 - bounds.min.0 + 1) as usize,
        (bounds.max.1 - bounds.min.1 + 1) as usize,
        (bounds.max.2 - bounds.min.2 + 1) as usize,
    );
    // BitSetDiscreteVoxelShape: index = (x * sy + y) * sz + z.
    let idx = |p: Pos| -> usize {
        let (x, y, z) = (
            (p.0 - bounds.min.0) as usize,
            (p.1 - bounds.min.1) as usize,
            (p.2 - bounds.min.2) as usize,
        );
        (x * sy + y) * sz + z
    };
    let mut shape = vec![false; sx * sy * sz];
    for &p in roots_and_decorations {
        if bounds.inside(p) {
            shape[idx(p)] = true;
        }
    }

    let mut to_check: Vec<JavaSet> = (0..7).map(|_| JavaSet::new()).collect();
    for &p in logs {
        to_check[0].insert(p);
    }
    let mut smallest = 0usize;
    let leaves_distance_tag = ctx.env.tags.prevents_nearby_leaf_decay;
    loop {
        while smallest >= 7 || !to_check[smallest].is_empty() {
            if smallest >= 7 {
                return;
            }
            let pos = to_check[smallest].pop_first().expect("non-empty bucket");
            if bounds.inside(pos) {
                if smallest != 0 {
                    let cur = ctx.id(pos);
                    let next = ctx.env.states.with(cur, "distance", &smallest.to_string());
                    ctx.set(pos, next);
                }
                shape[idx(pos)] = true;
                for dir in Dir::ALL {
                    let n = relative(pos, dir, 1);
                    if bounds.inside(n) && !shape[idx(n)] {
                        let cur = ctx.id(n);
                        if let Some(d) = leaf_distance_at(ctx, cur, leaves_distance_tag) {
                            let new_distance = d.min(smallest as i32 + 1);
                            if new_distance < 7 {
                                to_check[new_distance as usize].insert(n);
                                smallest = smallest.min(new_distance as usize);
                            }
                        }
                    }
                }
            }
        }
        smallest += 1;
    }
}

/// `LeavesBlock.getOptionalDistanceAt`: 0 for `prevents_nearby_leaf_decay` blocks (logs), else the
/// `distance` property when the state has one.
fn leaf_distance_at(ctx: &Ctx<'_, '_>, id: u16, prevents_tag: u16) -> Option<i32> {
    if ctx.region.tags.has(id, prevents_tag) {
        return Some(0);
    }
    ctx.env.states.get(id, "distance").map(|d| d.parse::<i32>().expect("distance"))
}

fn parse_feature_size(v: &Value) -> Result<FeatureSize, String> {
    let ty = v["type"].as_str().ok_or("minimum_size type")?;
    let int = |k: &str, d: i64| v[k].as_i64().unwrap_or(d) as i32;
    let min_clipped = v["min_clipped_height"].as_i64().map(|x| x as i32);
    match ty {
        "minecraft:two_layers_feature_size" => Ok(FeatureSize::Two {
            limit: int("limit", 1),
            lower: int("lower_size", 0),
            upper: int("upper_size", 1),
            min_clipped,
        }),
        "minecraft:three_layers_feature_size" => Ok(FeatureSize::Three {
            limit: int("limit", 1),
            upper_limit: int("upper_limit", 1),
            lower: int("lower_size", 0),
            middle: int("middle_size", 1),
            upper: int("upper_size", 1),
            min_clipped,
        }),
        other => Err(format!("unsupported feature size {other}")),
    }
}
