package rine.api;

/** Handles a command registered with {@link Server#registerCommand(String, CommandHandler)}. */
@FunctionalInterface
public interface CommandHandler {
    /**
     * @param player the player who typed the command
     * @param line   the full command line, including the leading slash if the player typed one
     */
    void execute(String player, String line);
}
