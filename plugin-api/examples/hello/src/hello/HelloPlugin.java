package hello;

import org.bukkit.Bukkit;
import org.bukkit.ChatColor;
import org.bukkit.Material;
import org.bukkit.block.Block;
import org.bukkit.command.Command;
import org.bukkit.command.CommandExecutor;
import org.bukkit.command.CommandSender;
import org.bukkit.event.EventHandler;
import org.bukkit.event.Listener;
import org.bukkit.event.block.BlockBreakEvent;
import org.bukkit.event.player.PlayerJoinEvent;
import org.bukkit.plugin.java.JavaPlugin;

/** Example Bukkit-style plugin: greets joiners, answers /hello, protects diamond ore. */
public final class HelloPlugin extends JavaPlugin implements Listener, CommandExecutor {

    @Override
    public void onEnable() {
        getServer().getPluginManager().registerEvents(this, this);
        getCommand("hello").setExecutor(this);
        getLogger().info("Hello enabled");
    }

    @Override
    public void onDisable() {
        getLogger().info("Hello disabled");
    }

    @EventHandler
    public void onJoin(PlayerJoinEvent event) {
        Bukkit.broadcastMessage("Привет, " + event.getPlayer().getName() + "!");
    }

    @EventHandler
    public void onBreak(BlockBreakEvent event) {
        Block block = event.getBlock();
        if (block.getType() == Material.DIAMOND_ORE) {
            event.setCancelled(true);
            event.getPlayer().sendMessage(ChatColor.RED + "Алмазную руду ломать нельзя.");
        }
    }

    @Override
    public boolean onCommand(CommandSender sender, Command command, String label, String[] args) {
        sender.sendMessage(ChatColor.GREEN + "Привет, " + sender.getName() + "!");
        return true;
    }
}
