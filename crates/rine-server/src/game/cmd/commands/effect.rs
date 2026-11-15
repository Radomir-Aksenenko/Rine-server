//! /effect, /attribute (`EffectCommands`, `AttributeCommand`) and the
//! mob effect / attribute state of players.

use crate::game::cmd::Commands;

pub fn register(_c: &mut Commands) {}

/// Active effects and attribute modifiers per entity id.
#[derive(Default)]
pub struct State {}
