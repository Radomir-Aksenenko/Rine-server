package rine.api;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;

/**
 * Plugin-facing server API. Every mutating call queues an action; the game applies the queue
 * at the start of the next tick. Only the plugin thread touches this object.
 */
public final class Server {
    private static final Server INSTANCE = new Server();

    private final List<String> actions = new ArrayList<>();
    private final Set<String> online = new LinkedHashSet<>();
    private final Map<String, CommandHandler> commands = new LinkedHashMap<>();

    private Server() {
    }

    public static Server get() {
        return INSTANCE;
    }

    public void broadcast(String text) {
        queue(Wire.pack("broadcast", text));
    }

    public void sendMessage(String player, String msg) {
        queue(Wire.pack("message", player, msg));
    }

    public void setBlock(int x, int y, int z, String block) {
        queue(Wire.pack("setblock", str(x), str(y), str(z), block));
    }

    public void teleport(String player, double x, double y, double z) {
        queue(Wire.pack("teleport", player, str(x), str(y), str(z)));
    }

    public void give(String player, String item, int count) {
        queue(Wire.pack("give", player, item, str(count)));
    }

    public void kick(String player, String reason) {
        queue(Wire.pack("kick", player, reason));
    }

    public void runCommand(String player, String line) {
        queue(Wire.pack("runcommand", player, line));
    }

    /** Names of players currently online, in join order. */
    public List<String> getOnlinePlayers() {
        return List.copyOf(online);
    }

    /** Registers a command word (without slash). Re-registering replaces the handler. */
    public void registerCommand(String name, CommandHandler handler) {
        commands.put(name.toLowerCase(Locale.ROOT), handler);
    }

    public <E extends Event> void on(Class<E> type, Listener<E> listener) {
        EventBus.get().register(type, EventBus.NORMAL, event -> {
            try {
                listener.handle(type.cast(event));
            } catch (RuntimeException ex) {
                Log.error("listener failed for " + type.getSimpleName(), ex);
            }
        });
    }

    // ---- internal, used by rine.host.Host ----

    /** Internal: called by the host on join, before join listeners run. */
    public void trackJoin(String name) {
        online.add(name);
    }

    /** Internal: called by the host on quit, after quit listeners run. */
    public void trackQuit(String name) {
        online.remove(name);
    }

    /** Internal: returns and clears queued actions in packed form. */
    public String[] drainActions() {
        String[] out = actions.toArray(new String[0]);
        actions.clear();
        return out;
    }

    /** Internal: command handler for a lower-case command word, or null. */
    public CommandHandler findCommand(String word) {
        return commands.get(word);
    }

    /** Internal: command words registered with registerCommand. */
    public List<String> commandNames() {
        return List.copyOf(commands.keySet());
    }

    /** Lower-case command word of a line such as "/Hello x" -> "hello". */
    public static String commandWord(String line) {
        String s = line.strip();
        if (s.startsWith("/")) {
            s = s.substring(1);
        }
        int space = s.indexOf(' ');
        if (space >= 0) {
            s = s.substring(0, space);
        }
        return s.toLowerCase(Locale.ROOT);
    }

    private void queue(String packed) {
        actions.add(packed);
    }

    private static String str(double v) {
        return Double.toString(v);
    }

    private static String str(int v) {
        return Integer.toString(v);
    }
}
