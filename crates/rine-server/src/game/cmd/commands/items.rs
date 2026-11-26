//! /give, /clear, /enchant (`GiveCommand`, `ClearInventoryCommands`,
//! `EnchantCommand`).
//!
//! Item stacks only carry id, count and wear (`crate::stack::Stack`). Data
//! components and enchantments are not stored, so `/enchant` cannot
//! succeed yet and item predicates cannot test components.

use crate::game::cmd::{feedback, lit, select, Commands, Ctx};
use crate::game::entity;
use crate::game::player::{max_stack, Player};
use crate::stack::Stack;
use crate::game::track;
use crate::game::Game;
use rine_command::args::item::{ItemMatch, ItemPredicate, ItemTest};
use rine_command::args::Arg;
use rine_command::{CmdError, CmdResult};
use rine_protocol::text::{hover_item, hover_text, Text};
use std::f64::consts::TAU;

/// `SoundSource.PLAYERS`.
const SOUND_PLAYERS: i32 = 7;

pub fn register(c: &mut Commands) {
    c.exec("give targets item", give);
    c.exec("give targets item count", give);
    c.exec("clear", clear_self);
    c.exec("clear targets", clear_targets);
    c.exec("clear targets item", clear_item);
    c.exec("clear targets item maxCount", clear_item_max);
    c.exec("enchant targets enchantment", enchant);
    c.exec("enchant targets enchantment level", enchant);
}

/// `Item.getName()` in brackets with the item hover (`ItemStack.getDisplayName`).
/// Rarity is not in the item tables, so every item uses the common colour.
fn item_display(item: u32) -> Text {
    let name = rine_data::get().items.get(item).map(|i| i.name.as_str()).unwrap_or("minecraft:air");
    let inner = Text::empty().push(Text::key(feedback::item_description_id(name)));
    Text::tr("chat.square_brackets", vec![inner]).color("white").on_hover(hover_item(name, 1, None))
}

fn item_id(name: &str) -> CmdResult<u32> {
    crate::game::items::id(name).ok_or_else(|| CmdError::tr("argument.item.id.invalid", vec![lit(name)]))
}

/// Pseudo-random value in [0, 1) from the tick and a per-call salt.
fn rnd(g: &Game, salt: u64) -> f64 {
    entity::rand_f64(g.tick.wrapping_mul(0x9E37_79B9).wrapping_add(salt))
}

/// `Player.playSound(ITEM_PICKUP)` at the player's position.
fn pickup_sound(g: &mut Game, slot: usize, k: u64) {
    let Some(pos) = g.players.get(slot).and_then(|p| p.as_ref()).map(|p| p.pos) else { return };
    let pitch = ((rnd(g, k) as f32 - rnd(g, k + 1) as f32) * 0.7 + 1.0) * 2.0;
    track::sound_near(g, "minecraft:entity.item.pickup", SOUND_PLAYERS, pos, 0.2, pitch, None);
}

/// `createItemStackToDrop(stack, false, false)` followed by
/// `setNoPickUpDelay()`: an item thrown from the player's hand, no delay.
/// The owner (`setTarget`) is not modelled.
fn drop_overflow(g: &mut Game, slot: usize, item: u32, count: i32) {
    let Some(p) = g.players.get(slot).and_then(|p| p.as_ref()) else { return };
    let eye_y = p.eye()[1];
    let pos = [p.pos[0], eye_y - 0.3, p.pos[2]];
    let xr = (p.pitch as f64).to_radians();
    let yr = (p.yaw as f64).to_radians();
    let (sin_x, cos_x) = (xr.sin(), xr.cos());
    let (sin_y, cos_y) = (yr.sin(), yr.cos());
    let base = g.tick.wrapping_mul(0x2545_F491).wrapping_add(slot as u64 * 131);
    let dir = rnd(g, base) * TAU;
    let pow2 = 0.02 * rnd(g, base + 1);
    let vel = [
        -sin_y * cos_x * 0.3 + dir.cos() * pow2,
        -sin_x * 0.3 + 0.1 + (rnd(g, base + 2) - rnd(g, base + 3)) * 0.1,
        cos_y * cos_x * 0.3 + dir.sin() * pow2,
    ];
    entity::spawn_item(g, pos, vel, Stack::new(item, count), 0);
}

/// `GiveCommand.giveItem`: stacks split by the max stack size, a full
/// insert plays the pickup sound, the rest drops at the player.
fn give_to(g: &mut Game, slot: usize, item: u32, count: i32) {
    let max = max_stack(item);
    let mut remaining = count;
    let mut touched = false;
    while remaining > 0 {
        let size = max.min(remaining);
        remaining -= size;
        let left = match g.player(slot) {
            Some(p) => p.inv.add(Stack::new(item, size)),
            None => return,
        };
        touched |= left < size;
        if left == 0 {
            pickup_sound(g, slot, 1);
        } else {
            drop_overflow(g, slot, item, left);
        }
    }
    if touched {
        sync_inventory(g, slot);
    }
}

/// `containerMenu.broadcastChanges()`: full inventory content to the
/// player and the held item to the others.
fn sync_inventory(g: &mut Game, slot: usize) {
    if let Some(p) = g.player(slot) {
        let f = p.inventory_packet();
        p.send(f);
    }
    track::equipment_changed(g, slot);
}

fn give(x: &mut Ctx) -> CmdResult<i32> {
    let targets = x.players("targets")?;
    let name = match x.arg("item") {
        Arg::Item(i) => i.item.clone(),
        _ => return Err(CmdError::key("argument.item.id.invalid")),
    };
    let count = x.int_or("count", 1);
    let item = item_id(&name)?;
    let max = max_stack(item);
    let max_allowed = max * 100;
    if count > max_allowed {
        return Err(CmdError::tr("commands.give.failed.toomanyitems", vec![lit(max_allowed), item_display(item)]));
    }
    for &slot in &targets {
        give_to(x.g, slot, item, count);
    }
    let n = targets.len() as i32;
    let display = item_display(item);
    let msg = if n == 1 {
        Text::tr("commands.give.success.single", vec![lit(count), display, x.player_name(targets[0])])
    } else {
        Text::tr("commands.give.success.multiple", vec![lit(count), display, lit(n)])
    };
    x.success(msg, true);
    Ok(n)
}

/// Item predicate with `None` for "any item" (plain `/clear`).
#[derive(Clone)]
enum Filter {
    Any,
    Pred(ItemPredicate),
}

fn norm_item(id: &str) -> String {
    if id.contains(':') {
        id.to_string()
    } else {
        format!("minecraft:{id}")
    }
}

/// Whether a test passes. Component and sub-predicate tests cannot be
/// evaluated (stacks carry no components): a positive test fails, a
/// negated one passes.
fn test_passes(t: &ItemTest) -> bool {
    match t {
        ItemTest::Component { negated, .. } | ItemTest::Predicate { negated, .. } => *negated,
        ItemTest::Any(v) => v.iter().any(test_passes),
    }
}

fn matches(f: &Filter, s: &Stack) -> bool {
    let p = match f {
        Filter::Any => return true,
        Filter::Pred(p) => p,
    };
    let data = rine_data::get();
    let name = match data.items.get(s.item) {
        Some(i) => i.name.as_str(),
        None => return false,
    };
    let item_ok = match &p.item {
        ItemMatch::Any => true,
        ItemMatch::Item(id) => norm_item(id) == name,
        ItemMatch::Tag(t) => data.registries.tag_contains("minecraft:item", t.trim_start_matches('#'), name),
    };
    item_ok && p.tests.iter().all(test_passes)
}

/// `ContainerHelper.clearOrCountMatchingItems` for one stack. `max < 0`
/// is unlimited, `counting` only reports the matching count.
fn take(stack: &mut Stack, f: &Filter, counting: bool, max: i32, so_far: i32) -> i32 {
    if stack.is_empty() || !matches(f, stack) {
        return 0;
    }
    if counting {
        return stack.count;
    }
    let amount = if max < 0 { stack.count } else { (max - so_far).min(stack.count).max(0) };
    *stack = stack.with_count(stack.count - amount);
    amount
}

/// `Inventory.clearOrCountMatchingItems`: hotbar, main, armor (feet first),
/// offhand, then the craft grid and the cursor.
fn clear_player(p: &mut Player, f: &Filter, counting: bool, max: i32) -> i32 {
    let mut count = 0;
    let order = (36..45).chain(9..36).chain([8, 7, 6, 5, 45]).chain(1..5);
    for i in order {
        let amount = take(&mut p.inv.slots[i], f, counting, max, count);
        count += amount;
    }
    count += take(&mut p.inv.cursor, f, counting, max, count);
    count
}

fn clear_self(x: &mut Ctx) -> CmdResult<i32> {
    let slot = x.source_player()?;
    clear_inventory(x, &[slot], Filter::Any, -1)
}

fn clear_targets(x: &mut Ctx) -> CmdResult<i32> {
    let slots = x.players("targets")?;
    clear_inventory(x, &slots, Filter::Any, -1)
}

fn clear_item(x: &mut Ctx) -> CmdResult<i32> {
    let slots = x.players("targets")?;
    let f = predicate_arg(x)?;
    clear_inventory(x, &slots, f, -1)
}

fn clear_item_max(x: &mut Ctx) -> CmdResult<i32> {
    let slots = x.players("targets")?;
    let f = predicate_arg(x)?;
    let max = x.int("maxCount");
    clear_inventory(x, &slots, f, max)
}

fn predicate_arg(x: &Ctx) -> CmdResult<Filter> {
    match x.arg("item") {
        Arg::ItemPredicate(p) => Ok(Filter::Pred((**p).clone())),
        _ => Err(CmdError::key("argument.item.id.invalid")),
    }
}

/// `ClearInventoryCommands.clearInventory`: `max == 0` only counts and
/// sends the test messages.
fn clear_inventory(x: &mut Ctx, slots: &[usize], f: Filter, max: i32) -> CmdResult<i32> {
    let counting = max == 0;
    let mut tracked: Vec<(usize, i32)> = Vec::with_capacity(slots.len());
    let mut total = 0;
    for &slot in slots {
        let value = match x.g.player(slot) {
            Some(p) => clear_player(p, &f, counting, max),
            None => 0,
        };
        if !counting {
            sync_inventory(x.g, slot);
        }
        total += value;
        tracked.push((slot, value));
    }

    if total == 0 {
        // ElementType.ANY: the single element is named, else the count.
        if tracked.len() == 1 {
            return Err(CmdError::tr("clear.failed.single", vec![x.player_name(tracked[0].0)]));
        }
        return Err(CmdError::tr("clear.failed.multiple", vec![lit(tracked.len())]));
    }

    // ElementType.NON_ZERO
    let nonzero: Vec<usize> = tracked.iter().filter(|(_, v)| *v != 0).map(|(s, _)| *s).collect();
    let msg = match (counting, nonzero.len()) {
        (true, 1) => Text::tr("commands.clear.test.single", vec![lit(total), x.player_name(nonzero[0])]),
        (true, n) => Text::tr("commands.clear.test.multiple", vec![lit(total), lit(n)]),
        (false, 1) => Text::tr("commands.clear.success.single", vec![lit(total), x.player_name(nonzero[0])]),
        (false, n) => Text::tr("commands.clear.success.multiple", vec![lit(total), lit(n)]),
    };
    x.success(msg, true);
    Ok(total)
}

/// Error for an enchant that would succeed: stacks carry no enchantments.
fn enchant_unmodelled() -> CmdError {
    CmdError::new(Text::key("command.failed").on_hover(hover_text(&Text::plain("Rine: enchantments are not stored on item stacks yet"))))
}

/// `EnchantCommand.enchant` checks, up to the point where a stack would be
/// enchanted. Level limits and enchantment compatibility need enchantment
/// definitions, which are not loaded, so a non-empty main hand is treated
/// as "would enchant" and reported as unmodelled.
fn enchant(x: &mut Ctx) -> CmdResult<i32> {
    let eids = x.entities("targets")?;
    let single = eids.len() == 1;
    let mut would_enchant = 0;
    for &eid in &eids {
        match x.g.by_eid.get(&eid).copied() {
            Some(slot) => {
                let held = x.g.players.get(slot).and_then(|p| p.as_ref()).map(|p| p.inv.hotbar(p.held)).unwrap_or(Stack::EMPTY);
                if held.is_empty() {
                    if single {
                        return Err(CmdError::tr("commands.enchant.failed.itemless", vec![lit(select::plain_name(x.g, eid))]));
                    }
                } else {
                    would_enchant += 1;
                }
            }
            None => {
                if single {
                    return Err(CmdError::tr("commands.enchant.failed.entity", vec![lit(select::plain_name(x.g, eid))]));
                }
            }
        }
    }
    if would_enchant > 0 {
        return Err(enchant_unmodelled());
    }
    // ERROR_NOTHING_HAPPENED
    Err(CmdError::key("commands.enchant.failed"))
}
