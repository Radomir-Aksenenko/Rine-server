//! /time, /weather, /worldborder, /forceload, /setworldspawn, /spawnpoint,
//! /tick, /difficulty (`TimeCommand`, `WeatherCommand`, `WorldBorderCommand`,
//! `ForceLoadCommand`, `SetWorldSpawnCommand`, `SetSpawnCommand`, `TickCommand`,
//! `DifficultyCommand`).

use crate::game::cmd::Commands;

pub fn register(_c: &mut Commands) {}

/// World border, forced chunks, tick rate state.
#[derive(Default)]
pub struct State {}
