package rine.api;

/** Base class for native Rine plugins. Declare the main class in rine.yml or plugin.yml. */
public abstract class RinePlugin implements Lifecycle {
    private String name = "unnamed";
    private Server server;

    public abstract void onEnable();

    public void onDisable() {
    }

    public final String getName() {
        return name;
    }

    public final Server getServer() {
        return server;
    }

    @Override
    public final void bind(String name, Server server) {
        this.name = name;
        this.server = server;
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
