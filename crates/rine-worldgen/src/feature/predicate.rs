//! `BlockPredicate` (`levelgen/blockpredicates`) for the predicates that only read block identity, tags
//! and fluids: `true`, `matching_blocks`, `matching_block_tag`, `matching_fluids`, `all_of`, `any_of`
//! and `not`, plus the solid-state and supported vegetation survival checks.
//!
//! `StateTestingPredicate.test(level, origin)` reads `origin + offset`; the combining predicates
//! short-circuit in the same order as the Java classes.

use serde_json::Value;

use super::region::Region;
use super::state::{Fluid, Palette};
use super::vegetation;

#[derive(Clone, Debug)]
pub enum BlockPredicate {
    True,
    Solid([i32; 3]),
    Replaceable([i32; 3]),
    InsideBuildHeight(i32),
    SturdyFace { offset: [i32; 3], face: usize },
    /// `matching_blocks`: the state's block is one of `blocks` (block indices).
    MatchingBlocks { offset: [i32; 3], blocks: Vec<u16> },
    /// `matching_block_tag`: the state's block is in the tag (tag id).
    MatchingTag { offset: [i32; 3], tag: u16 },
    /// `matching_fluids`: the state's fluid is one of `fluids`.
    MatchingFluids { offset: [i32; 3], fluids: Vec<Fluid> },
    AllOf(Vec<BlockPredicate>),
    AnyOf(Vec<BlockPredicate>),
    Not(Box<BlockPredicate>),
    Volume { min: [i32; 3], max: [i32; 3], predicate: Box<BlockPredicate> },
    /// `would_survive` for a sapling or mangrove propagule: `state.canSurvive(level, origin + offset)`. A standing
    /// state needs the block below in `support` (`VegetationBlock.canSurvive`). A hanging mangrove propagule needs
    /// the block above in `hanging` and nothing else (`MangrovePropaguleBlock.canSurvive`).
    WouldSurvive { offset: [i32; 3], support: u16, hanging: Option<u16> },
    /// `would_survive` for a block of `vegetation.rs`: `state.canSurvive(level, origin + offset)` with the
    /// survival class of that block. `state` is a palette id.
    VegetationSurvives { offset: [i32; 3], state: u16 },
}

fn offset_of(v: &Value) -> Result<[i32; 3], String> {
    match v.get("offset") {
        None => Ok([0, 0, 0]),
        Some(o) => {
            let a = o.as_array().ok_or("offset must be an array")?;
            if a.len() != 3 {
                return Err(format!("offset {o}"));
            }
            let mut out = [0; 3];
            for (i, c) in a.iter().enumerate() {
                out[i] = c.as_i64().ok_or("offset component")? as i32;
            }
            Ok(out)
        }
    }
}

/// `Identifier` with its property list: `name[k=v,...]` -> (`name`, `k=v,...`).
fn split_state(state: &str) -> (&str, Option<&str>) {
    match state.find('[') {
        Some(i) => (&state[..i], Some(state[i + 1..].trim_end_matches(']'))),
        None => (state, None),
    }
}

/// A single name or a list of names.
fn names_of(v: &Value) -> Result<Vec<&str>, String> {
    match v {
        Value::String(s) => Ok(vec![s.as_str()]),
        Value::Array(a) => a.iter().map(|x| x.as_str().ok_or_else(|| format!("name expected: {x}"))).collect(),
        other => Err(format!("name list expected: {other}")),
    }
}

fn fluid_by_name(name: &str) -> Result<Fluid, String> {
    match name {
        "minecraft:empty" => Ok(Fluid::Empty),
        "minecraft:water" => Ok(Fluid::Water),
        "minecraft:flowing_water" => Ok(Fluid::FlowingWater),
        "minecraft:lava" => Ok(Fluid::Lava),
        "minecraft:flowing_lava" => Ok(Fluid::FlowingLava),
        other => Err(format!("unsupported fluid {other}")),
    }
}

/// Parses a block predicate. `tag_names` is the sorted tag list of the bundle; blocks are interned in `pal`.
pub fn parse(v: &Value, tag_names: &[String], pal: &mut Palette) -> Result<BlockPredicate, String> {
    let ty = v["type"].as_str().ok_or_else(|| format!("block predicate without type: {v}"))?;
    match ty {
        "minecraft:true" => Ok(BlockPredicate::True),
        "minecraft:solid" => Ok(BlockPredicate::Solid(offset_of(v)?)),
        "minecraft:replaceable" => Ok(BlockPredicate::Replaceable(offset_of(v)?)),
        "minecraft:inside_world_bounds" => Ok(BlockPredicate::InsideBuildHeight(offset_of(v)?[1])),
        "minecraft:has_sturdy_face" => {
            let direction = super::column::direction_of(v["direction"].as_str().ok_or("direction")?)?;
            let face = super::shape::DIRECTIONS.iter().position(|&d| d == direction).ok_or("invalid face")?;
            Ok(BlockPredicate::SturdyFace { offset: offset_of(v)?, face })
        }
        "minecraft:matching_blocks" => {
            let mut blocks = Vec::new();
            for name in names_of(&v["blocks"])? {
                blocks.push(pal.block(name));
            }
            Ok(BlockPredicate::MatchingBlocks { offset: offset_of(v)?, blocks })
        }
        "minecraft:matching_block_tag" => {
            let name = v["tag"].as_str().ok_or("tag")?;
            let tag = tag_names.binary_search_by(|n| n.as_str().cmp(name)).map_err(|_| format!("tag {name} not bundled"))?;
            Ok(BlockPredicate::MatchingTag { offset: offset_of(v)?, tag: tag as u16 })
        }
        "minecraft:matching_fluids" => {
            let mut fluids = Vec::new();
            for name in names_of(&v["fluids"])? {
                fluids.push(fluid_by_name(name)?);
            }
            Ok(BlockPredicate::MatchingFluids { offset: offset_of(v)?, fluids })
        }
        "minecraft:all_of" | "minecraft:any_of" => {
            let list = v["predicates"]
                .as_array()
                .ok_or("predicates")?
                .iter()
                .map(|p| parse(p, tag_names, pal))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(if ty == "minecraft:all_of" { BlockPredicate::AllOf(list) } else { BlockPredicate::AnyOf(list) })
        }
        "minecraft:not" => Ok(BlockPredicate::Not(Box::new(parse(&v["predicate"], tag_names, pal)?))),
        "minecraft:volume_match" => {
            let bound = |key: &str| {
                let a = v[key].as_array().filter(|a| a.len() == 3).ok_or("volume_match bounds")?;
                let coord = |i: usize| a[i].as_i64().filter(|n| (-16..=16).contains(n)).map(|n| n as i32).ok_or("volume_match coordinate");
                Ok::<_, &str>([coord(0)?, coord(1)?, coord(2)?])
            };
            let min = bound("min")?;
            let max = bound("max")?;
            if min.iter().zip(max).any(|(&a, b)| a > b) {
                return Err("volume_match min exceeds max".into());
            }
            Ok(BlockPredicate::Volume { min, max, predicate: Box::new(parse(&v["match"], tag_names, pal)?) })
        }
        "minecraft:would_survive" => {
            let state = v["state"].as_str().ok_or("would_survive state")?;
            let (name, props) = split_state(state);
            let tag = |n: &str| -> Result<u16, String> {
                tag_names
                    .binary_search_by(|t| t.as_str().cmp(n))
                    .map(|i| i as u16)
                    .map_err(|_| format!("tag {n} not bundled"))
            };
            let offset = offset_of(v)?;
            if name == "minecraft:mangrove_propagule" {
                if props.is_some_and(|p| p.split(',').any(|kv| kv == "hanging=true")) {
                    Ok(BlockPredicate::WouldSurvive {
                        offset,
                        support: 0,
                        hanging: Some(tag("minecraft:supports_hanging_mangrove_propagule")?),
                    })
                } else {
                    Ok(BlockPredicate::WouldSurvive { offset, support: tag("minecraft:supports_mangrove_propagule")?, hanging: None })
                }
            } else if name.ends_with("_sapling") {
                Ok(BlockPredicate::WouldSurvive { offset, support: tag("minecraft:supports_vegetation")?, hanging: None })
            } else if vegetation::kind_of_name(name).is_some() {
                let state = match props {
                    None => pal.default_state(name),
                    Some(p) => {
                        let kv: Vec<(&str, &str)> = p.split(',').filter_map(|kv| kv.split_once('=')).collect();
                        pal.state_from_parts(name, &kv)
                    }
                };
                Ok(BlockPredicate::VegetationSurvives { offset, state })
            } else {
                Err(format!("would_survive for {name} is not ported"))
            }
        }
        other => Err(format!("unsupported block predicate {other}")),
    }
}

impl BlockPredicate {
    /// `BlockPredicate.test(level, origin)`. Reads only; draws no random numbers.
    pub fn test(&self, region: &Region, x: i32, y: i32, z: i32) -> bool {
        match self {
            BlockPredicate::True => true,
            BlockPredicate::InsideBuildHeight(dy) => !region.outside_build_height(y + dy),
            BlockPredicate::Solid([dx, dy, dz]) => region.tags.solid(region.get_block(x + dx, y + dy, z + dz)),
            BlockPredicate::Replaceable([dx, dy, dz]) => region.tags.replaceable(region.get_block(x + dx, y + dy, z + dz)),
            BlockPredicate::SturdyFace { offset: [dx, dy, dz], face } => {
                region.tags.sturdy(region.get_block(x + dx, y + dy, z + dz), *face)
            }
            BlockPredicate::MatchingBlocks { offset, blocks } => {
                let s = region.get_block(x + offset[0], y + offset[1], z + offset[2]);
                let b = region.tags.block_of(s);
                blocks.contains(&b)
            }
            BlockPredicate::MatchingTag { offset, tag } => {
                let s = region.get_block(x + offset[0], y + offset[1], z + offset[2]);
                region.tags.has(s, *tag)
            }
            BlockPredicate::MatchingFluids { offset, fluids } => {
                let s = region.get_block(x + offset[0], y + offset[1], z + offset[2]);
                fluids.contains(&region.tags.fluid(s))
            }
            BlockPredicate::AllOf(list) => list.iter().all(|p| p.test(region, x, y, z)),
            BlockPredicate::AnyOf(list) => list.iter().any(|p| p.test(region, x, y, z)),
            BlockPredicate::Not(p) => !p.test(region, x, y, z),
            BlockPredicate::Volume { min, max, predicate } => {
                for dx in min[0]..=max[0] {
                    for dz in min[2]..=max[2] {
                        for dy in min[1]..=max[1] {
                            if !predicate.test(region, x + dx, y + dy, z + dz) {
                                return false;
                            }
                        }
                    }
                }
                true
            }
            BlockPredicate::WouldSurvive { offset, support, hanging } => {
                let (px, py, pz) = (x + offset[0], y + offset[1], z + offset[2]);
                match hanging {
                    Some(tag) => region.tags.has(region.get_block(px, py + 1, pz), *tag),
                    None => region.tags.has(region.get_block(px, py - 1, pz), *support),
                }
            }
            BlockPredicate::VegetationSurvives { offset, state } => {
                vegetation::can_survive(region, *state, x + offset[0], y + offset[1], z + offset[2])
            }
        }
    }
}
