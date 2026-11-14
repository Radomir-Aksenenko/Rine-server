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
import java.util.BitSet;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.function.Function;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.data.registries.VanillaRegistries;
import net.minecraft.server.Bootstrap;
import net.minecraft.util.Mth;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.LevelHeightAccessor;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.BiomeManager;
import net.minecraft.world.level.biome.BiomeResolver;
import net.minecraft.world.level.biome.MultiNoiseBiomeSource;
import net.minecraft.world.level.biome.MultiNoiseBiomeSourceParameterLists;
import net.minecraft.world.level.biome.Climate;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.level.chunk.CarverOutput;
import net.minecraft.world.level.chunk.CarvingMask;
import net.minecraft.world.level.chunk.ChunkAccess;
import net.minecraft.world.level.chunk.PalettedContainerFactory;
import net.minecraft.world.level.chunk.ProtoChunk;
import net.minecraft.world.level.chunk.UpgradeData;
import net.minecraft.world.level.chunk.status.ChunkStatus;
import net.minecraft.world.level.levelgen.Aquifer;
import net.minecraft.world.level.levelgen.Beardifier;
import net.minecraft.world.level.levelgen.LegacyRandomSource;
import net.minecraft.world.level.levelgen.NoiseBasedChunkGenerator;
import net.minecraft.world.level.levelgen.NoiseChunk;
import net.minecraft.world.level.levelgen.NoiseGeneratorSettings;
import net.minecraft.world.level.levelgen.RandomState;
import net.minecraft.world.level.levelgen.RandomSupport;
import net.minecraft.world.level.levelgen.WorldGenerationContext;
import net.minecraft.world.level.levelgen.WorldgenRandom;
import net.minecraft.world.level.levelgen.blending.Blender;
import net.minecraft.world.level.levelgen.carver.CanyonWorldCarver;
import net.minecraft.world.level.levelgen.carver.WorldCarver;
import net.minecraft.world.level.levelgen.densityfunction.DensityVolume;
import net.minecraft.world.level.levelgen.densityfunction.SamplerContext;
import net.minecraft.world.level.levelgen.synth.NormalNoise;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.tags.BlockTags;
import net.minecraft.core.MappedRegistry;
import net.minecraft.tags.TagKey;

/**
 * Carver golden for rine-worldgen phase 4: overworld noise, surface, and the CARVERS step of vanilla 26.3.
 *
 * Per chunk: BIOMES (pure RTree model, as WorldgenGolden), NOISE (doFill), SURFACE (buildSurface), then the
 * carver step: the source loop of generateCarvers (same carver objects, same seeds, the biome of each
 * source chunk from the uncached biome source at quart (minX, 0, minZ)), the CarvingMask, and
 * applyCarvingMask (private, called by reflection, with the blender's null carving filter).
 *
 * The RTree lastResult ThreadLocal is cleared before every biome query, so each query is pure.
 *
 * Chunks: the five fixed chunks of phase 3, plus three chosen by a mask-only scan (see pick()):
 * the chunk with the most canyon cells, the one with the most cave cells, and the one with the most
 * cells below y = -54 (the lava level).
 *
 * Output (directory args[0]):
 *   carver_golden.bin      : int count, then per chunk: cx, cz, palette strings, u16 block indices
 *                            [((y-minY)*16+z)*16+x], int mask minY, maxY, long word count, long words
 *                            (BitSet.toLongArray of the CarvingMask), int dirt->grass count (topMaterial).
 *   carver_sin_table.bin   : Mth.SIN, 65536 big-endian float bit patterns.
 *   carver_biomes.txt      : per overworld biome, the carver names of its generation settings.
 */
public class CarverGolden {
    static final long SEED = 1592598566L;
    static final int MIN_Y = -64;
    static final int HEIGHT = 384;
    static final int MASK_MIN_Y = MIN_Y + 1;
    static final int MASK_MAX_Y = MIN_Y + HEIGHT - 1 - 7;
    static final int[][] FIXED = {{0, 0}, {5, -3}, {-17, 40}, {100, 100}, {-64, 9}};
    static final int SCAN_MIN = -30;
    static final int SCAN_MAX = 30;
    static final int SCAN_STEP = 3;

    static Method parametersMethod;
    static Field indexField;
    static Field lastResultField;
    static Field maskField;
    static MultiNoiseBiomeSource biomeSource;
    static Method applyCarvingMask;

    static class Ctx {
        RandomState rs;
        NoiseGeneratorSettings settings;
        NoiseBasedChunkGenerator gen;
        PalettedContainerFactory factory;
        Aquifer.FluidPicker fluidPicker;
        Method doFill;
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
            Object list = parametersMethod.invoke(biomeSource);
            Object index = indexField.get(list);
            Object tl = lastResultField.get(index);
            ((ThreadLocal<?>) tl).remove();
        } catch (ReflectiveOperationException e) {
            throw new RuntimeException(e);
        }
    }

    /** Pure uncached resolver: every query starts from an empty RTree history. */
    static BiomeResolver pure(BiomeResolver inner) {
        return (qx, qy, qz) -> {
            resetRTree();
            return inner.getNoiseBiome(qx, qy, qz);
        };
    }

    static WorldGenerationContext context(Ctx c) {
        return new WorldGenerationContext(c.gen, LevelHeightAccessor.create(MIN_Y, HEIGHT));
    }

    static long bitCount(BitSet b) {
        return b.cardinality();
    }

    static BitSet bits(CarvingMask m) {
        try {
            return (BitSet) maskField.get(m);
        } catch (IllegalAccessException e) {
            throw new RuntimeException(e);
        }
    }

    /** Number of set cells with y below -54 (the lava level). */
    static long deepCells(CarvingMask m) {
        BitSet b = bits(m);
        int height = MASK_MAX_Y - MASK_MIN_Y + 1;
        long n = 0;
        for (int i = b.nextSetBit(0); i >= 0; i = b.nextSetBit(i + 1)) {
            int y = MASK_MIN_Y + i % height;
            if (y < -54) n++;
        }
        return n;
    }

    /** One carving output that writes into two masks: the union mask and the mask of one carver kind. */
    static CarverOutput both(CarvingMask total, CarvingMask kind) {
        return new CarverOutput() {
            @Override
            public void carve(int x, int y, int z) {
                total.carve(x, y, z);
                kind.carve(x, y, z);
            }

            @Override
            public int minY() {
                return total.minY();
            }

            @Override
            public int maxY() {
                return total.maxY();
            }
        };
    }

    /**
     * generateCarvers, source loop: fills `total` (union of all carvers) and `caves`/`canyons` (per kind).
     * Uses the carver objects and the same seeding as the vanilla method.
     */
    static void carveSources(Ctx c, int cx, int cz, CarvingMask total, CarvingMask caves, CarvingMask canyons) {
        WorldGenerationContext context = context(c);
        WorldgenRandom random = new WorldgenRandom(new LegacyRandomSource(RandomSupport.generateUniqueSeed()));
        BiomeResolver resolver = pure(biomeSource.createUncachedResolver(c.rs));
        ChunkPos pos = new ChunkPos(cx, cz);
        for (int dx = -8; dx <= 8; dx++) {
            for (int dz = -8; dz <= 8; dz++) {
                ChunkPos sourcePos = new ChunkPos(pos.x() + dx, pos.z() + dz);
                int quartX = sourcePos.getMinBlockX() >> 2;
                int quartZ = sourcePos.getMinBlockZ() >> 2;
                Holder<Biome> biome = resolver.getNoiseBiome(quartX, 0, quartZ);
                Iterable<Holder<WorldCarver>> carvers = c.gen.getBiomeGenerationSettings(biome).getCarvers();
                int index = 0;
                for (Holder<WorldCarver> carverHolder : carvers) {
                    WorldCarver carver = carverHolder.value();
                    random.setLargeFeatureSeed(c.rs.seed() + index, sourcePos.x(), sourcePos.z());
                    if (carver.isStartChunk(random)) {
                        CarvingMask kind = carver instanceof CanyonWorldCarver ? canyons : caves;
                        carver.carve(context, random, pos, sourcePos, both(total, kind));
                    }
                    index++;
                }
            }
        }
    }

    static CarvingMask newMask() {
        return new CarvingMask(MASK_MIN_Y, MASK_MAX_Y);
    }

    /** Mask-only score of a chunk: cave cells, canyon cells, deep cells. */
    static long[] score(Ctx c, int cx, int cz) {
        CarvingMask total = newMask();
        CarvingMask caves = newMask();
        CarvingMask canyons = newMask();
        carveSources(c, cx, cz, total, caves, canyons);
        return new long[] {bitCount(bits(caves)), bitCount(bits(canyons)), deepCells(total)};
    }

    /** Chooses the three extra chunks from the scan grid: most canyon cells, most cave cells, most deep cells. */
    static List<int[]> pick(Ctx c) {
        int n = (SCAN_MAX - SCAN_MIN) / SCAN_STEP + 1;
        long[][] scores = new long[n * n][];
        int[][] coords = new int[n * n][];
        int k = 0;
        for (int cx = SCAN_MIN; cx <= SCAN_MAX; cx += SCAN_STEP) {
            for (int cz = SCAN_MIN; cz <= SCAN_MAX; cz += SCAN_STEP) {
                coords[k] = new int[] {cx, cz};
                scores[k] = score(c, cx, cz);
                k++;
            }
        }
        List<int[]> picks = new ArrayList<>();
        List<Integer> taken = new ArrayList<>();
        String[] names = {"canyon", "cave", "deep"};
        int[] keys = {1, 0, 2};
        for (int p = 0; p < 3; p++) {
            int best = argmax(scores, keys[p], taken);
            taken.add(best);
            picks.add(coords[best]);
            System.out.println("pick " + names[p] + " " + coords[best][0] + " " + coords[best][1] + " cave cells " + scores[best][0]
                + " canyon cells " + scores[best][1] + " deep " + scores[best][2]);
        }
        return picks;
    }

    /** Index of the grid entry with the largest score `key` that is not in `taken`. */
    static int argmax(long[][] scores, int key, List<Integer> taken) {
        int best = -1;
        for (int i = 0; i < scores.length; i++) {
            if (taken.contains(i)) continue;
            if (best < 0 || scores[i][key] > scores[best][key]) {
                best = i;
            }
        }
        return best;
    }

    static ProtoChunk fillBiomes(Ctx c, int cx, int cz) {
        ProtoChunk chunk = new ProtoChunk(new ChunkPos(cx, cz), UpgradeData.EMPTY, LevelHeightAccessor.create(MIN_Y, HEIGHT), c.factory, null);
        Climate.Sampler sampler = c.rs.createClimateSampler(SamplerContext.builder().enableCaches().build());
        BiomeResolver inner = biomeSource.createResolverForChunk(sampler, cx * 4, MIN_Y >> 2, cz * 4, 4, HEIGHT >> 2, 4);
        chunk.fillBiomesFromNoise(pure(inner));
        chunk.setPersistedStatus(ChunkStatus.BIOMES);
        return chunk;
    }

    /** Biome, noise, surface and carvers of one chunk. Fills palettes, indices, mask and the topMaterial count. */
    static Object[] runChunk(Ctx c, int cx, int cz) {
        ProtoChunk chunk = fillBiomes(c, cx, cz);
        // Region-style biome source for the surface step: stored biomes of the 3x3 chunks around (cx, cz).
        Map<Long, ProtoChunk> cache = new HashMap<>();
        BiomeResolver region = (qx, qy, qz) -> {
            int ncx = Math.floorDiv(qx, 4);
            int ncz = Math.floorDiv(qz, 4);
            ProtoChunk nc;
            if (ncx == cx && ncz == cz) {
                nc = chunk;
            } else {
                long key = ((long) ncx << 32) ^ (ncz & 0xffffffffL);
                nc = cache.computeIfAbsent(key, k -> fillBiomes(c, ncx, ncz));
            }
            return nc.getNoiseBiome(qx, qy, qz);
        };
        BiomeManager biomeManager = new BiomeManager(region, BiomeManager.obfuscateSeed(SEED));
        NoiseChunk noiseChunk = new NoiseChunk(c.rs, Beardifier.EMPTY, c.settings, c.fluidPicker, Blender.empty(),
            new DensityVolume(16, HEIGHT, 16, cx * 16, MIN_Y, cz * 16));
        try {
            Object materialRule = c.settings.materialRule().value();
            c.doFill.invoke(c.gen, noiseChunk, chunk);
            c.rs.surfaceSystem().buildSurface(c.rs, biomeManager, new WorldGenerationContext(c.gen, chunk.getHeightAccessorForGeneration()),
                chunk, noiseChunk, c.settings.materialRule().value(), null);

            // Surface-stage blocks, for the topMaterial count.
            BlockState[][] before = new BlockState[HEIGHT][256];
            for (int y = 0; y < HEIGHT; y++) {
                for (int z = 0; z < 16; z++) {
                    for (int x = 0; x < 16; x++) {
                        before[y][z * 16 + x] = chunk.getBlockState(new BlockPos(cx * 16 + x, MIN_Y + y, cz * 16 + z));
                    }
                }
            }

            CarvingMask total = newMask();
            CarvingMask caves = newMask();
            CarvingMask canyons = newMask();
            carveSources(c, cx, cz, total, caves, canyons);
            if (!total.isEmpty()) {
                BiomeManager correct = biomeManager.withDifferentSource(pure(biomeSource.createUncachedResolver(c.rs)));
                Function<BlockPos, Holder<Biome>> getter = correct::getBiome;
                applyCarvingMask.invoke(c.gen, chunk, total, c.rs, materialRule, context(c), noiseChunk, getter, null);
            }

            int dirtToGrass = 0;
            int dirtChanged = 0;
            for (int y = 0; y < HEIGHT; y++) {
                for (int z = 0; z < 16; z++) {
                    for (int x = 0; x < 16; x++) {
                        BlockState b = before[y][z * 16 + x];
                        if (b.is(Blocks.DIRT)) {
                            BlockState a = chunk.getBlockState(new BlockPos(cx * 16 + x, MIN_Y + y, cz * 16 + z));
                            if (a != b) {
                                dirtChanged++;
                                if (a.is(Blocks.GRASS_BLOCK) || a.is(Blocks.PODZOL) || a.is(Blocks.MYCELIUM)) dirtToGrass++;
                            }
                        }
                    }
                }
            }

            List<String> palette = new ArrayList<>();
            Map<String, Integer> index = new LinkedHashMap<>();
            int[] blocks = new int[HEIGHT * 256];
            for (int y = MIN_Y; y < MIN_Y + HEIGHT; y++) {
                for (int z = 0; z < 16; z++) {
                    for (int x = 0; x < 16; x++) {
                        String name = blockName(chunk.getBlockState(new BlockPos(cx * 16 + x, y, cz * 16 + z)));
                        Integer i = index.get(name);
                        if (i == null) {
                            i = index.size();
                            index.put(name, i);
                        }
                        blocks[((y - MIN_Y) * 16 + z) * 16 + x] = i;
                    }
                }
            }
            palette.addAll(index.keySet());
            long[] words = bits(total).toLongArray();
            return new Object[] {palette, blocks, words, dirtToGrass, dirtChanged, bitCount(bits(caves)), bitCount(bits(canyons)), deepCells(total)};
        } catch (ReflectiveOperationException e) {
            throw new RuntimeException(e);
        } finally {
            noiseChunk.close();
        }
    }

    static void writeStrings(DataOutputStream dos, List<String> strings) throws IOException {
        dos.writeInt(strings.size());
        for (String s : strings) {
            byte[] b = s.getBytes(StandardCharsets.UTF_8);
            dos.writeShort(b.length);
            dos.write(b);
        }
    }

    public static void main(String[] args) throws Exception {
        Path outDir = Path.of(args[0]);
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();

        HolderLookup.Provider registries = VanillaRegistries.createWorldLookup();
        // The registries of this harness carry no block tags, and the registry is frozen. applyCarvingMask skips
        // `#minecraft:uncarvable` (data/minecraft/tags/block/uncarvable.json: bedrock), so the bedrock holder
        // gets that tag, as the server's tag loader would give it (Holder.Reference.bindTags, package-private).
        Holder.Reference<Block> bedrock = (Holder.Reference<Block>) BuiltInRegistries.BLOCK.wrapAsHolder(Blocks.BEDROCK);
        Method bindTags = Holder.Reference.class.getDeclaredMethod("bindTags", java.util.Collection.class);
        bindTags.setAccessible(true);
        bindTags.invoke(bedrock, List.of(BlockTags.UNCARVABLE));
        if (!bedrock.is(BlockTags.UNCARVABLE)) throw new IllegalStateException("uncarvable tag not bound");
        HolderLookup.RegistryLookup<NormalNoise> noises = registries.lookupOrThrow(Registries.NOISE);
        Holder<NoiseGeneratorSettings> settingsHolder = registries.lookupOrThrow(Registries.NOISE_SETTINGS)
            .getOrThrow(NoiseGeneratorSettings.OVERWORLD);
        NoiseGeneratorSettings settings = settingsHolder.value();
        Holder<net.minecraft.world.level.biome.MultiNoiseBiomeSourceParameterList> preset =
            registries.lookupOrThrow(Registries.MULTI_NOISE_BIOME_SOURCE_PARAMETER_LIST).getOrThrow(MultiNoiseBiomeSourceParameterLists.OVERWORLD);
        biomeSource = MultiNoiseBiomeSource.createFromPreset(preset);

        parametersMethod = MultiNoiseBiomeSource.class.getDeclaredMethod("parameters");
        parametersMethod.setAccessible(true);
        Class<?> plClass = Class.forName("net.minecraft.world.level.biome.Climate$ParameterList");
        indexField = plClass.getDeclaredField("index");
        indexField.setAccessible(true);
        Class<?> rtClass = Class.forName("net.minecraft.world.level.biome.Climate$RTree");
        lastResultField = rtClass.getDeclaredField("lastResult");
        lastResultField.setAccessible(true);
        maskField = CarvingMask.class.getDeclaredField("mask");
        maskField.setAccessible(true);

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
        applyCarvingMask = null;
        for (Method m : NoiseBasedChunkGenerator.class.getDeclaredMethods()) {
            if (m.getName().equals("applyCarvingMask")) applyCarvingMask = m;
        }
        applyCarvingMask.setAccessible(true);

        // Carver names of every overworld biome, from the live registry.
        List<String> biomeLines = new ArrayList<>();
        for (Holder<Biome> h : registries.lookupOrThrow(Registries.BIOME).listElements().toList()) {
            List<String> names = new ArrayList<>();
            for (Holder<WorldCarver> ch : c.gen.getBiomeGenerationSettings(h).getCarvers()) {
                names.add(ch.unwrapKey().orElseThrow().identifier().getPath());
            }
            biomeLines.add(biomeName(h) + " " + String.join(",", names));
        }
        try (Writer w = new OutputStreamWriter(new FileOutputStream(outDir.resolve("carver_biomes.txt").toFile()), StandardCharsets.UTF_8)) {
            for (String line : biomeLines) w.write(line + "\n");
        }

        try (DataOutputStream sin = new DataOutputStream(new BufferedOutputStream(new FileOutputStream(outDir.resolve("carver_sin_table.bin").toFile())))) {
            Field sinField = Mth.class.getDeclaredField("SIN");
            sinField.setAccessible(true);
            float[] table = (float[]) sinField.get(null);
            for (float v : table) sin.writeInt(Float.floatToRawIntBits(v));
        }

        List<int[]> chunks = new ArrayList<>();
        for (int[] f : FIXED) chunks.add(f);
        List<int[]> picks = pick(c);
        chunks.addAll(picks);
        // topMaterial chunks: the full pipeline over the scan grid, the chunks with the most dirt changed
        // by carving and the most dirt that became a grass-type block.
        List<int[]> dirtCoords = new ArrayList<>();
        long[] dirtScore = new long[0];
        int grid = (SCAN_MAX - SCAN_MIN) / SCAN_STEP + 1;
        long[][] full = new long[grid * grid][];
        int n = 0;
        for (int cx = SCAN_MIN; cx <= SCAN_MAX; cx += SCAN_STEP) {
            for (int cz = SCAN_MIN; cz <= SCAN_MAX; cz += SCAN_STEP) {
                Object[] out = runChunk(c, cx, cz);
                full[n] = new long[] {(Integer) out[3], (Integer) out[4]};
                dirtCoords.add(new int[] {cx, cz});
                n++;
            }
        }
        List<Integer> taken = new ArrayList<>();
        for (int p = 0; p < 2; p++) {
            int best = -1;
            for (int i = 0; i < full.length; i++) {
                if (taken.contains(i)) continue;
                long key = p == 0 ? full[i][0] : full[i][1];
                if (best < 0 || key > (p == 0 ? full[best][0] : full[best][1])) best = i;
            }
            taken.add(best);
            int[] chosen = dirtCoords.get(best);
            System.out.println("pick dirt " + (p == 0 ? "to grass" : "changed") + " " + chosen[0] + " " + chosen[1] + " dirt->grass " + full[best][0]
                + " dirt changed " + full[best][1]);
            chunks.add(chosen);
        }

        List<Object[]> results = new ArrayList<>();
        for (int[] cc : chunks) {
            Object[] out = runChunk(c, cc[0], cc[1]);
            results.add(out);
            System.out.println("chunk " + cc[0] + " " + cc[1] + ": cave cells " + out[5] + ", canyon cells " + out[6]
                + ", deep cells " + out[7] + ", dirt->grass " + out[3] + ", dirt changed " + out[4]);
        }

        try (DataOutputStream dos = new DataOutputStream(new BufferedOutputStream(new FileOutputStream(outDir.resolve("carver_golden.bin").toFile())))) {
            dos.writeInt(chunks.size());
            for (int k = 0; k < chunks.size(); k++) {
                Object[] r = results.get(k);
                dos.writeInt(chunks.get(k)[0]);
                dos.writeInt(chunks.get(k)[1]);
                @SuppressWarnings("unchecked")
                List<String> palette = (List<String>) r[0];
                writeStrings(dos, palette);
                int[] blocks = (int[]) r[1];
                dos.writeInt(blocks.length);
                for (int v : blocks) dos.writeShort(v);
                long[] words = (long[]) r[2];
                dos.writeInt(MASK_MIN_Y);
                dos.writeInt(MASK_MAX_Y);
                dos.writeInt(words.length);
                for (long w : words) dos.writeLong(w);
                dos.writeInt((Integer) r[3]);
            }
        }
        System.out.println("wrote " + outDir);
    }
}
