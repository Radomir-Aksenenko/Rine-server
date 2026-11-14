use crate::net::{ClientInfo, NewPlayer, Out};
use ahash::AHashSet;
use bytes::{Bytes, BytesMut};
use rine_protocol::BlockPos;
use rine_world::ChunkPos;
use std::collections::VecDeque;
use std::time::Instant;
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

/// Item stack without components (components come later).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stack {
    pub item: u32,
    pub count: i32,
}

impl Stack {
    pub const EMPTY: Stack = Stack { item: 0, count: 0 };
    pub fn new(item: u32, count: i32) -> Self {
        if item == 0 || count <= 0 {
            Self::EMPTY
        } else {
            Self { item, count }
        }
    }
    pub fn is_empty(&self) -> bool {
        self.item == 0 || self.count <= 0
    }
    pub fn tuple(&self) -> (u32, i32) {
        if self.is_empty() {
            (0, 0)
        } else {
            (self.item, self.count)
        }
    }
}

/// Player inventory in window-0 slot numbering:
/// 0 craft result, 1-4 craft grid, 5-8 armor, 9-35 main, 36-44 hotbar, 45 offhand.
pub struct Inventory {
    pub slots: [Stack; 46],
    pub cursor: Stack,
    pub state_id: i32,
}

impl Default for Inventory {
    fn default() -> Self {
        Self { slots: [Stack::EMPTY; 46], cursor: Stack::EMPTY, state_id: 0 }
    }
}

impl Inventory {
    pub fn hotbar(&self, held: u8) -> Stack {
        self.slots[36 + held as usize]
    }
    /// Add items, filling existing stacks first (hotbar, then main). Returns leftover.
    pub fn add(&mut self, item: u32, mut count: i32) -> i32 {
        let max = max_stack(item);
        let order: Vec<usize> = (36..45).chain(9..36).collect();
        for &i in &order {
            let s = &mut self.slots[i];
            if s.item == item && s.count < max {
                let n = (max - s.count).min(count);
                s.count += n;
                count -= n;
                if count == 0 {
                    return 0;
                }
            }
        }
        for &i in &order {
            if self.slots[i].is_empty() {
                let n = max.min(count);
                self.slots[i] = Stack::new(item, n);
                count -= n;
                if count == 0 {
                    return 0;
                }
            }
        }
        count
    }
}

pub fn max_stack(item: u32) -> i32 {
    crate::game::items::info(item).map(|i| i.max_stack).unwrap_or(64)
}

pub struct Player {
    pub slot: usize,
    pub eid: i32,
    pub name: String,
    pub uuid: Uuid,
    pub props: Vec<(String, String, Option<String>)>,
    pub info: ClientInfo,
    pub inbound: crossbeam_channel::Receiver<BytesMut>,
    pub outbound: UnboundedSender<Out>,
    pub alive: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Frames queued during this tick, flushed once at tick end.
    pub out: Vec<Bytes>,

    pub pos: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
    /// Last state broadcast to viewers (for relative moves).
    pub sent_pos: [f64; 3],
    pub sent_rot: (f32, f32),
    pub sync_timer: u32,
    pub gamemode: u8,
    pub flying: bool,
    pub sneaking: bool,
    pub sprinting: bool,

    pub tp_id: i32,
    pub tp_pending: bool,
    pub ka_id: i64,
    pub ka_sent: Option<Instant>,
    pub ping_ms: i32,
    pub last_ka_reply: Instant,

    pub view: i32,
    pub center: ChunkPos,
    pub loaded: AHashSet<ChunkPos>,
    pub queue: VecDeque<ChunkPos>,
    pub unacked: u32,
    pub max_unacked: u32,
    pub desired: f32,
    pub quota: f32,
    pub stream_dirty: bool,

    /// Entity ids (players and others) this client currently sees.
    pub tracked: AHashSet<i32>,

    pub inv: Inventory,
    pub held: u8,
    pub health: f32,
    pub food: i32,
    pub saturation: f32,
    pub fall_start: f64,
    pub dead: bool,
    pub digging: Option<(BlockPos, u64)>,
    pub kick: Option<String>,
    pub joined_tick: u64,
}

impl Player {
    pub fn new(np: NewPlayer, slot: usize, eid: i32, spawn: [f64; 3], gamemode: u8, view: i32) -> Self {
        let view = view.min(np.info.view_distance.max(2) as i32);
        Self {
            slot,
            eid,
            name: np.name,
            uuid: np.uuid,
            props: np.properties,
            info: np.info,
            inbound: np.inbound,
            outbound: np.outbound,
            alive: np.alive,
            out: Vec::with_capacity(64),
            pos: spawn,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: false,
            sent_pos: spawn,
            sent_rot: (0.0, 0.0),
            sync_timer: 0,
            gamemode,
            flying: false,
            sneaking: false,
            sprinting: false,
            tp_id: 0,
            tp_pending: false,
            ka_id: 0,
            ka_sent: None,
            ping_ms: 0,
            last_ka_reply: Instant::now(),
            view,
            center: ChunkPos::new((spawn[0].floor() as i32) >> 4, (spawn[2].floor() as i32) >> 4),
            loaded: AHashSet::new(),
            queue: VecDeque::new(),
            unacked: 0,
            max_unacked: 1,
            desired: 9.0,
            quota: 0.0,
            stream_dirty: true,
            tracked: AHashSet::new(),
            inv: Inventory::default(),
            held: 0,
            health: 20.0,
            food: 20,
            saturation: 5.0,
            fall_start: spawn[1],
            dead: false,
            digging: None,
            kick: None,
            joined_tick: 0,
        }
    }

    #[inline]
    pub fn send(&mut self, p: BytesMut) {
        self.out.push(crate::out::frame(p));
    }

    #[inline]
    pub fn send_frame(&mut self, f: Bytes) {
        self.out.push(f);
    }

    pub fn flush(&mut self) {
        if !self.out.is_empty() {
            let v = std::mem::replace(&mut self.out, Vec::with_capacity(64));
            let _ = self.outbound.send(Out::Batch(v));
        }
    }

    pub fn chunk(&self) -> ChunkPos {
        ChunkPos::new((self.pos[0].floor() as i32) >> 4, (self.pos[2].floor() as i32) >> 4)
    }

    pub fn teleport(&mut self, x: f64, y: f64, z: f64, yaw: f32, pitch: f32) {
        self.tp_id += 1;
        self.tp_pending = true;
        self.pos = [x, y, z];
        self.yaw = yaw;
        self.pitch = pitch;
        self.fall_start = y;
        let p = crate::out::player_position(self.tp_id, x, y, z, yaw, pitch);
        self.send(p);
        self.stream_dirty = true;
    }

    pub fn msg(&mut self, t: &rine_protocol::text::Text) {
        let p = crate::out::system_chat(t, false);
        self.send(p);
    }

    pub fn abilities_packet(&self) -> BytesMut {
        let (mut flags, fly) = match self.gamemode {
            1 => (0x01 | 0x04 | 0x08, 0.05f32),
            3 => (0x01 | 0x02 | 0x04, 0.05),
            _ => (0, 0.05),
        };
        if self.flying && self.gamemode == 1 {
            flags |= 0x02;
        }
        crate::out::abilities(flags, fly, 0.1)
    }

    pub fn inventory_packet(&mut self) -> BytesMut {
        self.inv.state_id = self.inv.state_id.wrapping_add(1) & 0x7FFF;
        let items: Vec<(u32, i32)> = self.inv.slots.iter().map(|s| s.tuple()).collect();
        crate::out::container_content(0, self.inv.state_id, &items, self.inv.cursor.tuple())
    }

    pub fn eye(&self) -> [f64; 3] {
        let h = if self.sneaking { 1.27 } else { 1.62 };
        [self.pos[0], self.pos[1] + h, self.pos[2]]
    }
}
