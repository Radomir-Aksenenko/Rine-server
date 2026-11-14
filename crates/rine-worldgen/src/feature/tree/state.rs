//! Block states for the tree port.
//!
//! The palette holds canonical strings (`name[k=v,...]`, keys sorted, bare name when there are no
//! properties). For every block the trees can write or change, all property combinations are interned up
//! front, so a property change (`setValue`) is a table lookup and a runtime write never builds a string.
//! Schemas come from `features.json` (`blockProps`: property order and default state from the 26.3 block
//! report).

use std::collections::HashMap;

use serde_json::Value;

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

/// Per palette id: `(schema, combination)`, `u32::MAX` when the state is not in a schema.
#[derive(Clone, Copy)]
struct Slot {
    schema: u32,
    combo: u32,
}

const NO_SLOT: Slot = Slot { schema: u32::MAX, combo: u32::MAX };

pub struct TreeStates {
    schemas: Vec<Schema>,
    by_name: HashMap<String, usize>,
    slots: Vec<Slot>,
    /// `water[id]`: `FluidTags.WATER` holds at this state (water block or `waterlogged=true`).
    water: Vec<bool>,
    /// `minecraft:vine` blocks.
    vine: Vec<bool>,
    /// Block name (no properties) of every palette id.
    base: Vec<String>,
}

/// Canonical state string, the same format the golden dump writes.
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
    /// Interns the states of every block in `features.json` `blockProps` whose name is in `wanted` (names
    /// are matched exactly, and blocks with a `distance` property are always included). `palette` and
    /// `palette_ids` are the decorator palette; new states are appended.
    pub fn build(
        block_props: &Value,
        wanted: &[String],
        palette: &mut Vec<String>,
        palette_ids: &mut HashMap<String, u16>,
    ) -> Result<Self, String> {
        let obj = block_props.as_object().ok_or("blockProps")?;
        let mut names: Vec<&String> = obj
            .iter()
            .filter(|(name, entry)| {
                wanted.iter().any(|w| w == *name)
                    || entry["properties"]
                        .as_object()
                        .is_some_and(|p| p.contains_key("distance"))
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
                let s = canonical(name, &props);
                let id = match palette_ids.get(&s) {
                    Some(&id) => id,
                    None => {
                        let id = palette.len() as u16;
                        palette.push(s.clone());
                        palette_ids.insert(s, id);
                        id
                    }
                };
                ids.push(id);
            }
            by_name.insert(name.clone(), schemas.len());
            schemas.push(Schema { name: name.clone(), keys, values, strides, ids, default: default_idx });
        }

        let mut slots = vec![NO_SLOT; palette.len()];
        for (si, s) in schemas.iter().enumerate() {
            for (combo, &id) in s.ids.iter().enumerate() {
                slots[id as usize] = Slot { schema: si as u32, combo: combo as u32 };
            }
        }
        let water = palette.iter().map(|s| is_water_state(s)).collect();
        let vine = palette.iter().map(|s| s.split('[').next() == Some("minecraft:vine")).collect();
        let base = palette.iter().map(|s| s.split('[').next().unwrap_or(s).to_string()).collect();
        Ok(TreeStates { schemas, by_name, slots, water, vine, base })
    }

    /// Block name of any palette id (`minecraft:oak_log`).
    #[inline]
    pub fn base_name(&self, id: u16) -> &str {
        &self.base[id as usize]
    }

    /// Schema lookup for a block name (`minecraft:oak_log`).
    pub fn has_block(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// Palette id for a block with the given properties; unspecified properties take the default value.
    /// Panics on a block or property the schema does not know: the caller must list it in `wanted`.
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

    /// `state.setValue(key, value)` on an interned state. Panics when the state is not in a schema.
    #[inline]
    pub fn with(&self, id: u16, key: &str, value: &str) -> u16 {
        let slot = self.slots[id as usize];
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
        let slot = self.slots[id as usize];
        if slot.schema == u32::MAX {
            return None;
        }
        let s = &self.schemas[slot.schema as usize];
        let ki = s.keys.iter().position(|k| k == key)?;
        let vi = (slot.combo as usize / s.strides[ki]) % s.values[ki].len();
        Some(&s.values[ki][vi])
    }

    /// Block name of an interned state (`minecraft:oak_log`).
    pub fn block_name(&self, id: u16) -> Option<&str> {
        let slot = self.slots[id as usize];
        (slot.schema != u32::MAX).then(|| self.schemas[slot.schema as usize].name.as_str())
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

/// `FluidTags.WATER` for a canonical state: the water block itself or a waterlogged block.
fn is_water_state(s: &str) -> bool {
    let name = s.split('[').next().unwrap_or(s);
    name == "minecraft:water" || s.contains("waterlogged=true")
}
