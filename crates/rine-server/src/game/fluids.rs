//! Vanilla fluid flow: a port of `FlowingFluid`, `WaterFluid`, `LavaFluid`,
//! `LiquidBlock` and the bucket rules of `BucketItem` (Paper 26.3 sources).
//!
//! Every block change lands in `World::fluid_touched`. `tick` runs once per
//! game tick before `neighbors::settle`: it turns those changes into scheduled
//! fluid ticks (vanilla `onPlace` / `neighborChanged`), runs the due ticks and
//! their block changes ride in the same tick's block packets.
//!
//! Overworld constants only: water drops 1 per block, lava 2; water searches
//! for a drop 4 blocks away, lava 2; tick delays 5 and 30.

use super::items;
use super::loot;
use super::player::Stack;
use super::{entity, track, Game};
use crate::out;
use ahash::AHashMap;
use rine_data::blocks::{flag, StateInfo};
use rine_protocol::BlockPos;
use rine_world::ChunkPos;
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// Fluid ticks run per game tick; the rest carries over to the next tick.
const MAX_FLUID_TICKS: usize = 4096;
/// Reach of the bucket raycast, in blocks.
const REACH: f64 = 5.0;
const AIR: u16 = 0;
const LEVEL_EVENT_DESTROY: i32 = 2001;
const LEVEL_EVENT_LAVA_FIZZ: i32 = 1501;
const SOUND_CATEGORY_BLOCKS: i32 = 4;
const SOUND_CATEGORY_PLAYERS: i32 = 7;
/// `Fluid.canBeReplacedWith` for lava: flowing lava at least this high is replaced by water.
const LAVA_HEIGHT_CUTOFF: f32 = 0.444_444_45;

const DOWN: [i32; 3] = [0, -1, 0];
const UP: [i32; 3] = [0, 1, 0];
/// `Direction.Plane.HORIZONTAL`: north, east, south, west.
const HORIZ: [[i32; 3]; 4] = [[0, 0, -1], [1, 0, 0], [0, 0, 1], [-1, 0, 0]];
/// `LiquidBlock.POSSIBLE_FLOW_DIRECTIONS` mapped to the neighbour it reads
/// (`pos.relative(direction.getOpposite())`): above, north, south, west, east.
const LAVA_CHECK: [[i32; 3]; 5] = [UP, [0, 0, -1], [0, 0, 1], [-1, 0, 0], [1, 0, 0]];
/// Containers without `waterlogged`: `LiquidBlockContainer`s that accept no fluid.
const NO_FLUID_CONTAINERS: [&str; 4] = ["minecraft:kelp", "minecraft:kelp_plant", "minecraft:seagrass", "minecraft:tall_seagrass"];

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Kind {
    Water,
    Lava,
}

impl Kind {
    fn index(self) -> usize {
        match self {
            Kind::Water => 0,
            Kind::Lava => 1,
        }
    }
    fn drop_off(self) -> i32 {
        match self {
            Kind::Water => 1,
            Kind::Lava => 2,
        }
    }
    fn slope_distance(self) -> i32 {
        match self {
            Kind::Water => 4,
            Kind::Lava => 2,
        }
    }
    pub fn tick_delay(self) -> u64 {
        match self {
            Kind::Water => 5,
            Kind::Lava => 30,
        }
    }
    /// Gamerule defaults: `waterSourceConversion` true, `lavaSourceConversion` false.
    fn can_convert_to_source(self) -> bool {
        matches!(self, Kind::Water)
    }
    fn source(self) -> Fluid {
        Fluid { kind: self, amount: 8, falling: false, source: true }
    }
    fn flowing(self, amount: u8, falling: bool) -> Fluid {
        Fluid { kind: self, amount, falling, source: false }
    }
}

/// A `FluidState`. `source` is also the fluid *type*: `Fluids.WATER` versus
/// `Fluids.FLOWING_WATER`. Empty fluid is `None`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Fluid {
    pub kind: Kind,
    pub amount: u8,
    pub falling: bool,
    pub source: bool,
}

/// Fluid type: `(kind, is_source_type)`, `None` for empty.
type FType = Option<(Kind, bool)>;

fn ftype(f: Option<Fluid>) -> FType {
    f.map(|f| (f.kind, f.source))
}

fn is_source_of(f: Option<Fluid>, kind: Kind) -> bool {
    f.map_or(false, |f| f.kind == kind && f.source)
}

/// Per-state lookup tables, built once from the block data.
struct Tables {
    /// Fluid of each block state.
    fluid: Vec<Option<Fluid>>,
    /// Water or lava block itself (`LiquidBlock`).
    liquid: Vec<bool>,
    /// `LiquidBlockContainer` (waterlogged blocks and the kelp/seagrass family).
    container: Vec<bool>,
    /// Container that takes source water (`SimpleWaterloggedBlock`).
    accepts_water: Vec<bool>,
    waterlogged: Vec<bool>,
    /// `#minecraft:washed_away_by_fluids`: fluid may replace these blocks.
    washed: Vec<bool>,
    /// Block state for LEVEL 0..15 of water (index 0) and lava (index 1).
    level_state: [Vec<u16>; 2],
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(build_tables)
}

fn build_tables() -> Tables {
    let d = rine_data::get();
    let b = &d.blocks;
    let water = b.block("minecraft:water").expect("minecraft:water").id;
    let lava = b.block("minecraft:lava").expect("minecraft:lava").id;
    let nblocks = b.blocks.len();

    let mut washed_block = vec![false; nblocks];
    if let Some(names) = d.registries.tags.get("minecraft:block").and_then(|t| t.get("minecraft:washed_away_by_fluids")) {
        for n in names {
            if let Some(bi) = b.block(n) {
                washed_block[bi.id as usize] = true;
            }
        }
    }
    let mut container_block = vec![false; nblocks];
    let mut accepts_block = vec![false; nblocks];
    for bi in &b.blocks {
        let wl = bi.props.iter().any(|p| p.name == "waterlogged");
        accepts_block[bi.id as usize] = wl;
        container_block[bi.id as usize] = wl || NO_FLUID_CONTAINERS.contains(&bi.name.as_str());
    }

    let n = b.states.len();
    let mut t = Tables {
        fluid: Vec::with_capacity(n),
        liquid: Vec::with_capacity(n),
        container: Vec::with_capacity(n),
        accepts_water: Vec::with_capacity(n),
        waterlogged: Vec::with_capacity(n),
        washed: Vec::with_capacity(n),
        level_state: [vec![0; 16], vec![0; 16]],
    };
    for s in 0..n {
        let st = s as u16;
        let info: &StateInfo = b.state(st);
        let bid = info.block as usize;
        let is_w = info.block as u32 == water;
        let is_l = info.block as u32 == lava;
        let wl = b.get_prop(st, "waterlogged") == Some("true");
        let fluid = if is_w || is_l {
            let kind = if is_w { Kind::Water } else { Kind::Lava };
            let level = b.get_prop(st, "level").and_then(|v| v.parse::<u8>().ok()).unwrap_or(0);
            Some(match level {
                0 => kind.source(),
                1..=7 => kind.flowing(8 - level, false),
                _ => kind.flowing(8, true),
            })
        } else if wl {
            Some(Kind::Water.source())
        } else if info.flags & flag::FLUID_SOURCE != 0 && info.flags & flag::WATER != 0 {
            Some(Kind::Water.source())
        } else if info.flags & flag::FLUID_SOURCE != 0 && info.flags & flag::LAVA != 0 {
            Some(Kind::Lava.source())
        } else {
            None
        };
        t.fluid.push(fluid);
        t.liquid.push(is_w || is_l);
        t.container.push(container_block[bid]);
        t.accepts_water.push(accepts_block[bid]);
        t.waterlogged.push(wl);
        t.washed.push(washed_block[bid]);
    }
    for (i, name) in ["minecraft:water", "minecraft:lava"].iter().enumerate() {
        let dflt = b.default_state(name);
        for lv in 0..16 {
            t.level_state[i][lv] = b.with_prop(dflt, "level", &lv.to_string()).unwrap_or(dflt);
        }
    }
    t
}

fn fluid_of(st: u16) -> Option<Fluid> {
    tables().fluid[st as usize]
}

fn is_liquid(st: u16) -> bool {
    tables().liquid[st as usize]
}

/// Block state that holds `f` (`createLegacyBlock`); empty is air.
fn fluid_state(f: Option<Fluid>) -> u16 {
    match f {
        None => AIR,
        Some(f) => {
            let level = if f.source {
                0
            } else if f.falling {
                8
            } else {
                8 - f.amount as usize
            };
            tables().level_state[f.kind.index()][level]
        }
    }
}

fn with_waterlogged(st: u16, on: bool) -> u16 {
    rine_data::get().blocks.with_prop(st, "waterlogged", if on { "true" } else { "false" }).unwrap_or(st)
}

fn block_name(st: u16) -> &'static str {
    // block names live as long as the global block data
    rine_data::get().blocks.block_of(st).name.as_str()
}

/// Access to the world as the fluid code sees it.
pub trait Level {
    /// Block state at `p`; `None` for unloaded chunks (vanilla `getBlockStateIfLoaded`).
    fn get(&self, p: BlockPos) -> Option<u16>;
    fn set(&mut self, p: BlockPos, state: u16);
    /// Level event (1501 lava fizz) at `p`.
    fn level_event(&mut self, p: BlockPos, event: i32);
    /// Loot of a block that a fluid destroys.
    fn drop_loot(&mut self, p: BlockPos, state: u16);
    /// Uniform in [0, 1).
    fn rand(&mut self) -> f64;
}

fn st_or_air<L: Level>(l: &L, p: BlockPos) -> u16 {
    l.get(p).unwrap_or(AIR)
}

fn add(p: BlockPos, d: [i32; 3]) -> BlockPos {
    p.offset(d[0], d[1], d[2])
}

fn neg(d: [i32; 3]) -> [i32; 3] {
    [-d[0], -d[1], -d[2]]
}

/// Whether one full face of `boxes` covers the unit square on side `d`.
fn face_covered(boxes: &[[f32; 6]], d: [i32; 3]) -> bool {
    const E: f32 = 1e-4;
    let axis = if d[0] != 0 { 0 } else if d[1] != 0 { 1 } else { 2 };
    boxes.iter().any(|b| {
        let face = if d[axis] > 0 { b[axis + 3] >= 1.0 - E } else { b[axis] <= E };
        let others = (0..3).filter(|&a| a != axis).all(|a| b[a] <= E && b[a + 3] >= 1.0 - E);
        face && others
    })
}

/// `canPassThroughWall`: fluid may move from `from` to `to` across side `d`.
/// The shapes are merged approximately: one box must cover a full face.
fn can_pass_wall(d: [i32; 3], from: u16, to: u16) -> bool {
    let b = &rine_data::get().blocks;
    let (a, c) = (b.state(from), b.state(to));
    let no_col = |s: &StateInfo| s.flags & flag::NO_COLLISION != 0;
    if no_col(a) && no_col(c) {
        return true;
    }
    if a.flags & flag::FULL_CUBE != 0 || c.flags & flag::FULL_CUBE != 0 {
        return false;
    }
    !(face_covered(&b.shapes[a.shape as usize], d) || face_covered(&b.shapes[c.shape as usize], neg(d)))
}

fn can_hold_any(st: u16) -> bool {
    let t = tables();
    t.container[st as usize] || t.washed[st as usize]
}

/// `canHoldSpecificFluid`: containers take only source water.
fn can_hold_specific(st: u16, ty: FType) -> bool {
    let t = tables();
    !t.container[st as usize] || (t.accepts_water[st as usize] && ty == Some((Kind::Water, true)))
}

/// `canMaybePassThrough`.
fn can_maybe_pass(kind: Kind, d: [i32; 3], from: u16, to: u16) -> bool {
    !is_source_of(fluid_of(to), kind) && can_hold_any(to) && can_pass_wall(d, from, to)
}

fn can_pass_through(kind: Kind, d: [i32; 3], from: u16, to: u16) -> bool {
    can_maybe_pass(kind, d, from, to) && can_hold_specific(to, Some((kind, false)))
}

/// `isWaterHole`: `bottom` takes fluid below `top`.
fn is_water_hole(kind: Kind, top: u16, bottom: u16) -> bool {
    can_pass_wall(DOWN, top, bottom)
        && (fluid_of(bottom).map_or(false, |f| f.kind == kind) || (can_hold_any(bottom) && can_hold_specific(bottom, Some((kind, false)))))
}

fn is_hole<L: Level>(l: &L, kind: Kind, p: BlockPos, st: u16) -> bool {
    let below = add(p, DOWN);
    is_water_hole(kind, st, st_or_air(l, below))
}

/// `FlowingFluid.getNewLiquid`: the fluid `kind` that the block at `p` should hold.
fn new_liquid<L: Level>(l: &mut L, kind: Kind, p: BlockPos, st: u16) -> Option<Fluid> {
    let mut highest = 0u8;
    let mut sources = 0;
    for d in HORIZ {
        let np = add(p, d);
        let Some(ns) = l.get(np) else { continue };
        if let Some(f) = fluid_of(ns) {
            if f.kind == kind && can_pass_wall(d, st, ns) {
                if f.source {
                    sources += 1;
                }
                highest = highest.max(f.amount);
            }
        }
    }
    if sources >= 2 && kind.can_convert_to_source() {
        let bs = st_or_air(l, add(p, DOWN));
        if rine_data::get().blocks.has(bs, flag::SOLID) || is_source_of(fluid_of(bs), kind) {
            return Some(kind.source());
        }
    }
    let above = st_or_air(l, add(p, UP));
    if fluid_of(above).map_or(false, |f| f.kind == kind) && can_pass_wall(UP, st, above) {
        return Some(kind.flowing(8, true));
    }
    let amount = highest as i32 - kind.drop_off();
    if amount <= 0 {
        None
    } else {
        Some(kind.flowing(amount as u8, false))
    }
}

/// `FlowingFluid.canBeReplacedWith` for the fluid at `p` (state `test`).
fn can_replace<L: Level>(l: &L, test: Option<Fluid>, p: BlockPos, ty: FType, d: [i32; 3]) -> bool {
    match test {
        None => true,
        Some(f) => match f.kind {
            Kind::Water => d == DOWN && !matches!(ty, Some((Kind::Water, _))),
            Kind::Lava => height(l, f, p) >= LAVA_HEIGHT_CUTOFF && matches!(ty, Some((Kind::Water, _))),
        },
    }
}

/// `getHeight`: a full block when the same fluid is above.
fn height<L: Level>(l: &L, f: Fluid, p: BlockPos) -> f32 {
    if fluid_of(st_or_air(l, add(p, UP))).map_or(false, |a| a.kind == f.kind) {
        1.0
    } else {
        f.amount as f32 / 9.0
    }
}

fn source_neighbor_count<L: Level>(l: &L, kind: Kind, p: BlockPos) -> usize {
    HORIZ.iter().filter(|&&d| is_source_of(fluid_of(st_or_air(l, add(p, d))), kind)).count()
}

/// `getSlopeDistance`: distance to the nearest drop, searching sideways.
fn slope_distance<L: Level>(l: &mut L, kind: Kind, p: BlockPos, st: u16, pass: i32, from: [i32; 3]) -> i32 {
    let mut lowest = 1000;
    for d in HORIZ {
        if d == from {
            continue;
        }
        let tp = add(p, d);
        let Some(ts) = l.get(tp) else { continue };
        if can_pass_through(kind, d, st, ts) {
            if is_hole(l, kind, tp, ts) {
                return pass;
            }
            if pass < kind.slope_distance() {
                lowest = lowest.min(slope_distance(l, kind, tp, ts, pass + 1, neg(d)));
            }
        }
    }
    lowest
}

/// `getSpread`: sideways targets with the lowest drop distance.
fn get_spread<L: Level>(l: &mut L, kind: Kind, p: BlockPos, st: u16) -> Vec<([i32; 3], Option<Fluid>)> {
    let mut lowest = 1000;
    let mut out = Vec::new();
    for d in HORIZ {
        let tp = add(p, d);
        let Some(ts) = l.get(tp) else { continue };
        if !can_maybe_pass(kind, d, st, ts) {
            continue;
        }
        let nf = new_liquid(l, kind, tp, ts);
        if !can_hold_specific(ts, ftype(nf)) {
            continue;
        }
        let distance = if is_hole(l, kind, tp, ts) { 0 } else { slope_distance(l, kind, tp, ts, 1, neg(d)) };
        if distance < lowest {
            out.clear();
        }
        if distance <= lowest {
            if can_replace(l, fluid_of(ts), tp, ftype(nf), d) {
                out.push((d, nf));
            }
            lowest = distance;
        }
    }
    out
}

/// `LavaFluid.spreadTo` and `FlowingFluid.spreadTo`: put `target` at `p`.
fn spread_to<L: Level>(l: &mut L, kind: Kind, p: BlockPos, st: u16, d: [i32; 3], target: Option<Fluid>) {
    if kind == Kind::Lava && d == DOWN && fluid_of(st).map_or(false, |f| f.kind == Kind::Water) {
        // lava onto water: a liquid block turns to stone, a waterlogged block only fizzes
        if is_liquid(st) {
            l.set(p, rine_data::get().blocks.default_state("minecraft:stone"));
        }
        l.level_event(p, LEVEL_EVENT_LAVA_FIZZ);
        return;
    }
    let t = tables();
    if t.container[st as usize] {
        // SimpleWaterloggedBlock.placeLiquid: only source water
        if t.accepts_water[st as usize] && !t.waterlogged[st as usize] && matches!(target, Some(f) if f.kind == Kind::Water && f.source) {
            l.set(p, with_waterlogged(st, true));
        }
        return;
    }
    if st != AIR {
        // beforeDestroyingBlock: water drops the block's loot, lava fizzes
        match kind {
            Kind::Water => {
                if !is_liquid(st) {
                    l.drop_loot(p, st);
                }
            }
            Kind::Lava => l.level_event(p, LEVEL_EVENT_LAVA_FIZZ),
        }
    }
    l.set(p, fluid_state(target));
}

/// `FlowingFluid.spreadToSides`.
fn spread_to_sides<L: Level>(l: &mut L, kind: Kind, p: BlockPos, f: Fluid, st: u16) {
    let n = if f.falling { 7 } else { f.amount as i32 - kind.drop_off() };
    if n <= 0 {
        return;
    }
    for (d, nf) in get_spread(l, kind, p, st) {
        let np = add(p, d);
        let Some(ns) = l.get(np) else { continue };
        spread_to(l, kind, np, ns, d, nf);
    }
}

/// `FlowingFluid.spread`.
fn spread<L: Level>(l: &mut L, kind: Kind, p: BlockPos, st: u16, fluid: Option<Fluid>) {
    let Some(f) = fluid else { return };
    let below = add(p, DOWN);
    let bs = st_or_air(l, below);
    if can_maybe_pass(kind, DOWN, st, bs) {
        let nb = new_liquid(l, kind, below, bs);
        if can_replace(l, fluid_of(bs), below, ftype(nb), DOWN) && can_hold_specific(bs, ftype(nb)) {
            spread_to(l, kind, below, bs, DOWN, nb);
            if source_neighbor_count(l, kind, p) >= 3 {
                spread_to_sides(l, kind, p, f, st);
            }
            return;
        }
    }
    if f.source || !is_water_hole(kind, st, bs) {
        spread_to_sides(l, kind, p, f, st);
    }
}

/// `LiquidBlock.shouldSpreadLiquid` for lava: lava touching water (or soul soil
/// over blue ice) is converted. Returns false when it was converted.
fn lava_may_spread<L: Level>(l: &mut L, p: BlockPos, st: u16) -> bool {
    let over_soul_soil = block_name(st_or_air(l, add(p, DOWN))) == "minecraft:soul_soil";
    let source = fluid_of(st).map_or(false, |f| f.source);
    for d in LAVA_CHECK {
        let ns = st_or_air(l, add(p, d));
        if fluid_of(ns).map_or(false, |f| f.kind == Kind::Water) {
            let to = if source { "minecraft:obsidian" } else { "minecraft:cobblestone" };
            l.set(p, rine_data::get().blocks.default_state(to));
            l.level_event(p, LEVEL_EVENT_LAVA_FIZZ);
            return false;
        }
        if over_soul_soil && block_name(ns) == "minecraft:blue_ice" {
            l.set(p, rine_data::get().blocks.default_state("minecraft:basalt"));
            l.level_event(p, LEVEL_EVENT_LAVA_FIZZ);
            return false;
        }
    }
    true
}

/// `getSpreadDelay`: lava spreads four times slower once it has grown.
fn spread_delay<L: Level>(l: &mut L, kind: Kind, p: BlockPos, old: Fluid, new: Option<Fluid>) -> u64 {
    let base = kind.tick_delay();
    if let Some(n) = new {
        if kind == Kind::Lava && !old.falling && !n.falling && height(l, n, p) > height(l, old, p) && (l.rand() * 4.0) as i32 != 0 {
            return base * 4;
        }
    }
    base
}

/// `FlowingFluid.tick` for the fluid of `kind` at `p`. Returns the delay of the
/// next tick when the block changed within its own block type; otherwise the
/// caller's neighbour pass schedules it (vanilla `onPlace` ordering).
fn tick_block<L: Level>(l: &mut L, kind: Kind, p: BlockPos) -> Option<u64> {
    let st = st_or_air(l, p);
    let cur = fluid_of(st).filter(|f| f.kind == kind)?;
    let mut explicit = None;
    let (st2, cur2) = if !cur.source {
        let nf = new_liquid(l, kind, p, st);
        let delay = spread_delay(l, kind, p, cur, nf);
        match nf {
            None => {
                l.set(p, AIR);
                (AIR, None)
            }
            Some(n) if n != cur => {
                let ns = fluid_state(Some(n));
                let blocks = &rine_data::get().blocks;
                let same_block = blocks.state(ns).block == blocks.state(st).block;
                l.set(p, ns);
                if same_block {
                    explicit = Some(delay);
                }
                (ns, Some(n))
            }
            _ => (st, Some(cur)),
        }
    } else {
        (st, Some(cur))
    };
    spread(l, kind, p, st2, cur2);
    explicit
}

/// Scheduled fluid ticks, bucketed by due tick. One entry per position: the
/// first schedule wins (vanilla `ServerTickList` dedup).
#[derive(Default)]
pub struct Sched {
    buckets: BTreeMap<u64, Vec<(BlockPos, Kind)>>,
    pending: AHashMap<BlockPos, u64>,
}

impl Sched {
    pub fn schedule(&mut self, p: BlockPos, kind: Kind, due: u64) {
        if self.pending.contains_key(&p) {
            return;
        }
        self.pending.insert(p, due);
        self.buckets.entry(due).or_default().push((p, kind));
    }

    /// Entries due at or before `now`, at most `budget`; the rest stays queued.
    fn pop_due(&mut self, now: u64, budget: usize) -> Vec<(BlockPos, Kind)> {
        let mut out = Vec::new();
        while out.len() < budget {
            let Some((&due, _)) = self.buckets.first_key_value() else { break };
            if due > now {
                break;
            }
            let bucket = self.buckets.remove(&due).unwrap_or_default();
            let mut it = bucket.into_iter();
            while let Some((p, k)) = it.next() {
                if out.len() >= budget {
                    let mut rest = vec![(p, k)];
                    rest.extend(it);
                    self.buckets.entry(due).or_default().extend(rest);
                    return out;
                }
                self.pending.remove(&p);
                out.push((p, k));
            }
        }
        out
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// Splitmix64 hash to [0, 1).
fn rand01(seed: u64) -> f64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

/// Game-side `Level`: block writes go through `World::set_block`.
struct Lv<'a> {
    g: &'a mut Game,
    n: u64,
}

impl Level for Lv<'_> {
    fn get(&self, p: BlockPos) -> Option<u16> {
        let w = &self.g.world;
        if w.is_loaded(p.x, p.z) {
            Some(w.get_block(p.x, p.y, p.z))
        } else {
            None
        }
    }

    fn set(&mut self, p: BlockPos, state: u16) {
        self.g.world.set_block(p, state);
    }

    fn level_event(&mut self, p: BlockPos, event: i32) {
        let f = out::frame(out::level_event(event, p, 0));
        self.g.send_chunk_viewers(chunk_of(p), &f);
    }

    fn drop_loot(&mut self, p: BlockPos, state: u16) {
        drop_stacks(self.g, p, state);
    }

    fn rand(&mut self) -> f64 {
        self.n += 1;
        rand01(self.g.tick.wrapping_mul(0x2545_F491_4F6C_DD1D) ^ self.n)
    }
}

fn chunk_of(p: BlockPos) -> ChunkPos {
    ChunkPos::new(p.x >> 4, p.z >> 4)
}

fn drop_stacks(g: &mut Game, p: BlockPos, state: u16) {
    for s in loot::block_drops(state, 0) {
        entity::spawn_item(g, [p.x as f64 + 0.5, p.y as f64 + 0.5, p.z as f64 + 0.5], [0.0, 0.2, 0.0], s, 10);
    }
}

/// Fluid ticks for the changes of the last pass, then the due ticks.
/// Call once per game tick, before `neighbors::settle`.
pub fn tick(g: &mut Game) {
    let now = g.tick;
    let mut sched = std::mem::take(&mut g.world.fluids);
    scan_changes(g, &mut sched, now);
    run_due(g, &mut sched, now);
    scan_changes(g, &mut sched, now);
    g.world.fluids = sched;
}

/// `onPlace` and `neighborChanged` of every changed position and its six
/// neighbours: lava checks for water first, then fluids are scheduled.
fn scan_changes(g: &mut Game, sched: &mut Sched, now: u64) {
    loop {
        let batch = std::mem::take(&mut g.world.fluid_touched);
        if batch.is_empty() {
            break;
        }
        let mut lv = Lv { g: &mut *g, n: 0 };
        for p in batch {
            let cells = [p, add(p, DOWN), add(p, UP), add(p, HORIZ[0]), add(p, HORIZ[1]), add(p, HORIZ[2]), add(p, HORIZ[3])];
            for q in cells {
                let Some(st) = lv.get(q) else { continue };
                let Some(f) = fluid_of(st) else { continue };
                if f.kind == Kind::Lava && !lava_may_spread(&mut lv, q, st) {
                    continue;
                }
                sched.schedule(q, f.kind, now + f.kind.tick_delay());
            }
        }
    }
}

fn run_due(g: &mut Game, sched: &mut Sched, now: u64) {
    let due = sched.pop_due(now, MAX_FLUID_TICKS);
    if due.is_empty() {
        return;
    }
    let mut lv = Lv { g: &mut *g, n: 0 };
    for (p, kind) in due {
        let Some(st) = lv.get(p) else { continue };
        if fluid_of(st).map(|f| f.kind) != Some(kind) {
            continue;
        }
        if let Some(delay) = tick_block(&mut lv, kind, p) {
            sched.schedule(p, kind, now + delay);
        }
    }
}

// ---- buckets ---------------------------------------------------------------

struct Ids {
    bucket: u32,
    water_bucket: u32,
    lava_bucket: u32,
    fill: u32,
    fill_lava: u32,
    empty: u32,
    empty_lava: u32,
}

fn ids() -> &'static Ids {
    static I: OnceLock<Ids> = OnceLock::new();
    I.get_or_init(|| {
        let d = rine_data::get();
        let snd = |n: &str| d.registries.id("minecraft:sound_event", n).unwrap_or(0);
        Ids {
            bucket: items::id("minecraft:bucket").unwrap_or(0),
            water_bucket: items::id("minecraft:water_bucket").unwrap_or(0),
            lava_bucket: items::id("minecraft:lava_bucket").unwrap_or(0),
            fill: snd("minecraft:item.bucket.fill"),
            fill_lava: snd("minecraft:item.bucket.fill_lava"),
            empty: snd("minecraft:item.bucket.empty"),
            empty_lava: snd("minecraft:item.bucket.empty_lava"),
        }
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Hit {
    pub pos: BlockPos,
    /// Outward normal of the entered face (points back at the ray origin).
    pub normal: [i32; 3],
    pub state: u16,
}

/// Voxel traversal (Amanatides and Woo) from `origin` along `dir`. The start
/// cell is not tested, as with vanilla `clip`. With `fluids`, source fluids
/// count as hits (empty bucket: `ClipContext.Fluid.SOURCE_ONLY`), otherwise
/// fluids are transparent.
pub fn raycast_with(get: impl Fn(BlockPos) -> Option<u16>, origin: [f64; 3], dir: [f64; 3], reach: f64, fluids: bool) -> Option<Hit> {
    let mut cell = [origin[0].floor() as i32, origin[1].floor() as i32, origin[2].floor() as i32];
    let mut step = [0i32; 3];
    let mut tmax = [f64::INFINITY; 3];
    let mut tdelta = [f64::INFINITY; 3];
    for a in 0..3 {
        if dir[a] > 1e-9 {
            step[a] = 1;
            tdelta[a] = 1.0 / dir[a];
            tmax[a] = (cell[a] as f64 + 1.0 - origin[a]) / dir[a];
        } else if dir[a] < -1e-9 {
            step[a] = -1;
            tdelta[a] = -1.0 / dir[a];
            tmax[a] = (origin[a] - cell[a] as f64) / -dir[a];
        }
    }
    let blocks = &rine_data::get().blocks;
    for _ in 0..64 {
        let a = (0..3).min_by(|&i, &j| tmax[i].total_cmp(&tmax[j]))?;
        if !(tmax[a] <= reach) {
            return None;
        }
        cell[a] += step[a];
        tmax[a] += tdelta[a];
        let mut normal = [0i32; 3];
        normal[a] = -step[a];
        let p = BlockPos::new(cell[0], cell[1], cell[2]);
        let st = get(p)?;
        if fluids && fluid_of(st).map_or(false, |f| f.source) {
            return Some(Hit { pos: p, normal, state: st });
        }
        if blocks.state(st).flags & flag::NO_COLLISION == 0 {
            return Some(Hit { pos: p, normal, state: st });
        }
    }
    None
}

/// `Entity.getViewVector` from yaw and pitch in degrees.
fn look_dir(yaw: f32, pitch: f32) -> [f64; 3] {
    let y = (yaw as f64).to_radians();
    let p = (pitch as f64).to_radians();
    [-y.sin() * p.cos(), -p.sin(), y.cos() * p.cos()]
}

/// Play a sound to the viewers of the chunk of `at`, except `except`.
fn play(g: &mut Game, sound: u32, category: i32, at: [f64; 3], volume: f32, except: Option<usize>) {
    let f = out::frame(out::sound(sound, category, at[0], at[1], at[2], volume, 1.0, g.tick as i64));
    let c = ChunkPos::new((at[0].floor() as i32) >> 4, (at[2].floor() as i32) >> 4);
    g.send_chunk_viewers_except(c, &f, except);
}

fn destroy_block(g: &mut Game, p: BlockPos, state: u16) {
    let f = out::frame(out::level_event(LEVEL_EVENT_DESTROY, p, state as i32));
    g.send_chunk_viewers(chunk_of(p), &f);
    drop_stacks(g, p, state);
    g.world.set_block(p, AIR);
}

/// `ItemUtils.createFilledResult`: replace the hand stack, or with more than
/// one in hand, shrink it and put the result into the inventory.
fn filled_result(g: &mut Game, slot: usize, inv_slot: usize, held: Stack, result: Stack) {
    let mut leftover = None;
    let eye = {
        let Some(p) = g.player(slot) else { return };
        if held.count <= 1 {
            p.inv.slots[inv_slot] = result;
        } else {
            p.inv.slots[inv_slot].count -= 1;
            let rest = p.inv.add(result.item, result.count);
            if rest > 0 {
                leftover = Some(Stack::new(result.item, rest));
            }
        }
        let inv = p.inventory_packet();
        p.send(inv);
        p.pos
    };
    if let Some(s) = leftover {
        entity::spawn_item(g, eye, [0.0, 0.2, 0.0], s, 40);
    }
    track::equipment_changed(g, slot);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Content {
    Empty,
    Filled(Kind),
}

/// Right click in the air with a bucket (vanilla `BucketItem.use`). Returns
/// false when the held item is not a bucket or the player cannot use items,
/// so the caller can run its other item logic.
pub fn use_bucket(g: &mut Game, slot: usize, hand: i32) -> bool {
    let ids = ids();
    let (inv_slot, held, eye, look, creative, pos) = {
        let Some(p) = g.player(slot) else { return false };
        if p.dead || p.gamemode == 3 {
            return false;
        }
        let inv_slot = if hand == 1 { 45 } else { 36 + p.held as usize };
        (inv_slot, p.inv.slots[inv_slot], p.eye(), look_dir(p.yaw, p.pitch), p.gamemode == 1, p.pos)
    };
    let content = if held.item == ids.bucket {
        Content::Empty
    } else if held.item == ids.water_bucket {
        Content::Filled(Kind::Water)
    } else if held.item == ids.lava_bucket {
        Content::Filled(Kind::Lava)
    } else {
        return false;
    };
    match content {
        Content::Empty => pick_up(g, slot, inv_slot, held, eye, look, pos),
        Content::Filled(kind) => fill(g, slot, inv_slot, held, kind, eye, look, creative),
    }
    true
}

/// `BucketItem.use` with an empty bucket: take a source fluid (or a waterlogged block's water).
fn pick_up(g: &mut Game, slot: usize, inv_slot: usize, held: Stack, eye: [f64; 3], look: [f64; 3], ppos: [f64; 3]) {
    let world = &g.world;
    let hit = raycast_with(|p| world.is_loaded(p.x, p.z).then(|| world.get_block(p.x, p.y, p.z)), eye, look, REACH, true);
    let Some(hit) = hit else { return };
    let st = hit.state;
    let t = tables();
    let (new_state, kind) = match fluid_of(st) {
        Some(f) if f.source && is_liquid(st) => (AIR, f.kind),
        _ if t.waterlogged[st as usize] => (with_waterlogged(st, false), Kind::Water),
        _ => return,
    };
    g.world.set_block(hit.pos, new_state);
    let ids = ids();
    let (sound, item) = match kind {
        Kind::Water => (ids.fill, ids.water_bucket),
        Kind::Lava => (ids.fill_lava, ids.lava_bucket),
    };
    // vanilla player.playSound: heard by everyone, the user included
    play(g, sound, SOUND_CATEGORY_PLAYERS, ppos, 1.0, None);
    filled_result(g, slot, inv_slot, held, Stack::new(item, 1));
}

/// `BucketItem.use` with a filled bucket: place the fluid on the clicked face.
fn fill(g: &mut Game, slot: usize, inv_slot: usize, held: Stack, kind: Kind, eye: [f64; 3], look: [f64; 3], creative: bool) {
    let world = &g.world;
    let hit = raycast_with(|p| world.is_loaded(p.x, p.z).then(|| world.get_block(p.x, p.y, p.z)), eye, look, REACH, false);
    let Some(hit) = hit else { return };
    let t = tables();
    let blocks = &rine_data::get().blocks;
    let place = if kind == Kind::Water && t.accepts_water[hit.state as usize] {
        hit.pos
    } else {
        BlockPos::new(hit.pos.x + hit.normal[0], hit.pos.y + hit.normal[1], hit.pos.z + hit.normal[2])
    };
    let pst = g.world.get_block(place.x, place.y, place.z);
    let may_replace = blocks.is_air(pst) || blocks.has(pst, flag::REPLACEABLE);
    let container_ok = kind == Kind::Water && t.accepts_water[pst as usize];
    if !(blocks.is_air(pst) || may_replace || container_ok) {
        return;
    }
    if container_ok {
        // LiquidBlockContainer.placeLiquid: waterlog; vanilla still uses the bucket
        if !t.waterlogged[pst as usize] {
            g.world.set_block(place, with_waterlogged(pst, true));
        }
    } else {
        if !blocks.is_air(pst) && !is_liquid(pst) && may_replace {
            destroy_block(g, place, pst);
        }
        g.world.set_block(place, fluid_state(Some(kind.source())));
    }
    let ids = ids();
    let sound = if kind == Kind::Water { ids.empty } else { ids.empty_lava };
    let at = [place.x as f64 + 0.5, place.y as f64 + 0.5, place.z as f64 + 0.5];
    // playEmptySound excludes the user: the client plays its own predicted sound
    play(g, sound, SOUND_CATEGORY_BLOCKS, at, 1.0, Some(slot));
    let result = if creative { held } else { Stack::new(ids.bucket, 1) };
    filled_result(g, slot, inv_slot, held, result);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ahash::AHashSet;

    fn init() {
        rine_data::init();
    }

    fn st(s: &str) -> u16 {
        rine_data::get().blocks.parse_state(s).unwrap_or_else(|| panic!("state {s}"))
    }

    fn at(x: i32, y: i32, z: i32) -> BlockPos {
        BlockPos::new(x, y, z)
    }

    /// Flat in-memory level: every position is loaded, unset positions are air.
    #[derive(Default)]
    struct Grid {
        cells: AHashMap<BlockPos, u16>,
        sets: Vec<(BlockPos, u16)>,
        fizz: Vec<BlockPos>,
        drops: Vec<(BlockPos, u16)>,
        unloaded: AHashSet<BlockPos>,
        rnd: f64,
    }

    impl Grid {
        fn put(&mut self, p: BlockPos, s: &str) {
            self.cells.insert(p, st(s));
        }
        fn at(&self, p: BlockPos) -> u16 {
            self.cells.get(&p).copied().unwrap_or(AIR)
        }
        fn fluid(&self, p: BlockPos) -> Option<Fluid> {
            fluid_of(self.at(p))
        }
        fn floor(&mut self, x0: i32, x1: i32, z0: i32, z1: i32, y: i32) {
            for x in x0..=x1 {
                for z in z0..=z1 {
                    self.put(at(x, y, z), "minecraft:stone");
                }
            }
        }
    }

    impl Level for Grid {
        fn get(&self, p: BlockPos) -> Option<u16> {
            if self.unloaded.contains(&p) {
                None
            } else {
                Some(self.at(p))
            }
        }
        fn set(&mut self, p: BlockPos, state: u16) {
            self.cells.insert(p, state);
            self.sets.push((p, state));
        }
        fn level_event(&mut self, p: BlockPos, _event: i32) {
            self.fizz.push(p);
        }
        fn drop_loot(&mut self, p: BlockPos, state: u16) {
            self.drops.push((p, state));
        }
        fn rand(&mut self) -> f64 {
            self.rnd
        }
    }

    #[test]
    fn liquid_blocks_have_only_level() {
        init();
        let b = &rine_data::get().blocks;
        for name in ["minecraft:water", "minecraft:lava"] {
            let bi = b.block(name).unwrap();
            assert_eq!(bi.props.len(), 1, "{name}");
            assert_eq!(bi.props[0].name, "level");
            assert_eq!(bi.props[0].values.len(), 16);
        }
    }

    #[test]
    fn fluid_state_round_trips() {
        init();
        for kind in [Kind::Water, Kind::Lava] {
            for lv in 0..=8usize {
                let s = tables().level_state[kind.index()][lv];
                assert_eq!(fluid_state(fluid_of(s)), s, "{kind:?} level {lv}");
            }
            // levels 9..15 are falling water as well
            let falling = tables().level_state[kind.index()][12];
            assert_eq!(fluid_of(falling), Some(kind.flowing(8, true)));
        }
    }

    #[test]
    fn two_sources_make_an_infinite_source_only_for_water() {
        init();
        let mut g = Grid::default();
        g.floor(-2, 2, -1, 1, 0);
        g.put(at(-1, 1, 0), "minecraft:water[level=0]");
        g.put(at(1, 1, 0), "minecraft:water[level=0]");
        assert_eq!(new_liquid(&mut g, Kind::Water, at(0, 1, 0), AIR), Some(Kind::Water.source()));

        let mut g = Grid::default();
        g.floor(-2, 2, -1, 1, 0);
        g.put(at(-1, 1, 0), "minecraft:lava[level=0]");
        g.put(at(1, 1, 0), "minecraft:lava[level=0]");
        assert_eq!(new_liquid(&mut g, Kind::Lava, at(0, 1, 0), AIR), Some(Kind::Lava.flowing(6, false)));
    }

    #[test]
    fn water_decays_one_per_block_and_spreads_both_ways() {
        init();
        let mut g = Grid::default();
        g.floor(-8, 8, -8, 8, 0);
        g.put(at(0, 1, 0), "minecraft:water");
        tick_block(&mut g, Kind::Water, at(0, 1, 0));
        for p in [at(1, 1, 0), at(-1, 1, 0), at(0, 1, 1), at(0, 1, -1)] {
            assert_eq!(g.fluid(p), Some(Kind::Water.flowing(7, false)), "{p:?}");
        }
        tick_block(&mut g, Kind::Water, at(1, 1, 0));
        assert_eq!(g.fluid(at(2, 1, 0)), Some(Kind::Water.flowing(6, false)));
    }

    #[test]
    fn lava_decays_two_per_block() {
        init();
        let mut g = Grid::default();
        g.floor(-8, 8, -8, 8, 0);
        g.put(at(0, 1, 0), "minecraft:lava");
        tick_block(&mut g, Kind::Lava, at(0, 1, 0));
        assert_eq!(g.fluid(at(1, 1, 0)), Some(Kind::Lava.flowing(6, false)));
        tick_block(&mut g, Kind::Lava, at(1, 1, 0));
        assert_eq!(g.fluid(at(2, 1, 0)), Some(Kind::Lava.flowing(4, false)));
    }

    #[test]
    fn falling_water_is_level_eight() {
        init();
        let mut g = Grid::default();
        g.floor(-2, 2, -2, 2, 0);
        g.put(at(0, 2, 0), "minecraft:water");
        tick_block(&mut g, Kind::Water, at(0, 2, 0));
        assert_eq!(g.at(at(0, 1, 0)), fluid_state(Some(Kind::Water.flowing(8, true))));
        assert_eq!(g.at(at(0, 1, 0)), tables().level_state[0][8]);
    }

    #[test]
    fn slope_prefers_the_nearest_drop() {
        init();
        let mut g = Grid::default();
        // stone floor everywhere except the column x=3, which is a drop
        g.floor(-6, 2, -8, 8, 0);
        g.floor(4, 8, -8, 8, 0);
        g.put(at(0, 1, 0), "minecraft:water");
        tick_block(&mut g, Kind::Water, at(0, 1, 0));
        // the drop is two blocks east; north and south need three
        assert!(g.fluid(at(1, 1, 0)).is_some());
        assert!(g.fluid(at(-1, 1, 0)).is_none());
        assert!(g.fluid(at(0, 1, -1)).is_none());
    }

    #[test]
    #[test]
    fn water_washes_grass_and_drops_it() {
        init();
        let mut g = Grid::default();
        g.floor(-8, 8, -8, 8, 0);
        g.put(at(1, 1, 0), "minecraft:short_grass");
        g.put(at(0, 1, 0), "minecraft:water");
        tick_block(&mut g, Kind::Water, at(0, 1, 0));
        assert_eq!(g.fluid(at(1, 1, 0)), Some(Kind::Water.flowing(7, false)));
        assert_eq!(g.drops, vec![(at(1, 1, 0), st("minecraft:short_grass"))]);
    }

    #[test]
    fn doors_block_the_flow() {
        init();
        let mut g = Grid::default();
        g.floor(-8, 8, -8, 8, 0);
        g.put(at(1, 1, 0), "minecraft:oak_door[half=lower]");
        g.put(at(0, 1, 0), "minecraft:water");
        tick_block(&mut g, Kind::Water, at(0, 1, 0));
        assert_eq!(g.at(at(1, 1, 0)), st("minecraft:oak_door[half=lower]"));
        assert!(g.drops.is_empty());
    }

    #[test]
    fn waterlogged_slab_does_not_take_flowing_water() {
        init();
        let mut g = Grid::default();
        g.floor(-8, 8, -8, 8, 0);
        g.put(at(0, 1, 0), "minecraft:oak_slab[type=bottom,waterlogged=true]");
        g.put(at(0, 2, 0), "minecraft:water[level=0]");
        let slab = g.at(at(0, 1, 0));
        tick_block(&mut g, Kind::Water, at(0, 2, 0));
        assert_eq!(g.at(at(0, 1, 0)), slab);
    }

    #[test]
    fn lava_touching_water_turns_to_cobblestone_or_obsidian() {
        init();
        let mut g = Grid::default();
        g.floor(-2, 2, -2, 2, 0);
        g.put(at(0, 1, 0), "minecraft:lava");
        g.put(at(1, 1, 0), "minecraft:water");
        let here = g.at(at(0, 1, 0));
        assert!(!lava_may_spread(&mut g, at(0, 1, 0), here));
        assert_eq!(g.at(at(0, 1, 0)), st("minecraft:obsidian"));
        assert_eq!(g.fizz, vec![at(0, 1, 0)]);

        let mut g = Grid::default();
        g.put(at(0, 1, 0), "minecraft:lava[level=3]");
        g.put(at(0, 1, 1), "minecraft:water");
        let here = g.at(at(0, 1, 0));
        assert!(!lava_may_spread(&mut g, at(0, 1, 0), here));
        assert_eq!(g.at(at(0, 1, 0)), st("minecraft:cobblestone"));
    }

    #[test]
    fn lava_falling_onto_water_makes_stone() {
        init();
        let mut g = Grid::default();
        g.floor(-2, 2, -2, 2, 0);
        g.put(at(0, 2, 0), "minecraft:lava");
        g.put(at(0, 1, 0), "minecraft:water[level=1]");
        tick_block(&mut g, Kind::Lava, at(0, 2, 0));
        assert_eq!(g.at(at(0, 1, 0)), st("minecraft:stone"));
        assert!(g.fizz.contains(&at(0, 1, 0)));
    }

    #[test]
    fn unloaded_neighbours_are_not_entered() {
        init();
        let mut g = Grid::default();
        g.floor(-2, 2, -2, 2, 0);
        g.unloaded.insert(at(1, 1, 0));
        g.put(at(0, 1, 0), "minecraft:water");
        tick_block(&mut g, Kind::Water, at(0, 1, 0));
        assert!(g.fluid(at(1, 1, 0)).is_none());
        assert!(g.sets.iter().all(|(p, _)| *p != at(1, 1, 0)));
    }

    #[test]
    fn scheduler_dedups_and_carries_over() {
        let mut s = Sched::default();
        s.schedule(at(0, 0, 0), Kind::Water, 10);
        s.schedule(at(0, 0, 0), Kind::Water, 10);
        s.schedule(at(1, 0, 0), Kind::Lava, 5);
        s.schedule(at(2, 0, 0), Kind::Water, 50);
        assert_eq!(s.len(), 3);
        assert!(s.pop_due(4, 10).is_empty());
        let first = s.pop_due(10, 1);
        assert_eq!(first, vec![(at(1, 0, 0), Kind::Lava)]);
        let second = s.pop_due(10, 10);
        assert_eq!(second, vec![(at(0, 0, 0), Kind::Water)]);
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn raycast_hits_the_first_solid_face() {
        init();
        let get = |p: BlockPos| Some(if p == at(0, 0, 0) { st("minecraft:stone") } else { AIR });
        let hit = raycast_with(get, [0.5, 3.5, 0.5], [0.0, -1.0, 0.0], REACH, false).unwrap();
        assert_eq!(hit.pos, at(0, 0, 0));
        assert_eq!(hit.normal, [0, 1, 0]);
        assert!(raycast_with(get, [0.5, 9.5, 0.5], [0.0, -1.0, 0.0], REACH, false).is_none());
    }

    #[test]
    fn raycast_sees_source_fluid_only_in_source_mode() {
        init();
        let get = |p: BlockPos| Some(if p == at(0, 0, 0) { st("minecraft:water[level=0]") } else if p == at(0, 1, 0) { st("minecraft:water[level=2]") } else { AIR });
        let src = raycast_with(get, [0.5, 2.5, 0.5], [0.0, -1.0, 0.0], REACH, true).unwrap();
        assert_eq!(src.pos, at(0, 0, 0));
        let filled = raycast_with(get, [0.5, 2.5, 0.5], [0.0, -1.0, 0.0], REACH, false);
        assert!(filled.is_none());
    }
}
