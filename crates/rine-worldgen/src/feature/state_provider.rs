//! `BlockStateProvider` (`levelgen/feature/stateproviders`): simple, weighted, randomized int property and
//! rule based. Every provider compiles to palette ids; the random draws happen at placement time in the
//! same order as the Java `getState`/`getOptionalState`.

use serde_json::Value;

use super::placement::{parse_int_provider, sample_int, IntProvider};
use super::predicate::{self, BlockPredicate};
use super::region::Region;
use super::rng::WorldgenRng;
use super::state::Palette;
use crate::noise::{NoiseStack, NormalNoise, Normalization};
use crate::random::{AnyRng, Legacy};

#[derive(Clone, Debug)]
pub enum StateProvider {
    /// `SimpleStateProvider`.
    Simple(u16),
    /// `WeightedStateProvider`: `WeightedList.getRandomOrThrow` (one `nextInt(totalWeight)`).
    Weighted { entries: Vec<(u16, i32)>, total: i32 },
    /// `RandomizedIntStateProvider`: `source.setValue(property, values.sample())`.
    /// `table[v - min]` is the palette id for each value the provider can sample.
    RandomizedInt { source: Box<StateProvider>, values: IntProvider, min: i32, tables: Vec<(u16, Vec<u16>)> },
    /// `RuleBasedStateProvider`: the first rule whose predicate holds and whose provider answers, else the
    /// fallback. `getState` falls back to the block already at the position.
    RuleBased { rules: Vec<(BlockPredicate, StateProvider)>, fallback: Option<Box<StateProvider>> },
    Noise { noise: NoiseStack, scale: f32, states: Vec<u16>, slow: Option<SlowNoise> },
    NoiseThreshold { noise: NoiseStack, scale: f32, threshold: f32, high_chance: f32,
        default: u16, low: Vec<u16>, high: Vec<u16> },
}

#[derive(Clone, Debug)]
pub struct SlowNoise {
    noise: NoiseStack,
    scale: f32,
    min: i32,
    max: i32,
}

fn compile_noise(v: &Value, seed: i64) -> Result<NoiseStack, String> {
    let octave = v["base_octave"].as_i64().ok_or("base_octave")? as i32;
    let count = v["octave_count"].as_u64().unwrap_or(1) as usize;
    let amplitude = v["base_amplitude"].as_f64().unwrap_or(1.0);
    let normalize = match &v["normalize"] {
        Value::Null | Value::Bool(true) => Normalization::Enabled,
        Value::Bool(false) => Normalization::Disabled,
        Value::String(s) if s == "legacy" => Normalization::Legacy,
        _ => return Err("invalid normalization".into()),
    };
    let modifiers = v["amplitude_modifiers"].as_array().map(|values| {
        values.iter().map(|n| n.as_f64().ok_or("amplitude modifier")).collect::<Result<Vec<_>, _>>()
    }).transpose()?.unwrap_or_default();
    if !(1..=32).contains(&count) || (!modifiers.is_empty() && modifiers.len() != count) {
        return Err("invalid noise octave count".into());
    }
    Ok(NormalNoise::new(amplitude, octave, count, normalize, &modifiers)
        .create(&mut AnyRng::Legacy(Legacy::new(seed))))
}

fn state_list(v: &Value, pal: &mut Palette) -> Result<Vec<u16>, String> {
    let states = v.as_array().filter(|a| !a.is_empty()).ok_or("empty or missing provider states")?;
    states.iter().map(|s| state_of(s, pal)).collect()
}

#[inline]
fn noise_index(value: f32, len: usize) -> usize {
    (((1.0 + value) / 2.0).clamp(0.0, 0.9999) * len as f32) as usize
}

#[inline]
fn sample_noise(noise: &NoiseStack, scale: f32, x: i32, y: i32, z: i32) -> f32 {
    let scale = scale as f64;
    noise.get_value(x as f64 * scale, y as f64 * scale, z as f64 * scale)
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
        "minecraft:noise" | "minecraft:dual_noise" | "minecraft:noise_threshold" => {
            let seed = v["seed"].as_i64().ok_or("noise provider seed")?;
            let noise = compile_noise(&v["noise"], seed)?;
            let scale = v["scale"].as_f64().ok_or("noise scale")? as f32;
            if ty == "minecraft:noise_threshold" {
                return Ok(StateProvider::NoiseThreshold {
                    noise, scale,
                    threshold: v["threshold"].as_f64().ok_or("threshold")? as f32,
                    high_chance: v["high_chance"].as_f64().ok_or("high_chance")? as f32,
                    default: state_of(&v["default_state"], pal)?,
                    low: state_list(&v["low_states"], pal)?,
                    high: state_list(&v["high_states"], pal)?,
                });
            }
            let slow = if ty == "minecraft:dual_noise" {
                let min = v["variety"][0].as_i64().ok_or("variety min")? as i32;
                let max = v["variety"][1].as_i64().ok_or("variety max")? as i32;
                if min < 1 || max > 64 || min > max {
                    return Err("invalid dual noise variety".into());
                }
                Some(SlowNoise {
                    noise: compile_noise(&v["slow_noise"], seed)?,
                    scale: v["slow_scale"].as_f64().ok_or("slow_scale")? as f32,
                    min, max,
                })
            } else {
                None
            };
            Ok(StateProvider::Noise { noise, scale, states: state_list(&v["states"], pal)?, slow })
        }
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
            let source = parse(&v["source"], tag_names, pal)?;
            let candidates = match &source {
                StateProvider::Simple(s) => vec![*s],
                StateProvider::Weighted { entries, .. } => entries.iter().map(|&(s, _)| s).collect(),
                StateProvider::Noise { states, .. } => states.clone(),
                StateProvider::NoiseThreshold { default, low, high, .. } => {
                    std::iter::once(*default).chain(low.iter().chain(high).copied()).collect()
                }
                _ => return Err("unsupported source for randomized_int".into()),
            };
            let property = v["property"].as_str().ok_or("property")?;
            let values = parse_int_provider(&v["values"])?;
            let (min, max) = values.bounds();
            let mut tables = Vec::new();
            for state in candidates {
                if pal.with_property(state, property, &min.to_string()).is_none() {
                    continue;
                }
                let table = (min..=max).map(|n| pal.with_property(state, property, &n.to_string()).unwrap_or(state)).collect();
                tables.push((state, table));
            }
            Ok(StateProvider::RandomizedInt { source: Box::new(source), values, min, tables })
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
            StateProvider::Noise { noise, scale, states, slow } => {
                let value = sample_noise(noise, *scale, x, y, z);
                let index = if let Some(slow) = slow {
                    let sample = |x, z| slow.noise.get_value(
                        (x as f32 * slow.scale) as f64,
                        (y as f32 * slow.scale) as f64,
                        (z as f32 * slow.scale) as f64,
                    );
                    let t = ((sample(x, z) as f64 + 1.0) / 2.0).clamp(0.0, 1.0);
                    let variety = (slow.min as f64 + t * (slow.max + 1 - slow.min) as f64) as usize;
                    // Only the selected slow-noise entry is observable.
                    let i = noise_index(value, variety) as i32;
                    noise_index(sample(x.wrapping_add(i * 54545), z.wrapping_add(i * 34234)), states.len())
                } else {
                    noise_index(value, states.len())
                };
                Some(states[index])
            }
            StateProvider::NoiseThreshold { noise, scale, threshold, high_chance, default, low, high } => {
                let states = if sample_noise(noise, *scale, x, y, z) < *threshold {
                    low
                } else if rng.next_float() < *high_chance {
                    high
                } else {
                    return Some(*default);
                };
                Some(states[rng.next_int_bound(states.len() as i32) as usize])
            }
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
            StateProvider::RandomizedInt { source, values, min, tables } => {
                let state = source.get(region, rng, x, y, z);
                Some(tables.iter().find(|(s, _)| *s == state).map_or(state, |(_, table)| {
                    table[(sample_int(values, rng) - min) as usize]
                }))
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
