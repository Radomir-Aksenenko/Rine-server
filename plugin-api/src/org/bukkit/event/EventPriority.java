package org.bukkit.event;

/** Order in which handlers for one event run. Ordinals are used as sort keys. */
public enum EventPriority {
    LOWEST,
    LOW,
    NORMAL,
    HIGH,
    HIGHEST,
    MONITOR
}
