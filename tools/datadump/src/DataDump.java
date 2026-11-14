import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonNull;
import com.google.gson.JsonObject;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.Enumeration;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.Registry;
import net.minecraft.core.component.DataComponentInitializers;
import net.minecraft.core.component.DataComponents;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.data.registries.VanillaRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.food.FoodProperties;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.Item;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.material.LavaFluid;
import net.minecraft.world.level.material.WaterFluid;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.shapes.VoxelShape;

/**
 * Bootstraps vanilla server 26.3 (Mojang names) and dumps block states, items,
 * entity types and the synchronized registry entry order to JSON.
 *
 * Usage: java -cp ... DataDump &lt;serverJar&gt; &lt;outDir&gt;
 */
public class DataDump {

    private static final Gson GSON = new GsonBuilder().setPrettyPrinting().serializeNulls().disableHtmlEscaping().create();

    /** Sort order of MultiPackResourceManager.listResources: Identifier.compareTo (path, then namespace). */
    private static final Comparator<Identifier> ENTRY_ORDER = Comparator.comparing(Identifier::getPath).thenComparing(Identifier::getNamespace);

    public static void main(String[] args) throws Exception {
        if (args.length != 2) {
            System.err.println("usage: DataDump <serverJar> <outDir>");
            System.exit(2);
        }
        Path serverJar = Paths.get(args[0]);
        Path outDir = Paths.get(args[1]);
        Files.createDirectories(outDir);

        // Same sequence as net.minecraft.data.DataGenerator (static init, which Main triggers).
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        log("bootstrapped, version " + SharedConstants.getCurrentVersion().name());

        // Item/block component maps are only bound when the server applies the reloadable
        // registry lookup (ReloadableServerResources.apply). Same steps here, without a world.
        HolderLookup.Provider worldRegistries = VanillaRegistries.createWorldLookup();
        HolderLookup.Provider reloadableRegistries = VanillaRegistries.createReloadableLookup(worldRegistries);
        for (DataComponentInitializers.PendingComponents<?> pending : BuiltInRegistries.DATA_COMPONENT_INITIALIZERS.build(reloadableRegistries)) {
            pending.apply();
        }
        log("data component maps bound");

        JsonArray blocks = dumpBlockStates();
        write(outDir.resolve("blocks_ext.json"), blocks);
        log("blocks_ext.json: " + blocks.size() + " block state slots");

        JsonArray items = dumpItems();
        write(outDir.resolve("items.json"), items);
        log("items.json: " + items.size() + " item slots");

        JsonArray entities = dumpEntities();
        write(outDir.resolve("entities.json"), entities);
        log("entities.json: " + entities.size() + " entity type slots");

        JsonArray sync = dumpSynchronizedRegistries(serverJar);
        write(outDir.resolve("synchronized_registries.json"), sync);
        int total = 0;
        for (var e : sync) {
            total += e.getAsJsonObject().getAsJsonArray("entries").size();
        }
        log("synchronized_registries.json: " + sync.size() + " registries, " + total + " entries");
    }

    // ---------------------------------------------------------------- blocks

    private static JsonArray dumpBlockStates() {
        TreeMap<Integer, JsonObject> byId = new TreeMap<>();
        for (BlockState state : Block.BLOCK_STATE_REGISTRY) {
            int id = Block.BLOCK_STATE_REGISTRY.getId(state);
            byId.put(id, blockStateToJson(id, state));
        }
        return denseArray(byId);
    }

    private static JsonObject blockStateToJson(int id, BlockState state) {
        Block block = state.getBlock();
        JsonObject o = new JsonObject();
        o.addProperty("id", id);
        o.addProperty("block", BuiltInRegistries.BLOCK.getKey(block).toString());
        o.addProperty("air", state.isAir());
        o.addProperty("opaque", state.canOcclude());
        o.addProperty("light", state.getLightEmission());
        // BlockStateBase.getLightDampening(): no-arg, 0..15.
        o.addProperty("lightBlock", state.getLightDampening());
        // BlockStateBase.isSolid(). blocksMotion() does not exist in 26.3 (checked with javap).
        o.addProperty("solid", state.isSolid());

        JsonArray collision = new JsonArray();
        VoxelShape shape = state.getCollisionShape(EmptyBlockGetter.INSTANCE, BlockPos.ZERO);
        for (AABB box : shape.toAabbs()) {
            JsonArray b = new JsonArray();
            b.add(box.minX);
            b.add(box.minY);
            b.add(box.minZ);
            b.add(box.maxX);
            b.add(box.maxY);
            b.add(box.maxZ);
            collision.add(b);
        }
        o.add("collision", collision);

        o.addProperty("hardness", state.getDestroySpeed(EmptyBlockGetter.INSTANCE, BlockPos.ZERO));
        o.addProperty("resistance", block.getExplosionResistance());
        o.addProperty("toolRequired", state.requiresCorrectToolForDrops());
        o.addProperty("replaceable", state.canBeReplaced());

        FluidState fs = state.getFluidState();
        String fluid;
        if (fs.isEmpty()) {
            fluid = "none";
        } else if (fs.getType() instanceof WaterFluid) {
            fluid = "water";
        } else if (fs.getType() instanceof LavaFluid) {
            fluid = "lava";
        } else {
            fluid = "other";
        }
        o.addProperty("fluid", fluid);
        o.addProperty("fluidLevel", fs.getAmount());
        o.addProperty("fluidSource", fs.isSource());

        o.addProperty("randomTicks", state.isRandomlyTicking());
        o.addProperty("item", BuiltInRegistries.ITEM.getKey(block.asItem()).toString());
        return o;
    }

    // ---------------------------------------------------------------- items

    private static JsonArray dumpItems() {
        TreeMap<Integer, JsonObject> byId = new TreeMap<>();
        for (Item item : BuiltInRegistries.ITEM) {
            byId.put(BuiltInRegistries.ITEM.getId(item), itemToJson(item));
        }
        return denseArray(byId);
    }

    private static JsonObject itemToJson(Item item) {
        JsonObject o = new JsonObject();
        o.addProperty("id", BuiltInRegistries.ITEM.getId(item));
        o.addProperty("name", BuiltInRegistries.ITEM.getKey(item).toString());
        o.addProperty("maxStack", item.components().getOrDefault(DataComponents.MAX_STACK_SIZE, 1));
        o.addProperty("maxDamage", item.components().getOrDefault(DataComponents.MAX_DAMAGE, 0));
        if (item instanceof BlockItem blockItem) {
            o.addProperty("block", BuiltInRegistries.BLOCK.getKey(blockItem.getBlock()).toString());
        } else {
            o.add("block", JsonNull.INSTANCE);
        }
        FoodProperties food = item.components().get(DataComponents.FOOD);
        if (food != null) {
            JsonObject f = new JsonObject();
            f.addProperty("nutrition", food.nutrition());
            f.addProperty("saturation", food.saturation());
            f.addProperty("canAlwaysEat", food.canAlwaysEat());
            o.add("food", f);
        } else {
            o.add("food", JsonNull.INSTANCE);
        }
        return o;
    }

    // ------------------------------------------------------------- entities

    private static JsonArray dumpEntities() {
        TreeMap<Integer, JsonObject> byId = new TreeMap<>();
        for (EntityType<?> type : BuiltInRegistries.ENTITY_TYPE) {
            byId.put(BuiltInRegistries.ENTITY_TYPE.getId(type), entityToJson(type));
        }
        return denseArray(byId);
    }

    private static JsonObject entityToJson(EntityType<?> type) {
        JsonObject o = new JsonObject();
        o.addProperty("id", BuiltInRegistries.ENTITY_TYPE.getId(type));
        o.addProperty("name", BuiltInRegistries.ENTITY_TYPE.getKey(type).toString());
        o.addProperty("width", type.getWidth());
        o.addProperty("height", type.getHeight());
        // EntityDimensions.eyeHeight() is public; EntityType.getDimensions() is public.
        o.addProperty("eyeHeight", type.getDimensions().eyeHeight());
        return o;
    }

    // ------------------------------------------------- synchronized registries

    /**
     * Mirrors RegistrySynchronization.packRegistries: registries go out in
     * RegistryDataLoader.SYNCHRONIZED_REGISTRIES order. Entries of a datapack registry
     * come in listing order of MultiPackResourceManager (sorted by Identifier).
     * The list is read from the vanilla data inside the server jar, not from a live world.
     */
    private static JsonArray dumpSynchronizedRegistries(Path serverJar) throws IOException {
        List<String> vanillaJsonPaths = listVanillaJsonPaths(serverJar);

        JsonArray result = new JsonArray();
        for (RegistryDataLoader.RegistryData<?> data : RegistryDataLoader.SYNCHRONIZED_REGISTRIES) {
            ResourceKey<?> key = data.key();
            @SuppressWarnings("unchecked")
            String dir = Registries.elementsDirPath((ResourceKey<? extends Registry<?>>) key);
            String prefix = dir + "/";

            List<Identifier> ids = new ArrayList<>();
            for (String path : vanillaJsonPaths) {
                if (path.startsWith(prefix)) {
                    ids.add(Identifier.fromNamespaceAndPath("minecraft", path.substring(prefix.length())));
                }
            }
            ids.sort(ENTRY_ORDER);

            JsonObject entry = new JsonObject();
            entry.addProperty("registry", key.identifier().toString());
            entry.addProperty("dir", dir);
            JsonArray names = new JsonArray();
            for (Identifier id : ids) {
                names.add(id.toString());
            }
            entry.add("entries", names);
            if (ids.isEmpty()) {
                log("WARN: no jar files for synchronized registry " + key.identifier() + " (dir " + dir + ")");
            }
            result.add(entry);
        }
        return result;
    }

    /** Relative paths (without data/minecraft/ prefix and .json suffix) of every vanilla data json in the jar. */
    private static List<String> listVanillaJsonPaths(Path serverJar) throws IOException {
        List<String> out = new ArrayList<>();
        try (ZipFile zip = new ZipFile(serverJar.toFile())) {
            Enumeration<? extends ZipEntry> en = zip.entries();
            while (en.hasMoreElements()) {
                ZipEntry ze = en.nextElement();
                String name = ze.getName();
                if (ze.isDirectory() || !name.startsWith("data/minecraft/") || !name.endsWith(".json")) {
                    continue;
                }
                out.add(name.substring("data/minecraft/".length(), name.length() - ".json".length()));
            }
        }
        return out;
    }

    // ----------------------------------------------------------------- io

    /** Array where index equals the registry id; gaps become null. */
    private static JsonArray denseArray(TreeMap<Integer, JsonObject> byId) {
        JsonArray arr = new JsonArray();
        int expected = 0;
        for (Map.Entry<Integer, JsonObject> e : byId.entrySet()) {
            while (expected < e.getKey()) {
                arr.add(JsonNull.INSTANCE);
                expected++;
            }
            arr.add(e.getValue());
            expected++;
        }
        return arr;
    }

    private static void write(Path file, JsonArray arr) throws IOException {
        Files.writeString(file, GSON.toJson(arr) + "\n", StandardCharsets.UTF_8);
    }

    private static void log(String msg) {
        Bootstrap.realStdoutPrintln("[datadump] " + msg);
    }
}
