//! Strict validator for clientbound play packets (protocol 777, Minecraft 26.3).
//!
//! Each supported packet is parsed field by field and its body must be consumed
//! exactly. Outcomes are kept apart:
//! - `Err`: the body does not match the wire format (counted, see `--expect-clean`).
//! - `Unsupported`: the body uses a layout this validator does not decode, for
//!   example a non-empty item component patch, an entity data serializer outside
//!   the supported list, an unknown brigadier argument parser, or a particle
//!   payload. Not an error.
//! - `Unvalidated`: the packet id is not covered by this validator. Not an error.
//!
//! Layouts: docs/spec/02-play-join-chunks.md and 03-entities-items-interaction.md,
//! checked against Paper-26.3 sources where the spec was marked UNCONFIRMED.

use std::collections::{BTreeMap, HashSet};

use rine_protocol::Reader;

use crate::chunk::{parse_level_chunk, ChunkSummary};
use crate::names::{play_name, PARTICLE_SIMPLE};
use crate::nbt::skip_nbt;

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Unsupported(String),
    Unvalidated,
}

#[derive(Debug, Clone)]
pub struct ParseError {
    /// Byte offset inside the packet body (the packet id is not counted).
    pub offset: usize,
    pub msg: String,
    /// True when this (packet id, message) pair was seen for the first time.
    pub first: bool,
}

#[derive(Debug)]
enum Fail {
    Bad(usize, String),
    Unsup(String),
}

type R<T> = Result<T, Fail>;

/// Counters that are merged across bots and printed with --stats.
#[derive(Default, Debug, Clone)]
pub struct Stats {
    pub ok: BTreeMap<i32, u64>,
    pub unsupported: BTreeMap<(i32, String), u64>,
    pub unvalidated: BTreeMap<i32, u64>,
    /// (packet id, message) -> (count, body offset of the first occurrence).
    pub errors: BTreeMap<(i32, String), (u64, usize)>,
    pub error_count: u64,
    /// Unknown-entity hits per packet id (warnings, not errors).
    pub unknown_entity: BTreeMap<i32, u64>,
}

impl Stats {
    pub fn merge(&mut self, o: &Stats) {
        for (k, v) in &o.ok {
            *self.ok.entry(*k).or_default() += v;
        }
        for (k, v) in &o.unsupported {
            *self.unsupported.entry(k.clone()).or_default() += v;
        }
        for (k, v) in &o.unvalidated {
            *self.unvalidated.entry(*k).or_default() += v;
        }
        for (k, (c, off)) in &o.errors {
            let e = self.errors.entry(k.clone()).or_insert((0, *off));
            e.0 += c;
        }
        self.error_count += o.error_count;
        for (k, v) in &o.unknown_entity {
            *self.unknown_entity.entry(*k).or_default() += v;
        }
    }
}

/// Per-connection state the validator needs (entity registry and stats).
#[derive(Default)]
pub struct Ctx {
    pub entities: HashSet<i32>,
    pub stats: Stats,
    /// Human-readable warnings for the caller to print. One per packet type
    /// for unknown entity ids; the rest are only counted in `stats`.
    pub warnings: Vec<String>,
    /// Summary of the most recent level_chunk_with_light that parsed.
    pub last_chunk: Option<ChunkSummary>,
}

/// Validates one clientbound play packet body (packet id already stripped).
pub fn validate(id: i32, body: &[u8], ctx: &mut Ctx) -> Result<Outcome, ParseError> {
    let mut p = P::new(body);
    match dispatch(id, &mut p, ctx) {
        Ok(false) => {
            *ctx.stats.unvalidated.entry(id).or_default() += 1;
            Ok(Outcome::Unvalidated)
        }
        Ok(true) => {
            if p.left() != 0 {
                let msg = format!("{} trailing bytes after the packet body", p.left());
                return Err(record_error(ctx, id, p.pos(), msg));
            }
            *ctx.stats.ok.entry(id).or_default() += 1;
            Ok(Outcome::Ok)
        }
        Err(Fail::Unsup(reason)) => {
            *ctx.stats.unsupported.entry((id, reason.clone())).or_default() += 1;
            Ok(Outcome::Unsupported(reason))
        }
        Err(Fail::Bad(off, msg)) => Err(record_error(ctx, id, off, msg)),
    }
}

fn record_error(ctx: &mut Ctx, id: i32, offset: usize, msg: String) -> ParseError {
    ctx.stats.error_count += 1;
    let e = ctx.stats.errors.entry((id, msg.clone())).or_insert((0, offset));
    e.0 += 1;
    ParseError { offset, msg, first: e.0 == 1 }
}

/// Byte cursor that tags every failure with the offset where it happened.
struct P<'a> {
    r: Reader<'a>,
}

macro_rules! prim {
    ($($name:ident -> $t:ty;)*) => {
        $(
            fn $name(&mut self) -> R<$t> {
                let o = self.r.pos;
                self.r.$name().map_err(|e| Fail::Bad(o, e.to_string()))
            }
        )*
    };
}

impl<'a> P<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { r: Reader::new(b) }
    }

    fn pos(&self) -> usize {
        self.r.pos
    }

    fn left(&self) -> usize {
        self.r.remaining()
    }

    fn bad<T>(&self, off: usize, msg: impl Into<String>) -> R<T> {
        Err(Fail::Bad(off, msg.into()))
    }

    prim! {
        u8 -> u8;
        i8 -> i8;
        bool -> bool;
        i16 -> i16;
        i32 -> i32;
        i64 -> i64;
        f32 -> f32;
        f64 -> f64;
        varint -> i32;
        varlong -> i64;
    }

    fn skip(&mut self, n: usize) -> R<()> {
        let o = self.r.pos;
        self.r.bytes(n).map(|_| ()).map_err(|e| Fail::Bad(o, e.to_string()))
    }

    /// Protocol string: VarInt byte length + UTF-8, at most `max` characters.
    fn string(&mut self, max: usize) -> R<()> {
        let o = self.r.pos;
        self.r.string(max).map(|_| ()).map_err(|e| Fail::Bad(o, format!("string: {e}")))
    }

    fn uuid(&mut self) -> R<()> {
        self.i64()?;
        self.i64()?;
        Ok(())
    }

    /// VarInt count in `0..=max`.
    fn count(&mut self, max: i32) -> R<usize> {
        let o = self.pos();
        let n = self.varint()?;
        if n < 0 || n > max {
            return self.bad(o, format!("count {n} outside 0..={max}"));
        }
        Ok(n as usize)
    }

    /// VarInt length + bytes, length at most `max`.
    fn byte_array(&mut self, max: i32) -> R<()> {
        let n = self.count(max)?;
        self.skip(n)
    }

    /// Network NBT text component: type byte + payload. TAG_End root is rejected.
    fn component(&mut self) -> R<()> {
        let o = self.pos();
        let t = self.u8()?;
        if t == 0 {
            return self.bad(o, "component root is TAG_End (0x00)");
        }
        let res = skip_nbt(&mut self.r, t);
        res.map_err(|e| Fail::Bad(self.pos(), format!("component NBT (tag {t} at body offset {o}): {e:#}")))
    }
}

/// Returns Ok(true) when the packet was validated, Ok(false) when it is not covered.
fn dispatch(id: i32, p: &mut P, ctx: &mut Ctx) -> R<bool> {
    match id {
        1 => add_entity(p, ctx)?,
        4 => {
            p.varint()?; // sequence
        }
        8 => {
            p.i64()?; // BlockPos
            p.varint()?; // block state id
        }
        10 => {
            difficulty(p)?;
        }
        11 => {
            p.varint()?; // batch size
        }
        12 => {} // chunk batch start: empty payload
        16 => commands(p)?,
        17 => {
            p.varint()?; // container id
        }
        18 => {
            p.varint()?; // container id
            p.varint()?; // state id
            let n = p.count(1 << 16)?;
            for _ in 0..n {
                item_stack(p)?;
            }
            item_stack(p)?; // carried item
        }
        19 => {
            p.varint()?; // container id
            p.i16()?; // property id
            p.i16()?; // value
        }
        20 => {
            p.varint()?; // container id
            p.varint()?; // state id
            p.i16()?; // slot
            item_stack(p)?;
        }
        24 => {
            p.string(32767)?; // channel identifier
            return Err(Fail::Unsup("custom_payload body is skipped, not validated".into()));
        }
        32 => p.component()?,
        34 => {
            p.i32()?; // entity id (readInt, not VarInt)
            p.i8()?; // event
        }
        35 => position_sync(p, ctx)?,
        36 => explode(p)?,
        38 => {
            p.i64()?; // ChunkPos
        }
        39 => {
            p.u8()?; // event
            p.f32()?; // param
        }
        43 => {
            p.varint()?; // entity id
            p.f32()?; // yaw
        }
        44 => {
            for _ in 0..4 {
                p.f64()?;
            }
            p.varlong()?; // lerp time
            p.varint()?; // absolute max size
            p.varint()?; // warning blocks
            p.varint()?; // warning time
        }
        45 => {
            p.i64()?; // keep alive id
        }
        46 => {
            let rest = p.r.rest();
            match parse_level_chunk(rest) {
                Ok(sum) => ctx.last_chunk = Some(sum),
                Err(e) => return Err(Fail::Bad(e.offset, format!("{:#}", e.source))),
            }
        }
        47 => {
            p.i32()?; // event type
            p.i64()?; // BlockPos
            p.i32()?; // data
            p.bool()?; // global
        }
        49 => light_update(p)?,
        50 => login(p, ctx)?,
        54 => move_entity(p, ctx, 54)?,
        55 => move_entity(p, ctx, 55)?,
        57 => move_entity(p, ctx, 57)?,
        60 => {
            p.varint()?; // container id
            p.varint()?; // menu type registry id
            p.component()?; // title
        }
        63 => {
            p.i64()?; // pong time
        }
        65 => {
            p.u8()?; // flags
            p.f32()?; // flying speed
            p.f32()?; // walking speed
        }
        70 => {
            let n = p.count(1 << 16)?;
            for _ in 0..n {
                p.uuid()?;
            }
        }
        71 => player_info_update(p)?,
        73 => {
            p.varint()?; // teleport id
            for _ in 0..6 {
                p.f64()?; // position + delta movement
            }
            p.f32()?; // y rot
            p.f32()?; // x rot
            p.i32()?; // relatives bitfield
        }
        78 => remove_entities(p, ctx)?,
        84 => {
            common_spawn_info(p)?;
            p.i8()?; // data to keep
        }
        85 => {
            p.varint()?; // entity id
            p.i8()?; // head yaw
        }
        86 => {
            p.i64()?; // SectionPos
            let n = p.count(1 << 16)?;
            for _ in 0..n {
                p.varlong()?;
            }
        }
        89 => p.component()?,
        96 => {
            p.varint()?;
            p.varint()?;
        }
        97 => {
            p.varint()?;
        }
        99 => {
            identifier_pos(p)?; // GlobalPos
            p.f32()?; // yaw
            p.f32()?; // pitch
        }
        101 => entity_data(p, ctx)?,
        103 => {
            p.varint()?; // entity id
            lp_vec3(p)?;
        }
        104 => equipment(p)?,
        106 => {
            p.f32()?; // health
            p.varint()?; // food
            p.f32()?; // saturation
        }
        107 => {
            p.varint()?; // slot
        }
        113 => {
            p.varint()?; // simulation distance
        }
        115 => set_time(p)?,
        118 => {
            sound_holder(p)?;
            source(p)?;
            p.varint()?; // entity id
            p.f32()?; // volume
            p.f32()?; // pitch
            p.i64()?; // seed
        }
        119 => {
            sound_holder(p)?;
            source(p)?;
            p.i32()?; // x * 8
            p.i32()?; // y * 8
            p.i32()?; // z * 8
            p.f32()?; // volume
            p.f32()?; // pitch
            p.i64()?; // seed
        }
        123 => {
            p.varint()?; // entity id
            let hand = p.varint()?;
            if !(0..=1).contains(&hand) {
                return p.bad(p.pos(), format!("swing hand {hand} is not 0 or 1"));
            }
            let t = p.varint()?;
            if !(0..=2).contains(&t) {
                return p.bad(p.pos(), format!("swing animation type {t} is not 0..=2"));
            }
            p.varint()?; // duration
        }
        124 => {
            p.component()?;
            p.bool()?; // overlay
        }
        125 => {
            p.component()?; // header
            p.component()?; // footer
        }
        127 => {
            p.varint()?;
            p.varint()?;
            p.varint()?;
        }
        130 => {
            p.f32()?; // tick rate
            p.bool()?; // frozen
        }
        131 => {
            p.varint()?; // tick steps
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Note an entity id seen in a packet; counts (and warns once per packet type) when unknown.
fn note_entity(ctx: &mut Ctx, id: i32, pk: i32) {
    if ctx.entities.contains(&id) {
        return;
    }
    let c = ctx.stats.unknown_entity.entry(pk).or_default();
    *c += 1;
    if *c == 1 {
        ctx.warnings.push(format!(
            "unknown entity id {id} in {} (later hits of this packet type are only counted)",
            play_name(pk)
        ));
    }
}

fn difficulty(p: &mut P) -> R<()> {
    let o = p.pos();
    let d = p.varint()?;
    if !(0..=3).contains(&d) {
        return p.bad(o, format!("difficulty {d} is not 0..=3"));
    }
    p.bool()?; // locked
    Ok(())
}

/// Plain registry holder (VarInt) used by item and holder-registry codecs.
fn source(p: &mut P) -> R<()> {
    let o = p.pos();
    let s = p.varint()?;
    if !(0..=10).contains(&s) {
        return p.bad(o, format!("sound source {s} is not 0..=10"));
    }
    Ok(())
}

/// SoundEvent holder: VarInt 0 = inline Identifier + Optional<f32>; else registry id + 1.
fn sound_holder(p: &mut P) -> R<()> {
    let o = p.pos();
    let h = p.varint()?;
    if h < 0 {
        return p.bad(o, format!("negative sound holder {h}"));
    }
    if h == 0 {
        p.string(32767)?; // identifier
        if p.bool()? {
            p.f32()?; // fixed range
        }
    }
    Ok(())
}

/// LpVec3 (spec 1.6): one byte 0 means zero; otherwise 6 bytes, plus a VarInt
/// when bit 2 of the first byte is set.
fn lp_vec3(p: &mut P) -> R<()> {
    let b0 = p.u8()?;
    if b0 == 0 {
        return Ok(());
    }
    p.u8()?;
    p.i32()?;
    if b0 & 4 != 0 {
        p.varint()?;
    }
    Ok(())
}

fn add_entity(p: &mut P, ctx: &mut Ctx) -> R<()> {
    let id = p.varint()?;
    p.uuid()?;
    let o = p.pos();
    let ty = p.varint()?;
    if ty < 0 {
        return p.bad(o, format!("negative entity type id {ty}"));
    }
    for _ in 0..3 {
        p.f64()?;
    }
    lp_vec3(p)?;
    p.i8()?; // x rot
    p.i8()?; // y rot
    p.i8()?; // y head rot
    p.varint()?; // data
    ctx.entities.insert(id);
    Ok(())
}

fn position_sync(p: &mut P, ctx: &mut Ctx) -> R<()> {
    let id = p.varint()?;
    let o = p.pos();
    match p.varint()? {
        0 => {
            for _ in 0..3 {
                p.f64()?;
            }
        }
        1 => {
            let n = p.count(1 << 16)?;
            for _ in 0..n {
                for _ in 0..3 {
                    p.f64()?;
                }
                p.varint()?; // tick offset
            }
        }
        t => return p.bad(o, format!("unknown PositionPath type {t}")),
    }
    p.f32()?; // y rot
    p.f32()?; // x rot
    p.bool()?; // on ground
    note_entity(ctx, id, 35);
    Ok(())
}

fn explode(p: &mut P) -> R<()> {
    for _ in 0..3 {
        p.f64()?;
    }
    p.f32()?; // radius
    p.i32()?; // block count (INT)
    if p.bool()? {
        for _ in 0..3 {
            p.f64()?; // player knockback
        }
    }
    particle(p)?;
    sound_holder(p)?;
    let n = p.count(1 << 12)?;
    for _ in 0..n {
        particle(p)?;
        p.f32()?; // scaling
        p.f32()?; // speed
        p.varint()?; // weight
    }
    p.bool()?; // play sound
    Ok(())
}

/// ParticleOptions: VarInt type id, then the per-type payload.
/// Only SimpleParticleType (no payload) is decoded; others are unsupported.
fn particle(p: &mut P) -> R<()> {
    let o = p.pos();
    let id = p.varint()?;
    if id < 0 || id as usize >= PARTICLE_SIMPLE.len() {
        return p.bad(o, format!("particle type id {id} out of range"));
    }
    if PARTICLE_SIMPLE[id as usize] {
        return Ok(());
    }
    Err(Fail::Unsup(format!("particle payload not supported (particle type id {id})")))
}

fn light_update(p: &mut P) -> R<()> {
    p.varint()?; // chunk x
    p.varint()?; // chunk z
    let sky = bitset(p)?;
    let block = bitset(p)?;
    bitset(p)?; // empty sky
    bitset(p)?; // empty block
    let sky_updates = light_arrays(p)?;
    let block_updates = light_arrays(p)?;
    if sky_updates != sky {
        return p.bad(p.pos(), format!("sky arrays {sky_updates} != sky mask bits {sky}"));
    }
    if block_updates != block {
        return p.bad(p.pos(), format!("block arrays {block_updates} != block mask bits {block}"));
    }
    Ok(())
}

/// BitSet: VarInt byte length + bytes. Returns the number of set bits.
fn bitset(p: &mut P) -> R<u32> {
    let o = p.pos();
    let n = p.varint()?;
    if !(0..=512).contains(&n) {
        return p.bad(o, format!("bitset byte length {n}"));
    }
    let start = p.pos();
    p.skip(n as usize)?;
    let bits = p.r.data[start..start + n as usize].iter().map(|b| b.count_ones()).sum();
    Ok(bits)
}

/// VarInt count, then per entry VarInt 2048 + 2048 bytes. Returns the count.
fn light_arrays(p: &mut P) -> R<u32> {
    let n = p.count(128)?;
    for _ in 0..n {
        let o = p.pos();
        let len = p.varint()?;
        if len != 2048 {
            return p.bad(o, format!("light array length {len}, expected 2048"));
        }
        p.skip(2048)?;
    }
    Ok(n as u32)
}

fn login(p: &mut P, ctx: &mut Ctx) -> R<()> {
    let eid = p.i32()?;
    ctx.entities.insert(eid);
    p.bool()?; // hardcore
    let n = p.count(4096)?;
    for _ in 0..n {
        p.string(32767)?; // level identifier
    }
    p.varint()?; // max players
    p.varint()?; // view distance
    p.varint()?; // simulation distance
    p.bool()?; // reduced debug info
    p.bool()?; // show death screen
    p.bool()?; // limited crafting
    common_spawn_info(p)?;
    p.bool()?; // online mode
    p.bool()?; // enforces secure chat
    Ok(())
}

/// CommonPlayerSpawnInfo (spec 1, 4.17).
fn common_spawn_info(p: &mut P) -> R<()> {
    p.varint()?; // dimension type holder (plain registry id)
    p.string(32767)?; // dimension identifier
    p.i64()?; // hashed seed
    let o = p.pos();
    let gm = p.varint()?;
    if !(0..=3).contains(&gm) {
        return p.bad(o, format!("game type {gm} is not 0..=3"));
    }
    let o = p.pos();
    let prev = p.varint()?;
    if !(0..=4).contains(&prev) {
        return p.bad(o, format!("previous game type {prev} is not 0 (absent) or 1..=4"));
    }
    p.bool()?; // debug
    p.bool()?; // flat
    if p.bool()? {
        // Optional<GlobalPos>
        p.string(32767)?;
        p.i64()?;
    }
    p.varint()?; // portal cooldown
    p.varint()?; // sea level
    Ok(())
}

/// GlobalPos / RespawnData head: Identifier dimension + i64 BlockPos.
fn identifier_pos(p: &mut P) -> R<()> {
    p.string(32767)?;
    p.i64()?;
    Ok(())
}

fn set_time(p: &mut P) -> R<()> {
    p.i64()?; // game time
    let n = p.count(1 << 12)?;
    for _ in 0..n {
        p.varint()?; // WorldClock holder (plain registry id)
        p.varlong()?; // total ticks
        p.f32()?; // partial tick
        p.f32()?; // rate
    }
    Ok(())
}

fn move_entity(p: &mut P, ctx: &mut Ctx, pk: i32) -> R<()> {
    let id = p.varint()?;
    if pk == 57 {
        // Rot: onGround is a plain bool here, not in the properties VarInt.
        p.bool()?;
        p.i8()?;
        p.i8()?;
    } else {
        let o = p.pos();
        let props = p.varint()?;
        // stepCount = properties >>> 1 (spec 2.2); 0 means linear delta.
        let steps = ((props as u32) >> 1) as usize;
        if steps == 0 {
            p.skip(6)?;
        } else {
            if steps > p.left() / 7 {
                return p.bad(o, format!("stepped VecDelta of {steps} steps does not fit the body"));
            }
            for _ in 0..steps {
                p.varint()?; // ticks
                p.skip(6)?;
            }
        }
        if pk == 55 {
            p.i8()?; // y rot
            p.i8()?; // x rot
        }
    }
    note_entity(ctx, id, pk);
    Ok(())
}

fn remove_entities(p: &mut P, ctx: &mut Ctx) -> R<()> {
    let n = p.count(1 << 16)?;
    for _ in 0..n {
        let id = p.varint()?;
        if ctx.entities.remove(&id) {
            continue;
        }
        note_entity(ctx, id, 78);
    }
    Ok(())
}

fn player_info_update(p: &mut P) -> R<()> {
    let mask = p.u8()?; // EnumSet<Action> as a fixed 1-byte bitset
    let n = p.count(1 << 16)?;
    for _ in 0..n {
        p.uuid()?;
        if mask & 0x01 != 0 {
            // ADD_PLAYER: name (max 16) + properties
            p.string(16)?;
            let np = p.count(16)?;
            for _ in 0..np {
                p.string(64)?; // property name
                p.string(32767)?; // value
                if p.bool()? {
                    p.string(1024)?; // signature
                }
            }
        }
        if mask & 0x02 != 0 && p.bool()? {
            // INITIALIZE_CHAT: session uuid, expiry, public key, key signature
            p.uuid()?;
            p.i64()?; // expires at
            p.byte_array(512)?; // X.509 public key bytes
            p.byte_array(4096)?; // signature
        }
        if mask & 0x04 != 0 {
            p.varint()?; // game type
        }
        if mask & 0x08 != 0 {
            p.bool()?; // listed
        }
        if mask & 0x10 != 0 {
            p.varint()?; // latency
        }
        if mask & 0x20 != 0 && p.bool()? {
            p.component()?; // display name
        }
        if mask & 0x40 != 0 {
            p.varint()?; // list order
        }
        if mask & 0x80 != 0 {
            p.bool()?; // hat
        }
    }
    Ok(())
}

fn entity_data(p: &mut P, ctx: &mut Ctx) -> R<()> {
    let id = p.varint()?;
    loop {
        let index = p.u8()?;
        if index == 0xFF {
            break;
        }
        let so = p.pos();
        let ser = p.varint()?;
        entity_value(p, ser, so)?;
    }
    note_entity(ctx, id, 101);
    Ok(())
}

/// Value codec for EntityDataSerializers ids (spec 3.2). Only the listed ids are decoded.
fn entity_value(p: &mut P, ser: i32, so: usize) -> R<()> {
    match ser {
        0 => {
            p.i8()?;
        }
        1 => {
            p.varint()?;
        }
        2 => {
            p.varlong()?;
        }
        3 => {
            p.f32()?;
        }
        4 => p.string(32767)?,
        5 => p.component()?,
        6 => {
            if p.bool()? {
                p.component()?;
            }
        }
        7 => item_stack(p)?,
        8 => {
            p.bool()?;
        }
        9 => {
            for _ in 0..3 {
                p.f32()?;
            }
        }
        10 => {
            p.i64()?;
        }
        11 => {
            if p.bool()? {
                p.i64()?;
            }
        }
        12 | 14 | 15 | 19 | 20 => {
            p.varint()?;
        }
        42 => {
            let o = p.pos();
            let arm = p.varint()?;
            if !(0..=1).contains(&arm) {
                return p.bad(o, format!("humanoid arm {arm} is not 0 or 1 (value at body offset {so})"));
            }
        }
        _ => return Err(Fail::Unsup(format!("entity data serializer {ser} not supported"))),
    }
    Ok(())
}

fn equipment(p: &mut P) -> R<()> {
    p.varint()?; // entity id
    loop {
        let b = p.u8()?;
        item_stack(p)?;
        if b & 0x80 == 0 {
            break;
        }
    }
    Ok(())
}

/// ItemStack OPTIONAL_STREAM_CODEC: VarInt count; if > 0, holder id + DataComponentPatch.
fn item_stack(p: &mut P) -> R<()> {
    let count = p.varint()?;
    if count <= 0 {
        return Ok(());
    }
    p.varint()?; // item registry id (plain)
    patch(p)
}

/// DataComponentPatch: VarInt set count, VarInt removed count, then values.
/// Only the empty patch is decoded.
fn patch(p: &mut P) -> R<()> {
    let o = p.pos();
    let set = p.varint()?;
    let removed = p.varint()?;
    if set < 0 || removed < 0 {
        return p.bad(o, format!("negative component patch counts {set}/{removed}"));
    }
    if set != 0 || removed != 0 {
        return Err(Fail::Unsup("component patch not supported by validator".into()));
    }
    Ok(())
}

/// ClientboundCommandsPacket (spec 4.18): node graph, then root index.
fn commands(p: &mut P) -> R<()> {
    let o = p.pos();
    let n = p.varint()?;
    if !(1..=100_000).contains(&n) {
        return p.bad(o, format!("command node count {n}"));
    }
    let n = n as usize;
    let in_range = |idx: i32| idx >= 0 && (idx as usize) < n;
    for i in 0..n {
        let fo = p.pos();
        let flags = p.u8()?;
        let ty = flags & 0x03;
        let nc = p.varint()?;
        if nc < 0 || nc as usize > n {
            return p.bad(fo, format!("node {i}: child count {nc} out of range"));
        }
        for _ in 0..nc {
            let co = p.pos();
            let c = p.varint()?;
            if !in_range(c) {
                return p.bad(co, format!("node {i}: child index {c} out of range (node count {n})"));
            }
        }
        if flags & 0x08 != 0 {
            let ro = p.pos();
            let r = p.varint()?;
            if !in_range(r) {
                return p.bad(ro, format!("node {i}: redirect index {r} out of range (node count {n})"));
            }
        }
        match ty {
            0 => {}
            1 => p.string(32767)?, // literal
            2 => {
                p.string(32767)?; // argument name
                let po = p.pos();
                let parser = p.varint()?;
                argument_props(p, parser, po)?;
                if flags & 0x10 != 0 {
                    p.string(32767)?; // suggestion identifier
                }
            }
            _ => return p.bad(fo, format!("node {i}: invalid node type 3")),
        }
    }
    let ro = p.pos();
    let root = p.varint()?;
    if !in_range(root) {
        return p.bad(ro, format!("root index {root} out of range (node count {n})"));
    }
    Ok(())
}

/// Argument parser properties by ArgumentTypeInfos registry id (spec 4.18.1, source-checked).
fn argument_props(p: &mut P, id: i32, off: usize) -> R<()> {
    match id {
        1 => {
            let f = p.u8()?;
            if f & 1 != 0 {
                p.f32()?;
            }
            if f & 2 != 0 {
                p.f32()?;
            }
        }
        2 => {
            let f = p.u8()?;
            if f & 1 != 0 {
                p.f64()?;
            }
            if f & 2 != 0 {
                p.f64()?;
            }
        }
        3 => {
            let f = p.u8()?;
            if f & 1 != 0 {
                p.i32()?;
            }
            if f & 2 != 0 {
                p.i32()?;
            }
        }
        4 => {
            let f = p.u8()?;
            if f & 1 != 0 {
                p.i64()?;
            }
            if f & 2 != 0 {
                p.i64()?;
            }
        }
        5 => {
            let o = p.pos();
            let t = p.varint()?;
            if !(0..=2).contains(&t) {
                return p.bad(o, format!("string argument type {t} is not 0..=2"));
            }
        }
        6 => {
            p.u8()?; // single | players only
        }
        // Singleton parsers (SingletonArgumentInfo, empty body): 0, 7..=30, 32..=42, 49..=61.
        0 | 7..=30 | 32..=42 | 49..=61 => {}
        // Parsers with a non-empty Info body that this validator does not decode.
        31 | 43..=48 => {
            return Err(Fail::Unsup(format!("argument parser id {id} (body not decoded) at offset {off}")))
        }
        _ => return Err(Fail::Unsup(format!("unknown argument parser id {id} at offset {off}"))),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(id: i32, body: &[u8]) -> Result<Outcome, ParseError> {
        validate(id, body, &mut Ctx::default())
    }

    #[test]
    fn lp_vec3_zero_is_one_byte() {
        // set_entity_motion: VarInt entity 1, LpVec3 zero (single 0x00 byte).
        assert_eq!(run(103, &[0x01, 0x00]).unwrap(), Outcome::Ok);
        // An extra byte after the zero vector is a trailing-bytes error.
        let e = run(103, &[0x01, 0x00, 0x00]).unwrap_err();
        assert_eq!(e.offset, 2);
        assert!(e.msg.contains("trailing"), "{}", e.msg);
    }

    #[test]
    fn lp_vec3_partial_reads_continuation_varint() {
        // First byte 0x04 (bit 2 set): 6 bytes plus one VarInt.
        let body = [0x01, 0x04, 0, 0, 0, 0, 0, 0x05];
        assert_eq!(run(103, &body).unwrap(), Outcome::Ok);
    }

    #[test]
    fn entity_data_needs_0xff_terminator() {
        // entity 5: index 0 BYTE 7, then terminator.
        assert_eq!(run(101, &[0x05, 0x00, 0x00, 0x07, 0xFF]).unwrap(), Outcome::Ok);
        let e = run(101, &[0x05, 0x00, 0x00, 0x07]).unwrap_err();
        assert!(e.msg.contains("end of packet") || e.msg.contains("Eof") || e.msg.contains("unexpected"), "{}", e.msg);
    }

    #[test]
    fn commands_child_index_out_of_range_is_error() {
        // 2 nodes. node0: root, child 5 (invalid). node1: root. root index 0.
        let body = [0x02, 0x00, 0x01, 0x05, 0x00, 0x00, 0x00];
        let e = run(16, &body).unwrap_err();
        assert!(e.msg.contains("child index 5 out of range"), "{}", e.msg);
    }

    #[test]
    fn commands_valid_graph_passes() {
        // 2 nodes: node0 root (flags 0x00) with child 1; node1 literal "x"
        // (flags 0x01, no children, literal = VarInt len 1 + 'x'). Root index 0.
        let body = [0x02, 0x00, 0x01, 0x01, 0x01, 0x00, 0x01, b'x', 0x00];
        assert_eq!(run(16, &body).unwrap(), Outcome::Ok);
    }

    #[test]
    fn non_empty_component_patch_is_unsupported() {
        // container_set_slot: container 1, state 2, slot 3, item count 1, holder 5, patch with one added component.
        let body = [0x01, 0x02, 0x00, 0x03, 0x01, 0x05, 0x01, 0x00];
        assert_eq!(
            run(20, &body).unwrap(),
            Outcome::Unsupported("component patch not supported by validator".into())
        );
        // Empty patch is fine.
        let ok = [0x01, 0x02, 0x00, 0x03, 0x01, 0x05, 0x00, 0x00];
        assert_eq!(run(20, &ok).unwrap(), Outcome::Ok);
    }

    #[test]
    fn unlisted_packet_is_unvalidated() {
        assert_eq!(run(0, &[]).unwrap(), Outcome::Unvalidated);
    }

    #[test]
    fn chunk_batch_start_is_empty() {
        assert_eq!(run(12, &[]).unwrap(), Outcome::Ok);
        assert!(run(12, &[0x00]).is_err());
    }
}
