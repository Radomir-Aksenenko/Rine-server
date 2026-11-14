import java.io.BufferedOutputStream;
import java.io.DataOutputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.OutputStreamWriter;
import java.io.Writer;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import net.minecraft.SharedConstants;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.registries.Registries;
import net.minecraft.server.Bootstrap;
import net.minecraft.data.registries.VanillaRegistries;
import net.minecraft.world.level.levelgen.Beardifier;
import net.minecraft.world.level.levelgen.NoiseChunk;
import net.minecraft.world.level.levelgen.NoiseGeneratorSettings;
import net.minecraft.world.level.levelgen.NoiseRouter;
import net.minecraft.world.level.levelgen.RandomState;
import net.minecraft.world.level.levelgen.blending.Blender;
import net.minecraft.world.level.levelgen.densityfunction.DensityFunction;
import net.minecraft.world.level.levelgen.densityfunction.DensityVolume;
import net.minecraft.world.level.levelgen.densityfunction.ScopedDensityBuffer;

/**
 * Density golden generator for rine-worldgen phase 2.
 *
 * Output (both written into crates/rine-worldgen/tests/):
 *   density_points.txt : "router.<field>.<k> <float raw bits as int>" for 7 router fields at POINTS.
 *                        Values come from RandomState.sampleBlockValueUncached (pure sampleValue path).
 *   density_chunks.bin : for each chunk in CHUNKS, 16*16*384 big-endian int32 float raw bits of the
 *                        final_density volume, looping x (0..15), then z (0..15), then y (0..383, y = -64 + i).
 *                        This is exactly what NoiseBasedChunkGenerator.doFill reads (volume.sampleVolume).
 *
 * The POINTS and CHUNKS tables MUST stay identical to crates/rine-worldgen/tests/density.rs.
 */
public class DensityGolden {
    static final long SEED = 1592598566L;

    static final int[][] POINTS = {
        {0, 0, 0}, {1, 64, 1}, {-100, -64, 200}, {1000, -300, -1000}, {123456, 80, -654321},
        {-30000000, 0, 30000000}, {5, 0, 7}, {-17, 3, 40}, {16, 1, -16}, {-1, -1, -1},
        {300, 120, 300}, {-2000, 15, 4000}, {9, 0, 0}, {0, 0, 9}, {-4, 200, 5},
        {77, -20, -77}, {2048, 63, -2048}, {1, 2, 3}, {-65536, 17, 65535}, {31, 0, -31},
        {8, 63, 8}, {8, 64, 8}, {15, -64, 15}, {16, -63, 16}, {-16, 320, -16},
        {100, -30, 100}, {100, 0, 100}, {100, 30, 100}, {100, 60, 100}, {100, 90, 100},
        {-200, 5, 37}, {-201, 6, 38}, {512, 128, -512}, {513, 129, -513}, {4, 5, 6},
        {7, 8, 9}, {-7, -8, -9}, {12, 40, -44}, {88, 73, 19}, {-5000, 100, 5000}, {3, 255, 3}
    };

    static final String[] FIELDS = {"temperature", "vegetation", "continents", "erosion", "depth", "ridges", "final_density", "chunk_surface_level"};

    static final int[][] CHUNKS = {{0, 0}, {5, -3}, {-17, 40}};

    static final int MIN_Y = -64;
    static final int HEIGHT = 384;

    public static void main(String[] args) throws IOException {
        Path outDir = Path.of(args[0]);
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();

        HolderLookup.Provider registries = VanillaRegistries.createWorldLookup();
        HolderLookup.RegistryLookup<net.minecraft.world.level.levelgen.synth.NormalNoise> noises =
            registries.lookupOrThrow(Registries.NOISE);
        NoiseGeneratorSettings settings = registries.lookupOrThrow(Registries.NOISE_SETTINGS)
            .getOrThrow(NoiseGeneratorSettings.OVERWORLD).value();
        NoiseRouter router = settings.noiseRouter();

        RandomState rs = RandomState.create(noises, SEED, settings);

        DensityFunction[] fns = {
            router.temperature(), router.vegetation(), router.continents(), router.erosion(),
            router.depth(), router.ridges(), router.finalDensity(), router.chunkSurfaceLevel()
        };

        try (Writer w = new OutputStreamWriter(new FileOutputStream(outDir.resolve("density_points.txt").toFile()), StandardCharsets.UTF_8)) {
            for (int f = 0; f < fns.length; f++) {
                for (int k = 0; k < POINTS.length; k++) {
                    int[] p = POINTS[k];
                    float v = rs.sampleBlockValueUncached(fns[f], p[0], p[1], p[2]);
                    w.write("router." + FIELDS[f] + "." + k + " " + Float.floatToRawIntBits(v) + "\n");
                }
            }
        }

        try (DataOutputStream out = new DataOutputStream(new BufferedOutputStream(
            new FileOutputStream(outDir.resolve("density_chunks.bin").toFile())))) {
            for (int[] c : CHUNKS) {
                DensityVolume volume = new DensityVolume(16, HEIGHT, 16, c[0] * 16, MIN_Y, c[1] * 16);
                NoiseChunk nc = new NoiseChunk(rs, Beardifier.EMPTY, settings, (x, y, z) -> null, Blender.empty(), volume);
                net.minecraft.world.level.levelgen.densityfunction.DensitySampler.Bound bound =
                    nc.cachingSamplers().get(router.finalDensity());
                try (ScopedDensityBuffer buf = bound.sampleVolume(nc.volume())) {
                    for (int x = 0; x < 16; x++) {
                        for (int z = 0; z < 16; z++) {
                            for (int y = 0; y < HEIGHT; y++) {
                                float v = buf.get(volume.indexUnchecked(x, y, z));
                                out.writeInt(Float.floatToRawIntBits(v));
                            }
                        }
                    }
                }
                nc.close();
            }
        }
        System.out.println("wrote " + outDir);
    }

}
