//! Executors, one module per vanilla command class
//! (`net/minecraft/server/commands/*.java`).

pub mod blocks;
pub mod bossbar;
pub mod chat;
pub mod data;
pub mod datapack;
pub mod effect;
pub mod entities;
pub mod execute;
pub mod experience;
pub mod function;
pub mod gamemode;
pub mod gamerule;
pub mod itemcmd;
pub mod items;
pub mod misc;
pub mod players;
pub mod random;
pub mod rine;
pub mod scoreboard;
pub mod sound;
pub mod stopwatch;
pub mod summon;
pub mod teleport;
pub mod world;

use crate::game::cmd::Commands;
use crate::game::Game;
use crate::out;
use rine_command::CmdResult;

pub fn register_all(c: &mut Commands) {
    gamemode::register(c);
    gamerule::register(c);
    data::register(c);
    experience::register(c);
    players::register(c);
    random::register(c);
    scoreboard::register(c);
    rine::register(c);
    execute::register(c);
    bossbar::register(c);
    function::register(c);
    datapack::register(c);
    itemcmd::register(c);
    misc::register(c);
    stopwatch::register(c);
    summon::register(c);
    blocks::register(c);
    chat::register(c);
    effect::register(c);
    entities::register(c);
    items::register(c);
    sound::register(c);
    teleport::register(c);
    world::register(c);
}

/// State of command subsystems, one field per module that keeps any.
#[derive(Default)]
pub struct States {
    /// Command storage (`/data storage`).
    pub storage: ahash::AHashMap<String, rine_protocol::nbt::Tag>,
    pub bossbar: bossbar::State,
    pub stopwatch: stopwatch::State,
    pub function: function::State,
    pub world: world::State,
    pub effect: effect::State,
    pub scoreboard: scoreboard::State,
    pub cmdblock: crate::game::cmd::cmdblock::State,
}

/// Change a player's game mode (state, abilities, tab list), no message.
pub fn apply_gamemode(g: &mut Game, slot: usize, gm: u8) {
    let Some(p) = g.player(slot) else { return };
    p.gamemode = gm;
    p.flying = false;
    p.send(out::game_event(3, gm as f32));
    let ab = p.abilities_packet();
    p.send(ab);
    let uuid = p.uuid;
    let f = out::frame(out::info_gamemode(uuid, gm as i32));
    g.broadcast.push(f);
}

/// Side effects of game rule changes (vanilla rule callbacks).
pub fn on_rule_changed(g: &mut Game, name: &str) {
    match name {
        "reduced_debug_info" => {
            let v = g.rules.bool("reduced_debug_info");
            for p in g.players.iter_mut().flatten() {
                let eid = p.eid;
                p.send(out::entity_event(eid, if v { 22 } else { 23 }));
            }
        }
        "immediate_respawn" => {
            let v = g.rules.bool("immediate_respawn");
            for p in g.players.iter_mut().flatten() {
                p.send(out::game_event(11, if v { 1.0 } else { 0.0 }));
            }
        }
        "limited_crafting" => {
            let v = g.rules.bool("limited_crafting");
            for p in g.players.iter_mut().flatten() {
                p.send(out::game_event(12, if v { 1.0 } else { 0.0 }));
            }
        }
        _ => {}
    }
}

/// Executor for plugin commands (routed to the JVM host).
pub fn plugin_command(x: &mut crate::game::cmd::Ctx) -> CmdResult<i32> {
    let line = x.c.input.to_string();
    let name = x.src.text_name.to_string();
    crate::game::plugins::check(x.g, rine_plugin::SyncKind::Command, rine_plugin::Event::Command { name, line });
    Ok(1)
}
