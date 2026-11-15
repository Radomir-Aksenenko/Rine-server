//! Player actions: middle-click pick (vanilla
//! `ServerGamePacketListenerImpl.tryPickItem`) and the F3+F4 game mode
//! switcher (`handleChangeGameMode`).

use super::entity::{Ent, Kind};
use super::player::{Inventory, Stack};
use super::{track, Game};
use crate::out;
use rine_protocol::BlockPos;

/// Window-0 slot of the first hotbar slot (vanilla `items[0]`).
const HOTBAR: usize = 36;
/// Vanilla `Player.blockInteractionRange` (survival / creative) plus the 1.0
/// extra that `handlePickItemFromBlock` passes to `isWithinBlockInteractionRange`.
const BLOCK_REACH_SURVIVAL: f64 = 4.5 + 1.0;
const BLOCK_REACH_CREATIVE: f64 = 5.0 + 1.0;
/// Vanilla `entityInteractionRange` (3.0 survival, 5.0 creative) plus the 3.0
/// extra that `handlePickItemFromEntity` passes.
const ENTITY_REACH_SURVIVAL: f64 = 3.0 + 3.0;
const ENTITY_REACH_CREATIVE: f64 = 5.0 + 3.0;

// ---------------------------------------------------------------- pick item

/// Vanilla `Inventory.getSuitableHotbarSlot`: first empty hotbar slot scanning
/// from the selected one and wrapping; otherwise the selected slot. Rine has no
/// enchantment data, so vanilla's second loop always lands on `held`.
fn suitable_hotbar(inv: &Inventory, held: u8) -> u8 {
    for k in 0..9u8 {
        let idx = (held + k) % 9;
        if inv.slots[HOTBAR + idx as usize].is_empty() {
            return idx;
        }
    }
    held
}

/// Vanilla `Inventory.items` index (0..9 hotbar, 9..36 main) to window slot.
#[inline]
fn vanilla_slot(i: usize) -> usize {
    if i < 9 {
        HOTBAR + i
    } else {
        i
    }
}

/// Vanilla `Inventory.findSlotMatchingItem`: scans hotbar then main, never offhand or armor.
fn find_matching(inv: &Inventory, item: u32) -> Option<usize> {
    (0..36).map(vanilla_slot).find(|&s| !inv.slots[s].is_empty() && inv.slots[s].item == item)
}

/// Core of `tryPickItem` for the non-entity-data case. Updates `inv` and the
/// selected hotbar index. Returns `(new_held, inventory_changed)`.
///
/// Found in hotbar: just select it. Found in main: swap it into the suitable
/// hotbar slot and select that. Not found: creative puts the stack into the
/// suitable slot (moving the old occupant to the first free slot); survival
/// changes nothing.
pub fn pick_into(inv: &mut Inventory, held: u8, stack: Stack, creative: bool) -> (u8, bool) {
    if let Some(src) = find_matching(inv, stack.item) {
        if src >= HOTBAR {
            return ((src - HOTBAR) as u8, false);
        }
        let target = suitable_hotbar(inv, held);
        inv.slots.swap(HOTBAR + target as usize, src);
        return (target, true);
    }
    if !creative {
        return (held, false);
    }
    let target = suitable_hotbar(inv, held);
    let dst = HOTBAR + target as usize;
    if !inv.slots[dst].is_empty() {
        // vanilla addAndPickItem: move the old stack to the first free slot in items order
        if let Some(free) = (0..36).map(vanilla_slot).find(|&s| inv.slots[s].is_empty()) {
            inv.slots[free] = inv.slots[dst];
        }
    }
    inv.slots[dst] = Stack::new(stack.item, stack.count);
    (target, true)
}

/// Apply a pick result to the player and resync the client: held slot always,
/// full inventory when it changed, equipment to trackers when the hand changed.
fn apply_pick(g: &mut Game, slot: usize, stack: Stack) {
    let Some(p) = g.player(slot) else { return };
    let creative = p.gamemode == 1;
    let (nh, changed) = pick_into(&mut p.inv, p.held, stack, creative);
    let held_changed = nh != p.held;
    p.held = nh;
    if changed {
        let inv = p.inventory_packet();
        p.send(inv);
    }
    p.send(out::set_held_slot(nh as i32));
    if changed || held_changed {
        track::equipment_changed(g, slot);
    }
}

/// Serverbound `pick_item_from_block` (middle click on a block).
pub fn pick_block(g: &mut Game, slot: usize, pos: BlockPos, _include_data: bool) {
    let Some(p) = g.player(slot) else { return };
    let creative = p.gamemode == 1;
    let reach = if creative { BLOCK_REACH_CREATIVE } else { BLOCK_REACH_SURVIVAL };
    let eye = p.eye();
    if dist2_to_block(eye, pos) > reach * reach {
        return;
    }
    // unloaded chunks read as air, which has no item
    let st = g.world.get_block(pos.x, pos.y, pos.z);
    let item = rine_data::get().blocks.state(st).item;
    if item == 0 {
        return;
    }
    // Block-entity NBT (includeData) is not modelled; the picked stack is a plain count-1 item.
    apply_pick(g, slot, Stack::new(item, 1));
}

/// Serverbound `pick_item_from_entity` (middle click on an entity).
pub fn pick_entity(g: &mut Game, slot: usize, eid: i32, _include_data: bool) {
    let Some(p) = g.player(slot) else { return };
    let creative = p.gamemode == 1;
    let reach = if creative { ENTITY_REACH_CREATIVE } else { ENTITY_REACH_SURVIVAL };
    let eye = p.eye();
    let Some(e) = g.entities.get(eid) else { return };
    let Some(stack) = entity_pick_result(e) else { return };
    let m = e.mid();
    let d2 = (m[0] - eye[0]).powi(2) + (m[1] - eye[1]).powi(2) + (m[2] - eye[2]).powi(2);
    if d2 > reach * reach {
        return;
    }
    apply_pick(g, slot, stack);
}

/// Vanilla `Entity.getPickResult`. The base class returns null, and none of
/// rine's entity kinds (item, TNT, falling block) override it, so nothing is
/// picked yet. Players and mobs do not exist in rine's entity list.
fn entity_pick_result(e: &Ent) -> Option<Stack> {
    match e.kind {
        Kind::Item { .. } | Kind::Tnt { .. } | Kind::Falling { .. } => None,
    }
}

/// Squared distance from `eye` to the closest point of the unit block cube at `pos`.
fn dist2_to_block(eye: [f64; 3], pos: BlockPos) -> f64 {
    let c = [pos.x as f64, pos.y as f64, pos.z as f64];
    let mut d = 0.0;
    for i in 0..3 {
        let v = eye[i].clamp(c[i], c[i] + 1.0);
        d += (eye[i] - v) * (eye[i] - v);
    }
    d
}

// ----------------------------------------------------------- game mode switch

/// Serverbound `change_game_mode` (F3+F4). Vanilla requires permission level 2;
/// operators in rine are level 4, so `Player.op` is the check. The codec maps
/// unknown ids to survival (`GameType.BY_ID` with `OutOfBoundsStrategy.ZERO`).
pub fn change_game_mode(g: &mut Game, slot: usize, mode: i32) {
    let Some(p) = g.player(slot) else { return };
    let name = p.name.clone();
    if g.op_level_of(&name) < 2 {
        return;
    }
    let gm = if (0..=3).contains(&mode) { mode as u8 } else { 0 };
    super::cmd::commands::gamemode::set_self(g, slot, gm);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inv_with(entries: &[(usize, u32, i32)]) -> Inventory {
        let mut inv = Inventory::default();
        for &(s, item, n) in entries {
            inv.slots[s] = Stack::new(item, n);
        }
        inv
    }

    #[test]
    fn hotbar_hit_only_selects() {
        let mut inv = inv_with(&[(38, 7, 3)]);
        let before = inv.slots;
        let (held, changed) = pick_into(&mut inv, 0, Stack::new(7, 1), true);
        assert_eq!((held, changed), (2, false));
        assert_eq!(inv.slots, before);
    }

    #[test]
    fn main_hit_moves_into_suitable_hotbar_slot() {
        // held slot 0 is occupied, slot 1 is empty: the main stack goes to hotbar 1
        let mut inv = inv_with(&[(36, 5, 1), (20, 7, 9)]);
        let (held, changed) = pick_into(&mut inv, 0, Stack::new(7, 1), false);
        assert_eq!((held, changed), (1, true));
        assert_eq!(inv.slots[37], Stack::new(7, 9));
        assert_eq!(inv.slots[20], Stack::EMPTY);
        assert_eq!(inv.slots[36], Stack::new(5, 1));
    }

    #[test]
    fn creative_miss_with_full_hotbar_replaces_selected_and_moves_old_item() {
        let mut entries: Vec<(usize, u32, i32)> = (0..9).map(|i| (36 + i, 100 + i as u32, 1)).collect();
        entries.push((10, 0, 0));
        let mut inv = inv_with(&entries[..9]);
        let (held, changed) = pick_into(&mut inv, 4, Stack::new(55, 1), true);
        assert_eq!((held, changed), (4, true));
        assert_eq!(inv.slots[40], Stack::new(55, 1));
        // old selected item goes to the first empty slot in vanilla order: main 9
        assert_eq!(inv.slots[9], Stack::new(104, 1));
    }

    #[test]
    fn survival_miss_changes_nothing() {
        let mut inv = inv_with(&[(36, 5, 1)]);
        let before = inv.slots;
        let (held, changed) = pick_into(&mut inv, 3, Stack::new(9, 1), false);
        assert_eq!((held, changed), (3, false));
        assert_eq!(inv.slots, before);
    }
}
