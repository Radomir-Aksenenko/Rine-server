//! Redstone through the real game tick: block changes queued by
//! `World::set_block`, `process`, scheduled ticks, block events, block
//! entity tickers and entities on pressure plates.

use super::data::*;
use crate::config::Config;
use crate::game::{entity, Game};
use crate::net::Shared;
use rine_protocol::BlockPos;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Arc;

fn st(s: &str) -> u16 {
    rine_data::get().blocks.parse_state(s).unwrap_or_else(|| panic!("state {s}"))
}

pub(crate) fn setup(tag: &str) -> Game {
    rine_data::init();
    let dir = std::env::temp_dir().join(format!("rine-rs-{tag}-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let config = Config { world_dir: dir.to_string_lossy().into_owned(), plugins_dir: dir.join("plugins").to_string_lossy().into_owned(), enable_command_block: true, ..Config::default() };
    let (new_tx, new_rx) = crossbeam_channel::unbounded();
    let (wake_tx, wake_rx) = crossbeam_channel::bounded::<()>(1);
    let shared = Arc::new(Shared { config, online: AtomicUsize::new(0), new_players: new_tx, wake: wake_tx, config_frames: Vec::new() });
    let mut g = Game::new(shared, new_rx, wake_rx, Arc::new(AtomicBool::new(true)));
    let sc = rine_world::ChunkPos::of_block(g.spawn[0] as i32, g.spawn[2] as i32);
    let r = 2;
    for dx in -r..=r {
        for dz in -r..=r {
            g.world.request(rine_world::ChunkPos::new(sc.x + dx, sc.z + dz));
        }
    }
    while g.world.chunks.len() < ((2 * r + 1) * (2 * r + 1)) as usize {
        let tick = g.tick;
        g.world.collect_loaded(tick);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    g
}

fn origin(g: &Game) -> BlockPos {
    BlockPos::new(g.spawn[0] as i32 - 8, 200, g.spawn[2] as i32 - 8)
}

fn put(g: &mut Game, p: BlockPos, s: &str) {
    g.world.set_block(p, st(s));
}

fn get_at(g: &Game, p: BlockPos) -> u16 {
    g.world.get_block(p.x, p.y, p.z)
}

fn ticks(g: &mut Game, n: u32) {
    for _ in 0..n {
        g.do_tick();
    }
}

#[test]
fn game_wire_lamp_piston_plate() {
    let mut g = setup("all");
    let o = origin(&g);
    for x in -2..=18 {
        for z in -2..=10 {
            put(&mut g, o.offset(x, -1, z), "stone");
        }
    }
    ticks(&mut g, 1);

    // wire line from a redstone block
    for x in 1..=16 {
        put(&mut g, o.offset(x, 0, 0), "redstone_wire");
    }
    ticks(&mut g, 1);
    put(&mut g, o, "redstone_block");
    ticks(&mut g, 1);
    for x in 1..=16 {
        assert_eq!(get(get_at(&g, o.offset(x, 0, 0)), P::Power) as i32, 16 - x, "wire x={x}");
    }
    // a straight line does not point sideways (wire getSignal checks the
    // connection side); the block under the wire sees it through UP
    assert!(!super::has_neighbor_signal(&g, o.offset(1, 0, 1)));
    assert!(super::has_neighbor_signal(&g, o.offset(1, -1, 0)));

    // lamp next to a redstone block, off again 4 ticks after it goes
    let lamp = o.offset(1, 0, 2);
    put(&mut g, lamp, "redstone_lamp");
    put(&mut g, o.offset(0, 0, 2), "redstone_block");
    ticks(&mut g, 1);
    assert!(on(get_at(&g, lamp), P::Lit));
    put(&mut g, o.offset(0, 0, 2), "air");
    ticks(&mut g, 2);
    assert!(on(get_at(&g, lamp), P::Lit));
    ticks(&mut g, 4);
    assert!(!on(get_at(&g, lamp), P::Lit));

    // piston pushes cobblestone
    let piston = o.offset(1, 0, 4);
    put(&mut g, piston, "piston[facing=east]");
    put(&mut g, o.offset(2, 0, 4), "cobblestone");
    ticks(&mut g, 1);
    put(&mut g, o.offset(0, 0, 4), "redstone_block");
    ticks(&mut g, 5);
    assert!(on(get_at(&g, piston), P::Extended));
    assert_eq!(block_name(get_at(&g, o.offset(2, 0, 4))), "piston_head");
    assert_eq!(block_name(get_at(&g, o.offset(3, 0, 4))), "cobblestone");
    put(&mut g, o.offset(0, 0, 4), "air");
    ticks(&mut g, 5);
    assert!(!on(get_at(&g, piston), P::Extended));
    assert!(is_air(get_at(&g, o.offset(2, 0, 4))), "plain piston leaves the block");

    // an item dropped on a wooden plate presses it; the wire next to it lights
    let plate = o.offset(1, 0, 6);
    put(&mut g, plate, "oak_pressure_plate");
    put(&mut g, o.offset(2, 0, 6), "redstone_wire");
    ticks(&mut g, 1);
    let stick = crate::game::player::Stack { item: crate::game::items::id("stick").unwrap(), count: 1, ..crate::game::player::Stack::EMPTY };
    entity::spawn_item(&mut g, [plate.x as f64 + 0.5, plate.y as f64 + 0.05, plate.z as f64 + 0.5], [0.0, 0.0, 0.0], stick, 0);
    ticks(&mut g, 10);
    assert!(on(get_at(&g, plate), P::Powered), "plate pressed by the item");
    assert_eq!(get(get_at(&g, o.offset(2, 0, 6)), P::Power), 15);

    // a torch turns off 2 ticks after its block becomes a redstone block
    let base = o.offset(1, 0, 8);
    put(&mut g, base, "stone");
    let torch = o.offset(2, 0, 8);
    put(&mut g, torch, "redstone_wall_torch[facing=east]");
    ticks(&mut g, 1);
    assert!(on(get_at(&g, torch), P::Lit));
    put(&mut g, base, "redstone_block");
    ticks(&mut g, 1);
    assert!(on(get_at(&g, torch), P::Lit));
    ticks(&mut g, 2);
    assert!(!on(get_at(&g, torch), P::Lit));
}

#[test]
fn game_command_block_impulse() {
    use crate::game::cmd::cmdblock;
    let mut g = setup("cb");
    let o = origin(&g);
    for x in -2..=4 {
        for z in -2..=4 {
            put(&mut g, o.offset(x, -1, z), "stone");
        }
    }
    let cb = o.offset(1, 0, 1);
    put(&mut g, cb, "command_block[facing=up]");
    ticks(&mut g, 1);
    let p3 = [cb.x, cb.y, cb.z];
    let mut d = cmdblock::read(&g, p3).unwrap_or_default();
    d.command = format!("setblock {} {} {} minecraft:gold_block", cb.x, cb.y + 2, cb.z);
    cmdblock::write(&mut g, p3, &d);
    ticks(&mut g, 2);
    assert!(is_air(get_at(&g, cb.offset(0, 2, 0))), "not run without power");

    // power edge -> one run, one tick later
    put(&mut g, o.offset(0, 0, 1), "redstone_block");
    ticks(&mut g, 3);
    assert_eq!(block_name(get_at(&g, cb.offset(0, 2, 0))), "gold_block");
    assert!(cmdblock::read(&g, p3).unwrap().powered);
    assert_eq!(cmdblock::read(&g, p3).unwrap().success_count, 1);

    // staying powered does not run again
    put(&mut g, cb.offset(0, 2, 0), "air");
    ticks(&mut g, 3);
    assert!(is_air(get_at(&g, cb.offset(0, 2, 0))), "impulse runs once per edge");

    // power off, then on again -> runs again
    put(&mut g, o.offset(0, 0, 1), "air");
    ticks(&mut g, 2);
    assert!(!cmdblock::read(&g, p3).unwrap().powered);
    put(&mut g, o.offset(0, 0, 1), "redstone_block");
    ticks(&mut g, 3);
    assert_eq!(block_name(get_at(&g, cb.offset(0, 2, 0))), "gold_block");
}
