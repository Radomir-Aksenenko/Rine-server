//! Phase 4: vanilla biome decoration (`ChunkGenerator.addVanillaDecorations` + `FeaturePlacer`).
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
//! Ported configured features: `ore`, `disk`, `spring_feature`, `block_column`, `blue_ice`,
//! `random_boolean_selector`, `simple_random_selector`, `weighted_random_selector`, `sequence`.
//!
//! Ordering: `sorter` reproduces `FeatureSorter`; within a chunk the features of each step run in the
//! sorted order, skipping features that no biome present in the 3x3 chunks has in that step.

pub mod column;
pub mod disk;
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

use std::collections::HashMap;

use serde_json::Value;

use crate::overworld::{BiomeMap, ChunkBlocks, Overworld};
use column::{direction_of, BlockColumn, Layer};
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
    Ore(OreFeature),
    Disk(Disk),
    Spring(Spring),
    BlockColumn(BlockColumn),
    BlueIce(BlueIce),
    /// `RandomBooleanSelectorFeature`: a placed-feature id per branch.
    RandomBoolean { on_true: usize, on_false: usize },
    /// `SimpleRandomSelectorFeature`: uniform over the list.
    SimpleRandom(Vec<usize>),
    /// `WeightedRandomSelectorFeature`: `(placed id, weight)`; `total` > 0.
    WeightedRandom { entries: Vec<(usize, i32)>, total: i32 },
    /// `SequenceFeature`: all placed features in order, stopping at the first failure.
    Sequence(Vec<usize>),
    /// `minecraft:tree`.
    Tree(tree::TreeFeature),
    /// `minecraft:fallen_tree`.
    FallenTree(tree::fallen::FallenTree),
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
}

impl<'a> Compiler<'a> {
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
                        state: state_provider::parse(&l["provider"], tn, &mut self.pal)?,
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
            "minecraft:sequence" => {
                let mut list = Vec::new();
                for f in cfg["features"].as_array().ok_or("features")? {
                    list.push(self.placed_ref(f)?);
                }
                Ok(Configured::Sequence(list))
            }
            "minecraft:tree" | "minecraft:fallen_tree" => {
                self.ensure_tree_states()?;
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

impl Decorator {
    /// Compiles the decoration of `o` for the top-level placed features `allow` accepts.
    pub fn new(o: &Overworld, allow: &dyn Fn(&str) -> bool) -> Result<Self, String> {
        Self::new_from(bundle(), o, allow)
    }

    /// Like [`Decorator::new`] with an explicit feature bundle (`features.json` layout). The tree golden bundles
    /// synthetic placed features with matching biome lists (`tools/noise-golden/make_tree_golden.js`).
    pub fn new_from(bundle: &Value, o: &Overworld, allow: &dyn Fn(&str) -> bool) -> Result<Self, String> {
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
        };

        // Biome feature lists, filtered; ids are placed-feature indices.
        let mut biome_lists: Vec<Vec<Vec<usize>>> = Vec::with_capacity(biome_names.len());
        for name in &biome_names {
            let steps = bundle["biomes"][name].as_array().ok_or_else(|| format!("biome {name} not bundled"))?;
            let mut lists = Vec::with_capacity(steps.len());
            for step in steps {
                let mut ids = Vec::new();
                for f in step.as_array().unwrap() {
                    let f = f.as_str().unwrap();
                    if !allow(f) {
                        continue;
                    }
                    ids.push(c.placed_named(f)?);
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

        // The tree tables and the tag and fluid tables cover the final palette.
        if let Some(ts) = c.tree_states.as_mut() {
            ts.finish(&c.pal);
        }
        let tags = Tags::new(&c.pal, &block_tags);
        let tree_env = match c.tree_states.take() {
            Some(states) => {
                let air = c.pal.intern("minecraft:air");
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
            min_y: o.min_y(),
            height: o.height(),
            tree_env,
        })
    }

    /// Output palette (overworld palette followed by the feature states).
    pub fn palette(&self) -> &[String] {
        &self.palette
    }

    /// Decorates the centre chunk of the 3x3 `chunks` (raster order, centre at index 4). Writes
    /// from the centre may land in all nine chunks; the nine are updated in place.
    pub fn decorate(&self, o: &Overworld, seed: i64, cx: i32, cz: i32, chunks: &mut [ChunkBlocks]) -> Result<(), String> {
        let presence = present_biomes(chunks);
        let biome_map = BiomeMap::new(o, cx, cz);
        let mut region = Region::new(&self.tags, &self.placed_in_biome, biome_map, self.min_y, self.height, cx, cz, chunks);

        let mut rng = WorldgenRng::new(0);
        let origin_x = cx * 16;
        let origin_z = cz * 16;
        let decoration_seed = rng.set_decoration_seed(seed, origin_x, origin_z);
        let origin_y = self.min_y;

        for (step, list) in self.steps.iter().enumerate() {
            for (index, &p) in list.iter().enumerate() {
                let present_here = presence.iter().enumerate().any(|(b, &here)| here && self.in_step[b][step][p]);
                if !present_here {
                    continue;
                }
                rng.set_feature_seed(decoration_seed, index as i32, step as i32);
                self.run_placed(p, &mut region, &mut rng, (origin_x, origin_y, origin_z), Some(p));
            }
        }

        let out = region.into_blocks();
        for (dst, src) in chunks.iter_mut().zip(out) {
            dst.blocks = src;
            dst.palette = self.palette.clone();
        }
        Ok(())
    }

    /// `FeaturePlacer.place` for placed feature `p`: the modifier stack, then the configured feature at
    /// every position that survives it. `top` is the feature the `biome` filter checks (`biomeCheck`).
    fn run_placed(&self, p: usize, region: &mut Region, rng: &mut WorldgenRng, origin: (i32, i32, i32), top: Option<usize>) -> bool {
        let pf = &self.placed[p];
        if pf.modifiers.is_empty() {
            return self.place_configured(p, region, rng, origin);
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
            Configured::Ore(f) => f.place(region, rng, pos),
            Configured::Disk(f) => f.place(region, rng, pos),
            Configured::Spring(f) => f.place(region, pos),
            Configured::BlockColumn(f) => f.place(region, rng, pos),
            Configured::BlueIce(f) => f.place(region, rng, pos),
            Configured::RandomBoolean { on_true, on_false } => {
                let result = rng.next_bool();
                self.run_placed(if result { *on_true } else { *on_false }, region, rng, pos, None)
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
                t.place(&mut ctx, pos)
            }
            Configured::FallenTree(t) => {
                let env = self.tree_env.as_ref().expect("tree env");
                let mut ctx = tree::Ctx::new(region, env, rng);
                t.place(&mut ctx, pos)
            }
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
