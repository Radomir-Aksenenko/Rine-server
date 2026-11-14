package org.bukkit.plugin.java;

import java.util.logging.Logger;

import org.bukkit.Server;
import org.bukkit.command.PluginCommand;
import org.bukkit.plugin.Plugin;
import rine.api.Lifecycle;

/** Bukkit-style base class. Subclasses override onEnable and onDisable. */
public abstract class JavaPlugin implements Plugin, Lifecycle {
    private String name = "unnamed";

    public JavaPlugin() {
    }

    public abstract void onEnable();

    public void onDisable() {
    }

    @Override
    public final String getName() {
        return name;
    }

    @Override
    public final Server getServer() {
        return Server.INSTANCE;
    }

    @Override
    public final Logger getLogger() {
        return Logger.getLogger(name);
    }

    /** Returns the command declared in plugin.yml, or null. */
    public final PluginCommand getCommand(String command) {
        return PluginCommand.find(command);
    }

    @Override
    public final void bind(String name, rine.api.Server server) {
        this.name = name;
    }

    @Override
    public final void enable() {
        onEnable();
    }

    @Override
    public final void disable() {
        onDisable();
    }
}
