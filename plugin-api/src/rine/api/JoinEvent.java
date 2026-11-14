package rine.api;

public final class JoinEvent extends Event {
    private final String playerName;
    private final String uuid;

    public JoinEvent(String playerName, String uuid) {
        this.playerName = playerName;
        this.uuid = uuid;
    }

    public String getPlayerName() {
        return playerName;
    }

    public String getUuid() {
        return uuid;
    }
}
