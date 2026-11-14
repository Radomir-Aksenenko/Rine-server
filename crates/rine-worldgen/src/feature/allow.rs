//! The shared golden allow rule (`assets/worldgen/ported_features.json`): a top-level placed feature is
//! allowed when every placement type is ported (`biome` only at the top level) and its configured feature
//! and every nested placed feature (selectors, sequences) are ported too. `tools/noise-golden`'s
//! `make_feature_datapack.js` applies the same rule to the vanilla datapack.

use serde_json::Value;

const FEATURES_JSON: &str = include_str!("../../assets/worldgen/features.json");
const PORTED_JSON: &str = include_str!("../../assets/worldgen/ported_features.json");

/// Ported-set check for a top-level placed feature name (`minecraft:...`).
pub fn ported_only(name: &str) -> bool {
    let bundle: Value = serde_json::from_str(FEATURES_JSON).expect("bundled features.json");
    let ported: Value = serde_json::from_str(PORTED_JSON).expect("bundled ported_features.json");
    let types: Vec<&str> = ported["types"].as_array().expect("types").iter().filter_map(Value::as_str).collect();
    let placements: Vec<&str> = ported["placements"].as_array().expect("placements").iter().filter_map(Value::as_str).collect();
    let predicates: Vec<&str> = ported["predicates"].as_array().expect("predicates").iter().filter_map(Value::as_str).collect();
    let rule = Rule { bundle: &bundle, types: &types, placements: &placements, predicates: &predicates };
    rule.placed(name, true)
}

struct Rule<'a> {
    bundle: &'a Value,
    types: &'a [&'a str],
    placements: &'a [&'a str],
    predicates: &'a [&'a str],
}

impl Rule<'_> {
    fn placed(&self, name: &str, top: bool) -> bool {
        let p = &self.bundle["placed"][name];
        !p.is_null() && self.entry(p, top)
    }

    /// A block predicate tree (`block_predicate_filter`, `allowed_placement`): every type is ported.
    fn block_predicate(&self, p: &Value) -> bool {
        let Some(ty) = p["type"].as_str() else { return false };
        if !self.predicates.contains(&ty) {
            return false;
        }
        match ty {
            "minecraft:all_of" | "minecraft:any_of" => p["predicates"].as_array().is_some_and(|l| l.iter().all(|x| self.block_predicate(x))),
            "minecraft:not" => self.block_predicate(&p["predicate"]),
            _ => true,
        }
    }

    /// A block state provider as `state_provider::parse` reads it: simple states, `weighted`,
    /// `randomized_int` over a simple source, and `rule_based` with ported predicates.
    fn state_provider(&self, v: &Value) -> bool {
        let Some(ty) = v.get("type").and_then(Value::as_str) else { return true };
        match ty {
            "minecraft:weighted" => true,
            "minecraft:randomized_int" => v["source"].get("type").is_none(),
            "minecraft:rule_based" => {
                let rules_ok = v["rules"].as_array().is_some_and(|rules| {
                    rules.iter().all(|r| self.block_predicate(&r["if_true"]) && self.state_provider(&r["then"]))
                });
                rules_ok && v.get("fallback").is_none_or(|f| self.state_provider(f))
            }
            _ => false,
        }
    }

    /// A placed-feature object (named or inline): placements and the configured feature.
    fn entry(&self, p: &Value, top: bool) -> bool {
        let placements_ok = p["placement"].as_array().is_some_and(|list| {
            list.iter().all(|m| {
                let ty = m["type"].as_str().unwrap_or("");
                if !self.placements.contains(&ty) || (!top && ty == "minecraft:biome") {
                    return false;
                }
                ty != "minecraft:block_predicate_filter" || self.block_predicate(&m["predicate"])
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
        if !self.types.contains(&ty) {
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
        let sub = |x: &Value| match x.as_str() {
            Some(n) => self.placed(n, false),
            None => self.entry(x, false),
        };
        match ty {
            "minecraft:random_boolean_selector" => sub(&cfg["feature_true"]) && sub(&cfg["feature_false"]),
            "minecraft:simple_random_selector" | "minecraft:sequence" => cfg["features"].as_array().is_some_and(|l| l.iter().all(sub)),
            "minecraft:weighted_random_selector" => cfg["features"].as_array().is_some_and(|l| l.iter().all(|e| sub(&e["data"]))),
            _ => true,
        }
    }
}
