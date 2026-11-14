package rine.api;

public final class BlockBreakEvent extends CancellableEvent {
    private final String playerName;
    private final int x;
    private final int y;
    private final int z;
    private final String block;

    public BlockBreakEvent(String playerName, int x, int y, int z, String block) {
        this.playerName = playerName;
        this.x = x;
        this.y = y;
        this.z = z;
        this.block = block;
    }

    public String getPlayerName() {
        return playerName;
    }

    public int getX() {
        return x;
    }

    public int getY() {
        return y;
    }

    public int getZ() {
        return z;
    }

    /** Block id, for example "minecraft:diamond_ore". */
    public String getBlock() {
        return block;
    }
}
