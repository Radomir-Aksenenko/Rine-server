//! Player list management: /op, /deop, /ban, /ban-ip, /banlist, /pardon,
//! /pardon-ip, /whitelist, /kick, /list, and the user cache used by game
//! profile arguments.

use crate::game::cmd::Commands;
use crate::game::Game;

pub fn register(_c: &mut Commands) {}

/// Known profile by name (online or seen before): (name, uuid).
pub fn cached_profile(g: &Game, name: &str) -> Option<(String, uuid::Uuid)> {
    let _ = (g, name);
    None
}
