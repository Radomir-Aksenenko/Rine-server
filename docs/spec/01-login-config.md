# 01. Handshake, Status, Login, Compression, Configuration (protocol 777, Paper 26.3)

Source base (unless noted): `E:\Projects\Rine-server\Paper-26.3\paper-server\src\minecraft\java\net\minecraft\` (abbreviated `MC/` below).
Paper-only files live under `E:\Projects\Rine-server\Paper-26.3\paper-server\src\main\java\` (abbreviated `PAPER/`).
Line numbers are from the files as they exist in the tree at the time of reading.

Every packet payload starts with a VarInt packet ID, written by `IdDispatchCodec` (`MC/network/codec/IdDispatchCodec.java:27,53`). The IDs themselves are not repeated here.

## 0. Primitive encodings

| Type | Wire format | Source |
|---|---|---|
| VarInt | 1-5 bytes, 7 bits per byte, LSB group first, MSB = continuation. Signed 32-bit semantics. | `MC/network/VarInt.java:37-66` |
| i8 / u8 | 1 byte (`writeByte` / `readUnsignedByte`) | `FriendlyByteBuf` / `ClientInformation.java:24,27` |
| i16 / u16 | 2 bytes big-endian (`writeShort` / `readUnsignedShort`) | `ClientIntentionPacket.java:19,25` |
| i32 | 4 bytes big-endian (`writeInt`) | `ClientboundPingPacket.java:19,23` |
| i64 | 8 bytes big-endian (`writeLong`) | `ServerboundPingRequestPacket.java:19,23` |
| bool | 1 byte, 0 or 1 (`writeBoolean`) | `ByteBufCodecs.java:63-73` |
| String(max) | VarInt byte length + UTF-8 bytes. Max is counted in characters after decode; byte length is bounded by `ByteBufUtil.utf8MaxBytes(max)`. Encoder throws if chars > max. | `MC/network/Utf8String.java:10-55` |
| `FriendlyByteBuf.readUtf()` default max | 32767 characters (`MAX_STRING_LENGTH`) | `MC/network/FriendlyByteBuf.java:60,549-564` |
| UUID | two i64 big-endian: most significant bits, then least significant bits | `MC/network/FriendlyByteBuf.java:416-426` |
| Identifier | String (`Identifier.STREAM_CODEC = STRING_UTF8.map(parse, toString)`, max 32767 chars). Written as `namespace:path`. | `MC/resources/Identifier.java:20` |
| Byte array | VarInt length + raw bytes (`readByteArray(maxSize)` checks length). Unbounded variant limited only by readable bytes. | `FriendlyByteBuf.java:196-223` |
| Optional<T> | bool (0 = absent, 1 = followed by T) | `ByteBufCodecs.java:512-528` |
| Collection / List / Map | VarInt count, then elements (map: key, value pairs). Count is rejected if > max. Default max = Integer.MAX_VALUE. | `ByteBufCodecs.java:531-545, 548-595, 637-...` |
| Enum via idMapper | VarInt id (not ordinal necessarily; see per-enum tables) | `ByteBufCodecs.java:735-749` |
| NBT (network) | Nameless root: 1 byte tag type id, then payload. No root name. TAG_End (0) = "absent". | `MC/nbt/NbtIo.java:162-165` (readAnyTag), `:258-260` (writeAnyTag = name null); `FriendlyByteBuf.java:444-500` |
| Component (login disconnect, status description) | JSON text as a String, via `ByteBufCodecs.lenientJson(max)` wrapping `ComponentSerialization.CODEC` | `MC/network/codec/ByteBufCodecs.java:855-876`, `MC/network/chat/ComponentSerialization.java` |
| Component (configuration disconnect) | NBT, via `ComponentSerialization.TRUSTED_CONTEXT_FREE_STREAM_CODEC = fromCodecTrusted(CODEC)` (nameless NBT, not JSON) | `ComponentSerialization.java:44`, `ClientboundDisconnectPacket.java:11` |

## 1. Handshake (state HANDSHAKING, serverbound)

### ClientIntentionPacket
`MC/network/protocol/handshake/ClientIntentionPacket.java:18-27`

| # | Field | Wire type | Notes |
|---|---|---|---|
| 1 | protocolVersion | VarInt | 777 for this build (`MC/SharedConstants.java:207-209`) |
| 2 | hostName | String(32767) | Read with `readUtf(Short.MAX_VALUE)` |
| 3 | port | u16 big-endian | `readUnsignedShort` |
| 4 | intention | VarInt | enum `ClientIntent`, see below |

`ClientIntent` (`MC/network/protocol/handshake/ClientIntent.java:4-27`):

| VarInt | Value |
|---|---|
| 1 | STATUS |
| 2 | LOGIN |
| 3 | TRANSFER |

Any other value throws `IllegalArgumentException`. Note that `isTerminal()` is true, so the handshake is the last packet in this state.

## 2. Status

### ServerboundStatusRequestPacket (status_request)
Empty body. `StreamCodec.unit(INSTANCE)` (`MC/network/protocol/status/ServerboundStatusRequestPacket.java:10`). A second request on the same connection causes a disconnect (`MC/server/network/ServerStatusPacketListenerImpl.java:34-38`).

### ClientboundStatusResponsePacket (status_response)
One field: `ServerStatus status`, written as a lenient JSON String with max 32767 characters (`ClientboundStatusResponsePacket.java:15-19`). Encoded with `JsonOps` over `ServerStatus.CODEC`.

Paper builds the actual response in `PAPER/main/java/com/destroystokyo/paper/network/StandardPaperServerListPingEventImpl.java:66-102` (event-driven, then `connection.send(new ClientboundStatusResponsePacket(ping))`). The field set below is the `ServerStatus` record, which is what gets serialized.

ServerStatus JSON (`MC/network/protocol/status/ServerStatus.java:17-33`). Top-level keys in this order. Optional keys are omitted when absent.

| Key | JSON type | Required | Content / default |
|---|---|---|---|
| `description` | Component (JSON object or string) | optional, default `""` (CommonComponents.EMPTY) | MOTD. `ServerStatus.java:26` |
| `players` | object | optional (omitted when players are hidden) | see below. `:27`, `:52-61` |
| `version` | object `{name, protocol}` | optional | `:28`, `:63-75` |
| `favicon` | String | optional | `"data:image/png;base64,<base64 PNG>"`, prefix must match exactly, newlines stripped on decode. `:35-50` |
| `enforcesSecureChat` | bool | optional, default `false` | `:30` |

`players`:
- `max` (int, required), `online` (int, required), `sample` (array, optional, default `[]`)
- each sample entry is `NameAndId` (`MC/server/players/NameAndId.java:12-15`): `{"id": "<UUID with dashes>", "name": "<string>"}`. The `id` uses `UUIDUtil.STRING_CODEC`, which encodes with `UUID.toString()` (dashed form) (`MC/core/UUIDUtil.java:27-33`).

`version`:
- `name` (String) = `"26.3"` (from `version.json` `name` / `id`, see section 7)
- `protocol` (int) = `777`

Paper's `ServerStatus.Version` is built from event values (`StandardPaperServerListPingEventImpl.java:90`). Default values for the event are UNCONFIRMED in this pass.

### ServerboundPingRequestPacket (ping_request)
`i64 time` (`MC/network/protocol/ping/ServerboundPingRequestPacket.java:19,23`).

### ClientboundPongResponsePacket (pong_response)
`i64 time`, echoed from the request (`MC/network/protocol/ping/ClientboundPongResponsePacket.java:14,18`). Status-state packets are not compressed.

## 3. Login (state LOGIN)

Serverbound packets: `MC/network/protocol/login/LoginProtocols.java:13-20`. Clientbound: `:22-30`.

### ServerboundHelloPacket (hello)
`MC/network/protocol/login/ServerboundHelloPacket.java:14-21`

| # | Field | Wire type | Notes |
|---|---|---|---|
| 1 | name | String(16) | player name, max 16 characters |
| 2 | profileId | UUID | client-provided UUID, two i64 |

### ClientboundHelloPacket (encryption request, `hello` clientbound)
`MC/network/protocol/login/ClientboundHelloPacket.java:27-38`

| # | Field | Wire type | Notes |
|---|---|---|---|
| 1 | serverId | String(20) | Server sends `""` (`MC/server/network/ServerLoginPacketListenerImpl.java:67,177`) |
| 2 | publicKey | byte array (VarInt len + bytes) | X.509 DER of the RSA public key (`Crypt.byteToPublicKey` uses `X509EncodedKeySpec`, `MC/util/Crypt.java:147-151`) |
| 3 | challenge | byte array | 4 bytes: `Ints.toByteArray(random int)` (`ServerLoginPacketListenerImpl.java:78`) |
| 4 | shouldAuthenticate | bool | `true` in this path |

Sent only when `server.usesAuthentication()` is true and the connection is not in-memory (`ServerLoginPacketListenerImpl.java:175-177`). Otherwise the server skips to profile creation (offline mode).

### ServerboundKeyPacket (key)
`MC/network/protocol/login/ServerboundKeyPacket.java:24-32`

| # | Field | Wire type | Notes |
|---|---|---|---|
| 1 | keybytes | byte array | RSA-encrypted 16-byte AES shared secret (`Crypt.encryptUsingKey(publicKey, secretKey.getEncoded())`, `:20`) |
| 2 | encryptedChallenge | byte array | RSA-encrypted challenge bytes (`:21`) |

RSA uses the JCE `"RSA"` transformation (`MC/util/Crypt.java:31,70,167-169`), which is PKCS#1 v1.5 padding by JCE default. After verifying the challenge, the server calls `connection.setEncryptionKey(secretKey)` (`ServerLoginPacketListenerImpl.java:246-259`). From then on the stream is encrypted with `AES/CFB8/NoPadding` (`MC/util/Crypt.java:191`). The Connection uses the Velocity native cipher wrapper; its equivalence to the JCE cipher is UNCONFIRMED in this pass.

Encryption does not change the frame layout in section 4.

### ClientboundLoginCompressionPacket (login_compression)
`MC/network/protocol/login/ClientboundLoginCompressionPacket.java:18-23`
- `VarInt compressionThreshold`

Sent only when `server.getCompressionThreshold() >= 0` (`ServerLoginPacketListenerImpl.java:218-227`). It is sent uncompressed, and compression is switched on in the same Netty task (section 4).

### ClientboundLoginFinishedPacket (login_finished)
`MC/network/protocol/login/ClientboundLoginFinishedPacket.java:13-18`

| # | Field | Wire type | Notes |
|---|---|---|---|
| 1 | gameProfile | GameProfile (see below) | `ByteBufCodecs.GAME_PROFILE` (`MC/network/codec/ByteBufCodecs.java:288-290`) |
| 2 | sessionId | UUID | `UUIDUtil.STREAM_CODEC`, two i64 |

GameProfile:

| # | Field | Wire type | Limit |
|---|---|---|---|
| 1 | id | UUID (two i64) | - |
| 2 | name | String | max 16 characters (`PLAYER_NAME = stringUtf8(16)`, `ByteBufCodecs.java:287`) |
| 3 | properties | VarInt count, then entries | count max 16 (`readCount(input, 16)`, `:262`, `:278`) |

Each property entry (`ByteBufCodecs.java:259-286`):

| # | Field | Wire type | Limit |
|---|---|---|---|
| 1 | name | String | max 64 characters |
| 2 | value | String | max 32767 characters |
| 3 | signature | Optional String (bool + String) | max 1024 characters when present |

The signature is written via `FriendlyByteBuf.writeNullable` (`:283`), which is a bool prefix followed by the string.

### ServerboundLoginAcknowledgedPacket (login_acknowledged)
Empty body (`MC/network/protocol/login/ServerboundLoginAcknowledgedPacket.java:10`). On receipt the server switches outbound to `ConfigurationProtocols.CLIENTBOUND`, inbound to `SERVERBOUND`, and starts configuration (`MC/server/network/ServerLoginPacketListenerImpl.java:430-439`).

### ClientboundLoginDisconnectPacket (login_disconnect)
`MC/network/protocol/login/ClientboundLoginDisconnectPacket.java:17-21`
- `Component reason`, encoded as a JSON String with max 262144 characters (`lenientJson(262144)` over `ComponentSerialization.CODEC`). This is JSON text, not NBT.

Other login packets (`ClientboundCustomQueryPacket`, `ServerboundCustomQueryAnswerPacket`, cookie request/response) are registered in `LoginProtocols.java` but their layouts are not documented here.

## 4. Framing and compression

### Pipeline order
Outbound: `Packet` → `PacketEncoder` (writes VarInt packet ID + body, `MC/network/PacketEncoder.java:22-57`) → `CompressionEncoder` (only when compression is on) → `Varint21LengthFieldPrepender` (writes frame length VarInt, `MC/network/Varint21LengthFieldPrepender.java:13-25`).

Inbound: socket → `Varint21FrameDecoder` (`MC/network/Varint21FrameDecoder.java:11-70`) → `CompressionDecoder` (only when compression is on) → `PacketDecoder` (`MC/network/PacketDecoder.java:23-67`).

The compression handlers are inserted by `Connection.setupCompression` (`MC/network/Connection.java:848-871`). `DECOMPRESS` is added after `SPLITTER` and `COMPRESS` after `PREPENDER`, so compression sits between the packet codec and the length prefix in both directions. This ordering is derived from the `addAfter` calls and not traced at runtime.

### Frame layouts
- Compression off (before `login_compression` is applied, and always during status):
  `VarInt frameLength | VarInt packetId | body`
- Compression on:
  `VarInt frameLength | VarInt dataLength | payload`
  - `frameLength` counts everything after it (the `dataLength` VarInt plus payload bytes).
  - `dataLength = 0`: `payload` is raw `VarInt packetId | body`. Used when the uncompressed size is below the threshold.
  - `dataLength > 0`: `dataLength` is the uncompressed size of `VarInt packetId | body`, and `payload` is the zlib-compressed bytes. `Deflater()` with default settings produces zlib (RFC 1950) format with a header, not raw deflate (`CompressionEncoder.java:20-22,42-54`).
  - Once compression is enabled, every frame in that direction carries the `dataLength` VarInt, including the ones that are not compressed (`CompressionEncoder.java:38-40`).

### Threshold semantics
- Encoder (`CompressionEncoder.java:33-66`): if the uncompressed length (packet ID + body) is `< threshold`, write `VarInt 0` and the raw bytes. Otherwise write the uncompressed length and the zlib data.
- Threshold `< 0` means compression disabled (`Connection.java:848-871`).
- Default threshold: `network-compression-threshold = 256` (`MC/server/dedicated/DedicatedServerProperties.java:108`). `DedicatedServer.getCompressionThreshold()` returns it (`MC/server/dedicated/DedicatedServer.java:761-763`). The base `MinecraftServer` returns 256 (`MC/server/MinecraftServer.java:2241-2243`).
- Decoder (`CompressionDecoder.java:34-72`): `dataLength == 0` means the rest is raw. Otherwise, with validation enabled (server path passes `validateDecompressed = true`, `ServerLoginPacketListenerImpl.java:224`), it rejects `dataLength < threshold` and `dataLength > 8388608`. It then inflates and requires the inflated size to equal `dataLength` exactly.

### Enable timing
The server sends `ClientboundLoginCompressionPacket` uncompressed, then calls `setupCompression` in the same Netty task (`ServerLoginPacketListenerImpl.java:218-227`). The `login_finished` packet is the first one sent after compression is on. The client must do the same.

### Limits
- Frame length prefix: at most 3 bytes (21 bits), so a frame body is at most `2097151` bytes (`Varint21FrameDecoder.java:12,25-39`). A zero-length frame is a corrupt frame (`:55-57`).
- `PacketEncoder`: uncompressed packet size max `8388608` (`PacketEncoder.java:63`), and the final encoded size max `2097151` = `(1 << 21) - 1` (`:61`, `:51-52`), with an exception for packets that define a large-packet fallback.
- `CompressionEncoder`: throws if the uncompressed length is above `8388608` (`CompressionEncoder.java:34-36`).
- `CompressionDecoder.MAXIMUM_COMPRESSED_LENGTH = 2097152` is declared (`:13`) but not used in the decode path shown here.

## 5. Configuration (state CONFIGURATION)

Protocol registration: `MC/network/protocol/configuration/ConfigurationProtocols.java:33-72`.

### ClientboundSelectKnownPacks (select_known_packs)
`MC/network/protocol/configuration/ClientboundSelectKnownPacks.java:11-14`
- `List<KnownPack>`: VarInt count (no max, `ByteBufCodecs.list()`), then each `KnownPack`.

### ServerboundSelectKnownPacks (select_known_packs response)
`MC/network/protocol/configuration/ServerboundSelectKnownPacks.java:11-14`
- `List<KnownPack>`: VarInt count with max 64 (`ByteBufCodecs.list(64)`), then each `KnownPack`.

### KnownPack
`MC/server/packs/repository/KnownPack.java:8-17`
- `String namespace`, `String id`, `String version`, each `STRING_UTF8` (max 32767 characters).

Vanilla core pack: `KnownPack.vanilla("core")` → `namespace = "minecraft"`, `id = "core"`, `version = SharedConstants.getCurrentVersion().id()` (`KnownPack.java:20-22`). `BuiltInPackSource.CORE_PACK_INFO = KnownPack.vanilla("core")` (`MC/server/packs/repository/BuiltInPackSource.java:25`). The version string is `"26.3"`, see section 7.

Feature packs use `KnownPack.vanilla(id)` as well (`MC/server/packs/repository/ServerPacksSource.java:43`). Their concrete ids are UNCONFIRMED in this pass.

Which packs are offered: `ServerConfigurationPacketListenerImpl.java:114-119` takes `resourceManager.listPacks()` in list order and keeps those with `knownPackInfo()` present. Datapacks that have no known-pack info are not offered.

Negotiation (`MC/server/network/config/SynchronizeRegistriesTask.java:46-52`): the server compares the client's accepted list to the requested list with `List.equals`, which is order-sensitive and requires the same entries. If equal, `negotiatedPacks = Set.copyOf(requested)`. Otherwise `negotiatedPacks = Set.of()`, and every registry entry is sent with full data.

### ClientboundRegistryDataPacket (registry_data)
`MC/network/protocol/configuration/ClientboundRegistryDataPacket.java:17-25`

| # | Field | Wire type | Notes |
|---|---|---|---|
| 1 | registry | Identifier (String) | e.g. `minecraft:worldgen/biome` |
| 2 | entries | VarInt count, then `PackedRegistryEntry` | no max |

PackedRegistryEntry (`MC/core/RegistrySynchronization.java:91-98`):

| # | Field | Wire type | Notes |
|---|---|---|---|
| 1 | id | Identifier (String) | entry key, `namespace:path` |
| 2 | data | Optional NBT: bool, then nameless NBT tag | absent (`false`) when the entry is covered by a negotiated known pack |

Skip rule (`MC/core/RegistrySynchronization.java:51-58`): an entry is sent without data when its `RegistrationInfo.knownPackInfo` is present and contained in the negotiated set. Otherwise the element is encoded with `elementCodec.encodeStart(NbtOps, value)` (`:59-62`) and sent as a nameless NBT tag.

The element codec used is the one listed in `RegistryDataLoader.SYNCHRONIZED_REGISTRIES` (section 6). Element schemas themselves are out of scope here.

### ClientboundUpdateTagsPacket (update_tags)
`MC/network/protocol/common/ClientboundUpdateTagsPacket.java:16-20`
- `Map<registryKey, NetworkPayload>` via `ByteBufCodecs.map(...)`: VarInt count, then for each entry:
  - registry key: Identifier (String), `ResourceKey.REGISTRY_STREAM_CODEC`
  - NetworkPayload: `Map<tagLocation, IntList>` (`MC/tags/TagNetworkSerialization.java:61-68`): VarInt count, then for each tag:
    - tag location: Identifier (String), without the `#` prefix
    - ids: VarInt count, then VarInt registry ids (`ID_LIST_STREAM_CODEC = VAR_INT.apply(collection(IntArrayList::new))`, `:63`)

Which registries are included (`MC/tags/TagNetworkSerialization.java:23-30`, `MC/core/RegistrySynchronization.java:73-85`):
- `networkSafeRegistries` = world-layer registries whose key is in `SYNCHRONIZED_REGISTRIES` (the networkable set) + every static-layer registry (`registries.getLayer(STATIC).registries()`).
- Registries with no tags are dropped (`.filter(e -> !e.getSecond().isEmpty())`).
- Every holder must be a registered reference (`REFERENCE`), otherwise it throws (`:37-41`).
- So the packet covers all static registries that have tags (blocks, items, and so on) plus the synchronized world registries that have tags. Non-synchronized world registries are not sent.

Tag ids are the registry ids of the entries in each registry (`registry.getId(holder.value())`, `:42`).

### ClientboundUpdateEnabledFeaturesPacket (update_enabled_features)
`MC/network/protocol/configuration/ClientboundUpdateEnabledFeaturesPacket.java:13-17`
- `Set<Identifier>` as a collection: VarInt count (no max), then Identifier (String) elements. Source: `FeatureFlags.REGISTRY.toNames(enabledFeatures)` (`ServerConfigurationPacketListenerImpl.java:120`). The concrete default names for a vanilla world are UNCONFIRMED in this pass.

### ClientboundFinishConfigurationPacket (finish_configuration, clientbound)
Empty body (`ClientboundFinishConfigurationPacket.java:8-27`). Sent by `JoinWorldTask.start` (`MC/server/network/config/JoinWorldTask.java:12-13`).

### ServerboundFinishConfigurationPacket (finish_configuration, serverbound)
Empty body (`ServerboundFinishConfigurationPacket.java:8-27`). On receipt the server sets outbound to the play protocol and proceeds (`MC/server/network/ServerConfigurationPacketListenerImpl.java:196-199`).

### ServerboundClientInformationPacket (client_information)
`MC/network/protocol/common/ServerboundClientInformationPacket.java:14-20` → `ClientInformation` (`MC/server/level/ClientInformation.java:21-45`)

| # | Field | Wire type | Notes |
|---|---|---|---|
| 1 | language | String(16) | e.g. `en_us` |
| 2 | viewDistance | i8 | `readByte` / `writeByte` (signed) |
| 3 | chatVisibility | VarInt id | `ChatVisiblity` (`MC/world/entity/player/ChatVisiblity.java:12-18`): 0 FULL, 1 SYSTEM, 2 HIDDEN. Out-of-range wraps. |
| 4 | chatColors | bool | |
| 5 | modelCustomisation | u8 | skin-part bitmask (`readUnsignedByte` / `writeByte`) |
| 6 | mainHand | VarInt id | `HumanoidArm` (`MC/world/entity/HumanoidArm.java:13-18`): 0 LEFT, 1 RIGHT. Out-of-range → 0. |
| 7 | textFilteringEnabled | bool | |
| 8 | allowsListing | bool | |
| 9 | particleStatus | VarInt id | `ParticleStatus` (`MC/server/level/ParticleStatus.java:12-18`): 0 ALL, 1 DECREASED, 2 MINIMAL. Out-of-range wraps. |

### Keep alive
- `ClientboundKeepAlivePacket` (keep_alive, clientbound): `i64 id` (`MC/network/protocol/common/ClientboundKeepAlivePacket.java:19`).
- `ServerboundKeepAlivePacket` (keep_alive, serverbound): `i64 id` (`MC/network/protocol/common/ServerboundKeepAlivePacket.java:19`).
- Server sends these from `keepConnectionAlive` (`MC/server/network/ServerCommonPacketListenerImpl.java:256-268`), driven from `ServerConfigurationPacketListenerImpl.tick` (`:222-224`).

Ping and pong (common, not keep-alive):
- `ClientboundPingPacket` (ping): `i32 id` (`MC/network/protocol/common/ClientboundPingPacket.java:19,23`).
- `ServerboundPongPacket` (pong): `i32 id` (`MC/network/protocol/common/ServerboundPongPacket.java:19,23`).
- Which code path sends the clientbound ping during configuration is UNCONFIRMED in this pass.

### custom_payload
Identifier, then the payload bytes. `CustomPacketPayload.codec` writes `Identifier` (`MC/network/protocol/common/custom/CustomPacketPayload.java:24-52`) then calls the matching payload codec.

- Clientbound (configuration): `ClientboundCustomPayloadPacket.CONFIG_STREAM_CODEC` (`MC/network/protocol/common/ClientboundCustomPayloadPacket.java:22-25`). Unknown channels use `DiscardedPayload` with max 1048576 bytes.
- Serverbound: `ServerboundCustomPayloadPacket.STREAM_CODEC` (`MC/network/protocol/common/ServerboundCustomPayloadPacket.java:15-19`). Unknown channels use `DiscardedPayload` with max 32767 bytes.
- `DiscardedPayload` (`MC/network/protocol/common/custom/DiscardedPayload.java:8-23`): raw bytes with **no length prefix**, filling the rest of the packet.

Brand (`minecraft:brand`):
- Identifier `minecraft:brand` (`BrandPayload.TYPE = createType("brand")` → `Identifier.withDefaultNamespace`, `MC/network/protocol/common/custom/BrandPayload.java:8`).
- Payload is `writeUtf(brand)`: VarInt byte length + UTF-8, max 32767 characters (`BrandPayload.java:14-16`).
- Server sends it first in configuration: `new BrandPayload(server.getServerModName())` (`ServerConfigurationPacketListenerImpl.java:104`). `getServerModName()` returns Paper's `ServerBuildInfo.buildInfo().brandName()` (`MC/server/MinecraftServer.java:1934-1936`). The literal brand string is UNCONFIRMED in this pass (it comes from Paper's build info).

### ClientboundCodeOfConductPacket (code_of_conduct)
`MC/network/protocol/configuration/ClientboundCodeOfConductPacket.java:10-12`
- `String codeOfConduct` via `STRING_UTF8` (max 32767 characters).

### ServerboundAcceptCodeOfConductPacket (accept_code_of_conduct)
Empty body (`ServerboundAcceptCodeOfConductPacket.java:9-10`).

## 6. SYNCHRONIZED_REGISTRIES (order sent)

Source: `MC/resources/RegistryDataLoader.java:164-196`. Packets are emitted in this list order by `RegistrySynchronization.packRegistries` (`MC/core/RegistrySynchronization.java:33-34`). A registry is skipped when it is not present in the world access (`:44-45`). Identifiers are from `MC/core/registries/Registries.java`. The `DIRECT` / `NETWORK` column is the element codec used for the NBT payload.

| # | Registry identifier | Element codec |
|---|---|---|
| 1 | `minecraft:worldgen/biome` | NETWORK (`Biome.NETWORK_CODEC`) |
| 2 | `minecraft:chat_type` | DIRECT |
| 3 | `minecraft:trim_pattern` | DIRECT |
| 4 | `minecraft:trim_material` | DIRECT |
| 5 | `minecraft:wolf_variant` | NETWORK |
| 6 | `minecraft:wolf_sound_variant` | NETWORK |
| 7 | `minecraft:pig_variant` | NETWORK |
| 8 | `minecraft:pig_sound_variant` | NETWORK |
| 9 | `minecraft:frog_variant` | NETWORK |
| 10 | `minecraft:cat_variant` | NETWORK |
| 11 | `minecraft:cat_sound_variant` | NETWORK |
| 12 | `minecraft:cow_sound_variant` | DIRECT |
| 13 | `minecraft:cow_variant` | NETWORK |
| 14 | `minecraft:chicken_sound_variant` | DIRECT |
| 15 | `minecraft:chicken_variant` | NETWORK |
| 16 | `minecraft:zombie_nautilus_variant` | NETWORK |
| 17 | `minecraft:painting_variant` | DIRECT |
| 18 | `minecraft:sulfur_cube_archetype` | DIRECT |
| 19 | `minecraft:dimension_type` | NETWORK (`DimensionType.NETWORK_CODEC`) |
| 20 | `minecraft:damage_type` | DIRECT |
| 21 | `minecraft:banner_pattern` | DIRECT |
| 22 | `minecraft:enchantment` | DIRECT |
| 23 | `minecraft:jukebox_song` | DIRECT |
| 24 | `minecraft:instrument` | DIRECT |
| 25 | `minecraft:test_environment` | DIRECT |
| 26 | `minecraft:test_instance` | DIRECT |
| 27 | `minecraft:dialog` | DIRECT |
| 28 | `minecraft:world_clock` | DIRECT |
| 29 | `minecraft:timeline` | NETWORK (`Timeline.NETWORK_CODEC`) |
| 30 | `minecraft:decorated_pot_pattern` | DIRECT (`CODEC`) |
| 31 | `minecraft:block_transformer` | DIRECT |
| 32 | `minecraft:worldgen/block_state_provider` | DIRECT |

Notes:
- `NETWORK` entries (1, 5-11, 13, 15, 16, 19, 29) use a `NETWORK_CODEC` variant. Entry 30 uses plain `CODEC`. All others use `DIRECT_CODEC`. Lines 165-196 of `RegistryDataLoader.java` are the authority.
- Entries in `WORLD_REGISTRIES`, `DIMENSION_REGISTRIES`, and `RELOADABLE_REGISTRIES` (`RegistryDataLoader.java:98-162`) are not sent in configuration unless also in this list.
- Registries with `RegistryValidator.nonEmpty()` are validated on load only, which does not change the wire format.

## 7. Version string and KnownPack core

- Version JSON: `E:\Projects\Rine-server\Paper-26.3\paper-server\src\minecraft\resources\version.json`. Relevant keys: `"id": "26.3"`, `"name": "26.3"`, `"protocol_version": 777`, `"world_version": 5023`.
- `DetectedVersion.tryDetectVersion()` reads `/version.json` (`MC/DetectedVersion.java:46-59`) and builds the `WorldVersion` with `id` and `protocol_version` (`:32-44`).
- `SharedConstants.getCurrentVersion()` returns that `WorldVersion` (`MC/SharedConstants.java:199-204`). `SharedConstants.getProtocolVersion()` is hard-coded to `777` (`:207-209`).
- `KnownPack.vanilla(id)` uses `SharedConstants.getCurrentVersion().id()` (`MC/server/packs/repository/KnownPack.java:21`), so the core pack version is **`"26.3"`**.
- Caveat: if `version.json` is missing, `DetectedVersion.BUILT_IN` is used and its `id` is a random UUID (`DetectedVersion.java:20,47-51`). A missing file would make the core KnownPack version mismatch the client.

## 8. Configuration task order

From `ServerConfigurationPacketListenerImpl.startConfiguration()` (`MC/server/network/ServerConfigurationPacketListenerImpl.java:102-126`) and `returnToWorld()` (`:128-133`):

| Order | Step | What is sent | Waits for |
|---|---|---|---|
| 1 | immediate | `custom_payload` `minecraft:brand` (`:104`) | - |
| 2 | immediate, only if links exist | `server_links` (`:105-113`) | - |
| 3 | immediate | `update_enabled_features` (`:120`) | - |
| 4 | SynchronizeRegistriesTask (`:121-122`) | `select_known_packs` at start (`SynchronizeRegistriesTask.java:31-33`). After the response: `registry_data` per registry in section 6 order, then `update_tags` (`:35-44`). | serverbound `select_known_packs` (`ServerConfigurationPacketListenerImpl.java:180-188`) |
| 5a | optional: ServerCodeOfConductConfigurationTask (`:135-158`) | `code_of_conduct` if a CoC text resolves for the client's language (falls back to `en_us`, then any). Skipped when the Paper event returns null. Added only when a CoC is configured or a Paper CoC event listener exists. | `accept_code_of_conduct` (`:190-192`) |
| 5b | optional: ServerResourcePackConfigurationTask (`:161`) | `resource_pack_push` (`ServerResourcePackConfigurationTask.java:19-`). Added only when a server resource pack is configured. | serverbound `resource_pack` with a terminal action (`:170-177`) |
| 6 | PaperConfigurationTask (`:124`) | No packets. Runs Paper's async configure event if listeners exist, else completes immediately (`PAPER/main/java/io/papermc/paper/connection/PaperConfigurationTask.java:30-40`). | - |
| 7 | PrepareSpawnTask (`:129-130`) | No packets in `start` (`MC/server/network/config/PrepareSpawnTask.java:54-`). Loads player data and spawn chunks. | internal completion |
| 8 | JoinWorldTask (`:131`) | `finish_configuration` (`JoinWorldTask.java:12-13`) | serverbound `finish_configuration` (`:196-199`), then the connection switches to the play protocol |

Keep-alive packets are sent throughout via `tick` (`:222-224`).

## 9. Unconfirmed items

- Literal brand string from `ServerBuildInfo.brandName()` (Paper build info).
- Concrete `FeatureFlags` names sent in `update_enabled_features` for a default world.
- Concrete ids of feature packs in `ServerPacksSource`.
- Which code sends the clientbound `ping` packet during configuration.
- Paper status event defaults for `version` and `favicon` (`StandardPaperServerListPingEventImpl.java:90-96`).
- Equivalence of the Velocity native cipher and compressor to the JCE `AES/CFB8` and `Deflater` (zlib) behaviour, not checked against the natives.
- Layouts of login custom query, login cookie packets, and the config resource-pack packets (not requested, not documented here).
- Element schemas inside `registry_data` entries (not requested).
