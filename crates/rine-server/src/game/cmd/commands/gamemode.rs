//! /gamemode, /defaultgamemode (`GameModeCommand`, `DefaultGameModeCommands`).

use crate::game::cmd::{Commands, Ctx};
use rine_command::CmdResult;
use rine_protocol::text::Text;

pub const NAMES: [&str; 4] = ["survival", "creative", "adventure", "spectator"];

pub fn register(c: &mut Commands) {
    c.exec("gamemode gamemode", |x| {
        let slot = x.source_player()?;
        let mode = x.enum_("gamemode");
        Ok(set_mode(x, &[slot], mode))
    });
    c.exec("gamemode gamemode target", |x| {
        let mode = x.enum_("gamemode");
        let targets = x.players("target")?;
        Ok(set_mode(x, &targets, mode))
    });
    c.exec("defaultgamemode gamemode", default_mode);
}

/// `GameModeCommand.setGameMode(player, type)` (F3+F4): as if the player
/// ran `/gamemode <mode>`.
pub fn set_self(g: &mut crate::game::Game, slot: usize, mode: u8) {
    let Some(src) = crate::game::cmd::source::Source::player(g, slot) else { return };
    let chain = std::sync::Arc::new(rine_command::Chain {
        modifiers: Vec::new(),
        executable: rine_command::parse(&g.cmds.tree.clone(), "gamemode survival", &rine_command::args::ParseEnv::new(4)).context,
    });
    let mut x = Ctx { g, src: &src, c: &chain.executable, chain: &chain, stage: 0 };
    set_mode(&mut x, &[slot], mode);
}

fn set_mode(x: &mut Ctx, slots: &[usize], mode: u8) -> i32 {
    let mut count = 0;
    for &slot in slots {
        // ServerPlayer.setGameMode: no change, no message
        let Some(p) = x.g.players[slot].as_ref() else { continue };
        if p.gamemode == mode {
            continue;
        }
        let eid = p.eid;
        crate::game::cmd::commands::apply_gamemode(x.g, slot, mode);
        let m = Text::key(format!("gameMode.{}", NAMES[mode as usize]));
        if x.src.entity == Some(eid) {
            x.success(Text::tr("commands.gamemode.success.self", vec![m]), true);
        } else {
            if x.g.rules.bool("send_command_feedback") {
                if let Some(p) = x.g.player(slot) {
                    p.msg(&Text::tr("gameMode.changed", vec![m.clone()]));
                }
            }
            let name = x.player_name(slot);
            x.success(Text::tr("commands.gamemode.success.other", vec![name, m]), true);
        }
        count += 1;
    }
    count
}

/// `DefaultGameModeCommands.setMode` (no `force-gamemode`: count 0).
fn default_mode(x: &mut Ctx) -> CmdResult<i32> {
    let mode = x.enum_("gamemode");
    x.g.default_gamemode = mode;
    let m = Text::key(format!("gameMode.{}", NAMES[mode as usize]));
    x.success(Text::tr("commands.defaultgamemode.success", vec![m]), true);
    Ok(0)
}
