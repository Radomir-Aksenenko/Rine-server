//! Compiles the bundled `worldgen/material_rule` and `material_condition` JSON (overworld surface rules)
//! into arenas of `Rule` and `Cond` nodes. Density functions and noises are built through the shared
//! `density::build::Builder`, so they are the same instances as the router and aquifer.

use std::collections::HashMap;

use serde_json::Value;

use crate::density::build::{bundled, Builder};
use crate::overworld::blocks::{Blk, Interner};
use crate::random::Positional;

pub type RuleResult<T> = Result<T, String>;

/// `VerticalAnchor` resolved against the overworld `WorldGenerationContext`.
pub fn resolve_anchor(anchor: &Value, min_gen_y: i32, gen_depth: i32) -> RuleResult<i32> {
    if let Some(v) = anchor.get("absolute") {
        return Ok(v.as_i64().ok_or("absolute")? as i32);
    }
    if let Some(v) = anchor.get("above_bottom") {
        return Ok(min_gen_y + v.as_i64().ok_or("above_bottom")? as i32);
    }
    if let Some(v) = anchor.get("below_top") {
        return Ok(gen_depth - 1 + min_gen_y - v.as_i64().ok_or("below_top")? as i32);
    }
    Err(format!("unknown vertical anchor {anchor}"))
}

#[derive(Clone, Debug)]
pub enum Cond {
    Biome(Vec<u16>),
    NoiseThreshold { noise: usize, min: f64, max: f64, is_3d: bool },
    VerticalGradient { rng: Positional, true_at: i32, false_at: i32 },
    YAbove { anchor: i32, multiplier: i32, add_stone_depth: bool },
    Water { offset: i32, multiplier: i32, add_stone_depth: bool },
    Temperature,
    Steep,
    Hole,
    AbovePreliminary,
    StoneDepth { offset: i32, add_surface_depth: bool, secondary_range: i32, ceiling: bool },
    Not(u32),
}

#[derive(Clone, Debug)]
pub struct OreVein {
    pub ore: Blk,
    pub raw_ore: Blk,
    pub filler: Blk,
    pub raw_ore_chance: f32,
    pub density: u32,
    pub richness: u32,
    pub filler_gap: u32,
    pub rng: Positional,
}

#[derive(Clone, Debug)]
pub enum Rule {
    Block(Blk),
    Bandlands,
    Sequence(Vec<u32>),
    Condition { cond: u32, then: u32 },
    OreVein(OreVein),
}

pub struct RuleCompiler<'a> {
    pub builder: &'a mut Builder,
    pub interner: &'a mut Interner,
    pub biomes: &'a [String],
    pub root_random: Positional,
    pub min_gen_y: i32,
    pub gen_depth: i32,
    pub rules: Vec<Rule>,
    pub conds: Vec<Cond>,
    named_rules: HashMap<String, u32>,
    named_conds: HashMap<String, u32>,
}

fn strip(name: &str) -> RuleResult<&str> {
    name.strip_prefix("minecraft:")
        .ok_or_else(|| format!("expected a minecraft: id, got {name}"))
}

impl<'a> RuleCompiler<'a> {
    pub fn new(
        builder: &'a mut Builder,
        interner: &'a mut Interner,
        biomes: &'a [String],
        root_random: Positional,
        min_gen_y: i32,
        gen_depth: i32,
    ) -> Self {
        RuleCompiler {
            builder,
            interner,
            biomes,
            root_random,
            min_gen_y,
            gen_depth,
            rules: Vec::new(),
            conds: Vec::new(),
            named_rules: HashMap::new(),
            named_conds: HashMap::new(),
        }
    }

    /// `randomState.getOrCreateRandomFactory(name)` = `random.fromHashOf(name).forkPositional()`.
    pub fn factory(&self, name: &str) -> Positional {
        let mut rng = self.root_random.from_hash_of(name);
        rng.fork_positional()
    }

    fn biome_index(&self, name: &str) -> RuleResult<u16> {
        self.biomes
            .iter()
            .position(|b| b == name)
            .map(|i| i as u16)
            .ok_or_else(|| format!("biome {name} is not in the overworld parameter list"))
    }

    fn push_rule(&mut self, rule: Rule) -> u32 {
        self.rules.push(rule);
        (self.rules.len() - 1) as u32
    }

    fn push_cond(&mut self, cond: Cond) -> u32 {
        self.conds.push(cond);
        (self.conds.len() - 1) as u32
    }

    /// A `MaterialRule` value: an inline object or a `minecraft:` reference to a rule file.
    pub fn rule(&mut self, v: &Value) -> RuleResult<u32> {
        if let Some(name) = v.as_str() {
            if let Some(&id) = self.named_rules.get(name) {
                return Ok(id);
            }
            let rel = format!("material_rule/{}.json", strip(name)?);
            let json: Value = serde_json::from_str(bundled(&rel)?).map_err(|e| format!("{rel}: {e}"))?;
            let id = self.rule(&json)?;
            self.named_rules.insert(name.to_string(), id);
            return Ok(id);
        }
        let kind = v["type"].as_str().ok_or_else(|| format!("rule without type: {v}"))?;
        match kind {
            "minecraft:block" => {
                let id = v["result_state"].as_str().ok_or("result_state")?;
                let blk = self.interner.get(id);
                Ok(self.push_rule(Rule::Block(blk)))
            }
            "minecraft:bandlands" => Ok(self.push_rule(Rule::Bandlands)),
            "minecraft:sequence" => {
                let mut kids = Vec::new();
                for item in v["sequence"].as_array().ok_or("sequence")? {
                    kids.push(self.rule(item)?);
                }
                Ok(self.push_rule(Rule::Sequence(kids)))
            }
            "minecraft:condition" => {
                let cond = self.condition(&v["if_true"])?;
                let then = self.rule(&v["then_run"])?;
                Ok(self.push_rule(Rule::Condition { cond, then }))
            }
            "minecraft:ore_vein" => {
                let ore = self.interner.get(v["ore_block"].as_str().ok_or("ore_block")?);
                let raw_ore = self.interner.get(v["raw_ore_block"].as_str().ok_or("raw_ore_block")?);
                let filler = self.interner.get(v["filler_block"].as_str().ok_or("filler_block")?);
                let raw_ore_chance = v["raw_ore_chance"].as_f64().ok_or("raw_ore_chance")? as f32;
                let density = self.builder.df(&v["density"])?;
                let richness = self.builder.df(&v["richness"])?;
                let filler_gap = self.builder.df(&v["filler_gap"])?;
                let rng = self.factory("minecraft:ore");
                Ok(self.push_rule(Rule::OreVein(OreVein {
                    ore,
                    raw_ore,
                    filler,
                    raw_ore_chance,
                    density,
                    richness,
                    filler_gap,
                    rng,
                })))
            }
            other => Err(format!("unsupported material rule {other}")),
        }
    }

    /// A `MaterialCondition` value: an inline object or a `minecraft:` reference.
    pub fn condition(&mut self, v: &Value) -> RuleResult<u32> {
        if let Some(name) = v.as_str() {
            if let Some(&id) = self.named_conds.get(name) {
                return Ok(id);
            }
            let rel = format!("material_condition/{}.json", strip(name)?);
            let json: Value = serde_json::from_str(bundled(&rel)?).map_err(|e| format!("{rel}: {e}"))?;
            let id = self.condition(&json)?;
            self.named_conds.insert(name.to_string(), id);
            return Ok(id);
        }
        let kind = v["type"].as_str().ok_or_else(|| format!("condition without type: {v}"))?;
        let int = |key: &str| -> RuleResult<i64> { v[key].as_i64().ok_or_else(|| format!("{key} in {kind}")) };
        let cond = match kind {
            "minecraft:biome" => {
                let list: Vec<&str> = match &v["biome_is"] {
                    Value::String(s) => vec![s.as_str()],
                    Value::Array(a) => a
                        .iter()
                        .map(|x| x.as_str().ok_or_else(|| "biome id".to_string()))
                        .collect::<RuleResult<_>>()?,
                    other => return Err(format!("biome_is {other}")),
                };
                let mut ids = Vec::new();
                for name in list {
                    ids.push(self.biome_index(name)?);
                }
                Cond::Biome(ids)
            }
            "minecraft:noise_threshold" => {
                let name = v["noise"].as_str().ok_or("noise")?;
                let noise = self.builder.noise(name)?;
                Cond::NoiseThreshold {
                    noise,
                    min: v["min_threshold"].as_f64().ok_or("min_threshold")?,
                    max: v["max_threshold"].as_f64().ok_or("max_threshold")?,
                    is_3d: v["is_3d"].as_bool().unwrap_or(false),
                }
            }
            "minecraft:vertical_gradient" => {
                let name = v["random_name"].as_str().ok_or("random_name")?;
                let true_at = resolve_anchor(&v["true_at_and_below"], self.min_gen_y, self.gen_depth)?;
                let false_at = resolve_anchor(&v["false_at_and_above"], self.min_gen_y, self.gen_depth)?;
                Cond::VerticalGradient {
                    rng: self.factory(name),
                    true_at,
                    false_at,
                }
            }
            "minecraft:y_above" => Cond::YAbove {
                anchor: resolve_anchor(&v["anchor"], self.min_gen_y, self.gen_depth)?,
                multiplier: int("surface_depth_multiplier")? as i32,
                add_stone_depth: v["add_stone_depth"].as_bool().ok_or("add_stone_depth")?,
            },
            "minecraft:water" => Cond::Water {
                offset: int("offset")? as i32,
                multiplier: int("surface_depth_multiplier")? as i32,
                add_stone_depth: v["add_stone_depth"].as_bool().ok_or("add_stone_depth")?,
            },
            "minecraft:temperature" => Cond::Temperature,
            "minecraft:steep" => Cond::Steep,
            "minecraft:hole" => Cond::Hole,
            "minecraft:above_preliminary_surface" => Cond::AbovePreliminary,
            "minecraft:stone_depth" => Cond::StoneDepth {
                offset: int("offset")? as i32,
                add_surface_depth: v["add_surface_depth"].as_bool().ok_or("add_surface_depth")?,
                secondary_range: int("secondary_depth_range")? as i32,
                ceiling: match v["surface_type"].as_str().ok_or("surface_type")? {
                    "ceiling" => true,
                    "floor" => false,
                    other => return Err(format!("surface_type {other}")),
                },
            },
            "minecraft:not" => {
                let inner = self.condition(&v["invert"])?;
                Cond::Not(inner)
            }
            other => return Err(format!("unsupported material condition {other}")),
        };
        Ok(self.push_cond(cond))
    }
}
