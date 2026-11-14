package rine.host;

import java.io.IOException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Set;
import java.util.jar.JarEntry;
import java.util.jar.JarFile;

import org.bukkit.command.CommandExecutor;
import org.bukkit.command.PluginCommand;
import org.bukkit.entity.Player;
import org.bukkit.event.player.AsyncPlayerChatEvent;
import org.bukkit.event.player.PlayerJoinEvent;
import org.bukkit.event.player.PlayerQuitEvent;
import org.bukkit.block.Block;
import rine.api.BlockBreakEvent;
import rine.api.BlockPlaceEvent;
import rine.api.ChatEvent;
import rine.api.CommandEvent;
import rine.api.CommandHandler;
import rine.api.EventBus;
import rine.api.JoinEvent;
import rine.api.Lifecycle;
import rine.api.Log;
import rine.api.QuitEvent;
import rine.api.Server;
import rine.api.Wire;

/**
 * Entry point called from Rust over JNI. All methods run on the single plugin thread.
 *
 * Event strings (fields separated by U+0001):
 *   join|name|uuid, quit|name, chat|name|message, command|name|line,
 *   break|name|x|y|z|block, place|name|x|y|z|block
 */
public final class Host {
    /** Sync mask bits. Must match SyncKind on the Rust side. */
    public static final int MASK_CHAT = 1;
    public static final int MASK_BLOCK_BREAK = 2;
    public static final int MASK_BLOCK_PLACE = 4;
    public static final int MASK_COMMAND = 8;

    private record Descriptor(String main, String name, List<String> commands) {
    }

    private record Loaded(String name, Lifecycle plugin, URLClassLoader loader) {
    }

    private static final List<Loaded> PLUGINS = new ArrayList<>();

    private Host() {
    }

    /** Loads and enables every plugin jar. A failing jar is logged and skipped. */
    public static void init(String[] jarPaths) {
        for (String jar : jarPaths) {
            try {
                load(Path.of(jar));
            } catch (Throwable t) {
                Log.error("failed to load " + jar, t);
            }
        }
    }

    private static void load(Path jar) throws Exception {
        Descriptor d = readDescriptor(jar);
        if (d == null) {
            Log.warn("no plugin.yml or rine.yml in " + jar + ", skipped");
            return;
        }
        URLClassLoader loader = new URLClassLoader(new URL[] {jar.toUri().toURL()},
                Host.class.getClassLoader());
        Class<?> mainClass = Class.forName(d.main(), true, loader);
        Object instance = mainClass.getDeclaredConstructor().newInstance();
        if (!(instance instanceof Lifecycle lifecycle)) {
            throw new IllegalStateException(d.main() + " must extend RinePlugin or JavaPlugin");
        }
        for (String cmd : d.commands()) {
            PluginCommand.declare(cmd);
        }
        lifecycle.bind(d.name(), Server.get());
        PLUGINS.add(new Loaded(d.name(), lifecycle, loader));
        try {
            lifecycle.enable();
            Log.info("enabled " + d.name());
        } catch (Throwable t) {
            Log.error("onEnable failed for " + d.name(), t);
        }
    }

    /** Delivers a batch of asynchronous events. Errors are logged per event. */
    public static void dispatch(String[] packed) {
        for (String s : packed) {
            deliver(s);
        }
    }

    /** Delivers one event synchronously. Returns true if a listener cancelled it. */
    public static boolean syncCancel(String packed) {
        return deliver(packed);
    }

    /** Returns queued actions in packed form and clears the queue. */
    public static String[] drainActions() {
        return Server.get().drainActions();
    }

    /** Command words registered by plugins (rine registerCommand and plugin.yml). */
    public static String[] commands() {
        Set<String> names = new LinkedHashSet<>(Server.get().commandNames());
        names.addAll(PluginCommand.names());
        return names.toArray(new String[0]);
    }

    /** Bit set of sync kinds that have at least one listener. */
    public static int syncMask() {
        EventBus bus = EventBus.get();
        int mask = 0;
        if (bus.has(ChatEvent.class) || bus.has(AsyncPlayerChatEvent.class)) {
            mask |= MASK_CHAT;
        }
        if (bus.has(BlockBreakEvent.class) || bus.has(org.bukkit.event.block.BlockBreakEvent.class)) {
            mask |= MASK_BLOCK_BREAK;
        }
        if (bus.has(BlockPlaceEvent.class) || bus.has(org.bukkit.event.block.BlockPlaceEvent.class)) {
            mask |= MASK_BLOCK_PLACE;
        }
        if (bus.has(CommandEvent.class)) {
            mask |= MASK_COMMAND;
        }
        return mask;
    }

    public static void shutdown() {
        for (Loaded l : PLUGINS) {
            try {
                l.plugin().disable();
            } catch (Throwable t) {
                Log.error("onDisable failed for " + l.name(), t);
            }
        }
        PLUGINS.clear();
    }

    // ---- event routing ----

    private static boolean deliver(String packed) {
        try {
            return route(Wire.split(packed));
        } catch (Throwable t) {
            Log.error("cannot handle event " + packed.replace(Wire.SEP, "|"), t);
            return false;
        }
    }

    private static boolean route(String[] f) {
        EventBus bus = EventBus.get();
        switch (f[0]) {
            case "join" -> {
                String name = f[1];
                Server.get().trackJoin(name);
                boolean c = false;
                if (bus.has(JoinEvent.class)) {
                    c |= bus.dispatch(new JoinEvent(name, f.length > 2 ? f[2] : ""));
                }
                if (bus.has(PlayerJoinEvent.class)) {
                    c |= bus.dispatch(new PlayerJoinEvent(new Player(name)));
                }
                return c;
            }
            case "quit" -> {
                String name = f[1];
                boolean c = false;
                if (bus.has(QuitEvent.class)) {
                    c |= bus.dispatch(new QuitEvent(name));
                }
                if (bus.has(PlayerQuitEvent.class)) {
                    c |= bus.dispatch(new PlayerQuitEvent(new Player(name)));
                }
                Server.get().trackQuit(name);
                return c;
            }
            case "chat" -> {
                String name = f[1];
                String msg = f.length > 2 ? f[2] : "";
                boolean c = false;
                if (bus.has(ChatEvent.class)) {
                    c |= bus.dispatch(new ChatEvent(name, msg));
                }
                if (bus.has(AsyncPlayerChatEvent.class)) {
                    c |= bus.dispatch(new AsyncPlayerChatEvent(new Player(name), msg));
                }
                return c;
            }
            case "command" -> {
                String name = f[1];
                String line = f.length > 2 ? f[2] : "";
                boolean c = false;
                if (bus.has(CommandEvent.class)) {
                    c = bus.dispatch(new CommandEvent(name, line));
                }
                if (!c) {
                    routeCommand(name, line);
                }
                return c;
            }
            case "break", "place" -> {
                if (f.length < 6) {
                    return false;
                }
                String name = f[1];
                int x = Integer.parseInt(f[2]);
                int y = Integer.parseInt(f[3]);
                int z = Integer.parseInt(f[4]);
                String block = f[5];
                boolean isBreak = f[0].equals("break");
                boolean c = false;
                if (isBreak) {
                    if (bus.has(BlockBreakEvent.class)) {
                        c |= bus.dispatch(new BlockBreakEvent(name, x, y, z, block));
                    }
                    if (bus.has(org.bukkit.event.block.BlockBreakEvent.class)) {
                        c |= bus.dispatch(new org.bukkit.event.block.BlockBreakEvent(
                                new Player(name), new Block(x, y, z, block)));
                    }
                } else {
                    if (bus.has(BlockPlaceEvent.class)) {
                        c |= bus.dispatch(new BlockPlaceEvent(name, x, y, z, block));
                    }
                    if (bus.has(org.bukkit.event.block.BlockPlaceEvent.class)) {
                        c |= bus.dispatch(new org.bukkit.event.block.BlockPlaceEvent(
                                new Player(name), new Block(x, y, z, block)));
                    }
                }
                return c;
            }
            default -> {
                Log.warn("unknown event kind: " + f[0]);
                return false;
            }
        }
    }

    /** Routes a command line to a rine CommandHandler or a plugin.yml command executor. */
    private static void routeCommand(String player, String line) {
        String word = Server.commandWord(line);
        CommandHandler handler = Server.get().findCommand(word);
        if (handler != null) {
            try {
                handler.execute(player, line);
            } catch (RuntimeException ex) {
                Log.error("command handler " + word + " failed", ex);
            }
            return;
        }
        PluginCommand command = PluginCommand.find(word);
        if (command == null) {
            return;
        }
        CommandExecutor executor = command.getExecutor();
        if (executor == null) {
            return;
        }
        String[] args = argsOf(line);
        try {
            executor.onCommand(new Player(player), command, word, args);
        } catch (RuntimeException ex) {
            Log.error("command " + word + " failed", ex);
        }
    }

    private static String[] argsOf(String line) {
        String s = line.strip();
        if (s.startsWith("/")) {
            s = s.substring(1);
        }
        String[] parts = s.split("\\s+");
        if (parts.length <= 1) {
            return new String[0];
        }
        String[] args = new String[parts.length - 1];
        System.arraycopy(parts, 1, args, 0, args.length);
        return args;
    }

    // ---- plugin.yml / rine.yml ----

    private static Descriptor readDescriptor(Path jar) throws IOException {
        try (JarFile jf = new JarFile(jar.toFile())) {
            JarEntry entry = jf.getJarEntry("plugin.yml");
            if (entry == null) {
                entry = jf.getJarEntry("rine.yml");
            }
            if (entry == null) {
                return null;
            }
            String text = new String(jf.getInputStream(entry).readAllBytes(), StandardCharsets.UTF_8);
            return parseDescriptor(text, fileBaseName(jar));
        }
    }

    /** Parses only the keys this host needs: main, name, and the direct children of commands. */
    static Descriptor parseDescriptor(String text, String defaultName) {
        String main = null;
        String name = null;
        List<String> commands = new ArrayList<>();
        boolean inCommands = false;
        int childIndent = -1;
        for (String raw : text.replace("﻿", "").split("\\R")) {
            if (raw.isBlank() || raw.strip().startsWith("#")) {
                continue;
            }
            int indent = indentOf(raw);
            String t = raw.strip();
            if (indent == 0) {
                inCommands = false;
                int colon = t.indexOf(':');
                if (colon < 0) {
                    continue;
                }
                String key = t.substring(0, colon).strip();
                String value = unquote(t.substring(colon + 1).strip());
                switch (key) {
                    case "main" -> main = value;
                    case "name" -> name = value;
                    case "commands" -> inCommands = true;
                    default -> { }
                }
            } else if (inCommands) {
                if (childIndent < 0) {
                    childIndent = indent;
                }
                if (indent == childIndent && t.endsWith(":")) {
                    commands.add(t.substring(0, t.length() - 1).strip());
                }
            }
        }
        if (main == null) {
            return null;
        }
        return new Descriptor(main, name != null ? name : defaultName, commands);
    }

    private static int indentOf(String line) {
        int i = 0;
        while (i < line.length() && (line.charAt(i) == ' ' || line.charAt(i) == '\t')) {
            i++;
        }
        return i;
    }

    private static String unquote(String s) {
        if (s.length() >= 2 && (s.startsWith("\"") && s.endsWith("\"") || s.startsWith("'") && s.endsWith("'"))) {
            return s.substring(1, s.length() - 1);
        }
        return s;
    }

    private static String fileBaseName(Path jar) {
        String n = jar.getFileName().toString();
        return n.endsWith(".jar") ? n.substring(0, n.length() - 4) : n;
    }
}
