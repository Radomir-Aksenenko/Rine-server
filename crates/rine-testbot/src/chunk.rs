//! Full parser for clientbound level_chunk_with_light (play 46).
//! Layout: spec 02 sections 5, 5.1, 5.2 and 6 (protocol 777, overworld).

use anyhow::{ensure, Context, Result};
use rine_protocol::Reader;

use crate::nbt::skip_nbt;

/// Overworld: min_y -64, height 384 => 24 sections, section 0 starts at y -64.
pub const OVERWORLD_SECTIONS: usize = 24;
pub const OVERWORLD_MIN_Y: i32 = -64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkSummary {
    pub x: i32,
    pub z: i32,
    pub sections: usize,
    /// Count of block states with id 0 (minecraft:air) are treated as air.
    pub non_air: u32,
    pub block_entities: u32,
    /// Topmost non-air state at local column (0,0): (world y, block state id).
    pub column_top: Option<(i32, u32)>,
}

/// Parses a full level_chunk_with_light packet body (id already stripped).
/// Errors carry the chunk coordinates when they are known.
pub fn parse_level_chunk(body: &[u8]) -> Result<ChunkSummary> {
    let mut r = Reader::new(body);
    let x = r.i32()?;
    let z = r.i32()?;
    parse_inner(&mut r, x, z).with_context(|| format!("chunk ({x},{z})"))
}

fn parse_inner(r: &mut Reader, x: i32, z: i32) -> Result<ChunkSummary> {
    // 5.1 heightmaps: VarInt count, then (VarInt type, VarInt n, n x i64)
    let hm_count = r.varint()?;
    ensure!((0..=8).contains(&hm_count), "heightmap count {hm_count}");
    for _ in 0..hm_count {
        let _ty = r.varint()?;
        let n = r.varint()?;
        ensure!((0..=4096).contains(&n), "heightmap long count {n}");
        r.bytes(n as usize * 8)?;
    }

    // 5.1 buffer: VarInt byte length + sections, must be consumed exactly
    let buf_len = r.varint()?;
    ensure!(buf_len >= 0, "negative buffer length {buf_len}");
    let buf = r.bytes(buf_len as usize)?;
    let mut sr = Reader::new(buf);
    let mut non_air = 0u32;
    let mut column_top = None;
    for s in 0..OVERWORLD_SECTIONS {
        let sec = parse_section(&mut sr).with_context(|| format!("section {s}"))?;
        non_air += sec.non_air;
        for (ly, &state) in sec.column.iter().enumerate() {
            if state != 0 {
                // Ascending iteration: the last hit is the topmost block.
                column_top = Some((OVERWORLD_MIN_Y + s as i32 * 16 + ly as i32, state));
            }
        }
    }
    ensure!(sr.remaining() == 0, "{} bytes left after {OVERWORLD_SECTIONS} sections", sr.remaining());

    // 5.1 block entities
    let be_count = r.varint()?;
    ensure!(be_count >= 0, "negative block entity count {be_count}");
    for _ in 0..be_count {
        let _packed_xz = r.u8()?;
        let _y = r.i16()?;
        let _ty = r.varint()?;
        // OPTIONAL_COMPOUND_TAG: 0x00 when empty, else nameless compound
        let tag = r.u8()?;
        if tag != 0 {
            ensure!(tag == 10, "block entity nbt tag {tag}, expected compound");
            skip_nbt(r, tag)?;
        }
    }

    // 5.2 light: four BitSets, then sky and block array lists
    let sky_mask = bitset_popcount(r)?;
    let block_mask = bitset_popcount(r)?;
    let _empty_sky = bitset_popcount(r)?;
    let _empty_block = bitset_popcount(r)?;
    let sky_updates = light_arrays(r).context("sky light arrays")?;
    let block_updates = light_arrays(r).context("block light arrays")?;
    ensure!(sky_updates == sky_mask, "sky arrays {sky_updates} != sky mask bits {sky_mask}");
    ensure!(block_updates == block_mask, "block arrays {block_updates} != block mask bits {block_mask}");

    ensure!(r.remaining() == 0, "{} trailing bytes after light data", r.remaining());

    Ok(ChunkSummary {
        x,
        z,
        sections: OVERWORLD_SECTIONS,
        non_air,
        block_entities: be_count as u32,
        column_top,
    })
}

/// BitSet on the wire: VarInt byte length + bytes. Returns the set bit count.
fn bitset_popcount(r: &mut Reader) -> Result<u32> {
    let len = r.varint()?;
    ensure!((0..=8).contains(&len), "bitset byte length {len}");
    let b = r.bytes(len as usize)?;
    Ok(b.iter().map(|x| x.count_ones()).sum())
}

/// VarInt count, then per entry VarInt 2048 + 2048 bytes. Returns the count.
fn light_arrays(r: &mut Reader) -> Result<u32> {
    let n = r.varint()?;
    ensure!((0..=64).contains(&n), "light array count {n}");
    for _ in 0..n {
        let len = r.varint()?;
        ensure!(len == 2048, "light array length {len}");
        r.bytes(2048)?;
    }
    Ok(n as u32)
}

struct Section {
    non_air: u32,
    /// State ids at local (0, y, 0) for y in 0..16.
    column: [u32; 16],
}

fn parse_section(r: &mut Reader) -> Result<Section> {
    let _non_empty = r.i16()?;
    let _fluids = r.i16()?;
    let states = read_paletted(r, 4096, Kind::Block).context("block states")?;
    let _biomes = read_paletted(r, 64, Kind::Biome).context("biomes")?;
    let non_air = states.iter().filter(|&&s| s != 0).count() as u32;
    let mut column = [0u32; 16];
    for (ly, slot) in column.iter_mut().enumerate() {
        // idx = (y << 8) | (z << 4) | x with x = z = 0
        *slot = states[ly << 8];
    }
    Ok(Section { non_air, column })
}

#[derive(Clone, Copy)]
enum Kind {
    Block,
    Biome,
}

/// Reads a PalettedContainer and returns every entry as a global id.
///
/// Block (4096 entries): 0 single value; 1..=4 linear, storage 4 bits;
/// 5..=8 hashmap, storage = written bits; >= 9 global, storage 16 bits, no palette.
/// Biome (64 entries): 0 single value; 1..=3 linear, storage = written bits;
/// >= 4 global, storage 7 bits, no palette.
/// Longs are fixed-size (no length prefix), entries do not span longs.
fn read_paletted(r: &mut Reader, entries: usize, kind: Kind) -> Result<Vec<u32>> {
    let bits = r.u8()?;
    if bits == 0 {
        let v = r.varint()?;
        ensure!(v >= 0, "negative single-value palette entry {v}");
        return Ok(vec![v as u32; entries]);
    }
    let (storage, has_palette) = match kind {
        Kind::Block => match bits {
            1..=4 => (4u32, true),
            5..=8 => (bits as u32, true),
            _ => (16u32, false),
        },
        Kind::Biome => match bits {
            1..=3 => (bits as u32, true),
            _ => (7u32, false),
        },
    };
    let palette = if has_palette {
        let size = r.varint()?;
        ensure!((1..=4096).contains(&size), "palette size {size}");
        let mut p = Vec::with_capacity(size as usize);
        for _ in 0..size {
            let v = r.varint()?;
            ensure!(v >= 0, "negative palette entry {v}");
            p.push(v as u32);
        }
        Some(p)
    } else {
        None
    };

    let storage_us = storage as usize;
    let per_long = 64 / storage_us;
    let nlongs = (entries + per_long - 1) / per_long;
    let mut longs = Vec::with_capacity(nlongs);
    for _ in 0..nlongs {
        longs.push(r.i64()? as u64);
    }
    let mask = (1u64 << storage) - 1;
    let mut out = Vec::with_capacity(entries);
    for idx in 0..entries {
        let raw = (longs[idx / per_long] >> ((idx % per_long) * storage_us)) & mask;
        let v = match &palette {
            Some(p) => *p.get(raw as usize).context("palette index out of range")?,
            None => raw as u32,
        };
        out.push(v);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rine_protocol::Writer;

    /// Single-value block and biome container (bits 0) for one section.
    fn single_section(out: &mut Vec<u8>, block: i32) {
        out.w_i16(4096);
        out.w_i16(0);
        out.w_u8(0);
        out.w_varint(block);
        out.w_u8(0);
        out.w_varint(0);
    }

    /// Wraps section bytes into a full chunk body with empty heightmaps,
    /// block entities and light.
    fn wrap(sections: &[u8], x: i32, z: i32) -> Vec<u8> {
        let mut body = Vec::new();
        body.w_i32(x);
        body.w_i32(z);
        body.w_varint(0); // heightmaps
        body.w_byte_array(sections);
        body.w_varint(0); // block entities
        for _ in 0..4 {
            body.w_varint(0); // empty bitsets
        }
        body.w_varint(0); // sky arrays
        body.w_varint(0); // block arrays
        body
    }

    #[test]
    fn parses_single_value_chunk() {
        let mut sections = Vec::new();
        for _ in 0..OVERWORLD_SECTIONS {
            single_section(&mut sections, 1);
        }
        let body = wrap(&sections, 3, -2);
        let s = parse_level_chunk(&body).expect("chunk should parse");
        assert_eq!((s.x, s.z), (3, -2));
        assert_eq!(s.sections, 24);
        assert_eq!(s.non_air, 24 * 4096);
        assert_eq!(s.block_entities, 0);
        // Top section is index 23, local y 15 => world y 319.
        assert_eq!(s.column_top, Some((319, 1)));
    }

    #[test]
    fn parses_packed_linear_palette() {
        // Each section: 4-bit linear palette [air, 7], only entry 0 is 7.
        let mut sections = Vec::new();
        for _ in 0..OVERWORLD_SECTIONS {
            sections.w_i16(1);
            sections.w_i16(0);
            sections.w_u8(4);
            sections.w_varint(2);
            sections.w_varint(0);
            sections.w_varint(7);
            let mut longs = vec![0i64; 256];
            longs[0] = 1; // entry 0 -> palette index 1
            for l in longs {
                sections.w_i64(l);
            }
            sections.w_u8(0);
            sections.w_varint(0);
        }
        let body = wrap(&sections, 0, 0);
        let s = parse_level_chunk(&body).expect("packed chunk should parse");
        assert_eq!(s.non_air, 24);
        // Topmost hit is section 23, local (0,0,0) => world y 304.
        assert_eq!(s.column_top, Some((304, 7)));
    }

    #[test]
    fn rejects_trailing_bytes() {
        let mut sections = Vec::new();
        for _ in 0..OVERWORLD_SECTIONS {
            single_section(&mut sections, 1);
        }
        sections.push(0xAA);
        let body = wrap(&sections, 5, 6);
        let err = format!("{:#}", parse_level_chunk(&body).unwrap_err());
        assert!(err.contains("chunk (5,6)"), "{err}");
    }
}
