//! Phase 4: vanilla biome decoration (`ChunkGenerator.addVanillaDecorations` + `FeaturePlacer`).
//!
//! Data comes from `assets/worldgen/features.json` (`tools/noise-golden/bundle_features.js`): biome
//! feature lists, placed features and configured features reached from the overworld biome list.
//! `allow` filters placed features by id before anything else, so a golden can run vanilla with the
//! same filtered biome lists (a datapack with the same `features` arrays).
//!
//! Implemented so far: the ore configured feature with its placement chain (count, rarity, in_square,
//! height_range, biome, offset). Every other configured feature type is reported as an error when it is
//! allowed, so an unported feature can never be silently skipped.
//!
//! Ordering: `sorter` reproduces `FeatureSorter`; within a chunk the features of each step run in the
//! sorted order, skipping features that no biome present in the 3x3 chunks has in that step.

pub mod placement;
pub mod region;
pub mod ore;
pub mod rng;
pub mod sorter;
pub mod tree;

use std::collections::HashMap;

use serde_json::Value;

use crate::overworld::{BiomeMap, ChunkBlocks, Overworld};
use ore::{OreFeature, RuleTest, Target};
use placement::{parse_modifier, Modifier};
use region::{Region, Tags};
use rng::WorldgenRng;

const FEATURES_JSON: &str = include_str!("../../assets/worldgen/features.json");

/// `GenerationStep.Decoration` count; vanilla always runs at least this many steps.
const DECORATION_STEPS: usize = 11;

enum Configured {
    Ore(OreFeature),
    Tree(tree::TreeFeature),
    FallenTree(tree::fallen::FallenTree),
}

struct PlacedFeature {
    name: String,
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
    /// Block states and tags of the tree port; `Some` when a tree placed feature is allowed.
    tree_env: Option<tree::TreeEnv>,
}

fn bundle() -> &'static Value {
    static BUNDLE: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    BUNDLE.get_or_init(|| serde_json::from_str(FEATURES_JSON).expect("bundled features.json"))
}

fn parse_predicate(v: &Value, tags: &Tags) -> Result<RuleTest, String> {
    let ty = v["predicate_type"].as_str().ok_or_else(|| format!("predicate without type: {v}"))?;
    match ty {
        "minecraft:tag_match" => {
            let name = v["tag"].as_str().ok_or("tag_match without tag")?;
            let id = tags.id(name).ok_or_else(|| format!("tag {name} not bundled"))?;
            Ok(RuleTest::TagMatch(id))
        }
        "minecraft:any_of" | "minecraft:all_of" => {
            let rules = v["rules"]
                .as_array()
                .ok_or("rules")?
                .iter()
                .map(|r| parse_predicate(r, tags))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(if ty == "minecraft:any_of" { RuleTest::AnyOf(rules) } else { RuleTest::AllOf(rules) })
        }
        "minecraft:not" => Ok(RuleTest::Not(Box::new(parse_predicate(&v["rule"], tags)?))),
        "minecraft:height_match" => Ok(RuleTest::HeightMatch {
            min: v["min_inclusive"].as_i64().ok_or("min_inclusive")? as i32,
            max: v["max_inclusive"].as_i64().ok_or("max_inclusive")? as i32,
        }),
        other => Err(format!("unsupported rule test {other}")),
    }
}

/// Block state strings used by the ore targets are canonical names (no properties).
fn state_name(v: &Value) -> Result<String, String> {
    match v.as_str() {
        Some(s) if !s.contains('[') => Ok(s.to_string()),
        _ => Err(format!("unsupported block state in ore target: {v}")),
    }
}

impl Decorator {
    /// Compiles the decoration of `o` for the placed features `allow` accepts.
    pub fn new(o: &Overworld, allow: &dyn Fn(&str) -> bool) -> Result<Self, String> {
        Self::new_from(bundle(), o, allow)
    }

    /// Like [`Decorator::new`] with an explicit feature bundle (`features.json` layout). The tree golden uses
    /// a bundle with synthetic placed features and matching biome lists (`tools/noise-golden/make_tree_golden.js`).
    pub fn new_from(bundle: &Value, o: &Overworld, allow: &dyn Fn(&str) -> bool) -> Result<Self, String> {
        let biome_names: Vec<String> = o.biome_names().to_vec();

        // Biome feature lists, filtered; placed ids in first-appearance order.
        let mut placed_ids: HashMap<String, usize> = HashMap::new();
        let mut placed_names: Vec<String> = Vec::new();
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
                    let id = match placed_ids.get(f) {
                        Some(&id) => id,
                        None => {
                            let id = placed_names.len();
                            placed_names.push(f.to_string());
                            placed_ids.insert(f.to_string(), id);
                            id
                        }
                    };
                    ids.push(id);
                }
                lists.push(ids);
            }
            biome_lists.push(lists);
        }

        // Compile placed features (ore only so far) and collect block states.
        let mut palette: Vec<String> = o.interner().names().to_vec();
        let mut palette_ids: HashMap<String, u16> = palette.iter().enumerate().map(|(i, s)| (s.clone(), i as u16)).collect();
        // Tree block states (all property combinations of the blocks the tree configs use), interned once
        // so runtime `setValue` on tree states is a table lookup. Only when a tree placed feature is allowed.
        let needs_trees = placed_names.iter().any(|n| {
            let f = bundle["placed"][n.as_str()]["feature"].as_str().unwrap_or("");
            matches!(bundle["configured"][f]["type"].as_str(), Some("minecraft:tree") | Some("minecraft:fallen_tree"))
        });
        let tree_states = if needs_trees {
            let wanted = tree::tree_wanted_blocks(bundle);
            Some(tree::state::TreeStates::build(&bundle["blockProps"], &wanted, &mut palette, &mut palette_ids)?)
        } else {
            None
        };

        let mut intern = |palette: &mut Vec<String>, name: String| -> u16 {
            if let Some(&id) = palette_ids.get(&name) {
                return id;
            }
            let id = palette.len() as u16;
            palette.push(name.clone());
            palette_ids.insert(name, id);
            id
        };

        // First pass for states only, so the tag table covers the final palette.
        let mut raw_configured: Vec<(usize, String)> = Vec::new();
        for (id, name) in placed_names.iter().enumerate() {
            let placed = &bundle["placed"][name.as_str()];
            let feature_ref = placed["feature"].as_str().ok_or_else(|| format!("{name}: inline feature not supported"))?;
            let cfg = &bundle["configured"][feature_ref];
            match cfg["type"].as_str() {
                Some("minecraft:ore") => {
                    for t in cfg["targets"].as_array().ok_or("targets")? {
                        intern(&mut palette, state_name(&t["state"])?);
                    }
                }
                Some("minecraft:tree") | Some("minecraft:fallen_tree") => {}
                Some(other) => {
                    return Err(format!("{name}: configured feature {other} is not ported yet"));
                }
                None => return Err(format!("{name}: configured feature without type")),
            }
            raw_configured.push((id, feature_ref.to_string()));
        }

        let block_tags: HashMap<String, Vec<String>> = bundle["blockTags"]
            .as_object()
            .ok_or("blockTags")?
            .iter()
            .map(|(k, v)| (k.clone(), v.as_array().unwrap().iter().map(|b| b.as_str().unwrap().to_string()).collect()))
            .collect();
        let tags = Tags::new(&palette, &block_tags);
        let tree_env = match tree_states {
            Some(states) => {
                let air = palette.iter().position(|s| s == "minecraft:air").expect("air in palette") as u16;
                Some(tree::TreeEnv { states, tags: tree::TreeTags::new(&tags)?, air })
            }
            None => None,
        };

        let mut placed = Vec::with_capacity(placed_names.len());
        for (id, feature_ref) in raw_configured {
            let name = &placed_names[id];
            let pj = &bundle["placed"][name.as_str()];
            let cfg = &bundle["configured"][feature_ref.as_str()];
            if let Some(env) = tree_env.as_ref() {
                let compile_env = tree::CompileEnv { env, tags: &tags, providers: &bundle["blockStateProviders"] };
                let modifiers = pj["placement"]
                    .as_array()
                    .ok_or("placement")?
                    .iter()
                    .map(parse_modifier)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("{name}: {e}"))?;
                match cfg["type"].as_str() {
                    Some("minecraft:tree") => {
                        let feature = tree::TreeFeature::compile(cfg, &compile_env).map_err(|e| format!("{name}: {e}"))?;
                        placed.push(PlacedFeature { name: name.clone(), modifiers, feature: Configured::Tree(feature) });
                        continue;
                    }
                    Some("minecraft:fallen_tree") => {
                        let feature = tree::fallen::FallenTree::compile(cfg, &compile_env).map_err(|e| format!("{name}: {e}"))?;
                        placed.push(PlacedFeature { name: name.clone(), modifiers, feature: Configured::FallenTree(feature) });
                        continue;
                    }
                    _ => {}
                }
            }
            let modifiers = pj["placement"]
                .as_array()
                .ok_or("placement")?
                .iter()
                .map(parse_modifier)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("{name}: {e}"))?;
            let targets = cfg["targets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| {
                    Ok(Target {
                        test: parse_predicate(&t["target"], &tags)?,
                        state: palette_ids[&state_name(&t["state"])?],
                    })
                })
                .collect::<Result<Vec<_>, String>>()
                .map_err(|e: String| format!("{name}: {e}"))?;
            let ore = OreFeature {
                targets,
                size: cfg["size"].as_i64().ok_or("size")? as i32,
                discard_chance_on_air_exposure: cfg["discard_chance_on_air_exposure"].as_f64().unwrap_or(0.0) as f32,
            };
            placed.push(PlacedFeature { name: name.clone(), modifiers, feature: Configured::Ore(ore) });
        }

        // Feature order per step (FeatureSorter over the filtered biome lists).
        let steps = sorter::build_features_per_step(&biome_lists)?;

        // Biome membership tables.
        let placed_count = placed.len();
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

        Ok(Decorator {
            palette,
            tags,
            placed,
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
                self.place_with_biome_check(p, &mut region, &mut rng, (origin_x, origin_y, origin_z));
            }
        }

        let out = region.into_blocks();
        for (dst, src) in chunks.iter_mut().zip(out) {
            dst.blocks = src;
            dst.palette = self.palette.clone();
        }
        Ok(())
    }

    /// `FeaturePlacer.placeWithBiomeCheck`: the modifier stack, then the configured feature.
    fn place_with_biome_check(&self, p: usize, region: &mut Region, rng: &mut WorldgenRng, origin: (i32, i32, i32)) -> bool {
        let pf = &self.placed[p];
        if pf.modifiers.is_empty() {
            return self.place_configured(pf, region, rng, origin);
        }
        let mut placed_any = false;
        let mut stack: Vec<((i32, i32, i32), usize)> = vec![(origin, 0)];
        let mut out = Vec::new();
        while let Some((pos, mi)) = stack.pop() {
            out.clear();
            placement::modify(&pf.modifiers[mi], region, rng, pos, p, &mut out);
            let next = mi + 1;
            if next < pf.modifiers.len() {
                for &q in out.iter().rev() {
                    stack.push((q, next));
                }
            } else {
                for &q in &out {
                    placed_any |= self.place_configured(pf, region, rng, q);
                }
            }
        }
        placed_any
    }

    fn place_configured(&self, pf: &PlacedFeature, region: &mut Region, rng: &mut WorldgenRng, pos: (i32, i32, i32)) -> bool {
        match &pf.feature {
            Configured::Ore(ore) => ore.place(region, rng, pos),
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
