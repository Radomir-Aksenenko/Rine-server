import java.io.BufferedOutputStream;
import java.io.DataOutputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.OutputStreamWriter;
import java.io.Writer;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.data.registries.VanillaRegistries;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.LevelHeightAccessor;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.BiomeManager;
import net.minecraft.world.level.biome.BiomeResolver;
import net.minecraft.world.level.biome.Climate;
import net.minecraft.world.level.biome.MultiNoiseBiomeSource;
import net.minecraft.world.level.biome.MultiNoiseBiomeSourceParameterLists;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.ChunkAccess;
import net.minecraft.world.level.chunk.PalettedContainerFactory;
import net.minecraft.world.level.chunk.ProtoChunk;
import net.minecraft.world.level.chunk.UpgradeData;
import net.minecraft.world.level.chunk.status.ChunkStatus;
import net.minecraft.world.level.levelgen.Aquifer;
import net.minecraft.world.level.levelgen.Beardifier;
import net.minecraft.world.level.levelgen.NoiseBasedChunkGenerator;
import net.minecraft.world.level.levelgen.NoiseChunk;
import net.minecraft.world.level.levelgen.NoiseGeneratorSettings;
import net.minecraft.world.level.levelgen.RandomState;
import net.minecraft.world.level.levelgen.WorldGenerationContext;
import net.minecraft.world.level.levelgen.blending.Blender;
import net.minecraft.world.level.levelgen.densityfunction.DensityVolume;
import net.minecraft.world.level.levelgen.densityfunction.SamplerContext;
import net.minecraft.world.level.levelgen.synth.NormalNoise;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.level.biome.MultiNoiseBiomeSourceParameterList;

/**
 * Worldgen golden generator for rine-worldgen phase 3 (biomes, noise fill, surface rules).
 *
 * Runs the vanilla 26.3 steps per chunk, without carvers, features and structure terrain (Beardifier.EMPTY):
 *   BIOMES  : ChunkGenerator.doCreateBiomes equivalent (createResolverForChunk + fillBiomesFromNoise).
 *   NOISE   : NoiseBasedChunkGenerator.doFill (private, called by reflection).
 *   SURFACE : MaterialSystem.buildSurface, with BiomeManager backed by the stored biomes of the 3x3 chunks.
 *
 * Biome lookups are "pure": the RTree lastResult ThreadLocal is cleared before every search, so the
 * result of a query never depends on earlier queries. The Rust port implements the same pure model.
 * The sequential (history-dependent) variant is also computed and the number of differing quart cells
 * is printed, so the size of that effect is visible.
 *
 * Output (written into the directory given as args[0]):
 *   worldgen_golden.bin   : header, then per chunk: cx, cz, block palette, biome palette, u16 block
 *                           indices [((y-minY)*16+z)*16+x], u16 quart biome indices [((qy)*4+qz)*4+qx].
 *   worldgen_params.txt   : the quantized overworld parameter list in list order (for the Rust bundle check).
 */
public class WorldgenGolden {
    static final long SEED = 1592598566L;
    static final int MIN_Y = -64;
    static final int HEIGHT = 384;
    static final int[][] CHUNKS = {{0, 0}, {5, -3}, {-17, 40}, {100, 100}, {-64, 9}};

    static BiomeSource_ src;

    static class BiomeSource_ {
        MultiNoiseBiomeSource biomeSource;
        Field indexField;
        Field lastResultField;
        Method parametersMethod;
    }

    static String blockName(BlockState s) {
        StringBuilder sb = new StringBuilder(BuiltInRegistries.BLOCK.getKey(s.getBlock()).toString());
        if (!s.getProperties().isEmpty()) {
            sb.append('[');
            boolean first = true;
            for (Property<?> p : s.getProperties()) {
                if (!first) sb.append(',');
                first = false;
                sb.append(p.getName()).append('=').append(propValue(s, p));
            }
            sb.append(']');
        }
        return sb.toString();
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    static String propValue(BlockState s, Property p) {
        return p.getName((Comparable) s.getValue(p));
    }

    /**
     * PalettedContainerFactory.create(RegistryAccess) equivalent without a RegistryAccess: the same block
     * state strategy, default air, biome strategy over the biome holders (ids only affect palette layout),
     * and PLAINS as the default biome. Codecs are not used by the worldgen steps and are passed as null.
     */
    static PalettedContainerFactory biomeFactory(HolderLookup.Provider registries) {
        HolderLookup.RegistryLookup<Biome> biomes = registries.lookupOrThrow(Registries.BIOME);
        List<Holder<Biome>> all = new ArrayList<>();
        biomes.listElements().forEach(all::add);
        Map<Holder<Biome>, Integer> ids = new java.util.IdentityHashMap<>();
        for (int i = 0; i < all.size(); i++) ids.put(all.get(i), i);
        net.minecraft.core.IdMap<Holder<Biome>> idMap = new net.minecraft.core.IdMap<>() {
            @Override
            public int getId(Holder<Biome> value) {
                Integer i = ids.get(value);
                return i == null ? -1 : i;
            }

            @Override
            public Holder<Biome> byId(int id) {
                return id >= 0 && id < all.size() ? all.get(id) : null;
            }

            @Override
            public int size() {
                return all.size();
            }

            @Override
            public java.util.Iterator<Holder<Biome>> iterator() {
                return all.iterator();
            }
        };
        net.minecraft.world.level.chunk.Strategy<BlockState> blockStrategy =
            net.minecraft.world.level.chunk.Strategy.createForBlockStates(net.minecraft.world.level.block.Block.BLOCK_STATE_REGISTRY);
        net.minecraft.world.level.chunk.Strategy<Holder<Biome>> biomeStrategy = net.minecraft.world.level.chunk.Strategy.createForBiomes(idMap);
        Holder<Biome> plains = biomes.getOrThrow(net.minecraft.world.level.biome.Biomes.PLAINS);
        return new PalettedContainerFactory(blockStrategy, net.minecraft.world.level.block.Blocks.AIR.defaultBlockState(), null, biomeStrategy, plains, null);
    }

    static String biomeName(Holder<Biome> h) {
        return h.unwrapKey().orElseThrow().identifier().toString();
    }

    /** Clears the RTree's ThreadLocal lastResult so the next search starts without a candidate. */
    static void resetRTree() {
        try {
            Method m = src.parametersMethod;
            Object list = m.invoke(src.biomeSource);
            Object index = src.indexField.get(list);
            Object tl = src.lastResultField.get(index);
            ((ThreadLocal<?>) tl).remove();
        } catch (ReflectiveOperationException e) {
            throw new RuntimeException(e);
        }
    }

    static class Ctx {
        RandomState rs;
        NoiseGeneratorSettings settings;
        NoiseBasedChunkGenerator gen;
        PalettedContainerFactory factory;
        Aquifer.FluidPicker fluidPicker;
        Method doFill;
        Map<Long, ProtoChunk> biomeChunks = new HashMap<>();
    }

    /** Surface-style biome fill of one chunk, with the RTree reset before every query when pure. */
    static ProtoChunk fillBiomes(Ctx c, int cx, int cz, boolean pure) {
        ProtoChunk chunk = new ProtoChunk(new ChunkPos(cx, cz), UpgradeData.EMPTY, LevelHeightAccessor.create(MIN_Y, HEIGHT), c.factory, null);
        Climate.Sampler sampler = c.rs.createClimateSampler(SamplerContext.builder().enableCaches().build());
        int minQX = cx * 4;
        int minQZ = cz * 4;
        BiomeResolver inner = src.biomeSource.createResolverForChunk(sampler, minQX, MIN_Y >> 2, minQZ, 4, HEIGHT >> 2, 4);
        BiomeResolver resolver = pure ? (qx, qy, qz) -> {
            resetRTree();
            return inner.getNoiseBiome(qx, qy, qz);
        } : inner;
        chunk.fillBiomesFromNoise(resolver);
        chunk.setPersistedStatus(ChunkStatus.BIOMES);
        return chunk;
    }

    static ProtoChunk neighbour(Ctx c, int cx, int cz) {
        long key = ((long) cx << 32) ^ (cz & 0xffffffffL);
        return c.biomeChunks.computeIfAbsent(key, k -> fillBiomes(c, cx, cz, true));
    }

    /** One chunk of the pure model: biomes, noise fill, surface. Fills the palettes and index arrays. */
    static void runChunk(Ctx c, int cx, int cz, List<String> blockPalette, List<String> biomePalette, int[] blocksOut, int[] biomesOut) {
        ProtoChunk chunk = fillBiomes(c, cx, cz, true);

        // Region-style biome source for the surface step: stored biomes of the 3x3 chunks around (cx, cz).
        final Ctx fc = c;
        BiomeResolver region = (qx, qy, qz) -> {
            int ncx = Math.floorDiv(qx, 4);
            int ncz = Math.floorDiv(qz, 4);
            ProtoChunk nc = ncx == cx && ncz == cz ? chunk : neighbour(fc, ncx, ncz);
            return nc.getNoiseBiome(qx, qy, qz);
        };
        BiomeManager biomeManager = new BiomeManager(region, BiomeManager.obfuscateSeed(SEED));

        NoiseChunk noiseChunk = new NoiseChunk(c.rs, Beardifier.EMPTY, c.settings, c.fluidPicker, Blender.empty(),
            new DensityVolume(16, HEIGHT, 16, cx * 16, MIN_Y, cz * 16));
        try {
            c.doFill.invoke(c.gen, noiseChunk, chunk);
            c.rs.surfaceSystem().buildSurface(c.rs, biomeManager, new WorldGenerationContext(c.gen, chunk.getHeightAccessorForGeneration()),
                chunk, noiseChunk, c.settings.materialRule().value(), null);
        } catch (ReflectiveOperationException e) {
            throw new RuntimeException(e);
        } finally {
            noiseChunk.close();
        }

        List<String> palette = new ArrayList<>();
        Map<String, Integer> index = new LinkedHashMap<>();
        for (int y = MIN_Y; y < MIN_Y + HEIGHT; y++) {
            for (int z = 0; z < 16; z++) {
                for (int x = 0; x < 16; x++) {
                    String name = blockName(chunk.getBlockState(new BlockPos(cx * 16 + x, y, cz * 16 + z)));
                    Integer i = index.get(name);
                    if (i == null) {
                        i = index.size();
                        index.put(name, i);
                    }
                    blocksOut[((y - MIN_Y) * 16 + z) * 16 + x] = i;
                }
            }
        }
        for (String name : index.keySet()) palette.add(name);
        blockPalette.clear();
        blockPalette.addAll(palette);

        Map<String, Integer> bidx = new LinkedHashMap<>();
        for (int qy = 0; qy < HEIGHT / 4; qy++) {
            for (int qz = 0; qz < 4; qz++) {
                for (int qx = 0; qx < 4; qx++) {
                    String name = biomeName(chunk.getNoiseBiome(cx * 4 + qx, (MIN_Y >> 2) + qy, cz * 4 + qz));
                    Integer i = bidx.get(name);
                    if (i == null) {
                        i = bidx.size();
                        bidx.put(name, i);
                    }
                    biomesOut[(qy * 4 + qz) * 4 + qx] = i;
                }
            }
        }
        biomePalette.clear();
        biomePalette.addAll(bidx.keySet());
    }

    public static void main(String[] args) throws Exception {
        Path outDir = Path.of(args[0]);
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();

        HolderLookup.Provider registries = VanillaRegistries.createWorldLookup();
        HolderLookup.RegistryLookup<NormalNoise> noises = registries.lookupOrThrow(Registries.NOISE);
        Holder<NoiseGeneratorSettings> settingsHolder = registries.lookupOrThrow(Registries.NOISE_SETTINGS)
            .getOrThrow(NoiseGeneratorSettings.OVERWORLD);
        NoiseGeneratorSettings settings = settingsHolder.value();
        Holder<MultiNoiseBiomeSourceParameterList> preset = registries.lookupOrThrow(Registries.MULTI_NOISE_BIOME_SOURCE_PARAMETER_LIST)
            .getOrThrow(MultiNoiseBiomeSourceParameterLists.OVERWORLD);
        MultiNoiseBiomeSource biomeSource = MultiNoiseBiomeSource.createFromPreset(preset);

        src = new BiomeSource_();
        src.biomeSource = biomeSource;
        src.parametersMethod = MultiNoiseBiomeSource.class.getDeclaredMethod("parameters");
        src.parametersMethod.setAccessible(true);
        Class<?> plClass = Class.forName("net.minecraft.world.level.biome.Climate$ParameterList");
        src.indexField = plClass.getDeclaredField("index");
        src.indexField.setAccessible(true);
        Class<?> rtClass = Class.forName("net.minecraft.world.level.biome.Climate$RTree");
        src.lastResultField = rtClass.getDeclaredField("lastResult");
        src.lastResultField.setAccessible(true);

        // Quantized parameter list, list order, for the Rust bundle cross-check.
        Method paramsMethod = src.parametersMethod;
        Object pl = paramsMethod.invoke(biomeSource);
        Method valuesMethod = plClass.getDeclaredMethod("values");
        List<?> values = (List<?>) valuesMethod.invoke(pl);
        try (Writer w = new OutputStreamWriter(new FileOutputStream(outDir.resolve("worldgen_params.txt").toFile()), StandardCharsets.UTF_8)) {
            for (Object pair : values) {
                Object pp = pairFirst(pair);
                Object biome = pairSecond(pair);
                Climate.ParameterPoint point = (Climate.ParameterPoint) pp;
                w.write(((Holder<Biome>) biome).unwrapKey().orElseThrow().identifier() + " "
                    + q(point.temperature()) + " " + q(point.humidity()) + " " + q(point.continentalness()) + " "
                    + q(point.erosion()) + " " + q(point.depth()) + " " + q(point.weirdness()) + " " + point.offset() + "\n");
            }
        }

        Ctx c = new Ctx();
        c.rs = RandomState.create(noises, SEED, settings);
        c.settings = settings;
        c.gen = new NoiseBasedChunkGenerator(biomeSource, settingsHolder);
        c.factory = biomeFactory(registries);
        Method fluid = NoiseBasedChunkGenerator.class.getDeclaredMethod("createFluidPicker", NoiseGeneratorSettings.class);
        fluid.setAccessible(true);
        c.fluidPicker = (Aquifer.FluidPicker) fluid.invoke(null, settings);
        c.doFill = NoiseBasedChunkGenerator.class.getDeclaredMethod("doFill", NoiseChunk.class, ChunkAccess.class);
        c.doFill.setAccessible(true);

        List<Object[]> results = new ArrayList<>();
        for (int[] cc : CHUNKS) {
            final int cx = cc[0];
            final int cz = cc[1];
            final Object[] out = new Object[5];
            Thread t = new Thread(() -> {
                try {
                    // Pure model. A fresh thread per chunk gives a fresh ThreadLocal; the reset makes each query pure.
                    List<String> bpal = new ArrayList<>();
                    List<String> ppal = new ArrayList<>();
                    int[] blocks = new int[HEIGHT * 256];
                    int[] biomes = new int[(HEIGHT / 4) * 16];
                    runChunk(c, cx, cz, bpal, ppal, blocks, biomes);
                    out[0] = bpal;
                    out[1] = ppal;
                    out[2] = blocks;
                    out[3] = biomes;
                } catch (Throwable e) {
                    e.printStackTrace();
                    out[4] = e;
                }
            });
            t.start();
            t.join();
            if (out[4] != null) throw new RuntimeException((Throwable) out[4]);
            results.add(out);
            // Sequential (history-dependent) biome fill of the same chunk on a fresh thread, for reporting only.
            final Object[] seq = new Object[2];
            Thread t2 = new Thread(() -> {
                ProtoChunk seqChunk = fillBiomes(c, cx, cz, false);
                List<String> sp = new ArrayList<>();
                for (int qy = 0; qy < HEIGHT / 4; qy++) {
                    for (int qz = 0; qz < 4; qz++) {
                        for (int qx = 0; qx < 4; qx++) {
                            sp.add(biomeName(seqChunk.getNoiseBiome(cx * 4 + qx, (MIN_Y >> 2) + qy, cz * 4 + qz)));
                        }
                    }
                }
                seq[0] = sp;
            });
            t2.start();
            t2.join();
            @SuppressWarnings("unchecked")
            List<String> sp = (List<String>) seq[0];
            List<String> pureList = new ArrayList<>();
            for (int qy = 0; qy < HEIGHT / 4; qy++) {
                for (int qz = 0; qz < 4; qz++) {
                    for (int qx = 0; qx < 4; qx++) {
                        int i = (qy * 4 + qz) * 4 + qx;
                        pureList.add(((List<String>) out[1]).get(((int[]) out[3])[i]));
                    }
                }
            }
            int diff = 0;
            for (int i = 0; i < pureList.size(); i++) {
                if (!pureList.get(i).equals(sp.get(i))) diff++;
            }
            System.out.println("chunk " + cx + " " + cz + ": quart biomes differing between pure and sequential RTree history: " + diff + " of " + pureList.size());
        }

        try (DataOutputStream dos = new DataOutputStream(new BufferedOutputStream(new FileOutputStream(outDir.resolve("worldgen_golden.bin").toFile())))) {
            dos.writeInt(CHUNKS.length);
            for (int k = 0; k < CHUNKS.length; k++) {
                Object[] r = results.get(k);
                dos.writeInt(CHUNKS[k][0]);
                dos.writeInt(CHUNKS[k][1]);
                writeStrings(dos, (List<String>) r[0]);
                writeStrings(dos, (List<String>) r[1]);
                int[] blocks = (int[]) r[2];
                dos.writeInt(blocks.length);
                for (int v : blocks) dos.writeShort(v);
                int[] biomes = (int[]) r[3];
                dos.writeInt(biomes.length);
                for (int v : biomes) dos.writeShort(v);
            }
        }
        System.out.println("wrote " + outDir);
    }

    static void writeStrings(DataOutputStream dos, List<String> strings) throws IOException {
        dos.writeInt(strings.size());
        for (String s : strings) {
            byte[] b = s.getBytes(StandardCharsets.UTF_8);
            dos.writeShort(b.length);
            dos.write(b);
        }
    }

    static String q(Climate.Parameter p) {
        return p.min() + " " + p.max();
    }

    static Object pairFirst(Object pair) throws ReflectiveOperationException {
        return pair.getClass().getMethod("getFirst").invoke(pair);
    }

    static Object pairSecond(Object pair) throws ReflectiveOperationException {
        return pair.getClass().getMethod("getSecond").invoke(pair);
    }
}
