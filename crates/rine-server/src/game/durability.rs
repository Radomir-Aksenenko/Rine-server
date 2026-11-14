//! Item durability (`minecraft:damage`): wear from mining, melee hits and
//! armor hits, breakage, and the damage value read from a creative slot.
//!
//! Rules follow vanilla 26.3: `Item.mineBlock` (tool `damage_per_block`,
//! only where the block has hardness), `ShearsItem.mineBlock` (any non-fire
//! block), `ItemStack.postHurtEnemy` (`Weapon.item_damage_per_attack`),
//! `LivingEntity.doHurtEquipment` and `ItemStack.hurtAndBreak`. Creative and
//! spectator players never wear their items.
//!
//! Not modelled: Unbreaking and other enchantments, armor absorb and
//! toughness in the armor formula, bows, fishing rods, flint and steel,
//! shears used on entities.

use super::items;
use super::player::Stack;
use super::Game;
use crate::out;
use rine_protocol::{ProtoError, Reader};

/// Tool family, as it decides wear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    /// pickaxe, axe, shovel, hoe: `damage_per_block` 1, `item_damage_per_attack` 2.
    Digger,
    /// sword: 2 per block, 1 per attack.
    Sword,
    /// mace, trident: 2 per block, 1 per attack.
    Heavy,
    /// shears: 1 per block (not fire), no attack wear.
    Shears,
    Other,
}

fn family(item: u32) -> Family {
    let Some(info) = items::info(item) else { return Family::Other };
    let n = info.name.as_str();
    if n == "minecraft:shears" {
        Family::Shears
    } else if n == "minecraft:mace" || n == "minecraft:trident" {
        Family::Heavy
    } else if n.ends_with("_sword") {
        Family::Sword
    } else if n.ends_with("_pickaxe") || n.ends_with("_axe") || n.ends_with("_shovel") || n.ends_with("_hoe") {
        Family::Digger
    } else {
        Family::Other
    }
}

/// Armor pieces that wear when hit (`Equippable.damageOnHurt`, elytra excluded).
fn is_armor(item: u32) -> bool {
    items::info(item).is_some_and(|i| {
        let n = i.name.as_str();
        n.ends_with("_helmet") || n.ends_with("_chestplate") || n.ends_with("_leggings") || n.ends_with("_boots")
    })
}

/// Wear for one block broken with `item` (`block` is the block name, `hardness`
/// the block hardness, where `-1` means unbreakable and never reaches here).
pub fn mine_wear(item: u32, block: &str, hardness: f32) -> i32 {
    match family(item) {
        Family::Digger => i32::from(hardness != 0.0),
        Family::Sword | Family::Heavy => {
            if hardness != 0.0 {
                2
            } else {
                0
            }
        }
        Family::Shears => i32::from(block != "minecraft:fire" && block != "minecraft:soul_fire"),
        Family::Other => 0,
    }
}

/// Wear for one melee hit that lands with `item` in hand.
pub fn attack_wear(item: u32) -> i32 {
    match family(item) {
        Family::Digger => 2,
        Family::Sword | Family::Heavy => 1,
        Family::Shears | Family::Other => 0,
    }
}

/// `ItemStack.hurtAndBreak` for the inventory slot `idx`. Only survival and
/// adventure players wear items. Breaking empties the slot and sends the
/// equipment break entity event (47 main hand, 48 off hand, 49-52 head,
/// chest, legs, feet) to the player and its trackers before the slot update.
pub fn wear(g: &mut Game, slot: usize, idx: usize, amount: i32) {
    if amount <= 0 {
        return;
    }
    let Some(p) = g.player(slot) else { return };
    if !(p.gamemode == 0 || p.gamemode == 2) {
        return;
    }
    let s = p.inv.slots[idx];
    let max = match items::info(s.item) {
        Some(i) if i.max_damage > 0 && !s.is_empty() => i.max_damage,
        _ => return,
    };
    let eid = p.eid;
    let sid = p.inv.state_id;
    let total = s.damage as i32 + amount;
    if total >= max {
        p.inv.slots[idx] = Stack::EMPTY;
        let ev = out::frame(out::entity_event(eid, break_event(idx)));
        p.out.push(ev.clone());
        super::track::send_to_trackers(g, eid, &ev);
    } else {
        p.inv.slots[idx].damage = total as u16;
    }
    let Some(p) = g.player(slot) else { return };
    let it = p.inv.slots[idx];
    p.send(out::container_slot(0, sid, idx as i16, &it));
    super::track::equipment_changed(g, slot);
}

/// Equipment break event for an inventory slot (`EntityEvent.*_BREAK`).
fn break_event(idx: usize) -> i8 {
    match idx {
        45 => 48,
        5 => 49,
        6 => 50,
        7 => 51,
        8 => 52,
        _ => 47,
    }
}

/// A block was broken by `slot` with the held item (survival or adventure).
pub fn on_mine(g: &mut Game, slot: usize, block: &str, hardness: f32) {
    let Some(p) = g.player(slot) else { return };
    let idx = 36 + p.held as usize;
    let n = mine_wear(p.inv.slots[idx].item, block, hardness);
    wear(g, slot, idx, n);
}

/// A melee hit landed with the held item.
pub fn on_attack(g: &mut Game, slot: usize) {
    let Some(p) = g.player(slot) else { return };
    let idx = 36 + p.held as usize;
    let n = attack_wear(p.inv.slots[idx].item);
    wear(g, slot, idx, n);
}

/// `LivingEntity.doHurtEquipment`: each worn armor piece takes
/// `max(1, damage / 4)`. Callers skip damage that bypasses armor.
pub fn hurt_armor(g: &mut Game, slot: usize, damage: f32) {
    if damage <= 0.0 {
        return;
    }
    let n = (damage / 4.0).max(1.0) as i32;
    let Some(p) = g.player(slot) else { return };
    let worn: Vec<usize> = [5usize, 6, 7, 8].into_iter().filter(|&i| is_armor(p.inv.slots[i].item)).collect();
    for idx in worn {
        wear(g, slot, idx, n);
    }
}

/// Reads an ItemStack body from a creative slot packet (the untrusted,
/// length-prefixed component patch) and keeps the damage component.
pub fn read_creative_stack(r: &mut Reader, item: u32, count: i32) -> Result<Stack, ProtoError> {
    let added = r.varint()?;
    let removed = r.varint()?;
    let damage_id = out::damage_component_id();
    let mut damage = 0i32;
    for _ in 0..added.clamp(0, 256) {
        let ty = r.varint()?;
        let len = r.varint()?;
        if len < 0 {
            return Err(ProtoError::Invalid("negative component length"));
        }
        let body = r.bytes(len as usize)?;
        if ty >= 0 && ty as u32 == damage_id {
            let mut br = Reader { data: body, pos: 0 };
            damage = br.varint()?;
        }
    }
    for _ in 0..removed.clamp(0, 256) {
        r.varint()?;
    }
    let max = items::info(item).map_or(0, |i| i.max_damage);
    let damage = if max > 0 { damage.clamp(0, max - 1) as u16 } else { 0 };
    Ok(Stack::damaged(item, count.min(99), damage))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rine_protocol::Writer;
    use bytes::BytesMut;

    fn id(name: &str) -> u32 {
        rine_data::init();
        items::id(name).unwrap_or_else(|| panic!("no item {name}"))
    }

    #[test]
    fn mining_wear_follows_vanilla_families() {
        let stone = "minecraft:stone";
        assert_eq!(mine_wear(id("minecraft:iron_pickaxe"), stone, 1.5), 1);
        assert_eq!(mine_wear(id("minecraft:iron_pickaxe"), "minecraft:grass_block", 0.0), 0);
        assert_eq!(mine_wear(id("minecraft:iron_sword"), stone, 1.5), 2);
        assert_eq!(mine_wear(id("minecraft:iron_sword"), "minecraft:grass_block", 0.0), 0);
        assert_eq!(mine_wear(id("minecraft:shears"), "minecraft:oak_leaves", 0.0), 1);
        assert_eq!(mine_wear(id("minecraft:shears"), "minecraft:fire", 0.0), 0);
        assert_eq!(mine_wear(id("minecraft:stick"), stone, 1.5), 0);
    }

    #[test]
    fn attack_wear_follows_weapon_component() {
        assert_eq!(attack_wear(id("minecraft:iron_axe")), 2);
        assert_eq!(attack_wear(id("minecraft:iron_pickaxe")), 2);
        assert_eq!(attack_wear(id("minecraft:iron_sword")), 1);
        assert_eq!(attack_wear(id("minecraft:stick")), 0);
    }

    #[test]
    fn break_event_codes_match_equipment_slots() {
        assert_eq!(break_event(36), 47);
        assert_eq!(break_event(45), 48);
        assert_eq!(break_event(5), 49);
        assert_eq!(break_event(8), 52);
    }

    #[test]
    fn creative_patch_keeps_damage_and_skips_other_components() {
        let damage_id = out::damage_component_id();
        let mut buf = BytesMut::new();
        buf.w_varint(2); // added
        buf.w_varint(0); // removed
        buf.w_varint(9999); // unknown component, length-prefixed body
        buf.w_varint(1);
        buf.w_u8(0x01);
        buf.w_varint(damage_id as i32);
        buf.w_varint(1);
        buf.w_varint(5);
        let mut r = Reader::new(&buf);
        let s = read_creative_stack(&mut r, id("minecraft:iron_pickaxe"), 1).unwrap();
        assert_eq!(s.damage, 5);
        assert_eq!(s.count, 1);
        assert_eq!(r.pos, buf.len());
    }
}
