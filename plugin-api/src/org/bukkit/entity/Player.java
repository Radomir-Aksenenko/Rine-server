package org.bukkit.entity;

import java.util.Objects;

import org.bukkit.Location;
import org.bukkit.command.CommandSender;

/** A player identified by name. Methods act on the Rine server through queued actions. */
public final class Player implements CommandSender {
    private final String name;

    public Player(String name) {
        this.name = name;
    }

    @Override
    public String getName() {
        return name;
    }

    @Override
    public void sendMessage(String message) {
        rine.api.Server.get().sendMessage(name, message);
    }

    public void teleport(Location location) {
        rine.api.Server.get().teleport(name, location.getX(), location.getY(), location.getZ());
    }

    public void kickPlayer(String reason) {
        rine.api.Server.get().kick(name, reason);
    }

    @Override
    public boolean equals(Object o) {
        return o instanceof Player p && p.name.equals(name);
    }

    @Override
    public int hashCode() {
        return Objects.hash(name);
    }

    @Override
    public String toString() {
        return "Player{" + name + "}";
    }
}
