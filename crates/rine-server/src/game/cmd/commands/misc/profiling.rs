//! /debug (`DebugCommand`), /perf (`PerfCommand`), /jfr (`JfrCommand`).
//!
//! The time profiler keeps a wall clock and the game tick at start. Rine has
//! no JVM flight recorder, so `/jfr start` fails the way vanilla does when
//! JFR is unavailable. `/debug function` needs the tracer, which Rine does
//! not have, so it only resolves the function and then reports a stub.

use super::{lit, stub};
use crate::game::cmd::commands::function::resolve;
use crate::game::cmd::{Commands, Ctx};
use rine_command::args::misc::FunctionRef;
use rine_command::args::Arg;
use rine_command::{CmdError, CmdResult};
use rine_protocol::text::Text;
use std::sync::Mutex;
use std::time::Instant;

pub fn register(c: &mut Commands) {
    c.exec("debug start", debug_start);
    c.exec("debug stop", debug_stop);
    c.exec("debug function name", debug_function);
    c.exec("perf start", perf_start);
    c.exec("perf stop", perf_stop);
    c.exec("jfr start", jfr_start);
    c.exec("jfr stop", jfr_stop);
}

/// Start wall clock and tick of a running profile.
#[derive(Clone, Copy)]
struct Timer {
    start: Instant,
    tick: u64,
}

static DEBUG: Mutex<Option<Timer>> = Mutex::new(None);
static PERF: Mutex<Option<Timer>> = Mutex::new(None);

fn take(m: &Mutex<Option<Timer>>) -> Option<Timer> {
    m.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// `(seconds, ticks)` since the timer started.
fn elapsed(t: Timer, now_tick: u64) -> (f64, i32) {
    let secs = t.start.elapsed().as_nanos() as f64 / 1e9;
    let ticks = now_tick.saturating_sub(t.tick) as i32;
    (secs, ticks)
}

fn debug_start(x: &mut Ctx) -> CmdResult<i32> {
    let mut slot = DEBUG.lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_some() {
        return Err(CmdError::key("commands.debug.alreadyRunning"));
    }
    *slot = Some(Timer { start: Instant::now(), tick: x.g.tick });
    drop(slot);
    x.success(Text::key("commands.debug.started"), true);
    Ok(0)
}

/// `DebugCommand.stop`: returns the ticks per second, truncated.
fn debug_stop(x: &mut Ctx) -> CmdResult<i32> {
    let Some(t) = take(&DEBUG) else {
        return Err(CmdError::key("commands.debug.notRunning"));
    };
    let (secs, ticks) = elapsed(t, x.g.tick);
    let tps = ticks as f64 / secs;
    x.success(
        Text::tr(
            "commands.debug.stopped",
            vec![Text::plain(format!("{secs:.2}")), lit(ticks), Text::plain(format!("{tps:.2}"))],
        ),
        true,
    );
    Ok(tps as i32)
}

/// `DebugCommand.TraceCustomExecutor`. The tracer is not available, so the
/// function is only resolved (same errors as vanilla) and then reported.
fn debug_function(x: &mut Ctx) -> CmdResult<i32> {
    let f: FunctionRef = match x.arg("name") {
        Arg::Function(f) => f.clone(),
        _ => return Err(stub(x)),
    };
    resolve(&*x.g, &f)?;
    Err(stub(x))
}

fn perf_start(x: &mut Ctx) -> CmdResult<i32> {
    let mut slot = PERF.lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_some() {
        return Err(CmdError::key("commands.perf.alreadyRunning"));
    }
    *slot = Some(Timer { start: Instant::now(), tick: x.g.tick });
    drop(slot);
    x.success(Text::key("commands.perf.started"), false);
    Ok(0)
}

/// `PerfCommand.whenStopped`. No report file is written, so the
/// `commands.perf.reportSaved` message is not sent.
fn perf_stop(x: &mut Ctx) -> CmdResult<i32> {
    let Some(t) = take(&PERF) else {
        return Err(CmdError::key("commands.perf.notRunning"));
    };
    let (secs, ticks) = elapsed(t, x.g.tick);
    let tps = ticks as f64 / secs;
    x.success(
        Text::tr(
            "commands.perf.stopped",
            vec![Text::plain(format!("{secs:.2}")), lit(ticks), Text::plain(format!("{tps:.2}"))],
        ),
        false,
    );
    Ok(0)
}

/// `JfrCommand.startJfr`: JFR is not available in Rine.
fn jfr_start(_x: &mut Ctx) -> CmdResult<i32> {
    Err(CmdError::key("commands.jfr.start.failed"))
}

/// `JfrCommand.stopJfr`: with no recording, `JfrProfiler.stop` throws
/// `IllegalStateException("Not currently profiling")`, which becomes the
/// dump failure.
fn jfr_stop(_x: &mut Ctx) -> CmdResult<i32> {
    Err(CmdError::tr("commands.jfr.dump.failed", vec![lit("Not currently profiling")]))
}
