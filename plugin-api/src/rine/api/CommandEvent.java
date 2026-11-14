package rine.api;

/**
 * A command the server did not handle itself. Cancelling it stops routing to the command
 * handler (only effective when delivered synchronously).
 */
public final class CommandEvent extends CancellableEvent {
    private final String playerName;
    private final String line;

    public CommandEvent(String playerName, String line) {
        this.playerName = playerName;
        this.line = line;
    }

    public String getPlayerName() {
        return playerName;
    }

    /** Full line as typed, for example "/hello world". */
    public String getLine() {
        return line;
    }

    /** Lower-case command word without the slash, for example "hello". */
    public String getCommandName() {
        return Server.commandWord(line);
    }
}
