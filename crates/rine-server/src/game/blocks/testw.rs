//! In-memory level for block behaviour tests: every position is loaded,
//! unset positions are air, light and weather are fields.

use crate::game::redstone::engine::{Lvl, Rs};
use ahash::AHashMap;
use rine_protocol::BlockPos;

#[derive(Default)]
pub struct World {
    pub cells: AHashMap<BlockPos, u16>,
    pub time: u64,
    pub day_time: i64,
    /// Sky light everywhere (unless `sky` has an entry).
    pub light: u8,
    pub sky: AHashMap<BlockPos, u8>,
    pub block_light: AHashMap<BlockPos, u8>,
    pub darken: u8,
    pub raining: bool,
    pub thundering: bool,
    pub biome: u8,
    /// Items popped by `pop_resource`: (pos, item, count).
    pub popped: Vec<(BlockPos, u32, i32)>,
    /// Blocks destroyed with drops: (pos, state).
    pub dropped: Vec<(BlockPos, u16)>,
    pub sounds: Vec<(String, BlockPos)>,
    pub events: Vec<(i32, BlockPos, i32)>,
}

impl World {
    pub fn new() -> Self {
        rine_data::init();
        Self { light: 15, day_time: 6000, ..Default::default() }
    }

    pub fn put(&mut self, p: BlockPos, st: u16) {
        self.cells.insert(p, st);
    }

    /// One game tick of scheduled block ticks.
    pub fn tick(&mut self, rs: &mut Rs) {
        self.time += 1;
        crate::game::redstone::engine::run_block_ticks(rs, self);
    }
}

impl Lvl for World {
    fn get(&self, p: BlockPos) -> u16 {
        self.cells.get(&p).copied().unwrap_or(0)
    }
    fn loaded(&self, p: BlockPos) -> bool {
        (rine_world::MIN_Y..=rine_world::MAX_Y).contains(&p.y)
    }
    fn raw_set(&mut self, p: BlockPos, s: u16) -> u16 {
        if s == 0 {
            self.cells.remove(&p).unwrap_or(0)
        } else {
            self.cells.insert(p, s).unwrap_or(0)
        }
    }
    fn time(&self) -> u64 {
        self.time
    }
    fn day_time(&self) -> i64 {
        self.day_time
    }
    fn sky_light(&self, p: BlockPos) -> u8 {
        self.sky.get(&p).copied().unwrap_or(self.light)
    }
    fn level_event(&mut self, ev: i32, p: BlockPos, data: i32) {
        self.events.push((ev, p, data));
    }
    fn sound(&mut self, n: &str, p: BlockPos, _v: f32, _pi: f32, _c: i32, _s: bool) {
        self.sounds.push((n.to_string(), p));
    }
    fn block_event_packet(&mut self, _p: BlockPos, _b: u16, _a: u8, _bb: u8) {}
    fn drop_block(&mut self, p: BlockPos, st: u16) {
        self.dropped.push((p, st));
    }
    fn prime_tnt(&mut self, _p: BlockPos) {}
    fn count_entities(&self, _min: [f64; 3], _max: [f64; 3], _living: bool) -> usize {
        0
    }
    fn container_signal(&self, _p: BlockPos, _st: u16) -> Option<u8> {
        None
    }
    fn chest_viewers(&self, _p: BlockPos) -> u8 {
        0
    }
    fn rand(&mut self) -> f32 {
        0.5
    }
    fn block_light(&self, p: BlockPos) -> u8 {
        self.block_light.get(&p).copied().unwrap_or(0)
    }
    fn sky_darken(&self) -> u8 {
        self.darken
    }
    fn raining(&self) -> bool {
        self.raining
    }
    fn thundering(&self) -> bool {
        self.thundering
    }
    fn biome(&self, _p: BlockPos) -> u8 {
        self.biome
    }
    fn height_motion_blocking(&self, x: i32, z: i32) -> i32 {
        let b = &rine_data::get().blocks;
        let mut top = rine_world::MIN_Y;
        for (p, &s) in &self.cells {
            if p.x == x && p.z == z && p.y + 1 > top && (b.has(s, rine_data::blocks::flag::SOLID) || b.has(s, rine_data::blocks::flag::WATER | rine_data::blocks::flag::LAVA)) {
                top = p.y + 1;
            }
        }
        top
    }
    fn pop_resource(&mut self, p: BlockPos, item: u32, count: i32) {
        self.popped.push((p, item, count));
    }
}
