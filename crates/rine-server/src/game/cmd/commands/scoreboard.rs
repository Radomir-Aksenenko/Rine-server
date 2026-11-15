//! Scoreboard: objectives, scores, teams, display slots; /scoreboard,
//! /team, /teammsg, /tm, /trigger (`ScoreboardCommand`, `TeamCommand`,
//! `TeamMsgCommand`, `TriggerCommand`, `ServerScoreboard`).

use crate::game::cmd::Commands;
use crate::game::Game;
use rine_protocol::text::Text;

pub fn register(_c: &mut Commands) {}

/// `PlayerTeam.formatNameForTeam(team of holder, name)`.
pub fn team_format(g: &Game, holder: &str, name: Text) -> Text {
    let _ = (g, holder);
    name
}

/// Team name of a score holder.
pub fn team_of(g: &Game, holder: &str) -> Option<String> {
    let _ = (g, holder);
    None
}

/// Score of a holder in an objective.
pub fn score(g: &Game, holder: &str, objective: &str) -> Option<i32> {
    let _ = (g, holder, objective);
    None
}

/// `score` text component value (`ScoreContents.getScore`): formatted
/// score or empty.
pub fn format_score(g: &Game, holder: &str, objective: &str) -> Text {
    match score(g, holder, objective) {
        Some(v) => Text::plain(v.to_string()),
        None => Text::empty(),
    }
}
