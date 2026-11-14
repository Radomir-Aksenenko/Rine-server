//! The game loop. One thread owns all game state and runs fixed 20 TPS
//! ticks made of phases; heavy per-player and per-entity work inside the
//! phases runs data-parallel on the rayon pool.

pub mod world;
pub mod player;
pub mod items;
pub mod handle;
pub mod track;
pub mod entity;
pub mod commands;
pub mod neighbors;
pub mod interact;
pub mod explosion;
pub mod loot;
pub mod place;
pub mod plugins;
pub mod containers;
pub mod survival;
pub mod actions;
pub mod durability;
pub mod playerdata;
pub mod fluids;
#[cfg(test)]
mod bench;

use crate::net::{NewPlayer, Shared};
use crate::out;
use ahash::AHashMap;
use bytes::Bytes;
use player::Player;
use rine_protocol::text::Text;
use rine_world::ChunkPos;
use std::sync::Arc;
use std::time::{Duration, Instant};
use world::World;

pub const TICK: Duration = Duration::from_millis(50);

const OPS_FILE: &str = "ops.txt";

pub struct Game {
    pub shared: Arc<Shared>,
    /// Operator nicknames: `ops` from rine.toml plus `ops.txt`.
    pub ops: Vec<String>,
    pub world: World,
    pub players: Vec<Option<Player>>,
    pub by_eid: AHashMap<i32, usize>,
    pub next_eid: i32,
    pub tick: u64,
    pub day_time: i64,
    pub spawn: [f64; 3],
    pub new_rx: crossbeam_channel::Receiver<NewPlayer>,
    /// Signalled by the net readers when inbound packets are queued.
    pub wake_rx: crossbeam_channel::Receiver<()>,
    pub entities: entity::Entities,
    /// Explosions waiting to be processed (spread over ticks under load).
    pub pending_blasts: std::collections::VecDeque<[f64; 3]>,
    pub mspt: f64,
    pub peak_ms: f64,
    pub tps: f64,
    /// Frames to send to every player this tick.
    pub broadcast: Vec<Bytes>,
    pub dim_type_id: u32,
    pub clock_ids: Vec<u32>,
    pub running: Arc<std::sync::atomic::AtomicBool>,
    pub plugins: Option<rine_plugin::PluginHost>,
}

impl Game {
    pub fn new(
        shared: Arc<Shared>,
        new_rx: crossbeam_channel::Receiver<NewPlayer>,
        wake_rx: crossbeam_channel::Receiver<()>,
        running: Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        let cfg = &shared.config;
        let world = World::new(cfg.seed, &cfg.world_dir);
        let (sx, sy, sz) = world.generator.find_spawn();
        let regs = &rine_data::get().registries;
        let dim_type_id = regs.synced("minecraft:dimension_type").id("minecraft:overworld").unwrap_or(0);
        let clock_ids = regs.synced("minecraft:world_clock").id("minecraft:overworld").into_iter().collect();
        let mut ops: Vec<String> = cfg.ops.clone();
        if let Ok(t) = std::fs::read_to_string(OPS_FILE) {
            for l in t.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
                if !ops.iter().any(|o| o.eq_ignore_ascii_case(l)) {
                    ops.push(l.to_string());
                }
            }
        }
        Self {
            ops,
            shared,
            world,
            players: Vec::new(),
            by_eid: AHashMap::new(),
            next_eid: 1,
            tick: 0,
            day_time: 1000,
            spawn: [sx as f64 + 0.5, sy as f64, sz as f64 + 0.5],
            new_rx,
            wake_rx,
            entities: entity::Entities::default(),
            pending_blasts: std::collections::VecDeque::new(),
            mspt: 0.0,
            peak_ms: 0.0,
            tps: 20.0,
            broadcast: Vec::new(),
            dim_type_id,
            clock_ids,
            running,
            plugins: None,
        }
    }

    pub fn is_op(&self, name: &str) -> bool {
        self.ops.iter().any(|o| o.eq_ignore_ascii_case(name))
    }

    pub fn save_ops(&self) {
        let mut t = String::from("# operators, one nickname per line\n");
        for o in &self.ops {
            t.push_str(o);
            t.push('\n');
        }
        if let Err(e) = std::fs::write(OPS_FILE, t) {
            tracing::warn!("cannot write {OPS_FILE}: {e}");
        }
    }

    pub fn alloc_eid(&mut self) -> i32 {
        let e = self.next_eid;
        self.next_eid += 1;
        e
    }

    pub fn run(mut self) {
        let pdir = self.shared.config.plugins_dir.clone();
        self.plugins = plugins::start(&pdir);
        // pre-generate the spawn area so the first join is instant
        let sc = ChunkPos::of_block(self.spawn[0] as i32, self.spawn[2] as i32);
        for dx in -6..=6 {
            for dz in -6..=6 {
                self.world.request(ChunkPos::new(sc.x + dx, sc.z + dz));
            }
        }
        let mut next = Instant::now();
        let mut window = std::collections::VecDeque::with_capacity(100);
        while self.running.load(std::sync::atomic::Ordering::Relaxed) {
            let start = Instant::now();
            self.do_tick();
            let took = start.elapsed();
            self.mspt = self.mspt * 0.95 + took.as_secs_f64() * 1000.0 * 0.05;
            self.peak_ms = self.peak_ms.max(took.as_secs_f64() * 1000.0);
            window.push_back(start);
            if window.len() > 100 {
                window.pop_front();
            }
            if window.len() > 1 {
                let span = window.back().unwrap().duration_since(*window.front().unwrap()).as_secs_f64();
                self.tps = ((window.len() - 1) as f64 / span).min(20.0);
            }
            next += TICK;
            let now = Instant::now();
            if now - next > Duration::from_secs(2) {
                next = now; // don't try to catch up after a long stall
            }
            self.wait_until(next);
        }
        tracing::info!("saving world...");
        for p in self.players.iter_mut().flatten() {
            let pk = out::disconnect(&Text::plain("Сервер остановлен"));
            p.send(pk);
            p.flush();
        }
        // return crafting grids and cursors to the inventories before saving
        for slot in 0..self.players.len() {
            if self.players[slot].is_some() {
                containers::player_left(&mut self, slot);
            }
        }
        playerdata::save_all(&self, true);
        playerdata::flush();
        self.world.save_all();
        tracing::info!("world saved");
        if let Some(h) = self.plugins.take() {
            h.shutdown();
        }
    }

    fn do_tick(&mut self) {
        self.tick += 1;
        self.day_time += 1;

        // 0. plugin actions from the previous tick
        plugins::tick(self);

        // 1. world: finished chunk loads
        let loaded = self.world.collect_loaded(self.tick);
        if !loaded.is_empty() {
            for p in self.players.iter_mut().flatten() {
                p.stream_dirty = true;
            }
        }

        // 2. joins
        while let Ok(np) = self.new_rx.try_recv() {
            self.join(np);
        }

        // 3. inbound packets
        for slot in 0..self.players.len() {
            handle::process_inbound(self, slot);
        }

        // 4. entities (items, TNT, falling blocks) + explosions
        entity::tick(self);

        super::game::containers::tick(self);
        super::game::survival::tick(self);
        interact::dig_tick(self);

        // 5. chunk streaming
        for slot in 0..self.players.len() {
            self.stream(slot);
        }

        // 6. movement + entity tracking
        track::tick(self);

        // 7. scheduled fluid ticks, then block changes this tick + acks
        fluids::tick(self);
        self.settle_outputs();

        // 8. periodic
        if self.tick % 20 == 0 {
            self.second();
        }
        if self.tick % (20 * 60 * 5) == 0 {
            self.world.save_all();
        }

        // 9. disconnects + flush
        self.flush_players(true);
    }

    /// Wait until `deadline`, handling inbound packets as they arrive. The
    /// wait is a blocking receive on the wake channel, so nothing spins; a
    /// packet is handled within microseconds instead of at the next tick.
    fn wait_until(&mut self, deadline: Instant) {
        loop {
            let now = Instant::now();
            if now >= deadline || !self.running.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            match self.wake_rx.recv_timeout(deadline - now) {
                Ok(()) => self.pump(),
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => return,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
                    return;
                }
            }
        }
    }

    /// Between-tick work: handle queued inbound packets, then settle the
    /// results right away (neighbor updates, block changes, acks, new entity
    /// spawns) and flush. The tick end runs the same steps, so the order
    /// (block updates before acks) is the same either way.
    fn pump(&mut self) {
        for slot in 0..self.players.len() {
            handle::process_inbound(self, slot);
        }
        track::spawn_fresh(self);
        self.settle_outputs();
        self.flush_players(false);
    }

    /// Neighbor updates for this part of the tick, drain block changes to
    /// their viewers, then send pending block acks after those updates.
    fn settle_outputs(&mut self) {
        let t = Instant::now();
        neighbors::settle(self);
        let changes = self.world.drain_changes();
        if t.elapsed().as_millis() > 20 {
            tracing::info!("block changes + relight took {:?}", t.elapsed());
        }
        for (frame, viewers) in changes {
            for v in viewers {
                if let Some(p) = self.players.get_mut(v).and_then(|p| p.as_mut()) {
                    p.out.push(frame.clone());
                }
            }
        }
        // block_changed_ack goes after the block updates (vanilla sends it from
        // the connection tick, after the level tick): an ack that arrives first
        // makes the client roll back to the old block.
        for p in self.players.iter_mut().flatten() {
            if p.ack_seq >= 0 {
                let a = out::block_ack(p.ack_seq);
                p.send(a);
                p.ack_seq = -1;
            }
        }
    }

    /// Send every player's queued frames (the tick end also adds the tick
    /// broadcast), handle kicks, and remove players whose connection is gone.
    fn flush_players(&mut self, with_broadcast: bool) {
        let bc = if with_broadcast { std::mem::take(&mut self.broadcast) } else { Vec::new() };
        let mut gone = Vec::new();
        for p in self.players.iter_mut().flatten() {
            if !bc.is_empty() {
                p.out.extend(bc.iter().cloned());
            }
            if let Some(reason) = p.kick.take() {
                let pk = out::disconnect(&Text::plain(reason));
                p.send(pk);
                p.flush();
                let _ = p.outbound.send(crate::net::Out::Close);
                gone.push(p.slot);
                continue;
            }
            p.flush();
            if p.outbound.is_closed() || !p.alive.load(std::sync::atomic::Ordering::Relaxed) {
                gone.push(p.slot);
            }
        }
        for slot in gone {
            self.leave(slot);
        }
    }

    fn second(&mut self) {
        let ids = self.clock_ids.clone();
        let time = out::frame(out::set_time(self.tick as i64, self.day_time, &ids, 1.0));
        let ka_id = self.tick as i64;
        for p in self.players.iter_mut().flatten() {
            p.out.push(time.clone());
            if p.ka_sent.is_none() && self.tick % 200 == 0 {
                p.ka_id = ka_id;
                p.ka_sent = Some(Instant::now());
                let pk = out::keep_alive(ka_id);
                p.send(pk);
            }
            if p.last_ka_reply.elapsed() > Duration::from_secs(30) {
                p.kick = Some("Тайм-аут соединения".into());
            }
        }
        if self.tick % 100 == 0 && (!self.entities.list.is_empty() || self.players.iter().any(|p| p.is_some())) {
            tracing::info!(
                "tps {:.1} mspt {:.2} peak {:.1}ms players {} entities {} chunks {}",
                self.tps,
                self.mspt,
                std::mem::take(&mut self.peak_ms),
                self.players.iter().flatten().count(),
                self.entities.list.len(),
                self.world.chunks.len()
            );
        }
        if self.tick % (20 * 60 * 5) == 0 {
            playerdata::save_all(self, false);
        }
        if self.tick % 100 == 0 {
            let lat: Vec<(uuid::Uuid, i32)> = self.players.iter().flatten().map(|p| (p.uuid, p.ping_ms)).collect();
            if !lat.is_empty() {
                let f = out::frame(out::info_latency(&lat));
                let footer = Text::plain(format!("TPS {:.1}  MSPT {:.2}  игроков {}", self.tps, self.mspt, lat.len())).color("gray");
                let tl = out::frame(out::tab_list(&Text::plain("Rine").color("gold").bold(), &footer));
                self.broadcast.push(f);
                self.broadcast.push(tl);
            }
        }
        // unload chunks nobody watches
        let spawn = ChunkPos::of_block(self.spawn[0] as i32, self.spawn[2] as i32);
        self.world.unload_idle(self.tick, |p| (p.x - spawn.x).abs() <= 4 && (p.z - spawn.z).abs() <= 4);
    }

    fn join(&mut self, np: NewPlayer) {
        let slot = match self.players.iter().position(|p| p.is_none()) {
            Some(s) => s,
            None => {
                self.players.push(None);
                self.players.len() - 1
            }
        };
        let eid = self.alloc_eid();
        let cfg = self.shared.config.clone();
        let gm = match cfg.gamemode.as_str() {
            "creative" => 1,
            "adventure" => 2,
            "spectator" => 3,
            _ => 0,
        };
        let saved = playerdata::read(&cfg.world_dir, np.uuid);
        let gm = saved.as_ref().map_or(gm, |d| d.gamemode);
        let mut p = Player::new(np, slot, eid, self.spawn, gm, cfg.view_distance as i32);
        if let Some(d) = &saved {
            playerdata::apply(&mut p, d);
        }
        p.joined_tick = self.tick;
        tracing::info!("{} joined ({}), eid {}", p.name, p.uuid, eid);

        // --- join sequence (docs/spec/02 section 10)
        let dim = self.dim_type_id;
        let login = rine_protocol::frame::packet(rine_protocol::ids::play::C_LOGIN, |b| {
            use rine_protocol::Writer;
            b.w_i32(eid);
            b.w_bool(false);
            b.w_varint(1);
            b.w_str("minecraft:overworld");
            b.w_varint(cfg.max_players as i32);
            b.w_varint(p.view);
            b.w_varint(cfg.simulation_distance as i32);
            b.w_bool(false);
            b.w_bool(true);
            b.w_bool(false);
            b.w_varint(dim as i32);
            b.w_str("minecraft:overworld");
            b.w_i64(cfg.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15u64 as i64));
            b.w_varint(gm as i32);
            b.w_varint(0);
            b.w_bool(false);
            b.w_bool(false);
            b.w_bool(false);
            b.w_varint(0);
            b.w_varint(63);
            b.w_bool(cfg.online_mode);
            b.w_bool(false);
        });
        p.send(login);
        p.send(rine_protocol::frame::packet(rine_protocol::ids::play::C_CHANGE_DIFFICULTY, |b| {
            use rine_protocol::Writer;
            b.w_varint(2);
            b.w_bool(false);
        }));
        let ab = p.abilities_packet();
        p.send(ab);
        p.send(out::set_held_slot(p.held as i32));
        let (x, y, z) = (self.spawn[0], self.spawn[1], self.spawn[2]);
        let [px, py, pz] = p.pos;
        p.teleport(px, py, pz, p.yaw, p.pitch);
        // border
        p.send(rine_protocol::frame::packet(rine_protocol::ids::play::C_INITIALIZE_BORDER, |b| {
            use rine_protocol::Writer;
            b.w_f64(0.0);
            b.w_f64(0.0);
            b.w_f64(59_999_968.0);
            b.w_f64(59_999_968.0);
            b.w_varlong(0);
            b.w_varint(29_999_984);
            b.w_varint(5);
            b.w_varint(15);
        }));
        let ids = self.clock_ids.clone();
        p.send(out::set_time(self.tick as i64, self.day_time, &ids, 1.0));
        p.send(rine_protocol::frame::packet(rine_protocol::ids::play::C_SET_DEFAULT_SPAWN_POSITION, |b| {
            use rine_protocol::Writer;
            b.w_str("minecraft:overworld");
            b.w_pos(rine_protocol::BlockPos::new(x as i32, y as i32, z as i32));
            b.w_f32(0.0);
            b.w_f32(0.0);
        }));
        p.send(out::game_event(13, 0.0));
        p.send(rine_protocol::frame::packet(rine_protocol::ids::play::C_TICKING_STATE, |b| {
            use rine_protocol::Writer;
            b.w_f32(20.0);
            b.w_bool(false);
        }));
        p.send(rine_protocol::frame::packet(rine_protocol::ids::play::C_TICKING_STEP, |b| {
            use rine_protocol::Writer;
            b.w_varint(0);
        }));
        let view = p.view;
        p.send(rine_protocol::frame::packet(rine_protocol::ids::play::C_SET_CHUNK_CACHE_RADIUS, |b| {
            use rine_protocol::Writer;
            b.w_varint(view);
        }));
        p.send(rine_protocol::frame::packet(rine_protocol::ids::play::C_SET_SIMULATION_DISTANCE, |b| {
            use rine_protocol::Writer;
            b.w_varint(cfg.simulation_distance as i32);
        }));
        let c = p.chunk();
        p.center = c;
        p.send(out::chunk_center(c.x, c.z));
        if gm == 0 && saved.is_none() {
            let starter = [("minecraft:stone_pickaxe", 1), ("minecraft:torch", 32), ("minecraft:bread", 8)];
            for (n, c) in starter {
                if let Some(id) = items::id(n) {
                    p.inv.add(player::Stack::new(id, c));
                }
            }
        }
        let inv = p.inventory_packet();
        p.send(inv);
        p.send(out::set_health(p.health, p.food, p.saturation));
        p.op = self.is_op(&p.name);
        let op = p.op;
        p.send(out::entity_event(eid, if op { 28 } else { 24 }));
        p.send(commands::commands_packet(op));

        // tab list: everyone to the newcomer, newcomer to everyone
        let mut entries = Vec::new();
        for o in self.players.iter().flatten() {
            entries.push(out::InfoEntry { uuid: o.uuid, name: &o.name, props: &o.props, gamemode: o.gamemode as i32, ping: o.ping_ms });
        }
        entries.push(out::InfoEntry { uuid: p.uuid, name: &p.name, props: &p.props, gamemode: gm as i32, ping: 0 });
        let all = out::info_add(&entries);
        drop(entries);
        p.send(all);
        let me = out::frame(out::info_add(&[out::InfoEntry { uuid: p.uuid, name: &p.name, props: &p.props, gamemode: gm as i32, ping: 0 }]));
        let joined = out::frame(out::system_chat(&Text::plain(format!("{} зашёл на сервер", p.name)).color("yellow"), false));
        for o in self.players.iter_mut().flatten() {
            o.out.push(me.clone());
            o.out.push(joined.clone());
        }
        let welcome = Text::plain("Добро пожаловать на ").color("gray").push(Text::plain("Rine").color("gold").bold()).push(Text::plain(". /help для списка команд").color("gray"));
        p.msg(&welcome);
        plugins::post(self, rine_plugin::Event::Join { name: p.name.clone(), uuid: p.uuid.to_string() });
        if let Some(d) = &saved {
            survival::set_exhaustion(slot, eid, d.exhaustion);
        }
        self.by_eid.insert(eid, slot);
        self.players[slot] = Some(p);
        self.stream(slot);
    }

    pub fn leave(&mut self, slot: usize) {
        if self.players.get(slot).is_some_and(|p| p.is_some()) {
            containers::player_left(self, slot);
        }
        let Some(p) = self.players.get_mut(slot).and_then(|p| p.take()) else { return };
        playerdata::save_player(&self.shared.config.world_dir, &p, survival::exhaustion_of(slot, p.eid), false);
        tracing::info!("{} left", p.name);
        plugins::post(self, rine_plugin::Event::Quit { name: p.name.clone() });
        self.by_eid.remove(&p.eid);
        for c in &p.loaded {
            if let Some(s) = self.world.chunks.get_mut(c) {
                s.viewers.retain(|v| *v != slot);
                if s.viewers.is_empty() {
                    s.idle_since = self.tick;
                }
            }
        }
        let rm = out::frame(out::info_remove(&[p.uuid]));
        let despawn = out::frame(out::remove_entities(&[p.eid]));
        let left = out::frame(out::system_chat(&Text::plain(format!("{} вышел", p.name)).color("yellow"), false));
        for o in self.players.iter_mut().flatten() {
            o.out.push(rm.clone());
            if o.tracked.remove(&p.eid) {
                o.out.push(despawn.clone());
            }
            o.out.push(left.clone());
        }
    }

    /// Per-player chunk streaming with vanilla's batch/ack flow control.
    fn stream(&mut self, slot: usize) {
        let Some(p) = self.players.get_mut(slot).and_then(|p| p.as_mut()) else { return };
        let c = p.chunk();
        if c != p.center {
            p.center = c;
            p.send(out::chunk_center(c.x, c.z));
            p.stream_dirty = true;
        }
        if p.stream_dirty {
            p.stream_dirty = false;
            let r = p.view;
            // forget chunks out of range
            let gone: Vec<ChunkPos> = p.loaded.iter().copied().filter(|q| (q.x - c.x).abs() > r + 1 || (q.z - c.z).abs() > r + 1).collect();
            for q in &gone {
                p.loaded.remove(q);
                p.send(out::forget_chunk(q.x, q.z));
                if let Some(s) = self.world.chunks.get_mut(q) {
                    s.viewers.retain(|v| *v != slot);
                    if s.viewers.is_empty() {
                        s.idle_since = self.tick;
                    }
                }
            }
            let mut want = Vec::new();
            for dx in -r..=r {
                for dz in -r..=r {
                    let q = ChunkPos::new(c.x + dx, c.z + dz);
                    if !p.loaded.contains(&q) {
                        want.push(q);
                    }
                }
            }
            want.sort_by_key(|q| q.dist2(c));
            p.queue = want.into();
            for q in p.queue.iter() {
                if !self.world.chunks.contains_key(q) {
                    self.world.request(*q);
                }
            }
        }
        if p.queue.is_empty() || p.unacked >= p.max_unacked {
            return;
        }
        p.quota = (p.quota + p.desired).min(p.desired.max(1.0));
        let n = p.quota.floor() as usize;
        if n == 0 {
            return;
        }
        let mut batch = Vec::with_capacity(n);
        let mut skipped = std::collections::VecDeque::new();
        while batch.len() < n {
            let Some(q) = p.queue.pop_front() else { break };
            if p.loaded.contains(&q) {
                continue;
            }
            if self.world.chunks.contains_key(&q) {
                batch.push(q);
            } else {
                skipped.push_back(q);
                if skipped.len() > 64 {
                    break;
                }
            }
        }
        while let Some(q) = skipped.pop_back() {
            p.queue.push_front(q);
        }
        if batch.is_empty() {
            return;
        }
        let p = self.players[slot].as_mut().unwrap();
        p.send(rine_protocol::frame::packet(rine_protocol::ids::play::C_CHUNK_BATCH_START, |_| {}));
        let count = batch.len();
        for q in batch {
            if let Some(f) = self.world.chunk_frame(q) {
                let p = self.players[slot].as_mut().unwrap();
                p.out.push(f);
                p.loaded.insert(q);
                if let Some(s) = self.world.chunks.get_mut(&q) {
                    s.viewers.push(slot);
                }
            }
        }
        let p = self.players[slot].as_mut().unwrap();
        p.send(rine_protocol::frame::packet(rine_protocol::ids::play::C_CHUNK_BATCH_FINISHED, |b| {
            use rine_protocol::Writer;
            b.w_varint(count as i32);
        }));
        p.quota -= count as f32;
        p.unacked += 1;
        // entities in newly loaded chunks become visible through tracking
    }

    pub fn player(&mut self, slot: usize) -> Option<&mut Player> {
        self.players.get_mut(slot).and_then(|p| p.as_mut())
    }

    pub fn find_player(&self, name: &str) -> Option<usize> {
        self.players.iter().flatten().find(|p| p.name.eq_ignore_ascii_case(name)).map(|p| p.slot)
    }

    /// Send a frame to every viewer of chunk `c` except `skip`.
    pub fn send_chunk_viewers_except(&mut self, c: ChunkPos, f: &Bytes, skip: Option<usize>) {
        if let Some(s) = self.world.chunks.get(&c) {
            for &v in &s.viewers {
                if Some(v) == skip {
                    continue;
                }
                if let Some(Some(p)) = self.players.get_mut(v) {
                    p.out.push(f.clone());
                }
            }
        }
    }

    /// Send a frame to every player that has chunk `c` loaded.
    pub fn send_chunk_viewers(&mut self, c: ChunkPos, f: &Bytes) {
        if let Some(s) = self.world.chunks.get(&c) {
            for &v in &s.viewers {
                if let Some(Some(p)) = self.players.get_mut(v) {
                    p.out.push(f.clone());
                }
            }
        }
    }

    pub fn broadcast_msg(&mut self, t: &Text) {
        self.broadcast.push(out::frame(out::system_chat(t, false)));
    }
}

