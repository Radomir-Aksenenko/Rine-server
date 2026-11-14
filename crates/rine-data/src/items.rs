use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ItemInfo {
    pub id: u32,
    pub name: String,
    #[serde(rename = "maxStack")]
    pub max_stack: i32,
    #[serde(rename = "maxDamage", default)]
    pub max_damage: i32,
    /// Name of the block this item places, if any.
    pub block: Option<String>,
    #[serde(default)]
    pub food: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EntityInfo {
    pub id: u32,
    pub name: String,
    pub width: f32,
    pub height: f32,
}

pub struct Items {
    pub items: Vec<ItemInfo>,
    /// item id -> default block state it places (0 = none)
    pub places: Vec<u16>,
    pub entities: Vec<EntityInfo>,
}

impl Items {
    pub fn load(blocks: &crate::blocks::Blocks) -> Self {
        let items: Vec<ItemInfo> = serde_json::from_str(include_str!("../assets/items.json")).expect("items.json");
        let entities: Vec<EntityInfo> = serde_json::from_str(include_str!("../assets/entities.json")).expect("entities.json");
        let places = items
            .iter()
            .map(|i| i.block.as_deref().and_then(|b| blocks.block(b)).map(|b| b.default_state as u16).unwrap_or(0))
            .collect();
        Self { items, places, entities }
    }
    pub fn get(&self, id: u32) -> Option<&ItemInfo> {
        self.items.get(id as usize)
    }
    pub fn by_name(&self, name: &str) -> Option<&ItemInfo> {
        let n = if name.contains(':') { name.to_string() } else { format!("minecraft:{name}") };
        self.items.iter().find(|i| i.name == n)
    }
    pub fn entity(&self, name: &str) -> u32 {
        let n = if name.contains(':') { name.to_string() } else { format!("minecraft:{name}") };
        self.entities.iter().find(|e| e.name == n).map(|e| e.id).unwrap_or_else(|| panic!("entity {name}"))
    }
}
