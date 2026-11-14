# 03. Entities, items, interaction: wire layouts (protocol 777, Minecraft 26.3)

Source root (all citations are relative to it):
`E:\Projects\Rine-server\Paper-26.3\paper-server\src\minecraft\java\`
Short form `net/minecraft/...` below. Line numbers were read from the files on disk for this pass.

Packet IDs are not listed here (given as known). Each section gives the body only, in wire order.

## 0. Conventions

- All integers are big-endian (Netty default) unless marked VarInt/VarLong (LEB128).
- `VarInt` = `VarInt.read/write`. `VarLong` = `VarLong.read/write`.
- `i8`/`u8` = `readByte` / `readUnsignedByte`. `i16` = `readShort`. `i32` = `readInt`. `i64` = `readLong`. `f32` = `readFloat`. `f64` = `readDouble`.
- `bool` = one byte, 0x00 / 0x01 (`readBoolean`), e.g. `ByteBufCodecs.BOOL` (`net/minecraft/network/codec/ByteBufCodecs.java:63-72`).
- `String` (`ByteBufCodecs.stringUtf8(max)`, line 334): VarInt byte length, then UTF-8 bytes. `max` is a character limit enforced on decode. `FriendlyByteBuf.writeUtf` uses the same layout.
- `Identifier` = `String` (`net/minecraft/resources/Identifier.java:20`, `ByteBufCodecs.STRING_UTF8.map(...)`, STRING_UTF8 = stringUtf8(32767), line 210).
- `UUID` = 2 x i64 BE: most significant bits, then least significant (`FriendlyByteBuf.java:416-419`).
- `Optional<T>` = bool, then T if true (`ByteBufCodecs.java:512-527`).
- `Nullable<T>` = same wire form as Optional (bool, then T).
- Collection (`ByteBufCodecs.list()` / `collection`, lines 548-600) = VarInt count, then count elements.
- `VarIntArray` = VarInt count, then count VarInts (`FriendlyByteBuf.java:235-243`, writer `writeVarIntArray`).
- `idMapper` (`ByteBufCodecs.java:735-749`) = VarInt id.
- `registry(key)` (lines 755-778) = VarInt registry id (no inline form).
- `holder(key, directCodec)` (lines 785-814) = VarInt `h`. If `h == 0`, the value follows inline (direct codec). If `h > 0`, the registry entry is `h - 1`. Encoding a reference writes `id + 1`.
- `holderSet` (lines 816-852) = VarInt `n`. `n == 0` means a tag: Identifier follows. Otherwise `n - 1` holders follow, each as a holder encoding.
- `EnumSet` as a fixed bitset (`FriendlyByteBuf.java:115-124`): ceil(enumValueCount / 8) bytes. Bit `i` (declaration order, ordinal) is in byte `i / 8`, bit `i % 8`, LSB first (`BitSet.toByteArray` order, `FriendlyByteBuf.java:611-617`).
- Fixed bitset (`ByteBufCodecs.fixedBitSet(n)`, line 348): ceil(n / 8) bytes, same bit order (`FriendlyByteBuf.java:605-617`).
- Enums coded with `writeEnum` / `readEnum` (`FriendlyByteBuf.java:384-391`) = VarInt ordinal (declaration order, first = 0).
- Container id (`readContainerId`/`writeContainerId`, `FriendlyByteBuf.java:624-634`) = VarInt.
- `BlockPos` = i64 packed (section 1.2).
- `Direction` (section 1.4): VarInt on most wire paths, but see the explicit exceptions there.

### 0.1 Paper-only wire deviations (observed, not vanilla)

- `ClientboundSetEntityDataPacket` wraps item encoding in an obfuscation session (`ClientboundSetEntityDataPacket.java:21-29`). Item bytes are the same format; counts may be sanitized.
- `ItemStack.createOptionalStreamCodec` (`net/minecraft/world/item/ItemStack.java`, function `createOptionalStreamCodec`, roughly lines 163-202) calls `ItemComponentSanitizer.sanitizeCount` before writing the count. Format unchanged.
- `ClientboundPlayerInfoUpdatePacket` `INITIALIZE_CHAT` writes `null` instead of an expired chat session (`ClientboundPlayerInfoUpdatePacket.java:181-189`). Format unchanged.
- `ServerboundUseItemOnPacket` / `ServerboundUseItemPacket` carry a `timestamp` field that is server-side only (set to `System.currentTimeMillis()`, not sent). See sections 5.3 and 5.4.
- `ServerboundChatCommandPacket` max length comes from system property `paper.maxChatCommandInputSize`, default 256 (`ServerboundChatCommandPacket.java:9`).

---

## 1. Shared types

### 1.1 Primitive stream codecs (`net/minecraft/network/codec/ByteBufCodecs.java`)

| Codec | Wire | Line |
|---|---|---|
| BOOL | u8 (0/1) via writeBoolean | 63 |
| BYTE | i8 | 74 |
| SHORT | i16 | 86 |
| UNSIGNED_SHORT | u16 | 97 |
| INT | i32 BE | 108 |
| VAR_INT | VarInt | 119 |
| LONG | i64 BE | 133 |
| VAR_LONG | VarLong | 144 |
| FLOAT | f32 BE | 155 |
| DOUBLE | f64 BE | 166 |
| STRING_UTF8(n) | VarInt len + UTF-8 | 210, 334 |
| BIT_SET | VarInt length + bytes (`BitSet.toByteArray`) | 199-209 |
| fixedBitSet(n) | ceil(n/8) bytes, LSB-first bits | 348-352 |
| INSTANT | i64 epoch millis (`LONG.map`) | 304 |

Note the mismatch: `ByteBufCodecs.INT` is i32, not VarInt. Use it where the source says `ByteBufCodecs.INT`.

### 1.2 BlockPos (packed i64)

Source: `net/minecraft/core/BlockPos.java:110-112`, decode at `:86-88`. Wire = `FriendlyByteBuf.writeBlockPos` = writeLong (`FriendlyByteBuf.java:314-316`).

```
bits 63..38 : x (26 bits, signed)
bits 37..12 : z (26 bits, signed)
bits 11..0  : y (12 bits, signed)
long = ((x & 0x3FFFFFF) << 38) | ((z & 0x3FFFFFF) << 12) | (y & 0xFFF)
```

Decode: `x = (int)(v >> 38)`, `y = (int)((v << 52) >> 52)`, `z = (int)((v << 26) >> 38)` (`BlockPos.java:87`).

### 1.3 SectionPos (packed i64)

Source: `net/minecraft/core/SectionPos.java:227-229` (`asLong(x,y,z)`), STREAM_CODEC at line 35 (`LONG.map`).

```
bits 63..42 : x (22 bits)
bits 41..20 : z (22 bits)
bits 19..0  : y (20 bits)
long = ((x & 0x3FFFFF) << 42) | ((y & 0xFFFFF)) | ((z & 0x3FFFFF) << 20)
```

Note the order: x high, z middle, y low. This differs from BlockPos.

Section-relative block index (short, 12 bits), `SectionPos.java:101-115`:

```
rel = (x & 15) << 8 | (z & 15) << 4 | (y & 15)
x = (rel >>> 8) & 15, z = (rel >>> 4) & 15, y = rel & 15
```

### 1.4 Direction

Source: `net/minecraft/core/Direction.java:33-38` (constructor argument 1 is the 3D data value), `:43` (`STREAM_CODEC = ByteBufCodecs.idMapper(BY_ID, get3DDataValue)`, i.e. VarInt), `:205` (`get3DDataValue`).

| Direction | 3D id |
|---|---|
| DOWN | 0 |
| UP | 1 |
| NORTH | 2 |
| SOUTH | 3 |
| WEST | 4 |
| EAST | 5 |

Wire depends on the codec used:
- `Direction.STREAM_CODEC` (used by BlockHitResult, `BlockHitResult.java:13`): VarInt.
- `ServerboundPlayerActionPacket` (`ServerboundPlayerActionPacket.java:33, 40`): u8 (`readUnsignedByte` / `writeByte(get3DDataValue())`).
- `EntityDataSerializers.DIRECTION` (`EntityDataSerializers.java:119`): uses `Direction.STREAM_CODEC`, so VarInt.

### 1.5 Vec3

- `Vec3.STREAM_CODEC` (`net/minecraft/world/phys/Vec3.java:25-37`): f64 x, f64 y, f64 z (24 bytes).
- `Vec3.LP_STREAM_CODEC` (`Vec3.java:38`): LpVec3 (section 1.6).

### 1.6 LpVec3 (compressed velocity, `net/minecraft/network/LpVec3.java`)

Constants (lines 8-18): DATA_BITS=15, MAX_QUANTIZED=32766.0, SCALE_BITS=2, CONTINUATION_FLAG=4, X_OFFSET=3, Y_OFFSET=18, Z_OFFSET=33, ABS_MAX=1.7179869183E10, ABS_MIN=3.051944088384301E-5.

Encoding (`write`, lines 41-63):
1. Sanitize each component (NaN -> 0, clamp to +-ABS_MAX).
2. `chessboard = max(|x|, |y|, |z|)`. If `chessboard < ABS_MIN`, write a single byte `0x00` and stop.
3. `scale = ceil(chessboard)` as long. `isPartial = (scale & 3) != scale`.
4. `markers = isPartial ? (scale & 3) | 4 : scale`.
5. Each component `q = round((c / scale * 0.5 + 0.5) * 32766)` (15-bit), shifted: x<<3, y<<18, z<<33.
6. `buffer = markers | xn | yn | zn` (48 bits used).
7. Write byte0 = (byte)buffer, byte1 = (byte)(buffer >> 8), then i32 BE of (int)(buffer >> 16) (4 bytes). That is 6 bytes total, covering bits 0..47.
8. If `isPartial`, write VarInt `(scale >> 2)`.

Decoding (`read`, lines 24-39): read u8 lowest. If 0, return ZERO (1 byte total). Else read u8 middle, u32 high (`readUnsignedInt`), `buffer = high << 16 | middle << 8 | lowest`. `scale = lowest & 3`. If `lowest & 4`, `scale |= VarInt << 2`. Each component `= unpack(buffer >> shift) * scale`, where unpack(v) = `min(v & 0x7FFF, 32766) * 2 / 32766 - 1`, shifts 3, 18, 33.

Byte layout summary: `[u8 b0][u8 b1][i32 b2..b5]([VarInt scale>>2] if b0 & 4)`. Bits 0-1 scale, bit 2 continuation, bits 3-17 x, 18-32 y, 33-47 z.

### 1.7 Vec3 velocity in AddEntity / SetEntityMotion

Both use LpVec3 (section 1.6): `ClientboundAddEntityPacket.java:104,118`, `ClientboundSetEntityMotionPacket.java:15`.

### 1.8 VecDelta (movement deltas) and PositionPath

`VecDelta` (`net/minecraft/network/protocol/game/VecDelta.java`):
- Read (lines 23-47), input `stepCount` (from MoveEntity properties):
  - `stepCount <= 0`: Linear = i16 xa, i16 ya, i16 za.
  - `stepCount > 0`: `stepCount` x { VarInt ticks, i16 xa, i16 ya, i16 za }.
- Write (lines 49-74): same order. For Linear, 3 x i16. For Stepped, per step VarInt ticks then 3 x i16.

Delta scale (`VecDeltaCodec.java`): `encode(v) = round(v * 4096)` (line 16), `decode(v) = v / 4096.0` (line 21). Per-axis delta = `encode(newPos) - encode(base)` (lines 58-66), where `base` is the previously sent position held by the encoder. The delta must fit in i16 (`isDeltaTooBig`, lines 28-30). When it does not fit, `VecDelta.tryEncode`/`Linear` return null (`VecDelta.java:148-172`, `VecDeltaCodec.java:55`); the caller then has to send an absolute update instead (caller not read in this pass).

`PositionPath` (`net/minecraft/world/entity/PositionPath.java`):
- `STREAM_CODEC` (line 12) = `Type.STREAM_CODEC.dispatch(type)`. `Type` is an enum (lines 51-66): LINEAR=0, STEPPED=1 (ordinal, written as VarInt via `idMapper(... Enum::ordinal)`).
- LINEAR: `Vec3.STREAM_CODEC` (f64 x3), lines 26-28.
- STEPPED: collection of `PositionStep` = VarInt count, then count x PositionStep.
- `PositionStep` (`net/minecraft/world/entity/PositionStep.java:8-11`): Vec3 (f64 x3), VarInt tickOffset.

`PositionMoveRotation` (`net/minecraft/world/entity/PositionMoveRotation.java:12-23`): Vec3 position (f64 x3), Vec3 deltaMovement (f64 x3), f32 yRot, f32 xRot. Not LpVec3.

`Relative` (`net/minecraft/world/entity/Relative.java`): bit index = ordinal of constant `bit` field. Set codec `SET_STREAM_CODEC = ByteBufCodecs.INT.map(...)` (line 24): **i32 BE bitmask**.

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

### 1.9 Holder encodings used here

- **SoundEvent** (`net/minecraft/sounds/SoundEvent.java`): `STREAM_CODEC = holder(SOUND_EVENT, DIRECT_STREAM_CODEC)` (lines 27-29). Direct form (`DIRECT_STREAM_CODEC`, lines 24-26): Identifier (String), then Optional<f32> fixedRange (bool + f32). Reference form: VarInt (registry id + 1).
- **Item** (`Item.java:116`): `holderRegistry(ITEM)` = VarInt registry id (no inline form). The numeric item ids come from registry sync and are UNCONFIRMED here.
- **EntityType** (`ClientboundAddEntityPacket.java:100,114`): `registry(ENTITY_TYPE)` = VarInt registry id.
- **DimensionType** (`DimensionType.java:55`): `holderRegistry` = VarInt.

### 1.10 ItemStack (network form)

Source: `net/minecraft/world/item/ItemStack.java:125` (`OPTIONAL_STREAM_CODEC`), `createOptionalStreamCodec` (about lines 163-202).

```
VarInt count                  // 0 (or <=0) => empty stack, nothing follows
if count > 0:
  holderRegistry(ITEM)        // VarInt item registry id (Item.java:116)
  DataComponentPatch          // see 1.11
```

- `OPTIONAL_UNTRUSTED_STREAM_CODEC` (line 126) uses `DataComponentPatch.DELIMITED_STREAM_CODEC`: each component value is additionally wrapped in a VarInt length prefix (`DataComponentPatch.java:54-62`, `registryFriendlyLengthPrefixed`).
- `OPTIONAL_LIST_STREAM_CODEC` (line 149): collection of OPTIONAL_STREAM_CODEC items: VarInt count, then each item.

### 1.11 DataComponentPatch (`net/minecraft/core/component/DataComponentPatch.java`)

`STREAM_CODEC` (lines 48-53), decode in `createStreamCodec` (lines 66-89):

```
VarInt positiveCount          // components that are set
VarInt negativeCount          // components that are removed
repeat positiveCount:
  DataComponentType<?>        // VarInt registry id (DataComponentType.java:18 = registry(DATA_COMPONENT_TYPE))
  value                       // per-type codec: type.streamCodec() (NOT resolved here)
repeat negativeCount:
  DataComponentType<?>        // VarInt registry id only, marks removal
```

Encode of an empty patch = VarInt 0, VarInt 0 (`encode`, start of the method, `DataComponentPatch.java:~80-88`). The per-component value codecs are per component type and are not resolved in this file (UNCONFIRMED: the component value layouts themselves).

### 1.12 HashedStack / HashedPatchMap (used by ContainerClick)

`net/minecraft/network/HashedStack.java`:
- `STREAM_CODEC` (lines 24-28) = `ByteBufCodecs.optional(ActualItem.STREAM_CODEC)`: bool, then ActualItem if true. False = EMPTY.
- `ActualItem.STREAM_CODEC` (lines 39-46) composite: `holderRegistry(ITEM)` (VarInt item id), VarInt count, HashedPatchMap.

`net/minecraft/network/HashedPatchMap.java:17-21`:
- `addedComponents`: `ByteBufCodecs.map(HashMap::new, registry(DATA_COMPONENT_TYPE), INT, 256)`: VarInt count (max 256), then count x { VarInt component type id, i32 BE hash }.
- `removedComponents`: `ByteBufCodecs.collection(HashSet::new, registry(DATA_COMPONENT_TYPE), 256)`: VarInt count (max 256), then count x VarInt component type id.
- The map codec body was not read in this pass (`ByteBufCodecs.map`), so the key/value pair ordering follows the standard map helper. UNCONFIRMED.

### 1.13 Other small types

- **GameType** (`net/minecraft/world/level/GameType.java:19-22, 27-31`): SURVIVAL=0, CREATIVE=1, ADVENTURE=2, SPECTATOR=3. `STREAM_CODEC` = VarInt. `OPTIONAL_STREAM_CODEC` = VarInt, 0 = absent, else id + 1 (`OPTIONAL_VAR_INT`, ByteBufCodecs.java:130-132).
- **InteractionHand** (`net/minecraft/world/InteractionHand.java:5-18`): MAIN_HAND=0, OFF_HAND=1. VarInt (`idMapper`).
- **HumanoidArm** (`net/minecraft/world/entity/HumanoidArm.java:13-18`): LEFT=0, RIGHT=1. VarInt (`idMapper`).
- **SoundSource** (`net/minecraft/sounds/SoundSource.java:3-14`): MASTER=0, MUSIC=1, RECORDS=2, WEATHER=3, BLOCKS=4, HOSTILE=5, NEUTRAL=6, PLAYERS=7, AMBIENT=8, VOICE=9, UI=10. VarInt (writeEnum).
- **Pose** (`net/minecraft/world/entity/Pose.java:12-33`): STANDING=0, FALL_FLYING=1, SLEEPING=2, SWIMMING=3, SPIN_ATTACK=4, CROUCHING=5, LONG_JUMPING=6, DYING=7, CROAKING=8, USING_TONGUE=9, SITTING=10, ROARING=11, SNIFFING=12, EMERGING=13, DIGGING=14, SLIDING=15, SHOOTING=16, INHALING=17. VarInt (idMapper).
- **ContainerInput** (`net/minecraft/world/inventory/ContainerInput.java:9-19`): PICKUP=0, QUICK_MOVE=1, SWAP=2, CLONE=3, THROW=4, QUICK_CRAFT=5, PICKUP_ALL=6. VarInt (idMapper).
- **Rotations** (`net/minecraft/core/Rotations.java:16-26`): f32 x, f32 y, f32 z (3 x f32, 12 bytes).
- **Component** (`ComponentSerialization.java:38-41`): `STREAM_CODEC`/`TRUSTED_STREAM_CODEC` = `ByteBufCodecs.fromCodecWithRegistries(...)`. The NBT body was not read in this pass. UNCONFIRMED: assumed network NBT (unnamed root). Optional form is bool + component.
- **ParticleOptions** (`net/minecraft/core/particles/ParticleTypes.java:175`): `registry(PARTICLE_TYPE)` (VarInt type id), then per-type options (per-type payload; UNCONFIRMED except for simple particles, which have none).
- **EntityReference** (`EntityDataSerializers.java:120-121`): UNCONFIRMED body (the `EntityReference.streamCodec()` was not read).
- **ResolvableProfile**, **VillagerData**, variant holders (cat, cow, wolf, frog, pig, chicken, painting, etc.): UNCONFIRMED (not read). They are used only as entity data values (section 3).

---

## 2. Clientbound entity movement and spawn

### 2.1 ClientboundAddEntityPacket (`network/protocol/game/ClientboundAddEntityPacket.java`)

STREAM_CODEC via `write` (lines 111-123), read (lines 97-109).

| # | Field | Wire | Source |
|---|---|---|---|
| 1 | entity id | VarInt | 112 |
| 2 | uuid | 2 x i64 BE | 113 |
| 3 | entity type | VarInt (registry ENTITY_TYPE id) | 114 |
| 4 | x | f64 | 115 |
| 5 | y | f64 | 116 |
| 6 | z | f64 | 117 |
| 7 | velocity (movement) | LpVec3 (section 1.6) | 118 |
| 8 | xRot | i8 (packed angle) | 119 |
| 9 | yRot | i8 (packed angle) | 120 |
| 10 | yHeadRot | i8 (packed angle) | 121 |
| 11 | data | VarInt | 122 |

Angle packing (`Mth.packDegrees`, `net/minecraft/util/Mth.java:182-184`): `byte = (byte) floor(deg * 256 / 360)`. Unpack: `deg = byte * 360 / 256` (`Mth.java:186-188`).

Constructor (lines 71-95): xRot/yRot are packed from floats, yHeadRot packed from double.

### 2.2 ClientboundMoveEntityPacket (`network/protocol/game/ClientboundMoveEntityPacket.java`)

Abstract base; subclasses are chosen by packet id.

Common flag constants (lines 13-33):
- `ON_GROUND_FLAG = 1` (bit 0).
- `STEP_COUNT_OFFSET = 1`. `properties = (onGround ? 1 : 0) | stepCount << 1`. So `stepCount = properties >>> 1`.

**Pos** (lines 90-116). Read lines 99-104, write lines 106-110.
```
VarInt entityId
VarInt properties        // bit0 = onGround, bits1.. = stepCount
VecDelta                 // stepCount <= 0 => 3 x i16; else stepCount x {VarInt ticks, 3 x i16} (section 1.8)
```

**PosRot** (lines 118-148). Read 127-134, write 136-142.
```
VarInt entityId
VarInt properties
VecDelta
i8 yRot                  // packed angle
i8 xRot                  // packed angle
```

**Rot** (lines 150-178). Read 159-165, write 167-172. **Note: onGround is a plain bool, not the properties VarInt.**
```
VarInt entityId
bool onGround
i8 yRot
i8 xRot
```

Delta encoding (section 1.8): each i16 is `round(new * 4096) - round(base * 4096)`, base = previous sent position for that entity.

### 2.3 ClientboundEntityPositionSyncPacket (`network/protocol/game/ClientboundEntityPositionSyncPacket.java:10-24`)

```
VarInt id
PositionPath                // VarInt type (0 = LINEAR: f64x3; 1 = STEPPED: VarInt count + PositionStep[])
f32 yRot                    // full float, NOT packed byte
f32 xRot                    // full float
bool onGround
```

### 2.4 ClientboundTeleportEntityPacket (`network/protocol/game/ClientboundTeleportEntityPacket.java:12-24`)

```
VarInt id
PositionMoveRotation        // f64x3 position, f64x3 deltaMovement, f32 yRot, f32 xRot
i32 relatives               // Relative set bitmask (section 1.8), ByteBufCodecs.INT
bool onGround
```

### 2.5 ClientboundRotateHeadPacket (`network/protocol/game/ClientboundRotateHeadPacket.java:24-32`)

```
VarInt entityId
i8 yHeadRot                 // packed angle (1/256 turn)
```

### 2.6 ClientboundSetEntityMotionPacket (`network/protocol/game/ClientboundSetEntityMotionPacket.java:12-18`)

```
VarInt id
LpVec3 movement             // section 1.6
```

### 2.7 ClientboundRemoveEntitiesPacket (`network/protocol/game/ClientboundRemoveEntitiesPacket.java:12-15`)

```
VarInt count
VarInt entityId x count     // IntList via collection(VAR_INT)
```

---

## 3. Entity metadata (SynchedEntityData)

### 3.1 Packet: ClientboundSetEntityDataPacket (`network/protocol/game/ClientboundSetEntityDataPacket.java`)

Codec (lines 12-14), write (42-45), pack (21-29), unpack (31-40).

```
VarInt entityId
repeat until terminator:
  u8 index                  // 0..254; 0xFF (255) = end of list (EOF_MARKER, line 15)
  VarInt serializerId       // EntityDataSerializers id (section 3.2)
  value                     // serializer.codec().encode(value)
u8 0xFF                     // terminator (line 28)
```

Decode loop (lines 34-37): `while ((id = readUnsignedByte()) != 255)`.

`SynchedEntityData.DataValue` (`net/minecraft/network/syncher/SynchedEntityData.java:242-251`): write = `writeByte(id)`, `writeVarInt(serializerId)`, `codec.encode(value)`. Read (lines 253-265) mirrors it. Max index 254 (`MAX_ID_VALUE`, line 18).

### 3.2 Serializer registration order (`net/minecraft/network/syncher/EntityDataSerializers.java`)

The id is the registration index (`CrudeIncrementalIntIdentityHashBiMap.add` assigns `nextId()` starting from 0, `net/minecraft/util/CrudeIncrementalIntIdentityHashBiMap.java:60-63`). Registration calls are lines 188-231 (static block, only call site of `registerSerializer`). 44 serializers, ids 0-43.

| id | Constant | Wire value | Source (value codec) |
|---|---|---|---|
| 0 | BYTE | i8 | line 54, ByteBufCodecs.BYTE |
| 1 | INT | VarInt | 55 |
| 2 | LONG | VarLong | 56 |
| 3 | FLOAT | f32 | 57 |
| 4 | STRING | String (UTF-8, VarInt len) | 58 |
| 5 | COMPONENT | Component (NBT, see 1.13) | 59 |
| 6 | OPTIONAL_COMPONENT | bool + Component | 60 |
| 7 | ITEM_STACK | ItemStack (1.10) | 80-90 (OVERSIZED_ITEM_CODEC = OPTIONAL_STREAM_CODEC) |
| 8 | BOOLEAN | bool | 109 |
| 9 | ROTATIONS | 3 x f32 | 114 |
| 10 | BLOCK_POS | i64 packed | 115 |
| 11 | OPTIONAL_BLOCK_POS | bool + i64 | 116-118 |
| 12 | DIRECTION | VarInt (3D id) | 119 |
| 13 | OPTIONAL_LIVING_ENTITY_REFERENCE | bool + EntityReference (UNCONFIRMED body) | 120-122 |
| 14 | BLOCK_STATE | VarInt global block-state id (idMapper) | 91 |
| 15 | OPTIONAL_BLOCK_STATE | VarInt (0 = empty, else state id) | 92-108 (OPTIONAL_BLOCK_STATE_CODEC; 0 = absent, else `Block.getId` state id) |
| 16 | PARTICLE | ParticleOptions (1.13) | 110 |
| 17 | PARTICLES | VarInt count + ParticleOptions[] | 111-113 |
| 18 | VILLAGER_DATA | VillagerData (UNCONFIRMED body) | 126 |
| 19 | OPTIONAL_UNSIGNED_INT | VarInt (0 = absent, else value + 1) | 127-139 |
| 20 | POSE | VarInt (Pose id, 1.13) | 140 |
| 21 | CAT_VARIANT | holder (variant codec body UNCONFIRMED) | reg. line 209 |
| 22 | CAT_SOUND_VARIANT | holder (UNCONFIRMED) | 210 |
| 23 | COW_VARIANT | holder (UNCONFIRMED) | 211 |
| 24 | COW_SOUND_VARIANT | holder (UNCONFIRMED) | 212 |
| 25 | WOLF_VARIANT | holder (UNCONFIRMED) | 213 |
| 26 | WOLF_SOUND_VARIANT | holder (UNCONFIRMED) | 214 |
| 27 | FROG_VARIANT | holder (UNCONFIRMED) | 215 |
| 28 | PIG_VARIANT | holder (UNCONFIRMED) | 216 |
| 29 | PIG_SOUND_VARIANT | holder (UNCONFIRMED) | 217 |
| 30 | CHICKEN_VARIANT | holder (UNCONFIRMED) | 218 |
| 31 | CHICKEN_SOUND_VARIANT | holder (UNCONFIRMED) | 219 |
| 32 | ZOMBIE_NAUTILUS_VARIANT | holder (UNCONFIRMED) | 220 |
| 33 | OPTIONAL_GLOBAL_POS | bool + GlobalPos (Identifier dimension + i64 BlockPos, GlobalPos.java:18-20) | 221 |
| 34 | PAINTING_VARIANT | holder (UNCONFIRMED) | 222 |
| 35 | SNIFFER_STATE | Sniffer.State enum codec (UNCONFIRMED form) | 223 |
| 36 | ARMADILLO_STATE | Armadillo.ArmadilloState codec (UNCONFIRMED form) | 224 |
| 37 | COPPER_GOLEM_STATE | CopperGolemState codec (UNCONFIRMED form) | 225 |
| 38 | WEATHERING_COPPER_STATE | WeatheringCopper.WeatherState codec (UNCONFIRMED form) | 226 |
| 39 | VECTOR3 | Vector3f via ByteBufCodecs.VECTOR3F (UNCONFIRMED body, presumably 3 x f32) | 227 |
| 40 | QUATERNION | Quaternionf via ByteBufCodecs.QUATERNIONF (UNCONFIRMED body) | 228 |
| 41 | RESOLVABLE_PROFILE | ResolvableProfile.STREAM_CODEC (UNCONFIRMED body) | 229 |
| 42 | HUMANOID_ARM | VarInt (LEFT=0, RIGHT=1) | 230 |
| 43 | DYE_COLOR | DyeColor.STREAM_CODEC (UNCONFIRMED form) | 231 |

Important: the order above is the **registration** order (lines 188-231), not the field declaration order. Example: BLOCK_STATE is declared at line 91 but registered 14th (line 202). Every row's "reg. line" is the `registerSerializer(...)` line in the static block; the id is that line's index minus 188.

Rows 21-43 with "holder" or UNCONFIRMED values come from variant codecs that were not read in this pass.

### 3.3 Entity data indices

Index allocation: `SynchedEntityData.defineId(clazz, serializer)` (`SynchedEntityData.java:29-46`) calls `ClassTreeIdRegistry.define(clazz)` (`net/minecraft/util/ClassTreeIdRegistry.java:32-37`). `define` returns `lastIdOfNearestDefinedSuperclass + 1`, or 0 if none. So a subclass continues numbering after its superclass's last index. Static initialization runs superclass first, so the numbering is deterministic.

Allocation (derived from source):

| Class | Indices | Notes |
|---|---|---|
| Entity | 0-7 | 8 defines (Entity.java:313-329) |
| ItemEntity | 8 | ItemEntity.java:41 |
| PrimedTnt | 8, 9 | fuse 8, block state 9 (PrimedTnt.java:41-42) |
| LivingEntity | 8-14 | 7 defines (LivingEntity.java:195-203) |
| Avatar | 15, 16 | main hand 15, skin/mode 16 (Avatar.java:38-39) |
| Player | 17-20 | absorption 17, score 18, left shoulder 19, right shoulder 20 (Player.java:135-140) |

Concrete class counts: `SynchedEntityData.Builder` allocates `getCount(clazz) = lastId + 1` (lines 148-150). So ServerPlayer = 21 entries, ItemEntity = 9, etc.

#### 3.3.1 Entity base (indices 0-7)

| Index | Field | Serializer id | Default / note |
|---|---|---|---|
| 0 | DATA_SHARED_FLAGS_ID | 0 (BYTE) | flag bits below |
| 1 | DATA_AIR_SUPPLY_ID | 1 (INT) | air ticks |
| 2 | DATA_CUSTOM_NAME | 6 (OPTIONAL_COMPONENT) | Entity.java:322 |
| 3 | DATA_CUSTOM_NAME_VISIBLE | 8 (BOOLEAN) | |
| 4 | DATA_SILENT | 8 (BOOLEAN) | |
| 5 | DATA_NO_GRAVITY | 8 (BOOLEAN) | |
| 6 | DATA_POSE | 20 (POSE) | |
| 7 | DATA_TICKS_FROZEN | 1 (INT) | |

Shared flags byte (index 0), bit values from `Entity.java:314-320` (mask = 1 << bit):

| Bit | Mask | Meaning |
|---|---|---|
| 0 | 0x01 | on fire (FLAG_ONFIRE) |
| 1 | 0x02 | shift key down / crouching (FLAG_SHIFT_KEY_DOWN) |
| 3 | 0x08 | sprinting (FLAG_SPRINTING) |
| 4 | 0x10 | swimming (FLAG_SWIMMING) |
| 5 | 0x20 | invisible (FLAG_INVISIBLE) |
| 6 | 0x40 | glowing (FLAG_GLOWING) |
| 7 | 0x80 | fall flying (FLAG_FALL_FLYING) |

Bit 2 is not defined here.

#### 3.3.2 ItemEntity

- Index 8: item stack, serializer 7 (ITEM_STACK). Source `ItemEntity.java:41`.

#### 3.3.3 PrimedTnt

- Index 8: fuse, serializer 1 (INT, VarInt). `PrimedTnt.java:41`.
- Index 9: block state, serializer 14 (BLOCK_STATE, VarInt global state id). `PrimedTnt.java:42`.

#### 3.3.4 LivingEntity (indices 8-14)

| Index | Field | Serializer |
|---|---|---|
| 8 | DATA_LIVING_ENTITY_FLAGS (BYTE) | 0 |
| 9 | DATA_HEALTH_ID (FLOAT) | 3 |
| 10 | DATA_EFFECT_PARTICLES (List<ParticleOptions>) | 17 |
| 11 | DATA_EFFECT_AMBIENCE_ID (BOOLEAN) | 8 |
| 12 | DATA_ARROW_COUNT_ID (INT) | 1 |
| 13 | DATA_STINGER_COUNT_ID (INT) | 1 |
| 14 | SLEEPING_POS_ID (Optional<BlockPos>) | 11 |

Living flags byte (index 8): bit 0 mask 0x01 = using item (`LIVING_ENTITY_FLAG_IS_USING`), bit 1 mask 0x02 = off-hand active (`LIVING_ENTITY_FLAG_OFF_HAND`), bit 2 mask 0x04 = spin attack (`LIVING_ENTITY_FLAG_SPIN_ATTACK`). `LivingEntity.java:192-194`.

#### 3.3.5 Avatar and Player

- Index 15: DATA_PLAYER_MAIN_HAND, serializer 42 (HUMANOID_ARM, VarInt 0=LEFT, 1=RIGHT). `Avatar.java:38`.
- Index 16: DATA_PLAYER_MODE_CUSTOMISATION, serializer 0 (BYTE). Skin parts bitmask. `Avatar.java:39`. Bits from `PlayerModelPart` (`net/minecraft/world/entity/player/PlayerModelPart.java:8-14`): CAPE=bit 0 (0x01), JACKET=bit 1 (0x02), LEFT_SLEEVE=bit 2 (0x04), RIGHT_SLEEVE=bit 3 (0x08), LEFT_PANTS_LEG=bit 4 (0x10), RIGHT_PANTS_LEG=bit 5 (0x20), HAT=bit 6 (0x40).
- Index 17: absorption, serializer 3 (FLOAT). `Player.java:135`.
- Index 18: score, serializer 1 (INT). `Player.java:136`.
- Index 19: left shoulder parrot, serializer 19 (OPTIONAL_UNSIGNED_INT). `Player.java:137`.
- Index 20: right shoulder parrot, serializer 19 (OPTIONAL_UNSIGNED_INT). `Player.java:140`.

---

## 4. Other clientbound packets

### 4.1 ClientboundAnimatePacket (`ClientboundAnimatePacket.java:9-32`)

```
VarInt entityId
u8 action            // 0 WAKE_UP, 1 CRITICAL_HIT, 2 MAGIC_CRITICAL_HIT (lines 13-15)
```

Other action values (e.g. swing) are not defined here. Swing animations use `ClientboundSwingAnimationPacket` (separate packet, not read in this pass).

### 4.2 ClientboundEntityEventPacket (`ClientboundEntityEventPacket.java:12-32`)

```
i32 entityId         // readInt, NOT VarInt
i8 eventId           // @EntityEvent.Value
```

### 4.3 ClientboundTakeItemEntityPacket (`ClientboundTakeItemEntityPacket.java:8-32`)

```
VarInt itemEntityId
VarInt collectorId   // player (or entity) that picked it up
VarInt amount
```

### 4.4 ClientboundPlayerInfoRemovePacket (`ClientboundPlayerInfoRemovePacket.java:12-15`)

```
VarInt count
UUID x count         // UUIDUtil.STREAM_CODEC, 2 x i64 each
```

### 4.5 ClientboundPlayerInfoUpdatePacket (`ClientboundPlayerInfoUpdatePacket.java`)

Codec (lines 30-32), read (131-134), write (136-139).

```
EnumSet<Action> as fixed bitset     // ceil(8/8) = 1 byte; bit i = ordinal i (see table)
VarInt entryCount
entries: for each entry:
  UUID profileId                    // 2 x i64
  for each action in ordinal order whose bit is set:
    action payload (below)
```

Entry payload is written in **ordinal order** (EnumSet iteration order), i.e. the declaration order below (`ClientboundPlayerInfoUpdatePacket.java:168-202`). The `id` argument in the enum constructors is not the wire bit.

| Ordinal (bit) | Action | Payload | Source |
|---|---|---|---|
| 0 | ADD_PLAYER | String name (`PLAYER_NAME` = stringUtf8(16)), then GAME_PROFILE_PROPERTIES | 169-177 |
| 1 | INITIALIZE_CHAT | Nullable<RemoteChatSession.Data> (bool; if true: UUID sessionId, ProfilePublicKey.Data: i64 expiresAt (INSTANT), PublicKey (UNCONFIRMED body), byte[] keySignature (UNCONFIRMED body)) | 178-190, `RemoteChatSession.java:29-34`, `ProfilePublicKey.java:49-53` |
| 2 | UPDATE_GAME_MODE | VarInt GameType | 191-192 |
| 3 | UPDATE_LISTED | bool | 194 |
| 4 | UPDATE_LATENCY | VarInt | 195 |
| 5 | UPDATE_DISPLAY_NAME | Nullable<Component> (bool; if true, Component) | 196-200 |
| 6 | UPDATE_LIST_ORDER | VarInt | 201 |
| 7 | UPDATE_HAT | bool | 202 |

GAME_PROFILE_PROPERTIES (`ByteBufCodecs.java:259-285`):
```
VarInt count (max 16)
repeat count:
  String name (max 64)
  String value (max 32767)
  Nullable<String> signature (bool; if true, String max 1024)
```

Example: `createPlayerInitializing` (`ClientboundPlayerInfoUpdatePacket.java:57-68`) sets all eight actions, so the bitset is one byte `0xFF`. Other action sets are built at runtime.

### 4.6 ClientboundBlockUpdatePacket (`ClientboundBlockUpdatePacket.java:14-20`)

```
i64 BlockPos                  // section 1.2
VarInt blockStateId           // Block.BLOCK_STATE_REGISTRY id (idMapper)
```

### 4.7 ClientboundSectionBlocksUpdatePacket (`ClientboundSectionBlocksUpdatePacket.java:66-72`, read 53-63)

```
i64 SectionPos                // section 1.3
VarInt count
repeat count:
  VarLong ((long)blockStateId << 12) | relativeIndex   // section 1.3 for relative index (12 bits)
```

Decode: `positions = packed & 0xFFF`, `stateId = packed >>> 12`.

### 4.8 ClientboundBlockChangedAckPacket (`ClientboundBlockChangedAckPacket.java:13-19`)

```
VarInt sequence
```

### 4.9 ClientboundBlockDestructionPacket (`ClientboundBlockDestructionPacket.java:23-33`)

```
VarInt breakerEntityId
i64 BlockPos                  // readBlockPos
u8 progress                   // 0..9 (readUnsignedByte)
```

### 4.10 ClientboundExplodePacket (`ClientboundExplodePacket.java:17-45`)

Fields in exact wire order (STREAM_CODEC, lines 27-45):

```
f64 x, f64 y, f64 z            // Vec3.STREAM_CODEC (center)
f32 radius                     // ByteBufCodecs.FLOAT
i32 blockCount                 // ByteBufCodecs.INT (NOT VarInt)
Optional<Vec3>                 // bool; if true: f64 x, f64 y, f64 z (player knockback)
ParticleOptions                // VarInt particle type id, then per-type payload
SoundEvent holder              // section 1.9 (VarInt 0 => inline Identifier + Optional<f32>; else VarInt id+1)
collection<ExplosionParticleInfo>   // VarInt count; each: ParticleOptions, f32 scaling, f32 speed, VarInt weight
bool playSound
```

`ExplosionParticleInfo` (`net/minecraft/core/particles/ExplosionParticleInfo.java:19-26`): ParticleOptions, f32 scaling, f32 speed. The `WeightedList.streamCodec` (`net/minecraft/util/random/WeightedList.java:127-129`) wraps each element in `Weighted` (`net/minecraft/util/random/Weighted.java:44-46`), which appends VarInt weight.

### 4.11 ClientboundSoundPacket (`ClientboundSoundPacket.java:12-65`)

```
SoundEvent holder              // section 1.9
VarInt source                  // SoundSource ordinal (section 1.13)
i32 x                          // (int)(x * 8.0)  (readInt)
i32 y                          // (int)(y * 8.0)
i32 z                          // (int)(z * 8.0)
f32 volume
f32 pitch
i64 seed
```

`LOCATION_ACCURACY = 8.0F` (line 15) is the fixed-point scale.

### 4.12 ClientboundLevelEventPacket (`ClientboundLevelEventPacket.java:26-38`)

```
i32 type                       // readInt (LevelEvent id)
i64 BlockPos                   // readBlockPos
i32 data                       // readInt
bool globalEvent               // readBoolean
```

### 4.13 ClientboundContainerSetContentPacket (`ClientboundContainerSetContentPacket.java:13-20`)

```
VarInt containerId             // CONTAINER_ID = VarInt
VarInt stateId
VarInt itemCount
  ItemStack[itemCount]         // OPTIONAL_STREAM_CODEC each (section 1.10)
ItemStack carriedItem          // OPTIONAL_STREAM_CODEC (VarInt count; 0 = empty), no leading bool
```

Note: `items` uses `OPTIONAL_LIST_STREAM_CODEC`, so the list is VarInt count + items. The carried item is an ItemStack with `OPTIONAL_STREAM_CODEC` (no separate bool), i.e. `VarInt count` then the rest. Paper hook: if the set is too large, it falls back to per-slot `ClientboundContainerSetSlotPacket` (lines 27-37).

### 4.14 ClientboundContainerSetSlotPacket (`ClientboundContainerSetSlotPacket.java:25-35`)

```
VarInt containerId
VarInt stateId
i16 slot
ItemStack                      // OPTIONAL_STREAM_CODEC
```

### 4.15 ClientboundSetPlayerInventoryPacket (`ClientboundSetPlayerInventoryPacket.java:12-17`)

```
VarInt slot
ItemStack                      // OPTIONAL_STREAM_CODEC
```

### 4.16 ClientboundSetCursorItemPacket (`ClientboundSetCursorItemPacket.java:10-12`)

```
ItemStack                      // OPTIONAL_STREAM_CODEC (the cursor/carried item)
```

### 4.17 ClientboundRespawnPacket (`ClientboundRespawnPacket.java:10-16`) and CommonPlayerSpawnInfo

```
CommonPlayerSpawnInfo:
  VarInt dimensionType holder            // DimensionType.STREAM_CODEC = holderRegistry (VarInt id)
  Identifier dimension                   // ResourceKey.streamCodec(DIMENSION) (UNCONFIRMED: Identifier string)
  i64 seed                               // ByteBufCodecs.LONG
  VarInt gameType                        // GameType (section 1.13)
  VarInt previousGameType (opt)          // OPTIONAL_STREAM_CODEC: 0 = none, else id + 1
  bool isDebug
  bool isFlat
  Optional<GlobalPos>                    // bool; if true: Identifier dimension, i64 BlockPos
  VarInt portalCooldown
  VarInt seaLevel
i8 dataToKeep                            // ByteBufCodecs.BYTE; 0x01 = keep attribute modifiers, 0x02 = keep entity data (ClientboundRespawnPacket.java:17-19)
```

`CommonPlayerSpawnInfo.java:15-47`. `GameType.OPTIONAL_STREAM_CODEC` (`GameType.java:28-31`).

### 4.18 ClientboundCommandsPacket (`ClientboundCommandsPacket.java:32-43`, codec lines 36-38)

Body:
```
VarInt entryCount
Entry[entryCount]
VarInt rootIndex
```

Entry (`readNode` lines 113-119, `Entry.write` lines 231-242):
```
u8 flags
VarIntArray children        // VarInt count + VarInt indices
if (flags & 0x08) VarInt redirectIndex
node stub (by type = flags & 0x03):
  0 root:    nothing
  1 literal: String literal (writeUtf)
  2 argument: String name, VarInt argumentTypeRegistryId, argument properties (below),
              if (flags & 0x10): Identifier suggestionId
```

Flags (lines 39-43): `MASK_TYPE = 0x03` (0 root, 1 literal, 2 argument), `FLAG_EXECUTABLE = 0x04`, `FLAG_REDIRECT = 0x08`, `FLAG_CUSTOM_SUGGESTIONS = 0x10`, `FLAG_RESTRICTED = 0x20`.

#### 4.18.1 Argument type registry ids and property payloads

Registry (`net/minecraft/commands/synchronization/ArgumentTypeInfos.java`, bootstrap from line 79). Id = registration index. Registrations are sequential (`BuiltInRegistries.java:215`, `registerSimple`), so id = position in this list, starting at 0. The `minecraft:` namespace is the default for unprefixed names.

| Id | Name | Properties (wire) | Source |
|---|---|---|---|
| 0 | brigadier:bool | none | 80 |
| 1 | brigadier:float | u8 flags (bit0 has min, bit1 has max); f32 min if bit0; f32 max if bit1 | 81; FloatArgumentInfo.java:15-29 |
| 2 | brigadier:double | u8 flags; f64 min, f64 max (same rule) | 82; DoubleArgumentInfo.java:15-29 |
| 3 | brigadier:integer | u8 flags; i32 min, i32 max (same rule) | 83; IntegerArgumentInfo.java:12-31 |
| 4 | brigadier:long | u8 flags; i64 min, i64 max (same rule) | 84; LongArgumentInfo.java:15-29 |
| 5 | brigadier:string | VarInt StringType ordinal: 0 SINGLE_WORD (word), 1 QUOTABLE_PHRASE (string), 2 GREEDY_PHRASE (greedy). Brigadier library enum order, UNCONFIRMED | 85; StringArgumentSerializer.java:12-20 |
| 6 | minecraft:entity | u8 flags: bit0 = single, bit1 = playersOnly | 86; EntityArgument.java:166-188 |
| 7 | minecraft:game_profile | none | 87 |
| 8 | minecraft:block_pos | none | 88 |
| 9 | minecraft:column_pos | none | 89 |
| 10 | minecraft:vec3 | none | 90 |
| 11 | minecraft:vec2 | none | 91 |
| 12 | minecraft:block_state | none (singleton, contextAware) | 92; SingletonArgumentInfo.java:26-27 (empty body) |
| 13 | minecraft:block_predicate | none | 93 |
| 14 | minecraft:item_stack | none | 94 |
| 15 | minecraft:item_predicate | none | 95 |
| 16 | minecraft:team_color | none | 96 |
| 17 | minecraft:hex_color | none | 97 |
| 18 | minecraft:component | none | 98 |
| 19 | minecraft:style | none | 99 |
| 20 | minecraft:message | none | 100 |
| 21 | minecraft:nbt_compound_tag | none | 101 |
| 22 | minecraft:nbt_tag | none | 102 |
| 23 | minecraft:nbt_path | none | 103 |
| 24 | minecraft:objective | none | 104 |
| 25 | minecraft:objective_criteria | none | 105 |
| 26 | minecraft:operation | none | 106 |
| 27 | minecraft:particle | none (contextAware) | 107 |
| 28 | minecraft:angle | none | 108 |
| 29 | minecraft:rotation | none | 109 |
| 30 | minecraft:scoreboard_slot | none | 110 |
| 31 | minecraft:score_holder | ScoreHolderArgument.Info (UNCONFIRMED body) | 111 |
| 32 | minecraft:swizzle | none | 112 |
| 33 | minecraft:team | none | 113 |
| 34 | minecraft:item_slot | none | 114 |
| 35 | minecraft:item_slots | none | 115 |
| 36 | minecraft:resource_location | none | 116 |
| 37 | minecraft:function | none | 117 |
| 38 | minecraft:entity_anchor | none | 118 |
| 39 | minecraft:int_range | none | 119 |
| 40 | minecraft:float_range | none | 120 |
| 41 | minecraft:dimension | none | 121 |
| 42 | minecraft:gamemode | none | 122 |
| 43 | minecraft:time | TimeArgument.Info (UNCONFIRMED body) | 123 |
| 44-52 | resource_or_tag, resource_or_tag_key, resource, resource_key, resource_selector, template_mirror, template_rotation, heightmap, loot_table | see names; mostly none or Info-based (UNCONFIRMED bodies) | 124-132 |
| 53-59 | loot_predicate, loot_modifier, context_float_provider, context_int_provider, slot_source, dialog, feature | multi-line registrations, lines 133-156 | 133-156 |
| 60 | minecraft:swing_animation | none | 157 |
| 61 | minecraft:uuid | none | 158 |

Ids 0-20 and 60-61 are the ones the spec needs most. The 21-59 index assignments come from line order in `bootstrap` (`ArgumentTypeInfos.java:79-158`), counted from the single-line `register(...)` list. Re-count against the file before relying on a specific index in 21-59.

The `brigadier:*` names use the literal namespace `brigadier`. Unprefixed names (e.g. `entity`, `message`) are registered under the default `minecraft` namespace.

Checked against the source for this pass: brigadier:bool=0, brigadier:float=1, brigadier:double=2, brigadier:integer=3, brigadier:long=4, brigadier:string=5, minecraft:entity=6, minecraft:game_profile=7, minecraft:block_pos=8, minecraft:vec3=10, minecraft:block_state=12, minecraft:item_stack=14, minecraft:message=20, minecraft:gamemode=42.

---

## 5. Serverbound packets

### 5.1 ServerboundMovePlayerPacket (`network/protocol/game/ServerboundMovePlayerPacket.java`)

Flags byte: bit 0 (0x01) = onGround (FLAG_ON_GROUND, line 10), bit 1 (0x02) = horizontalCollision (FLAG_HORIZONTAL_COLLISION, line 11). Packed by `packFlags` (lines 22-33), read as u8.

- **Pos** (ID-selected), lines 109-143: `f64 x, f64 y, f64 z, u8 flags`.
- **PosRot**, lines 145-185: `f64 x, f64 y, f64 z, f32 yRot, f32 xRot, u8 flags`.
- **Rot**, lines 187-215: `f32 yRot, f32 xRot, u8 flags`.
- **StatusOnly**, lines 217-241: `u8 flags`.

Angles are full floats (degrees), not packed bytes. Only the variant chosen by packet id determines which fields are present.

### 5.2 ServerboundPlayerActionPacket (`ServerboundPlayerActionPacket.java:30-42`, enum lines 70-80)

```
VarInt action            // readEnum (VarInt ordinal)
i64 BlockPos             // readBlockPos
u8 direction             // Direction 3D id (writeByte(get3DDataValue))
VarInt sequence
```

Action ids: START_DESTROY_BLOCK=0, CHANGE_DESTROY_DIRECTION=1, ABORT_DESTROY_BLOCK=2, STOP_DESTROY_BLOCK=3, DROP_ALL_ITEMS=4, DROP_ITEM=5, RELEASE_USE_ITEM=6, SWAP_ITEM_WITH_OFFHAND=7, STAB=8.

### 5.3 ServerboundUseItemOnPacket (`ServerboundUseItemOnPacket.java:17-25`)

```
VarInt hand              // InteractionHand (section 1.13)
BlockHitResult:
  i64 BlockPos           // BlockPos.STREAM_CODEC
  VarInt Direction       // Direction.STREAM_CODEC (VarInt, NOT u8)
  f32 clickX             // hit - blockPos (relative), BlockHitResult.java:14-16, 30-32
  f32 clickY
  f32 clickZ
  bool inside
  bool worldBorderHit
VarInt sequence
```

The server reconstructs the hit location as `blockPos + (clickX, clickY, clickZ)` (`BlockHitResult.java:19-20`). `timestamp` is server-side only.

### 5.4 ServerboundUseItemPacket (`ServerboundUseItemPacket.java:16-24`)

```
VarInt hand
VarInt sequence
f32 yRot
f32 xRot
```

### 5.5 ServerboundSetCarriedItemPacket (`ServerboundSetCarriedItemPacket.java:18-23`)

```
i16 slot
```

### 5.6 ServerboundSetCreativeModeSlotPacket (`ServerboundSetCreativeModeSlotPacket.java:11-16`)

```
i16 slotNum              // ByteBufCodecs.SHORT
ItemStack                // validatedStreamCodec(OPTIONAL_UNTRUSTED_STREAM_CODEC) (DELIMITED patch, section 1.11)
```

### 5.7 ServerboundContainerClickPacket (`ServerboundContainerClickPacket.java:14-43`)

```
VarInt containerId           // CONTAINER_ID (VarInt)
VarInt stateId
i16 slotNum
i8 buttonNum                 // ByteBufCodecs.BYTE
VarInt containerInput        // ContainerInput ordinal (section 1.13)
map<i16 slot, HashedStack> changedSlots    // VarInt count (max 128), then count x { i16 slot, HashedStack }
HashedStack carriedItem      // bool; if true: VarInt holder id, VarInt count, HashedPatchMap (section 1.12)
```

`SLOTS_STREAM_CODEC` (lines 24-26) = `ByteBufCodecs.map(..., SHORT key, HashedStack.STREAM_CODEC value, 128)`. The map body is UNCONFIRMED (key/value order follows the standard helper).

### 5.8 ServerboundContainerClosePacket (`ServerboundContainerClosePacket.java:18-23`)

```
VarInt containerId
```

### 5.9 Swing: no serverbound swing packet in this version

`ServerboundSwingPacket` does not exist in `GamePacketTypes.java`. The serverbound arm-swing packet is **`ServerboundPunchPacket`** (`GamePacketTypes.java:183`, `createServerbound("punch")`). Its payload is empty (see 5.18). Only `ClientboundSwingAnimationPacket` (`ClientboundSwingAnimationPacket.java`) exists as clientbound. Wire layout of that clientbound packet was not read in this pass (UNCONFIRMED).

### 5.10 ServerboundChatPacket (`ServerboundChatPacket.java:15-27`)

```
String message                    // stringUtf8(256)  (VarInt len + UTF-8, max 256 chars)
i64 timestamp                     // INSTANT = epoch millis
i64 salt                          // LONG
Optional<MessageSignature>        // bool; if true: 256 raw bytes (MessageSignature.java:33-35, BYTES=256)
LastSeenMessages.Update           // VarInt offset; fixedBitSet(20) = 3 bytes; i8 checksum (LastSeenMessages.java:67-77)
```

### 5.11 ServerboundChatCommandPacket (`ServerboundChatCommandPacket.java:14-19`)

```
String command                    // readUtf(256 by default; system property paper.maxChatCommandInputSize)
```

### 5.12 ServerboundChatAckPacket (`ServerboundChatAckPacket.java:13-18`)

```
VarInt offset
```

### 5.13 ServerboundClientCommandPacket (`ServerboundClientCommandPacket.java:18-23, 40-44`)

```
VarInt action             // readEnum
```

Action ids: PERFORM_RESPAWN=0, REQUEST_STATS=1, REQUEST_GAMERULE_VALUES=2.

### 5.14 ServerboundPlayerCommandPacket (`ServerboundPlayerCommandPacket.java:27-37, 61-69`)

```
VarInt entityId
VarInt action             // readEnum
VarInt data
```

Action ids: STOP_SLEEPING=0, START_SPRINTING=1, STOP_SPRINTING=2, START_RIDING_JUMP=3, STOP_RIDING_JUMP=4, OPEN_INVENTORY=5, START_FALL_FLYING=6.

### 5.15 ServerboundPlayerInputPacket (`ServerboundPlayerInputPacket.java:10-12`, `world/entity/player/Input.java:14-40`)

```
u8 flags
```

Flags: 0x01 forward, 0x02 backward, 0x04 left, 0x08 right, 0x10 jump, 0x20 shift, 0x40 sprint.

### 5.16 ServerboundInteractPacket (`ServerboundInteractPacket.java:13-22`)

Right-click use on an entity.

```
VarInt entityId
VarInt hand               // InteractionHand
Vec3 location             // LpVec3 (section 1.6), NOT f64 x3
bool usingSecondaryAction // shift held (handler sets shiftKeyDown, ServerGamePacketListenerImpl.java:2936+)
```

### 5.17 ServerboundAttackPacket (`ServerboundAttackPacket.java:9-12`)

Left-click attack on an entity.

```
VarInt entityId
```

Handler `ServerGamePacketListenerImpl.handleAttack` (line 2894): looks up `getEntityOrPart(entityId)`, rejects self-target (kicks), checks world border and attack range, rejects ItemEntity/ExperienceOrb/self/non-attackable arrows (kicks), then `player.attack(target)`.

### 5.18 ServerboundPunchPacket (`ServerboundPunchPacket.java:8-10`)

Empty payload. `STREAM_CODEC = StreamCodec.unit(INSTANCE)`. It is the arm-swing/"punch" packet (left click that does not target an entity). Handler `handlePunch` (line 2729): resets last action time, runs a Bukkit ray trace (range = max block and entity interaction ranges) to fire the interact event for "rogue arm swings". Whether it broadcasts an animation to other clients was not confirmed in this pass (UNCONFIRMED).

### 5.19 ServerboundPlayerLoadedPacket (`ServerboundPlayerLoadedPacket.java:8`)

Empty payload. `StreamCodec.unit(...)`. Handler `handleAcceptPlayerLoad` (line 818) marks the client loaded (`hasClientLoaded()` gate used in attack/interact, lines 2894, 2936).

### 5.20 ServerboundClientTickEndPacket (`ServerboundClientTickEndPacket.java:8-9`)

Empty payload. `StreamCodec.unit(INSTANCE)`. Handler `handleClientTickEnd` (line 3788).

### 5.21 ServerboundPickItemFromBlockPacket (`ServerboundPickItemFromBlockPacket.java:10-17`)

```
i64 BlockPos
bool includeData
```

---

## 6. Enum and id reference

| Enum | Ids (wire) | Source |
|---|---|---|
| Direction (3D id, used by PlayerAction u8 and by VarInt paths) | DOWN 0, UP 1, NORTH 2, SOUTH 3, WEST 4, EAST 5 | Direction.java:33-38, 205 |
| InteractionHand | MAIN_HAND 0, OFF_HAND 1 | InteractionHand.java:5-18 |
| HumanoidArm | LEFT 0, RIGHT 1 | HumanoidArm.java:13-14 |
| GameType | SURVIVAL 0, CREATIVE 1, ADVENTURE 2, SPECTATOR 3 | GameType.java:19-22 |
| SoundSource | MASTER 0 ... UI 10 (section 1.13) | SoundSource.java:4-14 |
| Pose | STANDING 0 ... INHALING 17 (section 1.13) | Pose.java:12-29 |
| ContainerInput | PICKUP 0 ... PICKUP_ALL 6 | ContainerInput.java:10-16 |
| PlayerInfo Action (bit) | ADD_PLAYER 0 ... UPDATE_HAT 7 | ClientboundPlayerInfoUpdatePacket.java:168-202 |
| PlayerAction Action | START_DESTROY_BLOCK 0 ... STAB 8 | ServerboundPlayerActionPacket.java:71-79 |
| PlayerCommand Action | STOP_SLEEPING 0 ... START_FALL_FLYING 6 | ServerboundPlayerCommandPacket.java:62-68 |
| ClientCommand Action | PERFORM_RESPAWN 0, REQUEST_STATS 1, REQUEST_GAMERULE_VALUES 2 | ServerboundClientCommandPacket.java:41-43 |

---

## 7. UNCONFIRMED items (not verified in this pass)

- Component (NBT) body in `ComponentSerialization`/`ByteBufCodecs.fromCodecWithRegistries` (1.13). Assumed network NBT.
- `ByteBufCodecs.map` key/value ordering (used by ContainerClick changedSlots and HashedPatchMap added components).
- Per-type DataComponent value codecs (inside ItemStack patches, 1.11).
- Entity data serializers 13, 18, 21-35, 35-38, 39-41, 43 bodies (EntityReference, VillagerData, variant holders, sniffer/armadillo/copper/weathering enums, VECTOR3F, QUATERNIONF, ResolvableProfile, DyeColor). Only their registration ids are confirmed.
- PublicKey and signature byte-array bodies in ProfilePublicKey.Data (INITIALIZE_CHAT).
- ResourceKey.streamCodec body (dimension in CommonPlayerSpawnInfo / GlobalPos): assumed Identifier string.
- Brigadier StringType ordinal (SINGLE_WORD/QUOTABLE_PHRASE/GREEDY_PHRASE) follows the brigadier library enum order.
- Argument type ids 21-59 and 61: derived from line order in ArgumentTypeInfos.bootstrap; only the listed ones were spot-checked.
- Global ids (block state, item, entity type, sound event, particle type, registry numeric ids): depend on runtime registries, not fixed by source.
- ClientboundSwingAnimationPacket body.
- Whether ServerboundPunchPacket broadcasts an animation.
- ClientboundAddEntity ordering of the `data` VarInt semantic per entity type (only the wire layout is confirmed).
- Packet ID numbers (out of scope, given as known).
