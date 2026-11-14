package org.bukkit.block;

import org.bukkit.Location;
import org.bukkit.Material;

/** A block position with its id. Immutable snapshot. */
public final class Block {
    private final int x;
    private final int y;
    private final int z;
    private final String key;

    public Block(int x, int y, int z, String key) {
        this.x = x;
        this.y = y;
        this.z = z;
        this.key = key;
    }

    public int getX() {
        return x;
    }

    public int getY() {
        return y;
    }

    public int getZ() {
        return z;
    }

    public Material getType() {
        return Material.fromKey(key);
    }

    public Location getLocation() {
        return new Location(x, y, z);
    }

    @Override
    public String toString() {
        return "Block{" + key + " at " + x + "," + y + "," + z + "}";
    }
}
