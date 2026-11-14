//! Interned block states. Ids are indices into `names` (canonical `name[props]` strings from
//! `assets/worldgen/block_states.txt`, i.e. the default state of each block).

use std::collections::HashMap;

const STATES: &str = include_str!("../../assets/worldgen/block_states.txt");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Blk(pub u16);

pub const AIR: Blk = Blk(0);

pub struct Interner {
    names: Vec<String>,
    index: HashMap<String, u16>,
    defaults: HashMap<String, String>,
}

impl Interner {
    pub fn new() -> Self {
        let mut defaults = HashMap::new();
        for line in STATES.lines().filter(|l| !l.starts_with('#')) {
            let mut parts = line.splitn(2, ' ');
            let id = parts.next().expect("block id").to_string();
            let state = parts.next().expect("block state").to_string();
            defaults.insert(id, state);
        }
        let mut interner = Interner {
            names: Vec::new(),
            index: HashMap::new(),
            defaults,
        };
        // Id 0 is always air.
        let air = interner.get("minecraft:air");
        assert_eq!(air, AIR);
        interner
    }

    /// Interns the default state of block `id` (`minecraft:stone`, ...).
    pub fn get(&mut self, id: &str) -> Blk {
        let state = self
            .defaults
            .get(id)
            .unwrap_or_else(|| panic!("no default state for {id}"))
            .clone();
        if let Some(&i) = self.index.get(&state) {
            return Blk(i);
        }
        let i = self.names.len() as u16;
        self.names.push(state.clone());
        self.index.insert(state, i);
        Blk(i)
    }

    pub fn name(&self, b: Blk) -> &str {
        &self.names[b.0 as usize]
    }

    /// `BlockState.isAir()` for the void and air states.
    pub fn is_air(&self, b: Blk) -> bool {
        b == AIR
    }

    /// `!state.getFluidState().isEmpty()`: water and lava carry a fluid.
    pub fn is_fluid(&self, b: Blk) -> bool {
        let n = &self.names[b.0 as usize];
        n.starts_with("minecraft:water") || n.starts_with("minecraft:lava")
    }

    /// `BlockState.is(block)` by block id (the part before `[`).
    pub fn is_block(&self, b: Blk, id: &str) -> bool {
        let n = &self.names[b.0 as usize];
        match n.find('[') {
            Some(i) => &n[..i] == id,
            None => n == id,
        }
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Default state string per plain block id (the `block_states.txt` table).
    pub fn defaults(&self) -> &HashMap<String, String> {
        &self.defaults
    }
}
