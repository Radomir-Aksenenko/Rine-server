//! Phase 4: vanilla biome decoraion (`ChunkGenerator.addVanillaDecorations` + `FeaturePlacer`).
//!
//! Data comes from `assets/worldgen/features.json` (`tools/noise-golden/bundle_features.js`): biome
//! feature lists, placed features and configured features reached from the overworld biome list.
//! `allow` filters the top-level placed features by id before anything else, so a golden can run vanilla
//! with the same filtered biome lists (a datapack with the same `features` arrays).
//!
//! Compilation is recursive: a placed feature's configured feature may reference more placed features
//! (selectors, sequences), which are compiled into the same table. Every configured type and placement
//! modifier outside the ported set is an error, so an unported feature is never skipped silently.
//!
//! Ported configured features: `ore`, `disk`, `spring_feature`, `block_column`, `blue_ice`, `huge_*_mushroom`,
//! `random_boolean_selector`, `simple_random_selector`, `weighted_random_selector`, `sequence`.
//!
//! Ordering: `sorter` reproduces `FeatureSorter`; within a chunk the features of each step run in the
//! sorted order, skipping features that no biome present in the 3x3 chunks has in that step.

pub mod allow;
pub mod column;
pub mod disk;
pub mod heightmap;
pub mod huge_mushroom;
pub mod ice;
pub mod ore;
pub mod placement;
pub mod predicate;
pub mod region;
pub mod rng;
pub mod sorter;
pub mod spring;
pub mod state;
pub mod state_provider;
pub mod tree;
pub mod simple_block;
pub mod vegetation;
mod huge_fungus;
mod moss_carpet;
mod shape;
mod vegetation_patch;
#[cfg(test)]
mod samples;

use std::collections::HashMap;

use serde_json::Value;

use crate::overworld::{ChunkBlocks, Overworld};
use column::{direction_of, BlockColumn, Layer};
use heightmap::WgHeights;
use disk::Disk;
use ice::BlueIce;
use ore::{OreFeature, RuleTest, Target};
use placement::{parse_int_provider, parse_modifier, Modifier};
use region::{Region, Tags};
use rng::WorldgenRng;
use spring::Spring;
use state::Palette;

const FEATURES_JSON: &str = include_str!("../../assets/worldgen/features.json");

/// `GenerationStep.Decoration` count; vanilla always runs at least this many steps.
const DECORATION_STEPS: usize = 11;

enum Configured {
    Unported,
    Ore(OreFeature),
    Disk(Disk),
    Spring(Spring),
    BlockColumn(BlockColumn),
    BlueIce(BlueIce),
    /// `RandomBooleanSelectorFeature`: a placed-feature id per branch.
    RandomBoolean { on_true: usize, on_false: usize },
    RandomSelector { entries: Vec<(usize, f32)>, default: usize },
    /// `SimpleRandomSelectorFeature`: uniform over the list.
    SimpleRandom(Vec<usize>),
    /// `WeightedRandomSelectorFeature`: `(placed id, weight)`; `total` > 0.
    WeightedRandom { entries: Vec<(usize, i32)>, total: i32 },
    /// `SequenceFeature`: all placed features in order, stopping at the first failure.
    Sequence(Vec<usize>),
    /// `SimpleBlockFeature` (`minecraft:simple_block`).
    SimpleBlock(simple_block::SimpleBlock),
    /// `minecraft:tree`.
    Tree(tree::TreeFeature),
    /// `minecraft:fallen_tree`.
    FallenTree(tree::fallen::FallenTree),
    /// `minecraft:huge_red_mushroom` / `minecraft:huge_brown_mushroom`.
    HugeMushroom(huge_mushroom::HugeMushroom),
    HugeFungus(huge_fungus::HugeFungus),
    VegetationPatch(vegetation_patch::VegetationPatch),
    Overlay(Vec<usize>),
    NoOp,
    RandomNeighborSpread {
        provider: state_provider::StateProvider,
        neighbors: predicate::BlockPredicate,
        replaceable: predicate::BlockPredicate,
        attempts: placement::IntProvider,
        xz: placement::IntProvider,
        y: placement::IntProvider,
    },
}

struct PlacedFeature {
    modifiers: Vec<Modifier>,
    feature: Configured,
}

/// Compiled decoration for one overworld generator. Immutable after construction: `Sync`.
pub struct Decorator {
    palette: Vec<String>,
    tags: Tags,
    placed: Vec<PlacedFeature>,
    /// Per step, placed-feature ids in `FeatureSorter` order.
    steps: Vec<Vec<usize>>,
    /// `placed_in_biome[biome][placed]`: `Biome.hasFeature` (any step).
    placed_in_biome: Vec<Vec<bool>>,
    /// `in_step[biome][step][placed]`: the feature is in that biome's list for that step.
    in_step: Vec<Vec<Vec<bool>>>,
    min_y: i32,
    height: i32,
    /// Block states, tags and special ids of the tree port; `Some` when a tree was compiled.
    tree_env: Option<tree::TreeEnv>,
    /// Grower decorators (`Decorator::new_growers`): configured feature name -> placed index.
    grower_ids: HashMap<String, usize>,
    bone_meal_flowers: Vec<Vec<usize>>,
    pale_moss_patch: Option<usize>,
    waterlogged: Vec<Option<u16>>,
    level_random: Option<crate::random::Positional>,
    /// Top-level placed features `Decorator::vanilla` could not compile (`name: error`), kept as no-ops.
    skipped: Vec<String>,
}

fn bundle() -> &'static Value {
    static BUNDLE: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    BUNDLE.get_or_init(|| serde_json::from_str(FEATURES_JSON).expect("bundled features.json"))
}

fn parse_predicate(v: &Value, tag_names: &[String]) -> Result<RuleTest, String> {
    let ty = v["predicate_type"].as_str().ok_or_else(|| format!("predicate without type: {v}"))?;
    match ty {
        "minecraft:tag_match" => {
            let name = v["tag"].as_str().ok_or("tag_match without tag")?;
            let id = tag_names.binary_search_by(|n| n.as_str().cmp(name)).map_err(|_| format!("tag {name} not bundled"))?;
            Ok(RuleTest::TagMatch(id as u16))
        }
        "minecraft:any_of" | "minecraft:all_of" => {
            let rules = v["rules"]
                .as_array()
                .ok_or("rules")?
                .iter()
                .map(|r| parse_predicate(r, tag_names))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(if ty == "minecraft:any_of" { RuleTest::AnyOf(rules) } else { RuleTest::AllOf(rules) })
        }
        "minecraft:not" => Ok(RuleTest::Not(Box::new(parse_predicate(&v["rule"], tag_names)?))),
        "minecraft:height_match" => Ok(RuleTest::HeightMatch {
            min: v["min_inclusive"].as_i64().ok_or("min_inclusive")? as i32,
            max: v["max_inclusive"].as_i64().ok_or("max_inclusive")? as i32,
        }),
        other => Err(format!("unsupported rule test {other}")),
    }
}

/// Compiles the JSON of the decorations into `placed`, interning block states into `pal`.
struct Compiler<'a> {
    bundle: &'a Value,
    pal: Palette,
    tag_names: Vec<String>,
    sea_level: i32,
    placed: Vec<PlacedFeature>,
    /// Named placed features (`minecraft:...`), by name.
    named: HashMap<String, usize>,
    /// Inline placed features (`{feature, placement}` objects), by their JSON text.
    inline: HashMap<String, usize>,
    /// Tree block schemas, built on the first tree configured feature.
    tree_states: Option<tree::state::TreeStates>,
    pale_moss_patch: Option<usize>,
}

impl<'a> Compiler<'a> {
    fn configured_named(&mut self, name: &str) -> Result<usize, String> {
        self.placed_from_parts(&Value::String(name.to_owned()), &Value::Array(Vec::new()), name)
    }

    /// Compiles a named placed feature once; returns its id.
    fn placed_named(&mut self, name: &str) -> Result<usize, String> {
        if let Some(&i) = self.named.get(name) {
            return Ok(i);
        }
        let bundle = self.bundle;
        let pj = &bundle["placed"][name];
        if pj.is_null() {
            return Err(format!("placed feature {name} not bundled"));
        }
        let i = self.placed_from_parts(&pj["feature"], &pj["placement"], name)?;
        self.named.insert(name.to_string(), i);
        Ok(i)
    }

    /// A placed-feature reference inside a configured feature: a name or an inline `{feature, placement}`.
    fn placed_ref(&mut self, v: &Value) -> Result<usize, String> {
        if let Some(name) = v.as_str() {
            return self.placed_named(name);
        }
        let key = v.to_string();
        if let Some(&i) = self.inline.get(&key) {
            return Ok(i);
        }
        let i = self.placed_from_parts(&v["feature"], &v["placement"], "inline")?;
        self.inline.insert(key, i);
        Ok(i)
    }

    fn placed_from_parts(&mut self, feature: &Value, placement: &Value, label: &str) -> Result<usize, String> {
        let feature = self.configured(feature).map_err(|e| format!("{label}: {e}"))?;
        let list = placement.as_array().ok_or_else(|| format!("{label}: placement list missing"))?;
        let mut modifiers = Vec::with_capacity(list.len());
        for m in list {
            let modifier = match m["type"].as_str() {
                Some("minecraft:block_predicate_filter") => {
                    Modifier::BlockPredicate(predicate::parse(&m["predicate"], &self.tag_names, &mut self.pal).map_err(|e| format!("{label}: {e}"))?)
                }
                Some("minecraft:environment_scan") => {
                    let dy = match m["direction_of_search"].as_str() {
                        Some("up") => 1,
                        Some("down") => -1,
                        _ => return Err(format!("{label}: invalid environment scan direction")),
                    };
                    Modifier::EnvironmentScan {
                        dy,
                        target: predicate::parse(&m["target_condition"], &self.tag_names, &mut self.pal)?,
                        allowed: match m.get("allowed_search_condition") {
                            Some(pred) => predicate::parse(pred, &self.tag_names, &mut self.pal)?,
                            None => predicate::BlockPredicate::True,
                        },
                        steps: m["max_steps"].as_i64().ok_or("environment scan steps")? as i32,
                    }
                }
                Some("minecraft:count_on_every_layer") => Modifier::CountOnEveryLayer {
                    count: parse_int_provider(&m["count"])?,
                    empty_blocks: [self.pal.block("minecraft:water"), self.pal.block("minecraft:lava")],
                    bedrock: self.pal.block("minecraft:bedrock"),
                },
                _ => parse_modifier(m).map_err(|e| format!("{label}: {e}"))?,
            };
            modifiers.push(modifier);
        }
        self.placed.push(PlacedFeature { modifiers, feature });
        Ok(self.placed.len() - 1)
    }

    /// Builds the tree block schemas on first use, so decorations without trees keep their palette.
    fn ensure_tree_states(&mut self) -> Result<(), String> {
        if self.tree_states.is_none() {
            let wanted = tree::state::tree_wanted_blocks(self.bundle);
            self.tree_states = Some(tree::state::TreeStates::build(&self.bundle["blockProps"], &wanted, &mut self.pal)?);
        }
        Ok(())
    }

    /// Configured feature from a name or an inline object.
    fn configured(&mut self, v: &Value) -> Result<Configured, String> {
        let bundle = self.bundle;
        let cfg = match v.as_str() {
            Some(name) => {
                let c = &bundle["configured"][name];
                if c.is_null() {
                    return Err(format!("configured feature {name} not bundled"));
                }
                c
            }
            None => v,
        };
        let ty = cfg["type"].as_str().ok_or_else(|| format!("configured feature without type: {cfg}"))?;
        let tn = &self.tag_names;
        match ty {
            "minecraft:ore" => Ok(Configured::Ore(self.ore(cfg)?)),
            "minecraft:disk" => Ok(Configured::Disk(Disk {
                state: state_provider::parse(&cfg["state_provider"], tn, &mut self.pal)?,
                target: predicate::parse(&cfg["target"], tn, &mut self.pal)?,
                radius: parse_int_provider(&cfg["radius"])?,
                half_height: cfg["half_height"].as_i64().ok_or("half_height")? as i32,
            })),
            "minecraft:spring_feature" => {
                let fluid = cfg["state"]["id"].as_str().ok_or("spring state")?;
                let state = spring::legacy_state(fluid, &mut self.pal);
                let mut blocks = Vec::new();
                for b in cfg["valid_blocks"].as_array().ok_or("valid_blocks")? {
                    blocks.push(self.pal.block(b.as_str().ok_or("valid block name")?));
                }
                Ok(Configured::Spring(Spring {
                    state,
                    requires_block_below: cfg["requires_block_below"].as_bool().unwrap_or(true),
                    rock_count: cfg["rock_count"].as_i64().unwrap_or(4) as i32,
                    hole_count: cfg["hole_count"].as_i64().unwrap_or(1) as i32,
                    valid: predicate::BlockPredicate::MatchingBlocks { offset: [0; 3], blocks },
                }))
            }
            "minecraft:block_column" => {
                let mut layers = Vec::new();
                for l in cfg["layers"].as_array().ok_or("layers")? {
                    layers.push(Layer {
                        height: parse_int_provider(&l["height"])?,
                        state: state_provider::parse(provider_value(bundle, &l["provider"]), tn, &mut self.pal)?,
                    });
                }
                Ok(Configured::BlockColumn(BlockColumn {
                    layers,
                    direction: direction_of(cfg["direction"].as_str().ok_or("direction")?)?,
                    allowed: predicate::parse(&cfg["allowed_placement"], tn, &mut self.pal)?,
                    prioritize_tip: cfg["prioritize_tip"].as_bool().unwrap_or(false),
                }))
            }
            "minecraft:blue_ice" => Ok(Configured::BlueIce(BlueIce {
                sea_level: self.sea_level,
                blue_ice: self.pal.default_state("minecraft:blue_ice"),
                water_block: self.pal.block("minecraft:water"),
                packed_block: self.pal.block("minecraft:packed_ice"),
                ice_block: self.pal.block("minecraft:ice"),
                blue_block: self.pal.block("minecraft:blue_ice"),
            })),
            "minecraft:random_boolean_selector" => Ok(Configured::RandomBoolean {
                on_true: self.placed_ref(&cfg["feature_true"])?,
                on_false: self.placed_ref(&cfg["feature_false"])?,
            }),
            "minecraft:random_selector" => {
                let mut entries = Vec::new();
                for entry in cfg["features"].as_array().ok_or("selector features")? {
                    entries.push((self.placed_ref(&entry["feature"])?, entry["chance"].as_f64().ok_or("selector chance")? as f32));
                }
                Ok(Configured::RandomSelector { entries, default: self.placed_ref(&cfg["default"])? })
            }
            "minecraft:simple_random_selector" => {
                let mut list = Vec::new();
                for f in cfg["features"].as_array().ok_or("features")? {
                    list.push(self.placed_ref(f)?);
                }
                if list.is_empty() {
                    return Err("simple_random_selector without features".into());
                }
                Ok(Configured::SimpleRandom(list))
            }
            "minecraft:weighted_random_selector" => {
                let mut entries = Vec::new();
                for e in cfg["features"].as_array().ok_or("features")? {
                    let weight = e["weight"].as_i64().ok_or("weight")? as i32;
                    entries.push((self.placed_ref(&e["data"])?, weight));
                }
                let total: i32 = entries.iter().map(|e| e.1).sum();
                if total <= 0 {
                    return Err("weighted_random_selector without weight".into());
                }
                Ok(Configured::WeightedRandom { entries, total })
            }
            "minecraft:simple_block" => {
                let provider = provider_value(bundle, &cfg["to_place"]);
                let mut names = Vec::new();
                vegetation::provider_names(provider, &mut names)?;
                for n in &names {
                    if vegetation::kind_of_name(n).is_none() {
                        return Err(format!("simple_block places {n}, which is not ported"));
                    }
                }
                Ok(Configured::SimpleBlock(simple_block::SimpleBlock {
                    to_place: state_provider::parse(provider, tn, &mut self.pal)?,
                    schedule_tick: cfg["schedule_tick"].as_bool().unwrap_or(false),
                }))
            }
            "minecraft:no_op" => Ok(Configured::NoOp),
            "minecraft:overlay" => {
                let mut list = Vec::new();
                if let Some(features) = cfg["features"].as_array() {
                    for feature in features {
                        list.push(self.placed_ref(feature)?);
                    }
                } else {
                    list.push(self.placed_ref(&cfg["features"])?);
                }
                Ok(Configured::Overlay(list))
            }
            "minecraft:vegetation_patch" | "minecraft:waterlogged_vegetation_patch" => {
                let ground = state_provider::parse(provider_value(bundle, &cfg["ground_state"]), tn, &mut self.pal)?;
                let replaceable = block_set(&cfg["replaceable"], tn, &mut self.pal)?;
                let vegetation = self.placed_ref(&cfg["vegetation_feature"])?;
                let inward = match cfg["surface"].as_str() {
                    Some("floor") => -1,
                    Some("ceiling") => 1,
                    _ => return Err("invalid cave surface".into()),
                };
                let probability = |key: &str| cfg[key].as_f64().map(|p| p as f32).ok_or_else(|| key.to_owned());
                let water = (ty == "minecraft:waterlogged_vegetation_patch").then(|| self.pal.default_state("minecraft:water"));
                Ok(Configured::VegetationPatch(vegetation_patch::VegetationPatch {
                    ground, replaceable, vegetation, inward, water,
                    depth: parse_int_provider(&cfg["depth"])?,
                    extra_bottom: probability("extra_bottom_block_chance")?,
                    vertical_range: cfg["vertical_range"].as_i64().ok_or("vertical_range")? as i32,
                    vegetation_chance: probability("vegetation_chance")?,
                    radius: parse_int_provider(&cfg["xz_radius"])?,
                    extra_edge: probability("extra_edge_column_chance")?,
                }))
            }
            "minecraft:random_neighbor_spread" => Ok(Configured::RandomNeighborSpread {
                provider: state_provider::parse(provider_value(bundle, &cfg["block"]), tn, &mut self.pal)?,
                neighbors: block_set(&cfg["accepted_neighbors"], tn, &mut self.pal)?,
                replaceable: predicate::parse(&cfg["can_replace"], tn, &mut self.pal)?,
                attempts: parse_int_provider(&cfg["attempts"])?,
                xz: parse_int_provider(&cfg["xz_offset"])?,
                y: parse_int_provider(&cfg["y_offset"])?,
            }),
            "minecraft:huge_fungus" => {
                let base = state_provider::state_of(&cfg["valid_base_block"], &mut self.pal)?;
                let hat = state_provider::state_of(&cfg["hat_state"], &mut self.pal)?;
                let wart = self.pal.block("minecraft:nether_wart_block");
                let vines = (self.pal.block_of()[hat as usize] == wart).then(|| {
                    let body = self.pal.default_state("minecraft:weeping_vines_plant");
                    let heads = ["23", "24", "25"].map(|age| self.pal.state_from_parts("minecraft:weeping_vines", &[("age", age)]));
                    (body, heads)
                });
                Ok(Configured::HugeFungus(huge_fungus::HugeFungus {
                    base: self.pal.block_of()[base as usize],
                    stem: state_provider::state_of(&cfg["stem_state"], &mut self.pal)?,
                    hat, vines,
                    decor: state_provider::state_of(&cfg["decor_state"], &mut self.pal)?,
                    air: self.pal.default_state("minecraft:air"),
                    replaceable: predicate::parse(&cfg["replaceable_blocks"], tn, &mut self.pal)?,
                    planted: cfg["planted"].as_bool().unwrap_or(false),
                }))
            }
            "minecraft:sequence" => {
                let mut list = Vec::new();
                for f in cfg["features"].as_array().ok_or("features")? {
                    list.push(self.placed_ref(f)?);
                }
                Ok(Configured::Sequence(list))
            }
            "minecraft:huge_red_mushroom" => Ok(Configured::HugeMushroom(huge_mushroom::HugeMushroom::compile(
                huge_mushroom::Kind::Red,
                cfg,
                tn,
                &mut self.pal,
            )?)),
            "minecraft:huge_brown_mushroom" => Ok(Configured::HugeMushroom(huge_mushroom::HugeMushroom::compile(
                huge_mushroom::Kind::Brown,
                cfg,
                tn,
                &mut self.pal,
            )?)),
            "minecraft:tree" | "minecraft:fallen_tree" => {
                self.ensure_tree_states()?;
                if self.pale_moss_patch.is_none() && cfg["decorators"].as_array().is_some_and(|decorators| {
                    decorators.iter().any(|d| d["type"] == "minecraft:pale_moss")
                }) {
                    self.pale_moss_patch = Some(self.configured_named("minecraft:pale_moss_patch")?);
                }
                let states = self.tree_states.as_ref().expect("tree states");
                let mut tc = tree::TreeCompile { states, pal: &mut self.pal, tag_names: &self.tag_names, providers: &bundle["blockStateProviders"] };
                if ty == "minecraft:tree" {
                    Ok(Configured::Tree(tree::TreeFeature::compile(cfg, &mut tc).map_err(|e| format!("tree: {e}"))?))
                } else {
                    Ok(Configured::FallenTree(tree::fallen::FallenTree::compile(cfg, &mut tc)?))
                }
            }
            other => Err(format!("configured feature {other} is not ported")),
        }
    }

    fn ore(&mut self, cfg: &Value) -> Result<OreFeature, String> {
        let mut targets = Vec::new();
        for t in cfg["targets"].as_array().ok_or("targets")? {
            let test = parse_predicate(&t["target"], &self.tag_names)?;
            let state = state_provider::state_of(&t["state"], &mut self.pal)?;
            targets.push(Target { test, state });
        }
        Ok(OreFeature {
            targets,
            size: cfg["size"].as_i64().ok_or("size")? as i32,
            discard_chance_on_air_exposure: cfg["discard_chance_on_air_exposure"].as_f64().unwrap_or(0.0) as f32,
        })
    }
}

fn provider_value<'a>(bundle: &'a Value, value: &'a Value) -> &'a Value {
    value.as_str().and_then(|name| bundle["blockStateProviders"].get(name)).unwrap_or(value)
}

fn block_set(value: &Value, tags: &[String], pal: &mut Palette) -> Result<predicate::BlockPredicate, String> {
    let v = if let Some(tag) = value.as_str().and_then(|s| s.strip_prefix('#')) {
        serde_json::json!({"type": "minecraft:matching_block_tag", "tag": tag})
    } else {
        serde_json::json!({"type": "minecraft:matching_blocks", "blocks": value})
    };
    predicate::parse(&v, tags, pal)
}

impl Decorator {
    /// Compiles the decoration of `o` for the top-level placed features `allow` accepts.
    pub fn new(o: &Overworld, allow: &dyn Fn(&str) -> bool) -> Result<Self, String> {
        Self::new_from(bundle(), o, allow)
    }

    /// Like [`Decorator::new`] with an explicit feature bundle (`features.json` layout). The tree golden bundles
    /// synthetic placed features with matching biome lists (`tools/noise-golden/make_tree_golden.js`).
    pub fn new_from(bundle: &Value, o: &Overworld, allow: &dyn Fn(&str) -> bool) -> Result<Self, String> {
        Self::compile_decoration(bundle, o, allow, false, false)
    }

    pub(crate) fn production(o: &Overworld, allow: &dyn Fn(&str) -> bool) -> Result<Self, String> {
        Self::compile_decoration(bundle(), o, allow, true, false)
    }

    /// The full vanilla decoration: every placed feature of the biome lists. A feature that does not compile
    /// (an unported type) becomes a no-op in its sorted slot and is listed by [`Decorator::skipped`]; the
    /// other features keep their vanilla order and seeds.
    pub(crate) fn vanilla(o: &Overworld) -> Result<Self, String> {
        Self::compile_decoration(bundle(), o, &|_| true, true, true)
    }

    /// Placed features [`Decorator::vanilla`] left out, as `name: error`.
    pub fn skipped(&self) -> &[String] {
        &self.skipped
    }

    fn compile_decoration(bundle: &Value, o: &Overworld, allow: &dyn Fn(&str) -> bool, full_order: bool, lenient: bool) -> Result<Self, String> {
        let biome_names: Vec<String> = o.biome_names().to_vec();
        let block_tags: HashMap<String, Vec<String>> = bundle["blockTags"]
            .as_object()
            .ok_or("blockTags")?
            .iter()
            .map(|(k, v)| (k.clone(), v.as_array().unwrap().iter().map(|b| b.as_str().unwrap().to_string()).collect()))
            .collect();

        let mut c = Compiler {
            bundle,
            pal: Palette::new(o.interner().names(), o.interner().defaults().clone()),
            tag_names: region::tag_names(&block_tags),
            sea_level: o.sea_level(),
            placed: Vec::new(),
            named: HashMap::new(),
            inline: HashMap::new(),
            tree_states: None,
            pale_moss_patch: None,
        };

        // Production preserves FeatureSorter indices even when a feature cannot yet execute.
        let mut excluded = HashMap::new();
        let mut failed: std::collections::BTreeMap<&str, String> = std::collections::BTreeMap::new();
        let mut biome_lists: Vec<Vec<Vec<usize>>> = Vec::with_capacity(biome_names.len());
        for name in &biome_names {
            let steps = bundle["biomes"][name].as_array().ok_or_else(|| format!("biome {name} not bundled"))?;
            let mut lists = Vec::with_capacity(steps.len());
            for step in steps {
                let mut ids = Vec::new();
                for f in step.as_array().unwrap() {
                    let f = f.as_str().unwrap();
                    let compiled = if !allow(f) {
                        None
                    } else if lenient {
                        if failed.contains_key(f) {
                            None
                        } else {
                            match c.placed_named(f) {
                                Ok(id) => Some(id),
                                Err(e) => {
                                    failed.insert(f, e);
                                    None
                                }
                            }
                        }
                    } else {
                        Some(c.placed_named(f)?)
                    };
                    if let Some(id) = compiled {
                        ids.push(id);
                    } else if full_order {
                        let index = *excluded.entry(f).or_insert_with(|| {
                            let index = c.placed.len();
                            c.placed.push(PlacedFeature { modifiers: Vec::new(), feature: Configured::Unported });
                            index
                        });
                        ids.push(index);
                    }
                }
                lists.push(ids);
            }
            biome_lists.push(lists);
        }

        // Feature order per step (FeatureSorter over the filtered biome lists).
        let steps = sorter::build_features_per_step(&biome_lists)?;

        // Biome membership tables.
        let placed_count = c.placed.len();
        let mut placed_in_biome = vec![vec![false; placed_count]; biome_names.len()];
        let mut in_step = vec![vec![vec![false; placed_count]; DECORATION_STEPS.max(steps.len())]; biome_names.len()];
        for (b, lists) in biome_lists.iter().enumerate() {
            for (s, ids) in lists.iter().enumerate() {
                for &p in ids {
                    placed_in_biome[b][p] = true;
                    in_step[b][s][p] = true;
                }
            }
        }

        let mut dec = finish(c, &block_tags, o.min_y(), o.height(), steps, placed_in_biome, in_step, HashMap::new())?;
        dec.level_random = Some(o.root_random().from_hash_of("minecraft:worldgen_region_random").fork_positional());
        dec.skipped = failed.into_iter().map(|(name, e)| format!("{name}: {e}")).collect();
        Ok(dec)
    }

    /// Compiles only the named configured features (`minecraft:oak`, ...) for single-feature placement on
    /// the live world. The palette is `palette_names` in the given order (the live state ids, so a live
    /// state id is a palette index), `defaults` maps block names to their default state strings.
    /// Features are placed with `place_grower`; they run in the overworld height range.
    pub fn new_growers(palette_names: &[String], defaults: HashMap<String, String>, growers: &[&str]) -> Result<Self, String> {
        Self::new_live_features(palette_names, defaults, growers, None)
    }

    pub fn new_bone_meal(palette_names: &[String], defaults: HashMap<String, String>, growers: &[&str], biome_names: &[String]) -> Result<Self, String> {
        Self::new_live_features(palette_names, defaults, growers, Some(biome_names))
    }

    fn new_live_features(
        palette_names: &[String], defaults: HashMap<String, String>, growers: &[&str],
        biome_names: Option<&[String]>,
    ) -> Result<Self, String> {
        let bundle = bundle();
        let block_tags: HashMap<String, Vec<String>> = bundle["blockTags"]
            .as_object()
            .ok_or("blockTags")?
            .iter()
            .map(|(k, v)| (k.clone(), v.as_array().unwrap().iter().map(|b| b.as_str().unwrap().to_string()).collect()))
            .collect();
        let mut c = Compiler {
            bundle,
            pal: Palette::new(palette_names, defaults),
            tag_names: region::tag_names(&block_tags),
            sea_level: 63,
            placed: Vec::new(),
            named: HashMap::new(),
            inline: HashMap::new(),
            tree_states: None,
            pale_moss_patch: None,
        };
        let mut grower_ids = HashMap::new();
        for &g in growers {
            grower_ids.insert(g.to_string(), c.configured_named(g)?);
        }
        let mut bone_meal_flowers = Vec::new();
        if let Some(biomes) = biome_names {
            let name = "minecraft:grass_bonemeal";
            grower_ids.insert(name.to_owned(), c.placed_named(name)?);
            for biome in biomes {
                let mut choices = Vec::new();
                if let Some(features) = bundle["boneMealFeatures"][biome].as_array() {
                    for feature in features {
                        let name = feature.as_str().ok_or("bone meal feature name")?;
                        let index = match grower_ids.get(name) {
                            Some(&index) => index,
                            None => {
                                let index = c.configured_named(name)?;
                                grower_ids.insert(name.to_owned(), index);
                                index
                            }
                        };
                        choices.push(index);
                    }
                }
                bone_meal_flowers.push(choices);
            }
        }
        let mut out = finish(c, &block_tags, OVERWORLD_MIN_Y, OVERWORLD_HEIGHT, Vec::new(), Vec::new(), Vec::new(), grower_ids)?;
        out.bone_meal_flowers = bone_meal_flowers;
        Ok(out)
    }

    pub fn bone_meal_flowers(&self, biome: u8) -> &[usize] {
        self.bone_meal_flowers.get(biome as usize).map_or(&[], Vec::as_slice)
    }

    pub fn moss_carpet_topper(&self, region: &mut Region, pos: region::Pos, state: u16) -> Option<u16> {
        let carpet = self.tags.veg.moss_carpet.as_ref()?;
        if !carpet.is_base(state) { return None; }
        carpet.topper(region, pos, None)
    }

    pub fn moss_carpet_update(&self, region: &Region, pos: region::Pos, state: u16, air: u16) -> u16 {
        self.tags.veg.moss_carpet.as_ref().map_or(state, |carpet| carpet.update_shape(region, pos, state, air))
    }

    /// Whether a grower of that name was compiled.
    pub fn has_grower(&self, name: &str) -> bool {
        self.grower_ids.contains_key(name)
    }

    /// Index of a compiled grower, for `place_grower_at` (resolve once, place many times).
    pub fn grower_index(&self, name: &str) -> Option<usize> {
        self.grower_ids.get(name).copied()
    }

    /// `foliageRadius()` of a compiled huge mushroom grower.
    pub fn grower_mushroom_radius(&self, name: &str) -> Option<i32> {
        match &self.placed[*self.grower_ids.get(name)?].feature {
            Configured::HugeMushroom(m) => Some(m.foliage_radius()),
            _ => None,
        }
    }

    /// `TreeGrower.getMinimumHeight` input: the trunk base height of a compiled tree grower.
    pub fn grower_base_height(&self, name: &str) -> Option<i32> {
        match &self.placed[*self.grower_ids.get(name)?].feature {
            Configured::Tree(t) => Some(t.trunk.base_height),
            _ => None,
        }
    }

    /// Block tags and block-state predicates of this palette.
    pub fn tags(&self) -> &Tags {
        &self.tags
    }

    /// A region for `place_grower` over the nine chunks around `(cx, cz)` in raster order (see
    /// `Region::for_growers`). Blocks are palette ids of this decorator's palette.
    pub fn grower_region(&self, cx: i32, cz: i32, blocks: Vec<Vec<u16>>) -> Region<'_> {
        Region::for_growers(&self.tags, self.min_y, self.height, cx, cz, blocks)
    }

    /// Places a compiled grower (`ConfiguredFeature.place`) at `pos`, returns the result of `TreeFeature.place`
    /// (or the feature's result). Writes go to the region; start a log first to read them back.
    /// `None` when the grower is not compiled.
    pub fn place_grower(&self, name: &str, region: &mut Region, rng: &mut WorldgenRng, pos: (i32, i32, i32)) -> Option<bool> {
        let p = *self.grower_ids.get(name)?;
        Some(self.place_grower_at(p, region, rng, pos))
    }

    /// `place_grower` by the index from `grower_index`: no name lookup per call.
    pub fn place_grower_at(&self, index: usize, region: &mut Region, rng: &mut WorldgenRng, pos: (i32, i32, i32)) -> bool {
        self.run_placed(index, region, rng, pos, None)
    }


    /// Output palette (overworld palette followed by the feature states).
    pub fn palette(&self) -> &[String] {
        &self.palette
    }

    /// Worldgen heightmaps (`WORLD_SURFACE_WG`, `OCEAN_FLOOR_WG`) of a chunk, from its terrain-end blocks.
    /// Compute them once per proto chunk, before any decoration writes into it.
    pub fn wg_heights(&self, chunk: &ChunkBlocks) -> WgHeights {
        heightmap::wg_heights(&chunk.blocks, &self.tags, self.min_y, self.height)
    }

    pub(crate) fn final_heights(&self, chunk: &ChunkBlocks) -> Vec<i32> {
        heightmap::FINAL.into_iter().flat_map(|kind| {
            (0..256).map(move |column| heightmap::prime_column(
                &chunk.blocks, &self.tags, kind, self.min_y, self.height, column,
            ))
        }).collect()
    }

    /// Decorates the centre chunk of the 3x3 `chunks` (raster order, centre at index 4). Writes
    /// from the centre may land in all nine chunks; the nine are updated in place. `wg` holds the
    /// terrain-end worldgen heights of the nine chunks (`wg_heights` of their protos).
    pub fn decorate(&self, o: &Overworld, seed: i64, cx: i32, cz: i32, chunks: &mut [ChunkBlocks], wg: &[WgHeights]) -> Result<Vec<(region::Pos, u16)>, String> {
        Ok(self.decorate_window(o, seed, cx, cz, chunks, wg, &mut Vec::new()))
    }

    pub(crate) fn decorate_window(
        &self, o: &Overworld, seed: i64, cx: i32, cz: i32, chunks: &mut [ChunkBlocks], wg: &[WgHeights],
        final_heights: &mut Vec<i32>,
    ) -> Vec<(region::Pos, u16)> {
        let presence = present_biomes(chunks);
        let mut region = Region::new(
            &self.tags, &self.placed_in_biome, o, self.min_y, self.height, cx, cz, chunks, wg,
            std::mem::take(final_heights),
        );
        if let Some(crate::random::AnyRng::Xoro(rng)) = self.level_random.as_ref().map(|factory| factory.at(cx * 16, 0, cz * 16)) {
            region.set_level_random(WorldgenRng::direct(rng));
        }

        let mut rng = WorldgenRng::new(0);
        let origin_x = cx * 16;
        let origin_z = cz * 16;
        let decoration_seed = rng.set_decoration_seed(seed, origin_x, origin_z);
        let origin_y = self.min_y;

        for (step, list) in self.steps.iter().enumerate() {
            for (index, &p) in list.iter().enumerate() {
                if matches!(self.placed[p].feature, Configured::Unported) {
                    continue;
                }
                let present_here = presence.iter().enumerate().any(|(b, &here)| here && self.in_step[b][step][p]);
                if !present_here {
                    continue;
                }
                rng.set_feature_seed(decoration_seed, index as i32, step as i32);
                self.run_placed(p, &mut region, &mut rng, (origin_x, origin_y, origin_z), Some(p));
            }
        }

        // Only the block ids change here. The palette of the output is `palette()` (the overworld palette
        // plus the feature states); callers read the chunks with that palette, so it is not cloned per call.
        let scheduled = region.take_scheduled();
        *final_heights = region.take_final_heights();
        let out = region.into_blocks();
        for (dst, src) in chunks.iter_mut().zip(out) {
            dst.blocks = src;
        }
        scheduled
    }

    /// `FeaturePlacer.place` for placed feature `p`: the modifier stack, then the configured feature at
    /// every position that survives it. `top` is the feature the `biome` filter checks (`biomeCheck`).
    fn run_placed(&self, p: usize, region: &mut Region, rng: &mut WorldgenRng, origin: (i32, i32, i32), top: Option<usize>) -> bool {
        let pf = &self.placed[p];
        if pf.modifiers.is_empty() {
            return self.place_configured(p, region, rng, origin);
        }
        if let [Modifier::BlockPredicate(filter)] = pf.modifiers.as_slice() {
            return filter.test(region, origin.0, origin.1, origin.2)
                && self.place_configured(p, region, rng, origin);
        }
        let mut placed_any = false;
        let mut stack: Vec<((i32, i32, i32), usize)> = vec![(origin, 0)];
        let mut out = Vec::new();
        while let Some((pos, mi)) = stack.pop() {
            out.clear();
            placement::modify(&pf.modifiers[mi], region, rng, pos, top, &mut out);
            let next = mi + 1;
            if next < pf.modifiers.len() {
                for &q in out.iter().rev() {
                    stack.push((q, next));
                }
            } else {
                for &q in &out {
                    placed_any |= self.place_configured(p, region, rng, q);
                }
            }
        }
        placed_any
    }

    /// `ConfiguredFeature.place` for the configured feature of placed feature `p`.
    fn place_configured(&self, p: usize, region: &mut Region, rng: &mut WorldgenRng, pos: (i32, i32, i32)) -> bool {
        match &self.placed[p].feature {
            Configured::Unported => false,
            Configured::Ore(f) => f.place(region, rng, pos),
            Configured::Disk(f) => f.place(region, rng, pos),
            Configured::Spring(f) => f.place(region, pos),
            Configured::BlockColumn(f) => f.place(region, rng, pos),
            Configured::BlueIce(f) => f.place(region, rng, pos),
            Configured::RandomBoolean { on_true, on_false } => {
                let result = rng.next_bool();
                self.run_placed(if result { *on_true } else { *on_false }, region, rng, pos, None)
            }
            Configured::RandomSelector { entries, default } => {
                let next = entries.iter().find(|(_, chance)| rng.next_float() < *chance).map_or(*default, |&(next, _)| next);
                self.run_placed(next, region, rng, pos, None)
            }
            Configured::SimpleRandom(list) => {
                let index = rng.next_int_bound(list.len() as i32) as usize;
                self.run_placed(list[index], region, rng, pos, None)
            }
            Configured::WeightedRandom { entries, total } => {
                // WeightedList.getRandom: nextInt(totalWeight), then the first entry whose running sum exceeds it.
                let mut selection = rng.next_int_bound(*total);
                let mut chosen = entries[entries.len() - 1].0;
                for &(q, w) in entries {
                    selection -= w;
                    if selection < 0 {
                        chosen = q;
                        break;
                    }
                }
                self.run_placed(chosen, region, rng, pos, None)
            }
            Configured::Tree(t) => {
                let env = self.tree_env.as_ref().expect("tree env");
                let mut ctx = tree::Ctx::new(region, env, rng);
                ctx.decorator = Some(self);
                t.place(&mut ctx, pos)
            }
            Configured::FallenTree(t) => {
                let env = self.tree_env.as_ref().expect("tree env");
                let mut ctx = tree::Ctx::new(region, env, rng);
                t.place(&mut ctx, pos)
            }
            Configured::HugeMushroom(f) => f.place(region, rng, pos),
            Configured::HugeFungus(f) => f.place(region, rng, pos),
            Configured::VegetationPatch(f) => f.place(self, region, rng, pos),
            Configured::Overlay(list) => {
                let mut placed = false;
                for &child in list {
                    placed |= self.run_placed(child, region, rng, pos, None);
                }
                placed
            }
            Configured::NoOp => true,
            Configured::RandomNeighborSpread { provider, neighbors, replaceable, attempts, xz, y } => {
                use placement::sample_int;
                let state = provider.get(region, rng, pos.0, pos.1, pos.2);
                region.write(pos, state, 2);
                for _ in 0..sample_int(attempts, rng) {
                    let (x, y, z) = (pos.0 + sample_int(xz, rng), pos.1 + sample_int(y, rng), pos.2 + sample_int(xz, rng));
                    if replaceable.test(region, x, y, z)
                        && shape::DIRECTIONS.into_iter().filter(|&(dx, dy, dz)| neighbors.test(region, x + dx, y + dy, z + dz)).take(2).count() == 1
                    {
                        let state = provider.get(region, rng, x, y, z);
                        region.write((x, y, z), state, 2);
                    }
                }
                true
            }
            Configured::SimpleBlock(f) => f.place(region, rng, pos),
            Configured::Sequence(list) => {
                for &q in list {
                    if !self.run_placed(q, region, rng, pos, None) {
                        return false;
                    }
                }
                true
            }
        }
    }
}

const OVERWORLD_MIN_Y: i32 = -64;
const OVERWORLD_HEIGHT: i32 = 384;

/// Second half of the compile: tree tables, tags and fluid tables over the final palette.
#[allow(clippy::too_many_arguments)]
fn finish(
    mut c: Compiler<'_>,
    block_tags: &HashMap<String, Vec<String>>,
    min_y: i32,
    height: i32,
    steps: Vec<Vec<usize>>,
    placed_in_biome: Vec<Vec<bool>>,
    in_step: Vec<Vec<Vec<bool>>>,
    grower_ids: HashMap<String, usize>,
) -> Result<Decorator, String> {
    // Vegetation survival tables read the final palette (both half variants of the double plants included).
    let veg = vegetation::VegTable::build(&mut c.pal, &c.tag_names)?;
    let mut waterlogged = Vec::new();
    if c.placed.iter().any(|placed| matches!(&placed.feature, Configured::VegetationPatch(patch) if patch.water.is_some())) {
        for state in 0..c.pal.states().len() {
            let wet = c.pal.with_property(state as u16, "waterlogged", "true");
            waterlogged.push(wet.filter(|&wet| wet != state as u16));
        }
    }
    let mut tags = Tags::new(&c.pal, block_tags);
    tags.set_vegetation(veg);
    // The tree tables cover the final palette: built after everything above that interns states.
    let tree_env = match c.tree_states.take() {
        Some(mut states) => {
            let air = c.pal.intern("minecraft:air");
            states.finish(&c.pal);
            Some(tree::TreeEnv { states, tags: tree::TreeTags::new(&tags)?, air })
        }
        None => None,
    };
    Ok(Decorator {
        palette: c.pal.states().to_vec(),
        tags,
        placed: c.placed,
        steps,
        placed_in_biome,
        in_step,
        min_y,
        height,
        tree_env,
        grower_ids,
        bone_meal_flowers: Vec::new(),
        pale_moss_patch: c.pale_moss_patch,
        waterlogged,
        level_random: None,
        skipped: Vec::new(),
    })
}

/// Biomes present in the 3x3 chunks: the biome palettes that `addVanillaDecorations` collects.
fn present_biomes(chunks: &[ChunkBlocks]) -> Vec<bool> {
    let mut out = vec![false; chunks.first().map_or(0, |c| c.biome_palette.len())];
    for c in chunks {
        for &b in &c.biomes {
            out[b as usize] = true;
        }
    }
    out
}
