//! Command blocks (impulse / chain / repeat): block entity state, the GUI
//! (`ServerPlayer.openCommandBlock`, `handleSetCommandBlock`) and
//! execution (`BaseCommandBlock`, `CommandBlockEntity`, `CommandBlock`).
//!
//! The chunk's block entity NBT is the single source of truth (so /data,
//! /clone and saving see the same state); parsed commands are cached by
//! the command parse cache, so repeating blocks never re-parse.
//! Scheduled ticks run in vanilla order (trigger tick, then scheduling
//! order). Redstone power is read through `neighbor_signal`.

use crate::game::Game;
use ahash::AHashSet;
use rine_protocol::buf::BlockPos;
use rine_protocol::nbt::Tag;
use rine_protocol::text::Text;
use rine_world::ChunkPos;
use std::collections::BTreeSet;

const KINDS: [&str; 3] = ["command_block", "chain_command_block", "repeating_command_block"];

/// `CommandBlockEntity.Mode`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Sequence,
    Auto,
    Redstone,
}

#[derive(Default)]
pub struct State {
    /// Known command block positions (discovered on use, packet, scans).
    known: AHashSet<[i32; 3]>,
    /// Scheduled block ticks: (trigger tick, order, pos).
    ticks: BTreeSet<(u64, u64, [i32; 3])>,
    scheduled: AHashSet<[i32; 3]>,
    seq: u64,
}

/// Parsed block entity (`BaseCommandBlock` + `CommandBlockEntity`).
#[derive(Clone, Debug)]
pub struct Data {
    pub command: String,
    pub success_count: i32,
    pub custom_name: Option<Tag>,
    pub track_output: bool,
    pub last_output: Option<Tag>,
    pub update_last_execution: bool,
    pub last_execution: i64,
    pub powered: bool,
    pub condition_met: bool,
    pub auto: bool,
}

impl Default for Data {
    fn default() -> Self {
        Self {
            command: String::new(),
            success_count: 0,
            custom_name: None,
            track_output: true,
            last_output: None,
            update_last_execution: true,
            last_execution: -1,
            powered: false,
            condition_met: false,
            auto: false,
        }
    }
}

impl Data {
    fn load(t: &Tag) -> Data {
        let b = |k: &str, d: bool| match t.get(k) {
            Some(Tag::Byte(v)) => *v != 0,
            _ => d,
        };
        let track_output = b("TrackOutput", true);
        let update_last_execution = b("UpdateLastExecution", true);
        Data {
            command: match t.get("Command") {
                Some(Tag::String(s)) => s.clone(),
                _ => String::new(),
            },
            success_count: match t.get("SuccessCount") {
                Some(Tag::Int(v)) => *v,
                _ => 0,
            },
            custom_name: t.get("CustomName").cloned(),
            track_output,
            last_output: if track_output { t.get("LastOutput").cloned() } else { None },
            update_last_execution,
            last_execution: match (update_last_execution, t.get("LastExecution")) {
                (true, Some(Tag::Long(v))) => *v,
                _ => -1,
            },
            powered: b("powered", false),
            condition_met: b("conditionMet", false),
            auto: b("auto", false),
        }
    }

    /// `saveAdditional` (without metadata).
    fn save(&self) -> Tag {
        let mut t = Tag::compound().with("Command", Tag::String(self.command.clone())).with("SuccessCount", Tag::Int(self.success_count));
        if let Some(n) = &self.custom_name {
            t = t.with("CustomName", n.clone());
        }
        t = t.with("TrackOutput", Tag::Byte(self.track_output as i8));
        if self.track_output {
            if let Some(o) = &self.last_output {
                t = t.with("LastOutput", o.clone());
            }
        }
        t = t.with("UpdateLastExecution", Tag::Byte(self.update_last_execution as i8));
        if self.update_last_execution && self.last_execution != -1 {
            t = t.with("LastExecution", Tag::Long(self.last_execution));
        }
        t.with("powered", Tag::Byte(self.powered as i8)).with("conditionMet", Tag::Byte(self.condition_met as i8)).with("auto", Tag::Byte(self.auto as i8))
    }

    /// `getName()`: custom name or "@".
    fn name(&self) -> Text {
        match &self.custom_name {
            Some(t) => super::select::text_from_tag(t),
            None => Text::plain("@"),
        }
    }
}

fn block_kind(g: &Game, pos: [i32; 3]) -> Option<(usize, u16)> {
    let s = g.world.get_block(pos[0], pos[1], pos[2]);
    let name = rine_data::get().blocks.block_of(s).name.as_str();
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    KINDS.iter().position(|k| *k == name).map(|i| (i, s))
}

/// Mode from the block (`CommandBlockEntity.getMode`).
fn mode_of(kind: usize) -> Mode {
    match kind {
        1 => Mode::Sequence,
        2 => Mode::Auto,
        _ => Mode::Redstone,
    }
}

fn facing(s: u16) -> [i32; 3] {
    match rine_data::get().blocks.get_prop(s, "facing") {
        Some("down") => [0, -1, 0],
        Some("up") => [0, 1, 0],
        Some("north") => [0, 0, -1],
        Some("south") => [0, 0, 1],
        Some("west") => [-1, 0, 0],
        _ => [1, 0, 0],
    }
}

fn is_conditional(s: u16) -> bool {
    rine_data::get().blocks.get_prop(s, "conditional") == Some("true")
}

fn be_type_id() -> u32 {
    rine_data::get().registries.find("minecraft:block_entity_type").and_then(|r| r.id("minecraft:command_block")).unwrap_or(0)
}

/// Read the block entity (`None` when the block is not a command block).
pub fn read(g: &Game, pos: [i32; 3]) -> Option<Data> {
    block_kind(g, pos)?;
    let slot = g.world.chunks.get(&ChunkPos::new(pos[0] >> 4, pos[2] >> 4))?;
    let be = slot.chunk.block_entities.iter().find(|b| b.x as i32 == pos[0] & 15 && b.z as i32 == pos[2] & 15 && b.y as i32 == pos[1]);
    Some(be.and_then(|b| b.nbt.as_ref()).map(Data::load).unwrap_or_default())
}

/// Write the block entity back (creating it when missing).
pub fn write(g: &mut Game, pos: [i32; 3], d: &Data) {
    let Some(slot) = g.world.chunks.get_mut(&ChunkPos::new(pos[0] >> 4, pos[2] >> 4)) else { return };
    let (lx, lz) = ((pos[0] & 15) as u8, (pos[2] & 15) as u8);
    let mut nbt = d.save();
    let ch = &mut slot.chunk;
    match ch.block_entities.iter_mut().find(|b| b.x == lx && b.z == lz && b.y as i32 == pos[1]) {
        Some(be) => {
            // keep keys we do not model (components, other data)
            if let (Some(Tag::Compound(old)), Tag::Compound(new)) = (&be.nbt, &mut nbt) {
                for (k, v) in old {
                    if !new.iter().any(|(nk, _)| nk == k) && !matches!(k.as_str(), "LastOutput" | "LastExecution" | "CustomName") {
                        new.push((k.clone(), v.clone()));
                    }
                }
            }
            be.nbt = Some(nbt);
        }
        None => ch.block_entities.push(rine_world::chunk::BlockEntity { x: lx, y: pos[1] as i16, z: lz, type_id: be_type_id(), nbt: Some(nbt) }),
    }
    ch.dirty = true;
    g.cs_cmdblock().known.insert(pos);
}

impl Game {
    fn cs_cmdblock(&mut self) -> &mut State {
        &mut self.cs.cmdblock
    }
}

/// `TrackOutput` of the command block at `pos`.
pub fn tracks_output(g: &Game, pos: [i32; 3]) -> bool {
    read(g, pos).is_none_or(|d| d.track_output)
}

/// `CloseableCommandBlockSource.sendSystemMessage`: `[HH:mm:ss] message`
/// as `LastOutput`.
pub fn set_last_output(g: &mut Game, pos: [i32; 3], t: &Text) {
    let Some(mut d) = read(g, pos) else { return };
    if !d.track_output {
        return;
    }
    let now = chrono_hms();
    d.last_output = Some(Text::plain(format!("[{now}] ")).push(t.clone()).to_nbt());
    write(g, pos, &d);
}

/// Local wall-clock time as `HH:mm:ss`.
fn chrono_hms() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let offset = local_offset_secs();
    let t = (secs as i64 + offset).rem_euclid(86400);
    format!("{:02}:{:02}:{:02}", t / 3600, t % 3600 / 60, t % 60)
}

#[cfg(windows)]
fn local_offset_secs() -> i64 {
    #[repr(C)]
    struct Tzi {
        bias: i32,
        _rest: [u8; 168],
    }
    extern "system" {
        fn GetTimeZoneInformation(tzi: *mut Tzi) -> u32;
    }
    let mut tzi = Tzi { bias: 0, _rest: [0; 168] };
    // SAFETY: plain Win32 call filling a caller-owned struct of the
    // documented size (172 bytes); the return value 2 means daylight time.
    let r = unsafe { GetTimeZoneInformation(&mut tzi) };
    let daylight = if r == 2 { i32::from_le_bytes([tzi._rest[164], tzi._rest[165], tzi._rest[166], tzi._rest[167]]) } else { 0 };
    -((tzi.bias + daylight) as i64) * 60
}

#[cfg(not(windows))]
fn local_offset_secs() -> i64 {
    0
}

fn schedule(g: &mut Game, pos: [i32; 3], delay: u64) {
    let st = &mut g.cs.cmdblock;
    if !st.scheduled.insert(pos) {
        return;
    }
    st.seq += 1;
    let t = g.tick + delay;
    st.ticks.insert((t, st.seq, pos));
}

/// `CommandBlockEntity.markConditionMet`.
fn mark_condition_met(g: &mut Game, pos: [i32; 3], d: &mut Data) -> bool {
    d.condition_met = true;
    let Some((_, s)) = block_kind(g, pos) else { return true };
    if is_conditional(s) {
        let f = facing(s);
        let behind = [pos[0] - f[0], pos[1] - f[1], pos[2] - f[2]];
        d.condition_met = read(g, behind).is_some_and(|b| b.success_count > 0);
    }
    d.condition_met
}

/// `CommandBlock.setPoweredAndUpdate`.
fn set_powered(g: &mut Game, pos: [i32; 3], powered: bool) {
    let Some(mut d) = read(g, pos) else { return };
    if d.powered == powered {
        return;
    }
    d.powered = powered;
    let Some((kind, _)) = block_kind(g, pos) else { return };
    if powered && !d.auto && mode_of(kind) != Mode::Sequence {
        mark_condition_met(g, pos, &mut d);
        write(g, pos, &d);
        schedule(g, pos, 1);
    } else {
        write(g, pos, &d);
    }
}

/// `CommandBlockEntity.setAutomatic`.
fn set_automatic(g: &mut Game, pos: [i32; 3], d: &mut Data, auto: bool) {
    let was = d.auto;
    d.auto = auto;
    let Some((kind, _)) = block_kind(g, pos) else { return };
    if !was && auto && !d.powered && mode_of(kind) != Mode::Sequence {
        mark_condition_met(g, pos, d);
        write(g, pos, d);
        schedule(g, pos, 1);
    }
}

/// `BaseCommandBlock.performCommand`: false when it already ran this tick.
fn perform(g: &mut Game, pos: [i32; 3], d: &mut Data) -> bool {
    let now = g.tick as i64;
    if now == d.last_execution {
        return false;
    }
    if d.command.eq_ignore_ascii_case("Searge") {
        d.last_output = Some(Text::plain("#itzlipofutzli").to_nbt());
        d.success_count = 1;
        return true;
    }
    d.success_count = 0;
    if g.shared.config.enable_command_block && !d.command.is_empty() {
        d.last_output = None;
        write(g, pos, d);
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0));
        let c2 = counter.clone();
        let cb: super::exec::Callback = Some(std::sync::Arc::new(move |_g: &mut Game, ok, _v| {
            if ok {
                c2.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }));
        let name = d.name();
        let src = super::source::Source::command_block(pos, &super::render_plain(&name), name).with_callback(cb);
        super::perform(g, src, &d.command);
        // re-read: the command may have changed this block (output, data)
        if let Some(cur) = read(g, pos) {
            d.last_output = cur.last_output;
            d.command = cur.command;
            d.track_output = cur.track_output;
            d.custom_name = cur.custom_name;
        }
        d.success_count = counter.load(std::sync::atomic::Ordering::Relaxed);
    }
    d.last_execution = if d.update_last_execution { now } else { -1 };
    true
}

/// `CommandBlock.tick` (a scheduled tick).
fn tick_block(g: &mut Game, pos: [i32; 3]) {
    let Some((kind, s)) = block_kind(g, pos) else { return };
    let Some(mut d) = read(g, pos) else { return };
    let has_command = !d.command.is_empty();
    let was_met = d.condition_met;
    match mode_of(kind) {
        Mode::Auto => {
            mark_condition_met(g, pos, &mut d);
            if was_met {
                execute(g, pos, s, &mut d, has_command);
            } else if is_conditional(s) {
                d.success_count = 0;
            }
            write(g, pos, &d);
            if d.powered || d.auto {
                schedule(g, pos, 1);
            }
        }
        Mode::Redstone => {
            if was_met {
                execute(g, pos, s, &mut d, has_command);
            } else if is_conditional(s) {
                d.success_count = 0;
            }
            write(g, pos, &d);
        }
        Mode::Sequence => {}
    }
}

/// `CommandBlock.execute` + `executeChain`.
fn execute(g: &mut Game, pos: [i32; 3], s: u16, d: &mut Data, has_command: bool) {
    if has_command {
        perform(g, pos, d);
    } else {
        d.success_count = 0;
    }
    write(g, pos, d);
    let mut p = pos;
    let mut dir = facing(s);
    let mut left = g.rules.int("max_command_sequence_length");
    while left > 0 {
        left -= 1;
        p = [p[0] + dir[0], p[1] + dir[1], p[2] + dir[2]];
        let Some((1, ns)) = block_kind(g, p) else { break };
        let Some(mut nd) = read(g, p) else { break };
        if nd.powered || nd.auto {
            if mark_condition_met(g, p, &mut nd) {
                let ran = perform(g, p, &mut nd);
                write(g, p, &nd);
                if !ran {
                    break;
                }
            } else {
                if is_conditional(ns) {
                    nd.success_count = 0;
                }
                write(g, p, &nd);
            }
        }
        dir = facing(ns);
    }
    if left <= 0 {
        tracing::warn!("Command Block chain tried to execute more than {} steps!", g.rules.int("max_command_sequence_length").max(0));
    }
}

/// `Level.hasNeighborSignal` for the direct power sources Rine models.
pub fn neighbor_signal(g: &Game, pos: [i32; 3]) -> bool {
    const DIRS: [([i32; 3], &str); 6] =
        [([0, -1, 0], "down"), ([0, 1, 0], "up"), ([0, 0, -1], "north"), ([0, 0, 1], "south"), ([-1, 0, 0], "west"), ([1, 0, 0], "east")];
    let blocks = &rine_data::get().blocks;
    DIRS.iter().any(|(d, dname)| {
        let q = [pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]];
        let s = g.world.get_block(q[0], q[1], q[2]);
        let name = blocks.block_of(s).name.as_str();
        let name = name.strip_prefix("minecraft:").unwrap_or(name);
        let prop = |k: &str| blocks.get_prop(s, k);
        // `dname` = direction from this block to the neighbour
        let opposite = match *dname {
            "down" => "up",
            "up" => "down",
            "north" => "south",
            "south" => "north",
            "west" => "east",
            _ => "west",
        };
        match name {
            "redstone_block" => true,
            "lever" => prop("powered") == Some("true"),
            n if n.ends_with("_button") => prop("powered") == Some("true"),
            n if n.ends_with("pressure_plate") => prop("powered") == Some("true") || prop("power").is_some_and(|p| p != "0"),
            "redstone_torch" => prop("lit") == Some("true") && *dname != "up",
            "redstone_wall_torch" => prop("lit") == Some("true") && prop("facing") != Some(dname),
            "repeater" | "comparator" => prop("powered") == Some("true") && prop("facing") == Some(dname),
            "observer" => prop("powered") == Some("true") && prop("facing") == Some(opposite),
            "redstone_wire" => {
                let powered = prop("power").is_some_and(|p| p != "0");
                powered && (*dname == "up" || (*dname != "down" && prop(opposite).is_some_and(|c| c != "none")))
            }
            "daylight_detector" | "target" | "sculk_sensor" | "calibrated_sculk_sensor" => prop("power").is_some_and(|p| p != "0"),
            _ => false,
        }
    })
}

/// Per-tick work: power edges of known blocks, scheduled ticks, and a
/// periodic scan of loaded chunks for command blocks.
pub fn tick(g: &mut Game) {
    if g.tick % 20 == 0 {
        scan(g);
    }
    let known: Vec<[i32; 3]> = g.cs.cmdblock.known.iter().copied().collect();
    for pos in known {
        if !g.world.is_loaded(pos[0], pos[2]) || block_kind(g, pos).is_none() {
            g.cs.cmdblock.known.remove(&pos);
            continue;
        }
        let p = neighbor_signal(g, pos);
        set_powered(g, pos, p);
    }
    loop {
        let Some(&first) = g.cs.cmdblock.ticks.iter().next() else { break };
        if first.0 > g.tick {
            break;
        }
        g.cs.cmdblock.ticks.remove(&first);
        g.cs.cmdblock.scheduled.remove(&first.2);
        tick_block(g, first.2);
    }
}

/// Find command blocks in loaded chunks; restart automatic / powered
/// ones found for the first time (their saved block ticks).
fn scan(g: &mut Game) {
    let ty = be_type_id();
    let mut found = Vec::new();
    for (cp, slot) in g.world.chunks.iter() {
        for be in &slot.chunk.block_entities {
            if be.type_id == ty {
                let pos = [cp.x * 16 + be.x as i32, be.y as i32, cp.z * 16 + be.z as i32];
                if !g.cs.cmdblock.known.contains(&pos) {
                    found.push(pos);
                }
            }
        }
    }
    for pos in found {
        let Some((kind, _)) = block_kind(g, pos) else { continue };
        g.cs.cmdblock.known.insert(pos);
        if let Some(d) = read(g, pos) {
            if (d.auto || d.powered) && mode_of(kind) != Mode::Sequence {
                schedule(g, pos, 1);
            }
        }
    }
}

/// `CommandBlock.useWithoutItem`: op in creative opens the editor.
/// Returns true when the click was handled.
pub fn on_use(g: &mut Game, slot: usize, pos: BlockPos) -> bool {
    let p3 = [pos.x, pos.y, pos.z];
    if block_kind(g, p3).is_none() {
        return false;
    }
    let Some(p) = g.players.get(slot).and_then(|p| p.as_ref()) else { return false };
    let level = g.op_level_of(&p.name);
    if p.gamemode != 1 || level < 2 {
        return false;
    }
    let d = read(g, p3).unwrap_or_default();
    write(g, p3, &d);
    let nbt = d.save();
    let pkt = rine_protocol::frame::packet(rine_protocol::ids::play::C_BLOCK_ENTITY_DATA, |b| {
        use rine_protocol::buf::Writer;
        b.w_i64(pos.pack());
        b.w_varint(be_type_id() as i32);
        nbt.write_network(b);
    });
    if let Some(p) = g.player(slot) {
        p.send(pkt);
    }
    true
}

/// `ServerGamePacketListenerImpl.handleSetCommandBlock`.
pub fn handle_set(g: &mut Game, slot: usize, pos: BlockPos, command: &str, mode: i32, flags: u8) {
    let Some(p) = g.players.get(slot).and_then(|p| p.as_ref()) else { return };
    let (name, gm) = (p.name.clone(), p.gamemode);
    if !g.shared.config.enable_command_block {
        if let Some(p) = g.player(slot) {
            p.msg(&Text::key("advMode.notEnabled"));
        }
        return;
    }
    if gm != 1 || g.op_level_of(&name) < 2 {
        if let Some(p) = g.player(slot) {
            p.msg(&Text::key("advMode.notAllowed"));
        }
        return;
    }
    let p3 = [pos.x, pos.y, pos.z];
    let Some((kind, s)) = block_kind(g, p3) else { return };
    let mut d = read(g, p3).unwrap_or_default();
    let old_mode = mode_of(kind);
    let new_mode = match mode {
        0 => Mode::Sequence,
        1 => Mode::Auto,
        _ => Mode::Redstone,
    };
    let (track, conditional, automatic) = (flags & 1 != 0, flags & 2 != 0, flags & 4 != 0);
    let blocks = &rine_data::get().blocks;
    let target = match new_mode {
        Mode::Sequence => "minecraft:chain_command_block",
        Mode::Auto => "minecraft:repeating_command_block",
        Mode::Redstone => "minecraft:command_block",
    };
    let face = blocks.get_prop(s, "facing").unwrap_or("north").to_string();
    let short = target.trim_start_matches("minecraft:");
    let mut ns = blocks.block(target).or_else(|| blocks.block(short)).map(|b| b.default_state as u16).unwrap_or(s);
    if let Some(v) = blocks.with_prop(ns, "facing", &face) {
        ns = v;
    }
    if let Some(v) = blocks.with_prop(ns, "conditional", if conditional { "true" } else { "false" }) {
        ns = v;
    }
    if ns != s {
        g.world.set_block(pos, ns);
    }
    d.command = command.to_string();
    d.track_output = track;
    if !track {
        d.last_output = None;
    }
    write(g, p3, &d);
    set_automatic(g, p3, &mut d, automatic);
    if old_mode != new_mode && d.auto {
        // `onModeSwitch`
        d.auto = false;
        set_automatic(g, p3, &mut d, true);
    }
    write(g, p3, &d);
    if !command.is_empty() {
        if let Some(p) = g.player(slot) {
            p.msg(&Text::tr("advMode.setCommand.success", vec![Text::plain(command)]));
        }
    }
}
