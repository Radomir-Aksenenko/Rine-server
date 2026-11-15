//! /test (`TestCommand`). Rine has no GameTest framework and no test instance
//! blocks, so the test set is always empty. Each form reports what vanilla
//! reports for an empty set: `run` says there are no tests, `clear` and
//! `reset` fail, `locate` fails after its start message. `create` builds a
//! test structure in vanilla, which Rine cannot do, so it stubs. Its size
//! limit is checked as in vanilla.

use super::{lit, stub};
use crate::game::cmd::{Commands, Ctx, SimpleFn};
use rine_command::{CmdError, CmdResult};
use rine_protocol::text::Text;

/// Largest test structure side (`TestCommand.createNewStructure`).
const MAX_SIZE: i32 = 48;

pub fn register(c: &mut Commands) {
    // every executable `test ...` path, routed by its second word
    let paths: Vec<&'static str> = c
        .tree
        .nodes
        .iter()
        .filter(|n| n.executable && n.path.starts_with("test "))
        .map(|n| n.path)
        .collect();
    for p in paths {
        if let Some(f) = handler(p) {
            c.exec(p, f);
        }
    }
}

fn handler(path: &str) -> Option<SimpleFn> {
    let word = path.split(' ').nth(1)?;
    let f: SimpleFn = match word {
        w if w.starts_with("run") => run,
        w if w.starts_with("reset") => reset,
        w if w.starts_with("clear") => clear,
        "stop" => stop,
        "pos" => pos,
        "verify" => verify,
        "locate" => locate,
        "create" => create,
        _ => return None,
    };
    Some(f)
}

/// `TestCommand.run`: no tests selected, so nothing runs.
fn run(x: &mut Ctx) -> CmdResult<i32> {
    x.success(Text::key("commands.test.no_tests"), false);
    Ok(0)
}

/// `TestCommand.verify`: the batch list is empty, so there is nothing to run.
fn verify(x: &mut Ctx) -> CmdResult<i32> {
    x.success(Text::key("commands.test.no_tests"), false);
    Ok(0)
}

/// `TestCommand.reset`: throws `CLEAR_NO_TESTS` when the set is empty.
fn reset(_x: &mut Ctx) -> CmdResult<i32> {
    Err(CmdError::key("commands.test.clear.error.no_tests"))
}

/// `TestCommand.clear`: no test blocks to clear.
fn clear(_x: &mut Ctx) -> CmdResult<i32> {
    Err(CmdError::key("commands.test.clear.error.no_tests"))
}

/// `TestCommand.stopTests`: returns 1, no message.
fn stop(_x: &mut Ctx) -> CmdResult<i32> {
    Ok(1)
}

/// `TestCommand.showPos`. Vanilla picks the block the player looks at; Rine
/// uses the block at the player's feet, so the reported position is an
/// approximation.
fn pos(x: &mut Ctx) -> CmdResult<i32> {
    x.source_player()?;
    let p = x.src.pos;
    let (bx, by, bz) = (p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32);
    Err(CmdError::tr(
        "commands.test.error.no_test_containing_pos",
        vec![lit(bx), lit(by), lit(bz)],
    ))
}

/// `TestCommand.locate`: announces the search, then finds no test instance.
fn locate(x: &mut Ctx) -> CmdResult<i32> {
    x.system(Text::key("commands.test.locate.started"));
    Err(CmdError::key("commands.test.error.no_test_instances"))
}

/// `TestCommand.createNewStructure`: the size check comes first, then the
/// structure would be built, which Rine cannot do.
fn create(x: &mut Ctx) -> CmdResult<i32> {
    let (w, h, d) = if x.has("width") {
        let w = x.int("width");
        if x.has("height") {
            (w, x.int("height"), x.int("depth"))
        } else {
            (w, w, w)
        }
    } else {
        (5, 5, 5)
    };
    if w <= MAX_SIZE && h <= MAX_SIZE && d <= MAX_SIZE {
        return Err(stub(x));
    }
    Err(CmdError::tr("commands.test.error.too_large", vec![lit(MAX_SIZE)]))
}
