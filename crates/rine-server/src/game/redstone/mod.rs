//! Vanilla redstone: signals, wire, sources, diodes, observers, pistons,
//! doors and the scheduled block ticks and neighbor updates that drive them,
//! in vanilla order (`engine`).
//!
//! Block changes made elsewhere (players, explosions, fluids, commands) go
//! through `World::set_block`, which queues an `RsEvent`; `process` runs the
//! vanilla consequences of each (`onPlace`, `affectNeighborsAfterRemoval`,
//! neighbor and shape updates). The engine's own writes are not queued: it
//! runs their side effects immediately, as vanilla does.
//!
//! Interface for other modules (command blocks):
//! - `has_neighbor_signal(g, pos)`: vanilla `Level.hasNeighborSignal`.
//! - `register_hooks("command_block", BlockHooks { neighbor_changed, tick })`:
//!   called from vanilla `neighborChanged` and from scheduled ticks.
//! - `schedule_tick(g, pos, delay, priority)`: vanilla `scheduleTick`.

pub mod data;
pub mod engine;
pub mod piston;
pub mod rails;
#[cfg(test)]
pub(crate) mod game_tests;
#[cfg(test)]
mod tests;

use super::{entity, Game};
use crate::out;
use data::{block, kind, K};
pub use engine::Rs;
use engine::{fl, Lvl};
use parking_lot::RwLock;
use rine_protocol::BlockPos;
use rine_world::{ChunkPos, MAX_Y, MIN_Y};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A block change waiting for its redstone side effects.
#[derive(Clone, Copy, Debug)]
pub enum RsEvent {
    /// Block at pos changed from old to new outside the engine.
    Set(BlockPos, u16, u16),
    /// A player placed the block at pos (`setPlacedBy`).
    Placed(BlockPos),
    /// Container contents changed (`updateNeighbourForOutputSignal`).
    Analog(BlockPos),
    /// Number of players looking into a chest changed (trapped chest).
    Viewers(BlockPos),
}

pub type Hook = fn(&mut Game, BlockPos, u16);

/// Callbacks for blocks the engine does not know (command blocks).
#[derive(Clone, Copy, Default)]
pub struct BlockHooks {
    /// Vanilla `neighborChanged(state, level, pos, ...)`.
    pub neighbor_changed: Option<Hook>,
    /// A scheduled tick of this block came due (vanilla `tick`).
    pub tick: Option<Hook>,
}

static HOOKS: RwLock<Vec<(u16, BlockHooks)>> = RwLock::new(Vec::new());
static HOOK_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Register hooks for a block by name ("command_block" or
/// "minecraft:command_block"). Returns false for an unknown block.
pub fn register_hooks(name: &str, hooks: BlockHooks) -> bool {
    let Some(b) = rine_data::get().blocks.block(name) else { return false };
    let id = b.id as u16;
    let mut h = HOOKS.write();
    h.retain(|(b, _)| *b != id);
    h.push((id, hooks));
    HOOK_COUNT.store(h.len(), Ordering::Relaxed);
    true
}

fn hooks_of(block: u16) -> Option<BlockHooks> {
    if HOOK_COUNT.load(Ordering::Relaxed) == 0 {
        return None;
    }
    HOOKS.read().iter().find(|(b, _)| *b == block).map(|(_, h)| *h)
}

// ------------------------------------------------------------------ public API

/// Vanilla `Level.hasNeighborSignal(pos)`.
pub fn has_neighbor_signal(g: &Game, pos: BlockPos) -> bool {
    engine::has_neighbor_signal(&g.world.redstone, &View { g }, pos)
}

/// Vanilla `Level.getBestNeighborSignal(pos)`.
pub fn best_neighbor_signal(g: &Game, pos: BlockPos) -> u8 {
    engine::best_neighbor_signal(&g.world.redstone, &View { g }, pos)
}

/// Vanilla `Level.getSignal(pos, direction)`; `dir` is a protocol direction
/// id pointing from the asking block to `pos`.
pub fn signal(g: &Game, pos: BlockPos, dir: u8) -> u8 {
    engine::signal(&g.world.redstone, &View { g }, pos, dir)
}

/// Vanilla `scheduleTick(pos, block, delay, priority)` for the block now at
/// `pos`; priority -3 (extremely high) ..= 3 (extremely low).
pub fn schedule_tick(g: &mut Game, pos: BlockPos, delay: u32, priority: i8) {
    let st = g.world.get_block(pos.x, pos.y, pos.z);
    let v = View { g: &*g };
    let (loaded, time) = (v.loaded(pos), v.time());
    if loaded {
        g.world.redstone.schedule_at(pos, block(st), time + delay as u64, priority);
    }
}

/// Vanilla `getBlockTicks().hasScheduledTick(pos, block)` for the block now at `pos`.
pub fn has_scheduled_tick(g: &Game, pos: BlockPos) -> bool {
    let st = g.world.get_block(pos.x, pos.y, pos.z);
    engine::has_scheduled(&g.world.redstone, pos, block(st))
}

/// Set a block with vanilla `UPDATE_ALL` side effects, run right away.
pub fn set_block(g: &mut Game, pos: BlockPos, st: u16) {
    with_engine(g, None, |rs, l| {
        engine::set_block(rs, l, pos, st, fl::ALL, engine::UPDATE_LIMIT);
    });
}

/// Right click on a redstone block (lever, button, repeater, comparator,
/// wire, daylight detector, note block, wooden door/trapdoor, fence gate).
pub fn use_block(g: &mut Game, slot: usize, pos: BlockPos, st: u16) -> bool {
    if kind(st) == K::None {
        return false;
    }
    let look = match g.player(slot) {
        Some(p) => match super::place::horizontal_dir(p.yaw) {
            0 => data::SOUTH,
            1 => data::WEST,
            2 => data::NORTH,
            _ => data::EAST,
        },
        None => data::NORTH,
    };
    process(g);
    let mut done = false;
    with_engine(g, Some(slot), |rs, l| done = engine::use_block(rs, l, pos, st, look));
    done
}

/// Which step of a right click on a block (see `blocks::interact`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UseStep {
    /// `useItemOn`, then `useWithoutItem` on `TryEmptyHand` (main hand).
    Block,
    /// `ItemStack.useOn`.
    Item,
}

/// Run one step of a right click on the block at `pos` through the block
/// families. Effects on the player come back in the result.
pub fn blocks_use(g: &mut Game, slot: usize, pos: BlockPos, ctx: &crate::game::blocks::interact::UseCtx, step: UseStep) -> crate::game::blocks::interact::Res {
    use crate::game::blocks::interact::{self as bi, Res};
    process(g);
    let seed = g.blocks.seed ^ g.tick.wrapping_mul(0x2545_F491_4F6C_DD1D);
    let mut rng = crate::game::blocks::Rng::derive(seed, pos.x as i64, ((pos.y as i64) << 32) ^ pos.z as i64);
    let mut res = Res::Pass;
    with_engine(g, Some(slot), |rs, l| {
        let st = l.get(pos);
        res = match step {
            UseStep::Block => match bi::use_item_on(rs, l, st, pos, ctx, &mut rng) {
                Res::TryEmptyHand if ctx.main_hand => bi::use_without_item(rs, l, st, pos, ctx, &mut rng),
                Res::TryEmptyHand => Res::Pass,
                r => r,
            },
            UseStep::Item => bi::item_use_on(rs, l, st, pos, ctx, &mut rng),
        };
    });
    res
}

/// Placement corrections (`getStateForPlacement`) for a block about to be
/// placed by a player.
pub fn placement_state(g: &Game, pos: BlockPos, st: u16) -> u16 {
    if kind(st) == K::None {
        return st;
    }
    engine::placement_state(&g.world.redstone, &View { g }, pos, st)
}

/// Placement corrections of the block behaviour families
/// (`blocks::placement`): connections, shapes, persistence. None when the
/// block cannot be placed there.
pub fn blocks_placement(g: &Game, pos: BlockPos, st: u16, ctx: &crate::game::blocks::PlaceCtx) -> Option<u16> {
    crate::game::blocks::placement(&View { g }, st, pos, ctx)
}

/// A player placed the block at `pos`; call after the `set_block`.
pub fn placed(g: &mut Game, pos: BlockPos) {
    g.world.rs_events.push(RsEvent::Placed(pos));
}

/// A container's contents changed (comparators update).
pub fn container_changed(w: &mut super::world::World, pos: BlockPos) {
    w.rs_events.push(RsEvent::Analog(pos));
}

/// The number of players looking into a chest changed.
pub fn viewers_changed(w: &mut super::world::World, pos: BlockPos) {
    w.rs_events.push(RsEvent::Viewers(pos));
}

/// Run the side effects of queued block changes. Called before the block
/// changes of a pass are sent.
pub fn process(g: &mut Game) {
    for _ in 0..64 {
        if g.world.rs_events.is_empty() {
            return;
        }
        let mut evs = std::mem::take(&mut g.world.rs_events);
        with_engine(g, None, |rs, l| {
            for e in evs.iter().copied() {
                match e {
                    RsEvent::Set(p, old, new) => engine::external_set(rs, l, p, old, new),
                    RsEvent::Placed(p) => engine::placed_by(rs, l, p),
                    RsEvent::Analog(p) => {
                        let st = l.get(p);
                        engine::update_comparators(rs, l, p, block(st));
                    }
                    RsEvent::Viewers(p) => {
                        let st = l.get(p);
                        if kind(st) == K::TrappedChest {
                            engine::update_neighbors_at(rs, l, p, block(st));
                            engine::update_neighbors_at(rs, l, p.offset(0, -1, 0), block(st));
                        }
                    }
                }
            }
        });
        evs.clear();
        if g.world.rs_events.is_empty() {
            g.world.rs_events = evs;
        }
    }
}

/// Scheduled block ticks (`blockTicks.tick`). Runs before the fluid ticks.
///
/// Changes queued earlier in this game tick (player packets) are processed
/// first with the previous tick's time, as vanilla handles packets between
/// ticks; then the level time advances (`tickTime`).
pub fn tick_scheduled(g: &mut Game) {
    process(g);
    g.world.rs_time = g.tick;
    chunks_loaded(g);
    with_engine(g, None, |rs, l| engine::run_block_ticks(rs, l));
}

/// Positions in a freshly generated or loaded chunk whose redstone block
/// entity data must be rebuilt: daylight detectors (ticker) and comparators
/// (stored output). Runs on the loading thread.
pub fn load_scan(c: &rine_world::Chunk) -> Vec<BlockPos> {
    let tb = data::t();
    let wanted = |st: u16| matches!(tb.kind[block(st) as usize], K::Daylight | K::Comparator);
    let mut out = Vec::new();
    let at = |y0: i32, i: usize| BlockPos::new(c.pos.x * 16 + (i & 15) as i32, y0 + (i >> 8) as i32, c.pos.z * 16 + ((i >> 4) & 15) as i32);
    for (si, sec) in c.sections.iter().enumerate() {
        if sec.non_air == 0 {
            continue;
        }
        let y0 = MIN_Y + (si as i32) * 16;
        match &sec.blocks {
            None if wanted(sec.single) => out.extend((0..4096).map(|i| at(y0, i))),
            None => {}
            Some(arr) => {
                for (i, &st) in arr.iter().enumerate() {
                    if wanted(st) {
                        out.push(at(y0, i));
                    }
                }
            }
        }
    }
    out
}

/// Rebuild block entity data for redstone blocks of chunks loaded since the
/// last call: daylight detectors start ticking, comparators recompute their
/// output on the next tick.
fn chunks_loaded(g: &mut Game) {
    if g.world.rs_loaded.is_empty() {
        return;
    }
    let list = std::mem::take(&mut g.world.rs_loaded);
    with_engine(g, None, |rs, l| {
        for &p in &list {
            let st = l.get(p);
            match kind(st) {
                K::Daylight => {
                    rs.daylight.insert(p);
                }
                K::Comparator => {
                    if !rs.comparators.contains_key(&p) {
                        rs.comparators.insert(p, if data::on(st, data::P::Powered) { 15 } else { 0 });
                        engine::schedule(rs, l, p, block(st), 1, engine::prio::NORMAL);
                    }
                }
                _ => {}
            }
        }
    });
}

/// Run random ticks for sampled positions (see `blocks::random`), in order.
/// A position is skipped when its block changed since it was sampled
/// (vanilla reads the state at the moment of the tick).
pub fn random_ticks(g: &mut Game, hits: &[crate::game::blocks::random::Hit], rng: &mut crate::game::blocks::Rng) {
    use crate::game::blocks::{self, random::Hit};
    let lava = rine_data::blocks::flag::LAVA;
    with_engine(g, None, |rs, l| {
        for h in hits {
            match *h {
                Hit::Block(p, sampled) => {
                    let st = l.get(p);
                    if st != sampled {
                        // changed earlier this pass: vanilla would tick what is there now
                        if !rine_data::get().blocks.has(st, rine_data::blocks::flag::RANDOM_TICK) {
                            continue;
                        }
                    }
                    blocks::random_tick(rs, l, st, p, rng);
                    // `tickChunk` then ticks the fluid of the state it read
                    // (lava blocks are ticked twice in total: MC-224294)
                    if rine_data::get().blocks.has(st, lava) {
                        blocks::fluid_random_tick(rs, l, st, p, rng);
                    }
                }
                Hit::Precip(x, z) => blocks::precipitation(rs, l, x, z, rng),
            }
        }
    });
}

/// Block events (pistons, note blocks), entities on pressure plates and
/// the block entity tickers (moving pistons, daylight detectors).
pub fn tick_late(g: &mut Game) {
    process(g);
    let inside = entities_on_plates(g);
    with_engine(g, None, |rs, l| {
        engine::run_block_events(rs, l);
        for p in inside {
            engine::entity_inside(rs, l, p);
        }
        engine::tick_block_entities(rs, l);
    });
}

/// Plate positions touched by players and entities this tick.
fn entities_on_plates(g: &Game) -> Vec<BlockPos> {
    let mut out = Vec::new();
    let mut visit = |min: [f64; 3], max: [f64; 3]| {
        let e = 1e-7;
        for x in (min[0] + e).floor() as i32..=(max[0] - e).floor() as i32 {
            for y in (min[1] + e).floor() as i32..=(max[1] - e).floor() as i32 {
                for z in (min[2] + e).floor() as i32..=(max[2] - e).floor() as i32 {
                    let st = g.world.get_block(x, y, z);
                    if matches!(kind(st), K::Plate { .. } | K::Weighted(_)) {
                        let p = BlockPos::new(x, y, z);
                        if !out.contains(&p) {
                            out.push(p);
                        }
                    }
                }
            }
        }
    };
    for p in g.players.iter().flatten() {
        if p.gamemode == 3 {
            continue;
        }
        let (min, max) = player_box(p);
        visit(min, max);
    }
    for e in &g.entities.list {
        if e.removed {
            continue;
        }
        let (min, max) = entity_box(e);
        visit(min, max);
    }
    out
}

fn player_box(p: &super::player::Player) -> ([f64; 3], [f64; 3]) {
    let h = if p.sneaking { 1.5 } else { 1.8 };
    ([p.pos[0] - 0.3, p.pos[1], p.pos[2] - 0.3], [p.pos[0] + 0.3, p.pos[1] + h, p.pos[2] + 0.3])
}

fn entity_box(e: &entity::Ent) -> ([f64; 3], [f64; 3]) {
    let (w, h) = match e.kind {
        entity::Kind::Item { .. } => (0.25, 0.25),
        _ => (0.98, 0.98),
    };
    ([e.pos[0] - w / 2.0, e.pos[1], e.pos[2] - w / 2.0], [e.pos[0] + w / 2.0, e.pos[1] + h, e.pos[2] + w / 2.0])
}

fn intersects(a: ([f64; 3], [f64; 3]), min: [f64; 3], max: [f64; 3]) -> bool {
    a.0[0] < max[0] && a.1[0] > min[0] && a.0[1] < max[1] && a.1[1] > min[1] && a.0[2] < max[2] && a.1[2] > min[2]
}

// ------------------------------------------------------------------ Lvl for the game

/// Run engine code against the game. The engine state is taken out of the
/// world for the call and put back in the hooks.
fn with_engine(g: &mut Game, actor: Option<usize>, f: impl FnOnce(&mut Rs, &mut GameLvl)) {
    let mut rs = std::mem::take(&mut g.world.redstone);
    {
        let seed = g.tick.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut l = GameLvl { g, actor, rng: seed };
        f(&mut rs, &mut l);
    }
    // anything scheduled from hooks while the state was out
    let inner = std::mem::replace(&mut g.world.redstone, rs);
    g.world.redstone.absorb(inner);
}

struct GameLvl<'a> {
    g: &'a mut Game,
    actor: Option<usize>,
    rng: u64,
}

/// Sound event id by short name, cached so repeated sounds do not allocate.
fn sound_id(name: &str) -> Option<u32> {
    static CACHE: RwLock<Option<ahash::AHashMap<String, Option<u32>>>> = RwLock::new(None);
    if let Some(m) = CACHE.read().as_ref() {
        if let Some(id) = m.get(name) {
            return *id;
        }
    }
    let id = rine_data::get().registries.id("minecraft:sound_event", &format!("minecraft:{name}"));
    CACHE.write().get_or_insert_with(Default::default).insert(name.to_string(), id);
    id
}

fn chunk_of(p: BlockPos) -> ChunkPos {
    ChunkPos::new(p.x >> 4, p.z >> 4)
}

fn read_sky(g: &Game, p: BlockPos) -> u8 {
    if p.y > MAX_Y {
        return 15;
    }
    if p.y < MIN_Y {
        return 0;
    }
    let Some(slot) = g.world.chunks.get(&chunk_of(p)) else { return 0 };
    let c = &slot.chunk;
    let si = ((p.y - MIN_Y) >> 4) as usize;
    let Some(sec) = c.sections.get(si) else { return 0 };
    let (lx, ly, lz) = ((p.x & 15) as usize, (p.y & 15) as usize, (p.z & 15) as usize);
    match &sec.sky {
        Some(arr) => {
            let i = rine_world::chunk::idx(lx, ly, lz);
            (arr[i >> 1] >> ((i & 1) * 4)) & 15
        }
        None => {
            let h = c.motion_blocking[lx + lz * 16] as i32 + MIN_Y - 1;
            if p.y >= h {
                15
            } else {
                0
            }
        }
    }
}

fn read_block_light(g: &Game, p: BlockPos) -> u8 {
    if !(MIN_Y..=MAX_Y).contains(&p.y) {
        return 0;
    }
    let Some(slot) = g.world.chunks.get(&chunk_of(p)) else { return 0 };
    let si = ((p.y - MIN_Y) >> 4) as usize;
    let Some(sec) = slot.chunk.sections.get(si) else { return 0 };
    match &sec.block_light {
        Some(arr) => {
            let i = rine_world::chunk::idx((p.x & 15) as usize, (p.y & 15) as usize, (p.z & 15) as usize);
            (arr[i >> 1] >> ((i & 1) * 4)) & 15
        }
        None => 0,
    }
}

fn count_in(g: &Game, min: [f64; 3], max: [f64; 3], living_only: bool) -> usize {
    let mut n = g.players.iter().flatten().filter(|p| p.gamemode != 3 && intersects(player_box(p), min, max)).count();
    if !living_only {
        n += g.entities.list.iter().filter(|e| !e.removed && intersects(entity_box(e), min, max)).count();
    }
    n
}

impl Lvl for GameLvl<'_> {
    fn get(&self, p: BlockPos) -> u16 {
        self.g.world.get_block(p.x, p.y, p.z)
    }

    fn loaded(&self, p: BlockPos) -> bool {
        (MIN_Y..=MAX_Y).contains(&p.y) && self.g.world.is_loaded(p.x, p.z)
    }

    fn raw_set(&mut self, p: BlockPos, st: u16) -> u16 {
        let w = &mut self.g.world;
        w.rs_mute = true;
        let old = w.set_block(p, st);
        w.rs_mute = false;
        old
    }

    fn time(&self) -> u64 {
        self.g.world.rs_time
    }

    fn day_time(&self) -> i64 {
        self.g.day_time
    }

    fn sky_light(&self, p: BlockPos) -> u8 {
        read_sky(self.g, p)
    }

    fn level_event(&mut self, ev: i32, p: BlockPos, data: i32) {
        let f = out::frame(out::level_event(ev, p, data));
        self.g.send_chunk_viewers(chunk_of(p), &f);
    }

    fn sound(&mut self, name: &str, p: BlockPos, vol: f32, pitch: f32, cat: i32, skip_actor: bool) {
        let Some(id) = sound_id(name) else { return };
        self.rng = self.rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let f = out::frame(out::sound(id, cat, p.x as f64 + 0.5, p.y as f64 + 0.5, p.z as f64 + 0.5, vol, pitch, self.rng as i64));
        let skip = if skip_actor { self.actor } else { None };
        self.g.send_chunk_viewers_except(chunk_of(p), &f, skip);
    }

    fn block_event_packet(&mut self, p: BlockPos, blk: u16, a: u8, b: u8) {
        use rine_protocol::Writer;
        let f = out::frame(rine_protocol::frame::packet(rine_protocol::ids::play::C_BLOCK_EVENT, |w| {
            w.w_pos(p);
            w.w_u8(a);
            w.w_u8(b);
            w.w_varint(blk as i32);
        }));
        self.g.send_chunk_viewers(chunk_of(p), &f);
    }

    fn drop_block(&mut self, p: BlockPos, st: u16) {
        for d in super::loot::block_drops(st, 0) {
            let off = entity::rand_f64((p.x as u64) ^ ((p.z as u64) << 20) ^ ((p.y as u64) << 40) ^ self.g.tick);
            entity::spawn_item(
                self.g,
                [p.x as f64 + 0.25 + off * 0.5, p.y as f64 + 0.25, p.z as f64 + 0.25 + (1.0 - off) * 0.5],
                [(off - 0.5) * 0.2, 0.2, (0.5 - off) * 0.2],
                d,
                10,
            );
        }
    }

    fn prime_tnt(&mut self, p: BlockPos) {
        entity::spawn_tnt(self.g, [p.x as f64 + 0.5, p.y as f64, p.z as f64 + 0.5], 80);
        self.sound("entity.tnt.primed", p, 1.0, 1.0, engine::CAT_BLOCKS, false);
    }

    fn count_entities(&self, min: [f64; 3], max: [f64; 3], living_only: bool) -> usize {
        count_in(self.g, min, max, living_only)
    }

    fn container_signal(&self, p: BlockPos, st: u16) -> Option<u8> {
        super::containers::analog_signal(p, st)
    }

    fn chest_viewers(&self, p: BlockPos) -> u8 {
        super::containers::viewers_at_pos(p)
    }

    fn dispense(&mut self, p: BlockPos, st: u16) {
        super::blockinv::dispense(self.g, p, st);
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    fn has_hooks(&self, blk: u16) -> bool {
        hooks_of(blk).is_some()
    }

    fn block_light(&self, p: BlockPos) -> u8 {
        read_block_light(self.g, p)
    }

    fn raining(&self) -> bool {
        self.g.cs.world.weather.raining
    }

    fn sky_darken(&self) -> u8 {
        let w = &self.g.cs.world.weather;
        crate::game::blocks::sky_darken(self.g.day_time, w.rain_level, w.thunder_level)
    }

    fn thundering(&self) -> bool {
        self.g.cs.world.weather.thundering
    }

    fn biome(&self, p: BlockPos) -> u8 {
        match self.g.world.chunks.get(&chunk_of(p)) {
            Some(s) => s.chunk.biome_at(p.x, p.y, p.z),
            None => 0,
        }
    }

    fn temperature(&self, biome: u8, p: BlockPos) -> f32 {
        self.g.world.generator.temperature(biome, p.x, p.y, p.z)
    }

    fn has_precipitation(&self, biome: u8) -> bool {
        self.g.world.generator.has_precipitation(biome)
    }

    fn height_motion_blocking(&self, x: i32, z: i32) -> i32 {
        match self.g.world.chunks.get(&ChunkPos::new(x >> 4, z >> 4)) {
            Some(s) => s.chunk.motion_blocking[(x & 15) as usize + (z & 15) as usize * 16] as i32 + MIN_Y,
            None => MIN_Y,
        }
    }

    fn rules(&self) -> Option<&crate::game::cmd::rules::GameRules> {
        Some(&self.g.rules)
    }

    fn pop_resource(&mut self, p: BlockPos, item: u32, count: i32) {
        // `Block.popResource`: random offset of +-0.25, random small velocity
        let r = |s: u64| entity::rand_f64(s);
        let k = (p.x as u64) ^ ((p.z as u64) << 21) ^ ((p.y as u64) << 42) ^ self.g.tick.wrapping_mul(31) ^ item as u64;
        let (ox, oy, oz) = (r(k) - 0.5, r(k ^ 1) - 0.5, r(k ^ 2) - 0.5);
        entity::spawn_item(
            self.g,
            [p.x as f64 + 0.5 + ox * 0.5, p.y as f64 + 0.5 + oy * 0.5 - 0.125, p.z as f64 + 0.5 + oz * 0.5],
            [(r(k ^ 3) - 0.5) * 0.2, 0.2, (r(k ^ 4) - 0.5) * 0.2],
            crate::game::player::Stack::new(item, count),
            10,
        );
    }

    fn game(&mut self) -> Option<&mut Game> {
        Some(self.g)
    }

    fn hook_neighbor(&mut self, rs: &mut Rs, p: BlockPos, st: u16) -> bool {
        let Some(f) = hooks_of(block(st)).and_then(|h| h.neighbor_changed) else { return false };
        std::mem::swap(rs, &mut self.g.world.redstone);
        f(self.g, p, st);
        std::mem::swap(rs, &mut self.g.world.redstone);
        true
    }

    fn hook_tick(&mut self, rs: &mut Rs, p: BlockPos, st: u16) -> bool {
        let Some(f) = hooks_of(block(st)).and_then(|h| h.tick) else { return false };
        std::mem::swap(rs, &mut self.g.world.redstone);
        f(self.g, p, st);
        std::mem::swap(rs, &mut self.g.world.redstone);
        true
    }
}

/// Read-only view for signal queries from other modules.
struct View<'a> {
    g: &'a Game,
}

impl Lvl for View<'_> {
    fn get(&self, p: BlockPos) -> u16 {
        self.g.world.get_block(p.x, p.y, p.z)
    }
    fn loaded(&self, p: BlockPos) -> bool {
        (MIN_Y..=MAX_Y).contains(&p.y) && self.g.world.is_loaded(p.x, p.z)
    }
    fn raw_set(&mut self, _p: BlockPos, _st: u16) -> u16 {
        0
    }
    fn time(&self) -> u64 {
        self.g.world.rs_time
    }
    fn day_time(&self) -> i64 {
        self.g.day_time
    }
    fn sky_light(&self, p: BlockPos) -> u8 {
        read_sky(self.g, p)
    }
    fn level_event(&mut self, _ev: i32, _p: BlockPos, _data: i32) {}
    fn sound(&mut self, _name: &str, _p: BlockPos, _vol: f32, _pitch: f32, _cat: i32, _skip_actor: bool) {}
    fn block_event_packet(&mut self, _p: BlockPos, _block: u16, _a: u8, _b: u8) {}
    fn drop_block(&mut self, _p: BlockPos, _st: u16) {}
    fn prime_tnt(&mut self, _p: BlockPos) {}
    fn count_entities(&self, min: [f64; 3], max: [f64; 3], living_only: bool) -> usize {
        count_in(self.g, min, max, living_only)
    }
    fn container_signal(&self, p: BlockPos, st: u16) -> Option<u8> {
        super::containers::analog_signal(p, st)
    }
    fn chest_viewers(&self, p: BlockPos) -> u8 {
        super::containers::viewers_at_pos(p)
    }
    fn rand(&mut self) -> f32 {
        0.5
    }
}
