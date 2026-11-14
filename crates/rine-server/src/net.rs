//! Network front-end. Each TCP connection is an async task that runs
//! handshake/status/login/configuration entirely off the game thread. Once
//! the client reaches play, a `NewPlayer` is handed to the game loop with a
//! raw inbound queue and an outbound sender drained by a writer task.

use crate::config::Config;
use bytes::{Bytes, BytesMut};
use rine_protocol::frame::{encode_frame, packet};
use rine_protocol::ids::{config as cfg, handshake, login, status};
use rine_protocol::text::{json_str, Text};
use rine_protocol::{FrameDecoder, ProtoError, Reader, Writer, PROTOCOL_VERSION, VERSION_NAME};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct ClientInfo {
    pub locale: String,
    pub view_distance: i8,
    pub chat_mode: i32,
    pub skin_parts: u8,
    pub main_hand: i32,
}

pub enum Out {
    /// Already-framed packets to write in order.
    Batch(Vec<Bytes>),
    /// Flush pending data and close the socket.
    Close,
}

pub struct NewPlayer {
    pub name: String,
    pub uuid: Uuid,
    pub properties: Vec<(String, String, Option<String>)>,
    pub info: ClientInfo,
    pub addr: SocketAddr,
    /// Decoded play payloads (`id + body`). Disconnected when the socket dies.
    pub inbound: crossbeam_channel::Receiver<BytesMut>,
    pub outbound: mpsc::UnboundedSender<Out>,
    /// Cleared by the reader task when the socket closes.
    pub alive: Arc<std::sync::atomic::AtomicBool>,
}

pub struct Shared {
    pub config: Config,
    pub online: AtomicUsize,
    pub new_players: crossbeam_channel::Sender<NewPlayer>,
    /// Wakes the game loop when inbound play packets are queued, so they are
    /// handled between ticks instead of at the next tick start. Capacity 1:
    /// one pending wake is enough, the loop drains every queue on each wake.
    pub wake: crossbeam_channel::Sender<()>,
    /// Precomputed configuration frames (registry data + tags), encoded once.
    pub config_frames: Vec<Bytes>,
}

/// Java's UUID.nameUUIDFromBytes("OfflinePlayer:" + name).
pub fn offline_uuid(name: &str) -> Uuid {
    use md5::{Digest, Md5};
    let mut h: [u8; 16] = Md5::digest(format!("OfflinePlayer:{name}").as_bytes()).into();
    h[6] = (h[6] & 0x0f) | 0x30;
    h[8] = (h[8] & 0x3f) | 0x80;
    Uuid::from_bytes(h)
}

pub fn build_config_frames(threshold: i32) -> Vec<Bytes> {
    let data = rine_data::get();
    let mut frames = Vec::new();
    for reg in &data.registries.synced {
        let p = packet(cfg::C_REGISTRY_DATA, |b| {
            b.w_str(&reg.name);
            b.w_varint(reg.entries.len() as i32);
            for e in &reg.entries {
                b.w_str(e);
                b.w_bool(false); // data comes from the client's built-in core pack
            }
        });
        frames.push(encode_frame(&p, threshold).0);
    }
    let tags = data.registries.tags_payload();
    let p = packet(cfg::C_UPDATE_TAGS, |b| b.w_slice(&tags));
    frames.push(encode_frame(&p, threshold).0);
    frames
}

pub async fn listen(shared: Arc<Shared>) -> anyhow::Result<()> {
    let listener = TcpListener::bind(&shared.config.bind).await?;
    tracing::info!("listening on {}", shared.config.bind);
    loop {
        let (sock, addr) = listener.accept().await?;
        let _ = sock.set_nodelay(true);
        let shared = shared.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(sock, addr, shared).await {
                tracing::debug!("{addr}: {e}");
            }
        });
    }
}

struct Conn {
    rd: OwnedReadHalf,
    wr: OwnedWriteHalf,
    dec: FrameDecoder,
    threshold: i32,
    buf: Vec<u8>,
}

impl Conn {
    async fn read_packet(&mut self) -> anyhow::Result<BytesMut> {
        loop {
            if let Some(p) = self.dec.next()? {
                return Ok(p);
            }
            let n = self.rd.read(&mut self.buf).await?;
            if n == 0 {
                anyhow::bail!("eof");
            }
            self.dec.feed(&self.buf[..n]);
        }
    }
    async fn send(&mut self, p: BytesMut) -> anyhow::Result<()> {
        let f = encode_frame(&p, self.threshold);
        self.wr.write_all(&f.0).await?;
        Ok(())
    }
}

async fn handle(sock: TcpStream, addr: SocketAddr, shared: Arc<Shared>) -> anyhow::Result<()> {
    let (rd, wr) = sock.into_split();
    let mut c = Conn { rd, wr, dec: FrameDecoder::default(), threshold: -1, buf: vec![0u8; 16384] };

    // ---- handshake
    let p = c.read_packet().await?;
    let mut r = Reader::new(&p);
    if r.varint()? != handshake::INTENTION {
        anyhow::bail!("expected handshake");
    }
    let proto = r.varint()?;
    let _host = r.string(255)?;
    let _port = r.u16()?;
    let intent = r.varint()?;
    match intent {
        1 => return handle_status(c, &shared).await,
        2 | 3 => {}
        _ => anyhow::bail!("bad intent"),
    }
    if proto != PROTOCOL_VERSION {
        let msg = Text::plain(format!("Нужен клиент Minecraft {VERSION_NAME}")).color("red");
        let p = packet(login::C_DISCONNECT, |b| b.w_str(&msg.to_json()));
        c.send(p).await?;
        return Ok(());
    }

    // ---- login (offline mode; online mode is TODO with AES/CFB8 + session server)
    let p = c.read_packet().await?;
    let mut r = Reader::new(&p);
    if r.varint()? != login::S_HELLO {
        anyhow::bail!("expected login hello");
    }
    let name = r.string(16)?.to_string();
    if !name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        anyhow::bail!("bad name");
    }
    let uuid = offline_uuid(&name);
    if shared.online.load(Ordering::Relaxed) >= shared.config.max_players as usize {
        let msg = Text::plain("Сервер заполнен").color("red");
        c.send(packet(login::C_DISCONNECT, |b| b.w_str(&msg.to_json()))).await?;
        return Ok(());
    }

    let th = shared.config.compression_threshold;
    if th >= 0 {
        c.send(packet(login::C_COMPRESSION, |b| b.w_varint(th))).await?;
        c.threshold = th;
        c.dec.threshold = th;
    }
    c.send(packet(login::C_FINISHED, |b| {
        b.w_uuid(uuid);
        b.w_str(&name);
        b.w_varint(0); // properties (skins need online mode)
        b.w_uuid(Uuid::new_v4());
    }))
    .await?;
    loop {
        let p = c.read_packet().await?;
        let mut r = Reader::new(&p);
        match r.varint()? {
            login::S_ACKNOWLEDGED => break,
            login::S_CUSTOM_QUERY_ANSWER | login::S_COOKIE_RESPONSE => {}
            id => anyhow::bail!("unexpected login packet {id}"),
        }
    }

    // ---- configuration
    c.send(packet(cfg::C_CUSTOM_PAYLOAD, |b| {
        b.w_str("minecraft:brand");
        b.w_str("Rine");
    }))
    .await?;
    c.send(packet(cfg::C_UPDATE_ENABLED_FEATURES, |b| {
        b.w_varint(1);
        b.w_str("minecraft:vanilla");
    }))
    .await?;
    c.send(packet(cfg::C_SELECT_KNOWN_PACKS, |b| {
        b.w_varint(1);
        b.w_str("minecraft");
        b.w_str("core");
        b.w_str(VERSION_NAME);
    }))
    .await?;

    let mut info = ClientInfo { view_distance: 10, main_hand: 1, skin_parts: 0x7f, ..Default::default() };
    let mut sent_registries = false;
    loop {
        let p = c.read_packet().await?;
        let mut r = Reader::new(&p);
        match r.varint()? {
            cfg::S_CLIENT_INFORMATION => {
                info.locale = r.string(16)?.to_string();
                info.view_distance = r.i8()?;
                info.chat_mode = r.varint()?;
                let _colors = r.bool()?;
                info.skin_parts = r.u8()?;
                info.main_hand = r.varint()?;
            }
            cfg::S_SELECT_KNOWN_PACKS => {
                let n = r.varint()?;
                let mut has_core = false;
                for _ in 0..n {
                    let ns = r.string(32767)?;
                    let id = r.string(32767)?;
                    let ver = r.string(32767)?;
                    if ns == "minecraft" && id == "core" && ver == VERSION_NAME {
                        has_core = true;
                    }
                }
                if !has_core {
                    let msg = Text::plain("Клиент не знает ванильный пакет данных 26.3").color("red");
                    c.send(packet(cfg::C_DISCONNECT, |b| msg.to_nbt().write_network(b))).await?;
                    return Ok(());
                }
                if !sent_registries {
                    sent_registries = true;
                    let mut all = BytesMut::new();
                    for f in &shared.config_frames {
                        all.extend_from_slice(f);
                    }
                    c.wr.write_all(&all).await?;
                    c.send(packet(cfg::C_FINISH, |_| {})).await?;
                }
            }
            cfg::S_FINISH => break,
            cfg::S_KEEP_ALIVE | cfg::S_PONG | cfg::S_CUSTOM_PAYLOAD | cfg::S_RESOURCE_PACK
            | cfg::S_ACCEPT_CODE_OF_CONDUCT => {}
            id => tracing::debug!("{addr}: ignoring config packet {id}"),
        }
    }

    // ---- hand off to the game loop
    let (in_tx, in_rx) = crossbeam_channel::unbounded::<BytesMut>();
    let (out_tx, out_rx) = mpsc::unbounded_channel::<Out>();
    shared.online.fetch_add(1, Ordering::Relaxed);
    let Conn { mut rd, wr, mut dec, buf, .. } = c;
    let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let np = NewPlayer { name: name.clone(), uuid, properties: Vec::new(), info, addr, inbound: in_rx, outbound: out_tx, alive: alive.clone() };
    if shared.new_players.send(np).is_err() {
        anyhow::bail!("game loop gone");
    }
    let writer = tokio::spawn(writer_task(wr, out_rx));
    // reader loop: decode frames and push raw payloads to the game thread
    let mut buf = buf;
    let res: anyhow::Result<()> = async {
        loop {
            let mut queued = false;
            while let Some(p) = dec.next()? {
                if in_tx.send(p).is_err() {
                    return Ok(());
                }
                queued = true;
            }
            if queued {
                let _ = shared.wake.try_send(());
            }
            let n = rd.read(&mut buf).await?;
            if n == 0 {
                return Ok(());
            }
            dec.feed(&buf[..n]);
        }
    }
    .await;
    drop(in_tx);
    alive.store(false, Ordering::Relaxed);
    shared.online.fetch_sub(1, Ordering::Relaxed);
    let _ = writer.await;
    res
}

async fn writer_task(mut wr: OwnedWriteHalf, mut rx: mpsc::UnboundedReceiver<Out>) {
    let mut scratch = BytesMut::with_capacity(64 * 1024);
    while let Some(first) = rx.recv().await {
        let mut close = false;
        let push = |o: Out, scratch: &mut BytesMut| match o {
            Out::Batch(v) => {
                for f in v {
                    scratch.extend_from_slice(&f);
                }
                false
            }
            Out::Close => true,
        };
        close |= push(first, &mut scratch);
        // coalesce everything already queued into one write
        while let Ok(o) = rx.try_recv() {
            close |= push(o, &mut scratch);
            if scratch.len() > 1 << 20 {
                break;
            }
        }
        if !scratch.is_empty() {
            if wr.write_all(&scratch).await.is_err() {
                return;
            }
            scratch.clear();
        }
        if close {
            let _ = wr.shutdown().await;
            return;
        }
    }
}

async fn handle_status(mut c: Conn, shared: &Shared) -> anyhow::Result<()> {
    loop {
        let p = match c.read_packet().await {
            Ok(p) => p,
            Err(_) => return Ok(()),
        };
        let mut r = Reader::new(&p);
        match r.varint()? {
            status::S_STATUS_REQUEST => {
                let json = format!(
                    "{{\"version\":{{\"name\":\"Rine {VERSION_NAME}\",\"protocol\":{PROTOCOL_VERSION}}},\"players\":{{\"max\":{},\"online\":{},\"sample\":[]}},\"description\":{},\"enforcesSecureChat\":false}}",
                    shared.config.max_players,
                    shared.online.load(Ordering::Relaxed),
                    json_str(&shared.config.motd)
                );
                c.send(packet(status::C_STATUS_RESPONSE, |b| b.w_str(&json))).await?;
            }
            status::S_PING => {
                let t = r.i64()?;
                c.send(packet(status::C_PONG, |b| b.w_i64(t))).await?;
                return Ok(());
            }
            _ => return Err(ProtoError::Invalid("status packet").into()),
        }
    }
}
