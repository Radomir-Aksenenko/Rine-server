//! /time, /weather, /worldborder, /forceload, /setworldspawn, /spawnpoint,
//! /tick, /difficulty (`TimeCommand`, `WeatherCommand`, `WorldBorderCommand`,
//! `ForceLoadCommand`, `SetWorldSpawnCommand`, `SetSpawnCommand`, `TickCommand`,
//! `DifficultyCommand`).
//!
//! Hooks the game loop must call (see the `pub fn` docs): `world::tick` once per
//! tick, `world::tick_should_run` to honour `/tick freeze`, `world::on_join`
//! for a new player, `world::difficulty_frame` / `default_spawn_frame` /
//! `ticking_state_frame` for join, `world::is_forced` for chunk unloading.

use crate::game::cmd::{lit, Commands, Ctx};
use crate::game::{entity, Game};
use crate::out;
use ahash::AHashMap;
use bytes::{Bytes, BytesMut};
use rine_command::error::{java_double, java_float};
use rine_command::{CmdError, CmdResult};
use rine_protocol::text::Text;
use rine_protocol::BlockPos;
use rine_world::ChunkPos;
use std::collections::BTreeSet;
use std::time::Duration;
use uuid::Uuid;

const OVERWORLD: &str = "minecraft:overworld";
const CLOCK: &str = "minecraft:overworld";
const DAY_PERIOD: i64 = 24000;
/// Overworld time markers from `timeline/day.json`.
const MARKERS: &[(&str, i32)] = &[
    ("minecraft:day", 1000),
    ("minecraft:midnight", 18000),
    ("minecraft:night", 13000),
    ("minecraft:noon", 6000),
    ("minecraft:roll_village_siege", 18000),
    ("minecraft:wake_up_from_sleep", 0),
];
const MAX_WORLD_BORDER: f32 = 5.999997E7;
const BORDER_LIMIT: f64 = 2.9999984E7;
const FORCELOAD_MAX_CHUNKS: i64 = 256;

// Clientbound packet ids (vanilla-data/generated/reports/packets.json, 26.3).
const C_SET_BORDER_CENTER: i32 = 90;
const C_SET_BORDER_LERP_SIZE: i32 = 91;
const C_SET_BORDER_SIZE: i32 = 92;
const C_SET_BORDER_WARNING_DELAY: i32 = 93;
const C_SET_BORDER_WARNING_DISTANCE: i32 = 94;

pub fn register(c: &mut Commands) {
    for cmd in ["time", "weather", "worldborder", "forceload", "setworldspawn", "spawnpoint", "tick", "difficulty"] {
        for p in paths(c, cmd) {
            c.exec(p, dispatch);
        }
    }
}

fn paths(c: &Commands, cmd: &str) -> Vec<&'static str> {
    c.tree.nodes.iter().filter(|n| n.executable && n.path.split(' ').next() == Some(cmd)).map(|n| n.path).collect()
}

/// One executor for every command here: picks the handler by the first
/// word of the path.
fn dispatch(x: &mut Ctx) -> CmdResult<i32> {
    let path = x.path();
    match path.split(' ').next().unwrap_or("") {
        "time" => time_cmd(x),
        "weather" => weather_cmd(x),
        "worldborder" => worldborder_cmd(x),
        "forceload" => forceload_cmd(x),
        "setworldspawn" => setworldspawn_cmd(x),
        "spawnpoint" => spawnpoint_cmd(x),
        "tick" => tick_cmd(x),
        _ => difficulty_cmd(x),
    }
}

// ---- state ---------------------------------------------------------------

/// Vanilla `WorldBorder` plus the lerp in progress.
pub struct Border {
    pub center: [f64; 2],
    pub size: f64,
    /// (from, to, start tick, duration in ticks)
    pub lerp: Option<(f64, f64, u64, i64)>,
    pub warning_blocks: i32,
    pub warning_time: i32,
    pub safe_zone: f32,
    pub damage_per_block: f32,
    pub absolute_max: i32,
}

impl Default for Border {
    fn default() -> Self {
        Self {
            center: [0.0, 0.0],
            size: 59_999_968.0,
            lerp: None,
            warning_blocks: 5,
            warning_time: 15,
            safe_zone: 5.0,
            damage_per_block: 0.2,
            absolute_max: 29_999_984,
        }
    }
}

impl Border {
    /// Ticks left in the lerp (0 when none).
    fn remaining(&self, now: u64) -> i64 {
        match self.lerp {
            Some((_, _, start, dur)) => (start as i64 + dur - now as i64).max(0),
            None => 0,
        }
    }
    /// `getSize()`: interpolated while a lerp runs.
    pub fn size_at(&self, now: u64) -> f64 {
        match self.lerp {
            Some((from, to, start, dur)) => {
                let rem = self.remaining(now);
                if rem <= 0 || dur <= 0 {
                    return to;
                }
                let progress = (dur - rem) as f64 / dur as f64;
                let _ = start;
                from + (to - from) * progress
            }
            None => self.size,
        }
    }
    /// `getLerpTarget()`.
    fn target(&self) -> f64 {
        self.lerp.map(|(_, to, _, _)| to).unwrap_or(self.size)
    }
}

pub struct Weather {
    pub clear: i32,
    pub rain: i32,
    pub thunder: i32,
    pub raining: bool,
    pub thundering: bool,
    pub rain_level: f32,
    pub thunder_level: f32,
}

impl Default for Weather {
    fn default() -> Self {
        Self { clear: 0, rain: 100_000, thunder: 100_000, raining: false, thundering: false, rain_level: 0.0, thunder_level: 0.0 }
    }
}

pub struct Clock {
    pub paused: bool,
    pub rate: f32,
    /// Fraction of a tick carried over when the rate is not 1.
    pub partial: f64,
}

impl Default for Clock {
    fn default() -> Self {
        Self { paused: false, rate: 1.0, partial: 0.0 }
    }
}

pub struct TickState {
    pub rate: f32,
    pub frozen: bool,
    pub step_left: i32,
    pub sprint_left: i64,
    /// Recent tick durations in ms (for the percentiles of `/tick query`).
    pub samples: std::collections::VecDeque<f64>,
}

impl Default for TickState {
    fn default() -> Self {
        Self { rate: 20.0, frozen: false, step_left: 0, sprint_left: 0, samples: std::collections::VecDeque::from(vec![0.0; 100]) }
    }
}

/// `LevelData.RespawnData` of one player (`/spawnpoint`).
#[derive(Clone, Copy, Debug)]
pub struct Respawn {
    pub pos: [i32; 3],
    pub yaw: f32,
    pub pitch: f32,
}

/// World border, weather, clock, forced chunks, tick rate, difficulty and
/// per-player spawn points.
#[derive(Default)]
pub struct State {
    pub border: Border,
    pub weather: Weather,
    pub clock: Clock,
    pub tick: TickState,
    /// Forced chunks, packed (`ChunkPos::pack`), sorted like the vanilla list.
    pub forced: BTreeSet<i64>,
    /// Difficulty id (0..=3). None = normal, the value the join packet sends.
    pub difficulty: Option<u8>,
    /// World spawn rotation: [yaw, pitch].
    pub spawn_rot: [f32; 2],
    pub respawn: AHashMap<Uuid, Respawn>,
}

// ---- hooks for the game loop ----------------------------------------------

/// Once per tick: clock, weather, border lerp end. Replace the
/// `self.day_time += 1` in `do_tick` with this call.
pub fn tick(g: &mut Game) {
    advance_clock(g);
    advance_weather(g);
    let now = g.tick;
    let b = &mut g.cs.world.border;
    if let Some((_, to, start, dur)) = b.lerp {
        if (start as i64 + dur - now as i64) <= 0 {
            b.size = to;
            b.lerp = None;
        }
    }
}

/// False while `/tick freeze` holds the game (step and sprint let ticks run).
/// Call before the world phases of `do_tick`; skip them when it returns false.
pub fn tick_should_run(g: &mut Game) -> bool {
    let t = &mut g.cs.world.tick;
    if t.sprint_left > 0 {
        t.sprint_left -= 1;
        return true;
    }
    if !t.frozen {
        return true;
    }
    if t.step_left > 0 {
        t.step_left -= 1;
        return true;
    }
    false
}

/// Time between ticks for the current `/tick rate` (the loop uses it in place of `TICK`).
pub fn tick_interval(g: &Game) -> Duration {
    let t = &g.cs.world.tick;
    if t.frozen {
        return crate::game::TICK;
    }
    Duration::from_secs_f64(1.0 / t.rate.max(1.0) as f64)
}

/// Record one tick's duration in ms (for `/tick query` percentiles).
pub fn record_tick_time(g: &mut Game, ms: f64) {
    let s = &mut g.cs.world.tick.samples;
    if s.len() >= 100 {
        s.pop_front();
    }
    s.push_back(ms);
}

/// Current difficulty id (0 peaceful, 1 easy, 2 normal, 3 hard).
pub fn difficulty(g: &Game) -> u8 {
    g.cs.world.difficulty.unwrap_or(2)
}

/// `ClientboundChangeDifficultyPacket` for the current difficulty.
pub fn difficulty_frame(g: &Game) -> Bytes {
    let d = difficulty(g) as i32;
    out::frame(rine_protocol::frame::packet(rine_protocol::ids::play::C_CHANGE_DIFFICULTY, |b| {
        use rine_protocol::Writer;
        b.w_varint(d);
        b.w_bool(false);
    }))
}

/// `ClientboundSetDefaultSpawnPositionPacket` for the world spawn.
pub fn default_spawn_frame(g: &Game) -> Bytes {
    out::frame(default_spawn_packet(g))
}

fn default_spawn_packet(g: &Game) -> BytesMut {
    let p = BlockPos::new(g.spawn[0] as i32, g.spawn[1] as i32, g.spawn[2] as i32);
    let [yaw, pitch] = g.cs.world.spawn_rot;
    rine_protocol::frame::packet(rine_protocol::ids::play::C_SET_DEFAULT_SPAWN_POSITION, |b| {
        use rine_protocol::Writer;
        b.w_str(OVERWORLD);
        b.w_pos(p);
        b.w_f32(yaw);
        b.w_f32(pitch);
    })
}

/// `ClientboundTickingStatePacket` (rate and frozen flag).
pub fn ticking_state_frame(g: &Game) -> Bytes {
    let t = &g.cs.world.tick;
    let (rate, frozen) = (t.rate, t.frozen);
    out::frame(rine_protocol::frame::packet(rine_protocol::ids::play::C_TICKING_STATE, |b| {
        use rine_protocol::Writer;
        b.w_f32(rate);
        b.w_bool(frozen);
    }))
}

/// Whether a chunk is forced (never unloaded). Pass it to
/// `unload_idle`'s keep closure.
pub fn is_forced(g: &Game, pos: ChunkPos) -> bool {
    g.cs.world.forced.contains(&pos.pack())
}

/// Spawn point set by `/spawnpoint` for a player, if any.
pub fn respawn_point(g: &Game, uuid: Uuid) -> Option<Respawn> {
    g.cs.world.respawn.get(&uuid).copied()
}

/// Join packets for the world border and weather. Call from `Game::join`
/// in place of the inline border packet.
pub fn on_join(g: &mut Game, slot: usize) {
    let init = init_border_packet(g);
    let (raining, rain, thunder) = (g.cs.world.weather.raining, g.cs.world.weather.rain_level, g.cs.world.weather.thunder_level);
    let Some(p) = g.player(slot) else { return };
    p.send(init);
    if raining {
        p.send(out::game_event(1, 0.0));
        p.send(out::game_event(7, rain));
        p.send(out::game_event(8, thunder));
    }
}

fn init_border_packet(g: &Game) -> BytesMut {
    let now = g.tick;
    let b = &g.cs.world.border;
    let (x, z) = (b.center[0], b.center[1]);
    let old = b.size_at(now);
    let new = b.target();
    let lerp = b.remaining(now);
    let (abs, wb, wt) = (b.absolute_max, b.warning_blocks, b.warning_time);
    rine_protocol::frame::packet(rine_protocol::ids::play::C_INITIALIZE_BORDER, |w| {
        use rine_protocol::Writer;
        w.w_f64(x);
        w.w_f64(z);
        w.w_f64(old);
        w.w_f64(new);
        w.w_varlong(lerp);
        w.w_varint(abs);
        w.w_varint(wb);
        w.w_varint(wt);
    })
}

fn advance_clock(g: &mut Game) {
    if !g.rules.bool("advance_time") || g.cs.world.clock.paused {
        return;
    }
    let c = &mut g.cs.world.clock;
    c.partial += c.rate as f64;
    let whole = c.partial.floor();
    c.partial -= whole;
    g.day_time += whole as i64;
}

fn broadcast_clock(g: &mut Game) {
    let ids = g.clock_ids.clone();
    let f = out::frame(out::set_time(g.tick as i64, g.day_time, &ids, g.cs.world.clock.rate));
    g.broadcast.push(f);
}

/// `ServerLevel.advanceWeatherCycle`, client updates as vanilla sends them.
fn advance_weather(g: &mut Game) {
    let was_raining = g.cs.world.weather.raining;
    let salt = g.tick;
    if g.rules.bool("advance_weather") {
        let mut w = std::mem::take(&mut g.cs.world.weather);
        let mut clear = w.clear;
        let mut thunder_t = w.thunder;
        let mut rain_t = w.rain;
        let mut thundering = w.thundering;
        let mut raining = w.raining;
        if clear > 0 {
            clear -= 1;
            thunder_t = if thundering { 0 } else { 1 };
            rain_t = if raining { 0 } else { 1 };
            thundering = false;
            raining = false;
        } else {
            if thunder_t > 0 {
                thunder_t -= 1;
                if thunder_t == 0 {
                    thundering = !thundering;
                }
            } else if thundering {
                thunder_t = uniform(salt, 1, 3600, 15600);
            } else {
                thunder_t = uniform(salt, 2, 12000, 180000);
            }
            if rain_t > 0 {
                rain_t -= 1;
                if rain_t == 0 {
                    raining = !raining;
                }
            } else if raining {
                rain_t = uniform(salt, 3, 12000, 24000);
            } else {
                rain_t = uniform(salt, 4, 12000, 180000);
            }
        }
        w.clear = clear;
        w.thunder = thunder_t;
        w.rain = rain_t;
        w.thundering = thundering;
        w.raining = raining;
        g.cs.world.weather = w;
    }
    let w = &mut g.cs.world.weather;
    let old_thunder = w.thunder_level;
    w.thunder_level = (w.thunder_level + if w.thundering { 0.01 } else { -0.01 }).clamp(0.0, 1.0);
    let old_rain = w.rain_level;
    w.rain_level = (w.rain_level + if w.raining { 0.01 } else { -0.01 }).clamp(0.0, 1.0);
    let (rain_level, thunder_level, raining) = (w.rain_level, w.thunder_level, w.raining);

    if old_rain != rain_level {
        g.broadcast.push(out::frame(out::game_event(7, rain_level)));
    }
    if old_thunder != thunder_level {
        g.broadcast.push(out::frame(out::game_event(8, thunder_level)));
    }
    if was_raining != raining {
        let ev = if was_raining { 2 } else { 1 };
        g.broadcast.push(out::frame(out::game_event(ev, 0.0)));
    }
}

/// `Mth.randomBetweenInclusive` stand-in: deterministic from the tick and a salt.
fn uniform(tick: u64, salt: u64, min: i32, max: i32) -> i32 {
    let r = entity::rand_f64(tick.wrapping_mul(31).wrapping_add(salt));
    min + (r * (max - min + 1) as f64) as i32
}

// ---- formatting helpers ---------------------------------------------------

fn fmt_secs(ticks: i64) -> String {
    format!("{:.2}", ticks as f64 / 20.0)
}

/// `TimeCommand.wrapTime`: `ticks % 2147483647` as an int.
fn wrap_time(t: i64) -> i32 {
    (t % 2147483647) as i32
}

/// `ChunkPos.toString()`.
fn chunk_str(x: i32, z: i32) -> String {
    format!("[{x}, {z}]")
}

/// `Mth.wrapDegrees(float)`.
fn wrap_degrees(v: f32) -> f32 {
    let mut f = v % 360.0;
    if f >= 180.0 {
        f -= 360.0;
    }
    if f < -180.0 {
        f += 360.0;
    }
    f
}

fn path_tokens(x: &Ctx) -> Vec<&'static str> {
    x.path().split(' ').collect()
}

// ---- /time ----------------------------------------------------------------

fn time_cmd(x: &mut Ctx) -> CmdResult<i32> {
    let toks = path_tokens(x);
    let of = toks.get(1) == Some(&"of");
    if of && x.id("clock") != CLOCK {
        return Err(CmdError::tr("argument.resource.not_found", vec![lit(x.id("clock")), lit("minecraft:world_clock")]));
    }
    let i = if of { 3 } else { 1 };
    let sub = toks[i];
    match sub {
        "set" => {
            if toks[i + 1] == "time" {
                let t = x.int("time") as i64;
                time_set_total(x, t)
            } else {
                let id = x.id("timemarker").to_string();
                time_set_marker(x, &id)
            }
        }
        "add" => {
            let t = x.int("time") as i64;
            let g = &mut *x.g;
            g.day_time = (g.day_time + t).max(0);
            broadcast_clock(g);
            let total = g.day_time;
            x.success(Text::tr("commands.time.set.absolute", vec![lit(CLOCK), lit(total)]), true);
            Ok(wrap_time(total))
        }
        "pause" | "resume" => {
            let paused = sub == "pause";
            let g = &mut *x.g;
            if g.cs.world.clock.paused == paused {
                let key = if paused { "commands.time.pause.already_paused" } else { "commands.time.pause.already_running" };
                return Err(CmdError::tr(key, vec![lit(CLOCK)]));
            }
            g.cs.world.clock.paused = paused;
            let key = if paused { "commands.time.pause" } else { "commands.time.resume" };
            x.success(Text::tr(key, vec![lit(CLOCK)]), true);
            Ok(1)
        }
        "rate" => {
            let rate = x.float("rate");
            let g = &mut *x.g;
            if g.cs.world.clock.rate == rate {
                return Err(CmdError::tr("commands.time.rate.already_same", vec![lit(CLOCK), lit(java_float(rate))]));
            }
            g.cs.world.clock.rate = rate;
            x.success(Text::tr("commands.time.rate", vec![lit(CLOCK), lit(java_float(rate))]), true);
            Ok(1)
        }
        _ => {
            // query
            match toks.get(i + 1).copied() {
                Some("gametime") => {
                    let t = x.g.tick as i64;
                    x.success(Text::tr("commands.time.query.gametime", vec![lit(t)]), false);
                    Ok(wrap_time(t))
                }
                Some("time") => {
                    let t = x.g.day_time;
                    x.success(Text::tr("commands.time.query.absolute", vec![lit(CLOCK), lit(t)]), false);
                    Ok(wrap_time(t))
                }
                _ => {
                    let id = x.id("timeline").to_string();
                    let period = timeline_period(&id)?;
                    let day = x.g.day_time;
                    if toks.get(i + 3) == Some(&"repetition") {
                        let reps = period.map(|p| day.div_euclid(p)).unwrap_or(0);
                        x.success(Text::tr("commands.time.query.timeline.repetitions", vec![lit(&id), lit(reps)]), false);
                        Ok(wrap_time(reps))
                    } else {
                        let cur = period.map(|p| day.rem_euclid(p)).unwrap_or(day);
                        x.success(Text::tr("commands.time.query.timeline", vec![lit(&id), lit(cur)]), false);
                        Ok(wrap_time(cur))
                    }
                }
            }
        }
    }
}

/// Period of an overworld timeline (None = not periodic); unknown ids fail.
fn timeline_period(id: &str) -> CmdResult<Option<i64>> {
    match id {
        "minecraft:day" => Ok(Some(DAY_PERIOD)),
        "minecraft:moon" => Ok(Some(192_000)),
        "minecraft:early_game" => Ok(None),
        _ => Err(CmdError::tr("argument.resource.not_found", vec![lit(id), lit("minecraft:timeline")])),
    }
}

/// `TimeCommand.setTotalTicks`.
fn time_set_total(x: &mut Ctx, t: i64) -> CmdResult<i32> {
    let g = &mut *x.g;
    if g.day_time == t {
        return Err(CmdError::tr("commands.time.set.already_at_time", vec![lit(CLOCK), lit(t)]));
    }
    g.day_time = t;
    broadcast_clock(g);
    x.success(Text::tr("commands.time.set.absolute", vec![lit(CLOCK), lit(t)]), true);
    Ok(wrap_time(t))
}

/// `TimeCommand.setTimeToTimeMarker`: next occurrence of the marker in the
/// day period (approximation of `ClockTimeMarker.resolveTimeToMoveTo`).
fn time_set_marker(x: &mut Ctx, id: &str) -> CmdResult<i32> {
    let Some(&(_, ticks)) = MARKERS.iter().find(|(n, _)| *n == id) else {
        return Err(CmdError::tr("commands.time.no_time_marker_found", vec![lit(id), lit(CLOCK)]));
    };
    let g = &mut *x.g;
    let cur = g.day_time;
    let delta = (ticks as i64 - cur.rem_euclid(DAY_PERIOD)).rem_euclid(DAY_PERIOD);
    if delta == 0 {
        return Err(CmdError::tr("commands.time.set.already_at_time_marker", vec![lit(CLOCK), lit(id)]));
    }
    g.day_time = cur + delta;
    broadcast_clock(g);
    x.success(Text::tr("commands.time.set.time_marker", vec![lit(CLOCK), lit(id)]), true);
    Ok(wrap_time(x.g.day_time))
}

// ---- /weather -------------------------------------------------------------

fn weather_cmd(x: &mut Ctx) -> CmdResult<i32> {
    let toks = path_tokens(x);
    let input = if x.has("duration") { x.int("duration") } else { -1 };
    let salt = x.g.tick;
    let dur = |min: i32, max: i32| if input == -1 { uniform(salt, 5, min, max) } else { input };
    let w = &mut x.g.cs.world.weather;
    let (key, clear, rain, raining, thundering) = match toks[1] {
        "clear" => ("commands.weather.set.clear", dur(12000, 180000), 0, false, false),
        "rain" => ("commands.weather.set.rain", 0, dur(12000, 24000), true, false),
        _ => ("commands.weather.set.thunder", 0, dur(3600, 15600), true, true),
    };
    w.clear = clear;
    w.rain = rain;
    w.thunder = rain;
    w.raining = raining;
    w.thundering = thundering;
    x.success(Text::tr(key, vec![]), true);
    Ok(input)
}

// ---- /worldborder ---------------------------------------------------------

fn worldborder_cmd(x: &mut Ctx) -> CmdResult<i32> {
    let toks = path_tokens(x);
    let now = x.g.tick;
    match toks[1] {
        "get" => {
            let size = x.g.cs.world.border.size_at(now);
            x.success(Text::tr("commands.worldborder.get", vec![lit(format!("{size:.0}"))]), false);
            Ok((size + 0.5).floor() as i32)
        }
        "add" => {
            let dist = x.double("distance");
            let cur = x.g.cs.world.border.size_at(now);
            let ticks = if toks.len() > 3 { x.g.cs.world.border.remaining(now) + x.int("time") as i64 } else { 0 };
            set_size(x, cur + dist, ticks)
        }
        "set" => {
            let dist = x.double("distance");
            let ticks = if toks.len() > 3 { x.int("time") as i64 } else { 0 };
            set_size(x, dist, ticks)
        }
        "center" => {
            let [cx, cz] = x.vec2("pos");
            let (cx, cz) = (cx as f64, cz as f64);
            let b = &mut x.g.cs.world.border;
            if b.center[0] == cx && b.center[1] == cz {
                return Err(CmdError::key("commands.worldborder.center.failed"));
            }
            if cx.abs() > BORDER_LIMIT || cz.abs() > BORDER_LIMIT {
                return Err(CmdError::tr("commands.worldborder.set.failed.far", vec![lit(java_double(BORDER_LIMIT))]));
            }
            b.center = [cx, cz];
            let g = &mut *x.g;
            g.broadcast.push(out::frame(border_center_packet(cx, cz)));
            x.success(Text::tr("commands.worldborder.center.success", vec![lit(format!("{cx:.2}")), lit(format!("{cz:.2}"))]), true);
            Ok(0)
        }
        "damage" => {
            if toks[2] == "amount" {
                let v = x.float("damagePerBlock");
                let b = &mut x.g.cs.world.border;
                if b.damage_per_block == v {
                    return Err(CmdError::key("commands.worldborder.damage.amount.failed"));
                }
                b.damage_per_block = v;
                x.success(Text::tr("commands.worldborder.damage.amount.success", vec![lit(format!("{v:.2}"))]), true);
                Ok(v as i32)
            } else {
                let v = x.float("distance");
                let b = &mut x.g.cs.world.border;
                if b.safe_zone == v {
                    return Err(CmdError::key("commands.worldborder.damage.buffer.failed"));
                }
                b.safe_zone = v;
                x.success(Text::tr("commands.worldborder.damage.buffer.success", vec![lit(format!("{v:.2}"))]), true);
                Ok(v as i32)
            }
        }
        _ => {
            // warning
            if toks[2] == "distance" {
                let d = x.int("distance");
                let b = &mut x.g.cs.world.border;
                if b.warning_blocks == d {
                    return Err(CmdError::key("commands.worldborder.warning.distance.failed"));
                }
                b.warning_blocks = d;
                x.g.broadcast.push(out::frame(int_packet(C_SET_BORDER_WARNING_DISTANCE, d)));
                x.success(Text::tr("commands.worldborder.warning.distance.success", vec![lit(d)]), true);
                Ok(d)
            } else {
                let t = x.int("time");
                let b = &mut x.g.cs.world.border;
                if b.warning_time == t {
                    return Err(CmdError::key("commands.worldborder.warning.time.failed"));
                }
                b.warning_time = t;
                x.g.broadcast.push(out::frame(int_packet(C_SET_BORDER_WARNING_DELAY, t)));
                x.success(Text::tr("commands.worldborder.warning.time.success", vec![lit(fmt_secs(t as i64))]), true);
                Ok(t)
            }
        }
    }
}

/// `WorldBorderCommand.setSize`.
fn set_size(x: &mut Ctx, distance: f64, ticks: i64) -> CmdResult<i32> {
    let now = x.g.tick;
    let current = x.g.cs.world.border.size_at(now);
    if current == distance {
        return Err(CmdError::key("commands.worldborder.set.failed.nochange"));
    }
    if distance < 1.0 {
        return Err(CmdError::key("commands.worldborder.set.failed.small"));
    }
    if distance > MAX_WORLD_BORDER as f64 {
        return Err(CmdError::tr("commands.worldborder.set.failed.big", vec![lit(java_float(MAX_WORLD_BORDER))]));
    }
    let formatted = format!("{distance:.1}");
    let g = &mut *x.g;
    if ticks > 0 {
        g.cs.world.border.lerp = Some((current, distance, now, ticks));
        g.broadcast.push(out::frame(lerp_packet(current, distance, ticks)));
        let key = if distance > current { "commands.worldborder.set.grow" } else { "commands.worldborder.set.shrink" };
        x.success(Text::tr(key, vec![lit(formatted), lit(fmt_secs(ticks))]), true);
    } else {
        let b = &mut g.cs.world.border;
        b.size = distance;
        b.lerp = None;
        g.broadcast.push(out::frame(size_packet(distance)));
        x.success(Text::tr("commands.worldborder.set.immediate", vec![lit(formatted)]), true);
    }
    Ok((distance - current) as i32)
}

fn border_center_packet(x: f64, z: f64) -> BytesMut {
    rine_protocol::frame::packet(C_SET_BORDER_CENTER, |b| {
        use rine_protocol::Writer;
        b.w_f64(x);
        b.w_f64(z);
    })
}

fn lerp_packet(from: f64, to: f64, ticks: i64) -> BytesMut {
    rine_protocol::frame::packet(C_SET_BORDER_LERP_SIZE, |b| {
        use rine_protocol::Writer;
        b.w_f64(from);
        b.w_f64(to);
        b.w_varlong(ticks);
    })
}

fn size_packet(size: f64) -> BytesMut {
    rine_protocol::frame::packet(C_SET_BORDER_SIZE, |b| {
        use rine_protocol::Writer;
        b.w_f64(size);
    })
}

fn int_packet(id: i32, v: i32) -> BytesMut {
    rine_protocol::frame::packet(id, |b| {
        use rine_protocol::Writer;
        b.w_varint(v);
    })
}

// ---- /forceload -----------------------------------------------------------

fn forceload_cmd(x: &mut Ctx) -> CmdResult<i32> {
    let toks = path_tokens(x);
    match toks[1] {
        "add" | "remove" => {
            let from = x.column_pos("from");
            let to = if x.has("to") { x.column_pos("to") } else { from };
            change_forceload(x, from, to, toks[1] == "add")
        }
        "query" => {
            if toks.len() == 2 {
                list_forceload(x)
            } else {
                query_forceload(x)
            }
        }
        _ => remove_all(x),
    }
}

fn change_forceload(x: &mut Ctx, from: [i32; 2], to: [i32; 2], add: bool) -> CmdResult<i32> {
    let (min_x, min_z) = (from[0].min(to[0]), from[1].min(to[1]));
    let (max_x, max_z) = (from[0].max(to[0]), from[1].max(to[1]));
    if !(min_x >= -30_000_000 && min_z >= -30_000_000 && max_x < 30_000_000 && max_z < 30_000_000) {
        return Err(CmdError::key("argument.pos.outofworld"));
    }
    let (cx0, cz0) = (min_x >> 4, min_z >> 4);
    let (cx1, cz1) = (max_x >> 4, max_z >> 4);
    let count = (cx1 - cx0 + 1) as i64 * (cz1 - cz0 + 1) as i64;
    if count > FORCELOAD_MAX_CHUNKS {
        return Err(CmdError::tr("commands.forceload.toobig", vec![lit(FORCELOAD_MAX_CHUNKS), lit(count)]));
    }
    let g = &mut *x.g;
    let mut changed = Vec::new();
    for cx in cx0..=cx1 {
        for cz in cz0..=cz1 {
            let cp = ChunkPos::new(cx, cz);
            let did = if add { g.cs.world.forced.insert(cp.pack()) } else { g.cs.world.forced.remove(&cp.pack()) };
            if did {
                if add {
                    g.world.request(cp);
                }
                changed.push(cp);
            }
        }
    }
    let (min_c, max_c) = (chunk_str(cx0, cz0), chunk_str(cx1, cz1));
    if changed.is_empty() {
        let key = if add { "commands.forceload.added.failure" } else { "commands.forceload.removed.failure" };
        return Err(CmdError::key(key));
    }
    let n = changed.len();
    let text = if n == 1 {
        let c = changed[0];
        let key = if add { "commands.forceload.added.single" } else { "commands.forceload.removed.single" };
        Text::tr(key, vec![lit(chunk_str(c.x, c.z)), lit(OVERWORLD)])
    } else {
        let key = if add { "commands.forceload.added.multiple" } else { "commands.forceload.removed.multiple" };
        Text::tr(key, vec![lit(n), lit(OVERWORLD), lit(min_c), lit(max_c)])
    };
    x.success(text, true);
    Ok(n as i32)
}

fn remove_all(x: &mut Ctx) -> CmdResult<i32> {
    x.g.cs.world.forced.clear();
    x.success(Text::tr("commands.forceload.removed.all", vec![lit(OVERWORLD)]), true);
    Ok(0)
}

fn list_forceload(x: &mut Ctx) -> CmdResult<i32> {
    let packed: Vec<i64> = x.g.cs.world.forced.iter().copied().collect();
    let n = packed.len();
    if n == 0 {
        x.failure(Text::tr("commands.forceload.added.none", vec![lit(OVERWORLD)]));
        return Ok(0);
    }
    let list = packed
        .iter()
        .map(|&p| {
            let x = p as u32 as i32;
            let z = (p >> 32) as i32;
            chunk_str(x, z)
        })
        .collect::<Vec<_>>()
        .join(", ");
    if n == 1 {
        x.success(Text::tr("commands.forceload.list.single", vec![lit(OVERWORLD), lit(list)]), false);
    } else {
        x.success(Text::tr("commands.forceload.list.multiple", vec![lit(n), lit(OVERWORLD), lit(list)]), false);
    }
    Ok(n as i32)
}

fn query_forceload(x: &mut Ctx) -> CmdResult<i32> {
    let [bx, bz] = x.column_pos("pos");
    let cp = ChunkPos::of_block(bx, bz);
    let s = chunk_str(cp.x, cp.z);
    if x.g.cs.world.forced.contains(&cp.pack()) {
        x.success(Text::tr("commands.forceload.query.success", vec![lit(s), lit(OVERWORLD)]), false);
        Ok(1)
    } else {
        Err(CmdError::tr("commands.forceload.query.failure", vec![lit(s), lit(OVERWORLD)]))
    }
}

// ---- /setworldspawn, /spawnpoint -------------------------------------------

fn setworldspawn_cmd(x: &mut Ctx) -> CmdResult<i32> {
    let (pos, rot) = if x.has("pos") {
        let p = x.spawnable_pos("pos")?;
        let r = if x.has("rotation") { x.rotation("rotation") } else { [0.0, 0.0] };
        (p, r)
    } else {
        let s = x.src.pos;
        ([s[0].floor() as i32, s[1].floor() as i32, s[2].floor() as i32], [0.0, 0.0])
    };
    // rotation vector: x = pitch, y = yaw
    let (pitch, yaw) = (rot[0], rot[1]);
    let g = &mut *x.g;
    g.spawn = [pos[0] as f64 + 0.5, pos[1] as f64, pos[2] as f64 + 0.5];
    g.cs.world.spawn_rot = [yaw, pitch];
    let pkt = default_spawn_packet(g);
    g.broadcast.push(out::frame(pkt));
    x.success(
        Text::tr(
            "commands.setworldspawn.success",
            vec![lit(pos[0]), lit(pos[1]), lit(pos[2]), lit(java_float(yaw)), lit(java_float(pitch)), lit(OVERWORLD)],
        ),
        true,
    );
    Ok(1)
}

fn spawnpoint_cmd(x: &mut Ctx) -> CmdResult<i32> {
    let targets = if x.has("targets") { x.players("targets")? } else { vec![x.source_player()?] };
    let pos = if x.has("pos") { x.spawnable_pos("pos")? } else {
        let s = x.src.pos;
        [s[0].floor() as i32, s[1].floor() as i32, s[2].floor() as i32]
    };
    let rot = if x.has("rotation") { x.rotation("rotation") } else { [0.0, 0.0] };
    let yaw = wrap_degrees(rot[1]);
    let pitch = rot[0].clamp(-90.0, 90.0);
    let respawn = Respawn { pos, yaw, pitch };
    let mut count = 0;
    let mut last = 0;
    for &slot in &targets {
        let Some(uuid) = x.g.players.get(slot).and_then(|p| p.as_ref()).map(|p| p.uuid) else { continue };
        x.g.cs.world.respawn.insert(uuid, respawn);
        count += 1;
        last = slot;
    }
    let (yaw_s, pitch_s) = (java_float(yaw), java_float(pitch));
    let text = if count == 1 {
        let name = x.player_name(last);
        Text::tr(
            "commands.spawnpoint.success.single",
            vec![lit(pos[0]), lit(pos[1]), lit(pos[2]), lit(yaw_s), lit(pitch_s), lit(OVERWORLD), name],
        )
    } else {
        Text::tr(
            "commands.spawnpoint.success.multiple",
            vec![lit(pos[0]), lit(pos[1]), lit(pos[2]), lit(yaw_s), lit(pitch_s), lit(OVERWORLD), lit(count)],
        )
    };
    x.success(text, true);
    Ok(count)
}

// ---- /tick ----------------------------------------------------------------

fn tick_cmd(x: &mut Ctx) -> CmdResult<i32> {
    let toks = path_tokens(x);
    match toks[1] {
        "query" => tick_query(x),
        "rate" => {
            let rate = x.float("rate");
            x.g.cs.world.tick.rate = rate;
            let f = ticking_state_frame(x.g);
            x.g.broadcast.push(f);
            x.success(Text::tr("commands.tick.rate.success", vec![lit(format!("{rate:.1}"))]), true);
            Ok(rate as i32)
        }
        "freeze" | "unfreeze" => {
            let freeze = toks[1] == "freeze";
            let t = &mut x.g.cs.world.tick;
            if freeze {
                t.sprint_left = 0;
                t.step_left = 0;
            }
            t.frozen = freeze;
            let f = ticking_state_frame(x.g);
            x.g.broadcast.push(f);
            let key = if freeze { "commands.tick.status.frozen" } else { "commands.tick.status.running" };
            x.success(Text::tr(key, vec![]), true);
            Ok(freeze as i32)
        }
        "step" => tick_step(x, &toks),
        _ => tick_sprint(x, &toks),
    }
}

fn tick_query(x: &mut Ctx) -> CmdResult<i32> {
    let mspt = x.g.mspt;
    let rate = x.g.cs.world.tick.rate;
    let rate_s = format!("{rate:.1}");
    let busy_s = format!("{mspt:.1}");
    let per_tick_ms = 1000.0 / rate as f64;
    if x.g.cs.world.tick.sprint_left > 0 {
        x.success(Text::tr("commands.tick.status.sprinting", vec![]), false);
        x.success(Text::tr("commands.tick.query.rate.sprinting", vec![lit(&rate_s), lit(&busy_s)]), false);
    } else {
        let key = if x.g.cs.world.tick.frozen {
            "commands.tick.status.frozen"
        } else if per_tick_ms < mspt {
            "commands.tick.status.lagging"
        } else {
            "commands.tick.status.running"
        };
        x.success(Text::tr(key, vec![]), false);
        x.success(
            Text::tr("commands.tick.query.rate.running", vec![lit(&rate_s), lit(&busy_s), lit(format!("{per_tick_ms:.1}"))]),
            false,
        );
    }
    let mut s: Vec<f64> = x.g.cs.world.tick.samples.iter().copied().collect();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = s.len();
    if n > 0 {
        let p = |f: f64| format!("{:.1}", s[((n as f64 * f) as usize).min(n - 1)]);
        let (p50, p95, p99) = (p(0.5), p(0.95), p(0.99));
        x.success(Text::tr("commands.tick.query.percentiles", vec![lit(p50), lit(p95), lit(p99), lit(n)]), false);
    }
    Ok(rate as i32)
}

fn tick_step(x: &mut Ctx, toks: &[&str]) -> CmdResult<i32> {
    if toks.get(2) == Some(&"stop") {
        if x.g.cs.world.tick.step_left <= 0 {
            return Err(CmdError::key("commands.tick.step.stop.fail"));
        }
        x.g.cs.world.tick.step_left = 0;
        x.success(Text::tr("commands.tick.step.stop.success", vec![]), true);
        return Ok(1);
    }
    let advance = if toks.get(2) == Some(&"time") { x.int("time") } else { 1 };
    if !x.g.cs.world.tick.frozen {
        return Err(CmdError::key("commands.tick.step.fail"));
    }
    x.g.cs.world.tick.step_left = advance;
    x.success(Text::tr("commands.tick.step.success", vec![lit(advance)]), true);
    Ok(1)
}

fn tick_sprint(x: &mut Ctx, toks: &[&str]) -> CmdResult<i32> {
    if toks.get(2) == Some(&"stop") {
        if x.g.cs.world.tick.sprint_left <= 0 {
            return Err(CmdError::key("commands.tick.sprint.stop.fail"));
        }
        x.g.cs.world.tick.sprint_left = 0;
        x.success(Text::tr("commands.tick.sprint.stop.success", vec![]), true);
        return Ok(1);
    }
    let time = x.int("time") as i64;
    let interrupted = x.g.cs.world.tick.sprint_left > 0;
    x.g.cs.world.tick.sprint_left = time;
    if interrupted {
        x.success(Text::tr("commands.tick.sprint.stop.success", vec![]), true);
    }
    x.success(Text::tr("commands.tick.status.sprinting", vec![]), true);
    Ok(1)
}

// ---- /difficulty ----------------------------------------------------------

const DIFFICULTY_KEYS: [&str; 4] = ["options.difficulty.peaceful", "options.difficulty.easy", "options.difficulty.normal", "options.difficulty.hard"];

fn difficulty_cmd(x: &mut Ctx) -> CmdResult<i32> {
    let toks = path_tokens(x);
    let cur = difficulty(x.g);
    let Some(name) = toks.get(1).copied() else {
        x.success(Text::tr("commands.difficulty.query", vec![Text::key(DIFFICULTY_KEYS[cur as usize])]), false);
        return Ok(cur as i32);
    };
    let new = match name {
        "peaceful" => 0u8,
        "easy" => 1,
        "normal" => 2,
        _ => 3,
    };
    if cur == new {
        return Err(CmdError::tr("commands.difficulty.failure", vec![Text::key(DIFFICULTY_KEYS[new as usize])]));
    }
    x.g.cs.world.difficulty = Some(new);
    let f = difficulty_frame(x.g);
    x.g.broadcast.push(f);
    x.success(Text::tr("commands.difficulty.success", vec![Text::key(DIFFICULTY_KEYS[new as usize])]), true);
    Ok(0)
}
