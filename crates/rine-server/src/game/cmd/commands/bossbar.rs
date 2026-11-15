//! /bossbar (`BossBarCommands`, `CustomBossEvents`, `CustomBossEvent`).

use crate::game::cmd::Commands;
use crate::game::Game;

pub fn register(_c: &mut Commands) {}

/// Custom boss bars by id (`CustomBossEvents`).
#[derive(Default)]
pub struct State {}

/// A custom boss bar with this id exists.
pub fn exists(g: &Game, id: &str) -> bool {
    let _ = (g, id);
    false
}

/// `CustomBossEvent.setValue` (clamped like vanilla), sends updates.
pub fn set_value(g: &mut Game, id: &str, v: i32) {
    let _ = (g, id, v);
}

/// `CustomBossEvent.setMax`, sends updates.
pub fn set_max(g: &mut Game, id: &str, v: i32) {
    let _ = (g, id, v);
}
