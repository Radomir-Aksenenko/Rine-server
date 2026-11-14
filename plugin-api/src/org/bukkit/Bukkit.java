package org.bukkit;

import org.bukkit.entity.Player;

/** Static entry points, as in Bukkit. */
public final class Bukkit {
    private Bukkit() {
    }

    public static Server getServer() {
        return Server.INSTANCE;
    }

    public static int broadcastMessage(String message) {
        return getServer().broadcastMessage(message);
    }

    public static Player getPlayer(String name) {
        return getServer().getPlayer(name);
    }
}
