//! Placement modifiers (`levelgen/placement/*`) and value providers (`util/valueproviders`,
//! `levelgen/heightproviders`) for the subset the ore placements use. Every `modify` mirrors the Java
//! `modify(context, random, origin, output)` including the number and order of RNG draws.

use serde_json::Value;

use super::heightmap::Kind;
use super::predicate::BlockPredicate;
use super::rng::WorldgenRng;
use super::region::Region;
use crate::overworld::biome::biome_info_noise;

/// `IntProvider`: constant, `UniformInt`, `TrapezoidInt`, `BiasedToBottomInt`.
#[derive(Clone, Debug)]
pub enum IntProvider {
    Constant(i32),
    Uniform { min: i32, max: i32 },
    Trapezoid { min: i32, max: i32, plateau: i32 },
    BiasedToBottom { min: i32, max: i32 },
    Clamped { source: Box<IntProvider>, min: i32, max: i32 },
    ClampedNormal { mean: f32, deviation: f32, min: i32, max: i32 },
    /// `WeightedListInt`: `(provider, weight)` entries; one `nextInt(total)` draw picks the entry.
    Weighted(Vec<(IntProvider, i32)>),
}

impl IntProvider {
    /// `minInclusive()` and `maxInclusive()`.
    pub fn bounds(&self) -> (i32, i32) {
        match self {
            IntProvider::Constant(v) => (*v, *v),
            IntProvider::Uniform { min, max } | IntProvider::Trapezoid { min, max, .. } | IntProvider::BiasedToBottom { min, max }
            | IntProvider::ClampedNormal { min, max, .. } => (*min, *max),
            IntProvider::Clamped { source, min, max } => {
                let (lo, hi) = source.bounds();
                (lo.max(*min), hi.min(*max))
            }
            IntProvider::Weighted(entries) => {
                let lo = entries.iter().map(|(p, _)| p.bounds().0).min().unwrap_or(0);
                let hi = entries.iter().map(|(p, _)| p.bounds().1).max().unwrap_or(0);
                (lo, hi)
            }
        }
    }
}

/// `VerticalAnchor`.
#[derive(Clone, Copy, Debug)]
pub enum Anchor {
    Absolute(i32),
    AboveBottom(i32),
    BelowTop(i32),
}

impl Anchor {
    /// `VerticalAnchor.resolveY(WorldGenerationContext)`.
    fn resolve(self, min_gen_y: i32, gen_depth: i32) -> i32 {
        match self {
            Anchor::Absolute(y) => y,
            Anchor::AboveBottom(o) => min_gen_y + o,
            Anchor::BelowTop(o) => gen_depth - 1 + min_gen_y - o,
        }
    }
}

/// `HeightProvider`: constant, `UniformHeight`, `TrapezoidHeight`.
#[derive(Clone, Copy, Debug)]
pub enum HeightProvider {
    Constant(Anchor),
    Uniform { min: Anchor, max: Anchor },
    Trapezoid { min: Anchor, max: Anchor, plateau: i32 },
    /// `VeryBiasedToBottomHeight` (`inner` is the minimum gap, 1 by default).
    VeryBiased { min: Anchor, max: Anchor, inner: i32 },
}

/// The placement modifiers that the ore placements use.
#[derive(Clone, Debug)]
pub enum Modifier {
    /// `count` (`CountPlacement`, a `RepeatingPlacement`).
    Count(IntProvider),
    CountOnEveryLayer { count: IntProvider, empty_blocks: [u16; 2], bedrock: u16 },
    /// `rarity_filter` (`RarityFilter`).
    RarityFilter(i32),
    /// `in_square` (`InSquarePlacement`).
    InSquare,
    /// `height_range` (`HeightRangePlacement`).
    HeightRange(HeightProvider),
    /// `biome` (`BiomeFilter`): the top placed feature must be in the biome at the origin.
    Biome,
    /// `block_predicate_filter` (`BlockPredicateFilter`): the predicate holds at the origin.
    BlockPredicate(BlockPredicate),
    EnvironmentScan { dy: i32, target: BlockPredicate, allowed: BlockPredicate, steps: i32 },
    SurfaceWaterDepth(i32),
    SurfaceRelativeThreshold { kind: Kind, min: i32, max: i32 },
    RandomChance(f32),
    Cuboid { xz: IntProvider, y: IntProvider, edges: bool, interior: bool },
    /// `offset` (`OffsetPlacement`).
    Offset(IntProvider, IntProvider, IntProvider),
    RandomlySelected(Vec<Modifier>),
    /// `heightmap` (`HeightmapPlacement`): the column's height, when above the build bottom.
    Heightmap(Kind),
    /// `noise_threshold_count` (`NoiseThresholdCountPlacement`): `below` or `above` by the biome info noise at
    /// `(x / 200, z / 200)` against `level`.
    NoiseThresholdCount { level: f64, below: i32, above: i32 },
    /// `noise_based_count` (`NoiseBasedCountPlacement`): `ceil((noise + offset) * ratio)` at `(x / factor, z / factor)`.
    NoiseBasedCount { ratio: i32, factor: f64, offset: f64 },
}

pub fn sample_int(p: &IntProvider, rng: &mut WorldgenRng) -> i32 {
    if let IntProvider::Weighted(entries) = p {
        // WeightedRandom.getRandomItem: nextInt(total), then the first entry whose running sum exceeds it.
        let total: i32 = entries.iter().map(|e| e.1).sum();
        let mut sel = rng.next_int_bound(total);
        for (inner, w) in entries {
            sel -= w;
            if sel < 0 {
                return sample_int(inner, rng);
            }
        }
        return sample_int(&entries[entries.len() - 1].0, rng);
    }
    match *p {
        IntProvider::Constant(v) => v,
        IntProvider::Clamped { ref source, min, max } => sample_int(source, rng).clamp(min, max),
        IntProvider::ClampedNormal { mean, deviation, min, max } =>
            (mean + rng.next_gaussian() as f32 * deviation).clamp(min as f32, max as f32) as i32,
        IntProvider::Weighted(_) => unreachable!("handled above"),
        // BiasedToBottomInt.sample: min + nextInt(nextInt(max - min + 1) + 1).
        IntProvider::BiasedToBottom { min, max } => {
            let bound = rng.next_int_bound(max - min + 1) + 1;
            min + rng.next_int_bound(bound)
        }
        IntProvider::Uniform { min, max } => rng.next_int_between(min, max),
        IntProvider::Trapezoid { min, max, plateau } => {
            // TrapezoidInt.sample
            if plateau == 0 && max == -min {
                return rng.next_int_bound(max + 1) - rng.next_int_bound(max + 1);
            }
            let range = max - min;
            if plateau == range {
                return rng.next_int_between(min, max);
            }
            let plateau_start = (range - plateau) / 2;
            let plateau_end = range - plateau_start;
            min + rng.next_int_between(0, plateau_end) + rng.next_int_between(0, plateau_start)
        }
    }
}

/// `Mth.nextInt(random, min, max)`.
fn mth_next_int(rng: &mut WorldgenRng, min: i32, max: i32) -> i32 {
    if min >= max {
        min
    } else {
        rng.next_int_bound(max - min + 1) + min
    }
}

pub fn sample_height(p: &HeightProvider, rng: &mut WorldgenRng, min_gen_y: i32, gen_depth: i32) -> i32 {
    match *p {
        HeightProvider::Constant(a) => a.resolve(min_gen_y, gen_depth),
        HeightProvider::Uniform { min, max } => {
            let lo = min.resolve(min_gen_y, gen_depth);
            let hi = max.resolve(min_gen_y, gen_depth);
            if lo > hi {
                lo
            } else {
                rng.next_int_between(lo, hi)
            }
        }
        HeightProvider::VeryBiased { min, max, inner } => {
            // VeryBiasedToBottomHeight.sample; Mth.nextInt(r, a, b) draws nothing when a >= b.
            let lo = min.resolve(min_gen_y, gen_depth);
            let hi = max.resolve(min_gen_y, gen_depth);
            if hi - lo - inner + 1 <= 0 {
                return lo;
            }
            let upper = mth_next_int(rng, lo + inner, hi);
            let biased_upper = mth_next_int(rng, lo, upper - 1);
            mth_next_int(rng, lo, biased_upper - 1 + inner)
        }
        HeightProvider::Trapezoid { min, max, plateau } => {
            let lo = min.resolve(min_gen_y, gen_depth);
            let hi = max.resolve(min_gen_y, gen_depth);
            if lo > hi {
                return lo;
            }
            let range = hi - lo;
            if plateau >= range {
                return rng.next_int_between(lo, hi);
            }
            let plateau_start = (range - plateau) / 2;
            let plateau_end = range - plateau_start;
            lo + rng.next_int_between(0, plateau_end) + rng.next_int_between(0, plateau_start)
        }
    }
}

/// Applies one modifier to `origin`, pushing the outputs (in Java output order) into `out`.
/// `top` is the placed-feature id for `biome` checks (`PlacementContext.topFeature`); nested placements
/// have none, and vanilla throws for `biome` there, so that case is a bug in the data and panics.
pub fn modify(m: &Modifier, region: &mut Region, rng: &mut WorldgenRng, origin: (i32, i32, i32), top: Option<usize>, out: &mut Vec<(i32, i32, i32)>) {
    modify_with(m, region, rng, origin, top, |p| out.push(p));
}

pub(super) fn modify_with(m: &Modifier, region: &mut Region, rng: &mut WorldgenRng, origin: (i32, i32, i32), top: Option<usize>, mut out: impl FnMut((i32, i32, i32))) {
    let (ox, oy, oz) = origin;
    match m {
        Modifier::Count(p) => {
            // RepeatingPlacement.modify: count(random, origin) then emit origin count times.
            let count = sample_int(p, rng);
            for _ in 0..count {
                out(origin);
            }
        }
        Modifier::RarityFilter(chance) => {
            // PlacementFilter.modify: shouldPlace(random) -> emit origin.
            if rng.next_float() < 1.0f32 / *chance as f32 {
                out(origin);
            }
        }
        Modifier::InSquare => {
            let x = rng.next_int_bound(16) + ox;
            let z = rng.next_int_bound(16) + oz;
            out((x, oy, z));
        }
        Modifier::HeightRange(p) => {
            let y = sample_height(p, rng, region.min_y(), region.height());
            out((ox, y, oz));
        }
        Modifier::Biome => {
            let top = top.expect("biome filter on a nested placed feature");
            if region.biome_has_feature(ox, oy, oz, top) {
                out(origin);
            }
        }
        Modifier::BlockPredicate(p) => {
            if p.test(region, ox, oy, oz) {
                out(origin);
            }
        }
        Modifier::EnvironmentScan { dy, target, allowed, steps } => {
            let mut y = oy;
            if !allowed.test(region, ox, y, oz) {
                return;
            }
            for _ in 0..*steps {
                if target.test(region, ox, y, oz) {
                    out((ox, y, oz));
                    return;
                }
                y += dy;
                if region.outside_build_height(y) {
                    return;
                }
                if !allowed.test(region, ox, y, oz) {
                    break;
                }
            }
            if target.test(region, ox, y, oz) {
                out((ox, y, oz));
            }
        }
        Modifier::CountOnEveryLayer { count, empty_blocks, bedrock } => {
            let mut floors: [Option<Vec<i32>>; 256] = std::array::from_fn(|_| None);
            let tags = region.tags;
            let empty = |state| tags.is_air(state) || empty_blocks.contains(&tags.block_of(state));
            for layer in 0.. {
                let mut found = false;
                let mut attempt = 0;
                // CountOnEveryLayerPlacement samples its bound on each loop check.
                while attempt < sample_int(count, rng) {
                    let lx = rng.next_int_bound(16);
                    let lz = rng.next_int_bound(16);
                    let (x, z) = (ox + lx, oz + lz);
                    let column = floors[(lz * 16 + lx) as usize].get_or_insert_with(|| {
                        let top = region.height_of(Kind::MotionBlocking, x, z);
                        let mut floors = Vec::new();
                        let mut air = empty(region.get_block(x, top, z));
                        for y in (region.min_y() + 1..=top).rev() {
                            let state = region.get_block(x, y - 1, z);
                            let below = empty(state);
                            if air && !below && !tags.is_block(state, *bedrock) {
                                floors.push(y);
                            }
                            air = below;
                        }
                        floors
                    });
                    if let Some(&y) = column.get(layer) {
                        out((x, y, z));
                        found = true;
                    }
                    attempt += 1;
                }
                if !found {
                    break;
                }
            }
        }
        Modifier::RandomChance(chance) => {
            if rng.next_float() < *chance {
                out(origin);
            }
        }
        Modifier::Cuboid { xz, y, edges, interior } => {
            let height = sample_int(y, rng);
            let width = sample_int(xz, rng);
            let length = sample_int(xz, rng);
            for x in 0..=width {
                let edge_x = x == 0 || x == width;
                for y in 0..=height {
                    let edge_y = y == 0 || y == height;
                    for z in 0..=length {
                        let boundary_axes = u8::from(edge_x) + u8::from(edge_y) + u8::from(z == 0 || z == length);
                        if (*edges || boundary_axes < 2) && (*interior || boundary_axes != 0) {
                            out((ox + x, oy + y, oz + z));
                        }
                    }
                }
            }
        }
        Modifier::SurfaceRelativeThreshold { kind, min, max } => {
            let delta = oy as i64 - region.height_of(*kind, ox, oz) as i64;
            if (*min as i64..=*max as i64).contains(&delta) {
                out(origin);
            }
        }
        Modifier::SurfaceWaterDepth(max_depth) => {
            let surface = region.height_of(Kind::WorldSurface, ox, oz);
            let bottom = region.height_of(Kind::OceanFloor, ox, oz);
            if surface - bottom <= *max_depth {
                out(origin);
            }
        }
        Modifier::Offset(x, y, z) => {
            let dx = sample_int(x, rng);
            let dy = sample_int(y, rng);
            let dz = sample_int(z, rng);
            out((ox + dx, oy + dy, oz + dz));
        }
        Modifier::RandomlySelected(list) => {
            let i = rng.next_int_bound(list.len() as i32) as usize;
            // Rare (randomly_selected only); a Vec avoids recursive generic instantiation.
            let mut inner = Vec::new();
            modify(&list[i], region, rng, origin, top, &mut inner);
            for q in inner {
                out(q);
            }
        }
        Modifier::NoiseThresholdCount { level, below, above } => {
            // RepeatingPlacement.count: no random draws; the origin is emitted `count` times.
            let noise = biome_info_noise(ox as f64 / 200.0, oz as f64 / 200.0) as f64;
            let count = if noise < *level { *below } else { *above };
            for _ in 0..count {
                out(origin);
            }
        }
        Modifier::NoiseBasedCount { ratio, factor, offset } => {
            // (int)Math.ceil((noise + offset) * ratio): Rust's saturating cast matches d2i.
            let noise = biome_info_noise(ox as f64 / factor, oz as f64 / factor) as f64;
            let count = ((noise + offset) * *ratio as f64).ceil() as i32;
            for _ in 0..count {
                out(origin);
            }
        }
        Modifier::Heightmap(kind) => {
            // HeightmapPlacement.modify: emit (x, height, z) when height > minY; draws no random numbers.
            let height = region.height_of(*kind, ox, oz);
            if height > region.min_y() {
                out((ox, height, oz));
            }
        }
    }
}

fn parse_anchor(v: &Value) -> Result<Anchor, String> {
    let obj = v.as_object().ok_or_else(|| format!("anchor not an object: {v}"))?;
    if let Some(y) = obj.get("absolute") {
        return Ok(Anchor::Absolute(y.as_i64().ok_or("absolute")? as i32));
    }
    if let Some(o) = obj.get("above_bottom") {
        return Ok(Anchor::AboveBottom(o.as_i64().ok_or("above_bottom")? as i32));
    }
    if let Some(o) = obj.get("below_top") {
        return Ok(Anchor::BelowTop(o.as_i64().ok_or("below_top")? as i32));
    }
    Err(format!("unknown anchor {v}"))
}

pub fn parse_int_provider(v: &Value) -> Result<IntProvider, String> {
    if let Some(n) = v.as_i64() {
        return Ok(IntProvider::Constant(n as i32));
    }
    let ty = v["type"].as_str().ok_or_else(|| format!("int provider without type: {v}"))?;
    let int = |k: &str| -> Result<i32, String> { v[k].as_i64().map(|n| n as i32).ok_or_else(|| format!("{ty} missing {k}")) };
    match ty {
        "minecraft:weighted_list" => {
            let mut entries = Vec::new();
            for e in v["distribution"].as_array().ok_or("weighted_list distribution")? {
                entries.push((parse_int_provider(&e["data"])?, e["weight"].as_i64().ok_or("weight")? as i32));
            }
            if entries.is_empty() || entries.iter().map(|e| e.1).sum::<i32>() <= 0 {
                return Err("weighted_list without weight".into());
            }
            Ok(IntProvider::Weighted(entries))
        }
        "minecraft:constant" => Ok(IntProvider::Constant(int("value")?)),
        "minecraft:uniform" => Ok(IntProvider::Uniform { min: int("min_inclusive")?, max: int("max_inclusive")? }),
        "minecraft:clamped" => {
            let min = int("min_inclusive")?;
            let max = int("max_inclusive")?;
            if min > max { return Err("invalid clamped int bounds".into()); }
            Ok(IntProvider::Clamped { source: Box::new(parse_int_provider(&v["source"])?), min, max })
        }
        "minecraft:clamped_normal" => {
            let min = int("min_inclusive")?;
            let max = int("max_inclusive")?;
            if min > max { return Err("invalid clamped normal int bounds".into()); }
            Ok(IntProvider::ClampedNormal {
                mean: v["mean"].as_f64().ok_or("clamped normal mean")? as f32,
                deviation: v["deviation"].as_f64().ok_or("clamped normal deviation")? as f32,
                min, max,
            })
        }
        // TrapezoidInt's codec uses "min"/"max" (UniformInt and BiasedToBottomInt use "min_inclusive"/"max_inclusive").
        "minecraft:trapezoid" => Ok(IntProvider::Trapezoid {
            min: int("min")?,
            max: int("max")?,
            plateau: int("plateau")?,
        }),
        "minecraft:biased_to_bottom" => Ok(IntProvider::BiasedToBottom { min: int("min_inclusive")?, max: int("max_inclusive")? }),
        other => Err(format!("unsupported int provider {other}")),
    }
}

pub fn parse_height_provider(v: &Value) -> Result<HeightProvider, String> {
    let ty = v["type"].as_str().ok_or_else(|| format!("height provider without type: {v}"))?;
    match ty {
        "minecraft:constant" => Ok(HeightProvider::Constant(parse_anchor(&v["value"])?)),
        "minecraft:uniform" => Ok(HeightProvider::Uniform {
            min: parse_anchor(&v["min_inclusive"])?,
            max: parse_anchor(&v["max_inclusive"])?,
        }),
        "minecraft:very_biased_to_bottom" => Ok(HeightProvider::VeryBiased {
            min: parse_anchor(&v["min_inclusive"])?,
            max: parse_anchor(&v["max_inclusive"])?,
            inner: v["inner"].as_i64().unwrap_or(1) as i32,
        }),
        "minecraft:trapezoid" => Ok(HeightProvider::Trapezoid {
            min: parse_anchor(&v["min_inclusive"])?,
            max: parse_anchor(&v["max_inclusive"])?,
            plateau: v["plateau"].as_i64().unwrap_or(0) as i32,
        }),
        other => Err(format!("unsupported height provider {other}")),
    }
}

/// Parses one placement entry. Types outside the implemented subset are errors, so a placed feature
/// that needs them cannot silently decorate without them.
pub fn parse_modifier(v: &Value) -> Result<Modifier, String> {
    let ty = v["type"].as_str().ok_or_else(|| format!("placement without type: {v}"))?;
    match ty {
        "minecraft:randomly_selected" => {
            let list = v["placements"].as_array().filter(|items| !items.is_empty()).ok_or("randomly_selected placements")?;
            Ok(Modifier::RandomlySelected(list.iter().map(parse_modifier).collect::<Result<_, _>>()?))
        }
        "minecraft:count" => Ok(Modifier::Count(parse_int_provider(&v["count"])?)),
        "minecraft:rarity_filter" => Ok(Modifier::RarityFilter(v["chance"].as_i64().ok_or("chance")? as i32)),
        "minecraft:random_chance" => Ok(Modifier::RandomChance(v["chance"].as_f64().ok_or("chance")? as f32)),
        "minecraft:in_square" => Ok(Modifier::InSquare),
        "minecraft:height_range" => Ok(Modifier::HeightRange(parse_height_provider(&v["height"])?)),
        "minecraft:biome" => Ok(Modifier::Biome),
        "minecraft:offset" => Ok(Modifier::Offset(
            parse_int_provider(&v["x"])?,
            parse_int_provider(&v["y"])?,
            parse_int_provider(&v["z"])?,
        )),
        "minecraft:heightmap" => Ok(Modifier::Heightmap(Kind::from_name(v["heightmap"].as_str().ok_or("heightmap")?)?)),
        "minecraft:surface_water_depth_filter" => Ok(Modifier::SurfaceWaterDepth(v["max_water_depth"].as_i64().ok_or("max_water_depth")? as i32)),
        "minecraft:cuboid" => Ok(Modifier::Cuboid {
            xz: parse_int_provider(&v["xz_size"])?,
            y: parse_int_provider(&v["y_size"])?,
            edges: v["include_edges"].as_bool().unwrap_or(true),
            interior: v["include_interior"].as_bool().unwrap_or(true),
        }),
        "minecraft:surface_relative_threshold_filter" => Ok(Modifier::SurfaceRelativeThreshold {
            kind: Kind::from_name(v["heightmap"].as_str().ok_or("heightmap")?)?,
            min: v["min_inclusive"].as_i64().unwrap_or(i32::MIN as i64) as i32,
            max: v["max_inclusive"].as_i64().unwrap_or(i32::MAX as i64) as i32,
        }),
        "minecraft:noise_threshold_count" => Ok(Modifier::NoiseThresholdCount {
            level: v["noise_level"].as_f64().ok_or("noise_level")?,
            below: v["below_noise"].as_i64().ok_or("below_noise")? as i32,
            above: v["above_noise"].as_i64().ok_or("above_noise")? as i32,
        }),
        "minecraft:noise_based_count" => Ok(Modifier::NoiseBasedCount {
            ratio: v["noise_to_count_ratio"].as_i64().ok_or("noise_to_count_ratio")? as i32,
            factor: v["noise_factor"].as_f64().ok_or("noise_factor")?,
            offset: v["noise_offset"].as_f64().unwrap_or(0.0),
        }),
        other => Err(format!("unsupported placement modifier {other}")),
    }
}
