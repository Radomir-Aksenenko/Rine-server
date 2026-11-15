//! /datapack, /reload (`DataPackCommand`, `ReloadCommand`).

use crate::game::cmd::Commands;
use crate::game::Game;

pub fn register(_c: &mut Commands) {}

/// Pack ids (`file/<folder>`) the operator disabled.
pub fn disabled(g: &Game) -> Vec<String> {
    let _ = g;
    Vec::new()
}
