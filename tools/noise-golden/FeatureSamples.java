import java.lang.reflect.InvocationHandler;
import java.lang.reflect.Proxy;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.function.Predicate;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.resources.RegistryDataLoader;
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
import net.minecraft.world.level.chunk.UpgradeData;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.LiquidBlock;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.levelgen.Heightmap;
import net.minecraft.world.level.levelgen.WorldgenRandom;
import net.minecraft.world.level.levelgen.XoroshiroRandomSource;
import net.minecraft.world.level.levelgen.presets.WorldPresets;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.material.Fluids;

/** Feature write and RNG fixtures. Run against the vanilla 26.3 server and libraries. */
public class FeatureSamples {
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        try (var packs = new MultiPackResourceManager(PackType.SERVER_DATA, List.of(ServerPacksSource.createVanillaPackSource().fullResources()))) {
            var layers = RegistryLayer.createRegistryAccess();
            var tags = TagLoader.loadTagsForExistingRegistries(packs, layers.getLayer(RegistryLayer.STATIC));
            var context = TagLoader.buildUpdatedLookups(layers.getAccessForLoading(RegistryLayer.WORLD), tags);
            var lookup = RegistryDataLoader.load(packs, context, RegistryDataLoader.WORLD_REGISTRIES, Runnable::run).join();
            tags.forEach(tag -> tag.apply());
            writeSamples(args[0], lookup);
        }
    }

    private static void writeSamples(String output, net.minecraft.core.RegistryAccess lookup) throws Exception {
        var registry = lookup.lookupOrThrow(Registries.FEATURE);
        var biomes = lookup.lookupOrThrow(Registries.BIOME);
        var plains = biomes.getOrThrow(Biomes.PLAINS);
        var snowyPlains = biomes.getOrThrow(Biomes.SNOWY_PLAINS);
        var factory = PalettedContainerFactory.create(lookup);
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
        cases.put("brown_mushroom", "mycelium,grass");
        cases.put("red_mushroom", "mycelium,grass");
        cases.put("berry_bush", "grass");
        cases.put("bush", "grass");
        cases.put("red_shrub", "grass");
        cases.put("dry_grass", "sand");
        cases.put("firefly_bush", "grass");
        cases.put("leaf_litter", "stone");
        cases.put("melon", "ceiling");
        cases.put("pumpkin", "ceiling");
        cases.put("waterlily", "water_surface");
        cases.put("spore_blossom", "ceiling");
        cases.put("seagrass_tall", "water,shallow,flowing,flowing_top,falling");
        cases.put("cactus", "sand,cactus_lava");
        cases.put("sugar_cane", "cane_water,cane_frost,cane_ice");
        cases.put("freeze_top_layer", "cold,grass");
        cases.put("vines", "ceiling,wall,grass");
        cases.put("forest_rock", "grass,ceiling");
        cases.put("bamboo_some_podzol", "grass,bamboo_obstacle,stone");
        cases.put("bamboo_no_podzol", "grass");
        cases.put("kelp", "water");
        cases.put("disk_grass", "dirt");
        cases.put("bamboo_vegetation", "grass");
        cases.put("birch_tall", "grass");
        cases.put("dark_forest_vegetation", "grass");
        cases.put("mushroom_island_vegetation", "mycelium");
        cases.put("trees_plains", "grass");
        cases.put("trees_snowy", "grass");
        var existing = Set.copyOf(cases.keySet());
        cases.put("lake_lava", "lake");
        cases.put("sulfur_pool", "lake");
        cases.put("ice_spike", "snow");
        cases.put("iceberg_blue", "ocean");
        cases.put("iceberg_packed", "ocean");
        cases.put("underwater_magma", "magma");
        cases.put("warm_ocean_vegetation", "reef");
        cases.put("glow_lichen", "ceiling");
        cases.put("sculk_vein", "lichen_water");
        cases.put("amethyst_geode", "geode");
        StringBuilder out = new StringBuilder();
        for (var entry : cases.entrySet()) {
            var feature = registry.getOrThrow(ResourceKey.create(Registries.FEATURE, Identifier.withDefaultNamespace(entry.getKey()))).value();
            boolean checkEffects = !existing.contains(entry.getKey());
            for (String baseScenario : entry.getValue().split(",")) {
                int count = checkEffects ? 20 : 16;
                for (int seed = 0; seed < count; seed++) {
                    String scenario = seed < 16 ? baseScenario : switch (baseScenario) {
                        case "lake" -> "low";
                        case "magma", "snow", "geode", "ceiling" -> "open";
                        case "lichen_water", "reef" -> "buried";
                        default -> baseScenario;
                    };
                    BlockPos origin = new BlockPos(seed * 13 - 80, scenario.equals("low") ? -60 : 64, -47 + seed * 3);
                    WorldgenRandom rng = new WorldgenRandom(new XoroshiroRandomSource(seed));
                    var biome = scenario.equals("cold") ? snowyPlains : plains;
                    Area area = new Area(scenario, origin, new XoroshiroRandomSource(seed + 100), biome, factory);
                    WorldGenLevel level = area.level();
                    boolean permitted = switch (entry.getKey()) {
                        case "cactus" -> Blocks.CACTUS.defaultBlockState().canSurvive(level, origin);
                        case "sugar_cane" -> Blocks.SUGAR_CANE.defaultBlockState().canSurvive(level, origin);
                        default -> true;
                    };
                    boolean placed = permitted && feature.place(level, generator, rng, origin);
                    out.append("%s %s %d %b %s %d".formatted(entry.getKey(), scenario, seed, placed,
                            Long.toUnsignedString(area.hash, 16), rng.nextInt()));
                    if (checkEffects) {
                        for (long value : new long[]{area.flags, area.blockTicks, area.fluidTicks, area.postProcessing}) {
                            out.append(' ').append(Long.toUnsignedString(value, 16));
                        }
                    }
                    out.append('\n');
                }
            }
        }
        Files.writeString(Path.of(output), out);
    }

    private static class Area {
        private final Map<BlockPos, BlockState> cells = new HashMap<>();
        private final String scenario;
        private final BlockPos origin;
        private final RandomSource random;
        private final Holder<Biome> biome;
        private final PalettedContainerFactory factory;
        private final Map<ChunkPos, ProtoChunk> chunks = new HashMap<>();
        private long hash = 0xcbf29ce484222325L;
        private long flags = hash;
        private long blockTicks = hash;
        private long fluidTicks = hash;
        private long postProcessing = hash;

        Area(String scenario, BlockPos origin, RandomSource random, Holder<Biome> biome, PalettedContainerFactory factory) {
            this.scenario = scenario;
            this.origin = origin;
            this.random = random;
            this.biome = biome;
            this.factory = factory;
        }

        private BlockState get(BlockPos p) {
            BlockState placed = cells.get(p);
            if (placed != null) return placed;
            int x = p.getX() - origin.getX();
            int y = p.getY() - origin.getY();
            int z = p.getZ() - origin.getZ();
            if (scenario.equals("open")) return Blocks.AIR.defaultBlockState();
            if (scenario.equals("buried")) return Blocks.STONE.defaultBlockState();
            if (scenario.equals("lake")) return y < 0 ? Blocks.STONE.defaultBlockState() : Blocks.AIR.defaultBlockState();
            if (scenario.equals("snow")) {
                if (y == -1) return Blocks.SNOW_BLOCK.defaultBlockState();
                if (y < -1) return Blocks.DIRT.defaultBlockState();
                return Blocks.AIR.defaultBlockState();
            }
            if (scenario.equals("reef")) return y < 0 ? Blocks.STONE.defaultBlockState() : Blocks.WATER.defaultBlockState();
            if (scenario.equals("ocean") || scenario.equals("magma")) {
                int floor = scenario.equals("magma") ? -3 : -32;
                if (y <= floor) return Blocks.STONE.defaultBlockState();
                return y < 0 || scenario.equals("magma") ? Blocks.WATER.defaultBlockState() : Blocks.AIR.defaultBlockState();
            }
            if (scenario.equals("lichen_water")) {
                if (y == -1 || x == -1) return Blocks.STONE.defaultBlockState();
                return Blocks.WATER.defaultBlockState();
            }
            if (scenario.equals("geode")) return (x == 8 || z == -2) ? Blocks.WATER.defaultBlockState() : Blocks.STONE.defaultBlockState();
            if (scenario.equals("ceiling")) return y >= 1 ? Blocks.STONE.defaultBlockState() : Blocks.AIR.defaultBlockState();
            if (scenario.equals("pale") && x == -1 && z == 0 && y >= 0 && y <= 3) return Blocks.STONE.defaultBlockState();
            if (scenario.equals("moss") && x == 1 && z == 0 && y == -1) return Blocks.BEDROCK.defaultBlockState();
            if (scenario.equals("pool") && y == -1 && (x == 1 || z == -1)) return Blocks.BEDROCK.defaultBlockState();
            if (scenario.equals("crimson") && x == 0 && z == 0 && y == 1) return Blocks.POPPY.defaultBlockState();
            if (scenario.equals("wall") && y == 0 && (z == -1 || x == -1)) return Blocks.STONE.defaultBlockState();
            if (scenario.equals("bamboo_obstacle") && x == 0 && z == 0 && y == 3) return Blocks.STONE.defaultBlockState();
            if (scenario.equals("cactus_lava") && z == -1 && y == 0) return Blocks.LAVA.defaultBlockState();
            if (scenario.equals("cane_water") && z == -1 && y == -1) return Blocks.WATER.defaultBlockState().setValue(LiquidBlock.LEVEL, 2);
            if (scenario.equals("cane_frost") && z == -1 && y == -1) return Blocks.FROSTED_ICE.defaultBlockState();
            if (scenario.equals("cane_ice") && z == -1 && y == -1) return Blocks.ICE.defaultBlockState();
            if (scenario.equals("water") && y >= 0 && y <= 11
                    || scenario.equals("shallow") && y == 0
                    || scenario.equals("flowing_top") && y == 0
                    || scenario.equals("falling") && y == 1
                    || scenario.equals("water_surface") && y == -1
                    || scenario.equals("cold") && y == -1 && (x & 1) == 0) return Blocks.WATER.defaultBlockState();
            if (scenario.equals("flowing") && y >= 0 && y <= 1
                    || scenario.equals("flowing_top") && y == 1) return Blocks.WATER.defaultBlockState().setValue(LiquidBlock.LEVEL, 2);
            if (scenario.equals("falling") && y == 0) return Blocks.WATER.defaultBlockState().setValue(LiquidBlock.LEVEL, 8);
            if (y >= 0) return Blocks.AIR.defaultBlockState();
            return switch (scenario) {
                case "grass", "cold", "bamboo_obstacle", "cane_water", "cane_frost", "cane_ice" -> Blocks.GRASS_BLOCK.defaultBlockState();
                case "mycelium" -> Blocks.MYCELIUM.defaultBlockState();
                case "sand", "cactus_lava" -> Blocks.SAND.defaultBlockState();
                case "dirt" -> Blocks.DIRT.defaultBlockState();
                case "crimson" -> Blocks.CRIMSON_NYLIUM.defaultBlockState();
                case "warped" -> Blocks.WARPED_NYLIUM.defaultBlockState();
                default -> Blocks.STONE.defaultBlockState();
            };
        }

        private static long mix(long hash, int value) {
            return (hash ^ value) * 0x100000001b3L;
        }

        private static long mixPos(long hash, BlockPos p) {
            return mix(mix(mix(hash, p.getX()), p.getY()), p.getZ());
        }

        private boolean put(BlockPos p, BlockState state, int updateFlags) {
            if (p.getY() < -64 || p.getY() >= 320) return false;
            hash = mix(mixPos(hash, p), Block.getId(state));
            flags = mix(flags, updateFlags);
            cells.put(p.immutable(), state);
            return true;
        }

        @SuppressWarnings("unchecked")
        private WorldGenLevel level() {
            return (WorldGenLevel) Proxy.newProxyInstance(WorldGenLevel.class.getClassLoader(), new Class<?>[]{WorldGenLevel.class},
                    (proxy, method, args) -> switch (method.getName()) {
                        case "getBlockState" -> get((BlockPos)args[0]);
                        case "getFluidState" -> get((BlockPos)args[0]).getFluidState();
                        case "getMinY" -> -64;
                        case "getMaxY" -> 319;
                        case "getHeight" -> args == null ? 384 : height((Heightmap.Types)args[0], (int)args[1], (int)args[2]);
                        case "getSeaLevel" -> 63;
                        case "getBiome" -> biome;
                        case "getBrightness" -> 0;
                        case "getRawBrightness" -> 15;
                        case "isFluidAtPosition" -> ((Predicate<FluidState>)args[1]).test(get((BlockPos)args[0]).getFluidState());
                        case "getBlockEntity" -> args.length == 1 ? null : java.util.Optional.empty();
                        case "markPosForPostprocessing" -> null;
                        case "getSeed" -> 0L;
                        case "isInsideBuildHeight" -> {
                            int y = args[0] instanceof BlockPos p ? p.getY() : (int)args[0];
                            yield y >= -64 && y < 320;
                        }
                        case "getChunk" -> chunks.computeIfAbsent(ChunkPos.containing((BlockPos)args[0]), p ->
                                new ProtoChunk(p, UpgradeData.EMPTY, LevelHeightAccessor.create(-64, 384), factory, null) {
                                    @Override public void markPosForPostProcessing(BlockPos pos) {
                                        super.markPosForPostProcessing(pos);
                                        if (isInsideBuildHeight(pos)) Area.this.postProcessing = mixPos(Area.this.postProcessing, pos);
                                    }
                                });
                        case "isEmptyBlock" -> get((BlockPos)args[0]).isAir();
                        case "isStateAtPosition" -> ((Predicate<BlockState>)args[1]).test(get((BlockPos)args[0]));
                        case "isOutsideBuildHeight" -> {
                            int y = args[0] instanceof BlockPos p ? p.getY() : (int)args[0];
                            yield y < -64 || y >= 320;
                        }
                        case "setBlock" -> put((BlockPos)args[0], (BlockState)args[1], (int)args[2]);
                        case "setBlockAndUpdate" -> put((BlockPos)args[0], (BlockState)args[1], Block.UPDATE_ALL);
                        case "destroyBlock" -> !get((BlockPos)args[0]).isAir() && put((BlockPos)args[0], Blocks.AIR.defaultBlockState(), Block.UPDATE_ALL);
                        case "getRandom" -> random;
                        case "scheduleTick" -> {
                            var p = (BlockPos)args[0];
                            var type = args[1];
                            if (type instanceof Block b) {
                                blockTicks = mix(mixPos(blockTicks, p), Block.getId(b.defaultBlockState()));
                            } else {
                                int fluid = type == Fluids.WATER ? 1 : type == Fluids.FLOWING_WATER ? 2 : type == Fluids.LAVA ? 3 : 4;
                                fluidTicks = mix(mixPos(fluidTicks, p), fluid);
                            }
                            yield null;
                        }
                        default -> {
                            if (method.isDefault()) yield InvocationHandler.invokeDefault(proxy, method, args);
                            throw new UnsupportedOperationException(method.toString());
                        }
                    });
        }

        private int height(Heightmap.Types type, int x, int z) {
            for (int y = 319; y >= -64; y--) {
                if (type.isOpaque().test(get(new BlockPos(x, y, z)))) return y + 1;
            }
            return -64;
        }
    }
}
