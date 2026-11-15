//! /experience, /xp (`ExperienceCommand`) and player XP state.

use crate::game::cmd::Commands;
use crate::game::Game;

pub fn register(_c: &mut Commands) {}

/// Experience level of a player entity id (0 when unknown).
pub fn level_of(g: &Game, eid: i32) -> i32 {
    let _ = (g, eid);
    0
}
