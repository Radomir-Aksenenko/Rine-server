package rine.api;

public final class QuitEvent extends Event {
    private final String playerName;

    public QuitEvent(String playerName) {
        this.playerName = playerName;
    }

    public String getPlayerName() {
        return playerName;
    }
}
