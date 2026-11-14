package org.bukkit.event.player;

import org.bukkit.entity.Player;
import org.bukkit.event.Event;

public final class PlayerJoinEvent extends Event {
    private final Player player;
    private String joinMessage;

    public PlayerJoinEvent(Player player) {
        this.player = player;
        this.joinMessage = "";
    }

    public Player getPlayer() {
        return player;
    }

    /** Stored only. The game does not yet use it for the join broadcast. */
    public String getJoinMessage() {
        return joinMessage;
    }

    public void setJoinMessage(String joinMessage) {
        this.joinMessage = joinMessage;
    }
}
