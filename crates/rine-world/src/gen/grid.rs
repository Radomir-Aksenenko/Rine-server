//! Flat block buffer for one chunk column stack (384 x 16 x 16), same layout as
//! a section stack: index `((y - MIN_Y) << 8) | (z << 4) | x`.

use crate::chunk::{HEIGHT, MAX_Y, MIN_Y};

pub struct Grid {
    pub data: Vec<u16>,
}

impl Grid {
    pub fn new(fill: u16) -> Self {
        Self { data: vec![fill; HEIGHT as usize * 256] }
    }

    #[inline]
    pub fn index(lx: i32, y: i32, lz: i32) -> usize {
        ((((y - MIN_Y) as usize) << 8) | ((lz as usize) << 4)) | lx as usize
    }

    #[inline]
    pub fn in_bounds(lx: i32, y: i32, lz: i32) -> bool {
        (0..16).contains(&lx) && (0..16).contains(&lz) && (MIN_Y..=MAX_Y).contains(&y)
    }

    #[inline]
    pub fn get(&self, lx: i32, y: i32, lz: i32) -> u16 {
        self.data[Self::index(lx, y, lz)]
    }

    #[inline]
    pub fn set(&mut self, lx: i32, y: i32, lz: i32, state: u16) {
        let i = Self::index(lx, y, lz);
        self.data[i] = state;
    }
}
