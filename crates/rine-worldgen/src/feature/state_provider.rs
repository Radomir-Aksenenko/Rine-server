//! `BlockStateProvider` (`levelgen/feature/stateproviders`): simple, weighted, randomized int property and
//! rule based. Every provider compiles to palette ids; the random draws happen at placement time in the
//! same order as the Java `getState`/`getOptionalState`.

use serde_json::Value;

use super::placement::{parse_int_provider, sample_int, IntProvider};
use super::predicate::{self, BlockPredicate};
use super::region::Region;
use super::rng::WorldgenRng;
use super::state::Palette;

#[derive(Clone, Debug)]
pub enum StateProvider {
    /// `SimpleStateProvider`.
    Simple(u16),
    /// `WeightedStateProvider`: `WeightedList.getRandomOrThrow` (one `nextInt(totalWeight)`).
    Weighted { entries: Vec<(u16, i32)>, total: i32 },
    /// `RandomizedIntStateProvider` over a simple source: `source.setValue(property, values.sample())`.
    /// `table[v - min]` is the palette id for each value the provider can sample.
    RandomizedInt { source: u16, values: IntProvider, min: i32, table: Vec<u16> },
    /// `RuleBasedStateProvider`: the first rule whose predicate holds and whose provider answers, else the
    /// fallback. `getState` falls back to the block already at the position.
    RuleBased { rules: Vec<(BlockPredicate, StateProvider)>, fallback: Option<Box<StateProvider>> },
}

/// A block state given as a name or as `{id, properties}`.
pub fn state_of(v: &Value, pal: &mut Palette) -> Result<u16, String> {
    if let Some(name) = v.as_str() {
        return Ok(pal.default_state(name));
    }
    let id = v["id"].as_str().ok_or_else(|| format!("block state without id: {v}"))?;
    match v.get("properties").and_then(|p| p.as_object()) {
        None => Ok(pal.default_state(id)),
        Some(props) => {
            let kv: Vec<(String, String)> = props
                .iter()
                .map(|(k, val)| (k.clone(), val.as_str().map(str::to_string).unwrap_or_else(|| val.to_string())))
                .collect();
            let refs: Vec<(&str, &str)> = kv.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            Ok(pal.state_from_parts(id, &refs))
        }
    }
}

/// Parses a block state or a provider (`{type: ...}`), resolving the palette ids.
pub fn parse(v: &Value, tag_names: &[String], pal: &mut Palette) -> Result<StateProvider, String> {
    let Some(ty) = v.get("type").and_then(|t| t.as_str()) else {
        return Ok(StateProvider::Simple(state_of(v, pal)?));
    };
    match ty {
        "minecraft:weighted" => {
            let mut entries = Vec::new();
            for e in v["entries"].as_array().ok_or("entries")? {
                let weight = e["weight"].as_i64().ok_or("weight")? as i32;
                if weight <= 0 {
                    continue;
                }
                entries.push((state_of(&e["data"], pal)?, weight));
            }
            let total: i32 = entries.iter().map(|e| e.1).sum();
            if total == 0 {
                return Err("weighted state provider without weight".into());
            }
            Ok(StateProvider::Weighted { entries, total })
        }
        "minecraft:randomized_int" => {
            let source = match parse(&v["source"], tag_names, pal)? {
                StateProvider::Simple(s) => s,
                other => return Err(format!("randomized_int over a non-simple source: {other:?}")),
            };
            let property = v["property"].as_str().ok_or("property")?;
            let values = parse_int_provider(&v["values"])?;
            let (min, max) = values.bounds();
            let mut table = Vec::with_capacity((max - min + 1) as usize);
            for val in min..=max {
                // Vanilla keeps the source state when it has no such property.
                let s = pal.with_property(source, property, &val.to_string()).unwrap_or(source);
                table.push(s);
            }
            Ok(StateProvider::RandomizedInt { source, values, min, table })
        }
        "minecraft:rule_based" => {
            let mut rules = Vec::new();
            for r in v["rules"].as_array().ok_or("rules")? {
                let pred = predicate::parse(&r["if_true"], tag_names, pal)?;
                rules.push((pred, parse(&r["then"], tag_names, pal)?));
            }
            let fallback = match v.get("fallback") {
                Some(f) => Some(Box::new(parse(f, tag_names, pal)?)),
                None => None,
            };
            Ok(StateProvider::RuleBased { rules, fallback })
        }
        other => Err(format!("unsupported block state provider {other}")),
    }
}

impl StateProvider {
    /// `getOptionalState`: `None` when a rule-based provider has no answer.
    pub fn get_optional(&self, region: &Region, rng: &mut WorldgenRng, x: i32, y: i32, z: i32) -> Option<u16> {
        match self {
            StateProvider::Simple(s) => Some(*s),
            StateProvider::Weighted { entries, total } => {
                let mut sel = rng.next_int_bound(*total);
                for &(s, w) in entries {
                    sel -= w;
                    if sel < 0 {
                        return Some(s);
                    }
                }
                unreachable!("selection below total weight")
            }
            StateProvider::RandomizedInt { values, min, table, .. } => {
                let v = sample_int(values, rng);
                Some(table[(v - min) as usize])
            }
            StateProvider::RuleBased { rules, fallback } => {
                for (pred, then) in rules {
                    if pred.test(region, x, y, z) {
                        if let Some(s) = then.get_optional(region, rng, x, y, z) {
                            return Some(s);
                        }
                    }
                }
                match fallback {
                    Some(f) => f.get_optional(region, rng, x, y, z),
                    None => None,
                }
            }
        }
    }

    /// `getState`: like `get_optional`, but a rule-based miss keeps the block at the position.
    pub fn get(&self, region: &Region, rng: &mut WorldgenRng, x: i32, y: i32, z: i32) -> u16 {
        match self.get_optional(region, rng, x, y, z) {
            Some(s) => s,
            None => region.get_block(x, y, z),
        }
    }
}
