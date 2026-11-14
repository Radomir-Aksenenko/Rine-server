package org.bukkit.plugin;

import java.util.logging.Logger;

import org.bukkit.Server;

public interface Plugin {
    String getName();

    Server getServer();

    Logger getLogger();
}
