//! Windows and containers: player inventory clicks, crafting (2x2 and
//! crafting table), chests, furnaces. (Stub: implemented by the containers task.)
use super::Game;
use rine_protocol::{BlockPos, ProtoError, Reader};

/// Serverbound container_click. Must fully consume the packet body.
pub fn click(g: &mut Game, slot: usize, r: &mut Reader) -> Result<(), ProtoError> {
    super::interact::container_click(g, slot, r)
}

pub fn close(g: &mut Game, slot: usize, _window: i32) {
    super::interact::container_close(g, slot);
}

/// Right-click on a block that opens a window. Return true if handled.
pub fn open_block(_g: &mut Game, _slot: usize, _pos: BlockPos, _state: u16) -> bool {
    false
}

/// Called once per tick (furnace smelting etc.).
pub fn tick(_g: &mut Game) {}

/// A block was removed: drop its container contents.
pub fn block_removed(_g: &mut Game, _pos: BlockPos, _old_state: u16) {}
