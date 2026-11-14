//! Headless TNT benchmark. Drives the real game loop without sockets. Joins
//! are queued on the new-player channel and leaves flip the client's alive
//! flag, so both run inside `do_tick` exactly as on the server. Viewers are
//! fake clients whose outbound queue is drained by a thread.
//!
//! Run (release, prints percentiles):
//!   cargo test --release -p rine-server bench_tnt -- --ignored --nocapture
//!
//! Environment:
//!   RINE_BENCH_TNT        primed TNT in a dense cube (default 1000)
//!   RINE_BENCH_VIEWERS    fake clients joining at tick 0 (default 1)
//!   RINE_BENCH_WARM       1: wait for the viewers' chunks before spawning TNT
//!   RINE_BENCH_LEAVE_AT   tick at which every client leaves
//!   RINE_BENCH_REJOIN_AT  tick at which the clients join again (timed)
//!   RINE_BENCH_MAX_TICKS  tick cap (default 100000)
//!   RINE_BENCH_MAX_SECS   wall-clock cap in seconds (default 600)

use super::*;
use crate::config::Config;
use crate::net::{offline_uuid, ClientInfo, NewPlayer, Out, Shared};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering as AO};
use std::sync::Arc;
use std::time::Instant;

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// Counters for one fake client; `alive` is what the server reads to leave.
struct Viewer {
    frames: Arc<AtomicU64>,
    bytes: Arc<AtomicU64>,
    alive: Arc<AtomicBool>,
}

/// A fake client: its outbound channel is drained on a thread, so the game
/// loop never blocks on it. Returns the NewPlayer to queue for `join`.
fn fake_client(name: &str, inbound: crossbeam_channel::Receiver<bytes::BytesMut>) -> (NewPlayer, Viewer) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Out>();
    let frames = Arc::new(AtomicU64::new(0));
    let bytes = Arc::new(AtomicU64::new(0));
    let backlog = Arc::new(AtomicUsize::new(0));
    let alive = Arc::new(AtomicBool::new(true));
    let (f2, b2, bl2) = (frames.clone(), bytes.clone(), backlog.clone());
    // the "socket": reads everything at once, like a fast client
    std::thread::spawn(move || {
        while let Some(o) = rx.blocking_recv() {
            if let Out::Batch(v) = o {
                let n: usize = v.iter().map(|b| b.len()).sum();
                f2.fetch_add(v.len() as u64, AO::Relaxed);
                b2.fetch_add(n as u64, AO::Relaxed);
                bl2.fetch_sub(n, AO::Relaxed);
            }
        }
    });
    let np = NewPlayer {
        uuid: offline_uuid(name),
        name: name.to_string(),
        properties: Vec::new(),
        info: ClientInfo { view_distance: 10, ..Default::default() },
        addr: "127.0.0.1:1".parse().unwrap(),
        inbound,
        outbound: tx,
        alive: alive.clone(),
        backlog,
    };
    (np, Viewer { frames, bytes, alive })
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i]
}

fn tnt_alive(g: &Game) -> usize {
    g.entities.list.iter().filter(|e| !e.removed && matches!(e.kind, entity::Kind::Tnt { .. })).count()
}

/// A game with the spawn area generated.
fn setup(dir: &std::path::Path) -> Game {
    rine_data::init();
    let config = Config { world_dir: dir.to_string_lossy().into_owned(), plugins_dir: dir.join("plugins").to_string_lossy().into_owned(), ..Config::default() };
    let (new_tx, new_rx) = crossbeam_channel::unbounded();
    let (wake_tx, wake_rx) = crossbeam_channel::bounded::<()>(1);
    let shared = Arc::new(Shared { config, online: AtomicUsize::new(0), new_players: new_tx, wake: wake_tx, config_frames: Vec::new() });
    let running = Arc::new(AtomicBool::new(true));
    let mut game = Game::new(shared, new_rx, wake_rx, running);
    // pre-generate the spawn area (what `run` does before the first join)
    let sc = rine_world::ChunkPos::of_block(game.spawn[0] as i32, game.spawn[2] as i32);
    let r = 8;
    for dx in -r..=r {
        for dz in -r..=r {
            game.world.request(rine_world::ChunkPos::new(sc.x + dx, sc.z + dz));
        }
    }
    while game.world.chunks.len() < ((2 * r + 1) * (2 * r + 1)) as usize {
        let tick = game.tick;
        game.world.collect_loaded(tick);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    game
}

fn temp_world(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rine-{tag}-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Queue a join; it runs at the start of the next `do_tick`, like a real login.
fn queue_join(game: &Game, name: &str) -> Viewer {
    let (rx_tx, rx_rx) = crossbeam_channel::unbounded::<bytes::BytesMut>();
    // keep the sender alive for the life of the client: a closed inbound
    // queue is not a disconnect here, but holding it keeps the bench honest
    std::mem::forget(rx_tx);
    let (np, v) = fake_client(name, rx_rx);
    game.shared.new_players.send(np).expect("join queue");
    v
}

/// Clients acknowledge chunk batches immediately, like a real client.
fn ack_all(game: &mut Game) {
    for p in game.players.iter_mut().flatten() {
        p.unacked = 0;
    }
}

/// Regression: a bulk spawn must not flood a client. One tick sends at most
/// `SPAWN_PER_TICK` entity spawns, and the tracked set stays under `MAX_TRACKED`.
#[test]
fn tracking_is_capped_per_client() {
    let dir = temp_world("cap");
    let mut game = setup(&dir);
    let _v = queue_join(&game, "bench0");
    game.do_tick();
    let center = game.spawn;
    commands::spawn_tnt_cube(&mut game, center, 5000);
    game.do_tick();
    let n1 = game.players.iter().flatten().next().unwrap().tracked.len();
    assert!(n1 > 0, "nothing spawned");
    assert!(n1 <= track::SPAWN_PER_TICK as usize, "one tick spawned {n1}");
    for _ in 0..40 {
        ack_all(&mut game);
        game.do_tick();
        game.pump();
    }
    let n = game.players.iter().flatten().next().unwrap().tracked.len();
    assert!(n <= track::MAX_TRACKED, "tracked {n}");
    assert!(n > n1, "tracking did not grow over time");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Regression: entities at the cap must not hide players from each other.
/// Two clients 4 chunks apart, 5000 TNT near the spawn point, they must see
/// each other while the entity cap is full.
#[test]
fn players_track_each_other_under_entity_load() {
    let dir = temp_world("players");
    let mut game = setup(&dir);
    let _a = queue_join(&game, "bench0");
    let _b = queue_join(&game, "bench1");
    game.do_tick();
    assert_eq!(game.players.iter().flatten().count(), 2, "both clients joined");
    let center = game.spawn;
    // move the second client 64 blocks east (4 chunks)
    if let Some(p) = game.players.iter_mut().flatten().nth(1) {
        p.teleport(center[0] + 64.0, center[1], center[2], 0.0, 0.0);
    }
    commands::spawn_tnt_cube(&mut game, center, 5000);
    let (ea, eb) = {
        let mut it = game.players.iter().flatten();
        let a = it.next().unwrap().eid;
        let b = it.next().unwrap().eid;
        (a, b)
    };
    let mut seen = false;
    for _ in 0..600 {
        ack_all(&mut game);
        game.do_tick();
        game.pump();
        let a_sees_b = game.players.iter().flatten().any(|p| p.eid == ea && p.tracked.contains(&eb));
        let b_sees_a = game.players.iter().flatten().any(|p| p.eid == eb && p.tracked.contains(&ea));
        if a_sees_b && b_sees_a {
            seen = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(seen, "players do not see each other while the entity cap is full");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore]
fn bench_tnt() {
    let n_tnt = env_usize("RINE_BENCH_TNT", 1000);
    let viewers = env_usize("RINE_BENCH_VIEWERS", 1);
    let max_ticks = env_usize("RINE_BENCH_MAX_TICKS", 100_000) as u64;
    let max_secs = env_usize("RINE_BENCH_MAX_SECS", 600) as u64;
    let leave_at = env_usize("RINE_BENCH_LEAVE_AT", usize::MAX) as u64;
    let rejoin_at = env_usize("RINE_BENCH_REJOIN_AT", usize::MAX) as u64;

    let dir = temp_world("bench");
    let t = Instant::now();
    let mut game = setup(&dir);
    println!("spawn area generated in {:?}", t.elapsed());

    let mut view: Vec<Viewer> = Vec::new();
    for i in 0..viewers {
        view.push(queue_join(&game, &format!("bench{i}")));
    }
    game.do_tick();
    if env_usize("RINE_BENCH_WARM", 0) == 1 {
        // wait until the viewers' chunk streaming has finished generating
        let mut last = 0;
        let mut stable = 0;
        let t = Instant::now();
        while stable < 40 && t.elapsed().as_secs() < 120 {
            ack_all(&mut game);
            let tick = game.tick;
            game.world.collect_loaded(tick);
            game.do_tick();
            if game.world.chunks.len() == last {
                stable += 1;
            } else {
                stable = 0;
                last = game.world.chunks.len();
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        println!("warm: {} chunks in {:?}", game.world.chunks.len(), t.elapsed());
    }

    let center = game.spawn;
    let t = Instant::now();
    let spawned = commands::spawn_tnt_cube(&mut game, center, n_tnt);
    println!("spawn {spawned} TNT: {:?}", t.elapsed());

    let mut tick_ms: Vec<f64> = Vec::new();
    let mut pump_ms: Vec<f64> = Vec::new();
    let mut worst: (f64, u64) = (0.0, 0);
    let mut join_ms = 0.0f64;
    let mut leave_ms = 0.0f64;
    let wall = Instant::now();
    let mut ticks = 0u64;
    let mut last_frames = vec![0u64; viewers];
    let mut max_frames_tick = vec![0u64; viewers];
    let mut capped = false;
    while ticks < max_ticks {
        if ticks == leave_at {
            for v in &view {
                v.alive.store(false, AO::Relaxed);
            }
        }
        if ticks == rejoin_at {
            for i in 0..viewers {
                view.push(queue_join(&game, &format!("bench{i}r")));
                last_frames.push(0);
                max_frames_tick.push(0);
            }
        }
        ack_all(&mut game);
        let players_before = game.players.iter().flatten().count();
        let t = Instant::now();
        game.do_tick();
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let players_after = game.players.iter().flatten().count();
        if ticks == leave_at {
            leave_ms = ms;
        }
        if ticks == rejoin_at {
            join_ms = ms;
        }
        // a join or a leave is a tick with a changed player count; keep them
        // out of the steady-state percentiles, they are reported on their own
        let is_event = players_before != players_after;
        if !is_event {
            tick_ms.push(ms);
        }
        if ms > worst.0 {
            worst = (ms, game.tick);
        }
        for (i, v) in view.iter().enumerate() {
            let f = v.frames.load(AO::Relaxed);
            let delta = f - last_frames[i];
            last_frames[i] = f;
            max_frames_tick[i] = max_frames_tick[i].max(delta);
        }
        let t = Instant::now();
        game.pump();
        pump_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        ticks += 1;
        if wall.elapsed().as_secs() >= max_secs {
            capped = true;
            break;
        }
        let events_done = ticks > leave_at.saturating_add(0).max(rejoin_at.min(u64::MAX)) || (leave_at == u64::MAX && rejoin_at == u64::MAX);
        let done = tnt_alive(&game) == 0 && game.pending_blasts.is_empty() && ticks > 100 && events_done && (rejoin_at == u64::MAX || ticks > rejoin_at + 100);
        if done {
            break;
        }
    }
    let total = wall.elapsed();
    let mut sorted = tick_ms.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut psorted = pump_ms.clone();
    psorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let over50 = tick_ms.iter().filter(|&&m| m > 50.0).count();
    if leave_at != u64::MAX {
        println!("leave tick {leave_at}: {leave_ms:.1} ms");
    }
    if rejoin_at != u64::MAX {
        println!("join tick {rejoin_at}: {join_ms:.1} ms");
    }
    println!(
        "RESULT tnt={spawned} viewers={viewers} ticks={ticks} capped={capped} wall={:.2}s tick_ms p50={:.2} p90={:.2} p99={:.2} max={:.1} (tick {}) over50ms={over50} pump_ms p50={:.2} p99={:.2} max={:.1}",
        total.as_secs_f64(),
        percentile(&sorted, 0.5),
        percentile(&sorted, 0.9),
        percentile(&sorted, 0.99),
        worst.0,
        worst.1,
        percentile(&psorted, 0.5),
        percentile(&psorted, 0.99),
        psorted.last().copied().unwrap_or(0.0),
    );
    for (i, v) in view.iter().enumerate() {
        println!("viewer {i}: frames {} bytes {} max_frames_in_tick {}", v.frames.load(AO::Relaxed), v.bytes.load(AO::Relaxed), max_frames_tick[i]);
    }
    println!("final: entities {} pending_blasts {} chunks {}", game.entities.list.len(), game.pending_blasts.len(), game.world.chunks.len());
    let _ = std::fs::remove_dir_all(&dir);
}
