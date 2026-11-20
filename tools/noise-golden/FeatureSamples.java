import java.lang.reflect.InvocationHandler;
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
import net.minecraft.core.Holder;
import net.minecraft.core.Registry;
import net.minecraft.core.registries.Registries;
import net.minecraft.data.registries.VanillaRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.tags.TagLoader;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.WorldGenLevel;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.Biomes;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.LiquidBlock;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.levelgen.Heightmap;
import net.minecraft.world.level.levelgen.WorldgenRandom;
import net.minecraft.world.level.levelgen.XoroshiroRandomSource;
import net.minecraft.world.level.levelgen.presets.WorldPresets;
import net.minecraft.world.level.material.FluidState;

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
        StringBuilder out = new StringBuilder();
        for (var entry : cases.entrySet()) {
            var feature = registry.getOrThrow(ResourceKey.create(Registries.FEATURE, Identifier.withDefaultNamespace(entry.getKey()))).value();
            for (String scenario : entry.getValue().split(",")) {
                var biome = lookup.lookupOrThrow(Registries.BIOME).getOrThrow(
                        scenario.equals("cold") ? Biomes.SNOWY_PLAINS : Biomes.PLAINS);
                for (int seed = 0; seed < 16; seed++) {
                    BlockPos origin = new BlockPos(seed * 13 - 80, 64, -47 + seed * 3);
                    WorldgenRandom rng = new WorldgenRandom(new XoroshiroRandomSource(seed));
                    Area area = new Area(scenario, origin, new XoroshiroRandomSource(seed + 100));
                    WorldGenLevel level = area.level(biome);
                    boolean permitted = switch (entry.getKey()) {
                        case "cactus" -> Blocks.CACTUS.defaultBlockState().canSurvive(level, origin);
                        case "sugar_cane" -> Blocks.SUGAR_CANE.defaultBlockState().canSurvive(level, origin);
                        default -> true;
                    };
                    boolean placed = permitted && feature.place(level, generator, rng, origin);
                    out.append("%s %s %d %b %s %d\n".formatted(entry.getKey(), scenario, seed, placed,
                            Long.toUnsignedString(area.hash, 16), rng.nextInt()));
                }
            }
        }
        Files.writeString(Path.of(args[0]), out);
    }

    private static class Area {
        private final Map<BlockPos, BlockState> cells = new HashMap<>();
        private final String scenario;
        private final BlockPos origin;
        private final RandomSource random;
        private long hash = 0xcbf29ce484222325L;

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

        private boolean put(BlockPos p, BlockState state) {
            if (p.getY() < -64 || p.getY() >= 320) return false;
            for (int value : new int[]{p.getX(), p.getY(), p.getZ(), Block.getId(state)}) {
                hash = (hash ^ value) * 0x100000001b3L;
            }
            cells.put(p.immutable(), state);
            return true;
        }

        @SuppressWarnings("unchecked")
        private WorldGenLevel level(Holder<Biome> biome) {
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
                        case "isEmptyBlock" -> get((BlockPos)args[0]).isAir();
                        case "isStateAtPosition" -> ((Predicate<BlockState>)args[1]).test(get((BlockPos)args[0]));
                        case "isOutsideBuildHeight" -> {
                            int y = args[0] instanceof BlockPos p ? p.getY() : (int)args[0];
                            yield y < -64 || y >= 320;
                        }
                        case "isInsideBuildHeight" -> (int)args[0] >= -64 && (int)args[0] < 320;
                        case "setBlock", "setBlockAndUpdate" -> put((BlockPos)args[0], (BlockState)args[1]);
                        case "destroyBlock" -> !get((BlockPos)args[0]).isAir() && put((BlockPos)args[0], Blocks.AIR.defaultBlockState());
                        case "getRandom" -> random;
                        case "scheduleTick" -> null;
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
