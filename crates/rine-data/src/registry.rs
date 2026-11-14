use ahash::AHashMap;
use rine_protocol::Writer;
use std::collections::BTreeMap;

/// Registries synchronized in configuration, in vanilla send order
/// (RegistryDataLoader.SYNCHRONIZED_REGISTRIES, see docs/spec/01).
pub const SYNCED: &[&str] = &[
    "minecraft:worldgen/biome",
    "minecraft:chat_type",
    "minecraft:trim_pattern",
    "minecraft:trim_material",
    "minecraft:wolf_variant",
    "minecraft:wolf_sound_variant",
    "minecraft:pig_variant",
    "minecraft:pig_sound_variant",
    "minecraft:frog_variant",
    "minecraft:cat_variant",
    "minecraft:cat_sound_variant",
    "minecraft:cow_sound_variant",
    "minecraft:cow_variant",
    "minecraft:chicken_sound_variant",
    "minecraft:chicken_variant",
    "minecraft:zombie_nautilus_variant",
    "minecraft:painting_variant",
    "minecraft:sulfur_cube_archetype",
    "minecraft:dimension_type",
    "minecraft:damage_type",
    "minecraft:banner_pattern",
    "minecraft:enchantment",
    "minecraft:jukebox_song",
    "minecraft:instrument",
    "minecraft:test_environment",
    "minecraft:test_instance",
    "minecraft:dialog",
    "minecraft:world_clock",
    "minecraft:timeline",
    "minecraft:decorated_pot_pattern",
    "minecraft:block_transformer",
    "minecraft:worldgen/block_state_provider",
];

pub struct Registry {
    pub name: String,
    pub entries: Vec<String>,
    pub index: AHashMap<String, u32>,
}

impl Registry {
    fn new(name: String, entries: Vec<String>) -> Self {
        let index = entries.iter().enumerate().map(|(i, e)| (e.clone(), i as u32)).collect();
        Self { name, entries, index }
    }
    #[inline]
    pub fn id(&self, name: &str) -> Option<u32> {
        self.index.get(name).copied()
    }
    pub fn name(&self, id: u32) -> Option<&str> {
        self.entries.get(id as usize).map(|s| s.as_str())
    }
}

pub struct Registries {
    /// Built-in (static) registries: block, item, entity_type, sound_event...
    pub builtin: AHashMap<String, Registry>,
    /// Synchronized datapack registries in SYNCED order.
    pub synced: Vec<Registry>,
    /// Raw resolved tags: registry -> tag -> entry names.
    pub tags: BTreeMap<String, BTreeMap<String, Vec<String>>>,
}

impl Registries {
    pub fn load() -> Self {
        let builtin_raw: BTreeMap<String, Vec<String>> =
            serde_json::from_str(include_str!("../assets/registries.json")).unwrap();
        let dp_raw: BTreeMap<String, Vec<String>> =
            serde_json::from_str(include_str!("../assets/datapack_registries.json")).unwrap();
        let tags: BTreeMap<String, BTreeMap<String, Vec<String>>> =
            serde_json::from_str(include_str!("../assets/tags.json")).unwrap();
        let builtin = builtin_raw.into_iter().map(|(k, v)| (k.clone(), Registry::new(k, v))).collect();
        let synced = SYNCED
            .iter()
            .map(|r| Registry::new(r.to_string(), dp_raw.get(*r).cloned().unwrap_or_default()))
            .collect();
        Self { builtin, synced, tags }
    }

    pub fn reg(&self, name: &str) -> &Registry {
        self.builtin
            .get(name)
            .or_else(|| self.synced.iter().find(|r| r.name == name))
            .unwrap_or_else(|| panic!("unknown registry {name}"))
    }

    /// Id of an entry in a builtin or synced registry.
    pub fn id(&self, reg: &str, entry: &str) -> Option<u32> {
        self.builtin
            .get(reg)
            .or_else(|| self.synced.iter().find(|r| r.name == reg))
            .and_then(|r| r.id(entry))
    }

    pub fn synced(&self, reg: &str) -> &Registry {
        self.synced.iter().find(|r| r.name == reg).unwrap()
    }

    /// Body of clientbound update_tags (without packet id).
    pub fn tags_payload(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut regs = Vec::new();
        for (reg, tags) in &self.tags {
            let r = match self.builtin.get(reg).or_else(|| self.synced.iter().find(|s| &s.name == reg)) {
                Some(r) => r,
                None => continue,
            };
            let mut body = Vec::new();
            let mut n = 0;
            for (tag, entries) in tags {
                let ids: Vec<u32> = entries.iter().filter_map(|e| r.id(e)).collect();
                body.w_str(tag);
                body.w_varint(ids.len() as i32);
                for id in ids {
                    body.w_varint(id as i32);
                }
                n += 1;
            }
            if n > 0 {
                regs.push((reg.clone(), n, body));
            }
        }
        out.w_varint(regs.len() as i32);
        for (reg, n, body) in regs {
            out.w_str(&reg);
            out.w_varint(n);
            out.extend_from_slice(&body);
        }
        out
    }
}
