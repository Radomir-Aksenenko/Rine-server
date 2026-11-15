//! Container blocks around the windows in `containers`: which window a
//! block opens (vanilla menus and titles, double chests, ender chests,
//! blocked chests and shulker boxes), lid animation and sounds, the block
//! entity NBT the containers are stored in, hoppers (`HopperBlockEntity`)
//! and droppers / dispensers (`DispenserBlock.dispenseFrom`).
//!
//! Typed contents live in `containers::State` while the chunk is loaded;
//! changed ones are written to the chunk's block entity NBT at the end of
//! every tick, so saves and `/data` always see the current items.

use super::blockent;
use super::containers::{self, lock_state, read_src, write_src, Fk, Furnace, Lay, Src, State};
use super::entity::{self, Kind};
use super::player::{max_stack, Stack};
use super::Game;
use crate::out;
use ahash::AHashMap;
use rine_data::blocks::flag;
use rine_protocol::nbt::Tag;
use rine_protocol::text::Text;
use rine_protocol::BlockPos;
use rine_world::ChunkPos;
use std::sync::atomic::{AtomicU64, Ordering};

/// `HopperBlockEntity` timing state.
#[derive(Clone, Copy, Debug, Default)]
pub struct Hopper {
    pub cooldown: i32,
    /// Game time of the last `pushItemsTick`.
    pub ticked: u64,
}

pub(super) enum Opened {
    /// The click is used, nothing opens (blocked chest / shulker box).
    Blocked,
    Menu(Menu),
}

pub(super) struct Menu {
    pub lay: Lay,
    pub title: Text,
    pub menu: &'static str,
    pub src: Src,
}

fn blocks() -> &'static rine_data::blocks::Blocks {
    &rine_data::get().blocks
}

fn short(state: u16) -> &'static str {
    let n = blocks().block_of(state).name.as_str();
    n.strip_prefix("minecraft:").unwrap_or(n)
}

pub fn is_chest(name: &str) -> bool {
    name == "chest" || name == "trapped_chest" || name.ends_with("copper_chest")
}

/// Slot count of a block stored in `State::chests`, and whether it is a
/// shulker box.
pub fn store_size(name: &str) -> Option<(usize, bool)> {
    Some(match name {
        n if is_chest(n) => (27, false),
        "barrel" => (27, false),
        "dispenser" | "dropper" => (9, false),
        "hopper" => (5, false),
        n if n.ends_with("shulker_box") => (27, true),
        _ => return None,
    })
}

fn furnace_kind(name: &str) -> Option<Fk> {
    Some(match name {
        "furnace" => Fk::Furnace,
        "smoker" => Fk::Smoker,
        "blast_furnace" => Fk::Blast,
        _ => return None,
    })
}

fn dir_of(name: &str) -> [i32; 3] {
    match name {
        "down" => [0, -1, 0],
        "up" => [0, 1, 0],
        "north" => [0, 0, -1],
        "south" => [0, 0, 1],
        "west" => [-1, 0, 0],
        _ => [1, 0, 0],
    }
}

fn rel(p: BlockPos, d: [i32; 3]) -> BlockPos {
    p.offset(d[0], d[1], d[2])
}

/// `ChestBlock.getConnectedDirection` as a step.
fn connected_dir(state: u16) -> Option<[i32; 3]> {
    let b = blocks();
    let ty = b.get_prop(state, "type")?;
    if ty == "single" {
        return None;
    }
    let f = dir_of(b.get_prop(state, "facing")?);
    // LEFT: facing.getClockWise(), RIGHT: facing.getCounterClockWise()
    let cw = [-f[2], 0, f[0]];
    let ccw = [f[2], 0, -f[0]];
    Some(if ty == "left" { cw } else { ccw })
}

/// The other half of a double chest when it is really there.
pub fn chest_partner(pos: BlockPos, state: u16) -> Option<BlockPos> {
    if !is_chest(short(state)) {
        return None;
    }
    let d = connected_dir(state)?;
    Some(rel(pos, d))
}

fn chest_partner_live(g: &Game, pos: BlockPos, state: u16) -> Option<(BlockPos, u16)> {
    let p = chest_partner(pos, state)?;
    let ps = g.world.get_block(p.x, p.y, p.z);
    let b = blocks();
    if b.block_of(ps).id != b.block_of(state).id {
        return None;
    }
    let (a, c) = (b.get_prop(state, "type")?, b.get_prop(ps, "type")?);
    let ok = (a == "left" && c == "right") || (a == "right" && c == "left");
    (ok && connected_dir(ps).is_some_and(|d| rel(p, d) == pos)).then_some((p, ps))
}

/// `ChestBlock.isChestBlockedAt` (cats are not modelled).
fn chest_blocked(g: &Game, pos: BlockPos) -> bool {
    let above = g.world.get_block(pos.x, pos.y + 1, pos.z);
    super::redstone::data::conductor(above)
}

/// `CustomName` of the block entity at `pos`.
fn custom_name(g: &Game, pos: BlockPos) -> Option<Text> {
    let slot = g.world.chunks.get(&ChunkPos::new(pos.x >> 4, pos.z >> 4))?;
    let t = blockent::find(&slot.chunk, pos)?.nbt.as_ref()?.get("CustomName")?;
    Some(super::cmd::select::text_from_tag(t))
}

fn title(g: &Game, pos: BlockPos, key: &'static str) -> Text {
    custom_name(g, pos).unwrap_or_else(|| Text::key(key))
}

/// The window a right click on `state` opens.
pub(super) fn menu_for(g: &Game, slot: usize, pos: BlockPos, state: u16) -> Option<Opened> {
    let name = short(state);
    let store = |n: u8| Lay::Store { n, shulker: false };
    let m = |lay, title, menu, src| Some(Opened::Menu(Menu { lay, title, menu, src }));
    match name {
        "crafting_table" => m(Lay::Table, Text::key("container.crafting"), "minecraft:crafting", Src::None),
        "furnace" => m(Lay::Furnace(Fk::Furnace), title(g, pos, "container.furnace"), "minecraft:furnace", Src::One(pos)),
        "smoker" => m(Lay::Furnace(Fk::Smoker), title(g, pos, "container.smoker"), "minecraft:smoker", Src::One(pos)),
        "blast_furnace" => m(Lay::Furnace(Fk::Blast), title(g, pos, "container.blast_furnace"), "minecraft:blast_furnace", Src::One(pos)),
        "barrel" => m(store(27), title(g, pos, "container.barrel"), "minecraft:generic_9x3", Src::One(pos)),
        "dispenser" => m(store(9), title(g, pos, "container.dispenser"), "minecraft:generic_3x3", Src::One(pos)),
        "dropper" => m(store(9), title(g, pos, "container.dropper"), "minecraft:generic_3x3", Src::One(pos)),
        "hopper" => m(store(5), title(g, pos, "container.hopper"), "minecraft:hopper", Src::One(pos)),
        "ender_chest" => {
            if chest_blocked(g, pos) {
                return Some(Opened::Blocked);
            }
            let uuid = g.players.get(slot)?.as_ref()?.uuid;
            m(store(27), Text::key("container.enderchest"), "minecraft:generic_9x3", Src::Ender(uuid))
        }
        n if n.ends_with("shulker_box") => {
            // `ShulkerBoxBlock.canOpen` for a closed box: the block it opens
            // towards must leave room for the lid
            let f = dir_of(blocks().get_prop(state, "facing").unwrap_or("up"));
            let front = rel(pos, f);
            let fs = g.world.get_block(front.x, front.y, front.z);
            if blocks().state(fs).flags & flag::SOLID != 0 {
                return Some(Opened::Blocked);
            }
            m(Lay::Store { n: 27, shulker: true }, title(g, pos, "container.shulkerBox"), "minecraft:shulker_box", Src::One(pos))
        }
        n if is_chest(n) => {
            if chest_blocked(g, pos) {
                return Some(Opened::Blocked);
            }
            match chest_partner_live(g, pos, state) {
                None => m(store(27), title(g, pos, "container.chest"), "minecraft:generic_9x3", Src::One(pos)),
                Some((p, _)) => {
                    if chest_blocked(g, p) {
                        return Some(Opened::Blocked);
                    }
                    // the RIGHT half is `FIRST` (top rows)
                    let right = blocks().get_prop(state, "type") == Some("right");
                    let (first, second) = if right { (pos, p) } else { (p, pos) };
                    let t = custom_name(g, first)
                        .or_else(|| custom_name(g, second))
                        .unwrap_or_else(|| Text::key("container.chestDouble"));
                    m(store(54), t, "minecraft:generic_9x6", Src::Two(first, second))
                }
            }
        }
        _ => None,
    }
}

/// Blocks whose opener counters an opened window changes.
pub(super) fn lid_blocks(pos: BlockPos, src: Src) -> Vec<BlockPos> {
    match src {
        Src::Two(a, b) => vec![a, b],
        _ => vec![pos],
    }
}

static RNG: AtomicU64 = AtomicU64::new(0x2545_F491_4F6C_DD1D);

/// Uniform in [0, 1).
fn rand() -> f64 {
    let mut z = RNG.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

fn rand_int(n: u32) -> u32 {
    ((rand() * n as f64) as u32).min(n.saturating_sub(1))
}

fn sound_at(g: &mut Game, name: &str, at: [f64; 3], vol: f32, pitch: f32) {
    super::track::sound_near(g, &format!("minecraft:{name}"), 4, at, vol, pitch, None);
}

fn block_event(g: &mut Game, p: BlockPos, block: u16, a: u8, b: u8) {
    use rine_protocol::Writer;
    let f = out::frame(rine_protocol::frame::packet(rine_protocol::ids::play::C_BLOCK_EVENT, |w| {
        w.w_pos(p);
        w.w_u8(a);
        w.w_u8(b);
        w.w_varint(block as i32);
    }));
    g.send_chunk_viewers(ChunkPos::new(p.x >> 4, p.z >> 4), &f);
}

/// `ContainerOpenersCounter` callbacks for one block: open / close sound on
/// the first opener and the last closer, the lid block event on every
/// change, the barrel `open` property.
pub fn lid(g: &mut Game, p: BlockPos, before: usize, after: usize) {
    if before == after {
        return;
    }
    let state = g.world.get_block(p.x, p.y, p.z);
    let name = short(state);
    let b = blocks();
    let center = [p.x as f64 + 0.5, p.y as f64 + 0.5, p.z as f64 + 0.5];
    let opening = before == 0 && after > 0;
    let closing = before > 0 && after == 0;
    let pitch = || (rand() * 0.1 + 0.9) as f32;
    if is_chest(name) {
        // `ChestBlockEntity.playSound`: the LEFT half is silent, the RIGHT half
        // plays between both halves
        if (opening || closing) && b.get_prop(state, "type") != Some("left") {
            let mut at = center;
            if b.get_prop(state, "type") == Some("right") {
                if let Some(d) = connected_dir(state) {
                    at[0] += d[0] as f64 * 0.5;
                    at[2] += d[2] as f64 * 0.5;
                }
            }
            let s = if opening { "block.chest.open" } else { "block.chest.close" };
            sound_at(g, s, at, 0.5, pitch());
        }
        block_event(g, p, b.block_of(state).id as u16, 1, after.min(255) as u8);
    } else if name == "ender_chest" {
        if opening || closing {
            let s = if opening { "block.ender_chest.open" } else { "block.ender_chest.close" };
            sound_at(g, s, center, 0.5, pitch());
        }
        block_event(g, p, b.block_of(state).id as u16, 1, after.min(255) as u8);
    } else if name.ends_with("shulker_box") {
        if opening || closing {
            let s = if opening { "block.shulker_box.open" } else { "block.shulker_box.close" };
            sound_at(g, s, center, 0.5, pitch());
        }
        block_event(g, p, b.block_of(state).id as u16, 1, after.min(255) as u8);
    } else if name == "barrel" && (opening || closing) {
        let f = dir_of(b.get_prop(state, "facing").unwrap_or("north"));
        let at = [center[0] + f[0] as f64 * 0.5, center[1] + f[1] as f64 * 0.5, center[2] + f[2] as f64 * 0.5];
        let s = if opening { "block.barrel.open" } else { "block.barrel.close" };
        sound_at(g, s, at, 0.5, pitch());
        if let Some(ns) = b.with_prop(state, "open", if opening { "true" } else { "false" }) {
            g.world.set_block(p, ns);
        }
    }
}

// ------------------------------------------------------------------ NBT

/// Loads the containers of a freshly loaded chunk into the typed state.
pub fn chunk_loaded(g: &mut Game, cp: ChunkPos) {
    let Some(slot) = g.world.chunks.get(&cp) else { return };
    let ch = &slot.chunk;
    if ch.block_entities.is_empty() {
        return;
    }
    let mut st = lock_state();
    for be in &ch.block_entities {
        let pos = blockent::pos_of(ch, be);
        let state = ch.get(be.x as i32, be.y as i32, be.z as i32);
        let name = short(state);
        let nbt = be.nbt.as_ref();
        if let Some((n, _)) = store_size(name) {
            let items = nbt.and_then(|t| t.get("Items")).map(|t| blockent::items_from_tag(t, n)).unwrap_or_else(|| vec![Stack::EMPTY; n]);
            st.chests.insert(pos, items);
            if name == "hopper" {
                let cd = nbt.and_then(|t| blockent::int_of(t, "TransferCooldown")).unwrap_or(-1);
                add_hopper(&mut st, pos, Hopper { cooldown: cd, ticked: 0 });
            }
        } else if let Some(fk) = furnace_kind(name) {
            let mut f = Furnace::new(fk);
            if let Some(t) = nbt {
                if let Some(items) = t.get("Items") {
                    let v = blockent::items_from_tag(items, 3);
                    f.items.copy_from_slice(&v);
                }
                f.burn_left = blockent::int_of(t, "lit_time_remaining").unwrap_or(0);
                f.burn_total = blockent::int_of(t, "lit_total_time").unwrap_or(0);
                f.cook = blockent::int_of(t, "cooking_time_spent").unwrap_or(0);
                f.cook_total = blockent::int_of(t, "cooking_total_time").unwrap_or(0);
            }
            if f.burn_left > 0 || f.items.iter().any(|s| !s.is_empty()) {
                st.furnaces.insert(pos, f);
            }
        }
    }
}

pub(super) fn add_hopper(st: &mut State, pos: BlockPos, h: Hopper) {
    if st.hoppers.insert(pos, h).is_none() {
        st.hopper_order.push(pos);
    }
}

/// Writes changed containers into their chunks' block entity NBT.
pub(super) fn flush(st: &mut State, world: &mut super::world::World) {
    if st.dirty.is_empty() {
        return;
    }
    let dirty: Vec<BlockPos> = st.dirty.drain().collect();
    for pos in dirty {
        let Some(slot) = world.chunks.get_mut(&ChunkPos::new(pos.x >> 4, pos.z >> 4)) else { continue };
        let ch = &mut slot.chunk;
        let state = ch.get(pos.x & 15, pos.y, pos.z & 15);
        let ty = blockent::type_of_block(blocks().block_of(state).id);
        let Some(ty) = ty else { continue };
        if blockent::find(ch, pos).is_none() {
            ch.block_entities.push(rine_world::chunk::BlockEntity { x: (pos.x & 15) as u8, y: pos.y as i16, z: (pos.z & 15) as u8, type_id: ty, nbt: None });
        }
        let be = blockent::find_mut(ch, pos).unwrap();
        let mut t = be.nbt.take().unwrap_or_else(Tag::compound);
        if let Some(items) = st.chests.get(&pos) {
            blockent::put(&mut t, "Items", blockent::items_tag(items));
            if let Some(h) = st.hoppers.get(&pos) {
                blockent::put(&mut t, "TransferCooldown", Tag::Int(h.cooldown));
            }
        } else if let Some(f) = st.furnaces.get(&pos) {
            blockent::put(&mut t, "Items", blockent::items_tag(&f.items));
            blockent::put(&mut t, "lit_time_remaining", Tag::Int(f.burn_left));
            blockent::put(&mut t, "lit_total_time", Tag::Int(f.burn_total));
            blockent::put(&mut t, "cooking_time_spent", Tag::Int(f.cook));
            blockent::put(&mut t, "cooking_total_time", Tag::Int(f.cook_total));
        } else if store_size(short(state)).is_some() || furnace_kind(short(state)).is_some() {
            blockent::put(&mut t, "Items", Tag::List(Vec::new()));
        }
        be.nbt = Some(t);
        ch.dirty = true;
    }
}

/// Before a chunk is saved and dropped: write its containers, then forget them.
pub fn chunk_unloading(world: &mut super::world::World, cp: ChunkPos) {
    let mut st = lock_state();
    flush(&mut st, world);
    let inside = |p: &BlockPos| (p.x >> 4) == cp.x && (p.z >> 4) == cp.z;
    st.chests.retain(|p, _| !inside(p));
    st.furnaces.retain(|p, _| !inside(p));
    st.hoppers.retain(|p, _| !inside(p));
    st.hopper_order.retain(|p| !inside(p));
}

/// Before every chunk is saved.
pub fn flush_all(world: &mut super::world::World) {
    let mut st = lock_state();
    flush(&mut st, world);
}

// ------------------------------------------------------------------ hoppers

/// A container a hopper or dropper can move items into or out of
/// (`HopperBlockEntity.getContainerAt`).
#[derive(Clone, Copy)]
enum Ct {
    /// Plain slots (`n` total); `shulker` refuses shulker boxes; `hopper`
    /// when it is a hopper (its cooldown reacts to inserts).
    Store { src: Src, n: usize, shulker: bool, hopper: Option<BlockPos> },
    /// A furnace (a WorldlyContainer).
    Furnace(BlockPos),
}

fn container_at(g: &Game, st: &mut State, pos: BlockPos) -> Option<Ct> {
    if !g.world.is_loaded(pos.x, pos.z) {
        return None;
    }
    let state = g.world.get_block(pos.x, pos.y, pos.z);
    let name = short(state);
    if let Some((n, shulker)) = store_size(name) {
        if is_chest(name) {
            if let Some((p, _)) = chest_partner_live(g, pos, state) {
                let right = blocks().get_prop(state, "type") == Some("right");
                let (a, b) = if right { (pos, p) } else { (p, pos) };
                st.chests.entry(a).or_insert_with(|| vec![Stack::EMPTY; 27]);
                st.chests.entry(b).or_insert_with(|| vec![Stack::EMPTY; 27]);
                return Some(Ct::Store { src: Src::Two(a, b), n: 54, shulker: false, hopper: None });
            }
        }
        st.chests.entry(pos).or_insert_with(|| vec![Stack::EMPTY; n]);
        let hopper = (name == "hopper").then_some(pos);
        if let Some(h) = hopper {
            if !st.hoppers.contains_key(&h) {
                add_hopper(st, h, Hopper { cooldown: -1, ticked: 0 });
            }
        }
        return Some(Ct::Store { src: Src::One(pos), n, shulker, hopper });
    }
    if let Some(fk) = furnace_kind(name) {
        st.furnaces.entry(pos).or_insert_with(|| Furnace::new(fk));
        return Some(Ct::Furnace(pos));
    }
    None
}

fn ct_items(st: &State, c: Ct) -> Vec<Stack> {
    match c {
        Ct::Store { src, n, .. } => read_src(st, src, n),
        Ct::Furnace(p) => st.furnaces.get(&p).map(|f| f.items.to_vec()).unwrap_or_else(|| vec![Stack::EMPTY; 3]),
    }
}

fn ct_store(st: &mut State, c: Ct, items: &[Stack]) {
    match c {
        Ct::Store { src, .. } => write_src(st, src, items),
        Ct::Furnace(p) => {
            if let Some(f) = st.furnaces.get_mut(&p) {
                f.items.copy_from_slice(&items[..3]);
            }
            st.dirty.insert(p);
        }
    }
}

fn ct_blocks(c: Ct) -> Vec<BlockPos> {
    match c {
        Ct::Store { src, .. } => containers::src_blocks(src),
        Ct::Furnace(p) => vec![p],
    }
}

fn dir_index(d: [i32; 3]) -> u8 {
    match d {
        [0, -1, 0] => 0,
        [0, 1, 0] => 1,
        _ => 2,
    }
}

/// `getSlots(container, face)`: WorldlyContainer faces for furnaces.
fn slots_for(c: Ct, len: usize, face: Option<u8>) -> Vec<usize> {
    match (c, face) {
        (Ct::Furnace(_), Some(0)) => vec![2, 1],
        (Ct::Furnace(_), Some(1)) => vec![0],
        (Ct::Furnace(_), Some(_)) => vec![1],
        _ => (0..len).collect(),
    }
}

fn is_item(item: u32, name: &str) -> bool {
    super::items::id(name) == Some(item)
}

fn is_shulker_item(item: u32) -> bool {
    super::items::info(item).is_some_and(|i| i.name.ends_with("shulker_box"))
}

/// `canPlaceItem` + `canPlaceItemThroughFace`.
fn can_place(c: Ct, items: &[Stack], slot: usize, item: u32, face: Option<u8>) -> bool {
    match c {
        Ct::Store { shulker, .. } => !(shulker && face.is_some() && is_shulker_item(item)),
        Ct::Furnace(_) => {
            if slot == 2 {
                false
            } else if slot != 1 {
                true
            } else {
                containers::is_fuel(item) || (is_item(item, "bucket") && !is_item(items[1].item, "bucket"))
            }
        }
    }
}

/// `canTakeItemThroughFace` (the hopper always pulls from below the source: DOWN).
fn can_take(c: Ct, slot: usize, item: u32) -> bool {
    match c {
        Ct::Furnace(_) => slot != 1 || is_item(item, "water_bucket") || is_item(item, "bucket"),
        _ => true,
    }
}

fn same(a: &Stack, b: &Stack) -> bool {
    a.item == b.item && a.damage == b.damage
}

/// `HopperBlockEntity.tryMoveInItem` over `addItem`'s slot order. Returns
/// what is left of `stack`; `from` is the hopper the item comes from.
fn add_item(st: &mut State, now: u64, from: Option<BlockPos>, c: Ct, mut stack: Stack, face: Option<u8>) -> Stack {
    let mut items = ct_items(st, c);
    let order = slots_for(c, items.len(), face);
    let mut changed = false;
    for slot in order {
        if stack.is_empty() {
            break;
        }
        if !can_place(c, &items, slot, stack.item, face) {
            continue;
        }
        let was_empty = items.iter().all(|s| s.is_empty());
        let cur = items[slot];
        let mut success = false;
        if cur.is_empty() {
            items[slot] = stack;
            stack = Stack::EMPTY;
            success = true;
        } else if cur.count <= max_stack(cur.item) && same(&cur, &stack) {
            let space = max_stack(stack.item) - cur.count;
            let n = stack.count.min(space).max(0);
            stack.count -= n;
            items[slot].count += n;
            if stack.count <= 0 {
                stack = Stack::EMPTY;
            }
            success = n > 0;
        }
        if success {
            changed = true;
            if let Ct::Store { hopper: Some(h), .. } = c {
                if was_empty {
                    let ticked = st.hoppers.get(&h).map(|x| x.ticked).unwrap_or(0);
                    let skip = from.and_then(|f| st.hoppers.get(&f)).is_some_and(|fh| ticked >= fh.ticked) as i32;
                    if let Some(x) = st.hoppers.get_mut(&h) {
                        x.cooldown = 8 - skip;
                    }
                }
            }
        }
    }
    let _ = now;
    if changed {
        ct_store(st, c, &items);
    }
    stack
}

fn is_full(st: &State, c: Ct, face: Option<u8>) -> bool {
    let items = ct_items(st, c);
    slots_for(c, items.len(), face).into_iter().all(|i| items[i].count >= max_stack(items[i].item) && !items[i].is_empty())
}

/// Per-tick hopper work (`HopperBlockEntity.pushItemsTick` for each hopper,
/// in the order they were loaded / placed).
pub(super) fn tick_hoppers(g: &mut Game, st: &mut State, changed: &mut Vec<BlockPos>) {
    if st.hoppers.is_empty() {
        return;
    }
    let now = g.tick;
    let order = st.hopper_order.clone();
    let mut items_index: Option<AHashMap<(i32, i32), Vec<usize>>> = None;
    for pos in order {
        if !g.world.is_loaded(pos.x, pos.z) {
            continue;
        }
        let state = g.world.get_block(pos.x, pos.y, pos.z);
        if short(state) != "hopper" {
            continue;
        }
        let Some(h) = st.hoppers.get_mut(&pos) else { continue };
        h.cooldown -= 1;
        h.ticked = now;
        if h.cooldown > 0 {
            continue;
        }
        h.cooldown = 0;
        // tryMoveItems
        if blocks().get_prop(state, "enabled") != Some("true") {
            continue;
        }
        st.chests.entry(pos).or_insert_with(|| vec![Stack::EMPTY; 5]);
        let me = Ct::Store { src: Src::One(pos), n: 5, shulker: false, hopper: Some(pos) };
        let mut moved = false;
        let mine = ct_items(st, me);
        if mine.iter().any(|s| !s.is_empty()) {
            moved = eject(g, st, now, pos, state, me, changed);
        }
        let full = ct_items(st, me).iter().all(|s| !s.is_empty() && s.count == max_stack(s.item));
        if !full {
            moved |= suck(g, st, now, pos, me, &mut items_index, changed);
        }
        if moved {
            if let Some(h) = st.hoppers.get_mut(&pos) {
                h.cooldown = 8;
            }
            st.dirty.insert(pos);
            changed.push(pos);
        }
    }
}

/// `ejectItems`: one item into the container the hopper faces.
fn eject(g: &Game, st: &mut State, now: u64, pos: BlockPos, state: u16, me: Ct, changed: &mut Vec<BlockPos>) -> bool {
    let f = dir_of(blocks().get_prop(state, "facing").unwrap_or("down"));
    let Some(target) = container_at(g, st, rel(pos, f)) else { return false };
    let face = Some(dir_index([-f[0], -f[1], -f[2]]));
    if is_full(st, target, face) {
        return false;
    }
    let mine = ct_items(st, me);
    for slot in 0..mine.len() {
        let s = mine[slot];
        if s.is_empty() {
            continue;
        }
        let left = add_item(st, now, Some(pos), target, s.with_count(1), face);
        if left.is_empty() {
            let mut v = ct_items(st, me);
            v[slot].count -= 1;
            if v[slot].count <= 0 {
                v[slot] = Stack::EMPTY;
            }
            ct_store(st, me, &v);
            changed.extend(ct_blocks(target));
            return true;
        }
    }
    false
}

/// `suckInItems`: from the container above, else item entities.
fn suck(
    g: &mut Game,
    st: &mut State,
    now: u64,
    pos: BlockPos,
    me: Ct,
    index: &mut Option<AHashMap<(i32, i32), Vec<usize>>>,
    changed: &mut Vec<BlockPos>,
) -> bool {
    let above = pos.offset(0, 1, 0);
    if let Some(src) = container_at(g, st, above) {
        let items = ct_items(st, src);
        for slot in slots_for(src, items.len(), Some(0)) {
            let s = items[slot];
            if s.is_empty() || !can_take(src, slot, s.item) {
                continue;
            }
            let left = add_item(st, now, None, me, s.with_count(1), None);
            if left.is_empty() {
                let mut v = ct_items(st, src);
                v[slot].count -= 1;
                if v[slot].count <= 0 {
                    v[slot] = Stack::EMPTY;
                }
                ct_store(st, src, &v);
                changed.extend(ct_blocks(src));
                return true;
            }
        }
        return false;
    }
    let ast = g.world.get_block(above.x, above.y, above.z);
    let info = blocks().state(ast);
    if info.flags & flag::FULL_CUBE != 0 && !does_not_block_hoppers(ast) {
        return false;
    }
    let idx = index.get_or_insert_with(|| {
        let mut m: AHashMap<(i32, i32), Vec<usize>> = AHashMap::new();
        for (i, e) in g.entities.list.iter().enumerate() {
            if !e.removed && matches!(e.kind, Kind::Item { .. }) {
                m.entry((e.pos[0].floor() as i32, e.pos[2].floor() as i32)).or_default().push(i);
            }
        }
        m
    });
    // suck box: x/z of the block, y from 11/16 to 2 blocks above the hopper;
    // item boxes are 0.25 wide and 0.25 tall
    let (minx, maxx) = (pos.x as f64, pos.x as f64 + 1.0);
    let (miny, maxy) = (pos.y as f64 + 11.0 / 16.0, pos.y as f64 + 2.0);
    let (minz, maxz) = (pos.z as f64, pos.z as f64 + 1.0);
    let mut cand: Vec<usize> = Vec::new();
    for dx in -1..=1 {
        for dz in -1..=1 {
            if let Some(v) = idx.get(&(pos.x + dx, pos.z + dz)) {
                cand.extend_from_slice(v);
            }
        }
    }
    cand.sort_unstable();
    for i in cand {
        let e = &g.entities.list[i];
        if e.removed {
            continue;
        }
        let Kind::Item { stack, .. } = e.kind else { continue };
        let (x, y, z) = (e.pos[0], e.pos[1], e.pos[2]);
        let inside = x - 0.125 < maxx && x + 0.125 > minx && y < maxy && y + 0.25 > miny && z - 0.125 < maxz && z + 0.125 > minz;
        if !inside {
            continue;
        }
        let left = add_item(st, now, None, me, stack, None);
        let eid = e.eid;
        let ent = &mut g.entities.list[i];
        if left.is_empty() {
            ent.removed = true;
            return true;
        }
        if left.count != stack.count {
            ent.kind = Kind::Item { stack: left, pickup_delay: match ent.kind { Kind::Item { pickup_delay, .. } => pickup_delay, _ => 0 } };
            g.entities.extra.push((eid, out::frame(out::Meta::new(eid).item(8, &left).finish())));
        }
    }
    false
}

fn does_not_block_hoppers(state: u16) -> bool {
    use std::sync::OnceLock;
    static SET: OnceLock<Vec<u32>> = OnceLock::new();
    let s = SET.get_or_init(|| {
        let d = rine_data::get();
        d.registries
            .tags
            .get("minecraft:block")
            .and_then(|t| t.get("minecraft:does_not_block_hoppers"))
            .map(|names| names.iter().filter_map(|n| d.blocks.block(n).map(|b| b.id)).collect())
            .unwrap_or_default()
    });
    s.contains(&blocks().block_of(state).id)
}

// ------------------------------------------------------------------ dispensers

/// `DispenserBlock.dispenseFrom` / `DropperBlock.dispenseFrom`.
pub fn dispense(g: &mut Game, pos: BlockPos, state: u16) {
    let name = short(state);
    let mut st = lock_state();
    st.chests.entry(pos).or_insert_with(|| vec![Stack::EMPTY; 9]);
    let items = st.chests.get(&pos).cloned().unwrap_or_default();
    // getRandomSlot: reservoir sampling over the non-empty slots
    let mut pick: i32 = -1;
    let mut seen = 1u32;
    for (i, s) in items.iter().enumerate() {
        if !s.is_empty() {
            if rand_int(seen) == 0 {
                pick = i as i32;
            }
            seen += 1;
        }
    }
    if pick < 0 {
        drop(st);
        let f = out::frame(out::level_event(1001, pos, 0));
        g.send_chunk_viewers(ChunkPos::new(pos.x >> 4, pos.z >> 4), &f);
        return;
    }
    let slot = pick as usize;
    let stack = items[slot];
    let facing = blocks().get_prop(state, "facing").unwrap_or("north");
    let f = dir_of(facing);
    let mut changed: Vec<BlockPos> = vec![pos];
    let remaining = if name == "dropper" {
        match container_at(g, &mut st, rel(pos, f)) {
            Some(into) => {
                let face = Some(dir_index([-f[0], -f[1], -f[2]]));
                let left = add_item(&mut st, g.tick, None, into, stack.with_count(1), face);
                if left.is_empty() {
                    changed.extend(ct_blocks(into));
                    let mut r = stack;
                    r.count -= 1;
                    r
                } else {
                    stack
                }
            }
            None => {
                drop_one(g, pos, f, stack);
                let mut r = stack;
                r.count -= 1;
                r
            }
        }
    } else {
        // dispenser: items without a special behaviour are dropped
        drop_one(g, pos, f, stack);
        let mut r = stack;
        r.count -= 1;
        r
    };
    let mut v = st.chests.get(&pos).cloned().unwrap_or_default();
    if slot < v.len() {
        v[slot] = if remaining.count > 0 { remaining } else { Stack::EMPTY };
    }
    st.chests.insert(pos, v);
    st.dirty.insert(pos);
    for p in &changed {
        containers::refresh_viewers(g, &mut st, *p);
    }
    drop(st);
    for p in changed {
        super::redstone::container_changed(&mut g.world, p);
    }
}

/// `DefaultDispenseItemBehavior`: one item flies out of the front, with the
/// dispense sound (1000) and smoke (2000).
fn drop_one(g: &mut Game, pos: BlockPos, f: [i32; 3], stack: Stack) {
    let mut at = [pos.x as f64 + 0.5 + 0.7 * f[0] as f64, pos.y as f64 + 0.5 + 0.7 * f[1] as f64, pos.z as f64 + 0.5 + 0.7 * f[2] as f64];
    at[1] -= if f[1] != 0 { 0.125 } else { 0.15625 };
    let pow = rand() * 0.1 + 0.2;
    let spread = 0.0172275 * 6.0;
    let tri = |m: f64| m + spread * (rand() - rand());
    let vel = [tri(f[0] as f64 * pow), tri(0.2), tri(f[2] as f64 * pow)];
    entity::spawn_item(g, at, vel, stack.with_count(1), 0);
    let c = ChunkPos::new(pos.x >> 4, pos.z >> 4);
    let snd = out::frame(out::level_event(1000, pos, 0));
    g.send_chunk_viewers(c, &snd);
    let d3 = match f {
        [0, -1, 0] => 0,
        [0, 1, 0] => 1,
        [0, 0, -1] => 2,
        [0, 0, 1] => 3,
        [-1, 0, 0] => 4,
        _ => 5,
    };
    let smoke = out::frame(out::level_event(2000, pos, d3));
    g.send_chunk_viewers(c, &smoke);
}

// ------------------------------------------------------------------ chests

/// `ChestBlock.getStateForPlacement`: joins a single chest of the same kind
/// beside the new one (sneaking on a side face picks that chest).
pub fn chest_placement(g: &Game, state: u16, pos: BlockPos, clicked_face: u8, sneaking: bool) -> u16 {
    let b = blocks();
    let name = short(state);
    if !is_chest(name) {
        return state;
    }
    let block = b.block_of(state).id;
    let facing = dir_of(b.get_prop(state, "facing").unwrap_or("north"));
    let cand = |d: [i32; 3]| -> Option<[i32; 3]> {
        let p = rel(pos, d);
        let s = g.world.get_block(p.x, p.y, p.z);
        (b.block_of(s).id == block && b.get_prop(s, "type") == Some("single")).then(|| dir_of(b.get_prop(s, "facing").unwrap_or("north")))
    };
    let face_dir = [[0, -1, 0], [0, 1, 0], [0, 0, -1], [0, 0, 1], [-1, 0, 0], [1, 0, 0]][clicked_face.min(5) as usize];
    let mut ty = "single";
    let mut fac = facing;
    if clicked_face >= 2 && sneaking {
        // the chest that was clicked is at -face_dir
        let toward = [-face_dir[0], -face_dir[1], -face_dir[2]];
        if let Some(nf) = cand(toward) {
            let horizontal_axis_differs = (nf[0] != 0) != (face_dir[0] != 0);
            if horizontal_axis_differs {
                fac = nf;
                let ccw = [fac[2], 0, -fac[0]];
                ty = if ccw == toward { "right" } else { "left" };
            }
        }
    }
    if ty == "single" && !sneaking {
        let cw = [-fac[2], 0, fac[0]];
        let ccw = [fac[2], 0, -fac[0]];
        if cand(cw) == Some(fac) {
            ty = "left";
        } else if cand(ccw) == Some(fac) {
            ty = "right";
        }
    }
    let fname = match fac {
        [0, 0, -1] => "north",
        [0, 0, 1] => "south",
        [-1, 0, 0] => "west",
        _ => "east",
    };
    let s = b.with_prop(state, "facing", fname).unwrap_or(state);
    b.with_prop(s, "type", ty).unwrap_or(s)
}

/// `ChestBlock.updateShape` for every horizontal neighbour at once: a single
/// chest joins a half that points at it, a half whose partner is gone turns
/// single. Returns the new state when it changes.
pub fn chest_update(g: &Game, pos: BlockPos, state: u16) -> Option<u16> {
    let b = blocks();
    if !is_chest(short(state)) {
        return None;
    }
    let block = b.block_of(state).id;
    let ty = b.get_prop(state, "type")?;
    if ty == "single" {
        for d in [[0, 0, -1], [0, 0, 1], [-1, 0, 0], [1, 0, 0]] {
            let p = rel(pos, d);
            let s = g.world.get_block(p.x, p.y, p.z);
            if b.block_of(s).id != block {
                continue;
            }
            let nt = b.get_prop(s, "type")?;
            if nt != "single" && b.get_prop(s, "facing") == b.get_prop(state, "facing") && connected_dir(s) == Some([-d[0], 0, -d[2]]) {
                return b.with_prop(state, "type", if nt == "left" { "right" } else { "left" });
            }
        }
        None
    } else {
        let d = connected_dir(state)?;
        let p = rel(pos, d);
        let s = g.world.get_block(p.x, p.y, p.z);
        (b.block_of(s).id != block).then(|| b.with_prop(state, "type", "single")).flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::containers;
    use crate::game::redstone::game_tests::setup;

    fn st(s: &str) -> u16 {
        rine_data::get().blocks.parse_state(s).unwrap_or_else(|| panic!("state {s}"))
    }
    fn put(g: &mut Game, p: BlockPos, s: &str) {
        g.world.set_block(p, st(s));
    }
    fn ticks(g: &mut Game, n: u32) {
        for _ in 0..n {
            g.do_tick();
        }
    }
    fn item(n: &str) -> u32 {
        crate::game::items::id(n).unwrap()
    }
    fn count(g: &Game, p: BlockPos) -> i32 {
        let _ = g;
        containers::block_items(p).map(|v| v.iter().map(|s| s.count).sum()).unwrap_or(0)
    }
    /// Each test uses its own spot: the typed container state is one table
    /// for the process, shared by tests running in parallel.
    fn base(g: &mut Game, k: i32) -> BlockPos {
        let o = BlockPos::new(g.spawn[0] as i32 - 4 + 4 * k, 200, g.spawn[2] as i32 - 4);
        for x in -3..=6 {
            for z in -3..=6 {
                put(g, o.offset(x, -1, z), "stone");
            }
        }
        ticks(g, 1);
        o
    }

    #[test]
    fn hopper_moves_one_item_every_8_ticks() {
        let mut g = setup("hopper");
        let o = base(&mut g, 0);
        let (top, hop, bottom) = (o.offset(0, 2, 0), o.offset(0, 1, 0), o);
        put(&mut g, bottom, "chest");
        put(&mut g, hop, "hopper[facing=down]");
        put(&mut g, top, "chest");
        ticks(&mut g, 1);
        let mut v = vec![Stack::EMPTY; 27];
        v[0] = Stack::new(item("cobblestone"), 10);
        containers::set_block_items(&mut g, top, v);
        // first pull right away, then the hopper pushes and pulls every 8 ticks
        ticks(&mut g, 1);
        assert_eq!(count(&g, top), 9);
        assert_eq!(count(&g, hop), 1);
        ticks(&mut g, 7);
        assert_eq!(count(&g, bottom), 0, "cooldown 8");
        ticks(&mut g, 1);
        assert_eq!(count(&g, bottom), 1);
        assert_eq!(count(&g, top), 8);
        ticks(&mut g, 8 * 20);
        assert_eq!(count(&g, bottom), 10);
        assert_eq!(count(&g, top) + count(&g, hop), 0);
    }

    #[test]
    fn powered_hopper_stops() {
        let mut g = setup("hopper-off");
        let o = base(&mut g, 1);
        let (top, hop) = (o.offset(0, 1, 0), o);
        put(&mut g, hop, "hopper[facing=down]");
        put(&mut g, top, "chest");
        put(&mut g, o.offset(1, 0, 0), "redstone_block");
        ticks(&mut g, 2);
        let mut v = vec![Stack::EMPTY; 27];
        v[3] = Stack::new(item("dirt"), 5);
        containers::set_block_items(&mut g, top, v);
        ticks(&mut g, 20);
        assert_eq!(count(&g, top), 5, "a powered hopper is disabled");
    }

    #[test]
    fn dropper_feeds_the_chest_in_front_on_a_rising_edge() {
        let mut g = setup("dropper");
        let o = base(&mut g, 2);
        let (dr, chest) = (o, o.offset(1, 0, 0));
        put(&mut g, chest, "chest");
        put(&mut g, dr, "dropper[facing=east]");
        ticks(&mut g, 1);
        let mut v = vec![Stack::EMPTY; 9];
        v[4] = Stack::new(item("diamond"), 3);
        containers::set_block_items(&mut g, dr, v);
        put(&mut g, o.offset(-1, 0, 0), "redstone_block");
        ticks(&mut g, 6);
        assert_eq!(count(&g, chest), 1);
        assert_eq!(count(&g, dr), 2);
        ticks(&mut g, 10);
        assert_eq!(count(&g, chest), 1, "one item per edge");
    }

    #[test]
    fn container_items_reach_the_block_entity_nbt_and_back() {
        let mut g = setup("persist");
        let o = base(&mut g, 3);
        put(&mut g, o, "barrel");
        ticks(&mut g, 1);
        let mut v = vec![Stack::EMPTY; 27];
        v[2] = Stack::new(item("iron_ingot"), 7);
        v[26] = Stack::damaged(item("iron_pickaxe"), 1, 9);
        containers::set_block_items(&mut g, o, v.clone());
        ticks(&mut g, 1);
        let cp = ChunkPos::new(o.x >> 4, o.z >> 4);
        let nbt = blockent::find(&g.world.chunks[&cp].chunk, o).and_then(|b| b.nbt.clone()).expect("block entity");
        assert_eq!(blockent::items_from_tag(nbt.get("Items").unwrap(), 27), v);
        // forget the typed copy, then load it again from the chunk
        lock_state().chests.remove(&o);
        chunk_loaded(&mut g, cp);
        assert_eq!(containers::block_items(o).unwrap(), v);
    }

    #[test]
    fn chests_join_into_a_double_chest() {
        let mut g = setup("double");
        let o = base(&mut g, 4);
        put(&mut g, o, "chest[facing=north]");
        ticks(&mut g, 1);
        // a second north-facing chest to the east joins as the RIGHT half
        let s = chest_placement(&g, st("chest[facing=north]"), o.offset(1, 0, 0), 1, false);
        assert_eq!(blocks().get_prop(s, "type"), Some("right"));
        g.world.set_block(o.offset(1, 0, 0), s);
        ticks(&mut g, 1);
        let first = g.world.get_block(o.x, o.y, o.z);
        assert_eq!(blocks().get_prop(first, "type"), Some("left"));
        assert_eq!(chest_partner_live(&g, o, first).map(|x| x.0), Some(o.offset(1, 0, 0)));
        // removing one half turns the other single again
        put(&mut g, o.offset(1, 0, 0), "air");
        ticks(&mut g, 1);
        assert_eq!(blocks().get_prop(g.world.get_block(o.x, o.y, o.z), "type"), Some("single"));
    }
}
