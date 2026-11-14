# 02. Play join, positions, chunks: wire layout (protocol 777, Paper-26.3 tree)

Source root (all citations are relative to it):
`E:\Projects\Rine-server\Paper-26.3\paper-server\src\minecraft\java\net\minecraft\`
Shortened below as `S/`. Ca/spottedleaf (moonrise) files are cited from `E:\Projects\Rine-server\Paper-26.3\paper-server\src\minecraft\java\`.

Vanilla classes are used for the wire format. Paper patches are marked **[Paper]** where they change what is sent or when.

Notation: `i8/u8` one byte, `i16/i32/i64` big-endian, `f32/f64` IEEE big-endian, `VarInt`/`VarLong` LEB128 (as in `FriendlyByteBuf`), `Identifier` = VarInt byte length + UTF-8 bytes (see section 9), `BlockPos` = i64 packed, `Component` = network NBT (section 8).

---

## 0. Conventions verified from source

| Codec | Wire | Source |
|---|---|---|
| `ByteBufCodecs.BOOL` | u8 (0/1) | `S/network/codec/ByteBufCodecs.java:63` |
| `ByteBufCodecs.INT` | i32 BE | `ByteBufCodecs.java:108` |
| `ByteBufCodecs.VAR_INT` | VarInt | `ByteBufCodecs.java:119` |
| `ByteBufCodecs.LONG` / `VAR_LONG` | i64 BE / VarLong | `ByteBufCodecs.java:133`, `:144` |
| `ByteBufCodecs.FLOAT` / `DOUBLE` | f32 BE / f64 BE | `ByteBufCodecs.java:155`, `:166` |
| `ByteBufCodecs.optional(c)` | bool, then `c` if true | `ByteBufCodecs.java:512-527` |
| `ByteBufCodecs.collection(...)` / `list()` | VarInt count + elements | `ByteBufCodecs.java:548-580` |
| `ByteBufCodecs.map(...)` | VarInt count + (key, value) pairs in `forEach` order | `ByteBufCodecs.java:637-660` |
| `ByteBufCodecs.idMapper(...)` | VarInt id | `ByteBufCodecs.java:735-748` |
| `ByteBufCodecs.holderRegistry(...)` | VarInt **plain** registry id (no inline form, no +1) | `ByteBufCodecs.java:781-783` |
| `ByteBufCodecs.holder(...)` (not used on the join path below) | VarInt: 0 = inline direct value, else registry id + 1 | `ByteBufCodecs.java:785+` |
| `ByteBufCodecs.OPTIONAL_VAR_INT` | VarInt, 0 = absent, otherwise value + 1 | `ByteBufCodecs.java:130-132` |
| `ByteBufCodecs.BYTE_ARRAY` / `byteArray(max)` | VarInt length + bytes | `ByteBufCodecs.java:177`, `:316`; `FriendlyByteBuf.java:205-208` |
| `ByteBufCodecs.LONG_ARRAY` | VarInt count + count x i64 | `ByteBufCodecs.java:188-198`; `FriendlyByteBuf.java:259-262` |
| `ByteBufCodecs.BIT_SET` | VarInt byte-length + `BitSet.toByteArray()` bytes (NOT a long array) | `ByteBufCodecs.java:199-210` |
| `ByteBufCodecs.OPTIONAL_COMPOUND_TAG` | NBT tag: `0x00` (TAG_End) when empty, otherwise a nameless compound. No bool prefix. | `ByteBufCodecs.java:213-225` |
| `Identifier.STREAM_CODEC` | `STRING_UTF8`: VarInt byte length + UTF-8 bytes (max 32767 chars) | `S/resources/Identifier.java:20`; `ByteBufCodecs.java:334` (`stringUtf8`); `S/network/Utf8String.java:35-50` |
| `ResourceKey.streamCodec(reg)` | Identifier (`Identifier.STREAM_CODEC.map`) | `S/resources/ResourceKey.java:24-26` |
| `BlockPos.STREAM_CODEC` | i64 `BlockPos.asLong()` | `S/core/BlockPos.java:37-47`, `FriendlyByteBuf.java:309-316` |
| `Vec3.STREAM_CODEC` | f64 x, f64 y, f64 z | `S/world/phys/Vec3.java:25-37` |
| `ChunkPos` (FriendlyByteBuf) | i64 `ChunkPos.pack()` | `FriendlyByteBuf.java:318-334`; `S/world/level/ChunkPos.java:60-62`, `:84-86` |

BlockPos packing (`S/core/BlockPos.java:110-112`): `((x & 0x3FFFFFF) << 38) | (y & 0xFFF) | ((z & 0x3FFFFFF) << 12)`. x and z are 26-bit fields, y is a 12-bit field. Decode: `x = v >> 38`, `z = (v << 26) >> 38`, `y = (v << 52) >> 52` (sign-extended).

ChunkPos packing (`ChunkPos.java:84-86`): `(x & 0xFFFFFFFFL) | ((z & 0xFFFFFFFFL) << 32)`. As bytes the i64 is BE, so it is `z` (4 bytes, BE) followed by `x` (4 bytes, BE).

---

## 1. ClientboundLoginPacket (play login)

Source: `S/network/protocol/game/ClientboundLoginPacket.java:14-54`

| # | Field | Wire |
|---|---|---|
| 1 | playerId | i32 |
| 2 | hardcore | bool (u8) |
| 3 | levels (Set<ResourceKey<Level>>) | VarInt count, then count x Identifier. Order = HashSet iteration (unspecified) |
| 4 | maxPlayers | VarInt |
| 5 | chunkRadius (view distance) | VarInt |
| 6 | simulationDistance | VarInt |
| 7 | reducedDebugInfo | bool |
| 8 | showDeathScreen | bool |
| 9 | doLimitedCrafting | bool |
| 10 | commonPlayerSpawnInfo | CommonPlayerSpawnInfo (below) |
| 11 | onlineMode | bool |
| 12 | enforcesSecureChat | bool |

### CommonPlayerSpawnInfo
Source: `S/network/protocol/game/CommonPlayerSpawnInfo.java:15-49`

| # | Field | Wire |
|---|---|---|
| 1 | dimensionType (Holder<DimensionType>) | VarInt = registry id of the dimension_type (`S/world/level/dimension/DimensionType.java:55` uses `holderRegistry`: plain id, no inline form, no +1) |
| 2 | dimension (ResourceKey<Level>) | Identifier, e.g. `minecraft:overworld` |
| 3 | seed (hashed) | i64 |
| 4 | gameType | VarInt. `GameType` via idMapper: 0 survival, 1 creative, 2 adventure, 3 spectator (`S/world/level/GameType.java:19-22`, `:27`) |
| 5 | previousGameType (Optional<GameType>) | VarInt: 0 = absent, otherwise gamemode id + 1 (`GameType.java:28-31`, `ByteBufCodecs.java:130-132`). It is NOT a u8 |
| 6 | isDebug | bool |
| 7 | isFlat | bool |
| 8 | lastDeathLocation (Optional<GlobalPos>) | bool; if true: GlobalPos = Identifier dimension + i64 BlockPos (`S/core/GlobalPos.java:18-20`) |
| 9 | portalCooldown | VarInt |
| 10 | seaLevel | VarInt |

Server side: `S/server/players/PlayerList.java:181-195` builds the packet. `PlayerList.java:184-194` supplies all fields.

---

## 2. Teleport and position

### ClientboundPlayerPositionPacket
Source: `S/network/protocol/game/ClientboundPlayerPositionPacket.java:13-21`, `S/world/entity/PositionMoveRotation.java:13-23`

| # | Field | Wire |
|---|---|---|
| 1 | id (teleport id) | VarInt |
| 2 | change.position | f64 x, f64 y, f64 z (`Vec3.STREAM_CODEC`) |
| 3 | change.deltaMovement | f64 x, f64 y, f64 z |
| 4 | change.yRot | f32 |
| 5 | change.xRot | f32 |
| 6 | relatives (Set<Relative>) | **i32 BE bitfield** (`Relative.SET_STREAM_CODEC = ByteBufCodecs.INT.map(...)`, `S/world/entity/Relative.java:24`) |

Relative bits (`Relative.java:11-18`, mask = `1 << bit`, `:89-90`, `pack` `:109-117`):

| Relative | bit | mask |
|---|---|---|
| X | 0 | 0x001 |
| Y | 1 | 0x002 |
| Z | 2 | 0x004 |
| Y_ROT | 3 | 0x008 |
| X_ROT | 4 | 0x010 |
| DELTA_X | 5 | 0x020 |
| DELTA_Y | 6 | 0x040 |
| DELTA_Z | 7 | 0x080 |
| ROTATE_DELTA | 8 | 0x100 |

Empty set = fully absolute. The join teleport uses `ServerGamePacketListenerImpl.teleport(...)` (`S/server/network/ServerGamePacketListenerImpl.java:1905-1908`, which reaches `internalTeleport` with an empty relatives set at `:1591`). The packet is sent at `:1991`.

Teleport id: `++this.awaitingTeleport` happens before the send (`ServerGamePacketListenerImpl.java:1979-1981`), so the first id sent is the initial value + 1 (initial value: UNCONFIRMED, so the first id is probably 1).

### ServerboundAcceptTeleportationPacket
Source: `S/network/protocol/game/ServerboundAcceptTeleportationPacket.java:9-24`

| # | Field | Wire |
|---|---|---|
| 1 | id | VarInt |
| 2 | x | f64 |
| 3 | y | f64 |
| 4 | z | f64 |
| 5 | yRot | f32 |
| 6 | xRot | f32 |

The server kicks the player if the id does not match `awaitingTeleport`, or if any value is NaN or infinite (`ServerGamePacketListenerImpl.java:792-797`).

---

## 3. ClientboundGameEventPacket

Source: `S/network/protocol/game/ClientboundGameEventPacket.java:41-49` (read), `:46-49` (write)

Wire: `u8 event id`, then `f32 param`. The param is always present (0 when unused).

| id | Name | Param meaning |
|---|---|---|
| 0 | NO_RESPAWN_BLOCK_AVAILABLE | unused |
| 1 | START_RAINING | unused |
| 2 | STOP_RAINING | unused |
| 3 | CHANGE_GAME_MODE | gamemode id as float |
| 4 | WIN_GAME | 0 = respawn screen, 1 = credits (vanilla convention, UNCONFIRMED from this tree) |
| 5 | DEMO_EVENT | 0 intro, 101-104 hints (`:28-32`) |
| 6 | PLAY_ARROW_HIT_SOUND | unused |
| 7 | RAIN_LEVEL_CHANGE | rain level 0..1 |
| 8 | THUNDER_LEVEL_CHANGE | thunder level 0..1 |
| 9 | PUFFER_FISH_STING | unused |
| 10 | GUARDIAN_ELDER_EFFECT | unused |
| 11 | IMMEDIATE_RESPAWN | 0/1 |
| 12 | LIMITED_CRAFTING | 0/1 |
| 13 | LEVEL_CHUNKS_LOAD_START | 0 (`:27`). The client shows the "loading terrain" screen until chunks arrive |

Source of the list: `ClientboundGameEventPacket.java:14-27`.

---

## 4. Chunk cache, batches, forget

| Packet | Source | Wire |
|---|---|---|
| ClientboundSetChunkCacheCenterPacket | `.../ClientboundSetChunkCacheCenterPacket.java:20-27` | VarInt chunkX, VarInt chunkZ |
| ClientboundSetChunkCacheRadiusPacket | `.../ClientboundSetChunkCacheRadiusPacket.java:18-23` | VarInt radius (the send radius, used by the client) |
| ClientboundSetSimulationDistancePacket | `.../ClientboundSetSimulationDistancePacket.java:13-18` | VarInt |
| ClientboundChunkBatchStartPacket | `.../ClientboundChunkBatchStartPacket.java:9-10` | **empty payload** (`StreamCodec.unit`) |
| ClientboundChunkBatchFinishedPacket | `.../ClientboundChunkBatchFinishedPacket.java:13-18` | VarInt batchSize |
| ServerboundChunkBatchReceivedPacket | `.../ServerboundChunkBatchReceivedPacket.java:13-18` | f32 desiredChunksPerTick |
| ClientboundForgetLevelChunkPacket | `.../ClientboundForgetLevelChunkPacket.java:14-20` | **one i64** = ChunkPos.pack() (x low 32 bits, z high 32 bits) |

ServerboundChunkBatchReceived handling (`S/server/network/PlayerChunkSender.java:127-139`): `unacknowledgedBatches--` (floored at 0). `desiredChunksPerTick` = NaN ? 0.01 : clamp(value, 0.01, 64). If unacknowledged is 0, `batchQuota = 1`. `maxUnacknowledgedBatches = 10`.

---

## 5. ClientboundLevelChunkWithLightPacket

Source: `S/network/protocol/game/ClientboundLevelChunkWithLightPacket.java:16-26`

| # | Field | Wire |
|---|---|---|
| 1 | x | **i32 BE** (not VarInt) |
| 2 | z | **i32 BE** |
| 3 | chunkData | ClientboundLevelChunkPacketData (5.1) |
| 4 | lightData | ClientboundLightUpdatePacketData (5.2) |

Per-chunk send: `S/server/network/PlayerChunkSender.java:81-85` passes `null` light filters, so every section is written.

### 5.1 ClientboundLevelChunkPacketData
Source: `S/network/protocol/game/ClientboundLevelChunkPacketData.java:28-40`

1. **heightmaps**: `ByteBufCodecs.map(EnumMap, Heightmap.Types.STREAM_CODEC, LONG_ARRAY)`. Wire: VarInt entry count, then per entry: VarInt Heightmap.Types id, VarInt n, n x i64.
   - Only types with `sendToClient()` (usage == CLIENT, `S/world/level/levelgen/Heightmap.java:202-203`) are included (`ClientboundLevelChunkPacketData.java:71-74`). Those are:
     - WORLD_SURFACE = id 1 (`Heightmap.java:170`)
     - MOTION_BLOCKING = id 4 (`:174`)
     - MOTION_BLOCKING_NO_LEAVES = id 5 (`:177-180`)
   - Other ids (for reference): WORLD_SURFACE_WG 0, OCEAN_FLOOR_WG 2, OCEAN_FLOOR 3 (`:169-172`). They are not sent.
   - Entry order: the map is built with `Collectors.toMap` (HashMap), so the wire order is NOT fixed. The client reads into an EnumMap by key.
   - Length n: `Heightmap` data = `SimpleBitStorage(bits, 256)` with `bits = ceillog2(chunk.getHeight() + 1)` (`Heightmap.java:48-49`). Overworld height 384 gives bits = 9. valuesPerLong = floor(64/9) = 7 (`SimpleBitStorage.java:264`), so n = ceil(256/7) = **37 longs**.

2. **buffer**: VarInt byte length + bytes. The bytes are the concatenation of `LevelChunkSection.write` for every section, bottom to top (`ClientboundLevelChunkPacketData.java:127-133`). Max 2 MiB (`:31-35`).

3. **blockEntities**: VarInt count, then per entry (`ClientboundLevelChunkPacketData.java:161-172`):
   - u8 packedXZ = `((x & 15) << 4) | (z & 15)` (`:177`)
   - i16 y (absolute world Y)
   - VarInt BlockEntityType registry id (`ByteBufCodecs.registry`, plain id)
   - OPTIONAL_COMPOUND_TAG: `0x00` if the update tag is empty, otherwise a nameless compound (0x0A + payload). No bool prefix.
   - **[Paper]** Block entities beyond 750 are not put in the list. Their update packets go to `extraPackets` and are sent as separate packets (`ClientboundLevelChunkPacketData.java:44-46`, `:86-92`).

### 5.2 ClientboundLightUpdatePacketData
Source: `S/network/protocol/game/ClientboundLightUpdatePacketData.java:16-34`

| # | Field | Wire |
|---|---|---|
| 1 | skyYMask | BIT_SET: VarInt L, then L bytes = `BitSet.toByteArray()` |
| 2 | blockYMask | BIT_SET (same) |
| 3 | emptySkyYMask | BIT_SET (same) |
| 4 | emptyBlockYMask | BIT_SET (same) |
| 5 | skyUpdates | VarInt count, then per entry: VarInt 2048 (max 2048) + 2048 bytes |
| 6 | blockUpdates | VarInt count, then per entry: VarInt 2048 + 2048 bytes |

BIT_SET details (`ByteBufCodecs.java:199-210`): `toByteArray()` is little-endian in bit order (bit i sits in byte i/8, at bit position i%8). Trailing zero bytes are dropped. An empty set is VarInt 0. This is **not** the `writeBitSet` long-array form (`FriendlyByteBuf.java:593`), which this packet does not use.

Section index: 0 .. (lightSectionCount - 1), where `lightSectionCount = sectionsCount + 2` (`S/world/level/lighting/LevelLightEngine.java:121-123`). Index 0 is section Y = minSectionY - 1 (`:125-127`). Overworld (min_y -64, height 384, from `vanilla-data/generated/data/minecraft/dimension_type/overworld.json`): sectionsCount = 24, so there are **26** light sections. Index i is section Y = -5 + i, so index 25 is Y = 20.

Per index: if the layer has data, the mask bit is set and the array is appended (`LightUpdatePacketData.java:55-73`). If the data is empty (all zero), the emptyMask bit is set and nothing is appended. Data arrays are in ascending index order, and each is 2048 bytes (nibbles: 4096 entries, 2 per byte).

---

## 6. LevelChunkSection and PalettedContainer

### 6.1 LevelChunkSection.write
Source: `S/world/level/chunk/LevelChunkSection.java:291-297`, size `:299-301`

1. i16 nonEmptyBlockCount (`writeShort`, BE)
2. i16 fluidCount
3. states: PalettedContainer<BlockState>, 4096 entries
4. biomes: PalettedContainer<Holder<Biome>>, 64 entries

`getSerializedSize` = 4 + states + biomes (not needed on the wire, but it does match).

### 6.2 PalettedContainer wire (write `PalettedContainer.java:265-277`, Data.write `:525-537`)

Wire: **u8 bits**, then the palette, then the raw long array.

- The raw long array is **fixed size**: no length prefix (`FriendlyByteBuf.java:269-272`, `writeFixedSizeLongArray`).
- Packing: `valuesPerLong = floor(64 / bits)`. Entries do **not** span longs (`S/util/SimpleBitStorage.java:264`). Count = ceil(entries / valuesPerLong) (`:269`).
- Entry k within a long (k = index % valuesPerLong) sits at bits [k*bits, (k+1)*bits), LSB first. Long index = index / valuesPerLong. This follows `SimpleBitStorage.getAndSet` (`:293-308`), which uses `data >>> divR & mask`. Exact layout: this matches vanilla, derived from the shift code, not a separate test.
- Index: blocks `idx = (y << 8) | (z << 4) | x` (x,y,z in 0..15, `Strategy.getIndex` `S/world/level/chunk/Strategy.java:69-71`, bitsPerAxis 4, 4096 entries). Biomes `idx = (y << 4) | (z << 2) | x` in quart coordinates (0..3, bitsPerAxis 2, 64 entries).
- Palette sections: determined by the bits byte (see 6.3). The data is always raw.

### 6.3 Configurations (Strategy.java and Configuration.java)

Block states (`Strategy.java:33-48`, `Strategy` bitsPerAxis 4):

| bits (u8 written) | Palette | Palette wire | Storage longs (4096 entries) |
|---|---|---|---|
| 0 | SingleValue (`SingleValuePalette.java:59-65`) | VarInt global id of the single value | 0 (ZeroBitStorage `RAW = new long[0]`, `ZeroBitStorage.java:8`) |
| 4 | Linear (`FOUR_BITS_LINEAR`; the written byte is 4 for cases 1-4) | VarInt size, then size x VarInt global id (`LinearPalette.java:87-93`) | 256 (16/long) |
| 5 | HashMap (`FIVE_BITS_HASHMAP`) | VarInt size, then size x VarInt global id (`HashMapPalette.java:84-91`) | 342 (12/long) |
| 6 | HashMap | same | 410 (10/long) |
| 7 | HashMap | same | 456 (9/long) |
| 8 | HashMap | same | 512 (8/long) |
| 16 (global) | Global (`GlobalPalette.java:40-41`: no palette bytes) | none | 1024 (4/long) |

Block rule: the writer uses the bits value from the in-memory storage. For a palette of `n` entries the server uses `bits = ceillog2(n)` (`Strategy.java:83-86`), so n = 1 gives 0, n = 2..16 gives 4 (linear), n = 17..256 gives 5..8 (hashmap), n > 256 gives global. The client maps the received bits through `getConfigurationForBitCount` (`Strategy.java:37-45`): `1-4 -> 4-bit linear`, `5-8 -> hashmap`, `>= 9 -> global`, with global storage sized at `globalPaletteBitsInMemory` regardless of the byte value. Writers should therefore send 4 for linear, 5-8 for hashmap, and 16 for global. For global, the written byte is `globalPaletteBitsInMemory` (16), not the entry bits. Global bits = `ceillog2(globalMap.size())` (`Strategy.java:28`).

Global block bits: the block state count in `vanilla-data/generated/reports/blocks.json` is 35723 (max id 35722), so ceillog2(35723) = **16**.

Biomes (`Strategy.java:50-63`, bitsPerAxis 2):

| bits (u8 written) | Palette | Palette wire | Storage longs (64 entries) |
|---|---|---|---|
| 0 | SingleValue | VarInt global id | 0 |
| 1 | Linear | VarInt size, then size x VarInt id | 1 (64/long) |
| 2 | Linear | same | 2 (32/long) |
| 3 | Linear | same | 4 (21/long) |
| 7 (global) | Global, no palette bytes | none | 8 (9/long) |

Biomes with 4 or more palette entries: `bits = ceillog2(n)` >= 4 gives global. The written byte is `globalPaletteBitsInMemory` = ceillog2(biome registry size). The biome registry count in `vanilla-data/generated/data/minecraft/worldgen/biome` is 67 files, so ceillog2(67) = **7**. The biome registry size on the wire is UNCONFIRMED (the file count may differ from the registry count).

Note: a 64-entry container with 7 bits takes ceil(64/9) = 8 longs.

---

## 7. Other play packets (join path)

| Packet | Source | Wire (in order) |
|---|---|---|
| ClientboundSetDefaultSpawnPositionPacket | `.../ClientboundSetDefaultSpawnPositionPacket.java:9-12` | LevelData.RespawnData: GlobalPos (Identifier dimension, i64 BlockPos), f32 yaw, f32 pitch (`S/world/level/storage/LevelData.java:44-50`) |
| ClientboundSetTimePacket | `.../ClientboundSetTimePacket.java:14-21` | i64 gameTime; VarInt n; n x (VarInt Holder<WorldClock> registry id (`WorldClock.java:14`, holderRegistry), VarLong totalTicks, f32 partialTick, f32 rate) (`S/world/clock/ClockNetworkState.java:8-15`) |
| ClientboundPlayerAbilitiesPacket | `.../ClientboundPlayerAbilitiesPacket.java:13-16`, `:43-64` | u8 bitfield (1 invulnerable, 2 flying, 4 mayFly, 8 instabuild), f32 flyingSpeed, f32 walkingSpeed |
| ClientboundSetHealthPacket | `.../ClientboundSetHealthPacket.java:28-32` | f32 health, VarInt food, f32 saturation |
| ClientboundSetExperiencePacket | `.../ClientboundSetExperiencePacket.java:28-31` (read `:22-25`) | f32 progress, **VarInt level**, **VarInt total** (write order, not constructor order) |
| ClientboundSetHeldSlotPacket | `.../ClientboundSetHeldSlotPacket.java:10-11` | VarInt slot |
| ClientboundKeepAlivePacket (common) | `S/network/protocol/common/ClientboundKeepAlivePacket.java:9-23` | i64 id |
| ServerboundKeepAlivePacket (common) | `S/network/protocol/common/ServerboundKeepAlivePacket.java:9-23` | i64 id |
| ClientboundTickingStatePacket | `.../ClientboundTickingStatePacket.java:9-24` | f32 tickRate, bool isFrozen |
| ClientboundTickingStepPacket | `.../ClientboundTickingStepPacket.java:9-23` | VarInt tickSteps |
| ClientboundInitializeBorderPacket | `.../ClientboundInitializeBorderPacket.java:44-52` | f64 newCenterX, f64 newCenterZ, f64 oldSize, f64 newSize, VarLong lerpTime, VarInt newAbsoluteMaxSize, VarInt warningBlocks, VarInt warningTime |
| ClientboundServerDataPacket | `.../ClientboundServerDataPacket.java:13-19` | Component motd (NBT, context-free), optional byte[]: bool, then VarInt len + bytes (icon) |
| ClientboundChangeDifficultyPacket | `.../ClientboundChangeDifficultyPacket.java:11-17` | VarInt difficulty (`S/world/Difficulty.java:13-20`: 0 peaceful, 1 easy, 2 normal, 3 hard), bool locked |
| ClientboundSystemChatPacket | `.../ClientboundSystemChatPacket.java:12-18` | Component content (NBT, registry context), bool overlay |
| ClientboundDisconnectPacket (common) | `S/network/protocol/common/ClientboundDisconnectPacket.java:10-12` | Component reason (NBT, context-free) |
| ClientboundTabListPacket | `.../ClientboundTabListPacket.java:11-17` | Component header (NBT), Component footer (NBT) |

Notes:
- The "ticking" packets are sent by `S/server/ServerTickRateManager.java:132-135` (`updateJoiningPlayer`), at the end of `sendLevelInfo`.
- Border: the constructor reads `border.getLerpTime()` etc. (`ClientboundInitializeBorderPacket.java:33-42`).

---

## 8. Component on the network (play)

Source: `S/network/chat/ComponentSerialization.java:37-44` (codecs), `:97-147` (codec tree).

- Play components: `TRUSTED_STREAM_CODEC` / `STREAM_CODEC` (registry context) and `TRUSTED_CONTEXT_FREE_STREAM_CODEC` (no registry). In all cases the value is encoded to an NBT tag and written with the nameless network NBT writer (section 9).
- Top-level codec (`:110-147`): `Either<Either<String, List>, full>`. Encoding calls `component.tryCollapseToString()`. If that returns a string (plain literal, no style, no siblings), the result is a **TAG_String**.
- Simplest valid encoding of a plain text component `"hi"`: `08 00 02 68 69` (type TAG_String 0x08, u16 length 2, bytes "hi"). No name, no type or value wrapper.
- Any styled or multi-part component encodes as a **TAG_Compound**. For a plain literal the contents encoder writes the `text` key only (`StrictEither.encode` delegates to `fuzzy.encode`, `ComponentSerialization.java:215-216`), e.g. `0A 08 00 04 74 65 78 74 ...`. Whether the `type` key is emitted on this path is UNCONFIRMED.
- Translatable, extra, and style fields use the normal Component codecs. Not covered here.

---

## 9. Network NBT

Sources: `S/nbt/NbtIo.java:258-260` (`writeAnyTag`), `:168-177` (`writeTagV`), `S/network/FriendlyByteBuf.java:439-452` (`writeNbt`), `:460-480` (`readNbt`).

- Root: **type byte, then the payload. No root name.** `writeAnyTag` calls `writeTag(tag, out, null)`, and the name is written only when it is non-null (`writeTagV`).
- The empty case: `writeNbt(null)` writes TAG_End (`0x00`) (`FriendlyByteBuf.java:444-447`).
- String payload: u16 BE length, then modified UTF-8 bytes (Java `DataOutput.writeUTF` format). It is NOT VarInt-prefixed. Protocol strings (Identifier, `Utf8String`) use VarInt length instead (`S/network/Utf8String.java:35-50`). Keep these two separate.
- Compound payload: children as (type byte, u16 name, payload), terminated by `0x00`.
- The Paper fast writer (moonrise NBT patches) is used for the same format. The output matches vanilla `writeUTF` (UNCONFIRMED at byte level for names, but the join path only uses root tags).

---

## 10. Join sequence (vanilla order, with Paper deviations)

Main entry: `S/server/players/PlayerList.java:160-300` (`placeNewPlayer`). `sendLevelInfo`: `:999-1022`.

Numbers are source lines.

1. ClientboundLoginPacket (`:181-195`).
2. ClientboundChangeDifficultyPacket (`:198`).
3. ClientboundPlayerAbilitiesPacket (`:199`).
4. ClientboundSetHeldSlotPacket (`:200`).
5. ClientboundUpdateRecipesPacket (`:202-203`), then recipe book and scoreboard sends (`:205-208`, not expanded here).
6. Teleport: ClientboundPlayerPositionPacket with an empty relatives set (`:220`, `ServerGamePacketListenerImpl.java:1975-1991`). The client must reply with ServerboundAcceptTeleportation before movement is accepted.
7. `sendLevelInfo` **call 1** (`:233`). In order:
   - a. ClientboundInitializeBorderPacket (`:1001`)
   - b. ClientboundSetTimePacket (clock full sync, `:1002`)
   - c. ClientboundSetDefaultSpawnPositionPacket (`:1003`)
   - d. Weather: `player.setPlayerWeather` and `updateWeather` (`:1008-1017`). These send RAIN_LEVEL_CHANGE / START_RAINING when they differ from the default (`S/server/level/ServerPlayer.java:3111-3116`).
   - e. **GameEvent LEVEL_CHUNKS_LOAD_START** (`:1021`)
   - f. ClientboundTickingStatePacket and ClientboundTickingStepPacket (`ServerTickRateManager.java:132-135`, called at `:1022`)
8. `level.addNewPlayer(player)` (`:234`). This is where the chunk loader starts (see 9).
9. **[Paper]** Chunk loader (moonrise `RegionizedPlayerChunkLoader.java:893-902`), on initial add: ClientboundSetChunkCacheRadiusPacket, then ClientboundSetSimulationDistancePacket, then ClientboundSetChunkCacheCenterPacket (center sent last). Exact call site inside `addNewPlayer` is UNCONFIRMED.
10. Player info (tab) packets (`:259-288`). Not expanded here.
11. `sendLevelInfo` **call 2** (`:296`). It sends the same set again (border, time, spawn, weather, LEVEL_CHUNKS_LOAD_START, ticking state and step). This is the observed code, not an error.
12. Chunks, every server tick (`S/server/MinecraftServer.java:1838-1841`, `PlayerChunkSender.sendNextChunks`, `S/server/network/PlayerChunkSender.java:55-79`):
    - Quota: `batchQuota = min(batchQuota + desired, max(1, desired))`. Starts with `desired = 9.0` (`:27`, `:31`), `maxUnacknowledged = 1` until the first ack (`:34`), then 10 (`:138`).
    - If `floor(batchQuota) >= 1` and there are pending chunks, it picks the nearest `floor(batchQuota)` chunks (sorted by distance to the player chunk, `:99-118`) and sends:
      - ClientboundChunkBatchStartPacket (empty payload)
      - N x ClientboundLevelChunkWithLightPacket (`sendChunk`, `:81-85`; light filters null = all sections)
      - ClientboundChunkBatchFinishedPacket(N)
    - Then `batchQuota -= N`, `unacknowledgedBatches++`.
    - The client answers ServerboundChunkBatchReceived(f32 desiredChunksPerTick) (section 4).
    - Batches stop while `unacknowledgedBatches >= maxUnacknowledged`.

**[Paper] deviations from the Mojang-style flow:**
- Radius and simulation distance packets are commented out in `sendLevelInfo` (`PlayerList.java:1005-1006`). Paper sends them from the chunk loader (`RegionizedPlayerChunkLoader.java:893-902`, `:975-978`). Vanilla (Mojang) sends them inside `sendLevelInfo`. The position of these packets relative to LEVEL_CHUNKS_LOAD_START is therefore Paper-specific.
- Chunk center: vanilla source for ClientboundSetChunkCacheCenterPacket is UNCONFIRMED (Mojang's ChunkMap is not in this tree; here it is only sent by moonrise, `RegionizedPlayerChunkLoader.java:576`, `:902`, `:1067`).
- `sendLevelInfo` runs twice (`:233`, `:296`).
- `sendSupportedChannels` is called at `:197` (custom payloads, not covered).
- Block entities over 750 go out as extra packets (section 5.1).

---

## UNCONFIRMED items (summary)

- Teleport id initial value (first id is probably 1).
- Exact weather conditions in `sendLevelInfo` (only the code path is confirmed).
- Heightmap map wire order (HashMap, unspecified).
- Login `levels` set order (HashSet, unspecified).
- Biome global bits = 7 is based on a 67-file count, not the registry size on the wire.
- Center and radius call site inside `addNewPlayer` (ordering relative to chunk batches).
- Whether the `type` key appears in plain-text component compounds (section 8).
- WIN_GAME param meaning (vanilla convention, not in this tree).
- Byte-level NBT string encoding under the Paper fast writer (vanilla `writeUTF` used as reference).
- Exact SimpleBitStorage bit placement: derived from shift code, not separately tested.
