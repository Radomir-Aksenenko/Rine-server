//! Chunk -> ClientboundLevelChunkWithLightPacket encoding (packet id 46).
//! Wire layout follows docs/spec/02-play-join-chunks.md sections 5 and 6.

use bytes::BytesMut;
use rine_protocol::buf::Writer;

use crate::chunk::{Chunk, Section, SECTIONS};

pub const BLOCK_GLOBAL_BITS: u8 = 16; // ceil(log2(35723))
pub const BIOME_GLOBAL_BITS: u8 = 7; // ceil(log2(67))

const PACKET_ID: i32 = 46;
/// Heightmap entries: 256 values, 9 bits each, 7 values per long -> 37 longs.
const HEIGHTMAP_LONGS: usize = 37;

/// Write the body of ClientboundLevelChunkWithLightPacket (packet id 46) INCLUDING the leading VarInt packet id 46.
pub fn encode_chunk_packet(chunk: &Chunk, out: &mut BytesMut) {
    out.w_varint(PACKET_ID);
    out.w_i32(chunk.pos.x);
    out.w_i32(chunk.pos.z);

    // heightmaps: WORLD_SURFACE (1), MOTION_BLOCKING (4), MOTION_BLOCKING_NO_LEAVES (5)
    out.w_varint(3);
    write_heightmap(out, 1, &chunk.world_surface);
    write_heightmap(out, 4, &chunk.motion_blocking);
    write_heightmap(out, 5, &chunk.motion_blocking_no_leaves);

    // section buffer, length-prefixed
    let mut buf: Vec<u8> = Vec::with_capacity(SECTIONS * 2600);
    for s in &chunk.sections {
        write_section(s, &mut buf);
    }
    out.w_varint(buf.len() as i32);
    out.w_slice(&buf);

    // block entities
    out.w_varint(chunk.block_entities.len() as i32);
    for be in &chunk.block_entities {
        out.w_u8(((be.x & 15) << 4) | (be.z & 15));
        out.w_i16(be.y);
        out.w_varint(be.type_id as i32);
        write_update_tag(be, out);
    }

    write_light(chunk, out);
}

fn write_heightmap<W: Writer + ?Sized>(out: &mut W, type_id: i32, map: &[u16; 256]) {
    out.w_varint(type_id);
    out.w_varint(HEIGHTMAP_LONGS as i32);
    for j in 0..HEIGHTMAP_LONGS {
        let mut v: u64 = 0;
        for k in 0..7 {
            let col = j * 7 + k;
            if col < 256 {
                v |= ((map[col] as u64) & 0x1FF) << (k * 9);
            }
        }
        out.w_i64(v as i64);
    }
}

/// Write one section exactly like LevelChunkSection.write into `out`.
pub fn write_section(s: &Section, out: &mut Vec<u8>) {
    out.w_i16(s.non_air as i16);
    out.w_i16(s.fluids as i16);
    write_blocks(s, out);
    write_biomes(&s.biomes, out);
}

/// ceil(log2(n)) for n >= 1.
#[inline]
fn ceil_log2(n: usize) -> u32 {
    if n <= 1 {
        0
    } else {
        usize::BITS - (n - 1).leading_zeros()
    }
}

/// PalettedContainer for blocks (4096 entries).
fn write_blocks(s: &Section, out: &mut Vec<u8>) {
    let arr = match &s.blocks {
        None => {
            out.w_u8(0);
            out.w_varint(s.single as i32);
            return;
        }
        Some(a) => a,
    };

    // Build the palette. Stop as soon as it exceeds 256 entries (global palette).
    let mut palette: Vec<u16> = Vec::with_capacity(16);
    let mut indices = [0u8; 4096];
    let mut overflow = false;
    let mut prev_state = u16::MAX;
    let mut prev_idx = 0u8;
    for (i, &st) in arr.iter().enumerate() {
        if st == prev_state {
            indices[i] = prev_idx;
            continue;
        }
        let k = match palette.iter().position(|&p| p == st) {
            Some(k) => k as u8,
            None => {
                if palette.len() == 256 {
                    overflow = true;
                    break;
                }
                palette.push(st);
                (palette.len() - 1) as u8
            }
        };
        indices[i] = k;
        prev_state = st;
        prev_idx = k;
    }

    if overflow {
        // Global palette: no palette, 4 raw state ids per long (16 bits each).
        out.w_u8(BLOCK_GLOBAL_BITS);
        for j in 0..1024 {
            let mut v: u64 = 0;
            for k in 0..4 {
                v |= (arr[j * 4 + k] as u64) << (k * 16);
            }
            out.w_i64(v as i64);
        }
        return;
    }

    match palette.len() {
        1 => {
            out.w_u8(0);
            out.w_varint(palette[0] as i32);
        }
        n => {
            let bits: u32 = if n <= 16 { 4 } else { ceil_log2(n) };
            let per_long = (64 / bits) as usize;
            let nlongs = 4096usize.div_ceil(per_long);
            let mut longs = vec![0u64; nlongs];
            for (i, &k) in indices.iter().enumerate() {
                longs[i / per_long] |= (k as u64) << ((i % per_long) as u32 * bits);
            }
            out.w_u8(bits as u8);
            out.w_varint(n as i32);
            for &id in &palette {
                out.w_varint(id as i32);
            }
            for l in longs {
                out.w_i64(l as i64);
            }
        }
    }
}

/// PalettedContainer for biomes (64 entries, quart coordinates).
fn write_biomes(biomes: &[u8; 64], out: &mut Vec<u8>) {
    let mut palette: Vec<u8> = Vec::with_capacity(8);
    let mut indices = [0u8; 64];
    let mut overflow = false;
    for (i, &b) in biomes.iter().enumerate() {
        let k = match palette.iter().position(|&p| p == b) {
            Some(k) => k as u8,
            None => {
                if palette.len() == 8 {
                    overflow = true;
                    break;
                }
                palette.push(b);
                (palette.len() - 1) as u8
            }
        };
        indices[i] = k;
    }

    if overflow {
        // Global palette: 9 entries per long, 7 bits each -> 8 longs.
        out.w_u8(BIOME_GLOBAL_BITS);
        for j in 0..8 {
            let mut v: u64 = 0;
            for k in 0..9 {
                let e = j * 9 + k;
                if e < 64 {
                    v |= ((biomes[e] as u64) & 0x7F) << (k * 7);
                }
            }
            out.w_i64(v as i64);
        }
        return;
    }

    match palette.len() {
        1 => {
            out.w_u8(0);
            out.w_varint(palette[0] as i32);
        }
        n => {
            let bits = ceil_log2(n).max(1);
            let per_long = (64 / bits) as usize;
            let nlongs = 64usize.div_ceil(per_long);
            let mut longs = vec![0u64; nlongs];
            for (i, &k) in indices.iter().enumerate() {
                longs[i / per_long] |= (k as u64) << ((i % per_long) as u32 * bits);
            }
            out.w_u8(bits as u8);
            out.w_varint(n as i32);
            for &id in &palette {
                out.w_varint(id as i32);
            }
            for l in longs {
                out.w_i64(l as i64);
            }
        }
    }
}

fn write_bitset<W: Writer + ?Sized>(out: &mut W, mask: u32) {
    let bytes = mask.to_le_bytes();
    let mut len = 4;
    while len > 0 && bytes[len - 1] == 0 {
        len -= 1;
    }
    out.w_varint(len as i32);
    out.w_slice(&bytes[..len]);
}

/// Light part (ClientboundLightUpdatePacketData) for 26 light sections.
/// Index i is section y = -5 + i: index 0 is below the world, index 25 above it.
pub fn write_light(chunk: &Chunk, out: &mut impl Writer) {
    const N: usize = SECTIONS + 2;
    static FULL: [u8; 2048] = [0xFF; 2048];

    let mut sky_mask = 0u32;
    let mut block_mask = 0u32;
    let mut empty_sky = 0u32;
    let mut empty_block = 0u32;
    let mut sky_arrays: Vec<&[u8]> = Vec::with_capacity(N);
    let mut block_arrays: Vec<&[u8]> = Vec::with_capacity(N);

    let nonzero = |a: &&[u8]| a.iter().any(|&b| b != 0);

    for i in 0..N {
        let inside = i > 0 && i < N - 1;
        let sec = if inside { chunk.sections.get(i - 1) } else { None };

        let sky: Option<&[u8]> = if i == N - 1 {
            Some(&FULL[..])
        } else if inside {
            sec.and_then(|s| s.sky.as_deref()).map(|a| &a[..]).filter(nonzero)
        } else {
            None
        };
        match sky {
            Some(a) => {
                sky_mask |= 1 << i;
                sky_arrays.push(a);
            }
            None => empty_sky |= 1 << i,
        }

        let blk: Option<&[u8]> = if inside {
            sec.and_then(|s| s.block_light.as_deref()).map(|a| &a[..]).filter(nonzero)
        } else {
            None
        };
        match blk {
            Some(a) => {
                block_mask |= 1 << i;
                block_arrays.push(a);
            }
            None => empty_block |= 1 << i,
        }
    }

    write_bitset(out, sky_mask);
    write_bitset(out, block_mask);
    write_bitset(out, empty_sky);
    write_bitset(out, empty_block);
    out.w_varint(sky_arrays.len() as i32);
    for a in sky_arrays {
        out.w_varint(2048);
        out.w_slice(a);
    }
    out.w_varint(block_arrays.len() as i32);
    for a in block_arrays {
        out.w_varint(2048);
        out.w_slice(a);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::{Chunk, ChunkPos};
    use rine_protocol::buf::Reader;

    #[test]
    fn packet_header_and_stone_section() {
        rine_data::init();
        let stone = rine_data::get().blocks.default_state("stone");
        let mut chunk = Chunk::new(ChunkPos::new(3, -5));
        chunk.sections[4].fill(stone);

        let mut out = BytesMut::new();
        encode_chunk_packet(&chunk, &mut out);

        let mut r = Reader::new(&out);
        assert_eq!(r.varint().unwrap(), 46);
        assert_eq!(r.i32().unwrap(), 3);
        assert_eq!(r.i32().unwrap(), -5);
    }

    #[test]
    fn single_value_section_layout() {
        rine_data::init();
        let stone = rine_data::get().blocks.default_state("stone");
        let mut chunk = Chunk::new(ChunkPos::new(0, 0));
        chunk.sections[4].fill(stone);

        let mut out = Vec::new();
        write_section(&chunk.sections[4], &mut out);

        let mut expected = Vec::new();
        expected.w_i16(4096);
        expected.w_i16(0);
        expected.w_u8(0);
        expected.w_varint(stone as i32);
        expected.w_u8(0);
        expected.w_varint(0);
        assert_eq!(out, expected);
    }

    #[test]
    fn two_state_section_uses_4_bit_palette() {
        rine_data::init();
        let stone = rine_data::get().blocks.default_state("stone");
        let mut chunk = Chunk::new(ChunkPos::new(0, 0));
        let s = &mut chunk.sections[0];
        for i in 0..2048 {
            s.set(i, stone);
        }

        let mut out = Vec::new();
        write_blocks(&chunk.sections[0], &mut out);
        // bits byte (4), palette size (2), palette in first-seen order (stone, then air), then 256 longs
        let mut expected = Vec::new();
        expected.w_u8(4);
        expected.w_varint(2);
        expected.w_varint(stone as i32);
        expected.w_varint(0);
        expected.extend(std::iter::repeat(0u8).take(256 * 8));
        assert_eq!(out.len(), expected.len());
        assert_eq!(&out[..out.len() - 256 * 8], &expected[..expected.len() - 256 * 8]);
    }

    #[test]
    fn light_section_mask_after_light_pass() {
        rine_data::init();
        let mut chunk = Chunk::new(ChunkPos::new(0, 0));
        crate::light::light_chunk(&mut chunk);
        let mut out: Vec<u8> = Vec::new();
        write_light(&chunk, &mut out);

        // Sky: only index 0 (below world) is empty, so bits 1..=25 are set in skyMask
        // (index 25 is the full open-sky section, indices 1..=24 are all-15 open air).
        // Block light is empty everywhere.
        let mut r = Reader::new(&out);
        let sky_len = r.varint().unwrap() as usize;
        let sky_mask = r.bytes(sky_len).unwrap().to_vec();
        assert_eq!(sky_mask, vec![0xFE, 0xFF, 0xFF, 0x03]);
        let block_len = r.varint().unwrap() as usize;
        assert_eq!(block_len, 0);
    }
}

/// Saved keys that vanilla `getUpdateTag` leaves out of the chunk packet, by block entity type (the default update
/// tag is empty; a spawner sends its saved data without `SpawnPotentials`). Generated block entities carry them.
fn hidden_keys(type_id: u32) -> &'static [&'static str] {
    static TABLE: std::sync::OnceLock<Vec<&'static [&'static str]>> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let Some(reg) = rine_data::get().registries.find("minecraft:block_entity_type") else { return Vec::new() };
        let mut v: Vec<&'static [&'static str]> = vec![&[]; reg.entries.len()];
        let rules: [(&str, &'static [&'static str]); 9] = [
            ("minecraft:mob_spawner", &["SpawnPotentials"]),
            ("minecraft:chest", &["LootTable", "LootTableSeed"]),
            ("minecraft:trapped_chest", &["LootTable", "LootTableSeed"]),
            ("minecraft:brushable_block", &["LootTable", "LootTableSeed"]),
            ("minecraft:beehive", &["bees"]),
            ("minecraft:sculk_catalyst", &["cursors"]),
            ("minecraft:sculk_sensor", &["last_vibration_frequency", "listener"]),
            ("minecraft:sculk_shrieker", &["warning_level", "listener"]),
            ("minecraft:potent_sulfur", &["countdown"]),
        ];
        for (name, keys) in rules {
            if let Some(id) = reg.id(name) {
                v[id as usize] = keys;
            }
        }
        v
    });
    table.get(type_id as usize).copied().unwrap_or(&[])
}

fn write_update_tag<W: Writer + ?Sized>(be: &crate::chunk::BlockEntity, out: &mut W) {
    use rine_protocol::nbt::Tag;
    let Some(tag) = &be.nbt else {
        out.w_u8(0);
        return;
    };
    let hidden = hidden_keys(be.type_id);
    match tag {
        Tag::Compound(fields) if fields.iter().any(|(k, _)| hidden.contains(&k.as_str())) => {
            let kept: Vec<(String, Tag)> = fields.iter().filter(|(k, _)| !hidden.contains(&k.as_str())).cloned().collect();
            if kept.is_empty() {
                out.w_u8(0);
            } else {
                Tag::Compound(kept).write_network(out);
            }
        }
        _ => tag.write_network(out),
    }
}
