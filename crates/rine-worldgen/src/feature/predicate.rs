//! `BlockPredicate` (`levelgen/blockpredicates`) for the predicates that only read block identity, tags
//! and fluids: `true`, `matching_blocks`, `matching_block_tag`, `matching_fluids`, `all_of`, `any_of`
//! and `not`. Predicates that need block shape or survival (`solid`, `has_sturdy_face`, `would_survive`,
//! `replaceable`, `unobstructed`, ...) are rejected at compile time instead of being approximated.
//!
//! `StateTestingPredicate.test(level, origin)` reads `origin + offset`; the combining predicates
//! short-circuit in the same order as the Java classes.

use serde_json::Value;

use super::region::Region;
use super::state::{Fluid, Palette};

#[derive(Clone, Debug)]
pub enum BlockPredicate {
    True,
    /// `matching_blocks`: the state's block is one of `blocks` (block indices).
    MatchingBlocks { offset: [i32; 3], blocks: Vec<u16> },
    /// `matching_block_tag`: the state's block is in the tag (tag id).
    MatchingTag { offset: [i32; 3], tag: u16 },
    /// `matching_fluids`: the state's fluid is one of `fluids`.
    MatchingFluids { offset: [i32; 3], fluids: Vec<Fluid> },
    AllOf(Vec<BlockPredicate>),
    AnyOf(Vec<BlockPredicate>),
    Not(Box<BlockPredicate>),
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
        other => Err(format!("unsupported block predicate {other}")),
    }
}

impl BlockPredicate {
    /// `BlockPredicate.test(level, origin)`. Reads only; draws no random numbers.
    pub fn test(&self, region: &Region, x: i32, y: i32, z: i32) -> bool {
        match self {
            BlockPredicate::True => true,
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
        }
    }
}
