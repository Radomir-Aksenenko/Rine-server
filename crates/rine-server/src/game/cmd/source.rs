//! `CommandSourceStack`: who runs a command, from where, with which
//! rights, and where its output goes. Immutable; `with_*` make copies.

use super::exec::Callback;
use crate::game::Game;
use rine_protocol::text::Text;
use std::sync::Arc;

/// `CommandSource`: the output sink.
#[derive(Clone, Debug, PartialEq)]
pub enum Output {
    /// A player, by entity id (survives slot reuse; gone = dropped).
    Player(i32),
    Console,
    /// A command block at a position.
    CommandBlock([i32; 3]),
    /// `CommandSource.NULL` (functions, `execute as` targets keep the
    /// original output instead).
    Null,
}

#[derive(Clone)]
pub struct Source {
    pub output: Output,
    pub pos: [f64; 3],
    /// (xRot = pitch, yRot = yaw).
    pub rot: [f32; 2],
    pub dim: &'static str,
    /// Op permission level 0..=4.
    pub level: u8,
    pub text_name: Arc<str>,
    pub display: Arc<Text>,
    /// Executing entity (player or entity id).
    pub entity: Option<i32>,
    pub silent: bool,
    /// Anchor for local coordinates: false feet, true eyes.
    pub anchor_eyes: bool,
    pub callback: Callback,
}

pub const OVERWORLD: &str = "minecraft:overworld";

impl Source {
    /// `ServerPlayer.createCommandSourceStack`.
    pub fn player(g: &Game, slot: usize) -> Option<Source> {
        let p = g.players.get(slot)?.as_ref()?;
        let level = g.op_level_of(&p.name);
        Some(Source {
            output: Output::Player(p.eid),
            pos: p.pos,
            rot: [p.pitch, p.yaw],
            dim: OVERWORLD,
            level,
            text_name: Arc::from(p.name.as_str()),
            display: Arc::new(super::feedback::player_display_name(g, slot)),
            entity: Some(p.eid),
            silent: false,
            anchor_eyes: false,
            callback: None,
        })
    }

    /// `MinecraftServer.createCommandSourceStack` (console, level 4).
    pub fn console(g: &Game) -> Source {
        Source {
            output: Output::Console,
            pos: [g.spawn[0].floor(), g.spawn[1].floor(), g.spawn[2].floor()],
            rot: [0.0, 0.0],
            dim: OVERWORLD,
            level: 4,
            text_name: Arc::from("Server"),
            display: Arc::new(Text::plain("Server")),
            entity: None,
            silent: false,
            anchor_eyes: false,
            callback: None,
        }
    }

    /// Command block source: centre of the block, level 2.
    pub fn command_block(pos: [i32; 3], name: &str, display: Text) -> Source {
        Source {
            output: Output::CommandBlock(pos),
            pos: [pos[0] as f64 + 0.5, pos[1] as f64 + 0.5, pos[2] as f64 + 0.5],
            rot: [0.0, 0.0],
            dim: OVERWORLD,
            level: 2,
            text_name: Arc::from(name),
            display: Arc::new(display),
            entity: None,
            silent: false,
            anchor_eyes: false,
            callback: None,
        }
    }

    pub fn with_pos(&self, pos: [f64; 3]) -> Source {
        Source { pos, ..self.clone() }
    }
    pub fn with_rot(&self, rot: [f32; 2]) -> Source {
        Source { rot, ..self.clone() }
    }
    pub fn with_level(&self, level: u8) -> Source {
        Source { level, ..self.clone() }
    }
    pub fn with_max_level(&self, level: u8) -> Source {
        Source { level: self.level.min(level), ..self.clone() }
    }
    pub fn with_silent(&self) -> Source {
        Source { silent: true, ..self.clone() }
    }
    pub fn with_anchor(&self, eyes: bool) -> Source {
        Source { anchor_eyes: eyes, ..self.clone() }
    }
    pub fn with_callback(&self, cb: Callback) -> Source {
        Source { callback: cb, ..self.clone() }
    }
    pub fn with_dim(&self, dim: &'static str) -> Source {
        Source { dim, ..self.clone() }
    }
    /// `withEntity`: name and display name follow the entity.
    pub fn with_entity(&self, g: &Game, eid: i32) -> Source {
        let (name, display) = super::feedback::entity_names(g, eid);
        Source { entity: Some(eid), text_name: Arc::from(name.as_str()), display: Arc::new(display), ..self.clone() }
    }
}
