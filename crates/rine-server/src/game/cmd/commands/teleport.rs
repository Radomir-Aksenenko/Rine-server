//! /teleport /tp, /rotate, /spectate, /spreadplayers (`TeleportCommand`,
//! `RotateCommand`, `SpectateCommand`, `SpreadPlayersCommand`).

use crate::game::cmd::select;
use crate::game::cmd::source::Source;
use crate::game::cmd::{lit, Commands, Ctx};
use crate::game::Game;
use crate::out;
use rine_command::args::coords::{facing, wrap_degrees};
use rine_command::{CmdError, CmdResult};
use rine_protocol::text::Text;
use rine_protocol::Writer;
use std::collections::HashMap;

/// `ClientboundSetCameraPacket` (`minecraft:set_camera`).
const C_SET_CAMERA: i32 = 95;
/// `ClientboundPlayerLookAtPacket` (`minecraft:player_look_at`).
const C_PLAYER_LOOK_AT: i32 = 72;

// `Relative` bits (ordinal order of `Relative.java`).
const REL_X: u32 = 1 << 0;
const REL_Y: u32 = 1 << 1;
const REL_Z: u32 = 1 << 2;
const REL_Y_ROT: u32 = 1 << 3;
const REL_X_ROT: u32 = 1 << 4;
const REL_DELTA_X: u32 = 1 << 5;
const REL_DELTA_Y: u32 = 1 << 6;
const REL_DELTA_Z: u32 = 1 << 7;
const REL_ROTATION: u32 = REL_X_ROT | REL_Y_ROT;
const REL_POSITION: [u32; 3] = [REL_X, REL_Y, REL_Z];

/// `Level.isInSpawnableBounds(BlockPos.containing(pos))`.
pub fn in_spawnable_bounds(p: [f64; 3]) -> bool {
    let b = [p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32];
    !(b[1] < -20_000_000 || b[1] >= 20_000_000) && b[0] >= -30_000_000 && b[2] >= -30_000_000 && b[0] < 30_000_000 && b[2] < 30_000_000
}

/// `Commands.formatDouble`: `String.format(Locale.ROOT, "%f", d)`.
fn format_double(v: f64) -> String {
    format!("{v:.6}")
}

/// `LookAt`: the target looks at a fixed point or at an entity's anchor.
#[derive(Clone, Copy)]
enum Look {
    Pos([f64; 3]),
    /// Entity id and its anchor (0 feet, 1 eyes).
    Entity(i32, u8),
}

pub fn register(c: &mut Commands) {
    c.exec("teleport location", |x| {
        let eid = x.source_entity()?;
        let dest = x.vec3("location");
        let rel = pos_relatives(x, "location", None);
        teleport_pos(x, vec![eid], dest, rel, None, None)
    });
    c.exec("teleport destination", |x| {
        let eid = x.source_entity()?;
        let dest = x.entity("destination")?;
        teleport_entity(x, vec![eid], dest)
    });
    c.exec("teleport targets location", |x| {
        let targets = x.entities("targets")?;
        let dest = x.vec3("location");
        let rel = pos_relatives(x, "location", None);
        teleport_pos(x, targets, dest, rel, None, None)
    });
    c.exec("teleport targets location rotation", |x| {
        let targets = x.entities("targets")?;
        let dest = x.vec3("location");
        let rot = x.rotation("rotation");
        let rel = pos_relatives(x, "location", Some("rotation"));
        teleport_pos(x, targets, dest, rel, Some(rot), None)
    });
    c.exec("teleport targets location facing entity facingEntity", |x| {
        let targets = x.entities("targets")?;
        let dest = x.vec3("location");
        let rel = pos_relatives(x, "location", None);
        let ent = x.entity("facingEntity")?;
        teleport_pos(x, targets, dest, rel, None, Some(Look::Entity(ent, 0)))
    });
    c.exec("teleport targets location facing entity facingEntity facingAnchor", |x| {
        let targets = x.entities("targets")?;
        let dest = x.vec3("location");
        let rel = pos_relatives(x, "location", None);
        let ent = x.entity("facingEntity")?;
        let anchor = x.enum_("facingAnchor");
        teleport_pos(x, targets, dest, rel, None, Some(Look::Entity(ent, anchor)))
    });
    c.exec("teleport targets location facing facingLocation", |x| {
        let targets = x.entities("targets")?;
        let dest = x.vec3("location");
        let rel = pos_relatives(x, "location", None);
        let p = x.vec3("facingLocation");
        teleport_pos(x, targets, dest, rel, None, Some(Look::Pos(p)))
    });
    c.exec("teleport targets destination", |x| {
        let targets = x.entities("targets")?;
        let dest = x.entity("destination")?;
        teleport_entity(x, targets, dest)
    });
    // `tp` is a redirect to `teleport` in the tree; no executors of its own.

    c.exec("rotate target rotation", |x| {
        let target = x.entity("target")?;
        let rot = x.rotation("rotation");
        let coords = x.coords("rotation");
        // rotation coords: [xRot, yRot]; relative flags per component
        let rel_y = coords.is_relative(1);
        let rel_x = coords.is_relative(0);
        let y = if rel_y { rot[1] - select::entity_rot(x.g, target)[1] } else { rot[1] };
        let xr = if rel_x { rot[0] - select::entity_rot(x.g, target)[0] } else { rot[0] };
        force_rotation(x.g, target, y, rel_y, xr, rel_x);
        rotate_done(x, target)
    });
    c.exec("rotate target facing entity facingEntity", |x| {
        let target = x.entity("target")?;
        let ent = x.entity("facingEntity")?;
        look_at(x.g, x.src, target, Look::Entity(ent, 0));
        rotate_done(x, target)
    });
    c.exec("rotate target facing entity facingEntity facingAnchor", |x| {
        let target = x.entity("target")?;
        let ent = x.entity("facingEntity")?;
        let anchor = x.enum_("facingAnchor");
        look_at(x.g, x.src, target, Look::Entity(ent, anchor));
        rotate_done(x, target)
    });
    c.exec("rotate target facing facingLocation", |x| {
        let target = x.entity("target")?;
        let p = x.vec3("facingLocation");
        look_at(x.g, x.src, target, Look::Pos(p));
        rotate_done(x, target)
    });

    c.exec("spectate", |x| {
        let slot = x.source_player()?;
        spectate(x, None, slot)
    });
    c.exec("spectate target", |x| {
        let target = x.entity("target")?;
        let slot = x.source_player()?;
        spectate(x, Some(target), slot)
    });
    c.exec("spectate target player", |x| {
        let target = x.entity("target")?;
        let slot = x.player("player")?;
        spectate(x, Some(target), slot)
    });

    c.exec("spreadplayers center spreadDistance maxRange respectTeams targets", |x| {
        // getMaxY() + 1
        spread_cmd(x, rine_world::MIN_Y + rine_world::HEIGHT)
    });
    c.exec("spreadplayers center spreadDistance maxRange under maxHeight respectTeams targets", |x| {
        let max_height = x.int("maxHeight");
        spread_cmd(x, max_height)
    });
}

/// Relative flags of a destination (`getRelatives`, same dimension).
/// `rot_arg` names the rotation argument, `None` when there is none.
fn pos_relatives(x: &Ctx, name: &str, rot_arg: Option<&str>) -> u32 {
    let c = x.coords(name);
    let mut rel = 0;
    for (i, bit) in REL_POSITION.iter().enumerate() {
        if c.is_relative(i) {
            rel |= bit;
            rel |= [REL_DELTA_X, REL_DELTA_Y, REL_DELTA_Z][i];
        }
    }
    rel |= match rot_arg {
        None => REL_ROTATION,
        Some(r) => {
            let rc = x.coords(r);
            (if rc.is_relative(1) { REL_Y_ROT } else { 0 }) | (if rc.is_relative(0) { REL_X_ROT } else { 0 })
        }
    };
    rel
}

/// `TeleportCommand.teleportToPos`.
fn teleport_pos(x: &mut Ctx, eids: Vec<i32>, dest: [f64; 3], rel: u32, rot: Option<[f32; 2]>, look: Option<Look>) -> CmdResult<i32> {
    for &e in &eids {
        match rot {
            // rot = [xRot, yRot]
            Some(r) => perform_teleport(x.g, x.src, e, dest, rel, r[1], r[0], look)?,
            None => {
                let cur = select::entity_rot(x.g, e);
                perform_teleport(x.g, x.src, e, dest, rel, cur[1], cur[0], look)?;
            }
        }
    }
    let n = eids.len();
    let t = if n == 1 {
        Text::tr(
            "commands.teleport.success.location.single",
            vec![x.name_of(eids[0]), lit(format_double(dest[0])), lit(format_double(dest[1])), lit(format_double(dest[2]))],
        )
    } else {
        Text::tr(
            "commands.teleport.success.location.multiple",
            vec![lit(n), lit(format_double(dest[0])), lit(format_double(dest[1])), lit(format_double(dest[2]))],
        )
    };
    x.success(t, true);
    Ok(n as i32)
}

/// `TeleportCommand.teleportToEntity`.
fn teleport_entity(x: &mut Ctx, eids: Vec<i32>, dest: i32) -> CmdResult<i32> {
    for &e in &eids {
        // destination is read per entity, as in Java
        let p = select::entity_pos(x.g, dest).unwrap_or([0.0; 3]);
        let r = select::entity_rot(x.g, dest);
        perform_teleport(x.g, x.src, e, p, 0, r[1], r[0], None)?;
    }
    let n = eids.len();
    let t = if n == 1 {
        Text::tr("commands.teleport.success.entity.single", vec![x.name_of(eids[0]), x.name_of(dest)])
    } else {
        Text::tr("commands.teleport.success.entity.multiple", vec![lit(n), x.name_of(dest)])
    };
    x.success(t, true);
    Ok(n as i32)
}

/// `TeleportCommand.performTeleport`: bounds check, relative resolution,
/// teleport, optional look, then zero the vertical speed and land.
#[allow(clippy::too_many_arguments)]
fn perform_teleport(g: &mut Game, src: &Source, eid: i32, dest: [f64; 3], rel: u32, y_rot: f32, x_rot: f32, look: Option<Look>) -> CmdResult<()> {
    if !in_spawnable_bounds(dest) {
        return Err(CmdError::key("commands.teleport.invalidPosition"));
    }
    let cur = select::entity_pos(g, eid).unwrap_or([0.0; 3]);
    let cur_rot = select::entity_rot(g, eid);
    let mut change = [0.0f64; 3];
    for i in 0..3 {
        change[i] = if rel & REL_POSITION[i] != 0 { dest[i] - cur[i] } else { dest[i] };
    }
    let y = if rel & REL_Y_ROT != 0 { y_rot - cur_rot[1] } else { y_rot };
    let xr = if rel & REL_X_ROT != 0 { x_rot - cur_rot[0] } else { x_rot };
    teleport_to(g, eid, change, rel, wrap_degrees(y), wrap_degrees(xr));
    if let Some(l) = look {
        look_at(g, src, eid, l);
    }
    // not fall flying: keep horizontal speed, zero vertical, land
    if let Some(&slot) = g.by_eid.get(&eid) {
        if let Some(p) = g.player(slot) {
            p.vel[1] = 0.0;
            p.on_ground = true;
        }
    } else if let Some(i) = g.entities.slot_of(eid) {
        let e = &mut g.entities.list[i];
        e.vel[1] = 0.0;
        e.on_ground = true;
    }
    Ok(())
}

/// `Entity.teleportTo`: `change` holds the offsets for relative axes and
/// absolute values otherwise (what the client expects in the packet).
pub fn teleport_to(g: &mut Game, eid: i32, change: [f64; 3], rel: u32, y_rot: f32, x_rot: f32) {
    let Some(cur) = select::entity_pos(g, eid) else { return };
    let cur_rot = select::entity_rot(g, eid); // [xRot, yRot]
    let mut abs = [0.0f64; 3];
    for i in 0..3 {
        abs[i] = if rel & REL_POSITION[i] != 0 { cur[i] + change[i] } else { change[i] };
    }
    let yaw = if rel & REL_Y_ROT != 0 { cur_rot[1] + y_rot } else { y_rot };
    let pitch = (if rel & REL_X_ROT != 0 { cur_rot[0] + x_rot } else { x_rot }).clamp(-90.0, 90.0);
    if let Some(&slot) = g.by_eid.get(&eid) {
        if let Some(p) = g.player(slot) {
            p.tp_id += 1;
            p.tp_pending = true;
            p.pos = abs;
            p.yaw = yaw;
            p.pitch = pitch;
            p.fall_distance = 0.0;
            let tp_id = p.tp_id;
            let f = rine_protocol::frame::packet(rine_protocol::ids::play::C_PLAYER_POSITION, |b| {
                b.w_varint(tp_id);
                b.w_f64(change[0]);
                b.w_f64(change[1]);
                b.w_f64(change[2]);
                b.w_f64(0.0);
                b.w_f64(0.0);
                b.w_f64(0.0);
                b.w_f32(y_rot);
                b.w_f32(x_rot);
                b.w_i32(rel as i32);
            });
            p.send(f);
            p.stream_dirty = true;
        }
    } else if let Some(i) = g.entities.slot_of(eid) {
        let on_ground = {
            let e = &mut g.entities.list[i];
            e.pos = abs;
            e.sent_pos = abs;
            e.on_ground
        };
        g.ent_meta.entry(eid).or_default().rot = [pitch, yaw];
        let f = out::frame(out::position_sync(eid, abs[0], abs[1], abs[2], yaw, pitch, on_ground));
        crate::game::track::send_to_trackers(g, eid, &f);
    }
}

/// `Entity.getEyePosition`-style anchor point (0 feet, 1 eyes).
fn anchor_pos(g: &Game, eid: i32, anchor: u8) -> [f64; 3] {
    let mut p = select::entity_pos(g, eid).unwrap_or([0.0; 3]);
    if anchor == 1 {
        p[1] += select::eye_height(g, eid);
    }
    p
}

fn set_rot(g: &mut Game, eid: i32, rot: [f32; 2]) {
    if let Some(&slot) = g.by_eid.get(&eid) {
        if let Some(p) = g.player(slot) {
            p.pitch = rot[0];
            p.yaw = rot[1];
        }
    } else {
        g.ent_meta.entry(eid).or_default().rot = rot;
    }
}

/// `Entity.lookAt` (players also get `ClientboundPlayerLookAtPacket`).
fn look_at(g: &mut Game, src: &Source, target: i32, look: Look) {
    let from_anchor: u8 = if src.anchor_eyes { 1 } else { 0 };
    let from = anchor_pos(g, target, from_anchor);
    let (to, tail) = match look {
        Look::Pos(p) => (p, None),
        Look::Entity(e, a) => (anchor_pos(g, e, a), Some((e, a))),
    };
    let rot = facing(from, to);
    set_rot(g, target, rot);
    if g.by_eid.contains_key(&target) {
        let f = rine_protocol::frame::packet(C_PLAYER_LOOK_AT, |b| {
            b.w_varint(from_anchor as i32);
            b.w_f64(to[0]);
            b.w_f64(to[1]);
            b.w_f64(to[2]);
            match tail {
                Some((e, a)) => {
                    b.w_bool(true);
                    b.w_varint(e);
                    b.w_varint(a as i32);
                }
                None => b.w_bool(false),
            }
        });
        if let Some(&slot) = g.by_eid.get(&target) {
            if let Some(p) = g.player(slot) {
                p.send(f);
            }
        }
    }
}

/// `Entity.forceSetRotation` (`calculateAbsolute` on the rotation pair).
/// Players also get `ClientboundPlayerRotationPacket` with the raw values.
fn force_rotation(g: &mut Game, eid: i32, y: f32, rel_y: bool, x: f32, rel_x: bool) {
    let cur = select::entity_rot(g, eid); // [xRot, yRot]
    let yaw = if rel_y { cur[1] + y } else { y };
    let pitch = (if rel_x { cur[0] + x } else { x }).clamp(-90.0, 90.0);
    set_rot(g, eid, [pitch, yaw]);
    if let Some(&slot) = g.by_eid.get(&eid) {
        let f = rine_protocol::frame::packet(rine_protocol::ids::play::C_PLAYER_ROTATION, |b| {
            b.w_f32(y);
            b.w_bool(rel_y);
            b.w_f32(x);
            b.w_bool(rel_x);
        });
        if let Some(p) = g.player(slot) {
            p.send(f);
        }
    }
}

fn rotate_done(x: &mut Ctx, target: i32) -> CmdResult<i32> {
    let name = x.name_of(target);
    x.success(Text::tr("commands.rotate.success", vec![name]), true);
    Ok(1)
}

/// `SpectateCommand.spectate`.
fn spectate(x: &mut Ctx, target: Option<i32>, slot: usize) -> CmdResult<i32> {
    let Some(pe) = x.g.players.get(slot).and_then(|p| p.as_ref()).map(|p| p.eid) else { return Ok(0) };
    if target == Some(pe) {
        return Err(CmdError::key("commands.spectate.self"));
    }
    let gamemode = x.g.players[slot].as_ref().map(|p| p.gamemode).unwrap_or(0);
    if gamemode != 3 {
        return Err(CmdError::tr("commands.spectate.not_spectator", vec![x.player_name(slot)]));
    }
    if let Some(t) = target {
        // every entity kind Rine has tracks at range > 0; the check is kept for parity
        let zero_range = !x.g.by_eid.contains_key(&t) && x.g.entities.get(t).is_some_and(|e| e.kind.track_range() == 0);
        if zero_range {
            return Err(CmdError::tr("commands.spectate.cannot_spectate", vec![x.name_of(t)]));
        }
    }
    let camera = target.unwrap_or(pe);
    // ServerPlayer.setCamera: move to the camera, then send the camera id
    let cam_pos = select::entity_pos(x.g, camera).unwrap_or([0.0; 3]);
    let rot = select::entity_rot(x.g, pe);
    teleport_to(x.g, pe, cam_pos, 0, rot[1], rot[0]);
    let f = rine_protocol::frame::packet(C_SET_CAMERA, |b| b.w_varint(camera));
    if let Some(p) = x.g.player(slot) {
        p.send(f);
    }
    match target {
        Some(t) => {
            let name = x.name_of(t);
            x.success(Text::tr("commands.spectate.success.started", vec![name]), false);
        }
        None => x.success(Text::tr("commands.spectate.success.stopped", vec![]), false),
    }
    Ok(1)
}

/// `Mth.nextDouble(random, min, max)` with a double in [0, 1) from the RNG.
fn next_double(min: f64, max: f64) -> f64 {
    use crate::game::cmd::commands::random::next_u32;
    if min >= max {
        return min;
    }
    let a = (next_u32() >> 5) as u64;
    let b = (next_u32() >> 6) as u64;
    let unit = ((a << 26) + b) as f64 * (1.0 / (1u64 << 53) as f64);
    unit * (max - min) + min
}

#[derive(Clone, Copy, Debug)]
struct Pos {
    x: f64,
    z: f64,
}

impl Pos {
    fn dist(&self, o: &Pos) -> f64 {
        let (dx, dz) = (self.x - o.x, self.z - o.z);
        (dx * dx + dz * dz).sqrt()
    }
    fn length(&self) -> f64 {
        (self.x * self.x + self.z * self.z).sqrt()
    }
    fn randomize(&mut self, b: &Bounds) {
        self.x = next_double(b.min_x, b.max_x);
        self.z = next_double(b.min_z, b.max_z);
    }
    fn clamp(&mut self, b: &Bounds) -> bool {
        let mut changed = false;
        if self.x < b.min_x {
            self.x = b.min_x;
            changed = true;
        } else if self.x > b.max_x {
            self.x = b.max_x;
            changed = true;
        }
        if self.z < b.min_z {
            self.z = b.min_z;
            changed = true;
        } else if self.z > b.max_z {
            self.z = b.max_z;
            changed = true;
        }
        changed
    }
}

#[derive(Clone, Copy)]
struct Bounds {
    min_x: f64,
    min_z: f64,
    max_x: f64,
    max_z: f64,
}

fn is_liquid(state: u16) -> bool {
    let name = rine_data::get().blocks.block_of(state).name.as_str();
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    n == "water" || n == "lava"
}

fn block_at(g: &Game, x: i32, y: i32, z: i32) -> u16 {
    if y < rine_world::MIN_Y || y >= rine_world::MIN_Y + rine_world::HEIGHT {
        return 0;
    }
    g.world.get_block(x, y, z)
}

fn is_air_at(g: &Game, x: i32, y: i32, z: i32) -> bool {
    rine_data::get().blocks.is_air(block_at(g, x, y, z))
}

/// `SpreadPlayersCommand.Position.getSpawnY`.
fn spawn_y(g: &Game, p: &Pos, max_height: i32) -> i32 {
    let (x, z) = (p.x.floor() as i32, p.z.floor() as i32);
    let mut y = max_height + 1;
    let mut air2 = is_air_at(g, x, y, z);
    y -= 1;
    let mut air1 = is_air_at(g, x, y, z);
    while y > rine_world::MIN_Y {
        y -= 1;
        let cur_air = is_air_at(g, x, y, z);
        if !cur_air && air1 && air2 {
            return y + 1;
        }
        air2 = air1;
        air1 = cur_air;
    }
    max_height + 1
}

/// `SpreadPlayersCommand.Position.isSafe`.
fn is_safe(g: &Game, p: &Pos, max_height: i32) -> bool {
    let y = spawn_y(g, p, max_height) - 1;
    let (x, z) = (p.x.floor() as i32, p.z.floor() as i32);
    let state = block_at(g, x, y, z);
    if y >= max_height || is_liquid(state) {
        return false;
    }
    let name = rine_data::get().blocks.block_of(state).name.clone();
    rine_data::get().registries.tag_contains("minecraft:block", "entities_can_teleport_to", &name)
}

/// `SpreadPlayersCommand.spreadPositions`.
fn spread_positions(g: &Game, pos: &mut [Pos], spread: f64, b: &Bounds, max_height: i32) -> Result<f64, f64> {
    let n = pos.len();
    let mut has_collisions = true;
    let mut min_distance = f32::MAX as f64;
    let mut iteration = 0;
    while iteration < 10000 && has_collisions {
        has_collisions = false;
        min_distance = f32::MAX as f64;
        for i in 0..n {
            let mut neighbours = 0;
            let mut avg = Pos { x: 0.0, z: 0.0 };
            for j in 0..n {
                if i != j {
                    let dist = pos[i].dist(&pos[j]);
                    min_distance = min_distance.min(dist);
                    if dist < spread {
                        neighbours += 1;
                        avg.x += pos[j].x - pos[i].x;
                        avg.z += pos[j].z - pos[i].z;
                    }
                }
            }
            if neighbours > 0 {
                avg.x /= neighbours as f64;
                avg.z /= neighbours as f64;
                let len = avg.length();
                if len > 0.0 {
                    avg.x /= len;
                    avg.z /= len;
                    pos[i].x -= avg.x;
                    pos[i].z -= avg.z;
                } else {
                    pos[i].randomize(b);
                }
                has_collisions = true;
            }
            if pos[i].clamp(b) {
                has_collisions = true;
            }
        }
        if !has_collisions {
            for i in 0..n {
                if !is_safe(g, &pos[i], max_height) {
                    pos[i].randomize(b);
                    has_collisions = true;
                }
            }
        }
        iteration += 1;
    }
    if min_distance == f32::MAX as f64 {
        min_distance = 0.0;
    }
    if iteration >= 10000 {
        return Err(min_distance);
    }
    Ok(min_distance)
}

/// `SpreadPlayersCommand.spreadPlayers` plus its argument wiring.
fn spread_cmd(x: &mut Ctx, max_height: i32) -> CmdResult<i32> {
    let center = x.vec2("center");
    let spread = x.float("spreadDistance") as f64;
    let max_range = x.float("maxRange") as f64;
    let respect = x.bool("respectTeams");
    let eids = x.entities("targets")?;
    let min_y = rine_world::MIN_Y;
    if max_height < min_y {
        return Err(CmdError::tr("commands.spreadplayers.failed.invalid.height", vec![lit(max_height), lit(min_y)]));
    }
    let (cx, cz) = (center[0] as f64, center[1] as f64);
    let b = Bounds { min_x: cx - max_range, min_z: cz - max_range, max_x: cx + max_range, max_z: cz + max_range };

    // getNumberOfTeams: distinct teams (None for non-players)
    let count = if respect {
        let mut teams: Vec<Option<String>> = Vec::new();
        for &e in &eids {
            let t = team_of_eid(x.g, e);
            if !teams.contains(&t) {
                teams.push(t);
            }
        }
        teams.len()
    } else {
        eids.len()
    };
    let mut pos: Vec<Pos> = (0..count)
        .map(|_| {
            let mut p = Pos { x: 0.0, z: 0.0 };
            p.randomize(&b);
            p
        })
        .collect();
    let min_distance = match spread_positions(x.g, &mut pos, spread, &b, max_height) {
        Ok(d) => d,
        Err(d) => {
            let key = if respect { "commands.spreadplayers.failed.teams" } else { "commands.spreadplayers.failed.entities" };
            return Err(CmdError::tr(
                key,
                vec![lit(count), lit(java_f(center[0])), lit(java_f(center[1])), lit(format!("{d:.2}"))],
            ));
        }
    };
    let distance = set_player_positions(x.g, &eids, &pos, max_height, respect);
    let key = if respect { "commands.spreadplayers.success.teams" } else { "commands.spreadplayers.success.entities" };
    x.success(
        Text::tr(key, vec![lit(count), lit(java_f(center[0])), lit(java_f(center[1])), lit(format!("{:.2}", distance))]),
        true,
    );
    let _ = min_distance;
    Ok(count as i32)
}

fn java_f(v: f32) -> String {
    rine_command::error::java_float(v)
}

fn team_of_eid(g: &Game, eid: i32) -> Option<String> {
    if !g.by_eid.contains_key(&eid) {
        return None;
    }
    super::scoreboard::team_of(g, &select::scoreboard_name(g, eid))
}

/// `SpreadPlayersCommand.setPlayerPositions`.
fn set_player_positions(g: &mut Game, eids: &[i32], pos: &[Pos], max_height: i32, respect: bool) -> f64 {
    let mut avg = 0.0;
    let mut next = 0usize;
    let mut team_index: HashMap<Option<String>, usize> = HashMap::new();
    for &e in eids {
        let pi = if respect {
            let team = team_of_eid(g, e);
            match team_index.get(&team) {
                Some(&i) => i,
                None => {
                    let i = next;
                    next += 1;
                    team_index.insert(team, i);
                    i
                }
            }
        } else {
            let i = next;
            next += 1;
            i
        };
        let p = pos[pi];
        let y = spawn_y(g, &p, max_height);
        let rot = select::entity_rot(g, e);
        teleport_to(g, e, [p.x.floor() + 0.5, y as f64, p.z.floor() + 0.5], 0, rot[1], rot[0]);
        let mut closest = f64::MAX;
        for (k, t) in pos.iter().enumerate() {
            if k != pi {
                closest = closest.min(p.dist(t));
            }
        }
        avg += closest;
    }
    if eids.len() < 2 {
        0.0
    } else {
        avg / eids.len() as f64
    }
}
