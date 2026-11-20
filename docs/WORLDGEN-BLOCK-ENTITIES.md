# Блок-сущности декорации 26.3

`rine_worldgen::pipeline::FullChunk::block_entities` содержит абсолютные координаты, строковый идентификатор типа и полезную нагрузку. Декорация передаёт записи между чанками через те же окна, в которых изменяет блоки. Смена типа блока удаляет запись, смена свойств того же блока её сохраняет.

При записи полной NBT блок-сущности добавляются `components: {}`, строковый `id` и целые `x`, `y`, `z`. Поля ниже перечислены в порядке вызовов `saveAdditional`. `components` и координаты добавляются после них через `BlockEntity.saveWithFullMetadata`.

| Тип | Данные ванильной NBT |
|---|---|
| `minecraft:chest` | Строка `LootTable: "minecraft:chests/simple_dungeon"`. Ненулевой `LootTableSeed` записывается как long. Пока таблица не открыта, содержимое сундука задаёт таблица добычи. |
| `minecraft:mob_spawner` | Поля short `Delay: 20s`, `MinSpawnDelay: 200s`, `MaxSpawnDelay: 800s`, `SpawnCount: 4s`, `MaxNearbyEntities: 6s`, `RequiredPlayerRange: 16s`, `SpawnRange: 4s`. Далее `SpawnData: {entity: {id: "<тип>"}}` и `SpawnPotentials: []`. |
| `minecraft:beehive` | `bees` в порядке добавления, каждая запись содержит `entity_data: {id: "minecraft:bee"}`, `ticks_in_hive` (int, от 0 до 598), `min_ticks_in_hive: 600` (int). Поле `flower_pos` отсутствует. |
| `minecraft:sculk_catalyst` | `cursors: []`. |
| `minecraft:sculk_sensor` | `last_vibration_frequency: 0` (int), `listener: {selector: {tick: -1L}, event_delay: 0}`. |
| `minecraft:sculk_shrieker` | `warning_level: 0` (int), `listener: {selector: {tick: -1L}, event_delay: 0}`. `can_summon` хранится в состоянии блока. |
| `minecraft:potent_sulfur` | `countdown: -1` (int). Состояние гейзера хранится в свойстве `potent_sulfur_state`. |
| `minecraft:brushable_block` | Строка `LootTable: "minecraft:archaeology/desert_well"` и, при ненулевом значении, `LootTableSeed` типа long. Процессор `AppendLoot` берёт сид из отдельного `LegacyRandomSource`, инициализированного через `Mth.getSeed` от положения блока. |

## Дисковое хранение промежуточных чанков

`SavedSlot` передаёт записи через `SlotStore`. Для распакованной записи доступны `pipeline::slot_codec::encode` и `decode`. Кодек сохраняет текущий двоичный формат до конца списка тиков. Хвост содержит массив JSON из `BlockEntity` в UTF-8. Пустой список записывается без хвоста, поэтому старые записи читаются без миграции. Маркер финального чанка остаётся двухбайтовым. Дисковый адаптер с LZ4 и запись Anvil находятся в `rine-world`.
