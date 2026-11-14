//! Block state palette for decoration. Ids index `states` (full state strings `minecraft:id[k=v,...]`,
//! the default state is the bare name). Every state carries its block index and fluid kind, so block
//! predicates and `Block.is(...)` checks are array lookups by id.
//!
//! The palette grows while the features are compiled; `Tags` is built after compilation, so the tables
//! always cover the final palette.

use std::collections::HashMap;

/// `FluidState.getType()` reduced to the fluids the ported features reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fluid {
    Empty,
    /// Source water (`Fluids.WATER`).
    Water,
    /// Flowing water (`Fluids.FLOWING_WATER`): a water block with level 1..=15.
    FlowingWater,
    Lava,
    FlowingLava,
}

pub struct Palette {
    states: Vec<String>,
    state_index: HashMap<String, u16>,
    block_of: Vec<u16>,
    fluid_of: Vec<Fluid>,
    blocks: Vec<String>,
    block_index: HashMap<String, u16>,
    /// Default state string per plain block id (`block_states.txt`); blocks without an entry use the bare id.
    defaults: HashMap<String, String>,
}

/// Splits `name[props]` into the block name and the raw property list (without brackets).
fn split_state(state: &str) -> (&str, Option<&str>) {
    match state.find('[') {
        Some(i) => (&state[..i], Some(state[i + 1..].trim_end_matches(']'))),
        None => (state, None),
    }
}

impl Palette {
    /// Starts from the overworld palette (`names` in id order, all distinct).
    pub fn new(names: &[String], defaults: HashMap<String, String>) -> Self {
        let mut p = Palette {
            states: Vec::with_capacity(names.len()),
            state_index: HashMap::new(),
            block_of: Vec::new(),
            fluid_of: Vec::new(),
            blocks: Vec::new(),
            block_index: HashMap::new(),
            defaults,
        };
        for n in names {
            p.intern(n);
        }
        p
    }

    /// Interns a full state string and returns its id.
    pub fn intern(&mut self, state: &str) -> u16 {
        if let Some(&id) = self.state_index.get(state) {
            return id;
        }
        let id = self.states.len();
        assert!(id < u16::MAX as usize, "palette overflow");
        let (name, props) = split_state(state);
        let block = self.block(name);
        let fluid = fluid_of(name, props);
        self.states.push(state.to_string());
        self.state_index.insert(state.to_string(), id as u16);
        self.block_of.push(block);
        self.fluid_of.push(fluid);
        id as u16
    }

    /// Interns a block name (no properties) and returns its block index.
    pub fn block(&mut self, name: &str) -> u16 {
        if let Some(&i) = self.block_index.get(name) {
            return i;
        }
        let i = self.blocks.len() as u16;
        self.blocks.push(name.to_string());
        self.block_index.insert(name.to_string(), i);
        i
    }

    /// Default state of a block id (`minecraft:grass_block` -> `minecraft:grass_block[snowy=false]`).
    pub fn default_state(&mut self, id: &str) -> u16 {
        let s = self.defaults.get(id).cloned().unwrap_or_else(|| id.to_string());
        self.intern(&s)
    }

    /// Parses a state given by id and properties, as in the feature configs (`{id, properties}`).
    pub fn state_from_parts(&mut self, id: &str, props: &[(&str, &str)]) -> u16 {
        if props.is_empty() {
            return self.default_state(id);
        }
        let mut s = String::from(id);
        s.push('[');
        for (i, (k, v)) in props.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(k);
            s.push('=');
            s.push_str(v);
        }
        s.push(']');
        self.intern(&s)
    }

    /// `state.setValue(property, value)` for a property that the state already has, as a palette id.
    /// `None` when the state has no such property (vanilla keeps the state unchanged in that case).
    pub fn with_property(&mut self, state: u16, key: &str, value: &str) -> Option<u16> {
        let s = self.states[state as usize].clone();
        let (name, props) = split_state(&s);
        let props = props?;
        let mut found = false;
        let parts: Vec<String> = props
            .split(',')
            .map(|kv| match kv.split_once('=') {
                Some((k, _)) if k == key => {
                    found = true;
                    format!("{k}={value}")
                }
                _ => kv.to_string(),
            })
            .collect();
        if !found {
            return None;
        }
        Some(self.intern(&format!("{name}[{}]", parts.join(","))))
    }

    /// Block index of a palette state (`BlockState.getBlock()`).
    pub fn block_of(&self) -> &[u16] {
        &self.block_of
    }

    /// Fluid kind per palette state (`FluidState.getType()`).
    pub fn fluid_of(&self) -> &[Fluid] {
        &self.fluid_of
    }

    /// Block index of a block name, if it was ever interned.
    pub fn block_id(&self, name: &str) -> Option<u16> {
        self.block_index.get(name).copied()
    }

    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    pub fn block_name(&self, block: u16) -> &str {
        &self.blocks[block as usize]
    }

    /// All palette state strings in id order.
    pub fn states(&self) -> &[String] {
        &self.states
    }
}

/// The fluid of a state: water blocks and `waterlogged=true` states carry water, lava carries lava.
fn fluid_of(name: &str, props: Option<&str>) -> Fluid {
    let level = props.and_then(|p| p.split(',').find_map(|kv| kv.strip_prefix("level=")).and_then(|v| v.parse::<u32>().ok()));
    let waterlogged = props.is_some_and(|p| p.split(',').any(|kv| kv == "waterlogged=true"));
    match name {
        "minecraft:water" => match level {
            Some(l) if l != 0 => Fluid::FlowingWater,
            _ => Fluid::Water,
        },
        "minecraft:lava" => match level {
            Some(l) if l != 0 => Fluid::FlowingLava,
            _ => Fluid::Lava,
        },
        _ if waterlogged => Fluid::Water,
        _ => Fluid::Empty,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> Palette {
        let mut defaults = HashMap::new();
        defaults.insert("minecraft:kelp".to_string(), "minecraft:kelp[age=0]".to_string());
        Palette::new(&["minecraft:air".to_string(), "minecraft:water[level=0]".to_string()], defaults)
    }

    #[test]
    fn property_replacement_keeps_other_keys() {
        let mut p = palette();
        let k = p.intern("minecraft:sea_pickle[pickles=1,waterlogged=true]");
        let k3 = p.with_property(k, "pickles", "3").unwrap();
        assert_eq!(p.states()[k3 as usize], "minecraft:sea_pickle[pickles=3,waterlogged=true]");
        assert!(p.with_property(k, "age", "1").is_none());
    }

    #[test]
    fn fluids_and_blocks() {
        let mut p = palette();
        let w = p.intern("minecraft:water[level=0]");
        assert_eq!(p.fluid_of()[w as usize], Fluid::Water);
        assert_eq!(p.block_name(p.block_of()[w as usize]), "minecraft:water");
        let kelp = p.default_state("minecraft:kelp");
        assert_eq!(p.states()[kelp as usize], "minecraft:kelp[age=0]");
        assert_eq!(p.fluid_of()[kelp as usize], Fluid::Empty);
    }
}
