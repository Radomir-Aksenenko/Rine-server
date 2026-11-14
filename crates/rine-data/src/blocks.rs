//! Block states. State ids follow vanilla: for each block, properties are
//! ordered by name and the last property varies fastest, so a state id is
//! `min + sum(value_index * stride)`. This lets us read/modify properties
//! with pure arithmetic: no hash lookups on the hot path.

use crate::Registries;
use ahash::AHashMap;
use serde::Deserialize;

#[derive(Deserialize)]
struct RawBlock {
    n: String,
    p: Vec<(String, Vec<String>)>,
    min: u32,
    d: u32,
}

#[derive(Debug, Clone)]
pub struct Property {
    pub name: String,
    pub values: Vec<String>,
    pub stride: u32,
}

#[derive(Debug, Clone)]
pub struct BlockInfo {
    pub id: u32,
    pub name: String,
    pub props: Vec<Property>,
    pub min_state: u32,
    pub state_count: u32,
    pub default_state: u32,
}

/// Per-state flags (filled from the datadump when available).
pub mod flag {
    pub const AIR: u16 = 1;
    pub const OPAQUE: u16 = 2; // canOcclude: full opaque cube for culling/light
    pub const SOLID: u16 = 4; // blocks motion
    pub const FULL_CUBE: u16 = 8; // collision is exactly one full block
    pub const REPLACEABLE: u16 = 16;
    pub const RANDOM_TICK: u16 = 32;
    pub const TOOL_REQUIRED: u16 = 64;
    pub const WATER: u16 = 128;
    pub const LAVA: u16 = 256;
    pub const FLUID_SOURCE: u16 = 512;
    pub const NO_COLLISION: u16 = 1024;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StateInfo {
    pub block: u16,
    pub flags: u16,
    pub light_emit: u8,
    pub light_block: u8,
    pub fluid_level: u8,
    pub hardness: f32,
    pub resistance: f32,
    /// Index into `Blocks::shapes` (collision boxes).
    pub shape: u32,
    /// Item id of block.asItem() (0 = air).
    pub item: u32,
}

pub struct Blocks {
    pub blocks: Vec<BlockInfo>,
    pub states: Vec<StateInfo>,
    pub by_name: AHashMap<String, u32>,
    /// Distinct collision shapes, each a list of AABBs in block-local coords.
    pub shapes: Vec<Vec<[f32; 6]>>,
}

impl Blocks {
    pub fn load(regs: &Registries) -> Self {
        let raw: Vec<RawBlock> = serde_json::from_str(include_str!("../assets/blocks.json")).unwrap();
        let mut blocks = Vec::with_capacity(raw.len());
        let mut by_name = AHashMap::with_capacity(raw.len());
        let mut total = 0;
        for (i, b) in raw.into_iter().enumerate() {
            let mut props: Vec<Property> =
                b.p.into_iter().map(|(name, values)| Property { name, values, stride: 1 }).collect();
            let mut acc = 1u32;
            for p in props.iter_mut().rev() {
                p.stride = acc;
                acc *= p.values.len() as u32;
            }
            total = total.max(b.min + acc);
            by_name.insert(b.n.clone(), i as u32);
            blocks.push(BlockInfo { id: i as u32, name: b.n, props, min_state: b.min, state_count: acc, default_state: b.d });
        }
        let mut states = vec![StateInfo::default(); total as usize];
        for b in &blocks {
            for s in b.min_state..b.min_state + b.state_count {
                states[s as usize].block = b.id as u16;
            }
        }
        let mut me = Self { blocks, states, by_name, shapes: vec![Vec::new(), vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]] };
        me.load_ext(regs);
        me
    }

    fn load_ext(&mut self, regs: &Registries) {
        // Extended per-state data from the vanilla datadump (optional asset).
        #[derive(Deserialize)]
        struct Ext {
            id: u32,
            air: bool,
            opaque: bool,
            light: u8,
            #[serde(rename = "lightBlock")]
            light_block: u8,
            solid: bool,
            collision: Vec<[f64; 6]>,
            hardness: f32,
            resistance: f32,
            #[serde(rename = "toolRequired")]
            tool_required: bool,
            replaceable: bool,
            fluid: String,
            #[serde(rename = "fluidLevel")]
            fluid_level: u8,
            #[serde(rename = "fluidSource")]
            fluid_source: bool,
            #[serde(rename = "randomTicks")]
            random_ticks: bool,
            item: String,
        }
        let text = include_str!("../assets/blocks_ext.json");
        if text.trim().is_empty() {
            return;
        }
        let ext: Vec<Ext> = serde_json::from_str(text).expect("blocks_ext.json");
        let mut shape_ids: AHashMap<Vec<[u32; 6]>, u32> = AHashMap::new();
        shape_ids.insert(Vec::new(), 0);
        shape_ids.insert(vec![[0f32.to_bits(), 0f32.to_bits(), 0f32.to_bits(), 1f32.to_bits(), 1f32.to_bits(), 1f32.to_bits()]], 1);
        for e in ext {
            let s = &mut self.states[e.id as usize];
            let mut f = 0;
            if e.air { f |= flag::AIR; }
            if e.opaque { f |= flag::OPAQUE; }
            if e.solid { f |= flag::SOLID; }
            if e.replaceable { f |= flag::REPLACEABLE; }
            if e.random_ticks { f |= flag::RANDOM_TICK; }
            if e.tool_required { f |= flag::TOOL_REQUIRED; }
            if e.fluid_source { f |= flag::FLUID_SOURCE; }
            match e.fluid.as_str() {
                "water" => f |= flag::WATER,
                "lava" => f |= flag::LAVA,
                _ => {}
            }
            let boxes: Vec<[f32; 6]> = e.collision.iter().map(|b| b.map(|v| v as f32)).collect();
            if boxes.is_empty() { f |= flag::NO_COLLISION; }
            if boxes.len() == 1 && boxes[0] == [0.0, 0.0, 0.0, 1.0, 1.0, 1.0] { f |= flag::FULL_CUBE; }
            let key: Vec<[u32; 6]> = boxes.iter().map(|b| b.map(f32::to_bits)).collect();
            let next = self.shapes.len() as u32;
            let sid = *shape_ids.entry(key).or_insert_with(|| next);
            if sid == next {
                self.shapes.push(boxes);
            }
            s.flags = f;
            s.light_emit = e.light;
            s.light_block = e.light_block;
            s.fluid_level = e.fluid_level;
            s.hardness = e.hardness;
            s.resistance = e.resistance;
            s.shape = sid;
            s.item = regs.id("minecraft:item", &e.item).unwrap_or(0);
        }
    }
}

impl Blocks {
    #[inline]
    pub fn state(&self, id: u16) -> &StateInfo {
        &self.states[id as usize]
    }
    #[inline]
    pub fn block_of(&self, state: u16) -> &BlockInfo {
        &self.blocks[self.states[state as usize].block as usize]
    }
    pub fn block(&self, name: &str) -> Option<&BlockInfo> {
        let n = if name.contains(':') { name.to_string() } else { format!("minecraft:{name}") };
        self.by_name.get(&n).map(|i| &self.blocks[*i as usize])
    }
    /// Default state of a block by name ("stone" or "minecraft:stone").
    pub fn default_state(&self, name: &str) -> u16 {
        self.block(name).map(|b| b.default_state as u16).unwrap_or_else(|| panic!("unknown block {name}"))
    }
    /// Value of property `prop` in `state`.
    pub fn get_prop(&self, state: u16, prop: &str) -> Option<&str> {
        let b = self.block_of(state);
        let local = state as u32 - b.min_state;
        let p = b.props.iter().find(|p| p.name == prop)?;
        let idx = (local / p.stride) % p.values.len() as u32;
        Some(&p.values[idx as usize])
    }
    /// `state` with property `prop` set to `value`; None if not applicable.
    pub fn with_prop(&self, state: u16, prop: &str, value: &str) -> Option<u16> {
        let b = self.block_of(state);
        let local = state as u32 - b.min_state;
        let p = b.props.iter().find(|p| p.name == prop)?;
        let n = p.values.len() as u32;
        let cur = (local / p.stride) % n;
        let new = p.values.iter().position(|v| v == value)? as u32;
        Some((b.min_state + local - cur * p.stride + new * p.stride) as u16)
    }
    /// Parse "minecraft:oak_log[axis=x]" into a state id.
    pub fn parse_state(&self, s: &str) -> Option<u16> {
        let (name, props) = match s.find('[') {
            Some(i) => (&s[..i], Some(s[i + 1..].trim_end_matches(']'))),
            None => (s, None),
        };
        let mut st = self.block(name)?.default_state as u16;
        if let Some(props) = props {
            for kv in props.split(',').filter(|x| !x.is_empty()) {
                let (k, v) = kv.split_once('=')?;
                st = self.with_prop(st, k.trim(), v.trim())?;
            }
        }
        Some(st)
    }
    #[inline]
    pub fn is_air(&self, state: u16) -> bool {
        self.states[state as usize].flags & flag::AIR != 0
    }
    #[inline]
    pub fn has(&self, state: u16, f: u16) -> bool {
        self.states[state as usize].flags & f != 0
    }
}
