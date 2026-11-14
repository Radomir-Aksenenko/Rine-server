//! Placement modifiers (`levelgen/placement/*`) and value providers (`util/valueproviders`,
//! `levelgen/heightproviders`) for the subset the ore placements use. Every `modify` mirrors the Java
//! `modify(context, random, origin, output)` including the number and order of RNG draws.

use serde_json::Value;

use super::rng::WorldgenRng;
use super::region::Region;

/// `IntProvider`: constant, `UniformInt`, `TrapezoidInt`.
#[derive(Clone, Copy, Debug)]
pub enum IntProvider {
    Constant(i32),
    Uniform { min: i32, max: i32 },
    Trapezoid { min: i32, max: i32, plateau: i32 },
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
}

/// The placement modifiers that the ore placements use.
#[derive(Clone, Copy, Debug)]
pub enum Modifier {
    /// `count` (`CountPlacement`, a `RepeatingPlacement`).
    Count(IntProvider),
    /// `rarity_filter` (`RarityFilter`).
    RarityFilter(i32),
    /// `in_square` (`InSquarePlacement`).
    InSquare,
    /// `height_range` (`HeightRangePlacement`).
    HeightRange(HeightProvider),
    /// `biome` (`BiomeFilter`): the top placed feature must be in the biome at the origin.
    Biome,
    /// `offset` (`OffsetPlacement`).
    Offset(IntProvider, IntProvider, IntProvider),
}

pub fn sample_int(p: &IntProvider, rng: &mut WorldgenRng) -> i32 {
    match *p {
        IntProvider::Constant(v) => v,
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
/// `top` is the placed-feature id for `biome` checks (`PlacementContext.topFeature`).
pub fn modify(m: &Modifier, region: &mut Region, rng: &mut WorldgenRng, origin: (i32, i32, i32), top: usize, out: &mut Vec<(i32, i32, i32)>) {
    let (ox, oy, oz) = origin;
    match *m {
        Modifier::Count(p) => {
            // RepeatingPlacement.modify: count(random, origin) then emit origin count times.
            let count = sample_int(&p, rng);
            for _ in 0..count {
                out.push(origin);
            }
        }
        Modifier::RarityFilter(chance) => {
            // PlacementFilter.modify: shouldPlace(random) -> emit origin.
            if rng.next_float() < 1.0f32 / chance as f32 {
                out.push(origin);
            }
        }
        Modifier::InSquare => {
            let x = rng.next_int_bound(16) + ox;
            let z = rng.next_int_bound(16) + oz;
            out.push((x, oy, z));
        }
        Modifier::HeightRange(p) => {
            let y = sample_height(&p, rng, region.min_y(), region.height());
            out.push((ox, y, oz));
        }
        Modifier::Biome => {
            if region.biome_has_feature(ox, oy, oz, top) {
                out.push(origin);
            }
        }
        Modifier::Offset(x, y, z) => {
            let dx = sample_int(&x, rng);
            let dy = sample_int(&y, rng);
            let dz = sample_int(&z, rng);
            out.push((ox + dx, oy + dy, oz + dz));
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
        "minecraft:constant" => Ok(IntProvider::Constant(int("value")?)),
        "minecraft:uniform" => Ok(IntProvider::Uniform { min: int("min_inclusive")?, max: int("max_inclusive")? }),
        "minecraft:trapezoid" => Ok(IntProvider::Trapezoid {
            min: int("min_inclusive")?,
            max: int("max_inclusive")?,
            plateau: int("plateau")?,
        }),
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
        "minecraft:count" => Ok(Modifier::Count(parse_int_provider(&v["count"])?)),
        "minecraft:rarity_filter" => Ok(Modifier::RarityFilter(v["chance"].as_i64().ok_or("chance")? as i32)),
        "minecraft:in_square" => Ok(Modifier::InSquare),
        "minecraft:height_range" => Ok(Modifier::HeightRange(parse_height_provider(&v["height"])?)),
        "minecraft:biome" => Ok(Modifier::Biome),
        "minecraft:offset" => Ok(Modifier::Offset(
            parse_int_provider(&v["x"])?,
            parse_int_provider(&v["y"])?,
            parse_int_provider(&v["z"])?,
        )),
        other => Err(format!("unsupported placement modifier {other}")),
    }
}
