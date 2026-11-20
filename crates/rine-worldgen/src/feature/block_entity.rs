use serde::{Deserialize, Serialize};

use super::region::Pos;

/// A block entity in absolute block coordinates. The NBT encoder adds `id`, `x`, `y` and `z`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockEntity {
    pub pos: Pos,
    pub type_id: String,
    pub data: BlockEntityData,
}

/// Payloads independent of the runtime world's block entity representation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockEntityData {
    Empty,
    /// `LootTable` is a string. Nonzero `LootTableSeed` is a long.
    Loot {
        table: String,
        seed: i64,
    },
    /// `SpawnData.entity.id` is the entity ID. The encoder supplies the vanilla spawner defaults.
    Spawner {
        entity: String,
    },
    /// `bees` contains `entity_data:{id:"minecraft:bee"}`, `ticks_in_hive`
    /// and `min_ticks_in_hive:600` for each occupant. There is no `flower_pos`.
    Beehive {
        ticks_in_hive: Vec<i32>,
    },
    /// `cursors` is an empty list.
    SculkCatalyst,
    /// `listener` is the default vibration data and `last_vibration_frequency` is zero.
    SculkSensor,
    /// `listener` is the default vibration data and `warning_level` is zero.
    SculkShrieker,
    /// `countdown` is an int initialized to -1.
    PotentSulfur,
}
