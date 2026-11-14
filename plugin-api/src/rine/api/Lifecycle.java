package rine.api;

/**
 * Implemented by every plugin base class (RinePlugin, JavaPlugin). The host calls these
 * methods; plugin code does not.
 */
public interface Lifecycle {
    void bind(String name, Server server);

    void enable();

    void disable();
}
