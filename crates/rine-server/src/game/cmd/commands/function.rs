//! /function, /schedule, /return, `execute if function`, the function
//! library (`FunctionCommand`, `ScheduleCommand`, `ReturnCommand`,
//! `ServerFunctionManager`, `ServerFunctionLibrary`).

use crate::game::cmd::exec::{handle_error, Frame, Mods};
use crate::game::cmd::source::Source;
use crate::game::cmd::{lit, Commands};
use crate::game::Game;
use rine_command::args::Arg;
use rine_command::{Chain, CmdError};
use std::sync::Arc;

pub fn register(_c: &mut Commands) {}

#[derive(Default)]
pub struct State {}

/// `ExecuteIfFunctionCustomModifier.apply`.
pub fn execute_if_function(g: &mut Game, original: Source, _sources: Vec<Source>, chain: Arc<Chain>, stage: usize, mods: Mods, _frame: Frame) {
    let ctx = &chain.modifiers[stage];
    let id = match ctx.arg("name").map(|a| &a.value) {
        Some(Arg::Function(f)) => f.id.clone(),
        _ => String::new(),
    };
    // no function library yet: every reference is unknown
    let e = CmdError::tr("arguments.function.unknown", vec![lit(&id)]);
    handle_error(g, &original, &e, mods.forked());
}
