//! Engine tests on an in-memory world.

use super::data::*;
use super::engine::{self, Lvl, Rs};
use ahash::AHashMap;
use rine_protocol::BlockPos;

fn st(s: &str) -> u16 {
    rine_data::init();
    rine_data::get().blocks.parse_state(s).unwrap_or_else(|| panic!("state {s}"))
}

fn at(x: i32, y: i32, z: i32) -> BlockPos {
    BlockPos::new(x, y, z)
}

/// Flat in-memory level: every position is loaded, unset positions are air.
#[derive(Default)]
struct Grid {
    cells: AHashMap<BlockPos, u16>,
    time: u64,
    events: Vec<(i32, BlockPos)>,
    primed: Vec<BlockPos>,
    /// Block id with test hooks and the order its neighbor hook ran.
    hook_block: u16,
    hook_log: Vec<BlockPos>,
    tick_log: Vec<BlockPos>,
    /// When set, the first hooked block to update also updates around this pos.
    nested: Option<BlockPos>,
}

impl Lvl for Grid {
    fn get(&self, p: BlockPos) -> u16 {
        self.cells.get(&p).copied().unwrap_or(0)
    }
    fn loaded(&self, _p: BlockPos) -> bool {
        true
    }
    fn raw_set(&mut self, p: BlockPos, s: u16) -> u16 {
        self.cells.insert(p, s).unwrap_or(0)
    }
    fn time(&self) -> u64 {
        self.time
    }
    fn day_time(&self) -> i64 {
        6000
    }
    fn sky_light(&self, _p: BlockPos) -> u8 {
        15
    }
    fn level_event(&mut self, ev: i32, p: BlockPos, _data: i32) {
        self.events.push((ev, p));
    }
    fn sound(&mut self, _n: &str, _p: BlockPos, _v: f32, _pi: f32, _c: i32, _s: bool) {}
    fn block_event_packet(&mut self, _p: BlockPos, _b: u16, _a: u8, _bb: u8) {}
    fn drop_block(&mut self, _p: BlockPos, _st: u16) {}
    fn prime_tnt(&mut self, p: BlockPos) {
        self.primed.push(p);
    }
    fn count_entities(&self, _min: [f64; 3], _max: [f64; 3], _living: bool) -> usize {
        0
    }
    fn container_signal(&self, _p: BlockPos, _st: u16) -> Option<u8> {
        None
    }
    fn chest_viewers(&self, _p: BlockPos) -> u8 {
        0
    }
    fn rand(&mut self) -> f32 {
        0.5
    }
    fn has_hooks(&self, b: u16) -> bool {
        b != 0 && b == self.hook_block
    }
    fn hook_tick(&mut self, _rs: &mut Rs, p: BlockPos, _st: u16) -> bool {
        self.tick_log.push(p);
        true
    }
    fn hook_neighbor(&mut self, rs: &mut Rs, p: BlockPos, _st: u16) -> bool {
        self.hook_log.push(p);
        if let Some(n) = self.nested.take() {
            engine::update_neighbors_at(rs, self, n, 0);
        }
        true
    }
}

impl Grid {
    fn new() -> Self {
        rine_data::init();
        Self::default()
    }

    /// Fill a floor of stone at y = -1 over the given x/z range.
    fn floor(&mut self, x0: i32, x1: i32, z0: i32, z1: i32) {
        let stone = st("stone");
        for x in x0..=x1 {
            for z in z0..=z1 {
                self.cells.insert(at(x, -1, z), stone);
            }
        }
    }
}

/// A player placing a block: placement state, write, then the queued side
/// effects (`external_set`) and `setPlacedBy`.
fn place(rs: &mut Rs, l: &mut Grid, p: BlockPos, s: &str) -> u16 {
    let s = engine::placement_state(rs, l, p, st(s));
    let old = l.raw_set(p, s);
    engine::external_set(rs, l, p, old, s);
    engine::placed_by(rs, l, p);
    l.get(p)
}

/// One game tick: block ticks, block events, block entities.
fn tick(rs: &mut Rs, l: &mut Grid) {
    l.time += 1;
    engine::run_block_ticks(rs, l);
    engine::run_block_events(rs, l);
    engine::tick_block_entities(rs, l);
}

fn ticks(rs: &mut Rs, l: &mut Grid, n: u32) {
    for _ in 0..n {
        tick(rs, l);
    }
}

fn use_at(rs: &mut Rs, l: &mut Grid, p: BlockPos) {
    let s = l.get(p);
    assert!(engine::use_block(rs, l, p, s, NORTH), "use {}", block_name(s));
}

fn power(l: &Grid, p: BlockPos) -> u8 {
    get(l.get(p), P::Power)
}

#[test]
fn java_hash_set_order_matches_java() {
    // expected orders printed by java.util.HashSet with Vec3i.hashCode
    let cases: [((i32, i32, i32), [(i32, i32, i32); 7]); 3] = [
        ((0, 64, 0), [(0, 64, 0), (0, 63, 0), (0, 64, 1), (1, 64, 0), (0, 65, 0), (0, 64, -1), (-1, 64, 0)]),
        ((10, 64, -7), [(10, 63, -7), (10, 64, -6), (11, 64, -7), (10, 64, -7), (10, 65, -7), (10, 64, -8), (9, 64, -7)]),
        ((-123, 5, 456), [(-123, 6, 456), (-123, 5, 455), (-124, 5, 456), (-123, 5, 456), (-123, 4, 456), (-123, 5, 457), (-122, 5, 456)]),
    ];
    for ((x, y, z), want) in cases {
        let p = at(x, y, z);
        let mut set7 = [p, p.relative(DOWN), p.relative(UP), p.relative(NORTH), p.relative(SOUTH), p.relative(WEST), p.relative(EAST)];
        engine::java_hash_order(&mut set7, 16);
        let got: Vec<(i32, i32, i32)> = set7.iter().map(|p| (p.x, p.y, p.z)).collect();
        assert_eq!(got, want.to_vec(), "around {p:?}");
    }
    assert_eq!(engine::java_table_size(7), 16);
    assert_eq!(engine::java_table_size(13), 32);
}

#[test]
fn update_neighbors_order_and_nesting() {
    let mut l = Grid::new();
    let mut rs = Rs::default();
    let cb = st("command_block");
    l.hook_block = block(cb);
    let c = at(0, 0, 0);
    for d in ALL {
        l.cells.insert(c.relative(d), cb);
    }
    engine::update_neighbors_at(&mut rs, &mut l, c, 0);
    let want: Vec<BlockPos> = UPDATE_ORDER.iter().map(|&d| c.relative(d)).collect();
    assert_eq!(l.hook_log, want, "W, E, D, U, N, S");

    // An update queued while one runs goes deeper: the six updates around
    // the west neighbor's own source run before the rest of the first batch.
    l.hook_log.clear();
    let far = at(10, 0, 0);
    for d in ALL {
        l.cells.insert(far.relative(d), cb);
    }
    l.nested = Some(far);
    engine::update_neighbors_at(&mut rs, &mut l, c, 0);
    let mut want = vec![c.relative(WEST)];
    want.extend(UPDATE_ORDER.iter().map(|&d| far.relative(d)));
    want.extend(UPDATE_ORDER[1..].iter().map(|&d| c.relative(d)));
    assert_eq!(l.hook_log, want);
}

#[test]
fn lever_powers_wire_line_with_decay() {
    let mut l = Grid::new();
    let mut rs = Rs::default();
    l.floor(-1, 20, -1, 1);
    let lever = at(0, 0, 0);
    place(&mut rs, &mut l, lever, "lever[face=floor,facing=east]");
    for x in 1..=17 {
        place(&mut rs, &mut l, at(x, 0, 0), "redstone_wire");
    }
    // a straight line connects east-west
    let w = l.get(at(5, 0, 0));
    assert_eq!(get(w, P::East), SIDE_SIDE);
    assert_eq!(get(w, P::West), SIDE_SIDE);
    assert_eq!(power(&l, at(1, 0, 0)), 0);

    use_at(&mut rs, &mut l, lever);
    assert!(on(l.get(lever), P::Powered));
    for x in 1..=17 {
        let want = 16i32 - x;
        assert_eq!(power(&l, at(x, 0, 0)) as i32, want.max(0), "wire at x={x}");
    }
    // wire and lever strongly power the block they stand on, which then
    // powers the blocks around it
    assert_eq!(engine::direct_signal_to(&rs, &l, at(1, -1, 0)), 15);
    assert_eq!(engine::direct_signal_to(&rs, &l, at(0, -1, 0)), 15);
    assert!(engine::has_neighbor_signal(&rs, &l, at(1, -2, 0)));
    assert!(!engine::has_neighbor_signal(&rs, &l, at(1, -3, 0)));

    use_at(&mut rs, &mut l, lever);
    for x in 1..=17 {
        assert_eq!(power(&l, at(x, 0, 0)), 0, "wire at x={x} after off");
    }
}

#[test]
fn repeater_delays_and_locks() {
    let mut l = Grid::new();
    let mut rs = Rs::default();
    l.floor(-1, 6, -2, 2);
    let lever = at(0, 0, 0);
    place(&mut rs, &mut l, lever, "lever[face=floor,facing=east]");
    // facing points at the input: west input, output east
    let rep = at(1, 0, 0);
    place(&mut rs, &mut l, rep, "repeater[facing=west,delay=2]");
    let wire = at(2, 0, 0);
    place(&mut rs, &mut l, wire, "redstone_wire");

    use_at(&mut rs, &mut l, lever);
    assert!(!on(l.get(rep), P::Powered));
    ticks(&mut rs, &mut l, 3);
    assert!(!on(l.get(rep), P::Powered), "delay 2 = 4 game ticks");
    tick(&mut rs, &mut l);
    assert!(on(l.get(rep), P::Powered));
    assert_eq!(power(&l, wire), 15, "repeater outputs 15");

    // a powered repeater pointing into the side locks it
    let side_lever = at(1, 0, -2);
    place(&mut rs, &mut l, side_lever, "lever[face=floor,facing=south]");
    place(&mut rs, &mut l, at(1, 0, -1), "repeater[facing=north,delay=1]");
    use_at(&mut rs, &mut l, side_lever);
    ticks(&mut rs, &mut l, 2);
    assert!(on(l.get(at(1, 0, -1)), P::Powered));
    assert!(on(l.get(rep), P::Locked));
    use_at(&mut rs, &mut l, lever);
    ticks(&mut rs, &mut l, 10);
    assert!(on(l.get(rep), P::Powered), "locked repeater keeps its output");
}

#[test]
fn torch_inverts_with_two_tick_delay() {
    let mut l = Grid::new();
    let mut rs = Rs::default();
    l.floor(-2, 4, -2, 2);
    let blockp = at(1, 0, 0);
    l.cells.insert(blockp, st("stone"));
    let torch = at(2, 0, 0);
    place(&mut rs, &mut l, torch, "redstone_wall_torch[facing=east]");
    assert!(on(l.get(torch), P::Lit));
    let lever = at(1, 1, 0);
    place(&mut rs, &mut l, lever, "lever[face=floor,facing=east]");
    use_at(&mut rs, &mut l, lever);
    tick(&mut rs, &mut l);
    assert!(on(l.get(torch), P::Lit));
    tick(&mut rs, &mut l);
    assert!(!on(l.get(torch), P::Lit), "torch turns off after 2 ticks");
    use_at(&mut rs, &mut l, lever);
    ticks(&mut rs, &mut l, 2);
    assert!(on(l.get(torch), P::Lit));
}

#[test]
fn torch_burns_out_on_fast_clock() {
    let mut l = Grid::new();
    let mut rs = Rs::default();
    l.floor(-2, 4, -2, 2);
    // vanilla burns a torch out after 8 turn-offs within 60 ticks: flip the
    // power of the block under it 20 times quickly with a lever on its side.
    let base = at(0, 0, 0);
    l.cells.insert(base, st("stone"));
    let torch = at(0, 1, 0);
    l.raw_set(torch, st("redstone_torch"));
    let lever = at(1, 0, 0);
    place(&mut rs, &mut l, lever, "lever[face=wall,facing=east]");
    for _ in 0..20 {
        use_at(&mut rs, &mut l, lever);
        ticks(&mut rs, &mut l, 3);
    }
    assert!(l.events.iter().any(|(e, p)| *e == 1502 && *p == torch), "burnout event");
    assert!(!on(l.get(torch), P::Lit));
}

#[test]
fn lamp_turns_off_after_four_ticks() {
    let mut l = Grid::new();
    let mut rs = Rs::default();
    l.floor(-2, 4, -2, 2);
    let lamp = at(1, 0, 0);
    place(&mut rs, &mut l, lamp, "redstone_lamp");
    let lever = at(0, 0, 0);
    place(&mut rs, &mut l, lever, "lever[face=floor,facing=east]");
    use_at(&mut rs, &mut l, lever);
    assert!(on(l.get(lamp), P::Lit), "lamp turns on at once");
    use_at(&mut rs, &mut l, lever);
    assert!(on(l.get(lamp), P::Lit));
    ticks(&mut rs, &mut l, 3);
    assert!(on(l.get(lamp), P::Lit));
    tick(&mut rs, &mut l);
    assert!(!on(l.get(lamp), P::Lit));
}

#[test]
fn button_releases_after_twenty_ticks() {
    let mut l = Grid::new();
    let mut rs = Rs::default();
    l.floor(-2, 4, -2, 2);
    let button = at(0, 0, 0);
    place(&mut rs, &mut l, button, "stone_button[face=floor,facing=east]");
    let wire = at(1, 0, 0);
    place(&mut rs, &mut l, wire, "redstone_wire");
    use_at(&mut rs, &mut l, button);
    assert_eq!(power(&l, wire), 15);
    ticks(&mut rs, &mut l, 19);
    assert_eq!(power(&l, wire), 15);
    tick(&mut rs, &mut l);
    assert_eq!(power(&l, wire), 0);
}

#[test]
fn piston_pushes_block_and_retracts() {
    let mut l = Grid::new();
    let mut rs = Rs::default();
    l.floor(-2, 6, -2, 2);
    let piston = at(0, 0, 0);
    place(&mut rs, &mut l, piston, "sticky_piston[facing=east]");
    let pushed = at(1, 0, 0);
    l.cells.insert(pushed, st("cobblestone"));
    let lever = at(-1, 0, 0);
    place(&mut rs, &mut l, lever, "lever[face=floor,facing=west]");

    use_at(&mut rs, &mut l, lever);
    tick(&mut rs, &mut l);
    // after the block event: head is moving, the block is moving
    ticks(&mut rs, &mut l, 3);
    assert!(on(l.get(piston), P::Extended));
    assert_eq!(block_name(l.get(at(1, 0, 0))), "piston_head");
    assert_eq!(block_name(l.get(at(2, 0, 0))), "cobblestone");

    use_at(&mut rs, &mut l, lever);
    ticks(&mut rs, &mut l, 4);
    assert!(!on(l.get(piston), P::Extended));
    assert_eq!(block_name(l.get(at(1, 0, 0))), "cobblestone", "sticky piston pulls back");
    assert!(is_air(l.get(at(2, 0, 0))));
}

#[test]
fn observer_pulses_once() {
    let mut l = Grid::new();
    let mut rs = Rs::default();
    l.floor(-2, 4, -2, 2);
    // observer facing west looks at x-1, outputs east (x+1)
    let obs = at(1, 0, 0);
    l.raw_set(obs, st("observer[facing=west]"));
    let wire = at(2, 0, 0);
    place(&mut rs, &mut l, wire, "redstone_wire");
    place(&mut rs, &mut l, at(0, 0, 0), "cobblestone");
    ticks(&mut rs, &mut l, 2);
    assert!(on(l.get(obs), P::Powered));
    assert_eq!(power(&l, wire), 15);
    ticks(&mut rs, &mut l, 2);
    assert!(!on(l.get(obs), P::Powered));
    assert_eq!(power(&l, wire), 0);
}

#[test]
fn tnt_primes_from_signal() {
    let mut l = Grid::new();
    let mut rs = Rs::default();
    l.floor(-2, 4, -2, 2);
    let tnt = at(1, 0, 0);
    place(&mut rs, &mut l, tnt, "tnt");
    place(&mut rs, &mut l, at(0, 0, 0), "redstone_block");
    assert_eq!(l.primed, vec![tnt]);
    assert!(is_air(l.get(tnt)));
}

#[test]
fn comparator_subtract_mode() {
    let mut l = Grid::new();
    let mut rs = Rs::default();
    l.floor(-4, 6, -4, 4);
    // rear input: redstone block behind, side input: wire with power 13
    l.raw_set(at(-1, 0, 0), st("redstone_block"));
    let comp = at(0, 0, 0);
    place(&mut rs, &mut l, comp, "comparator[facing=west]");
    ticks(&mut rs, &mut l, 2);
    assert!(on(l.get(comp), P::Powered));
    let out = at(1, 0, 0);
    place(&mut rs, &mut l, out, "redstone_wire");
    assert_eq!(power(&l, out), 15);

    // side input of strength 13 on the north side
    place(&mut rs, &mut l, at(0, 0, -3), "redstone_block");
    place(&mut rs, &mut l, at(0, 0, -2), "redstone_wire");
    place(&mut rs, &mut l, at(0, 0, -1), "redstone_wire");
    assert_eq!(power(&l, at(0, 0, -1)), 14);
    use_at(&mut rs, &mut l, comp); // subtract mode
    ticks(&mut rs, &mut l, 2);
    assert_eq!(power(&l, out), 1, "15 - 14");
}

#[test]
fn hooks_register_by_name_and_get_ticks() {
    rine_data::init();
    let h = super::BlockHooks::default();
    assert!(super::register_hooks("command_block", h));
    assert!(super::register_hooks("minecraft:repeating_command_block", h));
    assert!(!super::register_hooks("no_such_block", h));

    let mut l = Grid::new();
    let mut rs = Rs::default();
    let cb = st("command_block");
    l.hook_block = block(cb);
    let p = at(3, 0, 3);
    l.cells.insert(p, cb);
    engine::schedule(&mut rs, &l, p, block(cb), 1, engine::prio::NORMAL);
    assert!(engine::has_scheduled(&rs, p, block(cb)));
    tick(&mut rs, &mut l);
    assert_eq!(l.tick_log, vec![p]);
    // a lever next to it notifies the hook through neighborChanged
    l.floor(0, 6, 0, 6);
    place(&mut rs, &mut l, at(4, 0, 3), "lever[face=floor,facing=east]");
    l.hook_log.clear();
    use_at(&mut rs, &mut l, at(4, 0, 3));
    assert!(l.hook_log.contains(&p));
    assert!(engine::has_neighbor_signal(&rs, &l, p));
}

#[test]
fn door_halves_open_together() {
    let mut l = Grid::new();
    let mut rs = Rs::default();
    l.floor(-2, 4, -2, 2);
    let low = at(0, 0, 0);
    let up = at(0, 1, 0);
    l.raw_set(low, st("oak_door[half=lower,facing=north]"));
    l.raw_set(up, st("oak_door[half=upper,facing=north]"));
    use_at(&mut rs, &mut l, low);
    assert!(on(l.get(low), P::Open));
    assert!(on(l.get(up), P::Open), "upper half follows");
    use_at(&mut rs, &mut l, up);
    assert!(!on(l.get(low), P::Open));
    assert!(!on(l.get(up), P::Open));

    // iron door from a lever: opens with the signal, both halves
    let ilow = at(2, 0, 0);
    let iup = at(2, 1, 0);
    l.raw_set(ilow, st("iron_door[half=lower,facing=north]"));
    l.raw_set(iup, st("iron_door[half=upper,facing=north]"));
    let lever = at(3, 0, 0);
    place(&mut rs, &mut l, lever, "lever[face=floor,facing=east]");
    let s = l.get(ilow);
    assert!(!engine::use_block(&mut rs, &mut l, ilow, s, NORTH), "iron door ignores clicks");
    use_at(&mut rs, &mut l, lever);
    assert!(on(l.get(ilow), P::Open) && on(l.get(ilow), P::Powered));
    assert!(on(l.get(iup), P::Open) && on(l.get(iup), P::Powered));
    use_at(&mut rs, &mut l, lever);
    assert!(!on(l.get(ilow), P::Open));
    assert!(!on(l.get(iup), P::Open));
}

#[test]
fn ticks_in_unloaded_chunks_wait_for_load() {
    let mut l = Grid::new();
    let mut rs = Rs::default();
    l.floor(-2, 4, -2, 2);
    let button = at(0, 0, 0);
    place(&mut rs, &mut l, button, "stone_button[face=floor,facing=east]");
    use_at(&mut rs, &mut l, button);
    let mut gone = Unloaded { inner: &mut l };
    for _ in 0..40 {
        gone.inner.time += 1;
        engine::run_block_ticks(&mut rs, &mut gone);
    }
    assert!(on(l.get(button), P::Powered), "no tick while unloaded");
    rs.chunk_loaded(0, 0, l.time);
    tick(&mut rs, &mut l);
    assert!(!on(l.get(button), P::Powered), "released after load");
}

/// The grid with every chunk unloaded.
struct Unloaded<'a> {
    inner: &'a mut Grid,
}

impl Lvl for Unloaded<'_> {
    fn get(&self, p: BlockPos) -> u16 {
        self.inner.get(p)
    }
    fn loaded(&self, _p: BlockPos) -> bool {
        false
    }
    fn raw_set(&mut self, p: BlockPos, s: u16) -> u16 {
        self.inner.raw_set(p, s)
    }
    fn time(&self) -> u64 {
        self.inner.time
    }
    fn day_time(&self) -> i64 {
        6000
    }
    fn sky_light(&self, _p: BlockPos) -> u8 {
        15
    }
    fn level_event(&mut self, _ev: i32, _p: BlockPos, _data: i32) {}
    fn sound(&mut self, _n: &str, _p: BlockPos, _v: f32, _pi: f32, _c: i32, _s: bool) {}
    fn block_event_packet(&mut self, _p: BlockPos, _b: u16, _a: u8, _bb: u8) {}
    fn drop_block(&mut self, _p: BlockPos, _st: u16) {}
    fn prime_tnt(&mut self, _p: BlockPos) {}
    fn count_entities(&self, _min: [f64; 3], _max: [f64; 3], _living: bool) -> usize {
        0
    }
    fn container_signal(&self, _p: BlockPos, _st: u16) -> Option<u8> {
        None
    }
    fn chest_viewers(&self, _p: BlockPos) -> u8 {
        0
    }
    fn rand(&mut self) -> f32 {
        0.5
    }
}
