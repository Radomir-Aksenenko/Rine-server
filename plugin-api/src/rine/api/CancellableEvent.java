package rine.api;

/** An event that a listener can veto. Only meaningful for synchronous delivery. */
public abstract class CancellableEvent extends Event {
    private boolean cancelled;

    public boolean isCancelled() {
        return cancelled;
    }

    public void setCancelled(boolean cancelled) {
        this.cancelled = cancelled;
    }
}
