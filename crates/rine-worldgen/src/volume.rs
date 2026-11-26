//! `DensityVolume`: a box of block positions (origin, size, step) and its flat value layout.
//! Layout matches Java: `index = y + (x + z * sizeX) * sizeY`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vol {
    pub sx: i32,
    pub sy: i32,
    pub sz: i32,
    pub mx: i32,
    pub my: i32,
    pub mz: i32,
    pub stx: i32,
    pub sty: i32,
    pub stz: i32,
}

impl Vol {
    /// `new DensityVolume(sizeX, sizeY, sizeZ, minX, minY, minZ)`: step 1 on every axis.
    pub fn new(sx: i32, sy: i32, sz: i32, mx: i32, my: i32, mz: i32) -> Self {
        Self::with_step(sx, sy, sz, mx, my, mz, 1, 1, 1)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_step(
        sx: i32,
        sy: i32,
        sz: i32,
        mx: i32,
        my: i32,
        mz: i32,
        stx: i32,
        sty: i32,
        stz: i32,
    ) -> Self {
        assert!(sx > 0 && sy > 0 && sz > 0, "Size must be positive");
        assert!(stx > 0 && sty > 0 && stz > 0, "Step must be positive");
        Self { sx, sy, sz, mx, my, mz, stx, sty, stz }
    }

    #[inline]
    pub fn size(&self) -> usize {
        (self.sx as usize) * (self.sy as usize) * (self.sz as usize)
    }

    #[inline]
    pub fn index(&self, x: i32, y: i32, z: i32) -> usize {
        (y + (x + z * self.sx) * self.sy) as usize
    }

    #[inline]
    pub fn block_x(&self, x: i32) -> i32 {
        self.mx.wrapping_add(x.wrapping_mul(self.stx))
    }

    #[inline]
    pub fn block_y(&self, y: i32) -> i32 {
        self.my.wrapping_add(y.wrapping_mul(self.sty))
    }

    #[inline]
    pub fn block_z(&self, z: i32) -> i32 {
        self.mz.wrapping_add(z.wrapping_mul(self.stz))
    }

    #[inline]
    pub fn max_block_x(&self) -> i32 {
        self.mx + self.sx * self.stx - 1
    }

    #[inline]
    pub fn max_block_y(&self) -> i32 {
        self.my + self.sy * self.sty - 1
    }

    #[inline]
    pub fn max_block_z(&self) -> i32 {
        self.mz + self.sz * self.stz - 1
    }

    /// `DensityVolume.indexOfBlock`: index of a block position inside this volume, or `None`.
    pub fn index_of_block(&self, bx: i32, by: i32, bz: i32) -> Option<usize> {
        let rx = bx.wrapping_sub(self.mx);
        let ry = by.wrapping_sub(self.my);
        let rz = bz.wrapping_sub(self.mz);
        if self.stx == 1 && self.sty == 1 && self.stz == 1 {
            if rx >= 0 && ry >= 0 && rz >= 0 && rx < self.sx && ry < self.sy && rz < self.sz {
                return Some(self.index(rx, ry, rz));
            }
        } else if rx >= 0
            && ry >= 0
            && rz >= 0
            && rx < self.sx * self.stx
            && ry < self.sy * self.sty
            && rz < self.sz * self.stz
        {
            return Some(self.index(
                rx.div_euclid(self.stx),
                ry.div_euclid(self.sty),
                rz.div_euclid(self.stz),
            ));
        }
        None
    }
}
