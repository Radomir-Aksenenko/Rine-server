package org.bukkit.command;

import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.Locale;
import java.util.Map;
import java.util.Set;

/** A command declared in plugin.yml. Created by the host, looked up by plugins via getCommand. */
public final class PluginCommand extends Command {
    private static final Map<String, PluginCommand> REGISTRY = new LinkedHashMap<>();

    private CommandExecutor executor;

    private PluginCommand(String name) {
        super(name);
    }

    /** Internal: declares a command word (from plugin.yml) and returns its object. */
    public static synchronized PluginCommand declare(String name) {
        String key = name.toLowerCase(Locale.ROOT);
        return REGISTRY.computeIfAbsent(key, PluginCommand::new);
    }

    /** Returns the declared command, or null. */
    public static synchronized PluginCommand find(String name) {
        return REGISTRY.get(name.toLowerCase(Locale.ROOT));
    }

    /** Internal: all declared command words. */
    public static synchronized Set<String> names() {
        return Collections.unmodifiableSet(new java.util.LinkedHashSet<>(REGISTRY.keySet()));
    }

    public void setExecutor(CommandExecutor executor) {
        this.executor = executor;
    }

    public CommandExecutor getExecutor() {
        return executor;
    }
}
