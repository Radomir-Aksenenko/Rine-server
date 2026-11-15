//! Command blocks (impulse / chain / repeat) and command block minecarts:
//! block entity state, GUI updates, execution (`BaseCommandBlock`,
//! `CommandBlockEntity`, `CommandBlock`).

use crate::game::Game;
use rine_protocol::text::Text;

/// `TrackOutput` of the command block at `pos`.
pub fn tracks_output(g: &Game, pos: [i32; 3]) -> bool {
    let _ = (g, pos);
    true
}

/// Store `[HH:mm:ss] message` as the block's `LastOutput`.
pub fn set_last_output(g: &mut Game, pos: [i32; 3], t: &Text) {
    let _ = (g, pos, t);
}
