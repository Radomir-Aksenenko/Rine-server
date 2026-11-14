package org.bukkit.event.player;

import org.bukkit.entity.Player;
import org.bukkit.event.Event;

public final class PlayerQuitEvent extends Event {
    private final Player player;
    private String quitMessage;

    public PlayerQuitEvent(Player player) {
        this.player = player;
        this.quitMessage = "";
    }

    public Player getPlayer() {
        return player;
    }

    /** Stored only. The game does not yet use it for the quit broadcast. */
    public String getQuitMessage() {
        return quitMessage;
    }

    public void setQuitMessage(String quitMessage) {
        this.quitMessage = quitMessage;
    }
}
