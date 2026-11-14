//! Vegetation survival (`canSurvive` / `mayPlaceOn`) for the blocks the simple block feature places, and the
//! per-state tables that feature reads at placement time.
//!
//! Every block is classified by name when the decorator is compiled (`kind_of_name`). A name outside that
//! table is a compile error, and `assets/worldgen/ported_features.json` (`simple_blocks`) lists the same names
//! for the allow rule, so a feature that places an unclassified block is never decorated.
//!
//! Ground support uses `isFaceSturdy(UP)` of the default state (`block_behaviour.tsv`). The supporting blocks
//! of these features are full cubes, whose sturdy face does not depend on properties.

use serde_json::Value;

use super::region::{Region, Tags};
use super::state::Palette;
use super::tree::fallen::sturdy_up_default;

/// Survival class of a vegetation block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Survival {
    /// `VegetationBlock.canSurvive`: the block below is in `supports_vegetation`.
    Vegetation,
    /// `DryVegetationBlock.mayPlaceOn`: the block below is in `supports_dry_vegetation`.
    DryVegetation,
    /// `DoublePlantBlock.canSurvive`: a lower half as `Vegetation`; an upper half needs the same block's lower
    /// half below it.
    DoublePlant,
    /// `SeaPickleBlock.mayPlaceOn`: the block below has a sturdy UP face.
    SeaPickle,
    /// `SeagrassBlock.mayPlaceOn`: a sturdy UP face and not `cannot_support_seagrass`.
    Seagrass,
}

/// Survival class of a block name, `None` for blocks the port does not classify.
pub fn kind_of_name(name: &str) -> Option<Survival> {
    Some(match name {
        "minecraft:poppy" | "minecraft:dandelion" | "minecraft:short_grass" | "minecraft:fern" => Survival::Vegetation,
        "minecraft:tall_grass" | "minecraft:large_fern" | "minecraft:sunflower" | "minecraft:lilac" | "minecraft:rose_bush" | "minecraft:peony" => {
            Survival::DoublePlant
        }
        "minecraft:dead_bush" => Survival::DryVegetation,
        "minecraft:sea_pickle" => Survival::SeaPickle,
        "minecraft:seagrass" => Survival::Seagrass,
        _ => return None,
    })
}

/// Blocks that `canBeReplaced()` for the vegetation that a double plant can meet above it. Air is handled by the
/// caller. This is the vanilla `replaceable` set for the blocks listed here; any other block above a double plant
/// counts as not replaceable, which is exact for every state the ported features can produce.
const REPLACEABLE: [&str; 3] = ["minecraft:short_grass", "minecraft:fern", "minecraft:dead_bush"];

/// Block names a block state provider can answer with, from its JSON (`BlockStateProvider` codec).
pub fn provider_names(v: &Value, out: &mut Vec<String>) -> Result<(), String> {
    if let Some(name) = v.as_str() {
        out.push(name.to_string());
        return Ok(());
    }
    match v.get("type").and_then(Value::as_str) {
        None => out.push(v["id"].as_str().ok_or_else(|| format!("block state without id: {v}"))?.to_string()),
        Some("minecraft:weighted") => {
            for e in v["entries"].as_array().ok_or("entries")? {
                provider_names(&e["data"], out)?;
            }
        }
        Some("minecraft:randomized_int") => provider_names(&v["source"], out)?,
        Some("minecraft:rule_based") => {
            for r in v["rules"].as_array().ok_or("rules")? {
                provider_names(&r["then"], out)?;
            }
            if let Some(f) = v.get("fallback") {
                provider_names(f, out)?;
            }
        }
        Some(other) => return Err(format!("state provider {other} is not a vegetation provider")),
    }
    Ok(())
}

/// Per-block and per-state tables, built once the palette is final.
#[derive(Default)]
pub struct VegTable {
    /// Per block index: survival class, `None` for other blocks.
    kind: Vec<Option<Survival>>,
    /// Per block index: `isFaceSturdy(UP)` of the default state.
    sturdy_up: Vec<bool>,
    /// Per block index: `canBeReplaced()` (see `REPLACEABLE`).
    replaceable: Vec<bool>,
    /// Per palette state: 0 for no half, 1 for `lower`, 2 for `upper` (`DoubleBlockHalf`).
    half: Vec<u8>,
    /// Per palette state of a double plant: the same state with `half=lower`.
    lower_of: Vec<u16>,
    /// Per palette state of a double plant: the same state with `half=upper`.
    upper_of: Vec<u16>,
    tag_vegetation: u16,
    tag_dry: u16,
    tag_no_seagrass: u16,
}

impl VegTable {
    /// Builds the tables for the final palette. `tag_names` is the sorted tag list of the bundle. Interns the
    /// `half` variants of the double plant states, so it runs after every other palette entry is interned.
    pub fn build(pal: &mut Palette, tag_names: &[String]) -> Result<VegTable, String> {
        let tag = |n: &str| -> Result<u16, String> {
            tag_names.binary_search_by(|t| t.as_str().cmp(n)).map(|i| i as u16).map_err(|_| format!("tag {n} not bundled"))
        };
        let mut t = VegTable {
            tag_vegetation: tag("minecraft:supports_vegetation")?,
            tag_dry: tag("minecraft:supports_dry_vegetation")?,
            tag_no_seagrass: tag("minecraft:cannot_support_seagrass")?,
            ..VegTable::default()
        };

        // Both half variants of every double plant state exist in the palette before the tables are sized.
        let block_names: Vec<String> = (0..pal.block_count()).map(|b| pal.block_name(b as u16).to_string()).collect();
        let double_blocks: Vec<bool> = block_names.iter().map(|n| kind_of_name(n) == Some(Survival::DoublePlant)).collect();
        for s in 0..pal.states().len() {
            if double_blocks[pal.block_of()[s] as usize] {
                pal.with_property(s as u16, "half", "lower").ok_or("double plant without half")?;
                pal.with_property(s as u16, "half", "upper").ok_or("double plant without half")?;
            }
        }

        let n_states = pal.states().len();
        t.half = vec![0; n_states];
        t.lower_of = vec![u16::MAX; n_states];
        t.upper_of = vec![u16::MAX; n_states];
        for s in 0..n_states {
            if !double_blocks[pal.block_of()[s] as usize] {
                continue;
            }
            t.half[s] = if pal.states()[s].contains("half=upper") { 2 } else { 1 };
            t.lower_of[s] = pal.with_property(s as u16, "half", "lower").ok_or("double plant without half")?;
            t.upper_of[s] = pal.with_property(s as u16, "half", "upper").ok_or("double plant without half")?;
        }

        // `isFaceSturdy(UP)` is read for every block a vegetation state can stand on, so every palette block
        // needs behaviour data: a missing entry is an error, not a default.
        for name in &block_names {
            t.kind.push(kind_of_name(name));
            let sturdy = sturdy_up_default(name).ok_or_else(|| format!("no behaviour data for {name}"))?;
            t.sturdy_up.push(sturdy);
            t.replaceable.push(REPLACEABLE.contains(&name.as_str()));
        }
        Ok(t)
    }

    /// Survival class of a palette state's block.
    #[inline]
    pub fn kind_of_block(&self, block: u16) -> Survival {
        self.kind[block as usize].expect("vegetation block without survival class: checked at compile time")
    }

    #[inline]
    pub fn half(&self, state: u16) -> u8 {
        self.half[state as usize]
    }

    /// `state.setValue(HALF, LOWER)` / `state.setValue(HALF, UPPER)` for a double plant state.
    #[inline]
    pub fn lower_of(&self, state: u16) -> u16 {
        self.lower_of[state as usize]
    }

    #[inline]
    pub fn upper_of(&self, state: u16) -> u16 {
        self.upper_of[state as usize]
    }
}

/// `BlockState.canSurvive(level, pos)` for the vegetation state `s` at `(x, y, z)`. Reads only.
pub fn can_survive(region: &Region, s: u16, x: i32, y: i32, z: i32) -> bool {
    survive_with(region.tags, &|dx, dy, dz| region.get_block(x + dx, y + dy, z + dz), s)
}

fn survive_with(tags: &Tags, at: &dyn Fn(i32, i32, i32) -> u16, s: u16) -> bool {
    let v = &tags.veg;
    let block = tags.block_of(s);
    let below = at(0, -1, 0);
    match v.kind_of_block(block) {
        Survival::Vegetation => tags.has(below, v.tag_vegetation),
        Survival::DryVegetation => tags.has(below, v.tag_dry),
        Survival::DoublePlant => {
            if v.half(s) == 2 {
                // DoublePlantBlock.canSurvive: the upper half needs this block's lower half below.
                tags.block_of(below) == block && v.half(below) == 1
            } else {
                tags.has(below, v.tag_vegetation)
            }
        }
        Survival::SeaPickle => v.sturdy_up[tags.block_of(below) as usize],
        Survival::Seagrass => v.sturdy_up[tags.block_of(below) as usize] && !tags.has(below, v.tag_no_seagrass),
    }
}

/// `DoublePlantBlock`: the state above a double plant can be replaced (`Objects.equals` on the fluid is checked by
/// the caller). Air is replaceable by definition of `isAir`, so only the listed blocks reach here.
pub fn above_replaceable(tags: &Tags, s: u16) -> bool {
    tags.veg.replaceable[tags.block_of(s) as usize]
}

/// Whether `s` is a double plant state (`Survival::DoublePlant`), which places two halves.
pub fn is_double_plant(tags: &Tags, s: u16) -> bool {
    tags.veg.kind_of_block(tags.block_of(s)) == Survival::DoublePlant
}

pub fn lower_state(tags: &Tags, s: u16) -> u16 {
    tags.veg.lower_of(s)
}

pub fn upper_state(tags: &Tags, s: u16) -> u16 {
    tags.veg.upper_of(s)
}

impl Tags {
    /// Installs the vegetation tables built by `VegTable::build` (the tables need the final palette).
    pub fn set_vegetation(&mut self, veg: VegTable) {
        self.veg = veg;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ported_names_are_classified() {
        let json: Value = serde_json::from_str(include_str!("../../assets/worldgen/ported_features.json")).unwrap();
        for name in json["simple_blocks"].as_array().unwrap() {
            let name = name.as_str().unwrap();
            assert!(kind_of_name(name).is_some(), "{name} is listed in simple_blocks but has no survival class");
        }
    }

    #[test]
    fn provider_names_walks_weighted_and_sources() {
        let v: Value = serde_json::from_str(
            r#"{"type":"minecraft:weighted","entries":[{"data":"minecraft:poppy","weight":2},
               {"data":{"type":"minecraft:randomized_int","property":"pickles","source":{"id":"minecraft:sea_pickle","properties":{"pickles":"1"}},"values":{"type":"minecraft:uniform","min_inclusive":1,"max_inclusive":4}},"weight":1}]}"#,
        )
        .unwrap();
        let mut out = Vec::new();
        provider_names(&v, &mut out).unwrap();
        assert_eq!(out, ["minecraft:poppy", "minecraft:sea_pickle"]);
    }
}
