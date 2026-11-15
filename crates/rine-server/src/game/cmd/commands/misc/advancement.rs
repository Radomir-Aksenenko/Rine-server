//! /advancement (`AdvancementCommands`) and /recipe (`RecipeCommand`).
//!
//! Rine loads no advancement tree and no recipe manager. A named advancement
//! or recipe is therefore not found (the vanilla `...NotFound` message for an
//! id missing from the manager). `everything` and `recipe ... *` act on an
//! empty set and fail the way vanilla does when nothing changes.

use super::lit;
use super::tracker::Tracker;
use crate::game::cmd::{Commands, Ctx};
use rine_command::{CmdError, CmdResult};
use rine_protocol::text::Text;

pub fn register(c: &mut Commands) {
    for action in ["grant", "revoke"] {
        for mode in ["only", "from", "until", "through"] {
            c.exec(&format!("advancement {action} targets {mode} advancement"), advancement);
        }
        c.exec(&format!("advancement {action} targets only advancement criterion"), advancement);
        c.exec(&format!("advancement {action} targets everything"), everything);
    }
    c.exec("recipe give targets recipe", recipe_give);
    c.exec("recipe give targets *", recipe_give_all);
    c.exec("recipe take targets recipe", recipe_take);
    c.exec("recipe take targets *", recipe_take_all);
}

/// `AdvancementCommands.Action` for the grant or revoke literal.
fn action(x: &Ctx) -> &'static str {
    x.lit(1)
}

/// Players first (`EntityArgument.getPlayers`), then the advancement lookup.
fn advancement(x: &mut Ctx) -> CmdResult<i32> {
    let _ = x.players("targets")?;
    let id = x.id("advancement").to_string();
    Err(CmdError::tr("advancement.advancementNotFound", vec![Text::plain(id)]))
}

/// `perform(..., everything)`: no advancements, so no player changed.
fn everything(x: &mut Ctx) -> CmdResult<i32> {
    let slots = x.players("targets")?;
    let act = action(x);
    let mut t = Tracker::new();
    for slot in slots {
        t.track(slot, 0);
    }
    let zero = Text::plain("0");
    let (single, multi) = if act == "grant" {
        ("commands.advancement.grant.many.to.one.failure", "commands.advancement.grant.many.to.many.failure")
    } else {
        ("commands.advancement.revoke.many.to.one.failure", "commands.advancement.revoke.many.to.many.failure")
    };
    Err(t.fail(x, single, multi, &[zero]))
}

fn recipe_give(x: &mut Ctx) -> CmdResult<i32> {
    let _ = x.players("targets")?;
    let id = x.id("recipe").to_string();
    Err(CmdError::tr("recipe.notFound", vec![Text::plain(id)]))
}

fn recipe_take(x: &mut Ctx) -> CmdResult<i32> {
    recipe_give(x)
}

/// `recipe give * `: no learnable recipes, so nothing is awarded.
fn recipe_give_all(x: &mut Ctx) -> CmdResult<i32> {
    let slots = x.players("targets")?;
    let mut t = Tracker::new();
    for slot in slots {
        t.track(slot, 0);
    }
    t.send(x, Some("commands.recipe.give.failed"), "commands.recipe.give.success.single", "commands.recipe.give.success.multiple", &[lit(0)])
}

fn recipe_take_all(x: &mut Ctx) -> CmdResult<i32> {
    let slots = x.players("targets")?;
    let mut t = Tracker::new();
    for slot in slots {
        t.track(slot, 0);
    }
    t.send(x, Some("commands.recipe.take.failed"), "commands.recipe.take.success.single", "commands.recipe.take.success.multiple", &[lit(0)])
}
