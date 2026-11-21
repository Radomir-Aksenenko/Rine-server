//! Uncompressed `DiskSlots` payload. The store owns compression and atomic file replacement.

use super::{read_block_entity_suffix, write_block_entity_suffix, SavedSlot};

/// Encode the existing compact palette and append block entities after the legacy tick list.
pub fn encode(slot: &SavedSlot) -> Option<Vec<u8>> {
    if slot.final_chunk {
        return Some(vec![1, u8::from(slot.decorated)]);
    }
    let mut out = Vec::with_capacity(200 * 1024);
    out.extend_from_slice(&[0, u8::from(slot.decorated)]);
    let mut remap = vec![u16::MAX; slot.palette.len()];
    let mut names = Vec::new();
    let mut intern = |state: u16| -> Option<u16> {
        let local = remap.get_mut(state as usize)?;
        if *local == u16::MAX {
            *local = u16::try_from(names.len())
                .ok()
                .filter(|&id| id < u16::MAX)?;
            names.push(slot.palette.get(state as usize)?.as_str());
        }
        Some(*local)
    };
    let blocks = slot
        .blocks
        .iter()
        .map(|&state| intern(state))
        .collect::<Option<Vec<_>>>()?;
    let ticks = slot
        .ticks
        .iter()
        .map(|&(p, state)| Some((p, intern(state)?)))
        .collect::<Option<Vec<_>>>()?;
    strings(&mut out, names.into_iter())?;
    states(&mut out, &blocks)?;
    strings(&mut out, slot.biome_palette.iter().map(String::as_str))?;
    states(&mut out, &slot.biomes)?;
    for values in [&slot.wg[0], &slot.wg[1], &slot.heights] {
        put_len(&mut out, values.len())?;
        out.extend(values.iter().flat_map(|v| v.to_le_bytes()));
    }
    put_len(&mut out, ticks.len())?;
    for ((x, y, z), state) in ticks {
        out.extend([x, y, z].into_iter().flat_map(i32::to_le_bytes));
        out.extend_from_slice(&state.to_le_bytes());
    }
    write_block_entity_suffix(&mut out, &slot.block_entities).ok()?;
    Some(out)
}

/// Decode a record with or without the new suffix.
pub fn decode(data: &[u8]) -> Option<SavedSlot> {
    let mut reader = Reader(data);
    let final_chunk = reader.take(1)?[0] != 0;
    let decorated = reader.take(1)?[0] != 0;
    if final_chunk {
        return Some(SavedSlot {
            decorated,
            ..super::final_marker()
        });
    }
    let palette = reader.strings()?;
    let blocks = reader.states()?;
    let biome_palette = reader.strings()?;
    let biomes = reader.states()?;
    let wg = [reader.heights()?, reader.heights()?];
    let heights = reader.heights()?;
    let n = reader.len()?;
    let bytes = reader.take(n.checked_mul(14)?)?;
    let mut ticks = Vec::with_capacity(n);
    let mut tick = Reader(bytes);
    for _ in 0..n {
        ticks.push(((tick.i32()?, tick.i32()?, tick.i32()?), tick.u16()?));
    }
    let block_entities = read_block_entity_suffix(reader.0).ok()?;
    Some(SavedSlot {
        final_chunk,
        decorated,
        palette,
        blocks,
        biome_palette,
        biomes,
        wg,
        heights,
        ticks,
        block_entities,
    })
}

fn put_len(out: &mut Vec<u8>, len: usize) -> Option<()> {
    out.extend_from_slice(&u32::try_from(len).ok()?.to_le_bytes());
    Some(())
}

fn strings<'a>(out: &mut Vec<u8>, strings: impl ExactSizeIterator<Item = &'a str>) -> Option<()> {
    put_len(out, strings.len())?;
    for s in strings {
        out.extend_from_slice(&u16::try_from(s.len()).ok()?.to_le_bytes());
        out.extend_from_slice(s.as_bytes());
    }
    Some(())
}

fn states(out: &mut Vec<u8>, values: &[u16]) -> Option<()> {
    put_len(out, values.len())?;
    out.extend(values.iter().flat_map(|v| v.to_le_bytes()));
    Some(())
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, rest) = self.0.split_at_checked(n)?;
        self.0 = rest;
        Some(head)
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn len(&mut self) -> Option<usize> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?) as usize)
    }

    fn strings(&mut self) -> Option<Vec<String>> {
        let count = self.len()?;
        if count > self.0.len() / 2 {
            return None;
        }
        (0..count)
            .map(|_| {
                let len = self.u16()? as usize;
                Some(std::str::from_utf8(self.take(len)?).ok()?.to_owned())
            })
            .collect()
    }

    fn states(&mut self) -> Option<Vec<u16>> {
        let count = self.len()?;
        let data = self.take(count.checked_mul(2)?)?;
        Some(
            data.chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect(),
        )
    }

    fn heights(&mut self) -> Option<Vec<i32>> {
        let count = self.len()?;
        let data = self.take(count.checked_mul(4)?)?;
        Some(
            data.chunks_exact(4)
                .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{BlockEntity, BlockEntityData};

    #[test]
    fn legacy_record_is_byte_compatible() {
        // Legacy prefix with one block, one biome, one WG height, and one block tick.
        let old = b"\0\x01\x01\0\0\0\x0f\0minecraft:stone\x01\0\0\0\0\0\
            \x01\0\0\0\x10\0minecraft:plains\x01\0\0\0\0\0\
            \x01\0\0\0\x40\0\0\0\0\0\0\0\0\0\0\0\
            \x01\0\0\0\x10\0\0\0\xff\xff\xff\xff\0\0\0\0\0\0";
        let mut slot = decode(old).unwrap();
        assert_eq!(slot.blocks, [0]);
        assert_eq!(slot.ticks, [((16, -1, 0), 0)]);
        assert!(slot.block_entities.is_empty());
        assert_eq!(encode(&slot).unwrap(), old);
        slot.block_entities.push(BlockEntity {
            pos: (16, -1, 2),
            type_id: "minecraft:beehive".into(),
            data: BlockEntityData::Beehive {
                ticks_in_hive: vec![0, 598],
            },
        });
        let bytes = encode(&slot).unwrap();
        assert!(bytes.starts_with(old));
        assert_eq!(decode(&bytes), Some(slot));
        assert!(decode(&bytes[..bytes.len() - 1]).is_none());
        assert_eq!(encode(&decode(&[1, 1]).unwrap()).unwrap(), [1, 1]);
    }
}
