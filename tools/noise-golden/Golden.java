import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.world.level.levelgen.LegacyRandomSource;
import net.minecraft.world.level.levelgen.PositionalRandomFactory;
import net.minecraft.world.level.levelgen.XoroshiroRandomSource;
import net.minecraft.world.level.levelgen.densityfunction.DensitySampler;
import net.minecraft.world.level.levelgen.synth.BlendedNoise;
import net.minecraft.world.level.levelgen.synth.NormalNoise;
import net.minecraft.world.level.levelgen.synth.Noise;
import net.minecraft.world.level.levelgen.synth.PerlinNoise;
import net.minecraft.world.level.levelgen.synth.SimplexNoise;
import net.minecraft.util.RandomSource;
import net.minecraft.SharedConstants;
import net.minecraft.server.Bootstrap;

/**
 * Golden value generator. Output: one line per value, "label value".
 * Doubles are printed as raw long bits, floats as raw int bits, ints/longs in decimal.
 * The point tables here MUST stay identical to the ones in crates/rine-worldgen/tests/golden.rs.
 */
public class Golden {
    static final StringBuilder OUT = new StringBuilder();

    static void L(String label, long v) {
        OUT.append(label).append(' ').append(v).append('\n');
    }

    static void D(String label, double v) {
        L(label, Double.doubleToRawLongBits(v));
    }

    static void F(String label, float v) {
        L(label, Float.floatToRawIntBits(v));
    }

    static final double[][] P3 = {
        {0.0, 0.0, 0.0},
        {0.5, 0.5, 0.5},
        {1.0, 2.0, 3.0},
        {-1.0, -2.0, -3.0},
        {-0.25, 7.5, -12.125},
        {3.7, -4.2, 5.9},
        {100.0, 64.0, -100.0},
        {100000.0, 100000.0, 100000.0},
        {-100000.0, 37.25, 200000.5},
        {0.001, -0.001, 0.0005},
        {255.9, -255.9, 256.1},
        {-511.5, 511.5, -1023.75},
        {12345.678, -98.765, 4321.0987},
        {-0.9999999, 0.9999999, 1.0000001},
        {2.5, -7.0, -0.5},
        {16.0, -16.0, 8.0},
        {-33.3, 21.7, -9.1},
        {0.333, 0.666, 0.999},
        {-123456.789, -6.5, 654321.25},
        {7.0, 7.0, 7.0},
        {-3.0, 0.0, 3.0},
        {0.1, 0.2, 0.3},
        {1e-9, -1e-9, 5.0},
        {88.8, -44.4, 22.2},
        {-0.5, -0.5, -0.5},
        {1234.5, 0.0, -1234.5},
        {50.25, -150.75, 75.125},
        {3.14159, 2.71828, 1.41421},
        {-2000.0, 1500.0, -1000.0},
        {0.75, 0.25, -0.125},
        // wrap() range: |coord| >= 2^24 takes the modulo branch
        {30000000.0, -50000000.0, 1000000000.0},
        {-30000000.5, 16777216.0, -16777217.25},
        {0.0, 0.0, 0.0}
    };

    static final int[][] BLOCK = {
        {0, 0, 0},
        {1, 64, 1},
        {-100, -64, 200},
        {1000, -300, -1000},
        {123456, 80, -654321},
        {-30000000, 0, 30000000},
        {5, 0, 7},
        {-17, 3, 40},
        {16, 1, -16},
        {-1, -1, -1},
        {300, 120, 300},
        {-2000, 15, 4000},
        {9, 0, 0},
        {0, 0, 9},
        {-4, 200, 5},
        {77, -20, -77},
        {2048, 63, -2048},
        {1, 2, 3},
        {-65536, 17, 65535},
        {31, 0, -31}
    };

    static RandomSource make(String kind, long seed) {
        return kind.equals("xoro") ? new XoroshiroRandomSource(seed) : new LegacyRandomSource(seed);
    }

    static PositionalRandomFactory makeFactory(String kind, long seed) {
        return make(kind, seed).forkPositional();
    }

    public static void main(String[] args) throws IOException {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        long[] seeds = {0L, 1592598566L, -7L};
        String[] kinds = {"xoro", "legacy"};

        for (long seed : seeds) {
            for (String kind : kinds) {
                String p = kind + ".s" + seed;
                RandomSource r;

                r = make(kind, seed);
                for (int i = 0; i < 20; i++) L(p + ".long." + i, r.nextLong());
                r = make(kind, seed);
                for (int i = 0; i < 20; i++) L(p + ".int100." + i, r.nextInt(100));
                r = make(kind, seed);
                for (int i = 0; i < 20; i++) L(p + ".intraw." + i, r.nextInt());
                r = make(kind, seed);
                for (int i = 0; i < 20; i++) L(p + ".int64." + i, r.nextInt(64));
                r = make(kind, seed);
                for (int i = 0; i < 20; i++) L(p + ".int1000." + i, r.nextInt(1000));
                r = make(kind, seed);
                for (int i = 0; i < 20; i++) D(p + ".double." + i, r.nextDouble());
                r = make(kind, seed);
                for (int i = 0; i < 20; i++) F(p + ".float." + i, r.nextFloat());
                r = make(kind, seed);
                for (int i = 0; i < 20; i++) D(p + ".gauss." + i, r.nextGaussian());
                r = make(kind, seed);
                for (int i = 0; i < 20; i++) L(p + ".bool." + i, r.nextBoolean() ? 1 : 0);
                r = make(kind, seed);
                r.consumeCount(5);
                L(p + ".afterconsume", r.nextLong());
                r = make(kind, seed);
                L(p + ".fork.a", r.fork().nextLong());
                L(p + ".fork.b", r.fork().nextLong());

                PositionalRandomFactory f = makeFactory(kind, seed);
                int[][] pts = {
                    {3, -5, 77}, {0, 0, 0}, {-1, -1, -1}, {100000, 64, -100000},
                    {Integer.MAX_VALUE, Integer.MIN_VALUE, 12345}, {-7, 300, -2}, {Integer.MIN_VALUE, -1, Integer.MAX_VALUE}
                };
                for (int k = 0; k < pts.length; k++) {
                    RandomSource rr = f.at(pts[k][0], pts[k][1], pts[k][2]);
                    L(p + ".at." + k + ".long", rr.nextLong());
                    L(p + ".at." + k + ".int100", rr.nextInt(100));
                    D(p + ".at." + k + ".double", rr.nextDouble());
                }
                String[] names = {"minecraft:offset", "minecraft:terrain", "octave_-4", ""};
                for (String name : names) {
                    RandomSource rr = f.fromHashOf(name);
                    L(p + ".hash[" + name + "].a", rr.nextLong());
                    L(p + ".hash[" + name + "].b", rr.nextLong());
                    L(p + ".hash[" + name + "].int100", rr.nextInt(100));
                }
                RandomSource fs = f.fromSeed(0x123456789L);
                L(p + ".fromSeed.a", fs.nextLong());
                L(p + ".fromSeed.b", fs.nextLong());
            }
        }

        // Noise: single PerlinNoise (3D and 2D), per seed and random kind.
        for (long seed : seeds) {
            for (String kind : kinds) {
                String p = kind + ".s" + seed;
                PerlinNoise pn = new PerlinNoise(make(kind, seed));
                for (int k = 0; k < P3.length; k++) {
                    F(p + ".perlin3." + k, pn.get(P3[k][0], P3[k][1], P3[k][2]));
                }
                for (int k = 0; k < P3.length; k++) {
                    F(p + ".perlin2." + k, pn.get(P3[k][0], P3[k][2]));
                }
            }
        }

        // Simplex (2D and 3D).
        for (long seed : seeds) {
            for (String kind : kinds) {
                String p = kind + ".s" + seed;
                SimplexNoise sn = new SimplexNoise(make(kind, seed));
                for (int k = 0; k < P3.length; k++) {
                    F(p + ".simplex3." + k, sn.get(P3[k][0], P3[k][1], P3[k][2]));
                }
                for (int k = 0; k < P3.length; k++) {
                    F(p + ".simplex2." + k, sn.get(P3[k][0], P3[k][1]));
                }
            }
        }

        // NormalNoise via createParity (the builder path) with the non-legacy create(random).
        double[] amps1 = {1.0, 1.0, 0.0, 1.0, 1.0};
        double[] amps2 = {1.0, 1.0, 1.0};
        for (long seed : seeds) {
            for (String kind : kinds) {
                String p = kind + ".s" + seed;
                Noise n1 = NormalNoise.createParity(-9, amps1).create(make(kind, seed));
                for (int k = 0; k < P3.length; k++) {
                    F(p + ".normal1.3." + k, n1.get(P3[k][0], P3[k][1], P3[k][2]));
                }
                for (int k = 0; k < P3.length; k++) {
                    F(p + ".normal1.2." + k, n1.get(P3[k][0], P3[k][2]));
                }
                Noise n2 = NormalNoise.createParity(-7, amps2).create(make(kind, seed));
                for (int k = 0; k < P3.length; k++) {
                    F(p + ".normal2.3." + k, n2.get(P3[k][0], P3[k][1], P3[k][2]));
                }

                if (kind.equals("xoro")) {
                    NormalNoise par = NormalNoise.createParity(-9, amps1);
                    F(p + ".normal1.range.min", par.range().min());
                    F(p + ".normal1.range.max", par.range().max());
                    // General constructor, through the public builder (what the JSON codec produces).
                    for (int c = 0; c < 4; c++) {
                        NormalNoise.Builder b = NormalNoise.builder();
                        if (c == 3) {
                            b.setBaseAmplitude(2.5).setBaseOctave(-3).setOctaveCount(3);
                            b.setAmplitudeModifier(0, 0.0).setAmplitudeModifier(1, 1.0).setAmplitudeModifier(2, 1.0);
                        } else {
                            b.setBaseAmplitude(1.0).setBaseOctave(-6).setOctaveCount(4);
                            b.setAmplitudeModifier(1, 0.5).setAmplitudeModifier(2, 2.0);
                        }
                        if (c == 1) b.setNormalize(false);
                        if (c == 2) b.setLegacyNormalization();
                        NormalNoise nn = b.build();
                        Noise nb = nn.create(make(kind, seed));
                        String q = p + ".normal.c" + c;
                        for (int k = 0; k < P3.length; k++) {
                            F(q + ".3." + k, nb.get(P3[k][0], P3[k][1], P3[k][2]));
                        }
                        for (int k = 0; k < P3.length; k++) {
                            F(q + ".2." + k, nb.get(P3[k][0], P3[k][2]));
                        }
                        F(q + ".range.min", nn.range().min());
                        F(q + ".range.max", nn.range().max());
                    }
                }
            }
        }

        // BlendedNoise, overworld parameters, seeded the way RandomState seeds it:
        // random = XoroshiroRandomSource(seed).forkPositional().fromHashOf("minecraft:terrain")
        for (long seed : seeds) {
            String p = "xoro.s" + seed;
            RandomSource rnd = new XoroshiroRandomSource(seed).forkPositional().fromHashOf(BlendedNoise.NOISE_SEED);
            DensitySampler ds = new BlendedNoise(0.25, 0.125, 80.0, 160.0, 8.0).compileSampler(rnd);
            for (int k = 0; k < BLOCK.length; k++) {
                F(p + ".blended." + k, ds.sampleValue(null, BLOCK[k][0], BLOCK[k][1], BLOCK[k][2]));
            }
        }

        Path out = Path.of(args.length > 0 ? args[0] : "golden.txt");
        Files.writeString(out, OUT.toString(), StandardCharsets.UTF_8);
        System.err.println("wrote " + out.toAbsolutePath() + " lines=" + OUT.toString().split("\n").length);
    }
}
