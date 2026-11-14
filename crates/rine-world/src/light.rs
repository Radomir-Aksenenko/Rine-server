//! Lighting for a single chunk. Only data inside the chunk is used: block light
//! treats neighbours as dark, sky light treats the space above as open sky.
//!
//! Flat cell index `g = sy * 4096 + ((y & 15) << 8) | (z << 4) | x`, where `sy`
//! is the section index (0 = bottom). This is the section-local index shifted by
//! the section base, so `g >> 8` is the global row `sy * 16 + (y & 15)`.

use crate::chunk::{Chunk, Section, SECTIONS};

const SECTION_CELLS: usize = 4096;
const VOL: usize = SECTIONS * SECTION_CELLS;
const TOP_ROW: usize = SECTIONS * 16 - 1;

/// Compute sky light and block light for all sections of a chunk, using only data inside this chunk (treat neighbors as dark for block light, and as open sky above for sky light). Fills section.sky and section.block_light (Some(Box<[u8;2048]>) or None when all zero).
pub fn light_chunk(chunk: &mut Chunk) {
    let blocks = &rine_data::get().blocks;

    // Per-cell light properties.
    let mut emit = vec![0u8; VOL];
    let mut absorb = vec![0u8; VOL];
    let mut top: usize = 0; // number of sections up to and including the highest non-air one
    for (sy, s) in chunk.sections.iter().enumerate() {
        if s.non_air > 0 {
            top = sy + 1;
        }
        fill_props(s, blocks, &mut emit[sy * SECTION_CELLS..(sy + 1) * SECTION_CELLS], &mut absorb[sy * SECTION_CELLS..(sy + 1) * SECTION_CELLS]);
    }

    // Sky light: columns start at 15 at the top of the non-air region and are
    // darkened by light_block going down. Sections above `top` are all air.
    let mut sky = vec![0u8; VOL];
    sky[top * SECTION_CELLS..].fill(15);
    for z in 0..16 {
        for x in 0..16 {
            let mut l = 15u8;
            for sy in (0..top).rev() {
                for y in (0..16).rev() {
                    let g = sy * SECTION_CELLS + (y << 8) + (z << 4) + x;
                    let a = absorb[g];
                    if a > 0 {
                        l = l.saturating_sub(a);
                    }
                    sky[g] = l;
                }
            }
        }
    }
    let mut queue: Vec<u32> = (0..VOL).filter(|&g| sky[g] > 1).map(|g| g as u32).collect();
    spread(&mut sky, &absorb, &mut queue);

    // Block light: every emitter seeds the flood.
    let mut block = vec![0u8; VOL];
    let mut queue: Vec<u32> = Vec::new();
    for g in 0..VOL {
        if emit[g] > 0 {
            block[g] = emit[g];
            queue.push(g as u32);
        }
    }
    spread(&mut block, &absorb, &mut queue);

    for (sy, s) in chunk.sections.iter_mut().enumerate() {
        let base = sy * SECTION_CELLS;
        s.sky = pack_nibbles(&sky[base..base + SECTION_CELLS]);
        s.block_light = pack_nibbles(&block[base..base + SECTION_CELLS]);
    }
}

fn fill_props(s: &Section, blocks: &rine_data::blocks::Blocks, emit: &mut [u8], absorb: &mut [u8]) {
    match &s.blocks {
        None => {
            let st = blocks.state(s.single);
            emit.fill(st.light_emit);
            absorb.fill(st.light_block);
        }
        Some(arr) => {
            for i in 0..SECTION_CELLS {
                let st = blocks.state(arr[i]);
                emit[i] = st.light_emit;
                absorb[i] = st.light_block;
            }
        }
    }
}

/// Neighbours of flat cell `g` inside the chunk (6 directions).
#[inline]
fn neighbors(g: usize) -> ([usize; 6], usize) {
    let mut out = [0usize; 6];
    let mut n = 0;
    let x = g & 15;
    let z = (g >> 4) & 15;
    let y = g >> 8;
    if x > 0 {
        out[n] = g - 1;
        n += 1;
    }
    if x < 15 {
        out[n] = g + 1;
        n += 1;
    }
    if z > 0 {
        out[n] = g - 16;
        n += 1;
    }
    if z < 15 {
        out[n] = g + 16;
        n += 1;
    }
    if y > 0 {
        out[n] = g - 256;
        n += 1;
    }
    if y < TOP_ROW {
        out[n] = g + 256;
        n += 1;
    }
    (out, n)
}

/// BFS flood: a neighbour receives `light - max(1, absorb[neighbour])` when that is brighter.
fn spread(light: &mut [u8], absorb: &[u8], queue: &mut Vec<u32>) {
    let mut head = 0;
    while head < queue.len() {
        let g = queue[head] as usize;
        head += 1;
        let l = light[g];
        if l <= 1 {
            continue;
        }
        let (nbs, n) = neighbors(g);
        for &nb in &nbs[..n] {
            let cost = absorb[nb].max(1);
            if l > cost {
                let nl = l - cost;
                if nl > light[nb] {
                    light[nb] = nl;
                    queue.push(nb as u32);
                }
            }
        }
    }
    queue.clear();
}

/// Pack 4096 values (0..15) into the vanilla nibble layout. None when all zero.
fn pack_nibbles(vals: &[u8]) -> Option<Box<[u8; 2048]>> {
    if vals.iter().all(|&v| v == 0) {
        return None;
    }
    let mut out = Box::new([0u8; 2048]);
    for i in 0..2048 {
        out[i] = (vals[2 * i] & 15) | ((vals[2 * i + 1] & 15) << 4);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::{Chunk, ChunkPos, MIN_Y};

    fn nibble(arr: &[u8; 2048], i: usize) -> u8 {
        let b = arr[i >> 1];
        if i & 1 == 0 {
            b & 15
        } else {
            b >> 4
        }
    }

    #[test]
    fn empty_chunk_is_full_sky() {
        rine_data::init();
        let mut chunk = Chunk::new(ChunkPos::new(0, 0));
        light_chunk(&mut chunk);
        for s in &chunk.sections {
            let sky = s.sky.as_ref().expect("sky present");
            assert!(sky.iter().all(|&b| b == 0xFF));
            assert!(s.block_light.is_none());
        }
    }

    #[test]
    fn glowstone_block_light_spreads() {
        rine_data::init();
        let glow = rine_data::get().blocks.default_state("glowstone");
        let mut chunk = Chunk::new(ChunkPos::new(0, 0));
        chunk.set(8, 50, 8, glow);
        light_chunk(&mut chunk);

        let sy = ((50 - MIN_Y) >> 4) as usize;
        let ly = ((50 - MIN_Y) & 15) as usize;
        let s = &chunk.sections[sy];
        let bl = s.block_light.as_ref().expect("block light present");
        let at = |x: usize, z: usize| (ly << 8) | (z << 4) | x;
        assert_eq!(nibble(bl, at(8, 8)), 15);
        assert_eq!(nibble(bl, at(9, 8)), 14);
        assert_eq!(nibble(bl, at(10, 8)), 13);
    }
}
