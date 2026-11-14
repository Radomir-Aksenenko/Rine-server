//! Windows and containers: player inventory (window 0, 2x2 crafting),
//! crafting table (3x3), chests / barrels and furnace / smoker / blast furnace.
//!
//! Clicks are applied on the server state with vanilla semantics
//! (`AbstractContainerMenu.doClick`). The client's changed-slot hashes are
//! parsed and ignored, so a client can only move items the server already
//! holds. Every click ends with a full content resync of the window
//! (vanilla does the same when the state id is stale); quick-craft
//! start/add stages send nothing.
//!
//! Open windows, chest and furnace contents and drag state live in one
//! process-wide table: `Game` and `Player` belong to other modules. Nothing is
//! persisted yet. Clicks are pure functions (`apply_click`), the table is only
//! locked around the wrappers and never while calling into block removal.

use super::entity;
use super::items;
use super::player::{max_stack, Inventory, Stack};
use super::Game;
use crate::out;
use ahash::AHashMap;
use bytes::BytesMut;
use rine_protocol::frame::packet;
use rine_protocol::ids::play::{C_CONTAINER_CLOSE, C_CONTAINER_SET_DATA, C_OPEN_SCREEN, C_SET_CURSOR_ITEM};
use rine_protocol::text::Text;
use rine_protocol::{BlockPos, ProtoError, Reader, Writer};
use rine_world::ChunkPos;
use serde_json::Value;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// Serverbound container_click. Must fully consume the packet body.
pub fn click(g: &mut Game, slot: usize, r: &mut Reader) -> Result<(), ProtoError> {
    let window = r.varint()?;
    let client_state = r.varint()?;
    let sslot = r.i16()? as i32;
    let button = r.i8()? as i32;
    let mode = r.varint()?;
    let n = r.varint()?;
    if !(0..=128).contains(&n) {
        return Err(ProtoError::Invalid("too many slots"));
    }
    for _ in 0..n {
        r.i16()?;
        skip_hashed(r)?;
    }
    skip_hashed(r)?;

    let Some(creative) = g.player(slot).map(|p| p.gamemode == 1) else { return Ok(()) };
    let mut st = lock_state();
    sweep(&mut st, g);
    let inv_state = g.player(slot).map_or(0, |p| p.inv.state_id);
    let (lay, pos, grid, expected, cur) = match st.open.get(&slot) {
        Some(o) => (o.lay, o.pos, o.grid, o.window, o.state_id),
        None => (Lay::Inv, BlockPos::default(), [Stack::EMPTY; 9], 0, inv_state),
    };
    if window != expected {
        resync(g, slot, &mut st);
        return Ok(());
    }
    // A stale state id is still applied (pipelined drag and double-click
    // packets depend on it); the full resync below corrects the client.
    let _ = client_state;
    let _ = cur;

    let container = container_items(&st, lay, pos, &grid);
    let mut drag = st.drags.remove(&slot).unwrap_or_default();
    let mut fx = Fx::default();
    let Some(p) = g.players.get_mut(slot).and_then(|p| p.as_mut()) else { return Ok(()) };
    let mut win = load(&p.inv, lay, &container);
    let mut cursor = p.inv.cursor;
    apply_click(lay, &mut win, &mut cursor, &mut drag, sslot, button, mode, creative, &mut fx);
    store_inv(&mut p.inv, lay, &win);
    p.inv.cursor = cursor;
    let after = container_of(lay, &win);
    let mut throws = std::mem::take(&mut fx.throw);
    for s in fx.give {
        let left = p.inv.add(s);
        if left > 0 {
            throws.push(s.with_count(left));
        }
    }
    let visible = load(&p.inv, lay, &after);
    let quick_craft_pending = mode == 5 && (button & 3) != 2;
    if !quick_craft_pending {
        let pk = if lay == Lay::Inv {
            p.inventory_packet()
        } else {
            match st.open.get_mut(&slot) {
                Some(o) => {
                    o.state_id = bump(o.state_id);
                    out::container_content(o.window, o.state_id, &visible[..lay.size()], &p.inv.cursor)
                }
                None => return Ok(()),
            }
        };
        p.send(pk);
    }
    if drag.status != 0 {
        st.drags.insert(slot, drag);
    }
    super::track::equipment_changed(g, slot);

    match lay {
        Lay::Table => {
            if let Some(o) = st.open.get_mut(&slot) {
                o.grid.copy_from_slice(&after);
            }
        }
        Lay::Chest => {
            st.chests.insert(pos, after.clone());
            notify_viewers(g, &mut st, slot, lay, pos, &container, &after);
        }
        Lay::Furnace(_) => {
            if let Some(f) = st.furnaces.get_mut(&pos) {
                f.items.copy_from_slice(&after);
                f.sent_items = f.items;
            }
            notify_viewers(g, &mut st, slot, lay, pos, &container, &after);
        }
        Lay::Inv => {}
    }
    drop(st);
    for s in throws {
        entity::drop_from_player(g, slot, s, true);
    }
    Ok(())
}

/// The player is leaving. An open window closes (a table grid goes back to
/// the inventory), then the 2x2 grid and the cursor return to the inventory,
/// so the saved inventory is complete. Call it before the player is removed.
pub fn player_left(g: &mut Game, slot: usize) {
    {
        let mut st = lock_state();
        sweep(&mut st, g);
        st.drags.remove(&slot);
        if st.open.contains_key(&slot) {
            close_open(g, slot, &mut st, false);
        }
    }
    close_inventory(g, slot);
}

/// Client closed a window. Window 0 closes return the 2x2 grid and cursor.
pub fn close(g: &mut Game, slot: usize, window: i32) {
    let mut st = lock_state();
    sweep(&mut st, g);
    match st.open.get(&slot).map(|o| o.window) {
        Some(w) if w == window => close_open(g, slot, &mut st, false),
        Some(_) => {}
        None if window == 0 => close_inventory(g, slot),
        None => {}
    }
}

/// Right-click on a block that opens a window. Return true if handled.
pub fn open_block(g: &mut Game, slot: usize, pos: BlockPos, state: u16) -> bool {
    let (lay, title, menu) = match rine_data::get().blocks.block_of(state).name.as_str() {
        "minecraft:crafting_table" => (Lay::Table, "Верстак", "minecraft:crafting"),
        "minecraft:chest" | "minecraft:trapped_chest" => (Lay::Chest, "Сундук", "minecraft:generic_9x3"),
        "minecraft:barrel" => (Lay::Chest, "Бочка", "minecraft:generic_9x3"),
        "minecraft:furnace" => (Lay::Furnace(Fk::Furnace), "Печь", "minecraft:furnace"),
        "minecraft:smoker" => (Lay::Furnace(Fk::Smoker), "Коптильня", "minecraft:smoker"),
        "minecraft:blast_furnace" => (Lay::Furnace(Fk::Blast), "Плавильная печь", "minecraft:blast_furnace"),
        _ => return false,
    };
    let mut st = lock_state();
    sweep(&mut st, g);
    let Some(eid) = g.player(slot).map(|p| p.eid) else { return false };
    if st.open.contains_key(&slot) {
        close_open(g, slot, &mut st, false);
    }
    match lay {
        Lay::Chest => {
            st.chests.entry(pos).or_insert_with(|| vec![Stack::EMPTY; 27]);
        }
        Lay::Furnace(fk) => {
            let f = st.furnaces.entry(pos).or_insert_with(|| Furnace::new(fk));
            if f.kind != fk {
                *f = Furnace::new(fk);
            }
        }
        _ => {}
    }
    st.next_window = st.next_window % 100 + 1;
    let window = st.next_window;
    let menu_id = rine_data::get().registries.id("minecraft:menu", menu).unwrap_or(0) as i32;
    let mut o = Open { eid, window, state_id: 0, lay, pos, grid: [Stack::EMPTY; 9] };
    let container = container_items(&st, lay, pos, &o.grid);
    let Some(p) = g.players.get_mut(slot).and_then(|p| p.as_mut()) else { return false };
    p.send(packet(C_OPEN_SCREEN, |b| {
        b.w_varint(window);
        b.w_varint(menu_id);
        out::write_text(b, &Text::plain(title));
    }));
    let win = load(&p.inv, lay, &container);
    o.state_id = bump(0);
    p.send(out::container_content(window, o.state_id, &win[..lay.size()], &p.inv.cursor));
    if let Lay::Furnace(_) = lay {
        if let Some(f) = st.furnaces.get(&pos) {
            for (id, v) in furnace_data(f).into_iter().enumerate() {
                p.send(data_pk(window, id as i32, v));
            }
        }
    }
    st.open.insert(slot, o);
    true
}

/// Called once per tick. Advances every furnace that holds items or fuel;
/// does nothing when no furnace is tracked.
pub fn tick(g: &mut Game) {
    let mut st = lock_state();
    if st.furnaces.is_empty() {
        return;
    }
    sweep(&mut st, g);
    let blocks = &rine_data::get().blocks;
    let mut updates: Vec<(BlockPos, Vec<(usize, Stack)>, Vec<(i32, i32)>)> = Vec::new();
    let mut lit: Vec<(BlockPos, u16, bool)> = Vec::new();
    let mut gone: Vec<BlockPos> = Vec::new();
    let mut idle: Vec<BlockPos> = Vec::new();
    for (&pos, f) in st.furnaces.iter_mut() {
        if !g.world.chunks.contains_key(&ChunkPos::new(pos.x >> 4, pos.z >> 4)) {
            continue;
        }
        let cur = g.world.get_block(pos.x, pos.y, pos.z);
        if furnace_kind(cur) != Some(f.kind) {
            gone.push(pos);
            continue;
        }
        f.step();
        let want = f.burn_left > 0;
        if (blocks.get_prop(cur, "lit") == Some("true")) != want {
            lit.push((pos, cur, want));
        }
        let items_changed: Vec<(usize, Stack)> = (0..3).filter(|&i| f.items[i] != f.sent_items[i]).map(|i| (i, f.items[i])).collect();
        let data = furnace_data(f);
        let data_changed: Vec<(i32, i32)> = (0..4).filter(|&i| data[i] != f.sent_data[i]).map(|i| (i as i32, data[i])).collect();
        if !items_changed.is_empty() || !data_changed.is_empty() {
            f.sent_items = f.items;
            f.sent_data = data;
            updates.push((pos, items_changed, data_changed));
        }
        if f.burn_left == 0 && f.items.iter().all(|s| s.is_empty()) {
            idle.push(pos);
        }
    }
    for (pos, cur, want) in lit {
        let v = if want { "true" } else { "false" };
        if let Some(ns) = blocks.with_prop(cur, "lit", v) {
            g.world.set_block(pos, ns);
        }
    }
    for pos in gone {
        for v in viewers_at(&st, pos) {
            close_open(g, v, &mut st, true);
        }
        if let Some(f) = st.furnaces.remove(&pos) {
            drop_at(g, pos, &f.items);
        }
    }
    for pos in idle {
        if !st.open.values().any(|o| o.pos == pos) {
            st.furnaces.remove(&pos);
        }
    }
    for (pos, items, data) in updates {
        for (slot, o) in st.open.iter_mut() {
            if o.pos != pos || !matches!(o.lay, Lay::Furnace(_)) {
                continue;
            }
            let Some(p) = g.players.get_mut(*slot).and_then(|p| p.as_mut()) else { continue };
            for &(i, s) in &items {
                o.state_id = bump(o.state_id);
                p.send(out::container_slot(o.window, o.state_id, i as i16, &s));
            }
            for &(id, v) in &data {
                p.send(data_pk(o.window, id, v));
            }
        }
    }
}

/// A block was removed: drop its container contents and close open windows.
pub fn block_removed(g: &mut Game, pos: BlockPos, _old_state: u16) {
    let mut st = lock_state();
    for v in viewers_at(&st, pos) {
        close_open(g, v, &mut st, true);
    }
    let mut drops: Vec<Stack> = Vec::new();
    if let Some(items) = st.chests.remove(&pos) {
        drops.extend(items);
    }
    if let Some(f) = st.furnaces.remove(&pos) {
        drops.extend(f.items);
    }
    drop(st);
    drop_at(g, pos, &drops);
}

// ------------------------------------------------------------------ state

/// Furnace flavour: block name and recipe table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fk {
    Furnace,
    Smoker,
    Blast,
}

/// Window layout. Slot numbers are the vanilla ones of each menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lay {
    /// Player inventory (window 0), 2x2 crafting.
    Inv,
    /// Crafting table, 3x3 grid.
    Table,
    /// generic_9x3: chest, trapped chest, barrel.
    Chest,
    Furnace(Fk),
}

impl Lay {
    /// Window slots (vanilla `slots.size()`).
    pub fn size(self) -> usize {
        match self {
            Lay::Inv | Lay::Table => 46,
            Lay::Chest => 63,
            Lay::Furnace(_) => 39,
        }
    }
    /// Working array length: the window plus, for containers, an extra
    /// entry at `size()` holding the player's offhand.
    fn work_len(self) -> usize {
        self.size() + usize::from(self != Lay::Inv)
    }
    fn offhand(self) -> usize {
        if self == Lay::Inv {
            45
        } else {
            self.size()
        }
    }
    fn result(self) -> Option<usize> {
        match self {
            Lay::Inv | Lay::Table => Some(0),
            Lay::Furnace(_) => Some(2),
            Lay::Chest => None,
        }
    }
    fn craft(self) -> bool {
        matches!(self, Lay::Inv | Lay::Table)
    }
    /// (first grid slot, grid width) of a crafting window.
    fn grid(self) -> Option<(usize, usize)> {
        match self {
            Lay::Inv => Some((1, 2)),
            Lay::Table => Some((1, 3)),
            _ => None,
        }
    }
    /// Window slot of the container's own slots (crafting grid, chest, furnace).
    fn container_offset(self) -> usize {
        usize::from(self == Lay::Table)
    }
    fn container_len(self) -> usize {
        match self {
            Lay::Inv => 0,
            Lay::Table => 9,
            Lay::Chest => 27,
            Lay::Furnace(_) => 3,
        }
    }
    /// Player inventory index -> window slot (None when not in this window).
    fn from_inv(self, i: usize) -> Option<usize> {
        match self {
            Lay::Inv => Some(i),
            Lay::Table => (9..=44).contains(&i).then(|| i + 1),
            Lay::Chest => (9..=44).contains(&i).then(|| i + 18),
            Lay::Furnace(_) => (9..=44).contains(&i).then(|| i - 6),
        }
    }
    /// Window slot -> player inventory index (None for container slots).
    fn to_inv(self, w: usize) -> Option<usize> {
        match self {
            Lay::Inv => Some(w),
            Lay::Table => (10..46).contains(&w).then(|| w - 1),
            Lay::Chest => (27..63).contains(&w).then(|| w - 18),
            Lay::Furnace(_) => (3..39).contains(&w).then(|| w + 6),
        }
    }
}

#[derive(Default)]
struct State {
    next_window: i32,
    open: AHashMap<usize, Open>,
    drags: AHashMap<usize, Drag>,
    chests: AHashMap<BlockPos, Vec<Stack>>,
    furnaces: AHashMap<BlockPos, Furnace>,
}

/// A non-inventory window open by one player.
struct Open {
    eid: i32,
    window: i32,
    state_id: i32,
    lay: Lay,
    pos: BlockPos,
    /// Crafting table grid (per player; chests and furnaces keep theirs in `State`).
    grid: [Stack; 9],
}

/// Furnace block entity state, keyed by block position.
struct Furnace {
    kind: Fk,
    items: [Stack; 3],
    burn_left: i32,
    burn_total: i32,
    cook: i32,
    cook_total: i32,
    sent_items: [Stack; 3],
    sent_data: [i32; 4],
}

impl Furnace {
    fn new(kind: Fk) -> Self {
        Furnace {
            kind,
            items: [Stack::EMPTY; 3],
            burn_left: 0,
            burn_total: 0,
            cook: 0,
            cook_total: 0,
            sent_items: [Stack::EMPTY; 3],
            sent_data: [0; 4],
        }
    }

    fn step(&mut self) {
        let smelt = if self.items[0].is_empty() { None } else { smelt_for(self.kind, self.items[0].item) };
        let can = match smelt {
            Some(s) => {
                let out = self.items[2];
                out.is_empty() || (out.item == s.result.item && out.count + s.result.count <= max_stack(out.item))
            }
            None => false,
        };
        if self.burn_left > 0 {
            self.burn_left -= 1;
        }
        if self.burn_left == 0 && can && !self.items[1].is_empty() {
            let fuel = self.items[1].item;
            let t = fuel_time(fuel);
            if t > 0 {
                self.burn_left = t;
                self.burn_total = t;
                self.items[1].count -= 1;
                norm(&mut self.items[1]);
                if let Some(rem) = remainder(fuel) {
                    if self.items[1].is_empty() {
                        self.items[1] = Stack::new(rem, 1);
                    }
                }
            }
        }
        if self.burn_left > 0 && can {
            if let Some(s) = smelt {
                self.cook_total = s.time;
                self.cook += 1;
                if self.cook >= self.cook_total {
                    self.cook = 0;
                    self.items[0].count -= 1;
                    norm(&mut self.items[0]);
                    let out = &mut self.items[2];
                    if out.is_empty() {
                        *out = s.result;
                    } else {
                        out.count += s.result.count;
                    }
                }
            }
        } else if self.cook > 0 {
            self.cook = (self.cook - 2).max(0);
        }
    }
}

/// Drag (quick craft) state of one player.
#[derive(Default, Clone, Debug)]
pub struct Drag {
    status: u8,
    ty: i32,
    slots: Vec<usize>,
}

/// Side effects of a click that the caller applies to the player and world.
#[derive(Default, Debug)]
pub struct Fx {
    /// Items for the player's inventory (overflow is dropped).
    pub give: Vec<Stack>,
    /// Items thrown into the world.
    pub throw: Vec<Stack>,
}

fn lock_state() -> MutexGuard<'static, State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(State::default())).lock().unwrap_or_else(|e| e.into_inner())
}

/// Drops entries whose player slot no longer holds the same entity.
fn sweep(st: &mut State, g: &Game) {
    st.open.retain(|slot, o| g.players.get(*slot).and_then(|p| p.as_ref()).is_some_and(|p| p.eid == o.eid));
}

fn viewers_at(st: &State, pos: BlockPos) -> Vec<usize> {
    st.open.iter().filter(|(_, o)| o.pos == pos).map(|(s, _)| *s).collect()
}

fn bump(s: i32) -> i32 {
    s.wrapping_add(1) & 0x7FFF
}

fn container_items(st: &State, lay: Lay, pos: BlockPos, grid: &[Stack; 9]) -> Vec<Stack> {
    match lay {
        Lay::Inv => Vec::new(),
        Lay::Table => grid.to_vec(),
        Lay::Chest => st.chests.get(&pos).cloned().unwrap_or_else(|| vec![Stack::EMPTY; 27]),
        Lay::Furnace(_) => st.furnaces.get(&pos).map_or_else(|| vec![Stack::EMPTY; 3], |f| f.items.to_vec()),
    }
}

/// Working copy of a window: the inventory parts come from `inv`, the
/// container parts from `container`. Derived slots are recomputed.
fn load(inv: &Inventory, lay: Lay, container: &[Stack]) -> Vec<Stack> {
    let mut win = vec![Stack::EMPTY; lay.work_len()];
    if lay == Lay::Inv {
        win.copy_from_slice(&inv.slots);
    } else {
        for w in 0..lay.size() {
            if let Some(i) = lay.to_inv(w) {
                win[w] = inv.slots[i];
            }
        }
        let off = lay.container_offset();
        win[off..off + container.len()].copy_from_slice(container);
        win[lay.offhand()] = inv.slots[45];
    }
    refresh(lay, &mut win);
    win
}

fn store_inv(inv: &mut Inventory, lay: Lay, win: &[Stack]) {
    if lay == Lay::Inv {
        inv.slots.copy_from_slice(&win[..46]);
        return;
    }
    for w in 0..lay.size() {
        if let Some(i) = lay.to_inv(w) {
            inv.slots[i] = win[w];
        }
    }
    inv.slots[45] = win[lay.offhand()];
}

fn container_of(lay: Lay, win: &[Stack]) -> Vec<Stack> {
    if lay == Lay::Inv {
        return Vec::new();
    }
    let off = lay.container_offset();
    win[off..off + lay.container_len()].to_vec()
}

fn resync(g: &mut Game, slot: usize, st: &mut State) {
    let Some(p) = g.players.get_mut(slot).and_then(|p| p.as_mut()) else { return };
    let open = st.open.get(&slot).map(|o| (o.lay, o.pos, o.grid, o.window));
    let pk = match open {
        None => p.inventory_packet(),
        Some((lay, pos, grid, window)) => {
            let container = container_items(st, lay, pos, &grid);
            let win = load(&p.inv, lay, &container);
            match st.open.get_mut(&slot) {
                Some(o) => {
                    o.state_id = bump(o.state_id);
                    out::container_content(window, o.state_id, &win[..lay.size()], &p.inv.cursor)
                }
                None => return,
            }
        }
    };
    p.send(pk);
}

/// Notifies the other viewers of a chest or furnace about changed slots.
fn notify_viewers(g: &mut Game, st: &mut State, acting: usize, lay: Lay, pos: BlockPos, before: &[Stack], after: &[Stack]) {
    let changed: Vec<usize> = (0..after.len()).filter(|&i| before.get(i) != Some(&after[i])).collect();
    if changed.is_empty() {
        return;
    }
    let off = lay.container_offset();
    for (slot, o) in st.open.iter_mut() {
        if *slot == acting || o.pos != pos || o.lay != lay {
            continue;
        }
        let Some(p) = g.players.get_mut(*slot).and_then(|p| p.as_mut()) else { continue };
        for &i in &changed {
            o.state_id = bump(o.state_id);
            p.send(out::container_slot(o.window, o.state_id, (i + off) as i16, &after[i]));
        }
    }
}

/// Returns `items` and the cursor to the player's inventory. Overflow is
/// thrown on the ground. `extra` packets are sent after the inventory.
fn return_to_player(g: &mut Game, slot: usize, mut items: Vec<Stack>, extra: Vec<BytesMut>) {
    let mut drops = Vec::new();
    {
        let Some(p) = g.players.get_mut(slot).and_then(|p| p.as_mut()) else { return };
        let had_cursor = !p.inv.cursor.is_empty();
        items.push(p.inv.cursor);
        p.inv.cursor = Stack::EMPTY;
        for s in items {
            if s.is_empty() {
                continue;
            }
            let left = p.inv.add(s);
            if left > 0 {
                drops.push(s.with_count(left));
            }
        }
        let pk = p.inventory_packet();
        p.send(pk);
        if had_cursor {
            p.send(cursor_pk(Stack::EMPTY));
        }
        for pk in extra {
            p.send(pk);
        }
    }
    super::track::equipment_changed(g, slot);
    for s in drops {
        entity::drop_from_player(g, slot, s, false);
    }
}

fn close_open(g: &mut Game, slot: usize, st: &mut State, notify: bool) {
    let Some(o) = st.open.remove(&slot) else { return };
    st.drags.remove(&slot);
    let items = if o.lay == Lay::Table { o.grid.to_vec() } else { Vec::new() };
    let mut extra = Vec::new();
    if notify {
        let window = o.window;
        extra.push(packet(C_CONTAINER_CLOSE, |b| b.w_varint(window)));
    }
    return_to_player(g, slot, items, extra);
}

fn close_inventory(g: &mut Game, slot: usize) {
    let back = {
        let Some(p) = g.players.get_mut(slot).and_then(|p| p.as_mut()) else { return };
        let mut back = Vec::new();
        for i in 1..5 {
            back.push(p.inv.slots[i]);
            p.inv.slots[i] = Stack::EMPTY;
        }
        p.inv.slots[0] = Stack::EMPTY;
        back
    };
    return_to_player(g, slot, back, Vec::new());
}

fn drop_at(g: &mut Game, pos: BlockPos, items: &[Stack]) {
    for (i, s) in items.iter().enumerate() {
        if s.is_empty() {
            continue;
        }
        let seed = (pos.x as u64).wrapping_mul(0x9E37_79B9) ^ ((pos.z as u64) << 20) ^ g.tick ^ ((i as u64) << 40);
        let a = entity::rand_f64(seed);
        let b = entity::rand_f64(seed ^ 0xA5A5_5A5A);
        let at = [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5];
        entity::spawn_item(g, at, [(a - 0.5) * 0.2, 0.2, (b - 0.5) * 0.2], *s, 10);
    }
}

fn furnace_kind(state: u16) -> Option<Fk> {
    match rine_data::get().blocks.block_of(state).name.as_str() {
        "minecraft:furnace" => Some(Fk::Furnace),
        "minecraft:smoker" => Some(Fk::Smoker),
        "minecraft:blast_furnace" => Some(Fk::Blast),
        _ => None,
    }
}

fn furnace_data(f: &Furnace) -> [i32; 4] {
    [f.burn_left, f.burn_total, f.cook, f.cook_total]
}

fn data_pk(window: i32, id: i32, value: i32) -> BytesMut {
    let v = value.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    packet(C_CONTAINER_SET_DATA, |b| {
        b.w_varint(window);
        b.w_i16(id as i16);
        b.w_i16(v);
    })
}

fn cursor_pk(s: Stack) -> BytesMut {
    packet(C_SET_CURSOR_ITEM, |b| out::write_stack(b, &s))
}

/// Parses a HashedStack and discards it (the client's data is not trusted).
fn skip_hashed(r: &mut Reader) -> Result<(), ProtoError> {
    if !r.bool()? {
        return Ok(());
    }
    r.varint()?;
    r.varint()?;
    let added = r.varint()?;
    if !(0..=256).contains(&added) {
        return Err(ProtoError::Invalid("hashed components"));
    }
    for _ in 0..added {
        r.varint()?;
        r.i32()?;
    }
    let removed = r.varint()?;
    if !(0..=256).contains(&removed) {
        return Err(ProtoError::Invalid("hashed components"));
    }
    for _ in 0..removed {
        r.varint()?;
    }
    Ok(())
}

// ----------------------------------------------------------- item helpers

fn item_name(item: u32) -> &'static str {
    items::info(item).map_or("", |i| i.name.as_str())
}

fn item_id(name: &str) -> Option<u32> {
    rine_data::get().registries.id("minecraft:item", name)
}

fn norm(s: &mut Stack) {
    if s.item == 0 || s.count <= 0 {
        *s = Stack::EMPTY;
    }
}

/// Armor slot (5 head, 6 chest, 7 legs, 8 feet) an item fits, by name.
fn armor_slot(item: u32) -> Option<usize> {
    let n = item_name(item);
    if n.ends_with("_helmet") || n.ends_with("carved_pumpkin") || n.ends_with("_skull") || n.ends_with("_head") {
        Some(5)
    } else if n.ends_with("_chestplate") || n.ends_with("elytra") {
        Some(6)
    } else if n.ends_with("_leggings") {
        Some(7)
    } else if n.ends_with("_boots") {
        Some(8)
    } else {
        None
    }
}

fn offhand_item(item: u32) -> bool {
    item_name(item) == "minecraft:shield"
}

/// Items that leave a bucket behind when used as a crafting ingredient.
fn remainder(item: u32) -> Option<u32> {
    if item_name(item).ends_with("_bucket") {
        item_id("minecraft:bucket")
    } else {
        None
    }
}

fn fuel_of(name: &str) -> i32 {
    let short = name.strip_prefix("minecraft:").unwrap_or(name);
    match short {
        "lava_bucket" => 20000,
        "coal_block" => 16000,
        "dried_kelp_block" => 4001,
        "blaze_rod" => 2400,
        "coal" | "charcoal" => 1600,
        "stick" => 100,
        "bamboo" => 50,
        s if s.ends_with("_boat") => 1200,
        s if s.starts_with("wooden_") => 200,
        s if s.ends_with("_sapling") || s.ends_with("_wool") => 100,
        s if s.ends_with("_carpet") => 67,
        s if s.ends_with("_planks")
            || s.ends_with("_log")
            || s.ends_with("_wood")
            || s.ends_with("_fence")
            || s.ends_with("_fence_gate")
            || s.ends_with("_banner") =>
        {
            300
        }
        _ => 0,
    }
}

fn fuel_time(item: u32) -> i32 {
    static FUEL: OnceLock<Vec<i32>> = OnceLock::new();
    let t = FUEL.get_or_init(|| rine_data::get().items.items.iter().map(|i| fuel_of(&i.name)).collect());
    t.get(item as usize).copied().unwrap_or(0)
}

fn may_place(lay: Lay, w: usize, item: u32) -> bool {
    if lay.result() == Some(w) {
        return false;
    }
    match lay {
        Lay::Inv if (5..=8).contains(&w) => armor_slot(item) == Some(w),
        Lay::Furnace(_) if w == 1 => fuel_time(item) > 0,
        _ => true,
    }
}

fn max_for(lay: Lay, w: usize, item: u32) -> i32 {
    if lay == Lay::Inv && (5..=8).contains(&w) {
        1
    } else {
        max_stack(item)
    }
}

// --------------------------------------------------------------- recipes

#[derive(Debug)]
struct Recipe {
    result: Stack,
    pat: Pat,
}

#[derive(Debug)]
enum Pat {
    /// Trimmed pattern; `None` cells must be empty, `Some` cells list the accepted items.
    Shaped { w: usize, h: usize, cells: Vec<Option<Vec<u32>>> },
    Shapeless(Vec<Vec<u32>>),
}

/// Accepts a recipe ingredient: "minecraft:x", "#minecraft:tag", a list, or {"item": ...}.
fn resolve(v: &Value) -> Option<Vec<u32>> {
    let mut out = Vec::new();
    collect_items(v, &mut out)?;
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn collect_items(v: &Value, out: &mut Vec<u32>) -> Option<()> {
    match v {
        Value::String(s) => match s.strip_prefix('#') {
            Some(tag) => {
                let names = rine_data::get().registries.tags.get("minecraft:item")?.get(tag)?;
                out.extend(names.iter().filter_map(|n| item_id(n)));
            }
            None => out.push(item_id(s)?),
        },
        Value::Array(a) => {
            for x in a {
                collect_items(x, out)?;
            }
        }
        Value::Object(o) => collect_items(o.get("item")?, out)?,
        _ => return None,
    }
    Some(())
}

/// Result stack; recipes with result components are skipped (not modelled).
fn parse_result(v: &Value) -> Option<Stack> {
    if v.get("components").is_some() {
        return None;
    }
    let item = item_id(v.get("id")?.as_str()?)?;
    let n = v.get("count").and_then(Value::as_i64).unwrap_or(1) as i32;
    let s = Stack::new(item, n);
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn shaped_pattern(r: &Value) -> Option<Pat> {
    let mut keys: AHashMap<char, Vec<u32>> = AHashMap::new();
    for (k, v) in r.get("key")?.as_object()? {
        let c = k.chars().next()?;
        keys.insert(c, resolve(v)?);
    }
    let rows: Vec<Vec<char>> = r.get("pattern")?.as_array()?.iter().map(|x| x.as_str().unwrap_or("").chars().collect()).collect();
    let (mut r0, mut r1, mut c0, mut c1) = (usize::MAX, 0usize, usize::MAX, 0usize);
    for (i, row) in rows.iter().enumerate() {
        for (j, &ch) in row.iter().enumerate() {
            if ch != ' ' {
                r0 = r0.min(i);
                r1 = r1.max(i);
                c0 = c0.min(j);
                c1 = c1.max(j);
            }
        }
    }
    if r0 == usize::MAX {
        return None;
    }
    let (w, h) = (c1 - c0 + 1, r1 - r0 + 1);
    let mut cells = Vec::with_capacity(w * h);
    for row in rows.iter().take(r1 + 1).skip(r0) {
        for j in c0..=c1 {
            let ch = row.get(j).copied().unwrap_or(' ');
            cells.push(if ch == ' ' { None } else { Some(keys.get(&ch)?.clone()) });
        }
    }
    Some(Pat::Shaped { w, h, cells })
}

fn build_recipe(r: &Value) -> Option<Recipe> {
    let result = parse_result(r.get("result")?)?;
    let pat = match r.get("type")?.as_str()? {
        "minecraft:crafting_shaped" => shaped_pattern(r)?,
        "minecraft:crafting_shapeless" => Pat::Shapeless(r.get("ingredients")?.as_array()?.iter().map(resolve).collect::<Option<Vec<_>>>()?),
        "minecraft:crafting_transmute" => Pat::Shapeless(vec![resolve(r.get("input")?)?, resolve(r.get("material")?)?]),
        _ => return None,
    };
    if let Pat::Shapeless(v) = &pat {
        if v.is_empty() || v.len() > 9 {
            return None;
        }
    }
    Some(Recipe { result, pat })
}

fn recipes() -> &'static [Recipe] {
    static RECIPES: OnceLock<Vec<Recipe>> = OnceLock::new();
    RECIPES.get_or_init(|| {
        let root: Value = serde_json::from_str(rine_data::raw::RECIPES).expect("recipes.json");
        let mut out = Vec::new();
        if let Some(map) = root.as_object() {
            for (_, r) in map {
                if let Some(rec) = build_recipe(r) {
                    out.push(rec);
                }
            }
        }
        out
    })
}

/// Result of a 2x2 or 3x3 grid (row-major, `grid.len() == width * width`).
pub fn craft_result(width: usize, grid: &[Stack]) -> Stack {
    let (mut r0, mut r1, mut c0, mut c1) = (usize::MAX, 0usize, usize::MAX, 0usize);
    let mut items: Vec<u32> = Vec::new();
    for (i, s) in grid.iter().enumerate() {
        if s.is_empty() {
            continue;
        }
        let (r, c) = (i / width, i % width);
        r0 = r0.min(r);
        r1 = r1.max(r);
        c0 = c0.min(c);
        c1 = c1.max(c);
        items.push(s.item);
    }
    if items.is_empty() {
        return Stack::EMPTY;
    }
    let (bw, bh) = (c1 - c0 + 1, r1 - r0 + 1);
    for rec in recipes() {
        match &rec.pat {
            Pat::Shaped { w, h, cells } => {
                if *w != bw || *h != bh {
                    continue;
                }
                if shaped_ok(grid, width, (r0, c0), (*w, *h), cells, false) || shaped_ok(grid, width, (r0, c0), (*w, *h), cells, true) {
                    return rec.result;
                }
            }
            Pat::Shapeless(ings) => {
                if ings.len() == items.len() && assign(ings, &items, &mut vec![false; items.len()], 0) {
                    return rec.result;
                }
            }
        }
    }
    Stack::EMPTY
}

fn shaped_ok(grid: &[Stack], width: usize, origin: (usize, usize), size: (usize, usize), cells: &[Option<Vec<u32>>], mirror: bool) -> bool {
    let (w, h) = size;
    for i in 0..h {
        for j in 0..w {
            let jj = if mirror { w - 1 - j } else { j };
            let g = grid[(origin.0 + i) * width + origin.1 + j];
            match &cells[i * w + jj] {
                None => {
                    if !g.is_empty() {
                        return false;
                    }
                }
                Some(list) => {
                    if g.is_empty() || !list.contains(&g.item) {
                        return false;
                    }
                }
            }
        }
    }
    true
}

/// Matches each shapeless ingredient to a distinct grid item (backtracking).
fn assign(ings: &[Vec<u32>], items: &[u32], used: &mut [bool], k: usize) -> bool {
    if k == ings.len() {
        return true;
    }
    for (i, &it) in items.iter().enumerate() {
        if !used[i] && ings[k].contains(&it) {
            used[i] = true;
            if assign(ings, items, used, k + 1) {
                return true;
            }
            used[i] = false;
        }
    }
    false
}

struct Smelt {
    result: Stack,
    time: i32,
}

#[derive(Default)]
struct Smelting {
    furnace: AHashMap<u32, Smelt>,
    blast: AHashMap<u32, Smelt>,
    smoker: AHashMap<u32, Smelt>,
}

fn smelting() -> &'static Smelting {
    static S: OnceLock<Smelting> = OnceLock::new();
    S.get_or_init(|| {
        let root: Value = serde_json::from_str(rine_data::raw::RECIPES).expect("recipes.json");
        let mut s = Smelting::default();
        let Some(map) = root.as_object() else { return s };
        for (_, r) in map {
            let (table, halve) = match r.get("type").and_then(Value::as_str).unwrap_or("") {
                "minecraft:smelting" => (&mut s.furnace, false),
                "minecraft:blasting" => (&mut s.blast, true),
                "minecraft:smoking" => (&mut s.smoker, true),
                _ => continue,
            };
            let (Some(ing), Some(result)) = (resolve(&r["ingredient"]), parse_result(&r["result"])) else { continue };
            let mut t = r.get("cookingtime").and_then(Value::as_i64).unwrap_or(200) as i32;
            if halve {
                t = (t + 1) / 2;
            }
            let t = t.max(1);
            for id in ing {
                table.entry(id).or_insert(Smelt { result, time: t });
            }
        }
        s
    })
}

fn smelt_for(kind: Fk, item: u32) -> Option<&'static Smelt> {
    let s = smelting();
    match kind {
        Fk::Furnace => s.furnace.get(&item),
        Fk::Blast => s.blast.get(&item),
        Fk::Smoker => s.smoker.get(&item),
    }
}

// ----------------------------------------------------------- click logic

/// Applies one click to a window. `win` has `lay.work_len()` entries.
/// Pure: side effects are returned in `fx`.
#[allow(clippy::too_many_arguments)]
pub fn apply_click(lay: Lay, win: &mut [Stack], cursor: &mut Stack, drag: &mut Drag, slot: i32, button: i32, mode: i32, creative: bool, fx: &mut Fx) {
    if mode == 5 {
        quick_craft(lay, win, cursor, drag, slot, button, creative, fx);
    } else if drag.status != 0 {
        *drag = Drag::default();
    } else {
        let w = (slot >= 0 && (slot as usize) < lay.size()).then_some(slot as usize);
        match mode {
            0 | 1 if button == 0 || button == 1 => {
                if slot == -999 {
                    if !cursor.is_empty() {
                        if button == 0 {
                            fx.throw.push(*cursor);
                            *cursor = Stack::EMPTY;
                        } else {
                            fx.throw.push(cursor.with_count(1));
                            cursor.count -= 1;
                            norm(cursor);
                        }
                    }
                } else if let Some(w) = w {
                    if mode == 1 {
                        quick_move(lay, win, w, fx);
                    } else {
                        pickup(lay, win, cursor, w, button == 0, fx);
                    }
                }
            }
            2 => {
                if let Some(w) = w {
                    swap(lay, win, w, button, fx);
                }
            }
            3 => {
                if let Some(w) = w {
                    if creative && cursor.is_empty() && !win[w].is_empty() {
                        *cursor = win[w].with_count(max_stack(win[w].item));
                    }
                }
            }
            4 => {
                if let Some(w) = w {
                    if cursor.is_empty() {
                        throw_slot(lay, win, w, button, fx);
                    }
                }
            }
            6 => {
                if let Some(w) = w {
                    pickup_all(lay, win, cursor, w, button == 0);
                }
            }
            _ => {}
        }
    }
    refresh(lay, win);
}

fn can_replace(s: Stack, c: Stack) -> bool {
    s.is_empty() || (s.item == c.item && s.count <= max_stack(c.item))
}

fn pickup(lay: Lay, win: &mut [Stack], cursor: &mut Stack, w: usize, primary: bool, fx: &mut Fx) {
    if lay.craft() && lay.result() == Some(w) {
        // Right-click takes the whole result too (vanilla takes half and still
        // consumes one craft; not modelled).
        let r = win[w];
        if r.is_empty() {
            return;
        }
        if cursor.is_empty() {
            *cursor = r;
        } else if cursor.item == r.item && cursor.count + r.count <= max_stack(r.item) {
            cursor.count += r.count;
        } else {
            return;
        }
        consume_craft(lay, win, fx);
        return;
    }
    let clicked = win[w];
    if clicked.is_empty() {
        if !cursor.is_empty() && may_place(lay, w, cursor.item) {
            let want = if primary { cursor.count } else { 1 };
            let n = want.min(max_for(lay, w, cursor.item));
            win[w] = cursor.with_count(n);
            cursor.count -= n;
            norm(cursor);
        }
    } else if cursor.is_empty() {
        let amount = if primary { clicked.count } else { (clicked.count + 1) / 2 };
        *cursor = clicked.with_count(amount);
        win[w].count -= amount;
        norm(&mut win[w]);
    } else if may_place(lay, w, cursor.item) {
        if clicked.item == cursor.item {
            let amount = if primary { cursor.count } else { 1 };
            let room = max_for(lay, w, clicked.item) - clicked.count;
            let n = amount.min(room).min(cursor.count).max(0);
            win[w].count += n;
            cursor.count -= n;
            norm(cursor);
        } else if cursor.count <= max_for(lay, w, cursor.item) {
            std::mem::swap(&mut win[w], cursor);
        }
    } else if clicked.item == cursor.item {
        let room = max_stack(cursor.item) - cursor.count;
        let n = clicked.count.min(room).max(0);
        cursor.count += n;
        win[w].count -= n;
        norm(&mut win[w]);
    }
}

/// Shift-click (`doClick` QUICK_MOVE with each menu's `quickMoveStack`).
fn quick_move(lay: Lay, win: &mut [Stack], w: usize, fx: &mut Fx) {
    if win[w].is_empty() {
        return;
    }
    if lay.craft() && lay.result() == Some(w) {
        // Vanilla repeats while the result keeps its item: craft as many as fit.
        for _ in 0..64 {
            let before = win[0];
            if before.is_empty() {
                break;
            }
            let mut tmp = win.to_vec();
            let mut s = tmp[0];
            if !route(lay, &mut tmp, 0, &mut s) || !s.is_empty() {
                break;
            }
            tmp[0] = Stack::EMPTY;
            consume(lay, &mut tmp, fx);
            refresh(lay, &mut tmp);
            win.copy_from_slice(&tmp);
            if win[0].item != before.item {
                break;
            }
        }
        return;
    }
    let mut s = win[w];
    if route(lay, win, w, &mut s) {
        win[w] = s;
        norm(&mut win[w]);
    }
}

/// Destinations of a shift-click, in vanilla order. Returns false when nothing moved.
fn route(lay: Lay, win: &mut [Stack], w: usize, s: &mut Stack) -> bool {
    match lay {
        Lay::Inv => {
            if w == 0 {
                return mv(lay, win, s, 9, 45, true);
            }
            if (1..=8).contains(&w) {
                return mv(lay, win, s, 9, 45, false);
            }
            if let Some(a) = armor_slot(s.item) {
                if win[a].is_empty() {
                    return mv(lay, win, s, a, a + 1, false);
                }
            }
            if offhand_item(s.item) && win[45].is_empty() {
                return mv(lay, win, s, 45, 46, false);
            }
            if (9..36).contains(&w) {
                return mv(lay, win, s, 36, 45, false);
            }
            if (36..45).contains(&w) {
                return mv(lay, win, s, 9, 36, false);
            }
            mv(lay, win, s, 9, 45, false)
        }
        Lay::Table => {
            if w == 0 {
                return mv(lay, win, s, 10, 46, true);
            }
            if (10..46).contains(&w) {
                if mv(lay, win, s, 1, 10, false) {
                    return true;
                }
                if w < 37 {
                    return mv(lay, win, s, 37, 46, false);
                }
                return mv(lay, win, s, 10, 37, false);
            }
            mv(lay, win, s, 10, 46, false)
        }
        Lay::Chest => {
            if w < 27 {
                mv(lay, win, s, 27, 63, true)
            } else {
                mv(lay, win, s, 0, 27, false)
            }
        }
        Lay::Furnace(fk) => {
            if w == 2 {
                return mv(lay, win, s, 3, 39, true);
            }
            if w < 2 {
                return mv(lay, win, s, 3, 39, false);
            }
            if smelt_for(fk, s.item).is_some() {
                return mv(lay, win, s, 0, 1, false);
            }
            if fuel_time(s.item) > 0 {
                return mv(lay, win, s, 1, 2, false);
            }
            if (3..30).contains(&w) {
                return mv(lay, win, s, 30, 39, false);
            }
            if (30..39).contains(&w) {
                return mv(lay, win, s, 3, 30, false);
            }
            false
        }
    }
}

/// `moveItemStackTo`: merge into matching stacks first, then fill empty slots.
fn mv(lay: Lay, win: &mut [Stack], s: &mut Stack, start: usize, end: usize, back: bool) -> bool {
    if s.is_empty() {
        return false;
    }
    let item = s.item;
    let order: Vec<usize> = if back { (start..end).rev().collect() } else { (start..end).collect() };
    let mut moved = false;
    for &i in &order {
        if s.is_empty() {
            break;
        }
        if !win[i].is_empty() && win[i].item == item {
            let room = max_for(lay, i, item) - win[i].count;
            let n = room.min(s.count).max(0);
            if n > 0 {
                win[i].count += n;
                s.count -= n;
                moved = true;
            }
        }
    }
    for &i in &order {
        if s.is_empty() {
            break;
        }
        if win[i].is_empty() && may_place(lay, i, item) {
            let n = max_for(lay, i, item).min(s.count);
            win[i] = Stack::new(item, n);
            s.count -= n;
            moved = true;
        }
    }
    norm(s);
    moved
}

/// Number-key swap with hotbar slot `button` (0..8) or offhand (40).
fn swap(lay: Lay, win: &mut [Stack], w: usize, button: i32, fx: &mut Fx) {
    let src = match button {
        0..=8 => match lay.from_inv(36 + button as usize) {
            Some(i) => i,
            None => return,
        },
        40 => lay.offhand(),
        _ => return,
    };
    if src == w {
        return;
    }
    let (source, target) = (win[src], win[w]);
    if source.is_empty() && target.is_empty() {
        return;
    }
    if source.is_empty() {
        if lay.craft() && lay.result() == Some(w) {
            win[src] = target;
            consume_craft(lay, win, fx);
            return;
        }
        win[src] = target;
        win[w] = Stack::EMPTY;
        return;
    }
    if target.is_empty() {
        if may_place(lay, w, source.item) {
            let max = max_for(lay, w, source.item);
            if source.count > max {
                win[w] = source.with_count(max);
                win[src].count -= max;
            } else {
                win[w] = source;
                win[src] = Stack::EMPTY;
            }
        }
        return;
    }
    if may_place(lay, w, source.item) {
        let max = max_for(lay, w, source.item);
        if source.count > max {
            win[w] = source.with_count(max);
            win[src].count -= max;
            fx.give.push(target);
        } else {
            win[src] = target;
            win[w] = source;
        }
    }
}

fn throw_slot(lay: Lay, win: &mut [Stack], w: usize, button: i32, fx: &mut Fx) {
    if win[w].is_empty() {
        return;
    }
    let s = win[w];
    let n = if button == 0 { 1 } else { s.count };
    if lay.craft() && lay.result() == Some(w) {
        fx.throw.push(s.with_count(n));
        consume_craft(lay, win, fx);
        return;
    }
    win[w].count -= n;
    norm(&mut win[w]);
    fx.throw.push(s.with_count(n));
}

/// Double-click: gathers matching items into the cursor (crafting results excluded).
fn pickup_all(lay: Lay, win: &mut [Stack], cursor: &mut Stack, w: usize, first: bool) {
    if cursor.is_empty() || !win[w].is_empty() {
        return;
    }
    let max = max_stack(cursor.item);
    let order: Vec<usize> = if first { (0..lay.size()).collect() } else { (0..lay.size()).rev().collect() };
    for pass in 0..2 {
        for &i in &order {
            if cursor.count >= max {
                return;
            }
            if lay.result() == Some(i) {
                continue;
            }
            let t = win[i];
            if t.is_empty() || t.item != cursor.item {
                continue;
            }
            if pass == 0 && t.count == max_stack(t.item) {
                continue;
            }
            let n = t.count.min(max - cursor.count);
            cursor.count += n;
            win[i].count -= n;
            norm(&mut win[i]);
        }
    }
}

/// Drag (QUICK_CRAFT): header 0 start, 1 add slot, 2 end. Type 0 left
/// (split evenly), 1 right (one each), 2 creative clone.
#[allow(clippy::too_many_arguments)]
fn quick_craft(lay: Lay, win: &mut [Stack], cursor: &mut Stack, drag: &mut Drag, slot: i32, button: i32, creative: bool, fx: &mut Fx) {
    let header = button & 3;
    let ty = (button >> 2) & 3;
    let expected = drag.status as i32;
    if (expected != 1 || header != 2) && expected != header {
        *drag = Drag::default();
        return;
    }
    if cursor.is_empty() {
        *drag = Drag::default();
        return;
    }
    match header {
        0 => {
            if ty == 0 || ty == 1 || (ty == 2 && creative) {
                drag.status = 1;
                drag.ty = ty;
                drag.slots.clear();
            } else {
                *drag = Drag::default();
            }
        }
        1 => {
            if slot >= 0 && (slot as usize) < lay.size() {
                let w = slot as usize;
                let cur = *cursor;
                if can_replace(win[w], cur)
                    && may_place(lay, w, cur.item)
                    && (drag.ty == 2 || cur.count > drag.slots.len() as i32)
                    && !drag.slots.contains(&w)
                {
                    drag.slots.push(w);
                }
            }
        }
        2 => {
            let slots = std::mem::take(&mut drag.slots);
            let ty = drag.ty;
            *drag = Drag::default();
            if slots.is_empty() {
                return;
            }
            if slots.len() == 1 {
                if ty == 0 || ty == 1 {
                    pickup(lay, win, cursor, slots[0], ty == 0, fx);
                }
                return;
            }
            let source = *cursor;
            let mut remaining = cursor.count;
            let mut dragged: Vec<(usize, Stack)> = Vec::new();
            for &w in &slots {
                let cur = *cursor;
                if can_replace(win[w], cur) && may_place(lay, w, cur.item) && (ty == 2 || cur.count >= slots.len() as i32) {
                    let carry = if win[w].is_empty() { 0 } else { win[w].count };
                    let max_size = max_stack(source.item).min(max_for(lay, w, source.item));
                    let place = match ty {
                        0 => source.count / slots.len() as i32,
                        1 => 1,
                        _ => max_stack(source.item),
                    };
                    let new_count = (place + carry).min(max_size);
                    remaining -= new_count - carry;
                    dragged.push((w, source.with_count(new_count)));
                }
            }
            for (w, s) in dragged {
                win[w] = s;
            }
            if ty != 2 {
                cursor.count = remaining;
                norm(cursor);
            }
        }
        _ => *drag = Drag::default(),
    }
}

/// Crafting: the grid items of one craft are consumed; buckets are left behind.
fn consume(lay: Lay, win: &mut [Stack], fx: &mut Fx) {
    let Some((first, width)) = lay.grid() else { return };
    for i in first..first + width * width {
        if win[i].is_empty() {
            continue;
        }
        let item = win[i].item;
        win[i].count -= 1;
        norm(&mut win[i]);
        if let Some(rem) = remainder(item) {
            if win[i].is_empty() {
                win[i] = Stack::new(rem, 1);
            } else {
                fx.give.push(Stack::new(rem, 1));
            }
        }
    }
}

fn consume_craft(lay: Lay, win: &mut [Stack], fx: &mut Fx) {
    consume(lay, win, fx);
    refresh(lay, win);
}

/// Recomputes the crafting result slot from the grid.
fn refresh(lay: Lay, win: &mut [Stack]) {
    let Some((first, width)) = lay.grid() else { return };
    win[0] = craft_result(width, &win[first..first + width * width]);
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: &str) -> u32 {
        items::id(n).unwrap_or_else(|| panic!("no item {n}"))
    }

    fn st(n: &str, c: i32) -> Stack {
        Stack::new(id(n), c)
    }

    fn init() {
        rine_data::init();
    }

    fn inv() -> Vec<Stack> {
        vec![Stack::EMPTY; 46]
    }

    fn run(win: &mut [Stack], cursor: &mut Stack, slot: i32, button: i32, mode: i32) -> Fx {
        let mut fx = Fx::default();
        let mut drag = Drag::default();
        apply_click(Lay::Inv, win, cursor, &mut drag, slot, button, mode, false, &mut fx);
        fx
    }

    #[test]
    fn four_planks_make_crafting_table() {
        init();
        let p = st("minecraft:oak_planks", 1);
        let r = craft_result(2, &[p; 4]);
        assert_eq!(r.item, id("minecraft:crafting_table"));
        assert_eq!(r.count, 1);
    }

    #[test]
    fn one_log_makes_four_planks() {
        init();
        let r = craft_result(2, &[st("minecraft:oak_log", 1), Stack::EMPTY, Stack::EMPTY, Stack::EMPTY]);
        assert_eq!(r.item, id("minecraft:oak_planks"));
        assert_eq!(r.count, 4);
    }

    #[test]
    fn two_planks_vertical_make_sticks() {
        init();
        let p = st("minecraft:oak_planks", 1);
        let r = craft_result(2, &[p, Stack::EMPTY, p, Stack::EMPTY]);
        assert_eq!(r.item, id("minecraft:stick"));
        assert_eq!(r.count, 4);
    }

    #[test]
    fn wooden_pickaxe_in_3x3() {
        init();
        let p = st("minecraft:oak_planks", 1);
        let s = st("minecraft:stick", 1);
        let grid = [p, p, p, Stack::EMPTY, s, Stack::EMPTY, Stack::EMPTY, s, Stack::EMPTY];
        let r = craft_result(3, &grid);
        assert_eq!(r.item, id("minecraft:wooden_pickaxe"));
        assert_eq!(r.count, 1);
    }

    #[test]
    fn asymmetric_recipe_matches_mirrored() {
        init();
        // Wooden axe: "##", "#|", " |" (stick column on the right of the head).
        // Its mirror image must match as well.
        let p = st("minecraft:oak_planks", 1);
        let s = st("minecraft:stick", 1);
        let e = Stack::EMPTY;
        let normal = [p, p, e, p, s, e, e, s, e];
        let mirrored = [e, p, p, e, s, p, e, s, e];
        assert_eq!(craft_result(3, &normal).item, id("minecraft:wooden_axe"));
        assert_eq!(craft_result(3, &mirrored).item, id("minecraft:wooden_axe"));
    }

    #[test]
    fn furnace_smelts_raw_iron() {
        init();
        let s = smelt_for(Fk::Furnace, id("minecraft:raw_iron")).expect("raw_iron smelting");
        assert_eq!(s.result.item, id("minecraft:iron_ingot"));
        assert_eq!(s.time, 200);
    }

    #[test]
    fn left_click_places_then_merges_then_swaps() {
        init();
        let stone = id("minecraft:stone");
        let dirt = id("minecraft:dirt");
        let mut win = inv();
        let mut cur = Stack::new(stone, 5);
        run(&mut win, &mut cur, 9, 0, 0);
        assert_eq!(win[9], Stack::new(stone, 5));
        assert!(cur.is_empty());

        let mut cur = Stack::new(stone, 10);
        run(&mut win, &mut cur, 9, 0, 0);
        assert_eq!(win[9], Stack::new(stone, 15));
        assert!(cur.is_empty());

        let mut cur = Stack::new(dirt, 3);
        win[9] = Stack::new(stone, 2);
        run(&mut win, &mut cur, 9, 0, 0);
        assert_eq!(win[9], Stack::new(dirt, 3));
        assert_eq!(cur, Stack::new(stone, 2));
    }

    #[test]
    fn right_click_places_one_and_splits_half() {
        init();
        let stone = id("minecraft:stone");
        let mut win = inv();
        let mut cur = Stack::new(stone, 5);
        run(&mut win, &mut cur, 10, 1, 0);
        assert_eq!(win[10], Stack::new(stone, 1));
        assert_eq!(cur, Stack::new(stone, 4));

        let mut win = inv();
        win[10] = Stack::new(stone, 5);
        let mut cur = Stack::EMPTY;
        run(&mut win, &mut cur, 10, 1, 0);
        assert_eq!(cur, Stack::new(stone, 3));
        assert_eq!(win[10], Stack::new(stone, 2));
    }

    #[test]
    fn shift_click_hotbar_moves_to_main() {
        init();
        let stone = id("minecraft:stone");
        let mut win = inv();
        win[36] = Stack::new(stone, 10);
        let mut cur = Stack::EMPTY;
        run(&mut win, &mut cur, 36, 0, 1);
        assert_eq!(win[9], Stack::new(stone, 10));
        assert!(win[36].is_empty());
    }

    #[test]
    fn shift_click_helmet_goes_to_head_slot() {
        init();
        let helmet = id("minecraft:iron_helmet");
        let mut win = inv();
        win[9] = Stack::new(helmet, 1);
        let mut cur = Stack::EMPTY;
        run(&mut win, &mut cur, 9, 0, 1);
        assert_eq!(win[5], Stack::new(helmet, 1));
        assert!(win[9].is_empty());
    }

    #[test]
    fn drag_split_evenly_with_left_drag() {
        init();
        let stone = id("minecraft:stone");
        let mut win = inv();
        let mut cur = Stack::new(stone, 6);
        let mut drag = Drag::default();
        let mut fx = Fx::default();
        // start (type 0 left, header 0), add 9, add 10, end (header 2)
        for (slot, button) in [(-999, 0), (9, 1), (10, 1), (-999, 2)] {
            apply_click(Lay::Inv, &mut win, &mut cur, &mut drag, slot, button, 5, false, &mut fx);
        }
        assert_eq!(win[9], Stack::new(stone, 3));
        assert_eq!(win[10], Stack::new(stone, 3));
        assert!(cur.is_empty());
        assert_eq!(drag.status, 0);
    }

    #[test]
    fn shift_click_result_crafts_table_and_consumes_grid() {
        init();
        let p = st("minecraft:oak_planks", 1);
        let mut win = inv();
        for i in 1..5 {
            win[i] = p;
        }
        refresh(Lay::Inv, &mut win);
        assert_eq!(win[0].item, id("minecraft:crafting_table"));
        let mut cur = Stack::EMPTY;
        run(&mut win, &mut cur, 0, 0, 1);
        assert!(win[1..5].iter().all(|s| s.is_empty()));
        assert!(win[0].is_empty());
        let table: i32 = win[9..45].iter().filter(|s| s.item == id("minecraft:crafting_table")).map(|s| s.count).sum();
        assert_eq!(table, 1);
    }

    #[test]
    fn furnace_fuel_times() {
        init();
        assert_eq!(fuel_time(id("minecraft:coal")), 1600);
        assert_eq!(fuel_time(id("minecraft:lava_bucket")), 20000);
        assert_eq!(fuel_time(id("minecraft:stone")), 0);
    }
}
