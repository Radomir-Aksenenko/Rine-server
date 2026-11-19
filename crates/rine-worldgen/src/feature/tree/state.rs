//! Dense block-state table for the tree port, on top of the shared decoration [`Palette`].
//!
//! For every block a tree can write or change (its schema comes from `features.json` `blockProps`: property
//! order and default state from the 26.3 block report), all property combinations are interned up front.
//! A property change (`setValue`) is then an array lookup on the palette id, with no string work.
//! Interning goes through `Palette::intern`, so ids agree with every other feature that uses the palette.

use std::collections::HashMap;

use serde_json::Value;

use crate::feature::state::{Fluid, Palette};

/// One block with its property schema and every combination interned in the palette.
struct Schema {
    name: String,
    /// Property names, sorted (canonical order).
    keys: Vec<String>,
    /// Allowed values per key, in report order.
    values: Vec<Vec<String>>,
    /// Mixed-radix stride per key (`combo = sum(value_index[k] * stride[k])`).
    strides: Vec<usize>,
    /// Palette id per combination.
    ids: Vec<u16>,
    /// Default value index per key.
    default: Vec<usize>,
}

/// Per palette id: `(schema, combination)`, `NO_SLOT` when the state is not in a schema.
#[derive(Clone, Copy)]
struct Slot {
    schema: u32,
    combo: u32,
}

const NO_SLOT: Slot = Slot { schema: u32::MAX, combo: u32::MAX };

pub struct TreeStates {
    schemas: Vec<Schema>,
    by_name: HashMap<String, usize>,
    /// Filled by [`TreeStates::finish`].
    slots: Vec<Slot>,
    water: Vec<bool>,
    vine: Vec<bool>,
    base: Vec<String>,
}

/// Canonical state string, the format of `Palette` and the golden dump.
pub fn canonical(name: &str, props: &[(String, String)]) -> String {
    if props.is_empty() {
        return name.to_string();
    }
    let mut sorted: Vec<&(String, String)> = props.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let body: Vec<String> = sorted.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{name}[{}]", body.join(","))
}

impl TreeStates {
    /// Interns every combination of the blocks in `wanted` (names present in `block_props`) and of every block
    /// with a `distance` property (leaves). Call [`TreeStates::finish`] once compilation is done.
    pub fn build(block_props: &Value, wanted: &[String], pal: &mut Palette) -> Result<Self, String> {
        let obj = block_props.as_object().ok_or("blockProps")?;
        let mut names: Vec<&String> = obj
            .iter()
            .filter(|(name, entry)| {
                wanted.iter().any(|w| w == *name) || entry["properties"].as_object().is_some_and(|p| p.contains_key("distance"))
            })
            .map(|(name, _)| name)
            .collect();
        names.sort();

        let mut schemas = Vec::with_capacity(names.len());
        let mut by_name = HashMap::new();
        for name in names {
            let entry = &obj[name];
            let props = entry["properties"].as_object().ok_or_else(|| format!("{name}: properties"))?;
            let default = entry["default"].as_object().ok_or_else(|| format!("{name}: default"))?;
            let mut keys: Vec<String> = props.keys().cloned().collect();
            keys.sort();
            let values: Vec<Vec<String>> = keys
                .iter()
                .map(|k| {
                    props[k]
                        .as_array()
                        .expect("property values")
                        .iter()
                        .map(|v| v.as_str().expect("value").to_string())
                        .collect()
                })
                .collect();
            let default_idx: Vec<usize> = keys
                .iter()
                .enumerate()
                .map(|(i, k)| {
                    let want = default.get(k).and_then(|v| v.as_str()).ok_or_else(|| format!("{name}: default {k}"))?;
                    values[i].iter().position(|v| v == want).ok_or_else(|| format!("{name}: default value {want}"))
                })
                .collect::<Result<_, String>>()?;
            let mut strides = Vec::with_capacity(keys.len());
            let mut total = 1usize;
            for v in &values {
                strides.push(total);
                total *= v.len();
            }
            let mut ids = Vec::with_capacity(total);
            for combo in 0..total {
                let props: Vec<(String, String)> = keys
                    .iter()
                    .enumerate()
                    .map(|(i, k)| {
                        let vi = (combo / strides[i]) % values[i].len();
                        (k.clone(), values[i][vi].clone())
                    })
                    .collect();
                ids.push(pal.intern(&canonical(name, &props)));
            }
            by_name.insert(name.clone(), schemas.len());
            schemas.push(Schema { name: name.clone(), keys, values, strides, ids, default: default_idx });
        }
        Ok(TreeStates { schemas, by_name, slots: Vec::new(), water: Vec::new(), vine: Vec::new(), base: Vec::new() })
    }

    /// Computes the per-state tables for the final palette. Must run after every state is interned.
    pub fn finish(&mut self, pal: &Palette) {
        let n = pal.states().len();
        self.slots = vec![NO_SLOT; n];
        for (si, s) in self.schemas.iter().enumerate() {
            for (combo, &id) in s.ids.iter().enumerate() {
                self.slots[id as usize] = Slot { schema: si as u32, combo: combo as u32 };
            }
        }
        self.water = (0..n).map(|i| matches!(pal.fluid_of()[i], Fluid::Water | Fluid::FlowingWater)).collect();
        self.base = pal.states().iter().map(|s| s.split('[').next().unwrap_or(s).to_string()).collect();
        self.vine = self.base.iter().map(|b| b == "minecraft:vine").collect();
    }

    pub fn has_block(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// Palette id for a block with the given properties; unspecified properties take the default value.
    /// Panics on a block or property the schema does not know.
    pub fn id(&self, name: &str, props: &[(&str, &str)]) -> u16 {
        let si = *self.by_name.get(name).unwrap_or_else(|| panic!("tree block {name} has no schema"));
        let s = &self.schemas[si];
        let mut combo = 0usize;
        for (i, k) in s.keys.iter().enumerate() {
            let vi = match props.iter().find(|(pk, _)| pk == k) {
                Some((_, v)) => s.values[i].iter().position(|x| x == v).unwrap_or_else(|| panic!("{name}: bad {k}={v}")),
                None => s.default[i],
            };
            combo += vi * s.strides[i];
        }
        for (pk, _) in props {
            assert!(s.keys.iter().any(|k| k == pk), "{name} has no property {pk}");
        }
        s.ids[combo]
    }

    #[inline]
    fn slot(&self, id: u16) -> Slot {
        self.slots.get(id as usize).copied().unwrap_or(NO_SLOT)
    }

    /// `state.setValue(key, value)` on an interned state. Panics when the state has no schema.
    #[inline]
    pub fn with(&self, id: u16, key: &str, value: &str) -> u16 {
        let slot = self.slot(id);
        assert!(slot.schema != u32::MAX, "state {id} has no schema for setValue({key})");
        let s = &self.schemas[slot.schema as usize];
        let ki = s.keys.iter().position(|k| k == key).unwrap_or_else(|| panic!("{} has no property {key}", s.name));
        let stride = s.strides[ki];
        let cur = (slot.combo as usize / stride) % s.values[ki].len();
        let vi = s.values[ki].iter().position(|v| v == value).unwrap_or_else(|| panic!("{}: bad {key}={value}", s.name));
        let combo = slot.combo as usize - cur * stride + vi * stride;
        s.ids[combo]
    }

    /// Value of a property of an interned state, `None` when the state has no such property.
    pub fn get(&self, id: u16, key: &str) -> Option<&str> {
        let slot = self.slot(id);
        if slot.schema == u32::MAX {
            return None;
        }
        let s = &self.schemas[slot.schema as usize];
        let ki = s.keys.iter().position(|k| k == key)?;
        let vi = (slot.combo as usize / s.strides[ki]) % s.values[ki].len();
        Some(&s.values[ki][vi])
    }

    /// Block name (no properties) of any palette id.
    #[inline]
    pub fn base_name(&self, id: u16) -> &str {
        &self.base[id as usize]
    }

    #[inline]
    pub fn is_water(&self, id: u16) -> bool {
        self.water[id as usize]
    }

    #[inline]
    pub fn is_vine(&self, id: u16) -> bool {
        self.vine[id as usize]
    }
}

/// Block names the tree configs and their block state providers use, plus the blocks the decorators write.
/// Only names present in `blockProps` get states; `TreeStates::build` adds every block with a `distance`
/// property on top of this list.
pub fn tree_wanted_blocks(bundle: &Value) -> Vec<String> {
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) => {
                let n = s.strip_prefix('#').unwrap_or(s);
                if n.starts_with("minecraft:") && !out.iter().any(|x| x == n) {
                    out.push(n.to_string());
                }
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            Value::Object(o) => o.values().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut out: Vec<String> = Vec::new();
    if let Some(cfgs) = bundle["configured"].as_object() {
        for cfg in cfgs.values() {
            if matches!(cfg["type"].as_str(), Some("minecraft:tree") | Some("minecraft:fallen_tree")) {
                walk(cfg, &mut out);
            }
        }
    }
    if let Some(provs) = bundle["blockStateProviders"].as_object() {
        for p in provs.values() {
            walk(p, &mut out);
        }
    }
    for n in ["minecraft:vine", "minecraft:cocoa", "minecraft:bee_nest", "minecraft:creaking_heart", "minecraft:pale_hanging_moss", "minecraft:shelf_mushroom"] {
        if !out.iter().any(|x| x == n) {
            out.push(n.to_string());
        }
    }
    out
}
