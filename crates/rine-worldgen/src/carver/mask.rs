//! `CarvingMask` (vanilla `net.minecraft.world.level.chunk.CarvingMask`): one bit per block of the carvable
//! column range, `index = y - minY + (z + (x << 4)) * height`, the same layout as the Java `BitSet`.

pub struct CarvingMask {
    min_y: i32,
    max_y: i32,
    height: i32,
    words: Vec<u64>,
}

impl CarvingMask {
    /// `new CarvingMask(minY, maxY)`: inclusive range of carvable y.
    pub fn new(min_y: i32, max_y: i32) -> Self {
        let height = max_y - min_y + 1;
        let bits = 256 * height as usize;
        Self {
            min_y,
            max_y,
            height,
            words: vec![0; bits.div_ceil(64)],
        }
    }

    pub fn min_y(&self) -> i32 {
        self.min_y
    }

    pub fn max_y(&self) -> i32 {
        self.max_y
    }

    #[inline]
    fn index(&self, x: i32, y: i32, z: i32) -> usize {
        (y - self.min_y + (z + (x << 4)) * self.height) as usize
    }

    /// `CarvingMask.carve(x, y, z)` with chunk-local `x`, `z`.
    #[inline]
    pub fn carve(&mut self, x: i32, y: i32, z: i32) {
        let i = self.index(x, y, z);
        self.words[i >> 6] |= 1u64 << (i & 63);
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|&w| w == 0)
    }

    /// Whether the bit of `(x, y, z)` is set (chunk-local `x`, `z`).
    pub fn get(&self, x: i32, y: i32, z: i32) -> bool {
        let i = self.index(x, y, z);
        self.words[i >> 6] & (1u64 << (i & 63)) != 0
    }

    /// Number of set bits.
    pub fn count(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// The bit words, least significant bit first (the Java `BitSet.toLongArray` layout).
    pub fn words(&self) -> &[u64] {
        &self.words
    }

    fn next_set_bit(&self, from: usize) -> Option<usize> {
        let total = self.words.len() * 64;
        let mut i = from;
        while i < total {
            let w = self.words[i >> 6] >> (i & 63);
            if w != 0 {
                return Some(i + w.trailing_zeros() as usize);
            }
            i = (i | 63) + 1;
        }
        None
    }

    fn next_clear_bit(&self, from: usize) -> usize {
        let total = self.words.len() * 64;
        let mut i = from;
        while i < total {
            let w = !self.words[i >> 6] >> (i & 63);
            if w != 0 {
                return i + w.trailing_zeros() as usize;
            }
            i = (i | 63) + 1;
        }
        total
    }

    /// `CarvingMask.visit`: maximal runs of set bits `(start, end)` in index order, each run split into
    /// per-column segments as `visitSegment` does. Calls `visit(x, z, bottomY, topY)` with chunk-local
    /// `x`, `z`. The mask must not change while visiting (the runs are fixed up front).
    pub fn visit<F: FnMut(i32, i32, i32, i32)>(&self, mut visit: F) {
        let h = self.height as usize;
        let mut start = self.next_set_bit(0);
        while let Some(s) = start {
            let e = self.next_clear_bit(s) - 1;
            let start_column = s / h;
            let end_column = e / h;
            for column in start_column..=end_column {
                let column_x = ((column >> 4) & 15) as i32;
                let column_z = (column & 15) as i32;
                let base = column * h;
                let bottom = (s as i64 - base as i64).max(0) as i32 + self.min_y;
                let top = (e as i64 - base as i64).min(h as i64 - 1) as i32 + self.min_y;
                visit(column_x, column_z, bottom, top);
            }
            start = self.next_set_bit(e + 1);
        }
    }
}
