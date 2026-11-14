package org.bukkit;

import java.util.Locale;

/**
 * A handful of common materials. Use {@link #fromKey(String)} for anything else: unknown keys
 * map to {@link #UNKNOWN} instead of throwing.
 */
public enum Material {
    UNKNOWN,
    AIR,
    STONE,
    COBBLESTONE,
    DIRT,
    GRASS_BLOCK,
    SAND,
    GRAVEL,
    BEDROCK,
    OAK_LOG,
    OAK_PLANKS,
    COAL_ORE,
    IRON_ORE,
    GOLD_ORE,
    DIAMOND_ORE,
    DEEPSLATE_DIAMOND_ORE,
    OBSIDIAN,
    WATER,
    LAVA,
    TORCH,
    CHEST,
    DIAMOND,
    DIAMOND_BLOCK,
    IRON_INGOT,
    GOLD_INGOT,
    APPLE,
    BREAD,
    STICK;

    /** Lower-case key without namespace, for example "diamond_ore". */
    public String getKey() {
        return name().toLowerCase(Locale.ROOT);
    }

    /** Accepts "minecraft:diamond_ore" or "DIAMOND_ORE". Returns UNKNOWN on a miss. */
    public static Material fromKey(String key) {
        Material m = matchMaterial(key);
        return m == null ? UNKNOWN : m;
    }

    /** Like fromKey, but returns null on a miss (Bukkit's matchMaterial contract). */
    public static Material matchMaterial(String key) {
        if (key == null) {
            return null;
        }
        String s = key.strip();
        if (s.regionMatches(true, 0, "minecraft:", 0, 10)) {
            s = s.substring(10);
        }
        s = s.toUpperCase(Locale.ROOT);
        for (Material m : values()) {
            if (m.name().equals(s) && m != UNKNOWN) {
                return m;
            }
        }
        return null;
    }
}
