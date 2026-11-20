import java.lang.reflect.Proxy;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.function.Predicate;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.Holder;
import net.minecraft.core.HolderSet;
import net.minecraft.core.Registry;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.data.registries.VanillaRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.nbt.NbtIo;
import net.minecraft.nbt.NbtAccounter;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.tags.TagLoader;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.WorldGenLevel;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.LevelHeightAccessor;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.Biomes;
import net.minecraft.world.level.chunk.PalettedContainerFactory;
import net.minecraft.world.level.chunk.ProtoChunk;
import net.minecraft.world.level.chunk.Strategy;
import net.minecraft.world.level.chunk.UpgradeData;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.EntityBlock;
import net.minecraft.world.level.block.Mirror;
import net.minecraft.world.level.block.Rotation;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.levelgen.WorldgenRandom;
import net.minecraft.world.level.levelgen.XoroshiroRandomSource;
import net.minecraft.world.level.levelgen.feature.Feature;
import net.minecraft.world.level.levelgen.feature.FossilFeature;
import net.minecraft.world.level.levelgen.feature.OverlayFeature;
import net.minecraft.world.level.levelgen.feature.RootSystemFeature;
import net.minecraft.world.level.levelgen.feature.SequenceFeature;
import net.minecraft.world.level.levelgen.feature.TemplateFeature;
import net.minecraft.world.level.levelgen.feature.WeightedRandomSelectorFeature;
import net.minecraft.world.level.levelgen.placement.PlacedFeature;
import net.minecraft.world.level.levelgen.structure.BoundingBox;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructurePlaceSettings;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplate;
import net.minecraft.world.level.levelgen.presets.WorldPresets;
import net.minecraft.world.level.material.Fluids;

/** Feature write and RNG fixtures. Run against the vanilla 26.3 server and libraries. */
public class FeatureSamples {
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        try (var packs = new MultiPackResourceManager(PackType.SERVER_DATA, List.of(ServerPacksSource.createVanillaPackSource().fullResources()))) {
            var builtins = RegistryLayer.createRegistryAccess().getLayer(RegistryLayer.STATIC);
            TagLoader.loadTagsForExistingRegistries(packs, builtins).forEach(Registry.PendingTags::apply);
        }
        var lookup = VanillaRegistries.createWorldLookup();
        var registry = lookup.lookupOrThrow(Registries.FEATURE);
        var generator = lookup.lookupOrThrow(Registries.WORLD_PRESET).getOrThrow(WorldPresets.NORMAL)
                .value().createWorldDimensions().overworld();
        Map<String, String> cases = new LinkedHashMap<>();
        cases.put("flower_plain", "grass");
        cases.put("flower_flower_forest", "grass");
        cases.put("flower_meadow", "grass");
        cases.put("moss_patch_bonemeal", "moss");
        cases.put("pale_moss_patch", "pale");
        cases.put("moss_patch_ceiling", "ceiling");
        cases.put("clay_pool_with_dripleaves", "pool");
        cases.put("nylium_bonemeal", "warped");
        cases.put("crimson_fungus_planted", "crimson");
        cases.put("warped_fungus_planted", "warped");
        cases.put("crimson_fungus", "crimson");
        cases.put("warped_fungus", "warped");
        cases.put("twisting_vines", "warped");
        cases.put("weeping_vines", "ceiling");
        cases.put("pointed_dripstone", "cave");
        cases.put("sulfur_spike", "cave");
        cases.put("dripstone_cluster", "cave");
        cases.put("sulfur_spike_cluster", "cave");
        cases.put("large_dripstone", "cave");
        cases.put("rooted_azalea_tree", "root");
        cases.put("rooted_sulfur_spring", "root");
        cases.put("monster_room", "room");
        cases.put("sculk_patch_deep_dark", "cave");
        cases.put("desert_well", "solid");
        cases.put("fossil_coal", "solid");
        cases.put("fossil_diamonds", "solid");
        cases.put("super_birch_bees", "grass");
        var plains = lookup.lookupOrThrow(Registries.BIOME).getOrThrow(Biomes.PLAINS);
        var biomeIds = net.minecraft.util.CrudeIncrementalIntIdentityHashBiMap.<net.minecraft.core.Holder<Biome>>create(1);
        biomeIds.add(plains);
        var containers = new PalettedContainerFactory(Strategy.createForBlockStates(Block.BLOCK_STATE_REGISTRY),
                Blocks.AIR.defaultBlockState(), null, Strategy.createForBiomes(biomeIds), plains, null);
        StringBuilder out = new StringBuilder();
        for (var entry : cases.entrySet()) {
            var feature = registry.getOrThrow(ResourceKey.create(Registries.FEATURE, Identifier.withDefaultNamespace(entry.getKey()))).value();
            if (entry.getKey().equals("desert_well") || entry.getKey().equals("rooted_sulfur_spring")) {
                feature = withoutTemplateManager(feature);
            }
            for (int seed = 0; seed < 16; seed++) {
                BlockPos origin = new BlockPos(seed * 13 - 80, 64, -47 + seed * 3);
                WorldgenRandom rng = new WorldgenRandom(new XoroshiroRandomSource(seed));
                Area area = new Area(entry.getValue(), origin, new XoroshiroRandomSource(seed + 100));
                if (entry.getKey().startsWith("sculk_")) {
                    area.postProcess = new ProtoChunk(ChunkPos.containing(origin), UpgradeData.EMPTY,
                            LevelHeightAccessor.create(-64, 384), containers, null);
                }
                boolean placed = feature instanceof FossilFeature fossil
                        ? placeFossil(fossil, area.level(), rng, origin)
                        : feature.place(area.level(), generator, rng, origin);
                out.append("%s %s %d %b %s %d".formatted(entry.getKey(), entry.getValue(), seed, placed,
                        Long.toUnsignedString(area.hash, 16), rng.nextInt()));
                if (!area.blockEntities.isEmpty()) out.append(' ').append(Long.toUnsignedString(area.entityHash(lookup), 16));
                out.append('\n');
            }
        }
        Files.writeString(Path.of(args[0]), out);
    }

    private static StructureTemplate template(Identifier id) {
        try (var input = FeatureSamples.class.getResourceAsStream("/data/" + id.getNamespace() + "/structure/" + id.getPath() + ".nbt")) {
            var result = new StructureTemplate();
            result.load(BuiltInRegistries.BLOCK, NbtIo.readCompressed(input, NbtAccounter.unlimitedHeap()));
            return result;
        } catch (java.io.IOException e) {
            throw new java.io.UncheckedIOException(e);
        }
    }

    /** The fixture supplies templates from the jar; normal placement gets them from the server. */
    private static Feature withoutTemplateManager(Feature feature) {
        return switch (feature) {
            case OverlayFeature overlay -> new OverlayFeature(wrap(overlay.features()));
            case SequenceFeature sequence -> new SequenceFeature(wrap(sequence.features()));
            case WeightedRandomSelectorFeature weighted -> new WeightedRandomSelectorFeature(weighted.features().map(FeatureSamples::wrap));
            case RootSystemFeature root -> new RootSystemFeature(wrap(root.treeFeature()), root.requiredVerticalSpaceForTree(),
                    root.levelTestDistance(), root.maxLevelDeviation(), root.rootRadius(), root.rootReplaceable(),
                    root.rootStateProvider(), root.rootPlacementAttempts(), root.rootColumnMaxHeight(), root.hangingRootRadius(),
                    root.hangingRootsVerticalSpan(), root.hangingRootStateProvider(), root.hangingRootPlacementAttempts(),
                    root.allowedVerticalWaterForTree(), root.allowedTreePosition());
            case TemplateFeature original -> new Feature() {
                public com.mojang.serialization.MapCodec<? extends Feature> codec() { return original.codec(); }
                public boolean place(WorldGenLevel level, net.minecraft.world.level.chunk.ChunkGenerator generator,
                        RandomSource rng, BlockPos origin) {
                    var entry = original.templates().getRandomOrThrow(rng);
                    var rot = net.minecraft.util.Util.getRandom(entry.rotations(), rng);
                    var structure = template(entry.template());
                    var size = structure.getSize();
                    var pos = origin.offset(rot.rotate(Direction.WEST).getUnitVec3i().multiply(size.getX() / 2))
                            .offset(rot.rotate(Direction.NORTH).getUnitVec3i().multiply(size.getZ() / 2));
                    var settings = new StructurePlaceSettings().setRotation(rot).setRandom(rng);
                    original.processors().ifPresent(list -> list.value().list().forEach(settings::addProcessor));
                    return structure.placeInWorld(level, pos, pos, settings, rng, Block.UPDATE_ALL);
                }
            };
            default -> feature;
        };
    }

    private static HolderSet<PlacedFeature> wrap(HolderSet<PlacedFeature> features) {
        return HolderSet.direct(features.stream().map(FeatureSamples::wrap).toList());
    }

    private static Holder<PlacedFeature> wrap(Holder<PlacedFeature> holder) {
        var placed = holder.value();
        return Holder.direct(new PlacedFeature(Holder.direct(withoutTemplateManager(placed.feature().value())), placed.placement()));
    }

    private static boolean placeFossil(FossilFeature feature, WorldGenLevel level, WorldgenRandom rng, BlockPos origin) {
        var rot = Rotation.getRandom(rng);
        int index = rng.nextInt(feature.fossilStructures().size());
        var bones = template(feature.fossilStructures().get(index));
        var ore = template(feature.overlayStructures().get(index));
        var chunk = ChunkPos.containing(origin);
        var window = new BoundingBox(chunk.getMinBlockX() - 16, level.getMinY(), chunk.getMinBlockZ() - 16,
                chunk.getMaxBlockX() + 16, level.getMaxY(), chunk.getMaxBlockZ() + 16);
        var settings = new StructurePlaceSettings().setRotation(rot).setBoundingBox(window).setRandom(rng);
        var size = bones.getSize(rot);
        var corner = origin.offset(-size.getX() / 2, 0, -size.getZ() / 2);
        // The solid fixture's ocean-floor height is above the input Y in every column.
        int bottom = Math.max(origin.getY() - 15 - rng.nextInt(10), level.getMinY() + 10);
        var pos = bones.getZeroPositionWithTransform(corner.atY(bottom), Mirror.NONE, rot);
        feature.fossilProcessors().value().list().forEach(settings::addProcessor);
        bones.placeInWorld(level, pos, pos, settings, rng, Block.UPDATE_NONE);
        settings.clearProcessors();
        feature.overlayProcessors().value().list().forEach(settings::addProcessor);
        ore.placeInWorld(level, pos, pos, settings, rng, Block.UPDATE_NONE);
        return true;
    }

    private static class Area {
        private final Map<BlockPos, BlockState> cells = new HashMap<>();
        private final Map<BlockPos, BlockEntity> blockEntities = new HashMap<>();
        private final String scenario;
        private final BlockPos origin;
        private final RandomSource random;
        private long hash = 0xcbf29ce484222325L;
        private ProtoChunk postProcess;

        Area(String scenario, BlockPos origin, RandomSource random) {
            this.scenario = scenario;
            this.origin = origin;
            this.random = random;
        }

        private BlockState get(BlockPos p) {
            BlockState placed = cells.get(p);
            if (placed != null) return placed;
            int x = p.getX() - origin.getX();
            int y = p.getY() - origin.getY();
            int z = p.getZ() - origin.getZ();
            if (scenario.equals("solid")) return Blocks.STONE.defaultBlockState();
            if (scenario.equals("cave")) {
                int ceiling = ((origin.getX() + 80) / 13 & 1) == 0 ? 8 : 20;
                return y < 0 || y >= ceiling && y <= 36 ? Blocks.STONE.defaultBlockState() : Blocks.AIR.defaultBlockState();
            }
            if (scenario.equals("root")) {
                if (y == 4) return Blocks.DIRT.defaultBlockState();
                return y < 0 || y >= 1 && y <= 3 ? Blocks.STONE.defaultBlockState() : Blocks.AIR.defaultBlockState();
            }
            if (scenario.equals("room")) return x == 0 && (z == 3 || z == 4) && (y == 0 || y == 1)
                    ? Blocks.AIR.defaultBlockState() : Blocks.STONE.defaultBlockState();
            if (scenario.equals("ceiling")) return y >= 1 ? Blocks.STONE.defaultBlockState() : Blocks.AIR.defaultBlockState();
            if (scenario.equals("pale") && x == -1 && z == 0 && y >= 0 && y <= 3) return Blocks.STONE.defaultBlockState();
            if (scenario.equals("moss") && x == 1 && z == 0 && y == -1) return Blocks.BEDROCK.defaultBlockState();
            if (scenario.equals("pool") && y == -1 && (x == 1 || z == -1)) return Blocks.BEDROCK.defaultBlockState();
            if (scenario.equals("crimson") && x == 0 && z == 0 && y == 1) return Blocks.POPPY.defaultBlockState();
            if (y >= 0) return Blocks.AIR.defaultBlockState();
            return switch (scenario) {
                case "grass" -> Blocks.GRASS_BLOCK.defaultBlockState();
                case "crimson" -> Blocks.CRIMSON_NYLIUM.defaultBlockState();
                case "warped" -> Blocks.WARPED_NYLIUM.defaultBlockState();
                default -> Blocks.STONE.defaultBlockState();
            };
        }

        private boolean put(BlockPos p, BlockState state) {
            if (p.getY() < -64 || p.getY() >= 320) return false;
            for (int value : new int[]{p.getX(), p.getY(), p.getZ(), Block.getId(state)}) {
                hash = (hash ^ value) * 0x100000001b3L;
            }
            BlockState old = cells.put(p.immutable(), state);
            if (old == null || old.getBlock() != state.getBlock()) {
                blockEntities.remove(p);
                if (state.getBlock() instanceof EntityBlock entityBlock) {
                    var entity = entityBlock.newBlockEntity(p.immutable(), state);
                    if (entity != null) blockEntities.put(p.immutable(), entity);
                }
            }
            return true;
        }

        private long entityHash(net.minecraft.core.HolderLookup.Provider lookup) {
            long h = 0xcbf29ce484222325L;
            var order = java.util.Comparator.<BlockPos>comparingInt(p -> p.getX())
                    .thenComparingInt(p -> p.getY()).thenComparingInt(p -> p.getZ());
            for (var p : blockEntities.keySet().stream().sorted(order).toList()) {
                var tag = blockEntities.get(p).saveWithFullMetadata(lookup);
                var id = tag.getStringOr("id", "");
                for (int value : new int[]{p.getX(), p.getY(), p.getZ()}) h = mix(h, value);
                h = mix(h, id);
                switch (id) {
                    case "minecraft:chest", "minecraft:brushable_block" -> {
                        h = mix(mix(h, tag.getStringOr("LootTable", "")), tag.getLongOr("LootTableSeed", 0));
                    }
                    case "minecraft:mob_spawner" -> {
                        h = mix(h, tag.getCompoundOrEmpty("SpawnData").getCompoundOrEmpty("entity").getStringOr("id", ""));
                    }
                    case "minecraft:beehive" -> {
                        var bees = tag.getListOrEmpty("bees");
                        h = mix(h, bees.size());
                        for (var bee : bees.compoundStream().toList()) {
                            h = mix(h, bee.getCompoundOrEmpty("entity_data").getStringOr("id", ""));
                            h = mix(mix(h, bee.getIntOr("ticks_in_hive", -1)), bee.getIntOr("min_ticks_in_hive", -1));
                        }
                    }
                    case "minecraft:sculk_catalyst" -> h = mix(h, tag.getListOrEmpty("cursors").size());
                    case "minecraft:potent_sulfur" -> h = mix(h, tag.getIntOr("countdown", 0));
                    case "minecraft:sculk_sensor", "minecraft:sculk_shrieker" -> {
                        var listener = tag.getCompoundOrEmpty("listener");
                        h = mix(h, tag.getIntOr(id.endsWith("sensor") ? "last_vibration_frequency" : "warning_level", -1));
                        h = mix(mix(h, listener.getIntOr("event_delay", -1)), listener.getCompoundOrEmpty("selector").getLongOr("tick", 0));
                    }
                    default -> throw new IllegalArgumentException("Unexpected entity " + id);
                }
            }
            return h;
        }

        private static long mix(long hash, long value) {
            return (hash ^ value) * 0x100000001b3L;
        }

        private static long mix(long hash, String value) {
            for (int i = 0; i < value.length(); i++) hash = mix(hash, value.charAt(i));
            return mix(hash, 0);
        }

        @SuppressWarnings("unchecked")
        private WorldGenLevel level() {
            return (WorldGenLevel) Proxy.newProxyInstance(WorldGenLevel.class.getClassLoader(), new Class<?>[]{WorldGenLevel.class},
                    (proxy, method, args) -> switch (method.getName()) {
                        case "getBlockState" -> get((BlockPos)args[0]);
                        case "getFluidState" -> get((BlockPos)args[0]).getFluidState();
                        case "isClientSide" -> false;
                        case "registryAccess" -> RegistryLayer.createRegistryAccess().getLayer(RegistryLayer.STATIC);
                        case "getChunk" -> postProcess;
                        case "isWaterAt" -> get((BlockPos)args[0]).getFluidState().is(Fluids.WATER);
                        case "getMinY" -> -64;
                        case "getMaxY" -> 319;
                        case "getHeight" -> {
                            if (args == null) yield 384;
                            if (scenario.equals("cave")) yield origin.getY() + 37;
                            if (scenario.equals("root")) yield origin.getY() + 5;
                            throw new UnsupportedOperationException(method.toString());
                        }
                        case "getBlockEntity" -> {
                            var entity = blockEntities.get((BlockPos)args[0]);
                            if (args.length == 1) yield entity;
                            yield java.util.Optional.ofNullable(entity).filter(e -> e.getType() == args[1]);
                        }
                        case "isEmptyBlock" -> get((BlockPos)args[0]).isAir();
                        case "isStateAtPosition" -> ((Predicate<BlockState>)args[1]).test(get((BlockPos)args[0]));
                        case "isFluidAtPosition" -> ((Predicate<net.minecraft.world.level.material.FluidState>)args[1]).test(get((BlockPos)args[0]).getFluidState());
                        case "isOutsideBuildHeight" -> {
                            int y = args[0] instanceof BlockPos p ? p.getY() : (int)args[0];
                            yield y < -64 || y >= 320;
                        }
                        case "setBlock", "setBlockAndUpdate" -> put((BlockPos)args[0], (BlockState)args[1]);
                        case "destroyBlock" -> !get((BlockPos)args[0]).isAir() && put((BlockPos)args[0], Blocks.AIR.defaultBlockState());
                        case "getRandom" -> random;
                        case "scheduleTick", "updateNeighborsAt", "levelEvent", "playSound" -> null;
                        default -> {
                            if (method.isDefault()) yield java.lang.reflect.InvocationHandler.invokeDefault(proxy, method, args);
                            throw new UnsupportedOperationException(method.toString());
                        }
                    });
        }
    }
}
