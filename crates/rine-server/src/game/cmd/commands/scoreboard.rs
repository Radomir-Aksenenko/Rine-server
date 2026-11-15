//! Scoreboard: objectives, scores, teams, display slots; /scoreboard,
//! /team, /teammsg, /tm, /trigger (`ScoreboardCommand`, `TeamCommand`,
//! `TeamMsgCommand`, `TriggerCommand`, `ServerScoreboard`).

use crate::game::cmd::Commands;
use crate::game::Game;
use rine_protocol::text::Text;

pub fn register(_c: &mut Commands) {}

/// Objectives, scores, teams, display slots (`ServerScoreboard`).
#[derive(Default)]
pub struct State {}

/// `ScoreHolder`: scoreboard name plus the entity it came from (for the
/// feedback display name).
#[derive(Debug, Clone, PartialEq)]
pub struct Holder {
    pub name: String,
    pub entity: Option<i32>,
}

impl Holder {
    pub fn named(name: impl Into<String>) -> Self {
        Self { name: name.into(), entity: None }
    }
    /// `getFeedbackDisplayName()`: entity display name or the literal name.
    pub fn display(&self, g: &Game) -> Text {
        match self.entity {
            Some(e) => crate::game::cmd::feedback::display_name(g, e),
            None => Text::plain(self.name.clone()),
        }
    }
}

/// `Scoreboard.getTrackedPlayers()` (wildcard `*`).
pub fn tracked_holders(g: &Game) -> Vec<Holder> {
    let _ = g;
    Vec::new()
}

/// Set a score, creating it (`getOrCreatePlayerScore(...).set(v)`).
pub fn set_score(g: &mut Game, holder: &Holder, objective: &str, value: i32) {
    let _ = (g, holder, objective, value);
}

/// Objective exists (`ObjectiveArgument.getObjective`: else
/// `arguments.objective.notFound`).
pub fn objective_exists(g: &Game, objective: &str) -> bool {
    let _ = (g, objective);
    false
}

/// `ObjectiveArgument.getWritableObjective`: read-only criteria error.
pub fn objective_writable(g: &Game, objective: &str) -> bool {
    let _ = (g, objective);
    true
}

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
