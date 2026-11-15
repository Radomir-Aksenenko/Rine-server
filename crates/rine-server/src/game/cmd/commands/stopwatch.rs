//! /stopwatch (`StopwatchCommand`, `Stopwatches`).

use crate::game::cmd::Commands;
use crate::game::Game;

pub fn register(_c: &mut Commands) {}

#[derive(Default)]
pub struct State {}

/// Elapsed seconds of a stopwatch (`Stopwatch.elapsedSeconds`), `None`
/// when it does not exist.
pub fn elapsed_seconds(g: &Game, id: &str) -> Option<f64> {
    let _ = (g, id);
    None
}
