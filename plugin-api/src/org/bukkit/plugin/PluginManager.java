package org.bukkit.plugin;

import java.lang.invoke.MethodHandle;
import java.lang.invoke.MethodHandles;
import java.lang.invoke.MethodType;
import java.lang.reflect.Method;
import java.util.Objects;

import org.bukkit.event.Cancellable;
import org.bukkit.event.Event;
import org.bukkit.event.EventHandler;
import org.bukkit.event.Listener;
import rine.api.EventBus;
import rine.api.Log;

public final class PluginManager {

    /**
     * Scans the listener for @EventHandler methods. Each method is resolved to a MethodHandle
     * once here; dispatch later calls the handle without reflection.
     */
    public void registerEvents(Listener listener, Plugin plugin) {
        Objects.requireNonNull(listener, "listener");
        for (Class<?> c = listener.getClass(); c != null && c != Object.class; c = c.getSuperclass()) {
            for (Method m : c.getDeclaredMethods()) {
                EventHandler ann = m.getAnnotation(EventHandler.class);
                if (ann == null || m.getParameterCount() != 1) {
                    continue;
                }
                Class<?> type = m.getParameterTypes()[0];
                if (!Event.class.isAssignableFrom(type)) {
                    continue;
                }
                bind(listener, m, type, ann, plugin);
            }
        }
    }

    private static void bind(Listener listener, Method m, Class<?> type, EventHandler ann, Plugin plugin) {
        try {
            m.setAccessible(true);
            // Typed as (Object, Object)void so the call site needs no casts or reflection.
            MethodHandle mh = MethodHandles.lookup()
                    .unreflect(m)
                    .asType(MethodType.methodType(void.class, Object.class, Object.class));
            boolean ignoreCancelled = ann.ignoreCancelled();
            String where = plugin == null ? "?" : plugin.getName();
            EventBus.Handler handler = event -> {
                if (ignoreCancelled && event instanceof Cancellable c && c.isCancelled()) {
                    return;
                }
                try {
                    mh.invokeExact((Object) listener, (Object) event);
                } catch (Throwable t) {
                    Log.error("handler " + m.getName() + " in " + where + " failed", t);
                }
            };
            EventBus.get().register(type, ann.priority().ordinal(), handler);
        } catch (ReflectiveOperationException | RuntimeException e) {
            Log.error("cannot register " + m + " as an event handler", e);
        }
    }
}
