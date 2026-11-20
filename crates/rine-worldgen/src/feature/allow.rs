//! The shared golden allow rule (`assets/worldgen/ported_features.json`): a top-level placed feature is allowed when
//! every placement type is ported (`biome` only at the top level) and its configured feature and every nested placed
//! feature (selectors, sequences) are ported too. `tools/noise-golden`'s `make_feature_datapack.js` applies the same
//! rule to the vanilla datapack.

use serde_json::Value;

use super::{bundle, provider_value, vegetation};

const PORTED_JSON: &str = include_str!("../../assets/worldgen/ported_features.json");

/// Ported-set check for a top-level placed feature name (`minecraft:...`).
pub fn ported_only(name: &str) -> bool {
    allowed(name, false)
}

/// Current decoration coverage. The older feature golden deliberately uses its original subset.
pub fn generation(name: &str) -> bool {
    allowed(name, true)
}

/// Vegetation placed by the server alongside its existing terrain and non-vegetation decoration.
pub fn live_generation(name: &str) -> bool {
    let name_without_namespace = name.strip_prefix("minecraft:").unwrap_or(name);
    (name_without_namespace.starts_with("flower_") || name_without_namespace.starts_with("wildflowers_")
        || name_without_namespace.starts_with("patch_grass") || name_without_namespace.starts_with("patch_tall_grass")
        || matches!(name_without_namespace, "forest_flowers" | "patch_sunflower" | "pale_garden_flowers" | "pale_moss_patch"
            | "pale_garden_vegetation" | "lush_caves_ceiling_vegetation" | "lush_caves_clay" | "lush_caves_vegetation"))
        && generation(name)
}

fn allowed(name: &str, expanded: bool) -> bool {
    static PORTED: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    let ported = PORTED.get_or_init(|| serde_json::from_str(PORTED_JSON).expect("bundled ported_features.json"));
    let strings = |key: &str| -> Vec<&str> { ported[key].as_array().expect(key).iter().filter_map(Value::as_str).collect() };
    let rule = Rule {
        bundle: bundle(),
        types: &strings("types"),
        placements: &strings("placements"),
        predicates: &strings("predicates"),
        simple_blocks: &strings("simple_blocks"),
        expanded,
    };
    rule.placed(name, true)
}

struct Rule<'a> {
    bundle: &'a Value,
    types: &'a [&'a str],
    placements: &'a [&'a str],
    predicates: &'a [&'a str],
    simple_blocks: &'a [&'a str],
    expanded: bool,
}

impl Rule<'_> {
    fn placed(&self, name: &str, top: bool) -> bool {
        let p = &self.bundle["placed"][name];
        !p.is_null() && self.entry(p, top)
    }

    /// A block predicate tree (`block_predicate_filter`, `allowed_placement`): every type is ported.
    fn block_predicate(&self, p: &Value) -> bool {
        let Some(ty) = p["type"].as_str() else { return false };
        if !(self.predicates.contains(&ty) || self.expanded && matches!(ty, "minecraft:solid" | "minecraft:volume_match" | "minecraft:has_sturdy_face")) {
            return false;
        }
        match ty {
            "minecraft:all_of" | "minecraft:any_of" => p["predicates"].as_array().is_some_and(|l| l.iter().all(|x| self.block_predicate(x))),
            "minecraft:not" => self.block_predicate(&p["predicate"]),
            "minecraft:volume_match" => self.block_predicate(&p["match"]),
            // `would_survive` is ported for the blocks of `vegetation.rs` only (the sapling and propagule paths are not
            // in the allow list).
            "minecraft:would_survive" => p["state"].as_str().is_some_and(|s| {
                self.simple_block_name(s) || self.expanded && s == "minecraft:pale_oak_sapling"
            }),
            _ => true,
        }
    }

    /// A block state string (`name` or `name[props]`) naming a block in the ported vegetation set.
    fn simple_block_name(&self, state: &str) -> bool {
        let name = state.split('[').next().unwrap_or(state);
        self.simple_blocks.contains(&name) || (self.expanded && vegetation::kind_of_name(name).is_some())
    }

    /// A block state provider as `state_provider::parse` reads it: simple states, `weighted`,
    /// `randomized_int` over a simple source, and `rule_based` with ported predicates.
    fn state_provider(&self, v: &Value) -> bool {
        let v = if self.expanded { provider_value(self.bundle, v) } else { v };
        let Some(ty) = v.get("type").and_then(Value::as_str) else { return true };
        match ty {
            "minecraft:weighted" => true,
            "minecraft:randomized_int" => v["source"].get("type").is_none()
                || self.expanded && matches!(v["source"]["type"].as_str(), Some("minecraft:weighted" | "minecraft:noise" | "minecraft:dual_noise" | "minecraft:noise_threshold")),
            "minecraft:noise" | "minecraft:dual_noise" | "minecraft:noise_threshold" => self.expanded,
            "minecraft:rule_based" => {
                let rules_ok = v["rules"].as_array().is_some_and(|rules| {
                    rules.iter().all(|r| self.block_predicate(&r["if_true"]) && self.state_provider(&r["then"]))
                });
                rules_ok && v.get("fallback").is_none_or(|f| self.state_provider(f))
            }
            _ => false,
        }
    }

    /// A `simple_block` provider: every block it can answer with is in the ported vegetation set, and the
    /// predicates of its `rule_based` parts are ported.
    fn simple_block_provider(&self, v: &Value) -> bool {
        let v = if self.expanded { provider_value(self.bundle, v) } else { v };
        let mut names = Vec::new();
        vegetation::provider_names(v, &mut names).is_ok()
            && names.iter().all(|n| self.simple_block_name(n))
            && self.rule_predicates(v)
    }

    /// The `if_true` predicates of every `rule_based` provider inside `v` are ported block predicates.
    fn rule_predicates(&self, v: &Value) -> bool {
        match v.get("type").and_then(Value::as_str) {
            Some("minecraft:rule_based") => {
                v["rules"].as_array().is_some_and(|rules| rules.iter().all(|r| self.block_predicate(&r["if_true"]) && self.rule_predicates(&r["then"])))
                    && v.get("fallback").is_none_or(|f| self.rule_predicates(f))
            }
            Some("minecraft:weighted") => v["entries"].as_array().is_some_and(|e| e.iter().all(|x| self.rule_predicates(&x["data"]))),
            Some("minecraft:randomized_int") => self.rule_predicates(&v["source"]),
            _ => true,
        }
    }

    /// A placed-feature object (named or inline): placements and the configured feature.
    fn entry(&self, p: &Value, top: bool) -> bool {
        let placements_ok = p["placement"].as_array().is_some_and(|list| {
            list.iter().all(|m| {
                let ty = m["type"].as_str().unwrap_or("");
                if !(self.placements.contains(&ty) || self.expanded && matches!(ty,
                    "minecraft:environment_scan" | "minecraft:count_on_every_layer" | "minecraft:surface_water_depth_filter"
                    | "minecraft:randomly_selected" | "minecraft:random_chance"))
                    || (!top && ty == "minecraft:biome")
                {
                    return false;
                }
                match ty {
                    "minecraft:block_predicate_filter" => self.block_predicate(&m["predicate"]),
                    "minecraft:randomly_selected" => m["placements"].as_array().is_some_and(|items| {
                        !items.is_empty() && items.iter().all(|item| item["type"] == "minecraft:offset")
                    }),
                    _ => true,
                }
            })
        });
        placements_ok && self.feature(&p["feature"])
    }

    fn feature(&self, r: &Value) -> bool {
        let cfg = match r.as_str() {
            Some(n) => &self.bundle["configured"][n],
            None => r,
        };
        let Some(ty) = cfg["type"].as_str() else { return false };
        let extra_tree = r.as_str().is_some_and(|name| name.starts_with("minecraft:pale_oak") || name == "minecraft:azalea_tree");
        if !(self.types.contains(&ty) || self.expanded && (matches!(ty,
            "minecraft:vegetation_patch" | "minecraft:waterlogged_vegetation_patch" | "minecraft:overlay"
            | "minecraft:random_neighbor_spread" | "minecraft:huge_fungus" | "minecraft:no_op" | "minecraft:random_selector"
            | "minecraft:monster_room" | "minecraft:root_system" | "minecraft:speleothem" | "minecraft:speleothem_cluster"
            | "minecraft:large_dripstone" | "minecraft:fossil" | "minecraft:template" | "minecraft:sculk_patch")
            || ty == "minecraft:tree" && extra_tree))
        {
            return false;
        }
        if ty == "minecraft:block_column" {
            let layers_ok = cfg["layers"].as_array().is_some_and(|l| l.iter().all(|x| self.state_provider(&x["provider"])));
            if !layers_ok || !self.block_predicate(&cfg["allowed_placement"]) {
                return false;
            }
        }
        if ty == "minecraft:disk" && (!self.block_predicate(&cfg["target"]) || !self.state_provider(&cfg["state_provider"])) {
            return false;
        }
        if ty == "minecraft:simple_block" && !self.simple_block_provider(&cfg["to_place"]) {
            return false;
        }
        let sub = |x: &Value| match x.as_str() {
            Some(n) => self.placed(n, false),
            None => self.entry(x, false),
        };
        match ty {
            "minecraft:random_boolean_selector" => sub(&cfg["feature_true"]) && sub(&cfg["feature_false"]),
            "minecraft:random_selector" => cfg["features"].as_array().is_some_and(|l| l.iter().all(|e| sub(&e["feature"]))) && sub(&cfg["default"]),
            "minecraft:simple_random_selector" | "minecraft:sequence" => cfg["features"].as_array().is_some_and(|l| l.iter().all(sub)),
            "minecraft:weighted_random_selector" => cfg["features"].as_array().is_some_and(|l| l.iter().all(|e| sub(&e["data"]))),
            "minecraft:overlay" => cfg["features"].as_array().map_or_else(|| sub(&cfg["features"]), |l| l.iter().all(sub)),
            "minecraft:vegetation_patch" | "minecraft:waterlogged_vegetation_patch" => sub(&cfg["vegetation_feature"]),
            "minecraft:root_system" => sub(&cfg["feature"]) && self.block_predicate(&cfg["allowed_tree_position"]),
            _ => true,
        }
    }
}
