//! One headless bot: handshake, login, configuration, play.

use std::time::Duration;

use anyhow::{ensure, Context, Result};
use bytes::BytesMut;
use rine_protocol::frame::packet;
use rine_protocol::ids::{config as cid, login as lid, play as pid};
use rine_protocol::{encode_frame, BlockPos, FrameDecoder, Reader, Writer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::TcpStream;
use tokio::time::{Instant as TokioInstant, MissedTickBehavior};
use uuid::Uuid;

use crate::chunk::parse_level_chunk;
use crate::nbt::{read_text_component, skip_nbt};

/// Walk speed in blocks per second for --walk.
const WALK_SPEED: f64 = 4.3;
const MOVE_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum State {
    Connect,
    Login,
    Config,
    Play,
}

impl State {
    fn label(self) -> &'static str {
        match self {
            State::Connect => "connect",
            State::Login => "login",
            State::Config => "config",
            State::Play => "play",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BotOpts {
    pub dump: bool,
    pub walk: bool,
}

#[derive(Debug)]
pub struct Report {
    pub name: String,
    pub state: State,
    pub registries: u32,
    pub registry_log: Vec<(String, u32)>,
    pub tag_registries: u32,
    pub tags: u64,
    pub chunks: u64,
    pub parse_errors: u64,
    pub keepalives: u64,
    pub bytes: u64,
    pub first_chunk_non_air: Option<u32>,
    pub play_secs: f64,
    pub note: Option<String>,
}

impl Report {
    pub fn chunks_per_sec(&self) -> Option<f64> {
        (self.state == State::Play && self.play_secs > 0.0).then(|| self.chunks as f64 / self.play_secs)
    }

    /// Prints the per-bot summary; `detail` adds the per-registry and tag counts.
    pub fn print(&self, detail: bool) {
        println!(
            "[{}] state={} registries={} tag_registries={} tags={} chunks={} errors={} keepalives={} bytes={} first_chunk_non_air={} play_secs={:.2}{}",
            self.name,
            self.state.label(),
            self.registries,
            self.tag_registries,
            self.tags,
            self.chunks,
            self.parse_errors,
            self.keepalives,
            self.bytes,
            self.first_chunk_non_air.map_or("-".to_string(), |v| v.to_string()),
            self.play_secs,
            self.note.as_ref().map_or(String::new(), |n| format!(" note={n}")),
        );
        if detail {
            for (reg, n) in &self.registry_log {
                println!("[{}]   registry {reg}: {n} entries", self.name);
            }
        }
    }
}

struct Bot {
    name: String,
    opts: BotOpts,
    state: State,
    dec: FrameDecoder,
    registries: u32,
    registry_log: Vec<(String, u32)>,
    tag_registries: u32,
    tags: u64,
    chunks: u64,
    parse_errors: u64,
    keepalives: u64,
    bytes_rx: u64,
    first_chunk_non_air: Option<u32>,
    first_chunk_logged: bool,
    x: f64,
    y: f64,
    z: f64,
    yaw: f32,
    pitch: f32,
    teleported: bool,
    play_started: Option<std::time::Instant>,
    note: Option<String>,
}

impl Bot {
    fn new(name: String, opts: BotOpts) -> Self {
        Self {
            name,
            opts,
            state: State::Connect,
            dec: FrameDecoder::default(),
            registries: 0,
            registry_log: Vec::new(),
            tag_registries: 0,
            tags: 0,
            chunks: 0,
            parse_errors: 0,
            keepalives: 0,
            bytes_rx: 0,
            first_chunk_non_air: None,
            first_chunk_logged: false,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            teleported: false,
            play_started: None,
            note: None,
        }
    }

    /// Frames a packet with the current compression threshold and writes it.
    async fn send_pkt(
        &self,
        wr: &mut OwnedWriteHalf,
        id: i32,
        f: impl FnOnce(&mut BytesMut),
    ) -> Result<()> {
        let body = packet(id, f);
        let frame = encode_frame(&body, self.dec.threshold);
        wr.write_all(&frame.0).await?;
        Ok(())
    }

    fn note_once(&mut self, msg: String) {
        if self.note.is_none() {
            self.note = Some(msg);
        }
    }

    async fn on_packet(&mut self, wr: &mut OwnedWriteHalf, pkt: &[u8]) -> Result<bool> {
        let mut r = Reader::new(pkt);
        let id = r.varint()?;
        let body = &pkt[r.pos..];
        if self.opts.dump {
            println!("[{}] {} id={} len={}", self.name, self.state.label(), id, pkt.len());
        }
        match self.state {
            State::Login => self.on_login(wr, id, body).await,
            State::Config => self.on_config(wr, id, body).await,
            State::Play => self.on_play(wr, id, body).await,
            State::Connect => Ok(true),
        }
    }

    async fn on_login(&mut self, wr: &mut OwnedWriteHalf, id: i32, body: &[u8]) -> Result<bool> {
        let mut r = Reader::new(body);
        match id {
            lid::C_COMPRESSION => {
                let threshold = r.varint()?;
                // Both directions switch once the server has sent this packet.
                self.dec.threshold = threshold;
                println!("[{}] login: compression threshold {threshold}", self.name);
            }
            lid::C_FINISHED => {
                self.send_pkt(wr, lid::S_ACKNOWLEDGED, |_| {}).await?;
                self.state = State::Config;
                self.send_client_information(wr).await?;
            }
            lid::C_DISCONNECT => {
                let reason = r.string(262144)?.to_owned();
                println!("[{}] login disconnect: {reason}", self.name);
                self.note_once(format!("login disconnect: {reason}"));
                return Ok(false);
            }
            _ => {}
        }
        Ok(true)
    }

    async fn send_client_information(&self, wr: &mut OwnedWriteHalf) -> Result<()> {
        self.send_pkt(wr, cid::S_CLIENT_INFORMATION, |b| {
            b.w_str("en_us");
            b.w_i8(10);
            b.w_varint(0); // chat visibility FULL
            b.w_bool(true); // chat colors
            b.w_u8(0x7f); // all skin parts
            b.w_varint(1); // main hand RIGHT
            b.w_bool(false); // text filtering
            b.w_bool(true); // allows listing
            b.w_varint(0); // particle status ALL
        })
        .await
    }

    async fn on_config(&mut self, wr: &mut OwnedWriteHalf, id: i32, body: &[u8]) -> Result<bool> {
        let mut r = Reader::new(body);
        match id {
            cid::C_KEEP_ALIVE => {
                let v = r.i64()?;
                self.send_pkt(wr, cid::S_KEEP_ALIVE, |b| b.w_i64(v)).await?;
                self.keepalives += 1;
            }
            cid::C_SELECT_KNOWN_PACKS => {
                let n = usize::try_from(r.varint()?)?;
                let mut packs: Vec<(&str, &str, &str)> = Vec::with_capacity(n);
                for _ in 0..n {
                    let ns = r.string(32767)?;
                    let pid_ = r.string(32767)?;
                    let ver = r.string(32767)?;
                    packs.push((ns, pid_, ver));
                }
                ensure!(r.remaining() == 0, "select_known_packs: {} trailing bytes", r.remaining());
                self.send_pkt(wr, cid::S_SELECT_KNOWN_PACKS, |b| {
                    b.w_varint(packs.len() as i32);
                    for (ns, pk, ver) in &packs {
                        b.w_str(ns);
                        b.w_str(pk);
                        b.w_str(ver);
                    }
                })
                .await?;
            }
            cid::C_REGISTRY_DATA => {
                let reg = r.string(32767)?;
                let n = usize::try_from(r.varint()?)?;
                for _ in 0..n {
                    let _key = r.string(32767)?;
                    if r.bool()? {
                        let t = r.u8()?;
                        skip_nbt(&mut r, t)?;
                    }
                }
                ensure!(r.remaining() == 0, "registry_data {reg}: {} trailing bytes", r.remaining());
                self.registries += 1;
                if self.opts.dump {
                    println!("[{}] registry {reg}: {n} entries", self.name);
                }
                self.registry_log.push((reg.to_owned(), n as u32));
            }
            cid::C_UPDATE_TAGS => {
                let nreg = usize::try_from(r.varint()?)?;
                for _ in 0..nreg {
                    let reg = r.string(32767)?;
                    let ntags = usize::try_from(r.varint()?)?;
                    for _ in 0..ntags {
                        let _tag = r.string(32767)?;
                        let nids = usize::try_from(r.varint()?)?;
                        for _ in 0..nids {
                            r.varint()?;
                        }
                        self.tags += 1;
                    }
                    self.tag_registries += 1;
                    if self.opts.dump {
                        println!("[{}] tags {reg}: {ntags} tags", self.name);
                    }
                }
                ensure!(r.remaining() == 0, "update_tags: {} trailing bytes", r.remaining());
            }
            cid::C_FINISH => {
                self.send_pkt(wr, cid::S_FINISH, |_| {}).await?;
                self.state = State::Play;
                self.play_started = Some(std::time::Instant::now());
                println!("[{}] reached play", self.name);
            }
            cid::C_DISCONNECT => {
                let text = read_text_component(&mut r)?.unwrap_or_else(|| "<non-text component>".into());
                println!("[{}] config disconnect: {text}", self.name);
                self.note_once(format!("config disconnect: {text}"));
                return Ok(false);
            }
            _ => {}
        }
        Ok(true)
    }

    async fn on_play(&mut self, wr: &mut OwnedWriteHalf, id: i32, body: &[u8]) -> Result<bool> {
        let mut r = Reader::new(body);
        match id {
            pid::C_LOGIN => print_login(&self.name, &mut r)?,
            pid::C_PLAYER_POSITION => {
                let tp_id = r.varint()?;
                let (x, y, z) = (r.f64()?, r.f64()?, r.f64()?);
                // delta movement (3 x f64), not used by the bot
                r.bytes(24)?;
                let yaw = r.f32()?;
                let pitch = r.f32()?;
                let flags = r.i32()?;
                self.apply_position(x, y, z, yaw, pitch, flags);
                let (px, py, pz, pyaw, ppitch) = (self.x, self.y, self.z, self.yaw, self.pitch);
                self.send_pkt(wr, pid::S_ACCEPT_TELEPORTATION, |b| {
                    b.w_varint(tp_id);
                    b.w_f64(px);
                    b.w_f64(py);
                    b.w_f64(pz);
                    b.w_f32(pyaw);
                    b.w_f32(ppitch);
                })
                .await?;
                self.teleported = true;
                if self.opts.dump {
                    println!("[{}] teleport id={tp_id} to ({px:.2},{py:.2},{pz:.2}) flags={flags:#x}", self.name);
                }
            }
            pid::C_KEEP_ALIVE => {
                let v = r.i64()?;
                self.send_pkt(wr, pid::S_KEEP_ALIVE, |b| b.w_i64(v)).await?;
                self.keepalives += 1;
            }
            pid::C_CHUNK_BATCH_FINISHED => {
                self.send_pkt(wr, pid::S_CHUNK_BATCH_RECEIVED, |b| b.w_f32(20.0)).await?;
            }
            pid::C_LEVEL_CHUNK_WITH_LIGHT => {
                self.chunks += 1;
                match parse_level_chunk(body) {
                    Ok(sum) => {
                        if !self.first_chunk_logged {
                            self.first_chunk_logged = true;
                            self.first_chunk_non_air = Some(sum.non_air);
                            let col = match sum.column_top {
                                Some((y, state)) => format!("column(0,0) top state={state} at y={y}"),
                                None => "column(0,0) all air".to_string(),
                            };
                            println!(
                                "[{}] first chunk ({},{}): sections={} non_air={} block_entities={} {col}",
                                self.name, sum.x, sum.z, sum.sections, sum.non_air, sum.block_entities
                            );
                        }
                    }
                    Err(e) => {
                        self.parse_errors += 1;
                        println!("[{}] PARSE ERROR {e:#}", self.name);
                    }
                }
            }
            pid::C_SYSTEM_CHAT => {
                let text = read_text_component(&mut r)?;
                let overlay = r.bool()?;
                let shown = text.unwrap_or_else(|| "<non-text component>".into());
                println!("[{}] chat{}: {shown}", self.name, if overlay { " (actionbar)" } else { "" });
            }
            pid::C_DISCONNECT => {
                let text = read_text_component(&mut r)?.unwrap_or_else(|| "<non-text component>".into());
                println!("[{}] play disconnect: {text}", self.name);
                self.note_once(format!("play disconnect: {text}"));
                return Ok(false);
            }
            _ => {}
        }
        Ok(true)
    }

    /// Applies a player_position packet, honouring the relative flag bits
    /// (X=1, Y=2, Z=4, Y_ROT=8, X_ROT=16). Velocity flags are ignored.
    fn apply_position(&mut self, x: f64, y: f64, z: f64, yaw: f32, pitch: f32, flags: i32) {
        let f = flags as u32;
        if f & 0x001 != 0 {
            self.x += x;
        } else {
            self.x = x;
        }
        if f & 0x002 != 0 {
            self.y += y;
        } else {
            self.y = y;
        }
        if f & 0x004 != 0 {
            self.z += z;
        } else {
            self.z = z;
        }
        if f & 0x008 != 0 {
            self.yaw += yaw;
        } else {
            self.yaw = yaw;
        }
        if f & 0x010 != 0 {
            self.pitch += pitch;
        } else {
            self.pitch = pitch;
        }
    }

    /// Periodic serverbound move_player_pos_rot (every 50 ms once teleported).
    async fn tick_move(&mut self, wr: &mut OwnedWriteHalf) -> Result<()> {
        if self.opts.walk {
            self.x += WALK_SPEED * MOVE_INTERVAL.as_secs_f64();
        }
        let (x, y, z, yaw, pitch) = (self.x, self.y, self.z, self.yaw, self.pitch);
        self.send_pkt(wr, pid::S_MOVE_PLAYER_POS_ROT, |b| {
            b.w_f64(x);
            b.w_f64(y);
            b.w_f64(z);
            b.w_f32(yaw);
            b.w_f32(pitch);
            b.w_u8(1); // onGround
        })
        .await
    }

    fn report(&self) -> Report {
        Report {
            name: self.name.clone(),
            state: self.state,
            registries: self.registries,
            registry_log: self.registry_log.clone(),
            tag_registries: self.tag_registries,
            tags: self.tags,
            chunks: self.chunks,
            parse_errors: self.parse_errors,
            keepalives: self.keepalives,
            bytes: self.bytes_rx,
            first_chunk_non_air: self.first_chunk_non_air,
            play_secs: self.play_started.map_or(0.0, |t| t.elapsed().as_secs_f64()),
            note: self.note.clone(),
        }
    }
}

/// Prints the play login packet (spec 02 section 1).
fn print_login(name: &str, r: &mut Reader) -> Result<()> {
    let entity_id = r.i32()?;
    let hardcore = r.bool()?;
    let nlevels = usize::try_from(r.varint()?)?;
    let mut levels = Vec::with_capacity(nlevels);
    for _ in 0..nlevels {
        levels.push(r.string(32767)?);
    }
    let max_players = r.varint()?;
    let view_distance = r.varint()?;
    let sim_distance = r.varint()?;
    let reduced_debug = r.bool()?;
    let death_screen = r.bool()?;
    let limited_crafting = r.bool()?;
    // CommonPlayerSpawnInfo
    let dim_type = r.varint()?;
    let dimension = r.string(32767)?;
    let seed = r.i64()?;
    let gamemode = r.varint()?;
    let prev_gamemode = r.varint()?;
    let is_debug = r.bool()?;
    let is_flat = r.bool()?;
    let last_death = if r.bool()? {
        let d = r.string(32767)?;
        let p = BlockPos::unpack(r.i64()?);
        format!("{d} {},{},{}", p.x, p.y, p.z)
    } else {
        "none".to_string()
    };
    let portal_cooldown = r.varint()?;
    let sea_level = r.varint()?;
    let online_mode = r.bool()?;
    let enforces_secure_chat = r.bool()?;
    let prev = if prev_gamemode == 0 { "none".to_string() } else { (prev_gamemode - 1).to_string() };
    println!(
        "[{name}] login: entity_id={entity_id} hardcore={hardcore} levels={levels:?} max_players={max_players} \
         view_distance={view_distance} sim_distance={sim_distance} reduced_debug={reduced_debug} \
         death_screen={death_screen} limited_crafting={limited_crafting} dim_type={dim_type} dimension={dimension} \
         seed={seed} gamemode={gamemode} prev_gamemode={prev} debug={is_debug} flat={is_flat} \
         last_death={last_death} portal_cooldown={portal_cooldown} sea_level={sea_level} \
         online_mode={online_mode} secure_chat={enforces_secure_chat}"
    );
    if r.remaining() != 0 {
        println!("[{name}] login: WARNING {} trailing bytes, spec mismatch?", r.remaining());
    }
    Ok(())
}

async fn session(bot: &mut Bot, addr: &str, seconds: f64) -> Result<()> {
    let (host, port_str) = addr.rsplit_once(':').context("address must be host:port")?;
    let port: u16 = port_str.parse().context("bad port")?;

    let stream = tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(addr))
        .await
        .context("connect timeout")??;
    stream.set_nodelay(true)?;
    let (mut rd, mut wr) = stream.into_split();

    // The run window starts at connect.
    let deadline = TokioInstant::now() + Duration::from_secs_f64(seconds);
    let sleep = tokio::time::sleep_until(deadline);
    tokio::pin!(sleep);
    let mut ticker = tokio::time::interval(MOVE_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    bot.send_pkt(&mut wr, 0, |b| {
        b.w_varint(rine_protocol::PROTOCOL_VERSION);
        b.w_str(host);
        b.w_u16(port);
        b.w_varint(2); // intent: LOGIN
    })
    .await?;
    bot.state = State::Login;

    let name = bot.name.clone();
    bot.send_pkt(&mut wr, lid::S_HELLO, |b| {
        b.w_str(&name);
        b.w_uuid(Uuid::new_v4());
    })
    .await?;

    let mut buf = BytesMut::with_capacity(16 * 1024);
    loop {
        tokio::select! {
            res = rd.read_buf(&mut buf) => {
                let n = res?;
                if n == 0 {
                    bot.note_once("connection closed by server".to_string());
                    break;
                }
                bot.bytes_rx += n as u64;
                bot.dec.feed(&buf);
                buf.clear();
                while let Some(pkt) = bot.dec.next()? {
                    let id = Reader::new(&pkt).varint().unwrap_or(-1);
                    let st = bot.state;
                    let keep = bot
                        .on_packet(&mut wr, &pkt)
                        .await
                        .with_context(|| format!("{st:?} packet id={id} len={}", pkt.len()))?;
                    if !keep {
                        return Ok(());
                    }
                }
            }
            _ = ticker.tick(), if bot.state == State::Play && bot.teleported => {
                bot.tick_move(&mut wr).await?;
            }
            _ = &mut sleep => break,
        }
    }
    Ok(())
}

/// Runs one bot until `seconds` elapse or the server disconnects it.
pub async fn run_bot(addr: String, name: String, opts: BotOpts, seconds: f64) -> Report {
    let mut bot = Bot::new(name, opts);
    if let Err(e) = session(&mut bot, &addr, seconds).await {
        bot.note_once(format!("error: {e:#}"));
    }
    bot.report()
}
