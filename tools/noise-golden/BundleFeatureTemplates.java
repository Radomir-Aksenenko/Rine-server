import com.google.gson.Gson;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.nbt.NbtAccounter;
import net.minecraft.nbt.NbtIo;
import net.minecraft.nbt.NbtUtils;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Rotation;

/** Arguments: vanilla structure directory, processor_list directory, output JSON. */
public class BundleFeatureTemplates {
    private record Entry(int group, int x, int y, int z, int state) {}
    private record Template(int[] size, List<List<String>> palette, List<int[]> blocks) {}
    private record Bundle(Map<String, Object> templates, Map<String, Object> processors) {}

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        var gson = new Gson();
        var root = Path.of(args[0]);
        Map<String, Object> templates = new LinkedHashMap<>();
        for (String directory : List.of("fossil", "desert_well", "spring")) {
            try (var files = Files.list(root.resolve(directory))) {
                for (var path : files.filter(p -> p.toString().endsWith(".nbt")).sorted().toList()) {
                    var nbt = NbtIo.readCompressed(path, NbtAccounter.unlimitedHeap());
                    if (nbt.contains("palettes") || !nbt.getListOrEmpty("entities").isEmpty()) {
                        throw new IllegalArgumentException("Unexpected palettes or entities in " + path);
                    }
                    var vec = nbt.getListOrEmpty("size");
                    int[] size = {vec.getIntOr(0, 0), vec.getIntOr(1, 0), vec.getIntOr(2, 0)};
                    var states = nbt.getListOrEmpty("palette").compoundStream()
                            .map(tag -> NbtUtils.readBlockState(BuiltInRegistries.BLOCK, tag)).toList();
                    var palette = states.stream().map(state -> Arrays.stream(Rotation.values())
                            .map(rot -> stateName(state.rotate(rot))).toList()).toList();
                    var entries = new ArrayList<Entry>();
                    for (var tag : nbt.getListOrEmpty("blocks").compoundStream().toList()) {
                        int idx = tag.getIntOr("state", -1);
                        var state = states.get(idx);
                        boolean entity = tag.contains("nbt");
                        if (entity) {
                            var data = tag.getCompoundOrEmpty("nbt");
                            if (!data.getStringOr("id", "").equals("minecraft:brushable_block")
                                    || !java.util.Set.of("id", "components").containsAll(data.keySet())
                                    || !data.getCompoundOrEmpty("components").isEmpty()) {
                                throw new IllegalArgumentException("Unexpected block entity in " + path + ": " + data);
                            }
                        }
                        int group = entity ? 2 : !state.getBlock().hasDynamicShape()
                                && state.isCollisionShapeFullBlock(EmptyBlockGetter.INSTANCE, BlockPos.ZERO) ? 0 : 1;
                        var pos = tag.getListOrEmpty("pos");
                        entries.add(new Entry(group, pos.getIntOr(0, 0), pos.getIntOr(1, 0), pos.getIntOr(2, 0), idx));
                    }
                    entries.sort(Comparator.comparingInt(Entry::group).thenComparingInt(Entry::y)
                            .thenComparingInt(Entry::x).thenComparingInt(Entry::z));
                    var blocks = entries.stream().map(e -> new int[]{e.x, e.y, e.z, e.state, e.group == 2 ? 1 : 0}).toList();
                    String name = path.getFileName().toString().replaceFirst("\\.nbt$", "");
                    templates.put("minecraft:" + directory + "/" + name, new Template(size, palette, blocks));
                }
            }
        }
        Map<String, Object> processors = new LinkedHashMap<>();
        for (String name : List.of("fossil_rot", "fossil_coal", "fossil_diamonds")) {
            processors.put("minecraft:" + name, gson.fromJson(Files.readString(Path.of(args[1], name + ".json")), Object.class));
        }
        Files.writeString(Path.of(args[2]), gson.toJson(new Bundle(templates, processors)));
    }

    private static String stateName(net.minecraft.world.level.block.state.BlockState state) {
        String name = BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString();
        var properties = NbtUtils.writeBlockState(state).getCompoundOrEmpty("properties");
        var keys = properties.keySet().stream().sorted().toList();
        if (keys.isEmpty()) return name;
        return name + "[" + String.join(",", keys.stream().map(k -> k + "=" + properties.getStringOr(k, "")).toList()) + "]";
    }
}
