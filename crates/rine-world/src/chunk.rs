//! Chunk storage. A section stores block states as a flat `[u16; 4096]`
//! (index `(y << 8) | (z << 4) | x`, same as vanilla) so get/set are a single
//! array access. All-air (or any single-state) sections keep no array at all.
//! Palettes only exist transiently when encoding for the network or disk.

pub const MIN_Y: i32 = -64;
pub const HEIGHT: i32 = 384;
pub const MAX_Y: i32 = MIN_Y + HEIGHT - 1; // 319
pub const SECTIONS: usize = (HEIGHT / 16) as usize; // 24

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ChunkPos {
    pub x: i32,
    pub z: i32,
}

impl ChunkPos {
    pub const fn new(x: i32, z: i32) -> Self {
        Self { x, z }
    }
    pub fn of_block(bx: i32, bz: i32) -> Self {
        Self { x: bx >> 4, z: bz >> 4 }
    }
    /// vanilla ChunkPos.pack(): x low 32 bits, z high 32 bits
    pub fn pack(self) -> i64 {
        (self.x as u32 as i64) | ((self.z as u32 as i64) << 32)
    }
    pub fn dist2(self, o: ChunkPos) -> i32 {
        let dx = self.x - o.x;
        let dz = self.z - o.z;
        dx * dx + dz * dz
    }
}

#[derive(Clone)]
pub struct Section {
    /// None => every block equals `single`.
    pub blocks: Option<Box<[u16; 4096]>>,
    pub single: u16,
    /// Count of non-air blocks (vanilla nonEmptyBlockCount).
    pub non_air: u16,
    /// Count of blocks with a fluid (vanilla fluidCount).
    pub fluids: u16,
    /// Biome ids (index into synced worldgen/biome registry), 4x4x4 quarts,
    /// index `(qy << 4) | (qz << 2) | qx`.
    pub biomes: [u8; 64],
    /// Nibble arrays, vanilla layout (index = block index, low nibble first).
    pub sky: Option<Box<[u8; 2048]>>,
    pub block_light: Option<Box<[u8; 2048]>>,
}

impl Default for Section {
    fn default() -> Self {
        Self { blocks: None, single: 0, non_air: 0, fluids: 0, biomes: [0; 64], sky: None, block_light: None }
    }
}

#[inline]
pub fn idx(x: usize, y: usize, z: usize) -> usize {
    (y << 8) | (z << 4) | x
}

impl Section {
    #[inline]
    pub fn get(&self, i: usize) -> u16 {
        match &self.blocks {
            Some(b) => b[i],
            None => self.single,
        }
    }

    /// Set block at index; returns the old state. Keeps counts up to date.
    pub fn set(&mut self, i: usize, state: u16) -> u16 {
        let old = self.get(i);
        if old == state {
            return old;
        }
        if self.blocks.is_none() {
            self.blocks = Some(Box::new([self.single; 4096]));
        }
        self.blocks.as_mut().unwrap()[i] = state;
        let b = &rine_data::get().blocks;
        if !b.is_air(old) {
            self.non_air -= 1;
        }
        if !b.is_air(state) {
            self.non_air += 1;
        }
        let fl = rine_data::blocks::flag::WATER | rine_data::blocks::flag::LAVA;
        if b.has(old, fl) {
            self.fluids -= 1;
        }
        if b.has(state, fl) {
            self.fluids += 1;
        }
        old
    }

    /// Fill the whole section with one state (cheap: drops the array).
    pub fn fill(&mut self, state: u16) {
        self.blocks = None;
        self.single = state;
        let b = &rine_data::get().blocks;
        self.non_air = if b.is_air(state) { 0 } else { 4096 };
        let fl = rine_data::blocks::flag::WATER | rine_data::blocks::flag::LAVA;
        self.fluids = if b.has(state, fl) { 4096 } else { 0 };
    }

    pub fn is_empty(&self) -> bool {
        self.non_air == 0
    }

    /// Recompute counts and collapse to single-value if possible.
    pub fn recount(&mut self) {
        let b = &rine_data::get().blocks;
        let fl = rine_data::blocks::flag::WATER | rine_data::blocks::flag::LAVA;
        if let Some(arr) = &self.blocks {
            let first = arr[0];
            let mut uniform = true;
            let mut na = 0u16;
            let mut fc = 0u16;
            for &s in arr.iter() {
                uniform &= s == first;
                if !b.is_air(s) {
                    na += 1;
                }
                if b.has(s, fl) {
                    fc += 1;
                }
            }
            self.non_air = na;
            self.fluids = fc;
            if uniform {
                self.blocks = None;
                self.single = first;
            }
        } else {
            self.fill(self.single);
        }
    }
}

pub struct Chunk {
    pub pos: ChunkPos,
    pub sections: Vec<Section>,
    /// Highest Y+1 of a motion-blocking block per column (x + z*16), as
    /// vanilla heightmap values (0 = nothing, value = y - MIN_Y + 1).
    pub motion_blocking: [u16; 256],
    pub world_surface: [u16; 256],
    pub motion_blocking_no_leaves: [u16; 256],
    /// Bumped on every modification; encoders cache by version.
    pub version: u64,
    /// Needs saving.
    pub dirty: bool,
    /// Block entity data (chests, signs...), keyed by packed local index.
    pub block_entities: Vec<BlockEntity>,
}

#[derive(Clone, Debug)]
pub struct BlockEntity {
    pub x: u8,
    pub y: i16,
    pub z: u8,
    pub type_id: u32,
    pub nbt: Option<rine_protocol::nbt::Tag>,
}

impl Chunk {
    pub fn new(pos: ChunkPos) -> Self {
        Self {
            pos,
            sections: vec![Section::default(); SECTIONS],
            motion_blocking: [0; 256],
            world_surface: [0; 256],
            motion_blocking_no_leaves: [0; 256],
            version: 0,
            dirty: true,
            block_entities: Vec::new(),
        }
    }

    /// Local x/z in 0..16, world y in MIN_Y..=MAX_Y.
    #[inline]
    pub fn get(&self, x: i32, y: i32, z: i32) -> u16 {
        if !(MIN_Y..=MAX_Y).contains(&y) {
            return 0;
        }
        let sy = ((y - MIN_Y) >> 4) as usize;
        self.sections[sy].get(idx(x as usize & 15, (y & 15) as usize, z as usize & 15))
    }

    /// Set a block. Returns old state. Updates heightmaps incrementally.
    pub fn set(&mut self, x: i32, y: i32, z: i32, state: u16) -> u16 {
        if !(MIN_Y..=MAX_Y).contains(&y) {
            return 0;
        }
        let sy = ((y - MIN_Y) >> 4) as usize;
        let (lx, lz) = (x as usize & 15, z as usize & 15);
        let old = self.sections[sy].set(idx(lx, (y & 15) as usize, lz), state);
        if old != state {
            self.version += 1;
            self.dirty = true;
            self.update_height(lx, lz, y, state);
        }
        old
    }

    /// Set without heightmap/version bookkeeping (bulk generation). Call
    /// `finish_bulk()` afterwards.
    #[inline]
    pub fn set_raw(&mut self, x: i32, y: i32, z: i32, state: u16) {
        if !(MIN_Y..=MAX_Y).contains(&y) {
            return;
        }
        let sy = ((y - MIN_Y) >> 4) as usize;
        let s = &mut self.sections[sy];
        let i = idx(x as usize & 15, (y & 15) as usize, z as usize & 15);
        if s.blocks.is_none() {
            if s.single == state {
                return;
            }
            s.blocks = Some(Box::new([s.single; 4096]));
        }
        s.blocks.as_mut().unwrap()[i] = state;
    }

    pub fn finish_bulk(&mut self) {
        for s in &mut self.sections {
            s.recount();
        }
        self.recalc_heightmaps();
        self.version += 1;
        self.dirty = true;
    }

    pub fn biome_at(&self, x: i32, y: i32, z: i32) -> u8 {
        let y = y.clamp(MIN_Y, MAX_Y);
        let sy = ((y - MIN_Y) >> 4) as usize;
        let (qx, qy, qz) = ((x as usize & 15) >> 2, ((y & 15) as usize) >> 2, (z as usize & 15) >> 2);
        self.sections[sy].biomes[(qy << 4) | (qz << 2) | qx]
    }

    pub fn fill_biome(&mut self, biome: u8) {
        for s in &mut self.sections {
            s.biomes = [biome; 64];
        }
    }

    fn blocks_motion(state: u16) -> bool {
        let b = &rine_data::get().blocks;
        use rine_data::blocks::flag::*;
        b.has(state, SOLID | WATER | LAVA)
    }

    fn update_height(&mut self, lx: usize, lz: usize, y: i32, state: u16) {
        let col = lx + lz * 16;
        let hv = (y - MIN_Y + 1) as u16;
        let b = &rine_data::get().blocks;
        let air = b.is_air(state);
        if !air && hv > self.world_surface[col] {
            self.world_surface[col] = hv;
        }
        if Self::blocks_motion(state) && hv > self.motion_blocking[col] {
            self.motion_blocking[col] = hv;
        }
        if Self::blocks_motion(state) && !is_leaves(state) && hv > self.motion_blocking_no_leaves[col] {
            self.motion_blocking_no_leaves[col] = hv;
        }
        // block removed at the top: rescan the column downward
        if hv == self.world_surface[col] && air
            || hv == self.motion_blocking[col] && !Self::blocks_motion(state)
            || hv == self.motion_blocking_no_leaves[col] && (!Self::blocks_motion(state) || is_leaves(state))
        {
            self.recalc_column(lx, lz);
        }
    }

    fn recalc_column(&mut self, lx: usize, lz: usize) {
        let col = lx + lz * 16;
        let b = &rine_data::get().blocks;
        let (mut ws, mut mb, mut mbnl) = (0u16, 0u16, 0u16);
        let mut y = MAX_Y;
        while y >= MIN_Y && (ws == 0 || mb == 0 || mbnl == 0) {
            let sy = ((y - MIN_Y) >> 4) as usize;
            let s = &self.sections[sy];
            if s.blocks.is_none() && b.is_air(s.single) {
                y = MIN_Y + (sy as i32) * 16 - 1;
                continue;
            }
            let st = s.get(idx(lx, (y & 15) as usize, lz));
            let hv = (y - MIN_Y + 1) as u16;
            if ws == 0 && !b.is_air(st) {
                ws = hv;
            }
            if mb == 0 && Self::blocks_motion(st) {
                mb = hv;
            }
            if mbnl == 0 && Self::blocks_motion(st) && !is_leaves(st) {
                mbnl = hv;
            }
            y -= 1;
        }
        self.world_surface[col] = ws;
        self.motion_blocking[col] = mb;
        self.motion_blocking_no_leaves[col] = mbnl;
    }

    pub fn recalc_heightmaps(&mut self) {
        for lz in 0..16 {
            for lx in 0..16 {
                self.recalc_column(lx, lz);
            }
        }
    }

    /// Highest non-air block Y at local column, or MIN_Y-1.
    pub fn surface_y(&self, lx: i32, lz: i32) -> i32 {
        self.world_surface[(lx as usize & 15) + (lz as usize & 15) * 16] as i32 + MIN_Y - 1
    }
}

pub fn is_leaves(state: u16) -> bool {
    let b = &rine_data::get().blocks;
    b.block_of(state).name.ends_with("_leaves")
}
