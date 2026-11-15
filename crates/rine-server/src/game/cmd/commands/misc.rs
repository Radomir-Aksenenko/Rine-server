//! /debug, /perf, /jfr, /dialog, /posteffect, /waypoint, /fetchprofile,
//! /compute, /place, /locate, /advancement, /recipe and /test. Rine keeps
//! no advancements, recipes, structures, templates, POIs or test instances,
//! so the commands that need them answer the way vanilla does for an empty
//! registry, or fail with `stub` where the feature itself is missing.

use crate::game::cmd::{Commands, Ctx};
use rine_command::CmdError;
use rine_protocol::text::Text;

pub mod advancement;
pub mod compute;
pub mod dialog;
pub mod fetch;
pub mod gametest;
pub mod locate;
pub mod place;
pub mod postfx;
pub mod profiling;
pub mod tracker;
pub mod waypoint;

pub fn register(c: &mut Commands) {
    profiling::register(c);
    dialog::register(c);
    postfx::register(c);
    waypoint::register(c);
    fetch::register(c);
    compute::register(c);
    place::register(c);
    locate::register(c);
    advancement::register(c);
    gametest::register(c);
}

/// Error for a command path whose feature Rine does not have yet.
pub fn stub(x: &Ctx) -> CmdError {
    let id = x.g.cmds.tree.find(x.path()).expect("command path in tree");
    crate::game::cmd::unimplemented_error(&*x.g, id)
}

/// Colour as `#RRGGBB` (the form the chat colour field takes).
pub fn hex_rgb(rgb: i32) -> String {
    format!("#{:06X}", rgb & 0xFF_FFFF)
}

/// `ComponentUtils.wrapInSquareBrackets`.
pub fn brackets(t: Text) -> Text {
    Text::tr("chat.square_brackets", vec![t])
}

/// Number as a text argument.
pub fn lit(v: impl ToString) -> Text {
    rine_command::error::lit(v)
}

/// UUID of the player in a slot.
pub fn uuid_of(x: &Ctx, slot: usize) -> Option<uuid::Uuid> {
    x.g.players.get(slot).and_then(|p| p.as_ref()).map(|p| p.uuid)
}
