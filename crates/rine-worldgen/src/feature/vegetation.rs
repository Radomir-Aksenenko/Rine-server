//! Vegetation survival (`canSurvive` / `mayPlaceOn`) for the blocks the simple block feature places, and the
//! per-state tables that feature reads at placement time.
//!
//! Every block is classified by name when the decorator is compiled (`kind_of_name`). A name outside that
//! table is a compile error, and `assets/worldgen/ported_features.json` (`simple_blocks`) lists the same names
//! for the allow rule, so a feature that places an unclassified block is never decorated.
//!
use serde_json::Value;

use super::region::{Region, Tags};
use super::state::{Fluid, Palette};

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
    SmallDripleaf,
    /// `SeaPickleBlock.mayPlaceOn`: the block below has a collision or sturdy UP face.
    SeaPickle,
    /// `SeagrassBlock.mayPlaceOn`: a sturdy UP face and not `cannot_support_seagrass`.
    Seagrass,
    TallSeagrass,
    Mushroom,
    SturdyGround,
    Unconditional,
    LilyPad,
    SporeBlossom,
    Cactus,
    SugarCane,
    Carpet,
    MossyCarpet,
    Tagged(usize),
}

/// Survival class of a block name, `None` for blocks the port does not classify.
pub fn kind_of_name(name: &str) -> Option<Survival> {
    Some(match name {
        "minecraft:tuff" | "minecraft:sculk_catalyst" => Survival::Unconditional,
        "minecraft:poppy" | "minecraft:dandelion" | "minecraft:short_grass" | "minecraft:fern"
        | "minecraft:allium" | "minecraft:azure_bluet" | "minecraft:blue_orchid" | "minecraft:cornflower"
        | "minecraft:lily_of_the_valley" | "minecraft:oxeye_daisy" | "minecraft:red_tulip" | "minecraft:pink_tulip"
        | "minecraft:orange_tulip" | "minecraft:white_tulip" | "minecraft:closed_eyeblossom" | "minecraft:open_eyeblossom"
        | "minecraft:pink_petals" | "minecraft:wildflowers" | "minecraft:sweet_berry_bush"
        | "minecraft:bush" | "minecraft:red_shrub" | "minecraft:firefly_bush" => Survival::Vegetation,
        "minecraft:tall_grass" | "minecraft:large_fern" | "minecraft:sunflower" | "minecraft:lilac" | "minecraft:rose_bush" | "minecraft:peony" => {
            Survival::DoublePlant
        }
        "minecraft:dead_bush" | "minecraft:short_dry_grass" | "minecraft:tall_dry_grass" => Survival::DryVegetation,
        "minecraft:sea_pickle" => Survival::SeaPickle,
        "minecraft:small_dripleaf" => Survival::SmallDripleaf,
        "minecraft:seagrass" => Survival::Seagrass,
        "minecraft:tall_seagrass" => Survival::TallSeagrass,
        "minecraft:brown_mushroom" | "minecraft:red_mushroom" => Survival::Mushroom,
        "minecraft:leaf_litter" => Survival::SturdyGround,
        "minecraft:melon" | "minecraft:pumpkin" => Survival::Unconditional,
        "minecraft:lily_pad" => Survival::LilyPad,
        "minecraft:spore_blossom" => Survival::SporeBlossom,
        "minecraft:cactus" => Survival::Cactus,
        "minecraft:sugar_cane" => Survival::SugarCane,
        "minecraft:moss_carpet" => Survival::Carpet,
        "minecraft:pale_moss_carpet" => Survival::MossyCarpet,
        "minecraft:potent_sulfur" | "minecraft:tube_coral_block" | "minecraft:brain_coral_block"
        | "minecraft:bubble_coral_block" | "minecraft:fire_coral_block" | "minecraft:horn_coral_block" => Survival::Unconditional,
        "minecraft:azalea" | "minecraft:flowering_azalea" => Survival::Tagged(0),
        "minecraft:crimson_fungus" => Survival::Tagged(1),
        "minecraft:warped_fungus" => Survival::Tagged(2),
        "minecraft:crimson_roots" => Survival::Tagged(3),
        "minecraft:warped_roots" => Survival::Tagged(4),
        "minecraft:nether_sprouts" => Survival::Tagged(5),
        _ => return None,
    })
}

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
        Some("minecraft:noise" | "minecraft:dual_noise") => {
            for state in v["states"].as_array().ok_or("states")? {
                provider_names(state, out)?;
            }
        }
        Some("minecraft:noise_threshold") => {
            provider_names(&v["default_state"], out)?;
            for list in ["low_states", "high_states"] {
                for state in v[list].as_array().ok_or("noise threshold states")? {
                    provider_names(state, out)?;
                }
            }
        }
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
    /// Per palette state: 0 for no half, 1 for `lower`, 2 for `upper` (`DoubleBlockHalf`).
    half: Vec<u8>,
    /// Per palette state of a double plant: the same state with `half=lower`.
    lower_of: Vec<u16>,
    /// Per palette state of a double plant: the same state with `half=upper`.
    upper_of: Vec<u16>,
    waterlogged: Vec<Option<[u16; 2]>>,
    pub(crate) snowy: Vec<Option<u16>>,
    tag_vegetation: u16,
    tag_dry: u16,
    tag_no_seagrass: u16,
    tag_small_dripleaf: Option<u16>,
    extra_tags: [Option<u16>; 6],
    mushroom_light_override: Option<u16>,
    lily_pad_support: Option<u16>,
    unstable_bottom_center: Option<u16>,
    cactus_support: Option<u16>,
    sugar_cane_support: Option<u16>,
    sugar_cane_adjacent: Option<u16>,
    pub(crate) moss_carpet: Option<super::moss_carpet::Carpet>,
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
            tag_small_dripleaf: tag("minecraft:supports_small_dripleaf").ok(),
            extra_tags: [
                "supports_azalea", "supports_crimson_fungus", "supports_warped_fungus",
                "supports_crimson_roots", "supports_warped_roots", "supports_nether_sprouts",
            ].map(|name| tag(&format!("minecraft:{name}")).ok()),
            mushroom_light_override: tag("minecraft:overrides_mushroom_light_requirement").ok(),
            lily_pad_support: tag("minecraft:supports_lily_pad").ok(),
            unstable_bottom_center: tag("minecraft:unstable_bottom_center").ok(),
            cactus_support: tag("minecraft:supports_cactus").ok(),
            sugar_cane_support: tag("minecraft:supports_sugar_cane").ok(),
            sugar_cane_adjacent: tag("minecraft:supports_sugar_cane_adjacently").ok(),
            ..VegTable::default()
        };

        // Both half variants of every double plant state exist in the palette before the tables are sized.
        let block_names: Vec<String> = (0..pal.block_count()).map(|b| pal.block_name(b as u16).to_string()).collect();
        if let Some(block) = pal.block_id("minecraft:pale_moss_carpet") {
            t.moss_carpet = Some(super::moss_carpet::Carpet::compile(pal, block));
        }
        let double_blocks: Vec<bool> = block_names.iter().map(|n| {
            matches!(kind_of_name(n), Some(Survival::DoublePlant | Survival::SmallDripleaf | Survival::TallSeagrass))
        }).collect();
        let mut state = 0;
        while state < pal.states().len() {
            let snowy = if pal.states()[state].contains("snowy=") {
                pal.with_property(state as u16, "snowy", "true")
            } else {
                None
            };
            t.snowy.push(snowy);
            if double_blocks[pal.block_of()[state] as usize] {
                let state = state as u16;
                pal.with_property(state, "half", "lower").ok_or("double plant without half")?;
                pal.with_property(state, "half", "upper").ok_or("double plant without half")?;
                if pal.block_id("minecraft:small_dripleaf") == Some(pal.block_of()[state as usize]) {
                    pal.with_property(state, "waterlogged", "true");
                    pal.with_property(state, "waterlogged", "false");
                }
            }
            state += 1;
        }

        let n_states = pal.states().len();
        t.waterlogged.resize(n_states, None);
        if let Some(dripleaf) = pal.block_id("minecraft:small_dripleaf") {
            for s in 0..n_states {
                let s = s as u16;
                if pal.block_of()[s as usize] == dripleaf {
                    t.waterlogged[s as usize] = Some([
                        pal.with_property(s, "waterlogged", "false").ok_or("small dripleaf waterlogged")?,
                        pal.with_property(s, "waterlogged", "true").ok_or("small dripleaf waterlogged")?,
                    ]);
                }
            }
        }
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
        }
        Ok(t)
    }

    /// Survival class of a palette state's block.
    #[inline]
    pub fn kind_of_block(&self, block: u16) -> Survival {
        self.kind.get(block as usize).copied().flatten().unwrap_or(Survival::Vegetation)
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
    // Direction.Plane.HORIZONTAL, not Direction.values().
    const HORIZONTAL: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];
    let tags = region.tags;
    let at = |dx, dy, dz| region.get_block(x + dx, y + dy, z + dz);
    let below = || at(0, -1, 0);
    let has = |state, tag: Option<u16>| tag.is_some_and(|tag| tags.has(state, tag));
    let v = &tags.veg;
    let block = tags.block_of(s);
    match v.kind_of_block(block) {
        Survival::Vegetation => tags.has(below(), v.tag_vegetation),
        Survival::DryVegetation => tags.has(below(), v.tag_dry),
        kind @ (Survival::DoublePlant | Survival::SmallDripleaf) => {
            let below = below();
            if v.half(s) == 2 {
                // DoublePlantBlock.canSurvive: the upper half needs this block's lower half below.
                tags.block_of(below) == block && v.half(below) == 1
            } else if kind == Survival::SmallDripleaf {
                v.tag_small_dripleaf.is_some_and(|tag| tags.has(below, tag))
                    || tags.fluid(at(0, 0, 0)) == Fluid::Water && tags.has(below, v.tag_vegetation)
            } else {
                tags.has(below, v.tag_vegetation)
            }
        }
        Survival::SeaPickle => {
            let below = below();
            tags.collision_top(below) || tags.sturdy(below, 1)
        }
        Survival::SturdyGround => tags.sturdy(below(), 1),
        Survival::Seagrass => {
            let below = below();
            tags.sturdy(below, 1) && !tags.has(below, v.tag_no_seagrass)
        }
        Survival::TallSeagrass => {
            if v.half(s) == 2 {
                let below = below();
                return tags.is_block(below, block) && v.half(below) == 1;
            }
            let here = at(0, 0, 0);
            let below = below();
            tags.sturdy(below, 1) && !tags.has(below, v.tag_no_seagrass)
                && tags.fluid(here).is_water() && tags.fluid_full(here)
        }
        Survival::Mushroom => {
            let below = below();
            has(below, v.mushroom_light_override)
                || region.feature_raw_brightness((x, y, z)) < 13 && tags.solid_render(below)
        }
        Survival::Unconditional => true,
        Survival::LilyPad => {
            let below = below();
            let above = at(0, 0, 0);
            (tags.fluid(below) == Fluid::Water || has(below, v.lily_pad_support)) && tags.fluid(above) == Fluid::Empty
        }
        Survival::SporeBlossom => {
            let above = at(0, 1, 0);
            !has(above, v.unstable_bottom_center) && tags.sturdy_down_center(above)
                && !tags.fluid(at(0, 0, 0)).is_water()
        }
        Survival::Cactus => {
            for (dx, dz) in HORIZONTAL {
                let side = at(dx, 0, dz);
                if tags.solid(side) || matches!(tags.fluid(side), Fluid::Lava | Fluid::FlowingLava) {
                    return false;
                }
            }
            let below = below();
            (tags.is_block(below, block) || has(below, v.cactus_support)) && !tags.liquid(at(0, 1, 0))
        }
        Survival::SugarCane => {
            let below = below();
            tags.is_block(below, block) || has(below, v.sugar_cane_support) && HORIZONTAL.iter().any(|&(dx, dz)| {
                let adjacent = at(dx, -1, dz);
                tags.fluid(adjacent).is_water() || has(adjacent, v.sugar_cane_adjacent)
            })
        }
        Survival::Carpet => !tags.is_air(below()),
        Survival::MossyCarpet => v.moss_carpet.as_ref().is_some_and(|carpet| carpet.can_survive(tags, s, below())),
        Survival::Tagged(i) => has(below(), v.extra_tags[i]),
    }
}

/// Whether `s` is a double plant state (`Survival::DoublePlant`), which places two halves.
pub fn is_double_plant(tags: &Tags, s: u16) -> bool {
    matches!(tags.veg.kind_of_block(tags.block_of(s)), Survival::DoublePlant | Survival::SmallDripleaf | Survival::TallSeagrass)
}

pub fn copy_waterlogged(tags: &Tags, state: u16, previous: impl FnOnce() -> u16) -> u16 {
    tags.veg.waterlogged[state as usize].map_or(state, |variants| {
        variants[usize::from(tags.fluid(previous()).is_water())]
    })
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
