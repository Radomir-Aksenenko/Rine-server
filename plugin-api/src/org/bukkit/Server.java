package org.bukkit;

import java.util.ArrayList;
import java.util.Collection;
import java.util.List;

import org.bukkit.entity.Player;
import org.bukkit.plugin.PluginManager;

/** Bukkit-compatible facade over rine.api.Server. */
public final class Server {
    public static final Server INSTANCE = new Server();

    private final PluginManager pluginManager = new PluginManager();

    private Server() {
    }

    /** Sends a message to every online player. Returns the number of recipients. */
    public int broadcastMessage(String message) {
        rine.api.Server core = rine.api.Server.get();
        core.broadcast(message);
        return core.getOnlinePlayers().size();
    }

    public Collection<Player> getOnlinePlayers() {
        List<Player> players = new ArrayList<>();
        for (String name : rine.api.Server.get().getOnlinePlayers()) {
            players.add(new Player(name));
        }
        return players;
    }

    /** Returns the online player with this name, or null. */
    public Player getPlayer(String name) {
        return rine.api.Server.get().getOnlinePlayers().contains(name) ? new Player(name) : null;
    }

    public PluginManager getPluginManager() {
        return pluginManager;
    }
}
