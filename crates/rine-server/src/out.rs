//! Clientbound play packet builders. Each returns the raw `id + body`
//! payload; callers frame it once (`frame()`) and fan the Frame out.

use bytes::{Bytes, BytesMut};
use rine_protocol::frame::{encode_frame, packet};
use rine_protocol::ids::play::*;
use rine_protocol::nbt::Tag;
use rine_protocol::text::Text;
use rine_protocol::{BlockPos, Writer};
use crate::game::player::Stack;
use std::sync::atomic::{AtomicI32, Ordering};
use uuid::Uuid;

/// Server-wide compression threshold (set once at startup).
pub static THRESHOLD: AtomicI32 = AtomicI32::new(256);

#[inline]
pub fn frame(p: BytesMut) -> Bytes {
    encode_frame(&p, THRESHOLD.load(Ordering::Relaxed)).0
}

pub fn write_text(b: &mut BytesMut, t: &Text) {
    t.to_nbt().write_network(b);
}

/// LpVec3 velocity encoding (docs/spec/03 section 1.6).
pub fn write_lpvec3(b: &mut BytesMut, x: f64, y: f64, z: f64) {
    const ABS_MAX: f64 = 1.7179869183E10;
    const ABS_MIN: f64 = 3.051944088384301E-5;
    let s = |v: f64| if v.is_nan() { 0.0 } else { v.clamp(-ABS_MAX, ABS_MAX) };
    let (x, y, z) = (s(x), s(y), s(z));
    let cb = x.abs().max(y.abs()).max(z.abs());
    if cb < ABS_MIN {
        b.w_u8(0);
        return;
    }
    let scale = cb.ceil() as i64;
    let partial = (scale & 3) != scale;
    let markers = if partial { (scale & 3) | 4 } else { scale };
    let q = |c: f64| -> i64 { ((c / scale as f64 * 0.5 + 0.5) * 32766.0).round() as i64 & 0x7FFF };
    let buf: i64 = markers | (q(x) << 3) | (q(y) << 18) | (q(z) << 33);
    b.w_u8(buf as u8);
    b.w_u8((buf >> 8) as u8);
    b.w_i32((buf >> 16) as i32);
    if partial {
        b.w_varint((scale >> 2) as i32);
    }
}

#[inline]
pub fn angle(deg: f32) -> u8 {
    ((deg * 256.0 / 360.0).floor() as i32) as u8
}

pub fn keep_alive(id: i64) -> BytesMut {
    packet(C_KEEP_ALIVE, |b| b.w_i64(id))
}

pub fn system_chat(t: &Text, overlay: bool) -> BytesMut {
    packet(C_SYSTEM_CHAT, |b| {
        write_text(b, t);
        b.w_bool(overlay);
    })
}

pub fn disconnect(t: &Text) -> BytesMut {
    packet(C_DISCONNECT, |b| write_text(b, t))
}

pub fn game_event(ev: u8, param: f32) -> BytesMut {
    packet(C_GAME_EVENT, |b| {
        b.w_u8(ev);
        b.w_f32(param);
    })
}

pub fn player_position(tp_id: i32, x: f64, y: f64, z: f64, yaw: f32, pitch: f32) -> BytesMut {
    packet(C_PLAYER_POSITION, |b| {
        b.w_varint(tp_id);
        b.w_f64(x);
        b.w_f64(y);
        b.w_f64(z);
        b.w_f64(0.0);
        b.w_f64(0.0);
        b.w_f64(0.0);
        b.w_f32(yaw);
        b.w_f32(pitch);
        b.w_i32(0);
    })
}

pub fn chunk_center(x: i32, z: i32) -> BytesMut {
    packet(C_SET_CHUNK_CACHE_CENTER, |b| {
        b.w_varint(x);
        b.w_varint(z);
    })
}

pub fn forget_chunk(x: i32, z: i32) -> BytesMut {
    packet(C_FORGET_LEVEL_CHUNK, |b| b.w_i64((x as u32 as i64) | ((z as u32 as i64) << 32)))
}

pub fn block_update(pos: BlockPos, state: u16) -> BytesMut {
    packet(C_BLOCK_UPDATE, |b| {
        b.w_pos(pos);
        b.w_varint(state as i32);
    })
}

pub fn section_pos(x: i32, y: i32, z: i32) -> i64 {
    ((x as i64 & 0x3FFFFF) << 42) | (y as i64 & 0xFFFFF) | ((z as i64 & 0x3FFFFF) << 20)
}

/// changes: (local index as (x<<8)|(z<<4)|y, state)
pub fn section_blocks(sx: i32, sy: i32, sz: i32, changes: &[(u16, u16)]) -> BytesMut {
    packet(C_SECTION_BLOCKS_UPDATE, |b| {
        b.w_i64(section_pos(sx, sy, sz));
        b.w_varint(changes.len() as i32);
        for &(rel, st) in changes {
            b.w_varlong(((st as i64) << 12) | rel as i64);
        }
    })
}

pub fn block_ack(seq: i32) -> BytesMut {
    packet(C_BLOCK_CHANGED_ACK, |b| b.w_varint(seq))
}

pub fn block_destruction(eid: i32, pos: BlockPos, progress: u8) -> BytesMut {
    packet(C_BLOCK_DESTRUCTION, |b| {
        b.w_varint(eid);
        b.w_pos(pos);
        b.w_u8(progress);
    })
}

pub fn level_event(ty: i32, pos: BlockPos, data: i32) -> BytesMut {
    packet(C_LEVEL_EVENT, |b| {
        b.w_i32(ty);
        b.w_pos(pos);
        b.w_i32(data);
        b.w_bool(false);
    })
}

#[allow(clippy::too_many_arguments)]
pub fn add_entity(eid: i32, uuid: Uuid, ty: u32, x: f64, y: f64, z: f64, vel: (f64, f64, f64), pitch: f32, yaw: f32, head: f32, data: i32) -> BytesMut {
    packet(C_ADD_ENTITY, |b| {
        b.w_varint(eid);
        b.w_uuid(uuid);
        b.w_varint(ty as i32);
        b.w_f64(x);
        b.w_f64(y);
        b.w_f64(z);
        write_lpvec3(b, vel.0, vel.1, vel.2);
        b.w_u8(angle(pitch));
        b.w_u8(angle(yaw));
        b.w_u8(angle(head));
        b.w_varint(data);
    })
}

pub fn remove_entities(ids: &[i32]) -> BytesMut {
    packet(C_REMOVE_ENTITIES, |b| {
        b.w_varint(ids.len() as i32);
        for &i in ids {
            b.w_varint(i);
        }
    })
}

/// Relative move; returns None if the delta does not fit in i16.
pub fn move_entity(eid: i32, old: (f64, f64, f64), new: (f64, f64, f64), rot: Option<(f32, f32)>, on_ground: bool) -> Option<BytesMut> {
    let enc = |v: f64| (v * 4096.0).round() as i64;
    let dx = enc(new.0) - enc(old.0);
    let dy = enc(new.1) - enc(old.1);
    let dz = enc(new.2) - enc(old.2);
    let fits = |d: i64| (i16::MIN as i64..=i16::MAX as i64).contains(&d);
    if !(fits(dx) && fits(dy) && fits(dz)) {
        return None;
    }
    let id = if rot.is_some() { C_MOVE_ENTITY_POS_ROT } else { C_MOVE_ENTITY_POS };
    Some(packet(id, |b| {
        b.w_varint(eid);
        b.w_varint(on_ground as i32);
        b.w_i16(dx as i16);
        b.w_i16(dy as i16);
        b.w_i16(dz as i16);
        if let Some((yaw, pitch)) = rot {
            b.w_u8(angle(yaw));
            b.w_u8(angle(pitch));
        }
    }))
}

pub fn rotate_entity(eid: i32, yaw: f32, pitch: f32, on_ground: bool) -> BytesMut {
    packet(C_MOVE_ENTITY_ROT, |b| {
        b.w_varint(eid);
        b.w_bool(on_ground);
        b.w_u8(angle(yaw));
        b.w_u8(angle(pitch));
    })
}

pub fn position_sync(eid: i32, x: f64, y: f64, z: f64, yaw: f32, pitch: f32, on_ground: bool) -> BytesMut {
    packet(C_ENTITY_POSITION_SYNC, |b| {
        b.w_varint(eid);
        b.w_varint(0); // PositionPath LINEAR
        b.w_f64(x);
        b.w_f64(y);
        b.w_f64(z);
        b.w_f32(yaw);
        b.w_f32(pitch);
        b.w_bool(on_ground);
    })
}

pub fn head_rot(eid: i32, yaw: f32) -> BytesMut {
    packet(C_ROTATE_HEAD, |b| {
        b.w_varint(eid);
        b.w_u8(angle(yaw));
    })
}

pub fn entity_motion(eid: i32, vx: f64, vy: f64, vz: f64) -> BytesMut {
    packet(C_SET_ENTITY_MOTION, |b| {
        b.w_varint(eid);
        write_lpvec3(b, vx, vy, vz);
    })
}

pub fn swing(eid: i32, hand: i32) -> BytesMut {
    packet(C_SWING_ANIMATION, |b| {
        b.w_varint(eid);
        b.w_varint(hand);
        b.w_varint(1); // WHACK
        b.w_varint(6);
    })
}

/// Clientbound animate: 1 = critical hit, 2 = magic critical hit (`ClientboundAnimatePacket`).
pub fn animate(eid: i32, action: u8) -> BytesMut {
    packet(C_ANIMATE, |b| {
        b.w_varint(eid);
        b.w_u8(action);
    })
}

pub fn entity_event(eid: i32, ev: i8) -> BytesMut {
    packet(C_ENTITY_EVENT, |b| {
        b.w_i32(eid);
        b.w_i8(ev);
    })
}

/// Entity metadata builder.
pub struct Meta(BytesMut);
impl Meta {
    pub fn new(eid: i32) -> Self {
        let mut b = BytesMut::with_capacity(32);
        b.w_varint(C_SET_ENTITY_DATA);
        b.w_varint(eid);
        Meta(b)
    }
    pub fn byte(mut self, idx: u8, v: u8) -> Self {
        self.0.w_u8(idx);
        self.0.w_varint(0);
        self.0.w_u8(v);
        self
    }
    pub fn varint(mut self, idx: u8, v: i32) -> Self {
        self.0.w_u8(idx);
        self.0.w_varint(1);
        self.0.w_varint(v);
        self
    }
    pub fn float(mut self, idx: u8, v: f32) -> Self {
        self.0.w_u8(idx);
        self.0.w_varint(3);
        self.0.w_f32(v);
        self
    }
    pub fn boolean(mut self, idx: u8, v: bool) -> Self {
        self.0.w_u8(idx);
        self.0.w_varint(8);
        self.0.w_bool(v);
        self
    }
    pub fn pose(mut self, idx: u8, v: i32) -> Self {
        self.0.w_u8(idx);
        self.0.w_varint(20);
        self.0.w_varint(v);
        self
    }
    pub fn block_state(mut self, idx: u8, v: u16) -> Self {
        self.0.w_u8(idx);
        self.0.w_varint(14);
        self.0.w_varint(v as i32);
        self
    }
    pub fn item(mut self, idx: u8, stack: &Stack) -> Self {
        self.0.w_u8(idx);
        self.0.w_varint(7);
        write_stack(&mut self.0, stack);
        self
    }
    pub fn humanoid_arm(mut self, idx: u8, v: i32) -> Self {
        self.0.w_u8(idx);
        self.0.w_varint(42);
        self.0.w_varint(v);
        self
    }
    /// `OPTIONAL_BLOCK_POS` (serializer 11): presence flag, then the packed position.
    pub fn optional_block_pos(mut self, idx: u8, v: Option<rine_protocol::BlockPos>) -> Self {
        self.0.w_u8(idx);
        self.0.w_varint(11);
        match v {
            Some(p) => {
                self.0.w_bool(true);
                self.0.w_pos(p);
            }
            None => self.0.w_bool(false),
        }
        self
    }
    pub fn finish(mut self) -> BytesMut {
        self.0.w_u8(0xFF);
        self.0
    }
}

/// Registry id of `minecraft:damage` in `minecraft:data_component_type`.
pub fn damage_component_id() -> u32 {
    static ID: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *ID.get_or_init(|| rine_data::get().registries.id("minecraft:data_component_type", "minecraft:damage").expect("minecraft:damage component"))
}

/// ItemStack (OPTIONAL_STREAM_CODEC). Count, item id, then the component
/// patch: a damaged stack adds one `minecraft:damage` value (VAR_INT).
pub fn write_stack(b: &mut BytesMut, s: &Stack) {
    write_stack_with(b, s, damage_component_id());
}

fn write_stack_with(b: &mut BytesMut, s: &Stack, damage_id: u32) {
    if s.is_empty() {
        b.w_varint(0);
        return;
    }
    b.w_varint(s.count);
    b.w_varint(s.item as i32);
    if s.damage > 0 {
        b.w_varint(1); // added components
        b.w_varint(0); // removed components
        b.w_varint(damage_id as i32);
        b.w_varint(s.damage as i32);
    } else {
        b.w_varint(0);
        b.w_varint(0);
    }
}

pub fn set_health(health: f32, food: i32, sat: f32) -> BytesMut {
    packet(C_SET_HEALTH, |b| {
        b.w_f32(health);
        b.w_varint(food);
        b.w_f32(sat);
    })
}

pub fn set_time(game_time: i64, day_time: i64, clock_ids: &[u32], rate: f32) -> BytesMut {
    packet(C_SET_TIME, |b| {
        b.w_i64(game_time);
        b.w_varint(clock_ids.len() as i32);
        for &id in clock_ids {
            b.w_varint(id as i32);
            b.w_varlong(day_time);
            b.w_f32(0.0);
            b.w_f32(rate);
        }
    })
}

pub fn abilities(flags: u8, fly_speed: f32, walk_speed: f32) -> BytesMut {
    packet(C_PLAYER_ABILITIES, |b| {
        b.w_u8(flags);
        b.w_f32(fly_speed);
        b.w_f32(walk_speed);
    })
}

pub fn tab_list(header: &Text, footer: &Text) -> BytesMut {
    packet(C_TAB_LIST, |b| {
        write_text(b, header);
        write_text(b, footer);
    })
}

pub fn action_bar(t: &Text) -> BytesMut {
    packet(C_SET_ACTION_BAR_TEXT, |b| write_text(b, t))
}

pub fn info_remove(uuids: &[Uuid]) -> BytesMut {
    packet(C_PLAYER_INFO_REMOVE, |b| {
        b.w_varint(uuids.len() as i32);
        for u in uuids {
            b.w_uuid(*u);
        }
    })
}

pub struct InfoEntry<'a> {
    pub uuid: Uuid,
    pub name: &'a str,
    pub props: &'a [(String, String, Option<String>)],
    pub gamemode: i32,
    pub ping: i32,
}

/// ADD_PLAYER | UPDATE_GAME_MODE | UPDATE_LISTED | UPDATE_LATENCY
pub fn info_add(entries: &[InfoEntry]) -> BytesMut {
    packet(C_PLAYER_INFO_UPDATE, |b| {
        b.w_u8(0b0001_1101);
        b.w_varint(entries.len() as i32);
        for e in entries {
            b.w_uuid(e.uuid);
            b.w_str(e.name);
            b.w_varint(e.props.len() as i32);
            for (n, v, s) in e.props {
                b.w_str(n);
                b.w_str(v);
                b.w_bool(s.is_some());
                if let Some(s) = s {
                    b.w_str(s);
                }
            }
            b.w_varint(e.gamemode);
            b.w_bool(true);
            b.w_varint(e.ping);
        }
    })
}

pub fn info_gamemode(uuid: Uuid, gm: i32) -> BytesMut {
    packet(C_PLAYER_INFO_UPDATE, |b| {
        b.w_u8(0b0000_0100);
        b.w_varint(1);
        b.w_uuid(uuid);
        b.w_varint(gm);
    })
}

pub fn info_latency(list: &[(Uuid, i32)]) -> BytesMut {
    packet(C_PLAYER_INFO_UPDATE, |b| {
        b.w_u8(0b0001_0000);
        b.w_varint(list.len() as i32);
        for (u, p) in list {
            b.w_uuid(*u);
            b.w_varint(*p);
        }
    })
}

pub fn set_held_slot(slot: i32) -> BytesMut {
    packet(C_SET_HELD_SLOT, |b| b.w_varint(slot))
}

pub fn container_content(window: i32, state_id: i32, items: &[Stack], cursor: &Stack) -> BytesMut {
    packet(C_CONTAINER_SET_CONTENT, |b| {
        b.w_varint(window);
        b.w_varint(state_id);
        b.w_varint(items.len() as i32);
        for s in items {
            write_stack(b, s);
        }
        write_stack(b, cursor);
    })
}

pub fn container_slot(window: i32, state_id: i32, slot: i16, item: &Stack) -> BytesMut {
    packet(C_CONTAINER_SET_SLOT, |b| {
        b.w_varint(window);
        b.w_varint(state_id);
        b.w_i16(slot);
        write_stack(b, item);
    })
}

pub fn take_item(item_eid: i32, collector: i32, amount: i32) -> BytesMut {
    packet(C_TAKE_ITEM_ENTITY, |b| {
        b.w_varint(item_eid);
        b.w_varint(collector);
        b.w_varint(amount);
    })
}

/// Sound by registry id (holder reference => id + 1).
pub fn sound(sound_id: u32, category: i32, x: f64, y: f64, z: f64, vol: f32, pitch: f32, seed: i64) -> BytesMut {
    packet(C_SOUND, |b| {
        b.w_varint(sound_id as i32 + 1);
        b.w_varint(category);
        b.w_i32((x * 8.0) as i32);
        b.w_i32((y * 8.0) as i32);
        b.w_i32((z * 8.0) as i32);
        b.w_f32(vol);
        b.w_f32(pitch);
        b.w_i64(seed);
    })
}

/// Explosion visual: center, radius, block count, optional knockback,
/// large particle, sound, no extra block particles.
pub fn explode(x: f64, y: f64, z: f64, radius: f32, blocks: i32, knock: Option<(f64, f64, f64)>, particle_id: u32, sound_id: u32) -> BytesMut {
    packet(C_EXPLODE, |b| {
        b.w_f64(x);
        b.w_f64(y);
        b.w_f64(z);
        b.w_f32(radius);
        b.w_i32(blocks);
        b.w_bool(knock.is_some());
        if let Some((kx, ky, kz)) = knock {
            b.w_f64(kx);
            b.w_f64(ky);
            b.w_f64(kz);
        }
        b.w_varint(particle_id as i32);
        b.w_varint(sound_id as i32 + 1);
        b.w_varint(0);
        b.w_bool(true);
    })
}

pub fn nbt_text(s: &str) -> Tag {
    Tag::String(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_encoding_matches_item_stack_patch_layout() {
        let mut b = BytesMut::new();
        write_stack_with(&mut b, &Stack::damaged(7, 1, 5), 3);
        assert_eq!(&b[..], &[1, 7, 1, 0, 3, 5]);

        let mut b = BytesMut::new();
        write_stack_with(&mut b, &Stack::new(7, 3), 3);
        assert_eq!(&b[..], &[3, 7, 0, 0]);

        let mut b = BytesMut::new();
        write_stack_with(&mut b, &Stack::EMPTY, 3);
        assert_eq!(&b[..], &[0]);
    }

    #[test]
    fn damage_component_is_registry_entry_three() {
        rine_data::init();
        assert_eq!(damage_component_id(), 3);
    }
}
