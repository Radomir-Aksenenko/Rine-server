//! The redstone engine: a port of vanilla `Level.setBlock` side effects,
//! `CollectingNeighborUpdater`, `LevelTicks` scheduling, `SignalGetter` and the
//! redstone blocks' `neighborChanged` / `tick` / `onPlace` /
//! `affectNeighborsAfterRemoval` / `updateShape` (Paper 26.3 sources, vanilla
//! behaviour only).
//!
//! Everything here works on the `Lvl` trait, so the logic is unit-tested on a
//! plain in-memory world.

use super::data::*;
use super::piston;
use ahash::{AHashMap, AHashSet};
use rine_protocol::BlockPos;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};

/// `Block.UPDATE_*` flags.
pub mod fl {
    pub const NEIGHBORS: u32 = 1;
    pub const CLIENTS: u32 = 2;
    pub const INVISIBLE: u32 = 4;
    pub const IMMEDIATE: u32 = 8;
    pub const KNOWN_SHAPE: u32 = 16;
    pub const SUPPRESS_DROPS: u32 = 32;
    pub const MOVE_BY_PISTON: u32 = 64;
    pub const SKIP_WIRE_SHAPE: u32 = 128;
    pub const SKIP_BE_SIDE_EFFECTS: u32 = 256;
    pub const SKIP_ON_PLACE: u32 = 512;
    pub const ALL: u32 = NEIGHBORS | CLIENTS;
    pub const NONE: u32 = INVISIBLE | SKIP_BE_SIDE_EFFECTS;
}

pub const UPDATE_LIMIT: i32 = 512;
/// `maxChainedNeighborUpdates` (server.properties default).
const MAX_CHAINED: usize = 1_000_000;
/// `maxBlockTicks` per game tick (vanilla 65536).
const MAX_TICKS: usize = 65_536;

/// `TickPriority` values.
pub mod prio {
    pub const EXTREMELY_HIGH: i8 = -3;
    pub const VERY_HIGH: i8 = -2;
    pub const HIGH: i8 = -1;
    pub const NORMAL: i8 = 0;
}

/// Sound categories (`SoundSource` ordinals).
pub const CAT_RECORDS: i32 = 2;
pub const CAT_BLOCKS: i32 = 4;

/// The world as the engine sees it.
pub trait Lvl {
    /// Block state; air for unloaded positions.
    fn get(&self, p: BlockPos) -> u16;
    fn loaded(&self, p: BlockPos) -> bool;
    /// Store a state and send it to clients. Returns the old state.
    fn raw_set(&mut self, p: BlockPos, st: u16) -> u16;
    /// Game time (`getGameTime`).
    fn time(&self) -> u64;
    /// Time of day (`getDayTime`).
    fn day_time(&self) -> i64;
    fn sky_light(&self, p: BlockPos) -> u8;
    fn level_event(&mut self, ev: i32, p: BlockPos, data: i32);
    /// Play a sound at the block center. `skip_actor`: not to the player who caused it.
    fn sound(&mut self, name: &str, p: BlockPos, vol: f32, pitch: f32, cat: i32, skip_actor: bool);
    /// `ClientboundBlockEventPacket` to players in range.
    fn block_event_packet(&mut self, p: BlockPos, block: u16, a: u8, b: u8);
    /// Drop the loot of a destroyed block.
    fn drop_block(&mut self, p: BlockPos, st: u16);
    /// Spawn primed TNT at the block.
    fn prime_tnt(&mut self, p: BlockPos);
    /// Entities touching the box (spectators excluded).
    fn count_entities(&self, min: [f64; 3], max: [f64; 3], living_only: bool) -> usize;
    /// Comparator reading of a container block (chest, barrel, furnace...).
    fn container_signal(&self, p: BlockPos, st: u16) -> Option<u8>;
    /// Players looking into a chest (trapped chest output).
    fn chest_viewers(&self, p: BlockPos) -> u8;
    /// Uniform in [0, 1).
    fn rand(&mut self) -> f32;
    /// `DispenserBlock.dispenseFrom` (the default: an empty dispenser clicks).
    fn dispense(&mut self, p: BlockPos, _st: u16) {
        self.level_event(1001, p, 0);
    }
    /// Custom block hooks (command blocks...). Return true when handled.
    fn hook_neighbor(&mut self, _rs: &mut Rs, _p: BlockPos, _st: u16) -> bool {
        false
    }
    fn hook_tick(&mut self, _rs: &mut Rs, _p: BlockPos, _st: u16) -> bool {
        false
    }
    fn has_hooks(&self, _block: u16) -> bool {
        false
    }
    /// Block light at a position (`getBrightness(LightLayer.BLOCK, pos)`).
    fn block_light(&self, _p: BlockPos) -> u8 {
        0
    }
    /// `Level.skyDarken`: how much the sky light is dimmed now (time, weather).
    fn sky_darken(&self) -> u8 {
        0
    }
    fn raining(&self) -> bool {
        false
    }
    fn thundering(&self) -> bool {
        false
    }
    /// Synced biome registry id at a position.
    fn biome(&self, _p: BlockPos) -> u8 {
        0
    }
    /// `Biome.getTemperature(pos, seaLevel)` of a synced biome id at a position.
    fn temperature(&self, _biome: u8, _p: BlockPos) -> f32 {
        0.8
    }
    /// `Biome.hasPrecipitation` of a synced biome id.
    fn has_precipitation(&self, _biome: u8) -> bool {
        true
    }
    /// Highest Y+1 of a motion-blocking block in the column (`MOTION_BLOCKING` heightmap).
    fn height_motion_blocking(&self, _x: i32, _z: i32) -> i32 {
        rine_world::MIN_Y
    }
    /// Game rules; None in tests (callers use vanilla defaults).
    fn rules(&self) -> Option<&crate::game::cmd::rules::GameRules> {
        None
    }
    /// `Block.popResource`: drop an item stack at the block.
    fn pop_resource(&mut self, _p: BlockPos, _item: u32, _count: i32) {}
    /// Escape hatch to the whole game for behaviour the trait does not cover.
    fn game(&mut self) -> Option<&mut crate::game::Game> {
        None
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tick {
    pub due: u64,
    pub prio: i8,
    pub sub: u64,
    pub pos: BlockPos,
    pub block: u16,
}

impl Ord for Tick {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        (self.due, self.prio, self.sub).cmp(&(o.due, o.prio, o.sub))
    }
}

impl PartialOrd for Tick {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

/// One entry of the collecting neighbor updater.
#[derive(Clone, Copy, Debug)]
enum Upd {
    Shape {
        dir: u8,
        nstate: u16,
        pos: BlockPos,
        npos: BlockPos,
        flags: u32,
        limit: i32,
    },
    Simple {
        pos: BlockPos,
        block: u16,
    },
    Full {
        pos: BlockPos,
        state: u16,
        block: u16,
        piston: bool,
    },
    /// `skip` 6 = none.
    Multi {
        src: BlockPos,
        block: u16,
        skip: u8,
        idx: u8,
    },
}

/// `PistonMovingBlockEntity`.
#[derive(Clone, Copy, Debug)]
pub struct Moving {
    pub state: u16,
    pub dir: u8,
    pub extending: bool,
    pub source: bool,
    pub progress: f32,
    pub progress_o: f32,
    pub last_ticked: u64,
    /// Identity, so a replaced block entity is not ticked twice.
    pub uid: u64,
}

/// Engine state: scheduled ticks, block events, updater stack and the block
/// entity data the redstone blocks keep.
pub struct Rs {
    heap: BinaryHeap<Reverse<Tick>>,
    pending: AHashSet<(BlockPos, u16)>,
    run: VecDeque<Tick>,
    run_set: AHashSet<(BlockPos, u16)>,
    sub: u64,
    stack: Vec<Upd>,
    added: Vec<Upd>,
    count: usize,
    /// `RedstoneWireBlock.shouldSignal`.
    pub wire_signals: bool,
    pub block_events: VecDeque<(BlockPos, u16, u8, u8)>,
    bev_set: AHashSet<(BlockPos, u16, u8, u8)>,
    /// `ComparatorBlockEntity.output`.
    pub comparators: AHashMap<BlockPos, u8>,
    pub moving: AHashMap<BlockPos, Moving>,
    /// Block entity ticker order of moving pistons.
    pub movers: Vec<(BlockPos, u64)>,
    pub next_uid: u64,
    pub daylight: AHashSet<BlockPos>,
    /// `Level.redstoneUpdateInfos` (torch burnout).
    torch_toggles: VecDeque<(BlockPos, u64)>,
    /// Inside the block tick loop (`ServerLevel.isHandlingTick`).
    pub handling_tick: bool,
    /// Scratch buffers reused by the piston code.
    pub scratch: piston::Scratch,
    /// Random source for block behaviour in scheduled ticks (`level.random`).
    pub rng: crate::game::blocks::Rng,
    /// Due ticks in unloaded chunks, by chunk; they run when it loads again
    /// (vanilla keeps them in the chunk's tick container).
    parked: AHashMap<(i32, i32), Vec<Tick>>,
}

impl Default for Rs {
    fn default() -> Self {
        Self {
            heap: BinaryHeap::new(),
            pending: AHashSet::new(),
            run: VecDeque::new(),
            run_set: AHashSet::new(),
            sub: 0,
            stack: Vec::new(),
            added: Vec::new(),
            count: 0,
            wire_signals: true,
            block_events: VecDeque::new(),
            bev_set: AHashSet::new(),
            comparators: AHashMap::new(),
            moving: AHashMap::new(),
            movers: Vec::new(),
            next_uid: 0,
            daylight: AHashSet::new(),
            torch_toggles: VecDeque::new(),
            handling_tick: false,
            scratch: Default::default(),
            rng: Default::default(),
            parked: AHashMap::new(),
        }
    }
}

impl Rs {
    pub fn scheduled_count(&self) -> usize {
        self.pending.len()
    }

    /// Schedule a tick due at game time `due` (no loaded check).
    pub fn schedule_at(&mut self, pos: BlockPos, blk: u16, due: u64, pr: i8) {
        if !self.pending.insert((pos, blk)) {
            return;
        }
        self.heap.push(Reverse(Tick { due, prio: pr, sub: self.sub, pos, block: blk }));
        self.sub += 1;
    }

    /// A chunk loaded: its overdue ticks run in the next tick pass, in the
    /// order they were scheduled.
    pub fn chunk_loaded(&mut self, cx: i32, cz: i32, now: u64) {
        let Some(list) = self.parked.remove(&(cx, cz)) else { return };
        for t in list {
            self.heap.push(Reverse(Tick { due: now, sub: self.sub, ..t }));
            self.sub += 1;
        }
    }

    /// A chunk is about to unload: finish its moving blocks (vanilla saves
    /// them and resumes after loading; Rine does not save block entities)
    /// and forget its block entity data.
    pub fn chunk_unloaded(&mut self, cx: i32, cz: i32, chunk: &mut rine_world::Chunk) {
        let inside = |p: &BlockPos| p.x >> 4 == cx && p.z >> 4 == cz;
        if self.moving.keys().any(inside) {
            let done: Vec<(BlockPos, Moving)> = self.moving.iter().filter(|(p, _)| inside(p)).map(|(p, m)| (*p, *m)).collect();
            for (p, m) in done {
                self.moving.remove(&p);
                if kind(chunk.get(p.x & 15, p.y, p.z & 15)) == K::MovingPiston {
                    let mut st = m.state;
                    if has(st, P::Waterlogged) {
                        st = setb(st, P::Waterlogged, false);
                    }
                    chunk.set(p.x & 15, p.y, p.z & 15, st);
                }
            }
            self.movers.retain(|(p, _)| !inside(p));
        }
        self.comparators.retain(|p, _| !inside(p));
        self.daylight.retain(|p| !inside(p));
    }

    /// Move ticks and block events queued on a temporary state into this one,
    /// keeping their relative order.
    pub fn absorb(&mut self, mut other: Rs) {
        if other.heap.is_empty() && other.block_events.is_empty() {
            return;
        }
        let mut ticks: Vec<Tick> = other.heap.drain().map(|Reverse(t)| t).collect();
        ticks.sort_by_key(|t| t.sub);
        for t in ticks {
            self.schedule_at(t.pos, t.block, t.due, t.prio);
        }
        for e in other.block_events.drain(..) {
            if self.bev_set.insert(e) {
                self.block_events.push_back(e);
            }
        }
    }
}

// ------------------------------------------------------------------ helpers

#[inline]
pub fn rel(p: BlockPos, d: u8) -> BlockPos {
    p.relative(d)
}

#[inline]
pub fn rel_n(p: BlockPos, d: u8, n: i32) -> BlockPos {
    let (x, y, z) = step(d);
    p.offset(x * n, y * n, z * n)
}

/// `Vec3i.hashCode` followed by `HashMap.hash`.
fn java_hash(p: BlockPos) -> u32 {
    let h = (p.y.wrapping_add(p.z.wrapping_mul(31))).wrapping_mul(31).wrapping_add(p.x) as u32;
    h ^ (h >> 16)
}

/// Iteration order of a `java.util.HashSet`/`HashMap` of distinct positions
/// inserted in `order`, with table size `cap` (power of two).
pub fn java_hash_order(order: &mut [BlockPos], cap: usize) {
    let mask = (cap - 1) as u32;
    // stable: equal buckets keep insertion order (linked bins)
    order.sort_by_key(|p| java_hash(*p) & mask);
}

/// Java `HashMap` table size after inserting `n` entries.
pub fn java_table_size(n: usize) -> usize {
    let mut cap = 16;
    while n > cap * 3 / 4 {
        cap *= 2;
    }
    cap
}

/// State left behind when a block is removed: water for waterlogged blocks.
pub fn fluid_legacy(st: u16) -> u16 {
    let ids = &t().ids;
    if has(st, P::Waterlogged) && on(st, P::Waterlogged) {
        ids.water_state
    } else if block(st) == ids.water {
        st
    } else {
        ids.air_state
    }
}

fn is_diode(st: u16) -> bool {
    matches!(kind(st), K::Repeater | K::Comparator)
}

fn is_rail(st: u16) -> bool {
    matches!(kind(st), K::Rail | K::PoweredRail | K::ActivatorRail | K::DetectorRail)
}

/// `FaceAttachedHorizontalDirectionalBlock.getConnectedDirection`.
pub fn connected_dir(st: u16) -> u8 {
    match get(st, P::Face) {
        FACE_CEILING => DOWN,
        FACE_FLOOR => UP,
        _ => get(st, P::Facing),
    }
}

// ------------------------------------------------------------------ ticks

pub fn schedule<L: Lvl>(rs: &mut Rs, l: &L, pos: BlockPos, blk: u16, delay: u32, pr: i8) {
    if !l.loaded(pos) {
        return;
    }
    if !rs.pending.insert((pos, blk)) {
        return;
    }
    let t = Tick { due: l.time() + delay as u64, prio: pr, sub: rs.sub, pos, block: blk };
    rs.sub += 1;
    rs.heap.push(Reverse(t));
}

pub fn has_scheduled(rs: &Rs, pos: BlockPos, blk: u16) -> bool {
    rs.pending.contains(&(pos, blk))
}

pub fn will_tick_this_tick(rs: &Rs, pos: BlockPos, blk: u16) -> bool {
    rs.run_set.contains(&(pos, blk))
}

/// `LevelTicks.tick` for blocks.
pub fn run_block_ticks<L: Lvl>(rs: &mut Rs, l: &mut L) {
    let now = l.time();
    while rs.run.len() < MAX_TICKS {
        match rs.heap.peek() {
            Some(Reverse(t)) if t.due <= now => {}
            _ => break,
        }
        let Reverse(t) = rs.heap.pop().unwrap();
        if !l.loaded(t.pos) {
            // stays scheduled (pending) until its chunk loads again
            rs.parked.entry((t.pos.x >> 4, t.pos.z >> 4)).or_default().push(t);
            continue;
        }
        rs.pending.remove(&(t.pos, t.block));
        rs.run_set.insert((t.pos, t.block));
        rs.run.push_back(t);
    }
    rs.handling_tick = true;
    while let Some(t) = rs.run.pop_front() {
        rs.run_set.remove(&(t.pos, t.block));
        if !l.loaded(t.pos) {
            continue;
        }
        let st = l.get(t.pos);
        if block(st) == t.block {
            tick_block(rs, l, st, t.pos);
        }
    }
    rs.handling_tick = false;
    rs.run_set.clear();
}

// ------------------------------------------------------------------ neighbor updater

fn add_and_run<L: Lvl>(rs: &mut Rs, l: &mut L, u: Upd) {
    let running = rs.count > 0;
    let too_many = rs.count >= MAX_CHAINED;
    rs.count += 1;
    if !too_many {
        if running {
            rs.added.push(u);
        } else {
            rs.stack.push(u);
        }
    } else if rs.count - 1 == MAX_CHAINED {
        tracing::error!("Too many chained neighbor updates. Skipping the rest.");
    }
    if !running {
        run_updates(rs, l);
    }
}

fn run_updates<L: Lvl>(rs: &mut Rs, l: &mut L) {
    while !rs.stack.is_empty() || !rs.added.is_empty() {
        while let Some(u) = rs.added.pop() {
            rs.stack.push(u);
        }
        loop {
            if !rs.added.is_empty() {
                break;
            }
            if !run_next(rs, l) {
                rs.stack.pop();
                break;
            }
        }
    }
    rs.count = 0;
}

/// Run the next step of the top entry. Returns true while it has more.
fn run_next<L: Lvl>(rs: &mut Rs, l: &mut L) -> bool {
    let Some(&top) = rs.stack.last() else { return false };
    match top {
        Upd::Multi { src, block: b, skip, idx } => {
            let d = UPDATE_ORDER[idx as usize];
            let mut next = idx + 1;
            if (next as usize) < 6 && UPDATE_ORDER[next as usize] == skip {
                next += 1;
            }
            if let Some(Upd::Multi { idx: i, .. }) = rs.stack.last_mut() {
                *i = next;
            }
            let np = rel(src, d);
            let st = l.get(np);
            neighbor_changed(rs, l, st, np, b, false);
            (next as usize) < 6
        }
        Upd::Simple { pos, block: b } => {
            let st = l.get(pos);
            neighbor_changed(rs, l, st, pos, b, false);
            false
        }
        Upd::Full { pos, state, block: b, piston } => {
            neighbor_changed(rs, l, state, pos, b, piston);
            false
        }
        Upd::Shape { dir, nstate, pos, npos, flags, limit } => {
            execute_shape(rs, l, dir, pos, npos, nstate, flags, limit);
            false
        }
    }
}

/// `Level.updateNeighborsAt`.
pub fn update_neighbors_at<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, blk: u16) {
    add_and_run(rs, l, Upd::Multi { src: pos, block: blk, skip: 6, idx: 0 });
}

/// `Level.updateNeighborsAtExceptFromFacing`.
pub fn update_neighbors_except<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, blk: u16, skip: u8) {
    let idx = if UPDATE_ORDER[0] == skip { 1 } else { 0 };
    add_and_run(rs, l, Upd::Multi { src: pos, block: blk, skip, idx });
}

/// `Level.neighborChanged(pos, block, orientation)`.
pub fn neighbor_changed_at<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, blk: u16) {
    add_and_run(rs, l, Upd::Simple { pos, block: blk });
}

/// `Level.neighborChanged(state, pos, block, orientation, movedByPiston)`.
pub fn neighbor_changed_full<L: Lvl>(rs: &mut Rs, l: &mut L, state: u16, pos: BlockPos, blk: u16, piston: bool) {
    add_and_run(rs, l, Upd::Full { pos, state, block: blk, piston });
}

/// `Level.neighborShapeChanged`: `dir` points from `pos` to `npos`.
pub fn neighbor_shape_changed<L: Lvl>(rs: &mut Rs, l: &mut L, dir: u8, pos: BlockPos, npos: BlockPos, nstate: u16, flags: u32, limit: i32) {
    add_and_run(rs, l, Upd::Shape { dir, nstate, pos, npos, flags, limit });
}

/// `NeighborUpdater.executeShapeUpdate`.
fn execute_shape<L: Lvl>(rs: &mut Rs, l: &mut L, dir: u8, pos: BlockPos, npos: BlockPos, nstate: u16, flags: u32, limit: i32) {
    let cur = l.get(pos);
    if flags & fl::SKIP_WIRE_SHAPE != 0 && block(cur) == t().ids.wire {
        return;
    }
    let new = update_shape(rs, l, cur, pos, dir, npos, nstate);
    update_or_destroy(rs, l, cur, new, pos, flags, limit);
}

/// `Block.updateOrDestroy`.
pub fn update_or_destroy<L: Lvl>(rs: &mut Rs, l: &mut L, cur: u16, new: u16, pos: BlockPos, flags: u32, limit: i32) {
    if new != cur {
        if is_air(new) {
            destroy_block(rs, l, pos, flags & fl::SUPPRESS_DROPS == 0, limit);
        } else {
            set_block(rs, l, pos, new, flags & !fl::SUPPRESS_DROPS, limit);
        }
    }
}

/// `BlockState.updateNeighbourShapes`.
pub fn neighbour_shapes<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, flags: u32, limit: i32) {
    for d in SHAPE_ORDER {
        neighbor_shape_changed(rs, l, opposite(d), rel(pos, d), pos, st, flags, limit);
    }
}

/// `BlockState.updateIndirectNeighbourShapes` (only the wire has one).
pub fn indirect_shapes<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, flags: u32, limit: i32) {
    if kind(st) != K::Wire {
        return;
    }
    let wire = t().ids.wire;
    for d in HORIZONTAL {
        let side = get(st, side_prop(d));
        if side != SIDE_NONE && block(l.get(rel(pos, d))) != wire {
            let down = rel(rel(pos, d), DOWN);
            if block(l.get(down)) == wire {
                let np = rel(down, opposite(d));
                let ns = l.get(np);
                neighbor_shape_changed(rs, l, opposite(d), down, np, ns, flags, limit);
            }
            let up = rel(rel(pos, d), UP);
            if block(l.get(up)) == wire {
                let np = rel(up, opposite(d));
                let ns = l.get(np);
                neighbor_shape_changed(rs, l, opposite(d), up, np, ns, flags, limit);
            }
        }
    }
}

// ------------------------------------------------------------------ setBlock

/// `Level.setBlock` with its side effects. Returns false when nothing changed.
pub fn set_block<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16, flags: u32, limit: i32) -> bool {
    if !l.loaded(pos) {
        return false;
    }
    let old = l.get(pos);
    if old == st {
        return false;
    }
    l.raw_set(pos, st);
    let (bo, bn) = (block(old), block(st));
    let changed = bo != bn;
    let piston = flags & fl::MOVE_BY_PISTON != 0;
    if changed {
        remove_block_entity(rs, pos, old);
    }
    if (changed || is_rail(st)) && (flags & fl::NEIGHBORS != 0 || piston) {
        after_removal(rs, l, old, pos, piston);
    }
    if block(l.get(pos)) != bn {
        return false;
    }
    if flags & fl::SKIP_ON_PLACE == 0 {
        on_place(rs, l, st, pos, old, piston);
    }
    post_set(rs, l, pos, old, st, flags, limit);
    true
}

/// `Level.notifyAndUpdatePhysics` minus the client update.
fn post_set<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, old: u16, st: u16, flags: u32, limit: i32) {
    if l.get(pos) != st {
        return;
    }
    if flags & fl::NEIGHBORS != 0 {
        update_neighbors_at(rs, l, pos, block(old));
        if has_analog(st) {
            update_comparators(rs, l, pos, block(st));
        }
    }
    if flags & fl::KNOWN_SHAPE == 0 && limit > 0 {
        let nf = flags & !(fl::SUPPRESS_DROPS | fl::NEIGHBORS);
        indirect_shapes(rs, l, old, pos, nf, limit - 1);
        neighbour_shapes(rs, l, st, pos, nf, limit - 1);
        indirect_shapes(rs, l, st, pos, nf, limit - 1);
    }
}

/// A block change made outside the engine (player, explosion, fluid,
/// command): run what vanilla runs after `setBlock(pos, new, UPDATE_ALL)`.
pub fn external_set<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, old: u16, new: u16) {
    let changed = block(old) != block(new);
    if changed {
        remove_block_entity(rs, pos, old);
        after_removal(rs, l, old, pos, false);
    } else if is_rail(new) {
        after_removal(rs, l, old, pos, false);
    }
    if l.get(pos) != new {
        return;
    }
    on_place(rs, l, new, pos, old, false);
    post_set(rs, l, pos, old, new, fl::ALL, UPDATE_LIMIT - 1);
}

/// `Level.removeBlock`.
pub fn remove_block<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, piston: bool) -> bool {
    let st = l.get(pos);
    set_block(rs, l, pos, fluid_legacy(st), fl::ALL | if piston { fl::MOVE_BY_PISTON } else { 0 }, UPDATE_LIMIT)
}

/// `Level.destroyBlock`: particles, drops, removal.
pub fn destroy_block<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, drops: bool, limit: i32) -> bool {
    let st = l.get(pos);
    if is_air(st) {
        return false;
    }
    l.level_event(2001, pos, st as i32);
    if drops {
        l.drop_block(pos, st);
    }
    set_block(rs, l, pos, fluid_legacy(st), fl::ALL, limit)
}

/// `Level.updateNeighbourForOutputSignal`.
pub fn update_comparators<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, blk: u16) {
    let cmp = t().ids.comparator;
    for d in HORIZONTAL {
        let rp = rel(pos, d);
        if !l.loaded(rp) {
            continue;
        }
        let s = l.get(rp);
        if block(s) == cmp {
            neighbor_changed_full(rs, l, s, rp, blk, false);
        } else if conductor(s) {
            let rp2 = rel(rp, d);
            let s2 = l.get(rp2);
            if block(s2) == cmp {
                neighbor_changed_full(rs, l, s2, rp2, blk, false);
            }
        }
    }
}

fn remove_block_entity(rs: &mut Rs, pos: BlockPos, old: u16) {
    match kind(old) {
        K::Comparator => {
            rs.comparators.remove(&pos);
        }
        K::MovingPiston => {
            rs.moving.remove(&pos);
        }
        K::Daylight => {
            rs.daylight.remove(&pos);
        }
        _ => {}
    }
}

/// `Block.dropResources`.
pub fn drop_resources<L: Lvl>(l: &mut L, st: u16, pos: BlockPos) {
    l.drop_block(pos, st);
}

// ------------------------------------------------------------------ signals

/// `SignalGetter.getSignal`: `dir` points from the asking block to `pos`.
pub fn signal<L: Lvl>(rs: &Rs, l: &L, pos: BlockPos, dir: u8) -> u8 {
    let st = l.get(pos);
    let s = block_signal(rs, l, st, pos, dir);
    if conductor(st) {
        s.max(direct_signal_to(rs, l, pos))
    } else {
        s
    }
}

pub fn has_signal<L: Lvl>(rs: &Rs, l: &L, pos: BlockPos, dir: u8) -> bool {
    signal(rs, l, pos, dir) > 0
}

/// `getDirectSignalTo`.
pub fn direct_signal_to<L: Lvl>(rs: &Rs, l: &L, pos: BlockPos) -> u8 {
    let mut r = 0;
    for d in ALL {
        r = r.max(direct_signal(rs, l, rel(pos, d), d));
        if r >= 15 {
            return r;
        }
    }
    r
}

pub fn direct_signal<L: Lvl>(rs: &Rs, l: &L, pos: BlockPos, dir: u8) -> u8 {
    let st = l.get(pos);
    block_direct_signal(rs, l, st, pos, dir)
}

/// `hasNeighborSignal`.
pub fn has_neighbor_signal<L: Lvl>(rs: &Rs, l: &L, pos: BlockPos) -> bool {
    ALL.iter().any(|&d| signal(rs, l, rel(pos, d), d) > 0)
}

/// `getBestNeighborSignal`.
pub fn best_neighbor_signal<L: Lvl>(rs: &Rs, l: &L, pos: BlockPos) -> u8 {
    let mut best = 0;
    for d in ALL {
        let s = signal(rs, l, rel(pos, d), d);
        if s >= 15 {
            return 15;
        }
        best = best.max(s);
    }
    best
}

/// `getControlInputSignal`.
fn control_input<L: Lvl>(rs: &Rs, l: &L, pos: BlockPos, dir: u8, only_diodes: bool) -> u8 {
    let st = l.get(pos);
    let ids = &t().ids;
    if only_diodes {
        if is_diode(st) {
            direct_signal(rs, l, pos, dir)
        } else {
            0
        }
    } else if block(st) == ids.redstone_block {
        15
    } else if block(st) == ids.wire {
        get(st, P::Power)
    } else if is_signal_source(rs, st) {
        direct_signal(rs, l, pos, dir)
    } else {
        0
    }
}

pub fn is_signal_source(rs: &Rs, st: u16) -> bool {
    match kind(st) {
        K::Wire => rs.wire_signals,
        K::Lever
        | K::Button(_)
        | K::Plate { .. }
        | K::Weighted(_)
        | K::Torch
        | K::WallTorch
        | K::Repeater
        | K::Comparator
        | K::Observer
        | K::RedstoneBlock
        | K::Daylight
        | K::Target
        | K::Lectern
        | K::TrappedChest
        | K::TripwireHook
        | K::DetectorRail => true,
        _ => false,
    }
}

fn plate_signal(st: u16) -> u8 {
    match kind(st) {
        K::Weighted(_) => get(st, P::Power),
        _ => {
            if on(st, P::Powered) {
                15
            } else {
                0
            }
        }
    }
}

fn comparator_out(rs: &Rs, pos: BlockPos) -> u8 {
    rs.comparators.get(&pos).copied().unwrap_or(0)
}

/// `BlockState.getSignal`.
pub fn block_signal<L: Lvl>(rs: &Rs, l: &L, st: u16, pos: BlockPos, dir: u8) -> u8 {
    let pw = |b: bool| if b { 15 } else { 0 };
    match kind(st) {
        K::Wire => {
            if !rs.wire_signals || dir == DOWN {
                return 0;
            }
            let p = get(st, P::Power);
            if p == 0 {
                0
            } else if dir == UP {
                p
            } else {
                let c = wire_connection_state(rs, l, st, pos);
                if get(c, side_prop(opposite(dir))) != SIDE_NONE {
                    p
                } else {
                    0
                }
            }
        }
        K::Lever | K::Button(_) | K::Lectern | K::TripwireHook | K::DetectorRail => pw(on(st, P::Powered)),
        K::Plate { .. } | K::Weighted(_) => plate_signal(st),
        K::Torch => pw(on(st, P::Lit) && dir != UP),
        K::WallTorch => pw(on(st, P::Lit) && get(st, P::Facing) != dir),
        K::Repeater | K::Observer => pw(on(st, P::Powered) && get(st, P::Facing) == dir),
        K::Comparator => {
            if on(st, P::Powered) && get(st, P::Facing) == dir {
                comparator_out(rs, pos)
            } else {
                0
            }
        }
        K::RedstoneBlock => 15,
        K::Daylight | K::Target => get(st, P::Power),
        K::TrappedChest => l.chest_viewers(pos).min(15),
        _ => 0,
    }
}

/// `BlockState.getDirectSignal`.
pub fn block_direct_signal<L: Lvl>(rs: &Rs, l: &L, st: u16, pos: BlockPos, dir: u8) -> u8 {
    let pw = |b: bool| if b { 15 } else { 0 };
    match kind(st) {
        K::Wire => {
            if rs.wire_signals {
                block_signal(rs, l, st, pos, dir)
            } else {
                0
            }
        }
        K::Lever | K::Button(_) => pw(on(st, P::Powered) && connected_dir(st) == dir),
        K::Plate { .. } | K::Weighted(_) => {
            if dir == UP {
                plate_signal(st)
            } else {
                0
            }
        }
        K::Torch | K::WallTorch => {
            if dir == DOWN {
                block_signal(rs, l, st, pos, dir)
            } else {
                0
            }
        }
        K::Repeater | K::Comparator | K::Observer => block_signal(rs, l, st, pos, dir),
        K::Lectern | K::DetectorRail => pw(dir == UP && on(st, P::Powered)),
        K::TrappedChest => {
            if dir == UP {
                l.chest_viewers(pos).min(15)
            } else {
                0
            }
        }
        K::TripwireHook => pw(on(st, P::Powered) && get(st, P::Facing) == dir),
        _ => 0,
    }
}

/// `shouldRedstoneWireConnectTo`; `dir` None for the up/down checks.
fn should_connect(rs: &Rs, st: u16, dir: Option<u8>) -> bool {
    match kind(st) {
        K::Wire => true,
        K::Repeater => dir.is_some_and(|d| axis(get(st, P::Facing)) == axis(d)),
        K::Observer => dir == Some(get(st, P::Facing)),
        _ => is_signal_source(rs, st) && dir.is_some(),
    }
}

/// `getAnalogOutputSignal`.
pub fn analog_output<L: Lvl>(l: &L, st: u16, pos: BlockPos) -> u8 {
    let b = &rine_data::get().blocks;
    let n = block_name(st);
    let num = |p: &str| b.get_prop(st, p).and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
    match n {
        "water_cauldron" | "powder_snow_cauldron" => num("level") as u8,
        "lava_cauldron" => 3,
        "cauldron" => 0,
        "composter" => num("level") as u8,
        "cake" => ((7 - num("bites")) * 2) as u8,
        "end_portal_frame" => {
            if b.get_prop(st, "eye") == Some("true") {
                15
            } else {
                0
            }
        }
        "respawn_anchor" => ((num("charges") as f32 / 4.0) * 15.0).floor() as u8,
        "beehive" | "bee_nest" => num("honey_level") as u8,
        _ if n.ends_with("copper_bulb") => {
            if on(l.get(pos), P::Lit) {
                15
            } else {
                0
            }
        }
        _ => l.container_signal(pos, st).unwrap_or(0),
    }
}

// ------------------------------------------------------------------ dispatch

/// `BlockState.handleNeighborChanged`.
pub fn neighbor_changed<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, from: u16, piston: bool) {
    match kind(st) {
        K::None => {
            if l.has_hooks(block(st)) {
                l.hook_neighbor(rs, pos, st);
            } else {
                crate::game::blocks::neighbor_changed(rs, l, st, pos, from);
            }
        }
        K::Wire => {
            if wire_can_survive(l, pos) {
                wire_update_power(rs, l, pos, st);
            } else {
                drop_resources(l, st, pos);
                remove_block(rs, l, pos, false);
            }
        }
        K::Repeater | K::Comparator => {
            if block(l.get(pos)) == block(st) {
                if diode_can_survive(l, pos) {
                    diode_check_tick(rs, l, pos, st);
                } else {
                    drop_resources(l, st, pos);
                    remove_block(rs, l, pos, false);
                    for d in ALL {
                        update_neighbors_at(rs, l, rel(pos, d), block(st));
                    }
                }
            }
        }
        K::Torch | K::WallTorch => {
            if on(st, P::Lit) == torch_has_signal(rs, l, pos, st) && !will_tick_this_tick(rs, pos, block(st)) {
                schedule(rs, l, pos, block(st), 2, prio::NORMAL);
            }
        }
        K::Lamp => {
            let lit = on(st, P::Lit);
            if lit != has_neighbor_signal(rs, l, pos) {
                if lit {
                    schedule(rs, l, pos, block(st), 4, prio::NORMAL);
                } else {
                    set_block(rs, l, pos, setb(st, P::Lit, true), fl::CLIENTS, UPDATE_LIMIT);
                }
            }
        }
        K::Door { .. } => {
            let other = if get(st, P::Half) == 0 { UP } else { DOWN };
            let sig = has_neighbor_signal(rs, l, pos) || has_neighbor_signal(rs, l, rel(pos, other));
            if from != block(st) && sig != on(st, P::Powered) {
                if sig != on(st, P::Open) {
                    door_sound(l, st, pos, sig, false);
                }
                let ns = setb(setb(st, P::Powered, sig), P::Open, sig);
                set_block(rs, l, pos, ns, fl::CLIENTS, UPDATE_LIMIT);
            }
        }
        K::Trapdoor { .. } => {
            let sig = has_neighbor_signal(rs, l, pos);
            if sig != on(st, P::Powered) {
                let mut ns = st;
                if on(st, P::Open) != sig {
                    ns = setb(ns, P::Open, sig);
                    door_sound(l, st, pos, sig, false);
                }
                set_block(rs, l, pos, setb(ns, P::Powered, sig), fl::CLIENTS, UPDATE_LIMIT);
            }
        }
        K::FenceGate => {
            let hp = has_neighbor_signal(rs, l, pos);
            if on(st, P::Powered) != hp {
                set_block(rs, l, pos, setb(setb(st, P::Powered, hp), P::Open, hp), fl::CLIENTS, UPDATE_LIMIT);
                if on(st, P::Open) != hp {
                    door_sound(l, st, pos, hp, false);
                }
            }
        }
        K::Tnt => {
            if has_neighbor_signal(rs, l, pos) {
                prime(l, pos);
                remove_block(rs, l, pos, false);
            }
        }
        K::Note => {
            let sig = has_neighbor_signal(rs, l, pos);
            if sig != on(st, P::Powered) {
                if sig {
                    play_note(rs, l, st, pos);
                }
                let cur = l.get(pos);
                set_block(rs, l, pos, setb(cur, P::Powered, sig), fl::ALL, UPDATE_LIMIT);
            }
        }
        K::CopperBulb => copper_bulb_flip(rs, l, st, pos),
        K::Bell => {
            let sig = has_neighbor_signal(rs, l, pos);
            if sig != on(st, P::Powered) {
                if sig {
                    ring_bell(rs, l, st, pos);
                }
                set_block(rs, l, pos, setb(st, P::Powered, sig), fl::ALL, UPDATE_LIMIT);
            }
        }
        K::Hopper => hopper_check(rs, l, st, pos),
        K::Dispenser | K::Dropper => {
            let sig = has_neighbor_signal(rs, l, pos) || has_neighbor_signal(rs, l, rel(pos, UP));
            let trig = on(st, P::Triggered);
            if sig && !trig {
                schedule(rs, l, pos, block(st), 4, prio::NORMAL);
                set_block(rs, l, pos, setb(st, P::Triggered, true), fl::CLIENTS | fl::INVISIBLE, UPDATE_LIMIT);
            } else if !sig && trig {
                set_block(rs, l, pos, setb(st, P::Triggered, false), fl::CLIENTS | fl::INVISIBLE, UPDATE_LIMIT);
            }
        }
        K::Piston { .. } => piston::check_if_extend(rs, l, pos, st),
        K::PistonHead => {
            if piston::head_can_survive(l, st, pos) {
                let base = rel(pos, opposite(get(st, P::Facing)));
                neighbor_changed_at(rs, l, base, from);
            }
        }
        K::PoweredRail | K::ActivatorRail | K::Rail | K::DetectorRail => {
            let _ = piston;
            super::rails::neighbor_changed(rs, l, st, pos, from);
        }
        K::Daylight => {
            rs.daylight.insert(pos);
        }
        _ => {}
    }
}

/// `BlockState.tick` (scheduled tick).
fn tick_block<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    match kind(st) {
        K::None => {
            if l.has_hooks(block(st)) {
                l.hook_tick(rs, pos, st);
            } else {
                crate::game::blocks::tick(rs, l, st, pos);
            }
        }
        K::Repeater => repeater_tick(rs, l, pos, st),
        K::Comparator => comparator_refresh(rs, l, pos, st),
        K::Torch | K::WallTorch => torch_tick(rs, l, pos, st),
        K::Lamp => {
            if on(st, P::Lit) && !has_neighbor_signal(rs, l, pos) {
                set_block(rs, l, pos, setb(st, P::Lit, false), fl::CLIENTS, UPDATE_LIMIT);
            }
        }
        K::Observer => {
            if on(st, P::Powered) {
                set_block(rs, l, pos, setb(st, P::Powered, false), fl::CLIENTS, UPDATE_LIMIT);
            } else {
                set_block(rs, l, pos, setb(st, P::Powered, true), fl::CLIENTS, UPDATE_LIMIT);
                schedule(rs, l, pos, block(st), 2, prio::NORMAL);
            }
            update_neighbors_in_front(rs, l, pos, st);
        }
        K::Button(_) => {
            if on(st, P::Powered) {
                // no arrows in the world: the button is released
                set_block(rs, l, pos, setb(st, P::Powered, false), fl::ALL, UPDATE_LIMIT);
                attached_update_neighbours(rs, l, pos, st);
                button_sound(l, st, pos, false);
            }
        }
        K::Plate { .. } | K::Weighted(_) => {
            let s = plate_signal(st);
            if s > 0 {
                plate_check(rs, l, pos, st, s);
            }
        }
        K::Target => {
            if get(st, P::Power) != 0 {
                set_block(rs, l, pos, set(st, P::Power, 0), fl::ALL, UPDATE_LIMIT);
            }
        }
        K::Dispenser | K::Dropper => l.dispense(pos, st),
        _ => {}
    }
}

/// `BlockState.onPlace`.
pub fn on_place<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, old: u16, _piston: bool) {
    let changed = block(old) != block(st);
    match kind(st) {
        K::Wire => {
            if changed {
                wire_update_power(rs, l, pos, st);
                for d in [UP, DOWN] {
                    update_neighbors_at(rs, l, rel(pos, d), block(st));
                }
                wire_update_neighbors_of_wires(rs, l, pos);
            }
        }
        K::Repeater | K::Comparator => {
            if changed && kind(st) == K::Comparator {
                rs.comparators.insert(pos, 0);
            }
            update_neighbors_in_front(rs, l, pos, st);
        }
        K::Torch | K::WallTorch => torch_notify(rs, l, pos, st),
        K::Observer => {
            if changed && on(st, P::Powered) && !has_scheduled(rs, pos, block(st)) {
                let ns = setb(st, P::Powered, false);
                set_block(rs, l, pos, ns, fl::CLIENTS | fl::KNOWN_SHAPE, UPDATE_LIMIT);
                update_neighbors_in_front(rs, l, pos, ns);
            }
        }
        K::Piston { .. } => {
            if changed && !rs.moving.contains_key(&pos) {
                piston::check_if_extend(rs, l, pos, st);
            }
        }
        K::Tnt => {
            if changed && has_neighbor_signal(rs, l, pos) {
                prime(l, pos);
                remove_block(rs, l, pos, false);
            }
        }
        K::Target => {
            if changed && get(st, P::Power) > 0 && !has_scheduled(rs, pos, block(st)) {
                set_block(rs, l, pos, set(st, P::Power, 0), fl::CLIENTS | fl::KNOWN_SHAPE, UPDATE_LIMIT);
            }
        }
        K::Hopper => {
            if changed {
                hopper_check(rs, l, st, pos);
            }
        }
        K::CopperBulb => {
            if changed {
                copper_bulb_flip(rs, l, st, pos);
            }
        }
        K::Daylight => {
            rs.daylight.insert(pos);
        }
        K::Rail | K::PoweredRail | K::ActivatorRail | K::DetectorRail => {
            if changed {
                super::rails::on_place(rs, l, st, pos);
            }
        }
        _ => crate::game::blocks::on_place(rs, l, st, pos, old),
    }
}

/// `BlockState.affectNeighborsAfterRemoval`.
pub fn after_removal<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, piston: bool) {
    match kind(st) {
        K::Wire => {
            if !piston {
                for d in ALL {
                    update_neighbors_at(rs, l, rel(pos, d), block(st));
                }
                wire_update_power(rs, l, pos, st);
                wire_update_neighbors_of_wires(rs, l, pos);
            }
        }
        K::Lever | K::Button(_) => {
            if !piston && on(st, P::Powered) {
                attached_update_neighbours(rs, l, pos, st);
            }
        }
        K::Plate { .. } | K::Weighted(_) => {
            if !piston && plate_signal(st) > 0 {
                plate_update_neighbours(rs, l, pos, st);
            }
        }
        K::Torch | K::WallTorch => {
            if !piston {
                torch_notify(rs, l, pos, st);
            }
        }
        K::Repeater | K::Comparator => {
            if !piston {
                update_neighbors_in_front(rs, l, pos, st);
            }
        }
        K::Observer => {
            if on(st, P::Powered) && has_scheduled(rs, pos, block(st)) {
                update_neighbors_in_front(rs, l, pos, setb(st, P::Powered, false));
            }
        }
        K::PistonHead => {
            let base = rel(pos, opposite(get(st, P::Facing)));
            if piston::fitting_base(st, l.get(base)) {
                destroy_block(rs, l, base, true, UPDATE_LIMIT);
            }
        }
        K::Rail | K::PoweredRail | K::ActivatorRail | K::DetectorRail => super::rails::after_removal(rs, l, st, pos, piston),
        _ => {
            if has_analog(st) {
                update_comparators(rs, l, pos, block(st));
            }
        }
    }
}

/// `BlockState.updateShape`: the state `st` at `pos` after its neighbor at
/// `npos` (side `dir`) became `nst`. Air means the block breaks.
pub fn update_shape<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, dir: u8, npos: BlockPos, nst: u16) -> u16 {
    let air = t().ids.air_state;
    let s = update_shape_redstone(rs, l, st, pos, dir, nst, air);
    if s != st || is_air(s) {
        return s;
    }
    crate::game::blocks::update_shape(rs, l, st, pos, dir, npos, nst)
}

fn update_shape_redstone<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, dir: u8, nst: u16, air: u16) -> u16 {
    match kind(st) {
        K::Wire => wire_update_shape(rs, l, st, pos, dir, nst),
        K::Repeater => {
            if dir == DOWN && !sturdy_rigid(nst, UP) {
                air
            } else if axis(dir) != axis(get(st, P::Facing)) {
                setb(st, P::Locked, repeater_locked(rs, l, pos, st))
            } else {
                st
            }
        }
        K::Comparator => {
            if dir == DOWN && !sturdy_rigid(nst, UP) {
                air
            } else {
                st
            }
        }
        K::Torch => {
            if dir == DOWN && !torch_can_survive(l, st, pos) {
                air
            } else {
                st
            }
        }
        K::WallTorch => {
            if opposite(dir) == get(st, P::Facing) && !torch_can_survive(l, st, pos) {
                air
            } else {
                st
            }
        }
        K::Lever | K::Button(_) => {
            if opposite(connected_dir(st)) == dir && !attached_can_survive(l, st, pos) {
                air
            } else {
                st
            }
        }
        K::Plate { .. } | K::Weighted(_) => {
            if dir == DOWN && !plate_can_survive(l, pos) {
                air
            } else {
                st
            }
        }
        K::Observer => {
            if get(st, P::Facing) == dir && !on(st, P::Powered) && !has_scheduled(rs, pos, block(st)) {
                schedule(rs, l, pos, block(st), 2, prio::NORMAL);
            }
            st
        }
        K::Door { .. } => {
            let lower = get(st, P::Half) == 0;
            if axis(dir) != 0 || lower != (dir == UP) {
                if lower && dir == DOWN && !sturdy(l.get(rel(pos, DOWN)), UP) {
                    air
                } else {
                    st
                }
            } else if matches!(kind(nst), K::Door { .. }) && get(nst, P::Half) != get(st, P::Half) {
                set(nst, P::Half, get(st, P::Half))
            } else {
                air
            }
        }
        K::PistonHead => {
            if opposite(dir) == get(st, P::Facing) && !piston::head_can_survive(l, st, pos) {
                air
            } else {
                st
            }
        }
        K::Note => {
            if axis(dir) == 0 {
                note_with_instrument(l, st, pos)
            } else {
                st
            }
        }
        _ => st,
    }
}

/// `Block.updateFromNeighbourShapes`.
pub fn update_from_neighbour_shapes<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) -> u16 {
    let mut s = st;
    for d in SHAPE_ORDER {
        let np = rel(pos, d);
        let ns = l.get(np);
        s = update_shape(rs, l, s, pos, d, np, ns);
    }
    s
}

// ------------------------------------------------------------------ wire

pub fn side_prop(d: u8) -> P {
    match d {
        NORTH => P::North,
        EAST => P::East,
        SOUTH => P::South,
        _ => P::West,
    }
}

fn wire_can_survive_on(st: u16) -> bool {
    sturdy(st, UP) || block(st) == t().ids.hopper
}

fn wire_can_survive<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    wire_can_survive_on(l.get(rel(pos, DOWN)))
}

fn is_cross(st: u16) -> bool {
    HORIZONTAL.iter().all(|&d| get(st, side_prop(d)) != SIDE_NONE)
}

fn is_dot(st: u16) -> bool {
    HORIZONTAL.iter().all(|&d| get(st, side_prop(d)) == SIDE_NONE)
}

fn cross_state(power: u8) -> u16 {
    let mut s = set(t().ids.wire_default, P::Power, power);
    for d in HORIZONTAL {
        s = set(s, side_prop(d), SIDE_SIDE);
    }
    s
}

fn should_connect_at<L: Lvl>(rs: &Rs, l: &L, pos: BlockPos) -> bool {
    should_connect(rs, l.get(pos), None)
}

fn wire_connecting_side<L: Lvl>(rs: &Rs, l: &L, pos: BlockPos, d: u8, can_up: bool) -> u8 {
    let rp = rel(pos, d);
    let rst = l.get(rp);
    if can_up {
        let placeable_above = matches!(kind(rst), K::Trapdoor { .. }) || wire_can_survive_on(rst);
        if placeable_above && should_connect_at(rs, l, rel(rp, UP)) {
            return if sturdy(rst, opposite(d)) { SIDE_UP } else { SIDE_SIDE };
        }
    }
    if !should_connect(rs, rst, Some(d)) && (conductor(rst) || !should_connect_at(rs, l, rel(rp, DOWN))) {
        SIDE_NONE
    } else {
        SIDE_SIDE
    }
}

fn wire_missing_connections<L: Lvl>(rs: &Rs, l: &L, mut s: u16, pos: BlockPos) -> u16 {
    let can_up = !conductor(l.get(rel(pos, UP)));
    for d in HORIZONTAL {
        if get(s, side_prop(d)) == SIDE_NONE {
            s = set(s, side_prop(d), wire_connecting_side(rs, l, pos, d, can_up));
        }
    }
    s
}

/// `RedstoneWireBlock.getConnectionState`.
pub fn wire_connection_state<L: Lvl>(rs: &Rs, l: &L, st: u16, pos: BlockPos) -> u16 {
    let was_dot = is_dot(st);
    let mut s = wire_missing_connections(rs, l, set(t().ids.wire_default, P::Power, get(st, P::Power)), pos);
    if was_dot && is_dot(s) {
        return s;
    }
    let n = get(s, P::North) != SIDE_NONE;
    let so = get(s, P::South) != SIDE_NONE;
    let e = get(s, P::East) != SIDE_NONE;
    let w = get(s, P::West) != SIDE_NONE;
    let ns_empty = !n && !so;
    let ew_empty = !e && !w;
    if !w && ns_empty {
        s = set(s, P::West, SIDE_SIDE);
    }
    if !e && ns_empty {
        s = set(s, P::East, SIDE_SIDE);
    }
    if !n && ew_empty {
        s = set(s, P::North, SIDE_SIDE);
    }
    if !so && ew_empty {
        s = set(s, P::South, SIDE_SIDE);
    }
    s
}

/// Placement state of a wire (`getStateForPlacement`).
pub fn wire_placement<L: Lvl>(rs: &Rs, l: &L, pos: BlockPos) -> u16 {
    wire_connection_state(rs, l, cross_state(0), pos)
}

fn wire_update_shape<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, dir: u8, nst: u16) -> u16 {
    if dir == DOWN {
        return if wire_can_survive_on(nst) { st } else { t().ids.air_state };
    }
    if dir == UP {
        return wire_connection_state(rs, l, st, pos);
    }
    let can_up = !conductor(l.get(rel(pos, UP)));
    let side = wire_connecting_side(rs, l, pos, dir, can_up);
    let cur = get(st, side_prop(dir));
    if (side != SIDE_NONE) == (cur != SIDE_NONE) && !is_cross(st) {
        set(st, side_prop(dir), side)
    } else {
        let s = set(cross_state(get(st, P::Power)), side_prop(dir), side);
        wire_connection_state(rs, l, s, pos)
    }
}

/// `DefaultRedstoneWireEvaluator.calculateTargetStrength`.
fn wire_target<L: Lvl>(rs: &mut Rs, l: &L, pos: BlockPos) -> u8 {
    rs.wire_signals = false;
    let bs = best_neighbor_signal(rs, l, pos);
    rs.wire_signals = true;
    if bs == 15 {
        return 15;
    }
    let wire = t().ids.wire;
    let wp = |s: u16| if block(s) == wire { get(s, P::Power) } else { 0 };
    let mut w = 0;
    let above_conducts = conductor(l.get(rel(pos, UP)));
    for d in HORIZONTAL {
        let np = rel(pos, d);
        let ns = l.get(np);
        w = w.max(wp(ns));
        if conductor(ns) && !above_conducts {
            w = w.max(wp(l.get(rel(np, UP))));
        } else if !conductor(ns) {
            w = w.max(wp(l.get(rel(np, DOWN))));
        }
    }
    bs.max(w.saturating_sub(1))
}

/// `DefaultRedstoneWireEvaluator.updatePowerStrength`.
fn wire_update_power<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16) {
    let target = wire_target(rs, l, pos);
    if get(st, P::Power) == target {
        return;
    }
    if l.get(pos) == st {
        set_block(rs, l, pos, set(st, P::Power, target), fl::CLIENTS, UPDATE_LIMIT);
    }
    // `Sets.newHashSet()` of pos and its six neighbors, iterated in hash order
    let mut set7 = [pos, rel(pos, DOWN), rel(pos, UP), rel(pos, NORTH), rel(pos, SOUTH), rel(pos, WEST), rel(pos, EAST)];
    java_hash_order(&mut set7, 16);
    let wire = t().ids.wire;
    for p in set7 {
        update_neighbors_at(rs, l, p, wire);
    }
}

fn wire_check_corner<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos) {
    let wire = t().ids.wire;
    if block(l.get(pos)) == wire {
        update_neighbors_at(rs, l, pos, wire);
        for d in ALL {
            update_neighbors_at(rs, l, rel(pos, d), wire);
        }
    }
}

fn wire_update_neighbors_of_wires<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos) {
    for d in HORIZONTAL {
        wire_check_corner(rs, l, rel(pos, d));
    }
    for d in HORIZONTAL {
        let tp = rel(pos, d);
        if conductor(l.get(tp)) {
            wire_check_corner(rs, l, rel(tp, UP));
        } else {
            wire_check_corner(rs, l, rel(tp, DOWN));
        }
    }
}

/// `RedstoneWireBlock.useWithoutItem`: toggle a dot and a cross.
fn wire_use<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) -> bool {
    if !(is_cross(st) || is_dot(st)) {
        return false;
    }
    let base = if is_cross(st) { t().ids.wire_default } else { cross_state(0) };
    let ns = wire_connection_state(rs, l, set(base, P::Power, get(st, P::Power)), pos);
    if ns == st {
        return false;
    }
    set_block(rs, l, pos, ns, fl::ALL, UPDATE_LIMIT);
    for d in HORIZONTAL {
        let rp = rel(pos, d);
        if (get(st, side_prop(d)) != SIDE_NONE) != (get(ns, side_prop(d)) != SIDE_NONE) && conductor(l.get(rp)) {
            update_neighbors_except(rs, l, rp, block(ns), opposite(d));
        }
    }
    true
}

// ------------------------------------------------------------------ diodes

fn diode_can_survive<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    sturdy_rigid(l.get(rel(pos, DOWN)), UP)
}

fn diode_input<L: Lvl>(rs: &Rs, l: &L, pos: BlockPos, st: u16) -> u8 {
    let f = get(st, P::Facing);
    let tp = rel(pos, f);
    let input = signal(rs, l, tp, f);
    if input >= 15 {
        return input;
    }
    let ts = l.get(tp);
    input.max(if block(ts) == t().ids.wire { get(ts, P::Power) } else { 0 })
}

fn comparator_input<L: Lvl>(rs: &Rs, l: &L, pos: BlockPos, st: u16) -> u8 {
    let mut r = diode_input(rs, l, pos, st);
    let f = get(st, P::Facing);
    let tp = rel(pos, f);
    let ts = l.get(tp);
    if has_analog(ts) {
        r = analog_output(l, ts, tp);
    } else if r < 15 && conductor(ts) {
        let tp2 = rel(tp, f);
        let ts2 = l.get(tp2);
        if has_analog(ts2) {
            r = analog_output(l, ts2, tp2);
        }
    }
    r
}

fn alternate_signal<L: Lvl>(rs: &Rs, l: &L, pos: BlockPos, st: u16, only_diodes: bool) -> u8 {
    let f = get(st, P::Facing);
    let cw = clockwise(f);
    let ccw = counter_clockwise(f);
    control_input(rs, l, rel(pos, cw), cw, only_diodes).max(control_input(rs, l, rel(pos, ccw), ccw, only_diodes))
}

pub fn repeater_locked<L: Lvl>(rs: &Rs, l: &L, pos: BlockPos, st: u16) -> bool {
    alternate_signal(rs, l, pos, st, true) > 0
}

fn diode_should_turn_on<L: Lvl>(rs: &Rs, l: &L, pos: BlockPos, st: u16) -> bool {
    match kind(st) {
        K::Comparator => {
            let input = comparator_input(rs, l, pos, st);
            if input == 0 {
                return false;
            }
            let side = alternate_signal(rs, l, pos, st, false);
            input > side || (input == side && get(st, P::Mode) == 0)
        }
        _ => diode_input(rs, l, pos, st) > 0,
    }
}

fn comparator_calc<L: Lvl>(rs: &Rs, l: &L, pos: BlockPos, st: u16) -> u8 {
    let input = comparator_input(rs, l, pos, st);
    if input == 0 {
        return 0;
    }
    let alt = alternate_signal(rs, l, pos, st, false);
    if alt > input {
        0
    } else if get(st, P::Mode) == 1 {
        input - alt
    } else {
        input
    }
}

fn diode_delay(st: u16) -> u32 {
    match kind(st) {
        K::Repeater => get(st, P::Delay) as u32 * 2,
        _ => 2,
    }
}

fn diode_prioritize<L: Lvl>(l: &L, pos: BlockPos, st: u16) -> bool {
    let d = opposite(get(st, P::Facing));
    let os = l.get(rel(pos, d));
    is_diode(os) && get(os, P::Facing) != d
}

fn diode_check_tick<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16) {
    let b = block(st);
    match kind(st) {
        K::Comparator => {
            if !will_tick_this_tick(rs, pos, b) {
                let out = comparator_calc(rs, l, pos, st);
                let old = comparator_out(rs, pos);
                if out != old || on(st, P::Powered) != diode_should_turn_on(rs, l, pos, st) {
                    let pr = if diode_prioritize(l, pos, st) { prio::HIGH } else { prio::NORMAL };
                    schedule(rs, l, pos, b, 2, pr);
                }
            }
        }
        _ => {
            if !repeater_locked(rs, l, pos, st) {
                let on_ = on(st, P::Powered);
                let sto = diode_should_turn_on(rs, l, pos, st);
                if on_ != sto && !will_tick_this_tick(rs, pos, b) {
                    let pr = if diode_prioritize(l, pos, st) {
                        prio::EXTREMELY_HIGH
                    } else if on_ {
                        prio::VERY_HIGH
                    } else {
                        prio::HIGH
                    };
                    schedule(rs, l, pos, b, diode_delay(st), pr);
                }
            }
        }
    }
}

fn repeater_tick<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16) {
    if repeater_locked(rs, l, pos, st) {
        return;
    }
    let on_ = on(st, P::Powered);
    let sto = diode_should_turn_on(rs, l, pos, st);
    if on_ && !sto {
        set_block(rs, l, pos, setb(st, P::Powered, false), fl::CLIENTS, UPDATE_LIMIT);
    } else if !on_ {
        set_block(rs, l, pos, setb(st, P::Powered, true), fl::CLIENTS, UPDATE_LIMIT);
        if !sto {
            schedule(rs, l, pos, block(st), diode_delay(st), prio::VERY_HIGH);
        }
    }
}

fn comparator_refresh<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16) {
    let out = comparator_calc(rs, l, pos, st);
    let old = rs.comparators.insert(pos, out).unwrap_or(0);
    if old != out || get(st, P::Mode) == 0 {
        let sto = diode_should_turn_on(rs, l, pos, st);
        let on_ = on(st, P::Powered);
        if on_ && !sto {
            set_block(rs, l, pos, setb(st, P::Powered, false), fl::CLIENTS, UPDATE_LIMIT);
        } else if !on_ && sto {
            set_block(rs, l, pos, setb(st, P::Powered, true), fl::CLIENTS, UPDATE_LIMIT);
        }
        update_neighbors_in_front(rs, l, pos, st);
    }
}

/// `DiodeBlock.updateNeighborsInFront` (also the observer's).
fn update_neighbors_in_front<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16) {
    let f = get(st, P::Facing);
    let op = rel(pos, opposite(f));
    neighbor_changed_at(rs, l, op, block(st));
    update_neighbors_except(rs, l, op, block(st), f);
}

// ------------------------------------------------------------------ torches

fn torch_has_signal<L: Lvl>(rs: &Rs, l: &L, pos: BlockPos, st: u16) -> bool {
    match kind(st) {
        K::WallTorch => {
            let o = opposite(get(st, P::Facing));
            has_signal(rs, l, rel(pos, o), o)
        }
        _ => has_signal(rs, l, rel(pos, DOWN), DOWN),
    }
}

fn torch_can_survive<L: Lvl>(l: &L, st: u16, pos: BlockPos) -> bool {
    match kind(st) {
        K::WallTorch => {
            let f = get(st, P::Facing);
            sturdy(l.get(rel(pos, opposite(f))), f)
        }
        _ => sturdy_center(l.get(rel(pos, DOWN)), UP),
    }
}

fn torch_notify<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16) {
    for d in ALL {
        update_neighbors_at(rs, l, rel(pos, d), block(st));
    }
}

fn torch_toggled_too_often(rs: &mut Rs, pos: BlockPos, now: u64, add: bool) -> bool {
    if add {
        rs.torch_toggles.push_back((pos, now));
    }
    rs.torch_toggles.iter().filter(|(p, _)| *p == pos).take(8).count() >= 8
}

fn torch_tick<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16) {
    let sig = torch_has_signal(rs, l, pos, st);
    let now = l.time();
    while let Some(&(_, when)) = rs.torch_toggles.front() {
        if now - when > 60 {
            rs.torch_toggles.pop_front();
        } else {
            break;
        }
    }
    if on(st, P::Lit) {
        if sig {
            set_block(rs, l, pos, setb(st, P::Lit, false), fl::ALL, UPDATE_LIMIT);
            if torch_toggled_too_often(rs, pos, now, true) {
                l.level_event(1502, pos, 0);
                let b = block(l.get(pos));
                schedule(rs, l, pos, b, 160, prio::NORMAL);
            }
        }
    } else if !sig && !torch_toggled_too_often(rs, pos, now, false) {
        set_block(rs, l, pos, setb(st, P::Lit, true), fl::ALL, UPDATE_LIMIT);
    }
}

// ------------------------------------------------------------------ levers, buttons, plates

fn attached_can_survive<L: Lvl>(l: &L, st: u16, pos: BlockPos) -> bool {
    let cd = connected_dir(st);
    sturdy(l.get(rel(pos, opposite(cd))), cd)
}

fn attached_update_neighbours<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16) {
    let front = opposite(connected_dir(st));
    update_neighbors_at(rs, l, pos, block(st));
    update_neighbors_at(rs, l, rel(pos, front), block(st));
}

/// `LeverBlock.pull`.
pub fn lever_pull<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16) {
    let ns = setb(st, P::Powered, !on(st, P::Powered));
    set_block(rs, l, pos, ns, fl::ALL, UPDATE_LIMIT);
    attached_update_neighbours(rs, l, pos, ns);
    l.sound("block.lever.click", pos, 0.3, if on(ns, P::Powered) { 0.6 } else { 0.5 }, CAT_BLOCKS, false);
}

/// `ButtonBlock.press`.
pub fn button_press<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16) {
    let K::Button(ticks) = kind(st) else { return };
    set_block(rs, l, pos, setb(st, P::Powered, true), fl::ALL, UPDATE_LIMIT);
    attached_update_neighbours(rs, l, pos, st);
    schedule(rs, l, pos, block(st), ticks as u32, prio::NORMAL);
    button_sound(l, st, pos, true);
}

fn set_prefix(st: u16) -> &'static str {
    match t().set[block(st) as usize] {
        Set::Wooden => "wooden",
        Set::Cherry => "cherry_wood",
        Set::Bamboo => "bamboo_wood",
        Set::Nether => "nether_wood",
        Set::Iron => "iron",
        Set::Copper => "copper",
        Set::Stone => "stone",
        Set::Gold => "metal",
    }
}

fn button_sound<L: Lvl>(l: &mut L, st: u16, pos: BlockPos, pressed: bool) {
    let name = format!("block.{}_button.click_{}", set_prefix(st), if pressed { "on" } else { "off" });
    l.sound(&name, pos, 1.0, 1.0, CAT_BLOCKS, pressed);
}

fn door_sound<L: Lvl>(l: &mut L, st: u16, pos: BlockPos, open: bool, skip_actor: bool) {
    let what = match kind(st) {
        K::Door { .. } => "door",
        K::Trapdoor { .. } => "trapdoor",
        _ => "fence_gate",
    };
    let name = if what == "fence_gate" && t().set[block(st) as usize] == Set::Wooden {
        format!("block.fence_gate.{}", if open { "open" } else { "close" })
    } else {
        format!("block.{}_{}.{}", set_prefix(st), what, if open { "open" } else { "close" })
    };
    let pitch = l.rand() * 0.1 + 0.9;
    l.sound(&name, pos, 1.0, pitch, CAT_BLOCKS, skip_actor);
}

fn plate_can_survive<L: Lvl>(l: &L, pos: BlockPos) -> bool {
    let below = l.get(rel(pos, DOWN));
    sturdy_rigid(below, UP) || sturdy_center(below, UP)
}

fn plate_update_neighbours<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16) {
    update_neighbors_at(rs, l, pos, block(st));
    update_neighbors_at(rs, l, rel(pos, DOWN), block(st));
}

fn plate_strength<L: Lvl>(l: &L, pos: BlockPos, st: u16) -> u8 {
    let min = [pos.x as f64 + 1.0 / 16.0, pos.y as f64, pos.z as f64 + 1.0 / 16.0];
    let max = [pos.x as f64 + 15.0 / 16.0, pos.y as f64 + 0.25, pos.z as f64 + 15.0 / 16.0];
    match kind(st) {
        K::Weighted(maxw) => {
            let n = l.count_entities(min, max, false);
            if n > 0 {
                let pct = n.min(maxw as usize) as f32 / maxw as f32;
                (pct * 15.0).ceil() as u8
            } else {
                0
            }
        }
        K::Plate { mobs } => {
            if l.count_entities(min, max, mobs) > 0 {
                15
            } else {
                0
            }
        }
        _ => 0,
    }
}

fn plate_check<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16, old: u8) {
    let sig = plate_strength(l, pos, st);
    if old != sig {
        let ns = match kind(st) {
            K::Weighted(_) => set(st, P::Power, sig),
            _ => setb(st, P::Powered, sig > 0),
        };
        set_block(rs, l, pos, ns, fl::CLIENTS, UPDATE_LIMIT);
        plate_update_neighbours(rs, l, pos, st);
    }
    let (was, is) = (old > 0, sig > 0);
    if was != is {
        let name = format!("block.{}_pressure_plate.click_{}", set_prefix(st), if is { "on" } else { "off" });
        l.sound(&name, pos, 1.0, 1.0, CAT_BLOCKS, false);
    }
    if is {
        let delay = if matches!(kind(st), K::Weighted(_)) { 10 } else { 20 };
        schedule(rs, l, pos, block(st), delay, prio::NORMAL);
    }
}

/// `BasePressurePlateBlock.entityInside`.
pub fn plate_entity_inside<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16) {
    if plate_signal(st) == 0 {
        plate_check(rs, l, pos, st, 0);
    }
}

// ------------------------------------------------------------------ misc blocks

fn prime<L: Lvl>(l: &mut L, pos: BlockPos) {
    l.prime_tnt(pos);
}

fn copper_bulb_flip<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    let sig = has_neighbor_signal(rs, l, pos);
    if sig != on(st, P::Powered) {
        let mut ns = st;
        if !on(st, P::Powered) {
            ns = setb(ns, P::Lit, !on(ns, P::Lit));
            let name = if on(ns, P::Lit) { "block.copper_bulb.turn_on" } else { "block.copper_bulb.turn_off" };
            l.sound(name, pos, 1.0, 1.0, CAT_BLOCKS, false);
        }
        set_block(rs, l, pos, setb(ns, P::Powered, sig), fl::ALL, UPDATE_LIMIT);
    }
}

fn hopper_check<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    let should = !has_neighbor_signal(rs, l, pos);
    if should != on(st, P::Enabled) {
        set_block(rs, l, pos, setb(st, P::Enabled, should), fl::CLIENTS, UPDATE_LIMIT);
    }
}

fn ring_bell<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    let dir = get(st, P::Facing);
    queue_block_event(rs, pos, block(st), 1, dir);
    l.sound("block.bell.use", pos, 2.0, 1.0, CAT_BLOCKS, false);
}

/// Instrument names by `NoteBlockInstrument` (block below / mob head above).
fn instrument_below(st: u16) -> &'static str {
    let n = block_name(st);
    if n.ends_with("_log")
        || n.ends_with("_planks")
        || n.ends_with("_wood")
        || n.ends_with("_stem")
        || n.ends_with("_hyphae")
        || n.ends_with("_fence")
        || n.ends_with("_fence_gate")
        || n.ends_with("_door")
        || n.ends_with("_trapdoor")
        || n.ends_with("_slab") && n.contains("oak")
        || matches!(
            n,
            "note_block"
                | "jukebox"
                | "bookshelf"
                | "chest"
                | "trapped_chest"
                | "crafting_table"
                | "barrel"
                | "lectern"
                | "composter"
                | "beehive"
                | "bee_nest"
                | "loom"
                | "cartography_table"
                | "fletching_table"
                | "smithing_table"
                | "chiseled_bookshelf"
                | "daylight_detector"
        )
    {
        return "bass";
    }
    if n == "sand" || n == "red_sand" || n == "gravel" || n.ends_with("concrete_powder") || n.starts_with("suspicious_") {
        return "snare";
    }
    if n == "glass" || n.ends_with("stained_glass") || n.ends_with("glass_pane") || n == "tinted_glass" || matches!(n, "sea_lantern" | "beacon") {
        return "hat";
    }
    match n {
        "gold_block" => return "bell",
        "clay" | "infested_stone" => return "flute",
        "packed_ice" => return "chime",
        "bone_block" => return "xylophone",
        "iron_block" => return "iron_xylophone",
        "soul_sand" => return "cow_bell",
        "pumpkin" | "carved_pumpkin" | "jack_o_lantern" => return "didgeridoo",
        "emerald_block" => return "bit",
        "hay_block" => return "banjo",
        "glowstone" => return "pling",
        _ => {}
    }
    if n.ends_with("_wool") {
        return "guitar";
    }
    let full = rine_data::get().blocks.has(st, rine_data::blocks::flag::TOOL_REQUIRED);
    if full
        || n.contains("stone")
        || n.contains("brick")
        || n.ends_with("_ore")
        || n.contains("deepslate")
        || n.contains("terracotta")
        || n.contains("quartz")
        || matches!(n, "netherrack" | "obsidian" | "crying_obsidian" | "bedrock" | "basalt" | "end_stone" | "prismarine" | "magma_block" | "respawn_anchor")
    {
        return "basedrum";
    }
    "harp"
}

/// Mob heads set a non-tunable instrument from above.
fn instrument_above(st: u16) -> Option<&'static str> {
    Some(match block_name(st) {
        "zombie_head" => "zombie",
        "skeleton_skull" => "skeleton",
        "creeper_head" => "creeper",
        "dragon_head" => "dragon",
        "wither_skeleton_skull" => "wither_skeleton",
        "piglin_head" => "piglin",
        "player_head" => "custom_head",
        _ => return None,
    })
}

fn note_with_instrument<L: Lvl>(l: &L, st: u16, pos: BlockPos) -> u16 {
    let ins = instrument_above(l.get(rel(pos, UP))).unwrap_or_else(|| instrument_below(l.get(rel(pos, DOWN))));
    rine_data::get().blocks.with_prop(st, "instrument", ins).unwrap_or(st)
}

fn instrument_name(st: u16) -> &'static str {
    rine_data::get().blocks.get_prop(st, "instrument").unwrap_or("harp")
}

fn play_note<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    let ins = instrument_name(st);
    let above_head = matches!(ins, "zombie" | "skeleton" | "creeper" | "dragon" | "wither_skeleton" | "piglin" | "custom_head");
    if above_head || is_air(l.get(rel(pos, UP))) {
        queue_block_event(rs, pos, block(st), 0, 0);
    }
}

/// `NoteBlock.triggerEvent`.
fn note_event<L: Lvl>(l: &mut L, st: u16, pos: BlockPos) -> bool {
    let ins = instrument_name(st);
    if ins == "custom_head" {
        return false;
    }
    let tunable = !matches!(ins, "zombie" | "skeleton" | "creeper" | "dragon" | "wither_skeleton" | "piglin");
    let pitch = if tunable { 2f32.powf((get(st, P::Note) as f32 - 12.0) / 12.0) } else { 1.0 };
    let name = format!("block.note_block.{ins}");
    l.sound(&name, pos, 3.0, pitch, CAT_RECORDS, false);
    true
}

/// Daylight detector update (`DaylightDetectorBlock.updateSignalStrength`).
pub fn daylight_update<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos) {
    let day = l.day_time();
    let tod = time_of_day(day);
    let sun = tod * std::f32::consts::TAU;
    let darken = {
        let mut h = 1.0 - ((sun.cos() * 2.0) + 0.5);
        h = h.clamp(0.0, 1.0);
        (h * 11.0) as i32
    };
    let mut target = l.sky_light(pos) as i32 - darken;
    if on(st, P::Inverted) {
        target = 15 - target;
    } else if target > 0 {
        let offset = if sun < std::f32::consts::PI { 0.0 } else { std::f32::consts::TAU };
        let a = sun + (offset - sun) * 0.2;
        target = (target as f32 * a.cos()).round() as i32;
    }
    let target = target.clamp(0, 15) as u8;
    if get(st, P::Power) != target {
        set_block(rs, l, pos, set(st, P::Power, target), fl::ALL, UPDATE_LIMIT);
    }
}

/// `DimensionType.timeOfDay` for the overworld.
fn time_of_day(day: i64) -> f32 {
    let d = (day.rem_euclid(24000)) as f64 / 24000.0 - 0.25;
    let f = d - d.floor();
    let g = 0.5 - (f * std::f64::consts::PI).cos() / 2.0;
    ((f * 2.0 + g) / 3.0) as f32
}

// ------------------------------------------------------------------ block events

pub fn queue_block_event(rs: &mut Rs, pos: BlockPos, blk: u16, a: u8, b: u8) {
    let e = (pos, blk, a, b);
    if rs.bev_set.insert(e) {
        rs.block_events.push_back(e);
    }
}

/// `ServerLevel.runBlockEvents`.
pub fn run_block_events<L: Lvl>(rs: &mut Rs, l: &mut L) {
    let mut later = Vec::new();
    while let Some(e) = rs.block_events.pop_front() {
        rs.bev_set.remove(&e);
        let (pos, blk, a, b) = e;
        if !l.loaded(pos) {
            later.push(e);
            continue;
        }
        let st = l.get(pos);
        if block(st) == blk && trigger_event(rs, l, st, pos, a, b) {
            l.block_event_packet(pos, blk, a, b);
        }
    }
    for e in later {
        if rs.bev_set.insert(e) {
            rs.block_events.push_back(e);
        }
    }
}

fn trigger_event<L: Lvl>(rs: &mut Rs, l: &mut L, st: u16, pos: BlockPos, a: u8, b: u8) -> bool {
    match kind(st) {
        K::Piston { .. } => piston::trigger_event(rs, l, st, pos, a, b),
        K::Note => note_event(l, st, pos),
        K::Bell => true,
        _ => false,
    }
}

// ------------------------------------------------------------------ player use

/// `useWithoutItem` of the redstone blocks. `look` is the player's horizontal
/// direction (fence gates). Returns true when handled.
pub fn use_block<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos, st: u16, look: u8) -> bool {
    match kind(st) {
        K::Lever => {
            lever_pull(rs, l, pos, st);
            true
        }
        K::Button(_) => {
            if !on(st, P::Powered) {
                button_press(rs, l, pos, st);
            }
            true
        }
        K::Repeater => {
            let d = get(st, P::Delay) % 4 + 1;
            set_block(rs, l, pos, set(st, P::Delay, d), fl::ALL, UPDATE_LIMIT);
            true
        }
        K::Comparator => {
            let ns = set(st, P::Mode, 1 - get(st, P::Mode));
            let pitch = if get(ns, P::Mode) == 1 { 0.55 } else { 0.5 };
            l.sound("block.comparator.click", pos, 0.3, pitch, CAT_BLOCKS, true);
            set_block(rs, l, pos, ns, fl::CLIENTS, UPDATE_LIMIT);
            if block(l.get(pos)) == block(ns) {
                let cur = l.get(pos);
                comparator_refresh(rs, l, pos, cur);
            }
            true
        }
        K::Wire => wire_use(rs, l, st, pos),
        K::Daylight => {
            let ns = setb(st, P::Inverted, !on(st, P::Inverted));
            set_block(rs, l, pos, ns, fl::CLIENTS, UPDATE_LIMIT);
            daylight_update(rs, l, ns, pos);
            rs.daylight.insert(pos);
            true
        }
        K::Note => {
            let ns = set(st, P::Note, (get(st, P::Note) + 1) % 25);
            set_block(rs, l, pos, ns, fl::ALL, UPDATE_LIMIT);
            play_note(rs, l, ns, pos);
            true
        }
        K::Door { iron: false } => {
            let ns = setb(st, P::Open, !on(st, P::Open));
            set_block(rs, l, pos, ns, fl::CLIENTS | fl::IMMEDIATE, UPDATE_LIMIT);
            door_sound(l, st, pos, on(ns, P::Open), true);
            true
        }
        K::Trapdoor { iron: false } => {
            let ns = setb(st, P::Open, !on(st, P::Open));
            set_block(rs, l, pos, ns, fl::CLIENTS, UPDATE_LIMIT);
            door_sound(l, st, pos, on(ns, P::Open), true);
            true
        }
        K::FenceGate => {
            let ns = if on(st, P::Open) {
                setb(st, P::Open, false)
            } else {
                let mut s = st;
                if get(s, P::Facing) == opposite(look) {
                    s = set(s, P::Facing, look);
                }
                setb(s, P::Open, true)
            };
            set_block(rs, l, pos, ns, fl::CLIENTS | fl::IMMEDIATE, UPDATE_LIMIT);
            door_sound(l, st, pos, on(ns, P::Open), true);
            true
        }
        K::Door { iron: true } | K::Trapdoor { iron: true } => false,
        _ => false,
    }
}

/// `setPlacedBy` effects of a block a player just placed.
pub fn placed_by<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos) {
    let st = l.get(pos);
    match kind(st) {
        K::Repeater | K::Comparator => {
            if diode_should_turn_on(rs, l, pos, st) {
                schedule(rs, l, pos, block(st), 1, prio::NORMAL);
            }
        }
        K::Piston { .. } => piston::check_if_extend(rs, l, pos, st),
        _ => {}
    }
}

/// `getStateForPlacement` corrections of redstone blocks: wire connections,
/// repeater lock, lamp/door/trapdoor/gate power, note block instrument.
pub fn placement_state<L: Lvl>(rs: &Rs, l: &L, pos: BlockPos, st: u16) -> u16 {
    match kind(st) {
        K::Wire => wire_placement(rs, l, pos),
        K::Repeater => setb(st, P::Locked, repeater_locked(rs, l, pos, st)),
        K::Lamp => setb(st, P::Lit, has_neighbor_signal(rs, l, pos)),
        K::Door { .. } => {
            let p = has_neighbor_signal(rs, l, pos) || has_neighbor_signal(rs, l, rel(pos, UP));
            setb(setb(st, P::Powered, p), P::Open, p)
        }
        K::Trapdoor { .. } | K::FenceGate => {
            if has_neighbor_signal(rs, l, pos) {
                setb(setb(st, P::Open, true), P::Powered, true)
            } else {
                st
            }
        }
        K::Note => note_with_instrument(l, st, pos),
        _ => st,
    }
}

/// Players and other entities standing in pressure plates (vanilla
/// `entityInside` from the entity movement pass).
pub fn entity_inside<L: Lvl>(rs: &mut Rs, l: &mut L, pos: BlockPos) {
    let st = l.get(pos);
    if matches!(kind(st), K::Plate { .. } | K::Weighted(_)) {
        plate_entity_inside(rs, l, pos, st);
    }
}

/// Moving piston and daylight detector block entities (`tickBlockEntities`).
pub fn tick_block_entities<L: Lvl>(rs: &mut Rs, l: &mut L) {
    piston::tick_movers(rs, l);
    if l.time() % 20 == 0 && !rs.daylight.is_empty() {
        let mut list: Vec<BlockPos> = rs.daylight.iter().copied().collect();
        list.sort_by_key(|p| (p.x, p.z, p.y));
        for p in list {
            if !l.loaded(p) {
                continue;
            }
            let st = l.get(p);
            if kind(st) == K::Daylight {
                daylight_update(rs, l, st, p);
            } else {
                rs.daylight.remove(&p);
            }
        }
    }
}
