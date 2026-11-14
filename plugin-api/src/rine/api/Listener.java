package rine.api;

/** Handles one event type. Registered with {@link Server#on(Class, Listener)}. */
@FunctionalInterface
public interface Listener<E> {
    void handle(E event);
}
