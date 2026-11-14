package rine.api;

import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Dispatches events to handlers by exact event class. Handler arrays are rebuilt at
 * registration time, so dispatch is a single map lookup plus a loop.
 * Only the plugin thread touches this class.
 */
public final class EventBus {
    /** Priority values match org.bukkit.event.EventPriority ordinals. */
    public static final int NORMAL = 2;

    public interface Handler {
        void call(Object event);
    }

    private static final EventBus INSTANCE = new EventBus();

    private final Map<Class<?>, List<Entry>> entries = new HashMap<>();
    private final Map<Class<?>, Handler[]> table = new HashMap<>();

    private record Entry(int priority, Handler handler) {
    }

    private EventBus() {
    }

    public static EventBus get() {
        return INSTANCE;
    }

    public synchronized void register(Class<?> type, int priority, Handler handler) {
        List<Entry> list = entries.computeIfAbsent(type, k -> new ArrayList<>());
        list.add(new Entry(priority, handler));
        list.sort(Comparator.comparingInt(Entry::priority));
        Handler[] arr = new Handler[list.size()];
        for (int i = 0; i < arr.length; i++) {
            arr[i] = list.get(i).handler();
        }
        table.put(type, arr);
    }

    public boolean has(Class<?> type) {
        return table.containsKey(type);
    }

    /** Delivers the event to every handler for its exact class. Returns true if it ended cancelled. */
    public boolean dispatch(Object event) {
        Handler[] handlers = table.get(event.getClass());
        if (handlers == null) {
            return false;
        }
        for (Handler h : handlers) {
            h.call(event);
        }
        return isCancelled(event);
    }

    public static boolean isCancelled(Object event) {
        if (event instanceof CancellableEvent c) {
            return c.isCancelled();
        }
        if (event instanceof org.bukkit.event.Cancellable c) {
            return c.isCancelled();
        }
        return false;
    }
}
