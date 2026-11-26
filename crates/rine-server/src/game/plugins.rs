//! Glue between the game loop and the JVM plugin host.
//!
//! Events are posted without blocking; the host ships them to the plugin
//! thread in one batch per tick. Synchronous (cancellable) calls happen only
//! for event kinds some plugin actually subscribed to.

use crate::stack::Stack;
use super::Game;
use rine_plugin::{Action, Event, PluginHost, SyncKind};
use rine_protocol::text::Text;
use rine_protocol::BlockPos;

pub fn start(dir: &str) -> Option<PluginHost> {
    let _ = std::fs::create_dir_all(dir);
    match PluginHost::start(std::path::Path::new(dir)) {
        Ok(h) => {
            if h.is_some() {
                tracing::info!("plugins: JVM started");
            }
            h
        }
        Err(e) => {
            tracing::warn!("plugins disabled: {e}");
            None
        }
    }
}

pub fn post(g: &Game, e: Event) {
    if let Some(h) = &g.plugins {
        h.post(e);
    }
}

/// Returns true if a plugin cancelled the event. If nobody listens
/// synchronously, the event is posted asynchronously and false returned.
pub fn check(g: &Game, kind: SyncKind, e: Event) -> bool {
    let Some(h) = &g.plugins else { return false };
    if h.wants_sync(kind) {
        h.sync_cancel(&e)
    } else {
        h.post(e);
        false
    }
}

pub fn has_command(g: &Game, name: &str) -> bool {
    g.plugins.as_ref().map(|h| h.commands().iter().any(|c| c.eq_ignore_ascii_case(name))).unwrap_or(false)
}

/// Apply actions produced by plugins since the last tick.
pub fn tick(g: &mut Game) {
    let actions = match &g.plugins {
        Some(h) => h.tick(),
        None => return,
    };
    for a in actions {
        match a {
            Action::Broadcast(s) => g.broadcast_msg(&Text::plain(s)),
            Action::Message { player, text } => {
                if let Some(s) = g.find_player(&player) {
                    g.player(s).unwrap().msg(&Text::plain(text));
                }
            }
            Action::SetBlock { x, y, z, block } => {
                if let Some(st) = rine_data::get().blocks.parse_state(&block) {
                    g.world.set_block(BlockPos::new(x, y, z), st);
                }
            }
            Action::Teleport { player, x, y, z } => {
                if let Some(s) = g.find_player(&player) {
                    let p = g.player(s).unwrap();
                    let (yaw, pitch) = (p.yaw, p.pitch);
                    p.teleport(x, y, z, yaw, pitch);
                }
            }
            Action::Give { player, item, count } => {
                if let (Some(s), Some(id)) = (g.find_player(&player), super::items::id(&item)) {
                    let p = g.player(s).unwrap();
                    let left = p.inv.add(crate::stack::Stack::new(id, count.max(1)));
                    let inv = p.inventory_packet();
                    p.send(inv);
                    if left > 0 {
                        super::entity::drop_from_player(g, s, Stack::new(id, left.min(64)), false);
                    }
                }
            }
            Action::Kick { player, reason } => {
                if let Some(s) = g.find_player(&player) {
                    g.player(s).unwrap().kick = Some(reason);
                }
            }
            Action::RunCommand { player, line } => {
                if let Some(s) = g.find_player(&player) {
                    super::cmd::player_command(g, s, line.trim_start_matches('/'));
                }
            }
        }
    }
}

pub fn block_name(state: u16) -> String {
    rine_data::get().blocks.block_of(state).name.clone()
}
