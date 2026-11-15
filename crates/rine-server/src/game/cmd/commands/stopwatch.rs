//! /stopwatch (`StopwatchCommand`, `Stopwatches`).
//!
//! Stopwatches live in memory only (vanilla keeps them in `stopwatches.dat`;
//! saving is not wired yet).

use crate::game::cmd::{lit, Commands, Ctx};
use crate::game::Game;
use rine_command::error::java_double;
use rine_command::{CmdError, CmdResult};
use rine_protocol::text::Text;
use std::collections::BTreeMap;

pub fn register(c: &mut Commands) {
    c.exec("stopwatch create id", create);
    c.exec("stopwatch query id", |x| query(x, 1.0));
    c.exec("stopwatch query id scale", query_scaled);
    c.exec("stopwatch restart id", restart);
    c.exec("stopwatch remove id", remove);
}

/// Stopwatches by id, holding the creation time in epoch milliseconds
/// (`Stopwatch.creationTime`; the accumulated time is always 0 here).
#[derive(Default)]
pub struct State {
    pub watches: BTreeMap<String, i64>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Elapsed seconds of a stopwatch (`Stopwatch.elapsedSeconds`), `None`
/// when it does not exist.
pub fn elapsed_seconds(g: &Game, id: &str) -> Option<f64> {
    g.cs.stopwatch.watches.get(id).map(|created| (now_ms() - created) as f64 / 1000.0)
}

fn create(x: &mut Ctx) -> CmdResult<i32> {
    let id = x.id("id").to_string();
    if x.g.cs.stopwatch.watches.contains_key(&id) {
        return Err(CmdError::tr("commands.stopwatch.already_exists", vec![lit(&id)]));
    }
    x.g.cs.stopwatch.watches.insert(id.clone(), now_ms());
    x.success(Text::tr("commands.stopwatch.create.success", vec![lit(&id)]), true);
    Ok(1)
}

fn query(x: &mut Ctx, scale: f64) -> CmdResult<i32> {
    let id = x.id("id").to_string();
    let Some(elapsed) = elapsed_seconds(x.g, &id) else {
        return Err(CmdError::tr("commands.stopwatch.does_not_exist", vec![lit(&id)]));
    };
    x.success(Text::tr("commands.stopwatch.query", vec![lit(&id), lit(java_double(elapsed))]), true);
    Ok((elapsed * scale) as i32)
}

fn query_scaled(x: &mut Ctx) -> CmdResult<i32> {
    let scale = x.double("scale");
    query(x, scale)
}

fn restart(x: &mut Ctx) -> CmdResult<i32> {
    let id = x.id("id").to_string();
    let Some(slot) = x.g.cs.stopwatch.watches.get_mut(&id) else {
        return Err(CmdError::tr("commands.stopwatch.does_not_exist", vec![lit(&id)]));
    };
    *slot = now_ms();
    x.success(Text::tr("commands.stopwatch.restart.success", vec![lit(&id)]), true);
    Ok(1)
}

fn remove(x: &mut Ctx) -> CmdResult<i32> {
    let id = x.id("id").to_string();
    if x.g.cs.stopwatch.watches.remove(&id).is_none() {
        return Err(CmdError::tr("commands.stopwatch.does_not_exist", vec![lit(&id)]));
    }
    x.success(Text::tr("commands.stopwatch.remove.success", vec![lit(&id)]), true);
    Ok(1)
}
