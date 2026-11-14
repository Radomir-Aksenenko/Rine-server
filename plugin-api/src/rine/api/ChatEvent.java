package rine.api;

public final class ChatEvent extends CancellableEvent {
    private final String playerName;
    private final String message;

    public ChatEvent(String playerName, String message) {
        this.playerName = playerName;
        this.message = message;
    }

    public String getPlayerName() {
        return playerName;
    }

    public String getMessage() {
        return message;
    }
}
